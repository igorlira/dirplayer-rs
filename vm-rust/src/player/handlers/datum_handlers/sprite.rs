use crate::director::lingo::datum::Datum;

use crate::player::{
    player_call_script_handler, player_call_global_handler, player_handle_scope_return,
    reserve_player_mut, reserve_player_ref,
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
}

impl SpriteDatumHandlers {
    /// Returns true if the handler should be called via the async path.
    /// This returns true for:
    /// 1. Handlers found on the sprite's attached script instances
    /// 2. Any handler that isn't a built-in sync handler (to allow fallback to global handlers)
    pub fn has_async_handler(datum: &DatumRef, handler_name: &String) -> Result<bool, ScriptError> {
        // First check if it's a built-in sync handler
        let is_sync_handler = matches!(handler_name.as_str(), "intersects" | "getProp");
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
                            let result = player.alloc_datum(prop_datum);

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
                        if let Ok(result) = crate::player::script::script_get_prop(
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

        // No handler found on sprite's scripts - fall back to global handlers
        // This allows game scripts to define handlers like "setcursor" that can be
        // called on sprites even if not attached directly to the sprite
        player_call_global_handler(handler_name, args).await
    }
}
