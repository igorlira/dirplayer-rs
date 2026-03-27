use crate::director::lingo::datum::Datum;

use crate::player::{
    cast_member::CastMemberType,
    font::{get_text_index_at_pos, DrawTextParams},
    player_call_script_handler, player_handle_scope_return,
    reserve_player_mut, reserve_player_ref,
    script::{script_get_prop, script_set_prop},
    script_ref::ScriptInstanceRef, DatumRef, DirPlayer, ScriptError, ScriptErrorCode,
    score::get_concrete_sprite_rect,
};

use super::script_instance::ScriptInstanceUtils;

pub struct SpriteDatumHandlers {}

pub struct SpriteDatumUtils {}

impl SpriteDatumUtils {
    pub fn get_script_instance_ids(
        datum: &DatumRef,
        player: &DirPlayer,
    ) -> Result<Vec<ScriptInstanceRef>, ScriptError> {
        let sprite_num = player.get_datum(datum).to_sprite_ref()?;
        let sprite = player.movie.score.get_sprite(sprite_num);
        if sprite.is_none() {
            return Ok(vec![]);
        }
        let sprite = sprite.unwrap();
        let instances = &sprite.script_instance_list;
        Ok(instances.clone())
    }

    /// Resolves the text content and character index at a stage point for a text/field sprite.
    /// Returns (text, char_index) or None if the sprite has no text member.
    fn get_text_char_index_at_point(
        player: &DirPlayer,
        datum: &DatumRef,
        point_arg: &DatumRef,
    ) -> Result<Option<(String, usize)>, ScriptError> {
        let sprite_num = player.get_datum(datum).to_sprite_ref()?;
        let point = player.get_datum(point_arg).to_point()?;
        let stage_x = player.get_datum(&point[0]).int_value()?;
        let stage_y = player.get_datum(&point[1]).int_value()?;

        let sprite = match player.movie.score.get_sprite(sprite_num) {
            Some(s) => s,
            None => return Ok(None),
        };

        let member_ref = match &sprite.member {
            Some(r) => r.clone(),
            None => return Ok(None),
        };

        let sprite_rect = get_concrete_sprite_rect(player, sprite);
        let local_x = stage_x - sprite_rect.left;
        let local_y = stage_y - sprite_rect.top;

        let member = match player.movie.cast_manager.find_member_by_ref(&member_ref) {
            Some(m) => m,
            None => return Ok(None),
        };

        let (text, fixed_line_space, top_spacing) = match &member.member_type {
            CastMemberType::Text(t) => (t.text.clone(), t.fixed_line_space, t.top_spacing),
            CastMemberType::Field(f) => (f.text.clone(), f.fixed_line_space, f.top_spacing),
            _ => return Ok(None),
        };

        let font = player.font_manager.get_system_font().unwrap();
        let params = DrawTextParams {
            font: &font,
            line_height: None,
            line_spacing: fixed_line_space,
            top_spacing,
        };

        let char_index = get_text_index_at_pos(&text, &params, local_x, local_y);
        Ok(Some((text, char_index)))
    }
}

impl SpriteDatumHandlers {
    /// Returns true if the handler should be called via the async path.
    /// This returns true for:
    /// 1. Handlers found on the sprite's attached script instances
    /// 2. Any handler that isn't a built-in sync handler (to allow fallback to global handlers)
    pub fn has_async_handler(datum: &DatumRef, handler_name: &String) -> Result<bool, ScriptError> {
        // First check if it's a built-in sync handler
        let is_sync_handler = matches!(handler_name.as_str(), "intersects" | "getProp" | "getAt" | "setAt" | "getaProp" | "setaProp" | "pointToWord" | "pointToLine");
        if is_sync_handler {
            return Ok(false);
        }

        // For all other handlers, use the async path which will:
        // 1. Try sprite's attached scripts
        // 2. Fall back to global handlers
        Ok(true)
    }

    pub fn call(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "intersects" => {
                reserve_player_mut(|player| {
                    if args.is_empty() {
                        return Err(ScriptError::new(
                            "intersects requires 1 argument (sprite number)".to_string(),
                        ));
                    }

                    let sprite_num = player.get_datum(datum).to_sprite_ref()?;
                    let other_sprite_num =
                        player.get_datum(&args[0]).int_value()? as i16;

                    // Get both sprites' rects
                    let sprite1 = player.movie.score.get_sprite(sprite_num);
                    let sprite2 = player.movie.score.get_sprite(other_sprite_num);

                    if sprite1.is_none() || sprite2.is_none() {
                        return Ok(player.alloc_datum(Datum::Int(0)));
                    }

                    let sprite1 = sprite1.unwrap();
                    let sprite2 = sprite2.unwrap();

                    // Get the concrete rects of both sprites
                    let rect1 = get_concrete_sprite_rect(player, sprite1);
                    let rect2 = get_concrete_sprite_rect(player, sprite2);

                    // Check if rectangles intersect
                    let intersects = !(
                        rect1.right <= rect2.left
                            || rect1.left >= rect2.right
                            || rect1.bottom <= rect2.top
                            || rect1.top >= rect2.bottom
                    );

                    Ok(player.alloc_datum(Datum::Int(if intersects { 1 } else { 0 })))
                })
            }
            "getProp" => {
                reserve_player_mut(|player| {
                    if args.is_empty() {
                        return Err(ScriptError::new(
                            "getProp requires at least 1 argument".to_string(),
                        ));
                    }

                    let sprite_num = player.get_datum(datum).to_sprite_ref()?;

                    // Get the property name from the first arg
                    let prop_name = player.get_datum(&args[0]).string_value()?;

                    // First, try to get it as a built-in sprite property
                    match crate::player::score::sprite_get_prop(
                        player,
                        sprite_num as i16,
                        &prop_name,
                    ) {
                        Ok(prop_datum) => {
                            let result = player.last_sprite_prop_ref.take()
                                .unwrap_or_else(|| player.alloc_datum(prop_datum));

                            // If there's a second argument, it's a sub-property access
                            if args.len() > 1 {
                                return crate::player::handlers::types::TypeUtils::get_sub_prop(
                                    &result, &args[1], player,
                                );
                            }

                            return Ok(result);
                        }
                        Err(_) => {
                            // Not a built-in sprite property, try script instances
                        }
                    }

                    let sprite = player.movie.score.get_sprite(sprite_num);
                    if sprite.is_none() {
                        return Err(ScriptError::new(format!("Sprite {} not found", sprite_num)));
                    }

                    // Clone the script instance list to avoid borrow conflicts
                    let instance_refs = sprite.unwrap().script_instance_list.clone();

                    // Try to get the property from the sprite's script instances
                    for instance_ref in instance_refs {
                        if let Ok(result) = script_get_prop(
                            player,
                            &instance_ref,
                            &prop_name,
                        ) {
                            // If there's a second argument, it's a sub-property access
                            if args.len() > 1 {
                                return crate::player::handlers::types::TypeUtils::get_sub_prop(
                                    &result, &args[1], player,
                                );
                            }
                            return Ok(result);
                        }
                    }

                    // If not found anywhere, return void
                    Ok(DatumRef::Void)
                })
            }
            // getAt / getaProp: bracket access on sprite, e.g. sprite(9)[#pLevel]
            "getAt" | "getaProp" => {
                reserve_player_mut(|player| {
                    if args.is_empty() {
                        return Err(ScriptError::new(
                            "getAt requires 1 argument".to_string(),
                        ));
                    }

                    let sprite_num = player.get_datum(datum).to_sprite_ref()?;
                    let prop_name = player.get_datum(&args[0]).string_value()?;

                    // Try built-in sprite property first
                    match crate::player::score::sprite_get_prop(
                        player,
                        sprite_num as i16,
                        &prop_name,
                    ) {
                        Ok(prop_datum) => {
                            return Ok(player.last_sprite_prop_ref.take()
                                .unwrap_or_else(|| player.alloc_datum(prop_datum)));
                        }
                        Err(_) => {}
                    }

                    // Fall back to sprite's script instance properties
                    let sprite = player.movie.score.get_sprite(sprite_num);
                    if sprite.is_none() {
                        return Ok(DatumRef::Void);
                    }
                    let instance_refs = sprite.unwrap().script_instance_list.clone();
                    for instance_ref in instance_refs {
                        if let Ok(result) = script_get_prop(player, &instance_ref, &prop_name) {
                            return Ok(result);
                        }
                    }

                    Ok(DatumRef::Void)
                })
            }
            // setAt / setaProp: bracket assignment on sprite, e.g. sprite(9)[#pLevel] = value
            "setAt" | "setaProp" => {
                reserve_player_mut(|player| {
                    if args.len() < 2 {
                        return Err(ScriptError::new(
                            "setAt requires 2 arguments".to_string(),
                        ));
                    }

                    let sprite_num = player.get_datum(datum).to_sprite_ref()?;
                    let prop_name = player.get_datum(&args[0]).string_value()?;
                    let value = player.get_datum(&args[1]).clone();
                    let value_ref = &args[1];

                    // Try built-in sprite property first
                    match crate::player::score::sprite_set_prop_from_lingo(
                        sprite_num as i16,
                        &prop_name,
                        value,
                    ) {
                        Ok(_) => return Ok(DatumRef::Void),
                        Err(_) => {}
                    }

                    // Fall back to sprite's script instance properties
                    let sprite = player.movie.score.get_sprite(sprite_num);
                    if sprite.is_none() {
                        return Err(ScriptError::new(format!("Sprite {} not found", sprite_num)));
                    }
                    let instance_refs = sprite.unwrap().script_instance_list.clone();
                    for instance_ref in instance_refs {
                        if let Ok(_) = script_set_prop(player, &instance_ref, &prop_name, value_ref, false) {
                            return Ok(DatumRef::Void);
                        }
                    }

                    Err(ScriptError::new(format!(
                        "Property {} not found on sprite {}", prop_name, sprite_num
                    )))
                })
            }
            "pointToWord" => {
                reserve_player_mut(|player| {
                    if args.is_empty() {
                        return Err(ScriptError::new(
                            "pointToWord requires 1 argument (point)".to_string(),
                        ));
                    }

                    let (text, char_index) = match SpriteDatumUtils::get_text_char_index_at_point(player, datum, &args[0])? {
                        Some(r) => r,
                        None => return Ok(player.alloc_datum(Datum::Int(-1))),
                    };

                    // Find which word (1-based) the character at char_index belongs to
                    let mut word_num = 0;
                    let mut char_count = 0;
                    let mut in_word = false;
                    for c in text.chars() {
                        if c.is_whitespace() {
                            in_word = false;
                        } else if !in_word {
                            word_num += 1;
                            in_word = true;
                        }
                        if char_count == char_index {
                            return Ok(player.alloc_datum(Datum::Int(word_num)));
                        }
                        char_count += 1;
                    }

                    // Past end of text: return the last word number
                    Ok(player.alloc_datum(Datum::Int(word_num)))
                })
            }
            "pointToLine" => {
                reserve_player_mut(|player| {
                    if args.is_empty() {
                        return Err(ScriptError::new(
                            "pointToLine requires 1 argument (point)".to_string(),
                        ));
                    }

                    let (text, char_index) = match SpriteDatumUtils::get_text_char_index_at_point(player, datum, &args[0])? {
                        Some(r) => r,
                        None => return Ok(player.alloc_datum(Datum::Int(-1))),
                    };

                    // Find which line (1-based) the character at char_index belongs to
                    let mut line_num = 1;
                    let mut char_count = 0;
                    for c in text.chars() {
                        if char_count == char_index {
                            return Ok(player.alloc_datum(Datum::Int(line_num)));
                        }
                        if c == '\r' || c == '\n' {
                            line_num += 1;
                        }
                        char_count += 1;
                    }

                    // Past end of text: return the last line number
                    Ok(player.alloc_datum(Datum::Int(line_num)))
                })
            }
            _ => Err(ScriptError::new_code(
                ScriptErrorCode::HandlerNotFound,
                format!("No sync handler {handler_name} for sprite"),
            )),
        }
    }

    pub async fn call_async(
        datum: DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        // First, try the sprite's attached script instances
        let instance_refs =
            reserve_player_ref(|player| SpriteDatumUtils::get_script_instance_ids(&datum, player))?;
        for instance_ref in instance_refs {
            let handler_ref = reserve_player_ref(|player| {
                ScriptInstanceUtils::get_script_instance_handler(
                    handler_name,
                    &instance_ref,
                    player,
                )
            })?;
            if let Some(handler_ref) = handler_ref {
                let result_scope =
                    player_call_script_handler(Some(instance_ref), handler_ref, args).await?;
                player_handle_scope_return(&result_scope);
                return Ok(result_scope.return_value);
            }
        }

        Err(ScriptError::new_code(
            ScriptErrorCode::HandlerNotFound,
            format!("No async handler {handler_name} found for sprite"),
        ))
    }
}
