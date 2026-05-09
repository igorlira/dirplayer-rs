use std::collections::VecDeque;

use crate::{
    director::lingo::datum::{Datum, FlashObjectRef},
    player::{
        handlers::datum_handlers::date::DateObject,
        reserve_player_mut,
        DatumRef,
        ScriptError,
    }
};
use wasm_bindgen::prelude::*;
use log::warn;

// JS bridge names use the `dirplayer_` prefix so this fork's globals don't
// collide with stock Ruffle if both are loaded on the same page (e.g. via a
// browser extension). Matching JS-side definitions live in
// src/services/flashPlayerManager.ts::initFlashBridge.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "dirplayer_ruffleGetVariable", catch)]
    fn ruffle_get_variable_global(cast_lib: i32, cast_member: i32, path: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = "dirplayer_ruffleSetVariable", catch)]
    fn ruffle_set_variable_global(cast_lib: i32, cast_member: i32, path: &str, value: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = "dirplayer_ruffleCallFunction", catch)]
    fn ruffle_call_function_global(cast_lib: i32, cast_member: i32, path: &str, args_xml: &str) -> Result<JsValue, JsValue>;

}

// Global counter for dynamic Flash objects.
thread_local! {
    pub static FLASH_OBJECT_COUNTER: std::cell::Cell<i32> = std::cell::Cell::new(0);
}

pub struct FlashObjectDatumHandlers {}

impl FlashObjectDatumHandlers {
    pub fn get_prop(obj_ref: &DatumRef, prop_name: &str) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let obj_datum = player.get_datum(obj_ref);

            if let Datum::FlashObjectRef(flash_ref) = obj_datum {
                let full_path = format!("{}.{}", flash_ref.path, prop_name);
                let cl = flash_ref.cast_lib;
                let cm = flash_ref.cast_member;

                match ruffle_get_variable_global(cl, cm, &full_path) {
                    Ok(result) => {
                        if result.is_null() || result.is_undefined() {
                            Ok(player.alloc_datum(Datum::Void))
                        } else if let Some(s) = result.as_string() {
                            Ok(player.alloc_datum(Datum::String(s)))
                        } else if let Some(n) = result.as_f64() {
                            if n.fract() == 0.0 && n >= i32::MIN as f64 && n <= i32::MAX as f64 {
                                Ok(player.alloc_datum(Datum::Int(n as i32)))
                            } else {
                                Ok(player.alloc_datum(Datum::Float(n)))
                            }
                        } else if let Some(b) = result.as_bool() {
                            Ok(player.alloc_datum(Datum::Int(if b { 1 } else { 0 })))
                        } else {
                            let new_flash_ref = FlashObjectRef::from_path_with_member(&full_path, cl, cm);
                            Ok(player.alloc_datum(Datum::FlashObjectRef(new_flash_ref)))
                        }
                    }
                    Err(_) => {
                        let new_flash_ref = FlashObjectRef::from_path_with_member(&full_path, cl, cm);
                        Ok(player.alloc_datum(Datum::FlashObjectRef(new_flash_ref)))
                    }
                }
            } else {
                Err(ScriptError::new("Expected FlashObjectRef, got different datum type".to_string()))
            }
        })
    }

    pub fn call(
        datum: &DatumRef,
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let flash_ref = {
                let datum_value = player.get_datum(datum);
                if let Some(flash_ref) = datum_value.as_flash_object() {
                    flash_ref.clone()
                } else {
                    return Err(ScriptError::new("Not a Flash object".to_string()));
                }
            };

            let method_path = format!("{}.{}", flash_ref.path, handler_name);

            // Convert Lingo arguments to a JSON array string for the bridge
            let mut js_args_parts = Vec::new();
            for arg_ref in args {
                let js_str = convert_lingo_datum_to_json_ref(player, arg_ref);
                js_args_parts.push(js_str);
            }
            let args_str = format!("[{}]", js_args_parts.join(","));

            match ruffle_call_function_global(flash_ref.cast_lib, flash_ref.cast_member, &method_path, &args_str) {
                Ok(result) => {
                    // Special handling for getGatewayConnection
                    if handler_name == "getGatewayConnection" {
                        let gateway_ref = FlashObjectRef::from_path_with_member("_level0.oGatewayConnection", flash_ref.cast_lib, flash_ref.cast_member);
                        return Ok(player.alloc_datum(Datum::FlashObjectRef(gateway_ref)));
                    }

                    convert_js_result_to_lingo_datum(player, result, &flash_ref.path, flash_ref.cast_lib, flash_ref.cast_member)
                }
                Err(e) => {
                    warn!(
                        "FlashObject.call WASM ERROR {}: {:?}", method_path, e
                    );
                    Ok(player.alloc_datum(Datum::Void))
                }
            }
        })
    }

    pub fn set_prop(
        datum: &DatumRef,
        prop_name: &str,
        value: &Datum,
    ) -> Result<(), ScriptError> {
        reserve_player_mut(|player| {
            let flash_ref = {
                let datum_value = player.get_datum(datum);
                if let Some(flash_ref) = datum_value.as_flash_object() {
                    flash_ref.clone()
                } else {
                    return Err(ScriptError::new("Not a Flash object".to_string()));
                }
            };

            let prop_path = format!("{}.{}", flash_ref.path, prop_name);
            let value_str = match value {
                Datum::Int(i) => i.to_string(),
                Datum::Float(f) => f.to_string(),
                Datum::String(s) => s.clone(),
                Datum::Void => "null".to_string(),
                _ => "null".to_string(),
            };

            match ruffle_set_variable_global(flash_ref.cast_lib, flash_ref.cast_member, &prop_path, &value_str) {
                Ok(_) => Ok(()),
                Err(e) => {
                    warn!("Failed to set Flash property {}: {:?}", prop_path, e);
                    Err(ScriptError::new(format!("Failed to set Flash property {}.{}", flash_ref.path, prop_name)))
                }
            }
        })
    }

}

fn convert_lingo_datum_to_json_ref(player: &crate::player::DirPlayer, datum_ref: &DatumRef) -> String {
    let datum = player.get_datum(datum_ref);
    convert_lingo_datum_to_json_inner(player, datum)
}

fn convert_lingo_datum_to_json_inner(player: &crate::player::DirPlayer, datum: &Datum) -> String {
    match datum {
        Datum::Int(i) => i.to_string(),
        Datum::Float(f) => f.to_string(),
        Datum::String(s) => {
            // JSON-escape the string
            let escaped = s.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\t', "\\t");
            format!("\"{}\"", escaped)
        },
        Datum::Symbol(s) => format!("\"#{}\"", s),
        // Match Adobe's Flash Asset Xtra: Lingo Void crosses into AS as the
        // numeric value 0, not null. CS's outgoing AV packets need this:
        //   - dance/stand send `(VOID, VOID, VOID, VOID, "dnc")` and the
        //     server requires `d="0"` — getUpdateAvatarNode gates `d` on
        //     `iDirection != null && != "" && toString() != "NaN"`. Null gets
        //     dropped, but 0 passes (0 != null is true, 0 != "" is true under
        //     Ruffle's equality, "0" != "NaN").
        //   - faceAvatar sends `(VOID, VOID, VOID, iDir, VOID)`. With number 0
        //     for the action slot the `typeof sAction == "string"` gate fails,
        //     so we don't pollute the packet with `act="0"`.
        //   - position args have an extra `!= 0` gate that filters 0 out, so
        //     x/y/z stay omitted when no movement happened.
        Datum::Void => "0".to_string(),
        Datum::FlashObjectRef(flash_ref) => {
            format!("\"__ruffle_path:{}\"", flash_ref.path)
        },
        Datum::List(_, items, _) => {
            let parts: Vec<String> = items.iter()
                .map(|item_ref| convert_lingo_datum_to_json_ref(player, item_ref))
                .collect();
            format!("[{}]", parts.join(","))
        },
        _ => "null".to_string(),
    }
}

fn convert_js_result_to_lingo_datum(
    player: &mut crate::player::DirPlayer,
    result: JsValue,
    context_path: &str,
    cast_lib: i32,
    cast_member: i32,
) -> Result<DatumRef, ScriptError> {
    if result.is_null() || result.is_undefined() {
        return Ok(player.alloc_datum(Datum::Void));
    }

    if let Some(s) = result.as_string() {
        return Ok(player.alloc_datum(Datum::String(s)));
    }

    if let Some(n) = result.as_f64() {
        if n.fract() == 0.0 && n >= i32::MIN as f64 && n <= i32::MAX as f64 {
            return Ok(player.alloc_datum(Datum::Int(n as i32)));
        } else {
            return Ok(player.alloc_datum(Datum::Float(n)));
        }
    }

    if let Some(b) = result.as_bool() {
        return Ok(player.alloc_datum(Datum::Int(if b { 1 } else { 0 })));
    }

    // AS Date instances arrive as JS Date — materialize as a Lingo DateRef
    // so subsequent .getYear()/.getMonth()/.getDate() etc. dispatch through
    // DateDatumHandlers (case-insensitive) instead of round-tripping back into
    // Ruffle (case-sensitive AS, which would fail).
    if result.is_instance_of::<js_sys::Date>() {
        let date: js_sys::Date = result.unchecked_into();
        let ts = date.get_time() as i64;
        let date_id = player.allocator.get_free_script_instance_id();
        let date_obj = DateObject::from_timestamp(date_id, ts);
        player.date_objects.insert(date_id, date_obj);
        return Ok(player.alloc_datum(Datum::DateRef(date_id)));
    }

    // Check for arrays before generic objects (arrays are objects in JS).
    //
    // BUT: if the array is marked with `__dirplayer_stored_path` it means
    // our Ruffle fork is keeping a live AS reference for it — typically
    // because Lingo intends to push into it and pass it back to AS
    // (e.g. Coke Studios' Studio.sendSendMessage flow:
    //   faToScreenNameList = me.foRoom.getArray()
    //   faToScreenNameList.push(name)
    //   me.foRoom.sendSendMessage(msg, faToScreenNameList)).
    // Materializing it as a Lingo Datum::List severs the AS reference, so
    // .push() mutates a local list and the AS array stays empty. Keep it as
    // a FlashObjectRef instead so .push round-trips back to AS.
    if js_sys::Array::is_array(&result) {
        let stored_path = js_sys::Reflect::get(&result, &JsValue::from_str("__dirplayer_stored_path"))
            .ok()
            .and_then(|v| v.as_string());
        if let Some(path) = stored_path {
            let flash_ref = FlashObjectRef::from_path_with_member(&path, cast_lib, cast_member);
            return Ok(player.alloc_datum(Datum::FlashObjectRef(flash_ref)));
        }
        let array = js_sys::Array::from(&result);
        let mut items = VecDeque::new();
        for i in 0..array.length() {
            let item = array.get(i);
            let item_ref = convert_js_result_to_lingo_datum(player, item, context_path, cast_lib, cast_member)?;
            items.push_back(item_ref);
        }
        return Ok(player.alloc_datum(Datum::List(
            crate::director::lingo::datum::DatumType::XmlChildNodes, // 0-based indexing for Flash arrays
            items,
            false,
        )));
    }

    if result.is_object() {
        // Check if Ruffle stored the object and included the path
        let stored_path = js_sys::Reflect::get(&result, &JsValue::from_str("__dirplayer_stored_path"))
            .ok()
            .and_then(|v| v.as_string());

        if let Some(path) = stored_path {
            let flash_ref = FlashObjectRef::from_path_with_member(&path, cast_lib, cast_member);
            return Ok(player.alloc_datum(Datum::FlashObjectRef(flash_ref)));
        }

        // Fallback: generate a path (won't be resolvable in Flash)
        let instance_id = FLASH_OBJECT_COUNTER.with(|c| {
            let current = c.get();
            c.set(current + 1);
            current + 1
        });
        let object_path = format!("_level0.__dirplayer_ref_{}", instance_id);
        warn!("FlashObject: no stored path, using fallback {}", object_path);
        let flash_ref = FlashObjectRef::from_path_with_member(&object_path, cast_lib, cast_member);
        return Ok(player.alloc_datum(Datum::FlashObjectRef(flash_ref)));
    }

    Ok(player.alloc_datum(Datum::Void))
}
