use log::warn;
use wasm_bindgen::prelude::*;

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

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "ruffleGoToFrame")]
    fn ruffle_goto_frame(cast_lib: i32, cast_member: i32, frame: i32);
    #[wasm_bindgen(js_name = "ruffleStop")]
    fn ruffle_stop(cast_lib: i32, cast_member: i32);
    #[wasm_bindgen(js_name = "rufflePlay")]
    fn ruffle_play(cast_lib: i32, cast_member: i32);
    #[wasm_bindgen(js_name = "ruffleRewind")]
    fn ruffle_rewind(cast_lib: i32, cast_member: i32);
    #[wasm_bindgen(js_name = "ruffleCallFrame")]
    fn ruffle_call_frame(cast_lib: i32, cast_member: i32, frame: i32);
    #[wasm_bindgen(js_name = "ruffleGetVariable", catch)]
    fn ruffle_get_variable(cast_lib: i32, cast_member: i32, path: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = "ruffleSetVariable", catch)]
    fn ruffle_set_variable(cast_lib: i32, cast_member: i32, path: &str, value: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = "ruffleCallFunction", catch)]
    fn ruffle_call_function(cast_lib: i32, cast_member: i32, path: &str, args_xml: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = "ruffleHitTest")]
    fn ruffle_hit_test(cast_lib: i32, cast_member: i32, x: f64, y: f64) -> bool;
    #[wasm_bindgen(js_name = "ruffleGetFlashProperty", catch)]
    fn ruffle_get_flash_property(cast_lib: i32, cast_member: i32, target: &str, prop_num: i32) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = "ruffleSetFlashProperty")]
    fn ruffle_set_flash_property(cast_lib: i32, cast_member: i32, target: &str, prop_num: i32, value: &str);
}

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
            char_spacing: 0,
            member_width: None,
        };

        let char_index = get_text_index_at_pos(&text, &params, local_x, local_y);
        Ok(Some((text, char_index)))
    }
}

impl SpriteDatumHandlers {
    /// Resolve a sprite datum to (cast_lib, cast_member) for Flash bridge calls.
    fn resolve_sprite_flash_member(datum: &DatumRef) -> Result<Option<(i32, i32)>, ScriptError> {
        reserve_player_ref(|player| {
            let sprite_num = player.get_datum(datum).to_sprite_ref()?;
            let sprite = match player.movie.score.get_sprite(sprite_num) {
                Some(s) => s,
                None => return Ok(None),
            };
            match &sprite.member {
                Some(member_ref) => Ok(Some((member_ref.cast_lib, member_ref.cast_member))),
                None => Ok(None),
            }
        })
    }

    /// Returns true if the handler should be called via the async path.
    /// This returns true for:
    /// 1. Handlers found on the sprite's attached script instances
    /// 2. Any handler that isn't a built-in sync handler (to allow fallback to global handlers)
    pub fn has_async_handler(datum: &DatumRef, handler_name: &String) -> Result<bool, ScriptError> {
        // First check if it's a built-in sync handler (case-insensitive)
        let name_lower = handler_name.to_lowercase();
        let is_sync_handler = matches!(name_lower.as_str(),
            "intersects" | "getprop" | "getat" | "setat" | "getaprop" | "setaprop" | "pointtoword" | "pointtoline" |
            "gotoframe" | "callframe" | "stop" | "play" | "rewind" | "hold" |
            "getvariable" | "setvariable" | "callfunction" | "setcallback" |
            "hittest" | "getflashproperty" | "setflashproperty" | "telltarget" |
            "findlabel" | "flashtostage" | "stagetoflash" | "mapstagetomember" | "getpropref" |
            "addcamera" | "removecamera" | "cameracount"
        );
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
        let name_lower = handler_name.to_lowercase();
        match name_lower.as_str() {
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
            "cameracount" => {
                reserve_player_mut(|player| {
                    let sprite_num = player.get_datum(datum).to_sprite_ref()?;
                    let count = if let Some(sprite) = player.movie.score.get_sprite(sprite_num) {
                        1 + sprite.w3d_cameras.len() as i32
                    } else { 1 };
                    Ok(player.alloc_datum(Datum::Int(count)))
                })
            }
            "addcamera" => {
                reserve_player_mut(|player| {
                    let sprite_num = player.get_datum(datum).to_sprite_ref()?;
                    // addCamera(cameraRef, index)
                    // index 1 = primary camera, 2+ = additional cameras rendered on top
                    let cam_name = if !args.is_empty() {
                        match player.get_datum(&args[0]) {
                            Datum::Shockwave3dObjectRef(r) => r.name.clone(),
                            Datum::String(s) => s.clone(),
                            _ => String::new(),
                        }
                    } else { String::new() };
                    let index = if args.len() >= 2 {
                        player.get_datum(&args[1]).int_value().unwrap_or(1) as usize
                    } else { 1 };
                    if !cam_name.is_empty() {
                        let sprite = player.movie.score.get_sprite_mut(sprite_num as i16);
                        if index <= 1 {
                            // Index 1: set as primary camera
                            sprite.w3d_camera = Some(cam_name);
                        } else {
                            // Index 2+: add to extra cameras list
                            let extra_idx = index.saturating_sub(2);
                            if extra_idx >= sprite.w3d_cameras.len() {
                                sprite.w3d_cameras.push(cam_name);
                            } else {
                                sprite.w3d_cameras.insert(extra_idx, cam_name);
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                })
            }
            "removecamera" => {
                reserve_player_mut(|player| {
                    let sprite_num = player.get_datum(datum).to_sprite_ref()?;
                    let index = if !args.is_empty() {
                        player.get_datum(&args[0]).int_value().unwrap_or(1) as usize
                    } else { 1 };
                    let sprite = player.movie.score.get_sprite_mut(sprite_num as i16);
                    if index >= 2 {
                        let idx = index - 2;
                        if idx < sprite.w3d_cameras.len() {
                            sprite.w3d_cameras.remove(idx);
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                })
            }
            "getprop" => {
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
            "getat" | "getaprop" => {
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
            "getpropref" => {
                reserve_player_mut(|player| {
                    if args.is_empty() {
                        return Err(ScriptError::new(
                            "getPropRef requires at least 1 argument".to_string(),
                        ));
                    }

                    let sprite_num = player.get_datum(datum).to_sprite_ref()?;
                    let prop_name = player.get_datum(&args[0]).string_value()?;

                    // Get the property value (this handles scriptInstanceList cache etc.)
                    match crate::player::score::sprite_get_prop(
                        player,
                        sprite_num as i16,
                        &prop_name,
                    ) {
                        Ok(prop_datum) => {
                            let result = player.last_sprite_prop_ref.take()
                                .unwrap_or_else(|| player.alloc_datum(prop_datum));

                            // If there's a second argument, it's an index into the property
                            if args.len() > 1 {
                                let index = player.get_datum(&args[1]).int_value()?;
                                let list_datum = player.get_datum(&result).clone();
                                match list_datum {
                                    Datum::List(_, item_refs, _) => {
                                        if index < 1 || index as usize > item_refs.len() {
                                            return Err(ScriptError::new(format!(
                                                "getPropRef: index {} out of range for list of length {}",
                                                index, item_refs.len()
                                            )));
                                        }
                                        return Ok(item_refs[(index - 1) as usize].clone());
                                    }
                                    _ => {
                                        return Err(ScriptError::new(format!(
                                            "getPropRef: cannot index into {} with {}",
                                            player.get_datum(&result).type_str(), index
                                        )));
                                    }
                                }
                            }

                            return Ok(result);
                        }
                        Err(_) => {
                            // Not a built-in sprite property, try script instances
                        }
                    }

                    // Fall back to script instance properties
                    let sprite = player.movie.score.get_sprite(sprite_num);
                    if sprite.is_none() {
                        return Err(ScriptError::new(format!("Sprite {} not found", sprite_num)));
                    }
                    let instance_refs = sprite.unwrap().script_instance_list.clone();
                    for instance_ref in instance_refs {
                        if let Ok(result) = script_get_prop(player, &instance_ref, &prop_name) {
                            if args.len() > 1 {
                                return crate::player::handlers::types::TypeUtils::get_sub_prop(
                                    &result, &args[1], player,
                                );
                            }
                            return Ok(result);
                        }
                    }

                    Ok(DatumRef::Void)
                })
            }
            // setAt / setaProp: bracket assignment on sprite, e.g. sprite(9)[#pLevel] = value
            "setat" | "setaprop" => {
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
                    match crate::player::score::sprite_set_prop(
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
            "pointtoword" => {
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
            "pointtoline" => {
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
            // Flash (SWF) sprite methods
            "gotoframe" => {
                if let Some((cl, cm)) = Self::resolve_sprite_flash_member(datum)? {
                    let frame = reserve_player_ref(|player| {
                        if args.is_empty() { return Ok(1); }
                        player.get_datum(&args[0]).int_value()
                    })?;
                    ruffle_goto_frame(cl, cm, frame);
                }
                Ok(DatumRef::Void)
            }
            "callframe" => {
                if let Some((cl, cm)) = Self::resolve_sprite_flash_member(datum)? {
                    let frame = reserve_player_ref(|player| {
                        if args.is_empty() { return Ok(1); }
                        player.get_datum(&args[0]).int_value()
                    })?;
                    ruffle_call_frame(cl, cm, frame);
                }
                Ok(DatumRef::Void)
            }
            "stop" => {
                if let Some((cl, cm)) = Self::resolve_sprite_flash_member(datum)? {
                    ruffle_stop(cl, cm);
                }
                Ok(DatumRef::Void)
            }
            "play" => {
                if let Some((cl, cm)) = Self::resolve_sprite_flash_member(datum)? {
                    ruffle_play(cl, cm);
                }
                Ok(DatumRef::Void)
            }
            "rewind" | "hold" => {
                if let Some((cl, cm)) = Self::resolve_sprite_flash_member(datum)? {
                    ruffle_rewind(cl, cm);
                }
                Ok(DatumRef::Void)
            }
            "getvariable" => {
                if let Some((cl, cm)) = Self::resolve_sprite_flash_member(datum)? {
                    let (path, return_as_object) = reserve_player_ref(|player| {
                        if args.is_empty() { return Ok((String::new(), false)); }
                        let p = player.get_datum(&args[0]).string_value()?;
                        // Second arg: 0 = return as Flash object reference, otherwise string
                        let as_obj = if args.len() >= 2 {
                            player.get_datum(&args[1]).int_value().unwrap_or(1) == 0
                        } else {
                            false
                        };
                        Ok((p, as_obj))
                    })?;

                    if return_as_object {
                        // In Director, getVariable(path, 0) returns an object reference.
                        // But if the Flash variable is a simple string/number, Director
                        // returns the value directly. We check via GetVariable first:
                        // if it returns a JS string, return as Datum::String so that
                        // stringp() works. If it returns an object or undefined, return
                        // a FlashObjectRef for setCallback/call usage.
                        match ruffle_get_variable(cl, cm, &path) {
                            Ok(val) => {
                                if val.is_string() {
                                    // Simple string variable - return as string
                                    let s = val.as_string().unwrap();
                                    return reserve_player_mut(|player| {
                                        Ok(player.alloc_datum(Datum::String(s)))
                                    });
                                }
                                // Object or other type - fall through to FlashObjectRef
                            }
                            Err(_) => {} // Fall through to FlashObjectRef
                        }
                        // Return a FlashObjectRef for use with setCallback etc.
                        return reserve_player_mut(|player| {
                            use crate::director::lingo::datum::FlashObjectRef;
                            Ok(player.alloc_datum(Datum::FlashObjectRef(FlashObjectRef::from_path_with_member(&path, cl, cm))))
                        });
                    }

                    match ruffle_get_variable(cl, cm, &path) {
                        Ok(val) => {
                            if let Some(s) = val.as_string() {
                                return reserve_player_mut(|player| {
                                    Ok(player.alloc_datum(Datum::String(s)))
                                });
                            }
                        }
                        Err(e) => warn!("sprite.getVariable error: {:?}", e),
                    }
                }
                Ok(DatumRef::Void)
            }
            "setvariable" => {
                if let Some((cl, cm)) = Self::resolve_sprite_flash_member(datum)? {
                    let path = reserve_player_ref(|player| {
                        player.get_datum(&args[0]).string_value()
                    })?;
                    let value = reserve_player_ref(|player| {
                        player.get_datum(&args[1]).string_value()
                    })?;
                    if let Err(e) = ruffle_set_variable(cl, cm, &path, &value) {
                        warn!("sprite.setVariable error: {:?}", e);
                    }
                }
                Ok(DatumRef::Void)
            }
            "callfunction" => {
                if let Some((cl, cm)) = Self::resolve_sprite_flash_member(datum)? {
                    let path = reserve_player_ref(|player| {
                        player.get_datum(&args[0]).string_value()
                    })?;
                    let args_xml = if args.len() > 1 {
                        reserve_player_ref(|player| {
                            player.get_datum(&args[1]).string_value()
                        })?
                    } else {
                        String::new()
                    };
                    match ruffle_call_function(cl, cm, &path, &args_xml) {
                        Ok(val) => {
                            if let Some(s) = val.as_string() {
                                return reserve_player_mut(|player| {
                                    Ok(player.alloc_datum(Datum::String(s)))
                                });
                            }
                        }
                        Err(e) => warn!("sprite.callFunction error: {:?}", e),
                    }
                }
                Ok(DatumRef::Void)
            }
            "hittest" => {
                if let Some((cl, cm)) = Self::resolve_sprite_flash_member(datum)? {
                    let x = reserve_player_ref(|player| player.get_datum(&args[0]).int_value())?;
                    let y = reserve_player_ref(|player| player.get_datum(&args[1]).int_value())?;
                    let result = ruffle_hit_test(cl, cm, x as f64, y as f64);
                    return reserve_player_mut(|player| {
                        Ok(player.alloc_datum(Datum::Int(if result { 1 } else { 0 })))
                    });
                }
                Ok(DatumRef::Void)
            }
            "getflashproperty" => {
                if let Some((cl, cm)) = Self::resolve_sprite_flash_member(datum)? {
                    let target = reserve_player_ref(|player| player.get_datum(&args[0]).string_value())?;
                    let prop_num = reserve_player_ref(|player| player.get_datum(&args[1]).int_value())?;
                    match ruffle_get_flash_property(cl, cm, &target, prop_num) {
                        Ok(val) => {
                            if let Some(s) = val.as_string() {
                                return reserve_player_mut(|player| {
                                    Ok(player.alloc_datum(Datum::String(s)))
                                });
                            }
                        }
                        Err(e) => warn!("sprite.getFlashProperty error: {:?}", e),
                    }
                }
                Ok(DatumRef::Void)
            }
            "setflashproperty" => {
                if let Some((cl, cm)) = Self::resolve_sprite_flash_member(datum)? {
                    let target = reserve_player_ref(|player| player.get_datum(&args[0]).string_value())?;
                    let prop_num = reserve_player_ref(|player| player.get_datum(&args[1]).int_value())?;
                    let value = reserve_player_ref(|player| player.get_datum(&args[2]).string_value())?;
                    ruffle_set_flash_property(cl, cm, &target, prop_num, &value);
                }
                Ok(DatumRef::Void)
            }
            "setcallback" => {
                // setCallback(flashObject, flashMethod, lingoHandler, lingoTarget)
                if args.len() >= 3 {
                    reserve_player_mut(|player| {
                        let flash_object_path = match player.get_datum(&args[0]) {
                            Datum::FlashObjectRef(ref fo) => fo.path.clone(),
                            Datum::String(s) => s.clone(),
                            other => {
                                let type_name = other.type_str();
                                return Err(ScriptError::new(format!("setCallback: first argument must be a Flash object or string, got {}", type_name)));
                            }
                        };
                        let flash_method = player.get_datum(&args[1]).string_value()?;
                        let lingo_handler = player.get_datum(&args[2]).symbol_value().unwrap_or_else(|_| {
                            player.get_datum(&args[2]).string_value().unwrap_or_default()
                        });

                        // Translate _level0 to _root
                        let translated_path = if flash_object_path.starts_with("_level0") {
                            flash_object_path.replace("_level0", "_root")
                        } else {
                            flash_object_path.clone()
                        };

                        // Resolve the lingo target's cast_lib/cast_member for callback dispatch
                        // args[3] (if present) is the lingo target (a script instance)
                        let (cast_lib, cast_member) = if args.len() >= 4 {
                            match player.get_datum(&args[3]) {
                                Datum::ScriptInstanceRef(si_ref) => {
                                    use crate::player::allocator::ScriptInstanceAllocatorTrait;
                                    let si = player.allocator.get_script_instance(&si_ref);
                                    (si.script.cast_lib, si.script.cast_member)
                                }
                                _ => {
                                    // Fallback to sprite's Flash member
                                    let sprite_num = player.get_datum(datum).to_sprite_ref()?;
                                    if let Some(sprite) = player.movie.score.get_sprite(sprite_num) {
                                        if let Some(member_ref) = &sprite.member {
                                            (member_ref.cast_lib, member_ref.cast_member)
                                        } else {
                                            (0, 0)
                                        }
                                    } else {
                                        (0, 0)
                                    }
                                }
                            }
                        } else {
                            // No target specified, use sprite's Flash member
                            let sprite_num = player.get_datum(datum).to_sprite_ref()?;
                            if let Some(sprite) = player.movie.score.get_sprite(sprite_num) {
                                if let Some(member_ref) = &sprite.member {
                                    (member_ref.cast_lib, member_ref.cast_member)
                                } else {
                                    (0, 0)
                                }
                            } else {
                                (0, 0)
                            }
                        };

                        // Get Flash member's cast_lib/cast_member from the flash object ref or sprite
                        let (flash_cl, flash_cm) = match player.get_datum(&args[0]) {
                            Datum::FlashObjectRef(ref fo) => (fo.cast_lib, fo.cast_member),
                            _ => {
                                // Fallback: get from sprite's member
                                let sprite_num = player.get_datum(datum).to_sprite_ref().unwrap_or(0);
                                if let Some(sprite) = player.movie.score.get_sprite(sprite_num) {
                                    if let Some(member_ref) = &sprite.member {
                                        (member_ref.cast_lib, member_ref.cast_member)
                                    } else { (0, 0) }
                                } else { (0, 0) }
                            }
                        };

                        // Call the JS bridge to register the callback in Ruffle
                        if let Some(window) = web_sys::window() {
                            if let Ok(func) = js_sys::Reflect::get(&window, &"ruffleRegisterLingoCallback".into()) {
                                if func.is_function() {
                                    let func = js_sys::Function::from(func);
                                    let js_args = js_sys::Array::new();
                                    js_args.push(&translated_path.clone().into());
                                    js_args.push(&flash_method.clone().into());
                                    js_args.push(&cast_lib.into());
                                    js_args.push(&cast_member.into());
                                    js_args.push(&lingo_handler.into());
                                    js_args.push(&flash_cl.into());
                                    js_args.push(&flash_cm.into());
                                    let _ = func.apply(&JsValue::NULL, &js_args);
                                }
                            }
                        }

                        Ok(player.alloc_datum(Datum::Int(1)))
                    })
                } else {
                    Ok(DatumRef::Void)
                }
            }
            "mapstagetomember" => {
                reserve_player_mut(|player| {
                    if args.is_empty() {
                        return Err(ScriptError::new(
                            "mapStageToMember requires 1 argument (point)".to_string(),
                        ));
                    }
                    let sprite_num = player.get_datum(datum).to_sprite_ref()?;
                    let stage_point = player.get_datum(&args[0]).clone();
                    let (stage_x, stage_y) = match &stage_point {
                        Datum::Point(refs) => {
                            let x = player.get_datum(&refs[0]).int_value()?;
                            let y = player.get_datum(&refs[1]).int_value()?;
                            (x as i32, y as i32)
                        }
                        _ => return Err(ScriptError::new(
                            "mapStageToMember requires a point argument".to_string(),
                        )),
                    };

                    let sprite = player.movie.score.get_sprite(sprite_num);
                    if sprite.is_none() {
                        return Ok(DatumRef::Void);
                    }
                    let sprite = sprite.unwrap();

                    let rect = get_concrete_sprite_rect(player, sprite);

                    // Convert stage coords to member-local coords
                    let member = sprite.member.as_ref()
                        .and_then(|mr| player.movie.cast_manager.find_member_by_ref(mr));

                    let (member_w, member_h) = if let Some(m) = &member {
                        match &m.member_type {
                            CastMemberType::Bitmap(bm) => (bm.info.width as i32, bm.info.height as i32),
                            _ => (rect.right - rect.left, rect.bottom - rect.top),
                        }
                    } else {
                        (rect.right - rect.left, rect.bottom - rect.top)
                    };

                    let sprite_w = rect.right - rect.left;
                    let sprite_h = rect.bottom - rect.top;

                    if sprite_w == 0 || sprite_h == 0 {
                        return Ok(DatumRef::Void);
                    }

                    // Map stage point to member coordinates, accounting for scaling
                    let local_x = (stage_x - rect.left) * member_w / sprite_w;
                    let local_y = (stage_y - rect.top) * member_h / sprite_h;

                    let x_ref = player.alloc_datum(Datum::Int(local_x));
                    let y_ref = player.alloc_datum(Datum::Int(local_y));
                    Ok(player.alloc_datum(Datum::Point([x_ref, y_ref])))
                })
            }
            "telltarget" | "findlabel" | "flashtostage" | "stagetoflash" => {
                warn!("Flash sprite method '{}' called but not yet implemented", handler_name);
                Ok(DatumRef::Void)
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

        // In Director, calling a handler on a sprite that doesn't handle it
        // is silently ignored and returns void.
        Ok(DatumRef::Void)
    }
}
