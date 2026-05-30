use log::{warn, debug};
use wasm_bindgen::prelude::*;

use super::{
    cast::CastHandlers,
    datum_handlers::{
        bitmap::BitmapDatumHandlers,
        list_handlers::ListDatumHandlers,
        player_call_datum_handler,
        point::PointDatumHandlers,
        prop_list::PropListDatumHandlers,
        script_instance::{ScriptInstanceDatumHandlers, ScriptInstanceUtils},
        sound_channel::SoundChannelDatumHandlers,
    },
    movie::MovieHandlers,
    net::NetHandlers,
    string::StringHandlers,
    types::TypeHandlers,
};
use std::collections::{HashMap, VecDeque};
use rand::Rng;

use crate::{
    director::lingo::datum::{Datum, DatumType, datum_bool},
    js_api::JsApi,
    player::{
        DatumRef, DirPlayer, ScriptError, ScriptErrorCode, bitmap::bitmap::{Bitmap, PaletteRef, get_system_default_palette}, datum_formatting::{format_concrete_datum, format_datum}, geometry::IntRect, handlers::datum_handlers::xml::XmlHelper, keyboard_map, player_alloc_datum, player_call_script_handler, reserve_player_mut, reserve_player_ref, score::get_concrete_sprite_rect, script_ref::ScriptInstanceRef, symbols::{builtin::BuiltInSymbol, symbol::Symbol}, trace_output, xtra::manager::call_xtra_instance_handler
    },
};

// JS bridge names use the `dirplayer_` prefix so this fork's globals don't
// collide with stock Ruffle if both are loaded on the same page (e.g. via a
// browser extension). Matching JS-side definitions live in
// src/services/flashPlayerManager.ts::initFlashBridge.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "dirplayer_ruffleGetVariable", catch)]
    fn ruffle_get_variable(sprite_num: i32, path: &str) -> Result<JsValue, JsValue>;

    // Per-sprite Flash bridge — each Flash sprite has its own Ruffle
    // instance keyed by sprite_num.
    #[wasm_bindgen(js_name = "dirplayer_ruffleSetVariable", catch)]
    fn ruffle_set_variable(sprite_num: i32, path: &str, value: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = "dirplayer_ruffleCallFunction", catch)]
    fn ruffle_call_function(sprite_num: i32, path: &str, args_xml: &str) -> Result<JsValue, JsValue>;

    /// Always pass a string here — JS-side parses int vs label.
    /// Director's gotoFrame accepts both `gotoFrame(5)` (numeric) and
    /// `gotoFrame("warm0")` (label); routing labels through `int_value()`
    /// silently parses to 0 and breaks animation.
    #[wasm_bindgen(js_name = "dirplayer_ruffleGoToFrame")]
    fn ruffle_goto_frame(sprite_num: i32, frame_or_label: &str);

    #[wasm_bindgen(js_name = "dirplayer_ruffleStop")]
    fn ruffle_stop(sprite_num: i32);

    #[wasm_bindgen(js_name = "dirplayer_rufflePlay")]
    fn ruffle_play(sprite_num: i32);

    #[wasm_bindgen(js_name = "dirplayer_ruffleRewind")]
    fn ruffle_rewind(sprite_num: i32);

    #[wasm_bindgen(js_name = "dirplayer_ruffleIsPlaying")]
    fn ruffle_is_playing(sprite_num: i32) -> bool;

    #[wasm_bindgen(js_name = "dirplayer_ruffleGetFrameCount")]
    fn ruffle_get_frame_count(sprite_num: i32) -> i32;

    #[wasm_bindgen(js_name = "dirplayer_ruffleGetCurrentFrame")]
    fn ruffle_get_current_frame(sprite_num: i32) -> i32;

    #[wasm_bindgen(js_name = "dirplayer_ruffleCallFrame")]
    fn ruffle_call_frame(sprite_num: i32, frame: i32);

    #[wasm_bindgen(js_name = "dirplayer_ruffleHitTest")]
    fn ruffle_hit_test(sprite_num: i32, x: f64, y: f64) -> bool;

    #[wasm_bindgen(js_name = "dirplayer_ruffleGetFlashProperty", catch)]
    fn ruffle_get_flash_property(sprite_num: i32, target: &str, prop_num: i32) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = "dirplayer_ruffleSetFlashProperty")]
    fn ruffle_set_flash_property(sprite_num: i32, target: &str, prop_num: i32, value: &str);
}

pub struct BuiltInHandlerManager {}

impl BuiltInHandlerManager {
    /// Resolve a sprite/integer datum to its sprite number for Flash bridge
    /// calls. Per-sprite Ruffle instances mean sprite_num is the lookup
    /// key; cast_lib/cast_member are returned alongside for callers that
    /// still need them (e.g. building FlashObjectRefs).
    fn resolve_flash_member(datum_ref: &DatumRef) -> Result<Option<(i32, i32, i32)>, ScriptError> {
        reserve_player_ref(|player| {
            let datum = player.get_datum(datum_ref);
            let sprite_num = match datum {
                Datum::SpriteRef(n) => *n,
                Datum::Int(n) => *n as i16,
                _ => return Ok(None),
            };
            let sprite = match player.movie.score.get_sprite(sprite_num) {
                Some(s) => s,
                None => return Ok(None),
            };
            match &sprite.member {
                Some(member_ref) => Ok(Some((
                    sprite_num as i32,
                    member_ref.cast_lib as i32,
                    member_ref.cast_member as i32,
                ))),
                None => Ok(None),
            }
        })
    }

    /// Like `resolve_flash_member`, but only returns Some when the
    /// sprite's member is actually a Flash cast member. Used by the
    /// `stop` / `play` / `rewind` Lingo built-ins so we route those to
    /// the Ruffle bridge only for Flash sprites — `stop sound 1`,
    /// `stop(member …)` etc. continue to fall through to the existing
    /// SWA no-op (which is correct for those operands in the web port).
    fn resolve_flash_sprite_strict(datum_ref: &DatumRef) -> Result<Option<i32>, ScriptError> {
        use crate::player::cast_member::CastMemberType;
        reserve_player_ref(|player| {
            let datum = player.get_datum(datum_ref);
            let sprite_num = match datum {
                Datum::SpriteRef(n) => *n,
                Datum::Int(n) => *n as i16,
                _ => return Ok(None),
            };
            let sprite = match player.movie.score.get_sprite(sprite_num) {
                Some(s) => s,
                None => return Ok(None),
            };
            let member_ref = match &sprite.member {
                Some(m) => m,
                None => return Ok(None),
            };
            let member = match player.movie.cast_manager.find_member_by_ref(member_ref) {
                Some(m) => m,
                None => return Ok(None),
            };
            if matches!(member.member_type, CastMemberType::Flash(_)) {
                Ok(Some(sprite_num as i32))
            } else {
                Ok(None)
            }
        })
    }

    fn param(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_ref(|player| {
            let param_number = player.get_datum(&args[0]).int_value()?;
            let scope_ref = player.current_scope_ref();
            let scope = player.scopes.get(scope_ref).unwrap();
            Ok(scope.args[(param_number - 1) as usize].clone())
        })
    }

    fn count(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let obj = player.get_datum(&args[0]);
            match obj {
                Datum::List(_, list, ..) => Ok(player.alloc_datum(Datum::Int(list.len() as i32))),
                Datum::PropList(prop_list, ..) => {
                    Ok(player.alloc_datum(Datum::Int(prop_list.len() as i32)))
                }
                // Director treats count(VOID) as 0 - this allows "repeat with i in VOID" to not iterate
                Datum::Void => Ok(player.alloc_datum(Datum::Int(0))),
                _ => {
                    Err(ScriptError::new(format!(
                        "Cannot get count of non-list (type: {})",
                        obj.type_str()
                    )))
                }
            }
        })
    }

    fn forward_bitmap_handler(handler_name: &str, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        let Some(bitmap_ref) = args.first() else {
            return Err(ScriptError::new(format!(
                "{} requires an image argument",
                handler_name
            )));
        };
        let handler_args = args[1..].to_vec();
        BitmapDatumHandlers::call(bitmap_ref, crate::player::symbols::symbol::Symbol::from_str(handler_name), &handler_args)
    }

    fn get_pos_global(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        // getPos(list, value) - find position of value in list.
        // Uses the looser membership equality so Symbol/String pairs with
        // matching text match each other, mirroring Director — verified with
        // `put getPos([#foo, #bar], "foo")` returning 1 in Director 11.5.
        use crate::player::compare::datum_equals_member;
        reserve_player_mut(|player| {
            let list_datum = player.get_datum(&args[0]);
            let search_ref = &args[1];
            match list_datum {
                Datum::List(_, items, _) => {
                    let items = items.clone();
                    for (i, item_ref) in items.iter().enumerate() {
                        let item = player.get_datum(item_ref);
                        let search = player.get_datum(search_ref);
                        if datum_equals_member(item, search, &player.allocator)? {
                            return Ok(player.alloc_datum(Datum::Int((i + 1) as i32)));
                        }
                    }
                    Ok(player.alloc_datum(Datum::Int(0)))
                }
                Datum::PropList(pairs, _) => {
                    let pairs = pairs.clone();
                    for (i, (_, val_ref)) in pairs.iter().enumerate() {
                        let val = player.get_datum(val_ref);
                        let search = player.get_datum(search_ref);
                        if datum_equals_member(val, search, &player.allocator)? {
                            return Ok(player.alloc_datum(Datum::Int((i + 1) as i32)));
                        }
                    }
                    Ok(player.alloc_datum(Datum::Int(0)))
                }
                _ => Err(ScriptError::new(format!("getPos: not a list")))
            }
        })
    }

    fn get_at(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        // Check if it's a FlashObjectRef first (needs special handling to avoid nested locks)
        let flash_info = reserve_player_ref(|player| {
            let obj = player.get_datum(&args[0]);
            if let Datum::FlashObjectRef(flash_ref) = obj {
                Some((flash_ref.clone(), player.get_datum(&args[1]).int_value().unwrap_or(0)))
            } else {
                None
            }
        });

        if let Some((flash_ref, position)) = flash_info {
            // Flash arrays use 0-based indexing
            let prop_name = position.to_string();
            let prop_ref = reserve_player_mut(|player| {
                player.alloc_datum(Datum::FlashObjectRef(flash_ref))
            });
            return crate::player::handlers::datum_handlers::flash_object::FlashObjectDatumHandlers::get_prop(&prop_ref, &prop_name);
        }

        reserve_player_mut(|player| {
            let obj = player.get_datum(&args[0]);
            let position = player.get_datum(&args[1]).int_value()?;
            let index = (position - 1) as usize;

            debug!(
                "getAt: list={}, index={}",
                format_concrete_datum(obj, player),
                position
            );

            match obj {
                Datum::Point(vals, flags) => {
                    if index >= 2 {
                        return Err(ScriptError::new(format!(
                            "point index {} out of bounds",
                            position
                        )));
                    }
                    Ok(player.alloc_datum(Datum::inline_component_to_datum(vals[index], Datum::inline_is_float(*flags, index))))
                }

                Datum::Rect(vals, flags) => {
                    if index >= 4 {
                        return Err(ScriptError::new(format!(
                            "rect index {} out of bounds",
                            position
                        )));
                    }
                    Ok(player.alloc_datum(Datum::inline_component_to_datum(vals[index], Datum::inline_is_float(*flags, index))))
                }
                Datum::List(datum_type, list, ..) => {
                    let index = if *datum_type == crate::director::lingo::datum::DatumType::XmlChildNodes {
                        position as usize // 0-based for Flash/XML arrays
                    } else {
                        (position - 1) as usize // 1-based for Lingo lists
                    };
                    if index >= list.len() {
                        return Err(ScriptError::new(format!(
                            "Index {} out of bounds for list of length {}",
                            position,
                            list.len()
                        )));
                    }
                    let result = list[index].clone();

                    debug!(
                        "getAt returned: {}",
                        format_concrete_datum(player.get_datum(&result), player)
                    );

                    Ok(result)
                }
                Datum::PropList(prop_list, ..) => {
                    let index = (position - 1) as usize;
                    if index >= prop_list.len() {
                        return Err(ScriptError::new(format!(
                            "Index {} out of bounds for proplist of length {}",
                            position,
                            prop_list.len()
                        )));
                    }
                    let result = prop_list[index].1.clone();

                    debug!(
                        "getAt returned (from PropList): {}",
                        format_concrete_datum(player.get_datum(&result), player)
                    );

                    Ok(result)
                }
                _ => {
                    Err(ScriptError::new(format!(
                        "Cannot getAt of non-list (type: {})",
                        obj.type_str()
                    )))
                }
            }
        })
    }

    fn get_last(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let obj = player.get_datum(&args[0]);
            match obj {
                Datum::List(_, list, ..) => {
                    Ok(list.back().cloned().unwrap_or(DatumRef::Void))
                }
                Datum::PropList(prop_list, ..) => {
                    Ok(prop_list.back().map(|(_, v)| v.clone()).unwrap_or(DatumRef::Void))
                }
                _ => {
                    Err(ScriptError::new(format!(
                        "Cannot getLast of non-list (type: {})",
                        obj.type_str()
                    )))
                }
            }
        })
    }

    fn set_at(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let list_ref = &args[0];
            let position = player.get_datum(&args[1]).int_value()?;
            let new_value = args[2].clone();
            let is_zero_based = matches!(player.get_datum(list_ref), Datum::List(crate::director::lingo::datum::DatumType::XmlChildNodes, ..));
            let index = if is_zero_based { position as usize } else { (position - 1) as usize };
            
            let list_datum = player.get_datum(list_ref);
            debug!(
                "setAt: list={}, index={}, new_value={}", 
                format_concrete_datum(list_datum, player),
                position,
                format_concrete_datum(player.get_datum(&new_value), player)
            );
            
            // Validate the new_value type BEFORE taking mutable borrow
            let new_value_datum = player.get_datum(&new_value).clone();
            
            // Now take the mutable borrow
            let list_datum = player.get_datum_mut(list_ref);
            match list_datum {
                Datum::Point(vals, flags) => {
                    if index >= 2 {
                        return Err(ScriptError::new(format!(
                            "point index {} out of bounds",
                            position
                        )));
                    }

                    let (component_val, is_float) = Datum::datum_to_inline_component(&new_value_datum)?;
                    vals[index] = component_val;
                    Datum::inline_set_float(flags, index, is_float);

                    Ok(())
                }
                Datum::Rect(vals, flags) => {
                    if index >= 4 {
                        return Err(ScriptError::new(format!(
                            "rect index {} out of bounds",
                            position
                        )));
                    }

                    let (component_val, is_float) = Datum::datum_to_inline_component(&new_value_datum)?;
                    vals[index] = component_val;
                    Datum::inline_set_float(flags, index, is_float);

                    Ok(())
                }
                Datum::List(_, list, ..) => {
                    if index < list.len() {
                        list[index] = new_value;
                        
                        debug!(
                            "setAt complete: list is now {}", 
                            format_concrete_datum(
                                &Datum::List(DatumType::List, list.clone(), false),
                                player
                            )
                        );
                        
                        Ok(())
                    } else {
                        Err(ScriptError::new(format!("Index {} out of bounds", position)))
                    }
                }
                Datum::PropList(prop_list, ..) => {
                    if index < prop_list.len() {
                        prop_list[index].1 = new_value;
                        Ok(())
                    } else {
                        Err(ScriptError::new(format!("Index {} out of bounds", position)))
                    }
                }
                _ => Err(ScriptError::new(format!(
                    "Cannot setAt of type {} (must be list, proplist, point, or rect)", 
                    list_datum.type_str()
                ))),
            }
        })?;
        Ok(DatumRef::Void)
    }

    pub fn put(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_ref(|player| {
            if args.is_empty() {
                trace_output(player, "--");
                return Ok(());
            }
            
            // Format the first argument to determine output
            let first_arg = player.get_datum(&args[0]);
            let output = if args.len() == 1 {
                // Single argument
                Self::format_for_put(first_arg, player)
            } else {
                // Multiple arguments - join with spaces
                let parts: Vec<String> = args
                    .iter()
                    .map(|arg| {
                        let datum = player.get_datum(arg);
                        // For multi-arg put, use string representation
                        match datum {
                            Datum::String(s) => s.clone(),
                            _ => format_concrete_datum(datum, player),
                        }
                    })
                    .collect();
                parts.join(" ")
            };
            
            trace_output(player, &format!("-- {}", output));
            Ok(())
        })?;
        Ok(DatumRef::Void)
    }

    fn format_for_put(datum: &Datum, player: &DirPlayer) -> String {
        match datum {
            // Strings are output with quotes
            Datum::String(s) => format!("\"{}\"", s),
            
            // Numbers are output without quotes
            Datum::Int(i) => i.to_string(),
            
            // Symbols are output with # prefix
            Datum::Symbol(s) => format!("#{}", s),
            
            // Void outputs as <Void>
            Datum::Void | Datum::Null => "<Void>".to_string(),
            
            // Lists
            Datum::List(_, list, _) => {
                let items: Vec<String> = list
                    .iter()
                    .map(|r| Self::format_for_put(player.get_datum(r), player))
                    .collect();
                format!("[{}]", items.join(", "))
            },
            
            // Everything else uses default formatting
            _ => format_concrete_datum(datum, player),
        }
    }

    pub fn inspect(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        if args.len() != 1 {
            return Err(ScriptError::new(
                "inspect requires exactly 1 argument".to_string(),
            ));
        }
        reserve_player_mut(|player| {
            let datum = player.get_datum(&args[0]);
            match datum {
                Datum::BitmapRef(bitmap_ref) => {
                    let src = player
                        .bitmap_manager
                        .get_bitmap(*bitmap_ref)
                        .ok_or_else(|| ScriptError::new("Invalid bitmap reference".to_string()))?;

                    let w = src.width;
                    let h = src.height;
                    let palettes = player.movie.cast_manager.palettes();
                    let mut dest = Bitmap::new(w, h, 32, 32, 0, PaletteRef::BuiltIn(get_system_default_palette()));
                    let rect = IntRect::from(0, 0, w as i32, h as i32);
                    dest.copy_pixels(&palettes, src, rect.clone(), rect, &HashMap::new(), None);

                    JsApi::dispatch_debug_bitmap(w as u32, h as u32, &dest.data);
                }
                Datum::List(..) | Datum::PropList(..) | Datum::ScriptInstanceRef(..) => {
                    JsApi::dispatch_debug_datum(&args[0], player);
                    player.debug_datum_refs.push(args[0].clone());
                }
                _ => {
                    return Err(ScriptError::new(format!(
                        "inspect does not support type: {}",
                        datum.type_str()
                    )));
                }
            }
            Ok(())
        })?;
        Ok(DatumRef::Void)
    }

    fn clear_globals(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            player.globals.clear();
            player.initialize_globals();
            Ok(DatumRef::Void)
        })
    }

    fn random(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let max = player.get_datum(&args[0]).int_value()?;
            if max <= 0 {
                return Ok(player.alloc_datum(Datum::Int(0)));
            }
            
            // Director's random(n) returns a value from 1 to n (inclusive)
            let random_int = match player.movie.next_random_int(max) {
                Some(value) => value,
                None => {
                    player.rng.random_range(1..=max)
                }
            };
            
            Ok(player.alloc_datum(Datum::Int(random_int)))
        })
    }

    fn bit_and(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let a = player.get_datum(&args[0]).int_value()?;
            let b = player.get_datum(&args[1]).int_value()?;
            Ok(player.alloc_datum(Datum::Int(a & b)))
        })
    }

    fn bit_or(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let a = player.get_datum(&args[0]).int_value()?;
            let b = player.get_datum(&args[1]).int_value()?;
            Ok(player.alloc_datum(Datum::Int(a | b)))
        })
    }

    fn bit_not(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let a = player.get_datum(&args[0]).int_value()?;
            Ok(player.alloc_datum(Datum::Int(!a)))
        })
    }

    async fn call(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        let receiver_ref = &args[1];
        let (handler_name, args, instance_ids, list_count) = reserve_player_mut(|player| {
            let handler_name = player.get_datum(&args[0]);
            let receiver_clone = player.get_datum(receiver_ref).clone();
            let args = args[2..].to_vec();
            if !handler_name.is_symbol() {
                return Err(ScriptError::new(
                    "Handler name must be a symbol".to_string(),
                ));
            }
            let handler_name = handler_name.symbol_value()?;

            let (instance_ids, list_count) = match receiver_clone {
                Datum::PropList(prop_list, ..) => {
                    let mut instance_ids = vec![];
                    let count = prop_list.len();
                    for (_, value_ref) in prop_list {
                        if player.get_datum(&value_ref).is_void() {
                            continue;
                        }
                        instance_ids.extend(get_datum_script_instance_ids(&value_ref, player)?);
                    }
                    (Ok(Some(instance_ids)), count)
                }
                Datum::List(_, list, _) => {
                    let mut instance_ids = vec![];
                    let count = list.len();
                    for value_ref in list {
                        if player.get_datum(&value_ref).is_void() {
                            continue;
                        }
                        instance_ids.extend(get_datum_script_instance_ids(&value_ref, player)?);
                    }
                    (Ok(Some(instance_ids)), count)
                }
                _ => (Ok(None), 0),
            };

            Ok((handler_name, args, instance_ids?, list_count))
        })?;

        if instance_ids.is_none() {
            return player_call_datum_handler(&receiver_ref, handler_name, &args).await;
        }
        let instance_refs = instance_ids.unwrap();

        let mut result = player_alloc_datum(Datum::Null);
        for instance_ref in instance_refs {
            let handler = reserve_player_ref(|player| {
                ScriptInstanceUtils::get_script_instance_handler(
                    handler_name,
                    &instance_ref,
                    player,
                )
            })?;
            if let Some(handler) = handler {
                let scope = player_call_script_handler(Some(instance_ref), handler, &args).await?;
                result = scope.return_value;
            }
        }

        Ok(result)
    }

    /// `importFileInto member, fileOrUrl, propertyList` — replaces the content
    /// of `member` with the decoded contents of `fileOrUrl`. Accepts both
    /// forms: the old-Lingo global verb (`args[0]` = member ref) and the
    /// method-form `member.importFileInto(...)` (forwarded from
    /// `CastMemberRefHandlers::call_async` with the receiver prepended).
    ///
    /// v1 only handles bitmap members: PNG/JPG/GIF/etc. (anything the
    /// `image` crate decodes) → a 32-bit RGBA Bitmap that replaces the
    /// existing BitmapRef. The fetch goes through `NetManager`, so URLs
    /// are resolved against `base_path` and respect any `override_base_path`
    /// (the fake-movie-root used by tests). Returns Director's documented
    /// integer status: 0 = success, negative = failure.
    ///
    /// propertyList properties honored:
    ///   #trimWhiteSpace — non-zero stores `trim_white_space = true` on the
    ///       Bitmap so the renderer auto-trims (matching the parsed-asset path).
    /// Properties accepted but unused (Director defaults pass through):
    ///   #dither, #linked, #remapImageToStage.
    async fn import_file_into(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        use crate::player::bitmap::bitmap::{BuiltInPalette};
        use crate::player::cast_member::CastMemberType;

        if args.is_empty() {
            return Err(ScriptError::new(
                "importFileInto: missing member argument".to_string(),
            ));
        }
        let receiver = args[0].clone();

        // Phase 1 — pull args + member ref out of the player lock.
        let (member_ref, file_or_url, trim_white_space) = reserve_player_mut(|player| {
            let member_ref = match player.get_datum(&receiver) {
                Datum::CastMember(r) => r.to_owned(),
                _ => return Err(ScriptError::new(
                    "importFileInto: receiver must be a cast member".to_string(),
                )),
            };
            if args.len() < 2 {
                return Err(ScriptError::new(
                    "importFileInto requires a file path or URL".to_string(),
                ));
            }
            let file_or_url = player.get_datum(&args[1]).string_value()?;
            // Optional property list — Director #trimWhiteSpace defaults TRUE.
            let mut trim_white_space = true;
            if let Some(prop_ref) = args.get(2) {
                if let Datum::PropList(pairs, _) = player.get_datum(prop_ref) {
                    let pairs = pairs.clone();
                    for (k_ref, v_ref) in pairs.iter() {
                        let key = player.get_datum(k_ref).string_value().unwrap_or_default();
                        if key.eq_ignore_ascii_case("trimWhiteSpace") {
                            let v = player.get_datum(v_ref).int_value().unwrap_or(1);
                            trim_white_space = v != 0;
                        }
                        // #dither / #linked / #remapImageToStage accepted but
                        // unused — the decode path always produces 32-bit
                        // RGBA so dither/remap have no effect, and we don't
                        // track "linked" status on members yet.
                    }
                }
            }
            Ok((member_ref, file_or_url, trim_white_space))
        })?;

        // Determine the target member kind up front so we don't fetch for a
        // member type we can't import into. importFileInto replaces a member's
        // content with the named file (Director 11.5 Scripting Dictionary,
        // `importFileInto()`); we support bitmap and sound members.
        enum ImportTarget {
            Bitmap(crate::player::bitmap::manager::BitmapRef),
            Sound,
        }
        let import_target = reserve_player_ref(|player| {
            let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
            match member.map(|m| &m.member_type) {
                Some(CastMemberType::Bitmap(b)) => Some(ImportTarget::Bitmap(b.image_ref)),
                Some(CastMemberType::Sound(_)) => Some(ImportTarget::Sound),
                _ => None,
            }
        });
        let import_target = match import_target {
            Some(t) => t,
            None => {
                warn!(
                    "importFileInto: member ({}, {}) is not a bitmap or sound (v1 scope)",
                    member_ref.cast_lib, member_ref.cast_member
                );
                return reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Int(-2))));
            }
        };

        // Phase 2 — kick off the fetch via NetManager and await the result.
        let task_id = reserve_player_mut(|player| {
            player.net_manager.preload_net_thing(file_or_url.clone())
        });
        {
            let player = unsafe { crate::player::PLAYER_OPT.as_mut().unwrap() };
            if !player.net_manager.is_task_done(Some(task_id)) {
                player.net_manager.await_task(task_id).await;
            }
        }
        let bytes = reserve_player_ref(|player| {
            player.net_manager.get_task_result(Some(task_id))
        });
        let bytes = match bytes {
            Some(Ok(b)) if !b.is_empty() => b,
            Some(Ok(_)) | None => {
                warn!("importFileInto: empty/no result for '{}'", file_or_url);
                return reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Int(-200))));
            }
            Some(Err(code)) => {
                warn!("importFileInto: net fetch failed ({}) for '{}'", code, file_or_url);
                return reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Int(-200))));
            }
        };

        // Sound members: store the downloaded media verbatim. The playback
        // path (sound_channel.rs) sniffs MP3 frames and hands compressed data
        // to the browser's decodeAudioData, so the raw file bytes are exactly
        // what it expects — no eager decode here. Sample metadata stays at
        // defaults until the browser decodes the buffer at play time.
        let existing_bitmap_ref = match import_target {
            ImportTarget::Bitmap(r) => r,
            ImportTarget::Sound => {
                use crate::director::chunks::sound::SoundChunk;
                let byte_len = bytes.len();
                return reserve_player_mut(|player| {
                    if let Some(member) =
                        player.movie.cast_manager.find_mut_member_by_ref(&member_ref)
                    {
                        if let CastMemberType::Sound(s) = &mut member.member_type {
                            s.sound = SoundChunk::new(bytes);
                        }
                    }
                    debug!(
                        "importFileInto: imported '{}' ({} bytes) into sound member ({}, {})",
                        file_or_url, byte_len, member_ref.cast_lib, member_ref.cast_member
                    );
                    JsApi::dispatch_cast_member_changed(member_ref.clone());
                    Ok(player.alloc_datum(Datum::Int(0)))
                });
            }
        };

        // Phase 3 — decode to RGBA8. `image::load_from_memory` sniffs
        // PNG/JPG/GIF/BMP/TIFF/WebP/etc. by header magic.
        let img = match image::load_from_memory(&bytes) {
            Ok(img) => img.to_rgba8(),
            Err(e) => {
                let head: Vec<String> = bytes.iter().take(8).map(|b| format!("{:02X}", b)).collect();
                warn!(
                    "importFileInto: decode failed for '{}' ({} bytes, head=[{}]): {}",
                    file_or_url, bytes.len(), head.join(" "), e
                );
                return reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Int(-120))));
            }
        };
        let (w, h) = (img.width() as u16, img.height() as u16);
        if w == 0 || h == 0 {
            return reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Int(-120))));
        }
        let rgba = img.into_raw();

        // Phase 4 — build the Bitmap and swap it in. We always produce
        // 32-bit RGBA (the decode path doesn't preserve original depth),
        // which matches what `newMember(#bitmap)` initializes to.
        let mut bitmap = Bitmap::new(
            w, h,
            32, 32, 8,
            PaletteRef::BuiltIn(BuiltInPalette::SystemWin),
        );
        bitmap.data = rgba;
        bitmap.trim_white_space = trim_white_space;

        debug!(
            "importFileInto: imported '{}' ({}x{}) into member ({}, {})",
            file_or_url, w, h, member_ref.cast_lib, member_ref.cast_member
        );

        reserve_player_mut(|player| {
            // Mirror new dimensions into the BitmapMember's info so getters
            // like `member.width` / `member.height` return the imported size.
            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                if let CastMemberType::Bitmap(b) = &mut member.member_type {
                    b.info.width = w;
                    b.info.height = h;
                    b.info.bit_depth = 32;
                    b.info.pitch = w.saturating_mul(4);
                    b.info.trim_white_space = trim_white_space;
                }
            }
            player.bitmap_manager.replace_bitmap(existing_bitmap_ref, bitmap);
            JsApi::dispatch_cast_member_changed(member_ref.clone());
            Ok(player.alloc_datum(Datum::Int(0)))
        })
    }

    async fn do_command(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        // Get the code string from the first argument
        let code = reserve_player_ref(|player| player.get_datum(&args[0]).string_value())?;
        let code = code.trim().to_string();
        debug!("do: executing code: {}", code);

        if code.is_empty() || code == "nothing" {
            return Ok(DatumRef::Void);
        }

        use crate::player::eval::eval_lingo_command;
        eval_lingo_command(code).await
    }

    pub fn has_async_handler(name: Symbol) -> bool {
        let name_builtin = match name.into_builtin() {
            Some(b) => b,
            None => return false,
        };
        match name_builtin {
            BuiltInSymbol::Call => true,
            BuiltInSymbol::New => true,
            BuiltInSymbol::NewObject => true,
            BuiltInSymbol::CallAncestor => true,
            BuiltInSymbol::SendSprite => true,
            BuiltInSymbol::SendAllSprites => true,
            BuiltInSymbol::Value => true,
            BuiltInSymbol::Do => true,
            BuiltInSymbol::UpdateStage => true,
            BuiltInSymbol::Go => true,
            BuiltInSymbol::Nothing => true,
            // Old-style Lingo lets `importFileInto member, url, props` be
            // called as a global verb; route it to the same async impl as
            // the method form `member.importFileInto(url, props)`.
            BuiltInSymbol::ImportFileInto => true,
            _ => false,
        }
    }

    pub async fn call_async_handler(
        name: Symbol,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let name_builtin = name.into_builtin_or_error()?;
        match name_builtin {
            BuiltInSymbol::Call => Self::call(args).await,
            BuiltInSymbol::New => TypeHandlers::new(args).await,
            BuiltInSymbol::NewObject => TypeHandlers::new_object(args).await,
            BuiltInSymbol::CallAncestor => TypeHandlers::call_ancestor(args).await,
            BuiltInSymbol::SendSprite => MovieHandlers::send_sprite(args).await,
            BuiltInSymbol::SendAllSprites => MovieHandlers::send_all_sprites(args).await,
            BuiltInSymbol::Value => TypeHandlers::value(args).await,
            BuiltInSymbol::Do => Self::do_command(args).await,
            BuiltInSymbol::UpdateStage => MovieHandlers::update_stage(args).await,
            BuiltInSymbol::Go => MovieHandlers::go(args).await,
            BuiltInSymbol::Nothing => MovieHandlers::nothing_async(args).await,
            BuiltInSymbol::ImportFileInto => Self::import_file_into(args).await,
            _ => {
                let msg = format!("No built-in async handler: {}", name);
                return Err(ScriptError::new(msg));
            }
        }
    }

    pub fn call_handler(name: Symbol, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        match name.into_builtin_or_error()? {
            BuiltInSymbol::CastLib => CastHandlers::cast_lib(args),
            BuiltInSymbol::FindEmpty => CastHandlers::find_empty(args),
            BuiltInSymbol::PreloadNetThing => NetHandlers::preload_net_thing(args),
            BuiltInSymbol::NetDone => NetHandlers::net_done(args),
            BuiltInSymbol::MoveToFront | BuiltInSymbol::PreloadMember | BuiltInSymbol::PreloadBuffer | BuiltInSymbol::UnloadMember | BuiltInSymbol::Beep => Ok(DatumRef::Void),
            BuiltInSymbol::PuppetTempo => MovieHandlers::puppet_tempo(args),
            BuiltInSymbol::Objectp => TypeHandlers::objectp(args),
            BuiltInSymbol::Voidp => TypeHandlers::voidp(args),
            BuiltInSymbol::Listp => TypeHandlers::listp(args),
            BuiltInSymbol::Symbolp => TypeHandlers::symbolp(args),
            BuiltInSymbol::Stringp => TypeHandlers::stringp(args),
            BuiltInSymbol::Integerp => TypeHandlers::integerp(args),
            BuiltInSymbol::Floatp => TypeHandlers::floatp(args),
            BuiltInSymbol::Offset => StringHandlers::offset(args),
            BuiltInSymbol::Length => StringHandlers::length(args),
            BuiltInSymbol::Script => MovieHandlers::script(args),
            BuiltInSymbol::Void => TypeHandlers::void(args),
            BuiltInSymbol::Param => Self::param(args),
            BuiltInSymbol::Count => Self::count(args),
            BuiltInSymbol::CreateMask => Self::forward_bitmap_handler("createMask", args),
            BuiltInSymbol::CreateMatte => Self::forward_bitmap_handler("createMatte", args),
            BuiltInSymbol::GetAt => Self::get_at(args),
            BuiltInSymbol::GetLast => Self::get_last(args),
            BuiltInSymbol::GetPos => Self::get_pos_global(args),
            BuiltInSymbol::SetAt => Self::set_at(args),
            BuiltInSymbol::Ilk => TypeHandlers::ilk(args),
            BuiltInSymbol::Member => MovieHandlers::member(args),
            BuiltInSymbol::Space => StringHandlers::space(args),
            BuiltInSymbol::Integer => TypeHandlers::integer(args),
            BuiltInSymbol::String => StringHandlers::string(args),
            BuiltInSymbol::CharToNum => StringHandlers::char_to_num(args),
            BuiltInSymbol::NumToChar => StringHandlers::num_to_char(args),
            BuiltInSymbol::Float => TypeHandlers::float(args),
            BuiltInSymbol::Put => Self::put(args),
            BuiltInSymbol::Inspect => Self::inspect(args),
            BuiltInSymbol::Random => Self::random(args),
            BuiltInSymbol::BitAnd => Self::bit_and(args),
            BuiltInSymbol::BitOr => Self::bit_or(args),
            BuiltInSymbol::BitNot => Self::bit_not(args),
            BuiltInSymbol::Symbol => TypeHandlers::symbol(args),
            BuiltInSymbol::PuppetSprite => MovieHandlers::puppet_sprite(args),
            BuiltInSymbol::ClearGlobals => Self::clear_globals(args),
            BuiltInSymbol::Sprite => MovieHandlers::sprite(args),
            BuiltInSymbol::Point => TypeHandlers::point(args),
            BuiltInSymbol::ClickLoc => {
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::Point([player.movie.click_loc.0 as f64, player.movie.click_loc.1 as f64], 0)))
                })
            }
            BuiltInSymbol::ConstrainH => {
                reserve_player_mut(|player| {
                    let sprite_num = player.get_datum(&args[0]).int_value()? as i16;
                    let posn = player.get_datum(&args[1]).int_value()?;
                    let sprite = player.movie.score.get_sprite(sprite_num);
                    let (left, right) = if let Some(sprite) = sprite {
                        let rect = get_concrete_sprite_rect(player, sprite);
                        (rect.left, rect.right)
                    } else {
                        (0, 0)
                    };
                    Ok(player.alloc_datum(Datum::Int(posn.max(left).min(right))))
                })
            }
            BuiltInSymbol::ConstrainV => {
                reserve_player_mut(|player| {
                    let sprite_num = player.get_datum(&args[0]).int_value()? as i16;
                    let posn = player.get_datum(&args[1]).int_value()?;
                    let sprite = player.movie.score.get_sprite(sprite_num);
                    let (top, bottom) = if let Some(sprite) = sprite {
                        let rect = get_concrete_sprite_rect(player, sprite);
                        (rect.top, rect.bottom)
                    } else {
                        (0, 0)
                    };
                    Ok(player.alloc_datum(Datum::Int(posn.max(top).min(bottom))))
                })
            }
            // `stop` / `play` / `rewind` / `pause` are overloaded Lingo
            // built-ins. Historically the web port stubbed them all to
            // no-op because they targeted SWA sound channels (`stop sound 1`)
            // which we don't implement. But Director also uses them on
            // Flash sprites (`stop(sprite N)`, `play(sprite N)`,
            // `rewind(sprite N)`) — storyscramble's BS38 calls
            // `stop(sprite(me.spriteNum))` right after `gotoFrame(...,31)`
            // to park the bubble. Route Flash-sprite operands to the
            // Ruffle bridge; everything else (sound channels, members,
            // bare integers that don't resolve to a Flash sprite) keeps
            // the historical no-op behaviour.
            BuiltInSymbol::Stop => {
                if args.len() >= 1 {
                    if let Some(sn) = Self::resolve_flash_sprite_strict(&args[0])? {
                        ruffle_stop(sn);
                    }
                }
                Ok(DatumRef::Void)
            }
            BuiltInSymbol::Play => {
                if args.len() >= 1 {
                    if let Some(sn) = Self::resolve_flash_sprite_strict(&args[0])? {
                        ruffle_play(sn);
                    }
                }
                Ok(DatumRef::Void)
            }
            BuiltInSymbol::Rewind => {
                if args.len() >= 1 {
                    if let Some(sn) = Self::resolve_flash_sprite_strict(&args[0])? {
                        ruffle_rewind(sn);
                    }
                }
                Ok(DatumRef::Void)
            }
            BuiltInSymbol::Pause  => Ok(DatumRef::Void),
            BuiltInSymbol::Cursor => TypeHandlers::cursor(args),
            BuiltInSymbol::ExternalParamCount => MovieHandlers::external_param_count(args),
            BuiltInSymbol::ExternalParamName => MovieHandlers::external_param_name(args),
            BuiltInSymbol::ExternalParamValue => MovieHandlers::external_param_value(args),
            BuiltInSymbol::GetNetText => NetHandlers::get_net_text(args),
            BuiltInSymbol::Timeout => TypeHandlers::timeout(args),
            BuiltInSymbol::Rect => TypeHandlers::rect(args),
            BuiltInSymbol::GetStreamStatus => NetHandlers::get_stream_status(args),
            BuiltInSymbol::NetError => NetHandlers::net_error(args),
            BuiltInSymbol::NetTextResult => NetHandlers::net_text_result(args),
            BuiltInSymbol::PostNetText => NetHandlers::post_net_text(args),
            BuiltInSymbol::Rgb => TypeHandlers::rgb(args),
            BuiltInSymbol::List => TypeHandlers::list(args),
            BuiltInSymbol::Image => TypeHandlers::image(args),
            BuiltInSymbol::Filter => TypeHandlers::filter(args),
            BuiltInSymbol::NewMatrix => TypeHandlers::new_matrix(args),
            BuiltInSymbol::ConstraintDesc => TypeHandlers::constraint_desc(args),
            // Director allows both the method form `bitmap.getPixel(x, y)` and
            // the global form `getPixel(bitmap, x, y)` (chapter 15). Same for
            // `setPixel`. Both end up at the BitmapDatumHandlers entry — the
            // global form just strips the bitmap from arg[0] and forwards the
            // rest as the method args.
            BuiltInSymbol::GetPixel => {
                if args.is_empty() {
                    return Err(ScriptError::new(
                        "getPixel requires a bitmap argument".to_string(),
                    ));
                }
                let rest: Vec<DatumRef> = args.iter().skip(1).cloned().collect();
                BitmapDatumHandlers::get_pixel(&args[0], &rest)
            }
            BuiltInSymbol::SetPixel => {
                if args.is_empty() {
                    return Err(ScriptError::new(
                        "setPixel requires a bitmap argument".to_string(),
                    ));
                }
                let rest: Vec<DatumRef> = args.iter().skip(1).cloned().collect();
                BitmapDatumHandlers::set_pixel(&args[0], &rest)
            }
            BuiltInSymbol::Chars => StringHandlers::chars(args),
            BuiltInSymbol::PaletteIndex => TypeHandlers::palette_index(args),
            BuiltInSymbol::Abs => TypeHandlers::abs(args),
            BuiltInSymbol::Xtra => TypeHandlers::xtra(args),
            BuiltInSymbol::StopEvent => MovieHandlers::stop_event(args),
            BuiltInSymbol::GetPref => MovieHandlers::get_pref(args),
            BuiltInSymbol::SetPref => MovieHandlers::set_pref(args),
            BuiltInSymbol::UrlEncode => StringHandlers::url_encode(args),
            BuiltInSymbol::GoToNetPage => MovieHandlers::go_to_net_page(args),
            BuiltInSymbol::GoToNetMovie => MovieHandlers::go_to_net_movie(args),
            BuiltInSymbol::Pass => MovieHandlers::pass(args),
            BuiltInSymbol::Union => TypeHandlers::union(args),
            BuiltInSymbol::BitXor => TypeHandlers::bit_xor(args),
            BuiltInSymbol::Power => TypeHandlers::power(args),
            BuiltInSymbol::Add => TypeHandlers::add(args),
            BuiltInSymbol::Abort => Err(ScriptError::new_code(ScriptErrorCode::Abort, "abort".to_string())),
            BuiltInSymbol::MouseDown => {
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(datum_bool(player.movie.mouse_down)))
                })
            }
            BuiltInSymbol::RightMouseDown => {
                // We don't track right mouse state separately yet — return FALSE
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(datum_bool(false)))
                })
            }
            BuiltInSymbol::GetRendererServices => {
                // Return a prop list with renderer info stubs
                reserve_player_mut(|player| {
                    let make_sym = |p: &mut DirPlayer, s: &str| p.alloc_datum(Datum::Symbol(Symbol::from_str(s)));
                    let make_str = |p: &mut DirPlayer, s: &str| p.alloc_datum(Datum::String(s.to_string()));
                    let make_int = |p: &mut DirPlayer, n: i32| p.alloc_datum(Datum::Int(n));

                    // rendererDeviceList
                    let rdl_key = make_sym(player, "rendererDeviceList");
                    let device = make_str(player, "WebGL2");
                    let rdl_val = player.alloc_datum(Datum::List(DatumType::List, VecDeque::from(vec![device]), false));

                    // renderer
                    let rend_key = make_sym(player, "renderer");
                    let rend_val = make_str(player, "#openGL");

                    // Hardware info as nested proplist
                    let vendor_k = make_sym(player, "vendor");
                    let vendor_v = make_str(player, "WebGL");
                    let model_k = make_sym(player, "model");
                    let model_v = make_str(player, "WebGL2 Renderer");
                    let version_k = make_sym(player, "version");
                    let version_v = make_str(player, "2.0");
                    let max_tex_k = make_sym(player, "maxTextureSize");
                    let max_tex_v = make_int(player, 4096);
                    let tex_fmt_k = make_sym(player, "supportedTextureRenderFormats");
                    let fmt = make_str(player, "rgba8880");
                    let tex_fmt_v = player.alloc_datum(Datum::List(DatumType::List, VecDeque::from(vec![fmt]), false));
                    let tex_units_k = make_sym(player, "textureUnits");
                    let tex_units_v = make_int(player, 8);
                    let depth_k = make_sym(player, "depthBufferRange");
                    let depth_v = make_int(player, 24);
                    let color_k = make_sym(player, "colorBufferRange");
                    let color_v = make_int(player, 32);

                    let hw_info = player.alloc_datum(Datum::PropList(VecDeque::from(vec![
                        (vendor_k, vendor_v), (model_k, model_v), (version_k, version_v),
                        (max_tex_k, max_tex_v), (tex_fmt_k, tex_fmt_v), (tex_units_k, tex_units_v),
                        (depth_k, depth_v), (color_k, color_v),
                    ]), false));
                    let hw_key = make_sym(player, "hardwareInfo");

                    let result = player.alloc_datum(Datum::PropList(VecDeque::from(vec![
                        (rdl_key, rdl_val), (rend_key, rend_val), (hw_key, hw_info),
                    ]), false));
                    Ok(result)
                })
            }
            BuiltInSymbol::GetVariable => {
                // Flash (SWF) member interop — getVariable(sprite, path)
                if args.len() >= 2 {
                    let member_ref = Self::resolve_flash_member(&args[0])?;
                    let path = reserve_player_ref(|player| {
                        player.get_datum(&args[1]).string_value()
                    })?;
                    if let Some((sn, _cl, _cm)) = member_ref {
                        match ruffle_get_variable(sn, &path) {
                            Ok(val) => {
                                if let Some(s) = val.as_string() {
                                    return reserve_player_mut(|player| {
                                        Ok(player.alloc_datum(Datum::String(s)))
                                    });
                                }
                            }
                            Err(e) => warn!("getVariable error: {:?}", e),
                        }
                    }
                }
                Ok(DatumRef::Void)
            }
            BuiltInSymbol::SetVariable => {
                // Flash (SWF) member interop — setVariable(sprite, path, value)
                if args.len() >= 3 {
                    let member_ref = Self::resolve_flash_member(&args[0])?;
                    let path = reserve_player_ref(|player| {
                        player.get_datum(&args[1]).string_value()
                    })?;
                    let value = reserve_player_ref(|player| {
                        player.get_datum(&args[2]).string_value()
                    })?;
                    if let Some((sn, _cl, _cm)) = member_ref {
                        if let Err(e) = ruffle_set_variable(sn, &path, &value) {
                            warn!("setVariable error: {:?}", e);
                        }
                    }
                }
                Ok(DatumRef::Void)
            }
            BuiltInSymbol::GoToFrame => {
                // Flash (SWF) member interop — goToFrame(sprite, frame_or_label)
                if args.len() >= 2 {
                    let member_ref = Self::resolve_flash_member(&args[0])?;
                    let frame_or_label = reserve_player_ref(|player| {
                        player.get_datum(&args[1]).string_value()
                    })?;
                    if let Some((sn, _cl, _cm)) = member_ref {
                        ruffle_goto_frame(sn, &frame_or_label);
                    }
                }
                Ok(DatumRef::Void)
            }
            BuiltInSymbol::CallFrame => {
                if args.len() >= 2 {
                    let member_ref = Self::resolve_flash_member(&args[0])?;
                    let frame = reserve_player_ref(|player| {
                        player.get_datum(&args[1]).int_value()
                    })?;
                    if let Some((sn, _cl, _cm)) = member_ref {
                        ruffle_call_frame(sn, frame);
                    }
                }
                Ok(DatumRef::Void)
            }
            BuiltInSymbol::GetFlashProperty => {
                if args.len() >= 3 {
                    let member_ref = Self::resolve_flash_member(&args[0])?;
                    let target = reserve_player_ref(|player| {
                        player.get_datum(&args[1]).string_value()
                    })?;
                    let prop_num = reserve_player_ref(|player| {
                        player.get_datum(&args[2]).int_value()
                    })?;
                    if let Some((sn, _cl, _cm)) = member_ref {
                        match ruffle_get_flash_property(sn, &target, prop_num) {
                            Ok(val) => {
                                if let Some(s) = val.as_string() {
                                    return reserve_player_mut(|player| {
                                        Ok(player.alloc_datum(Datum::String(s)))
                                    });
                                }
                            }
                            Err(e) => warn!("getFlashProperty error: {:?}", e),
                        }
                    }
                }
                Ok(DatumRef::Void)
            }
            BuiltInSymbol::SetFlashProperty => {
                if args.len() >= 4 {
                    let member_ref = Self::resolve_flash_member(&args[0])?;
                    let target = reserve_player_ref(|player| {
                        player.get_datum(&args[1]).string_value()
                    })?;
                    let prop_num = reserve_player_ref(|player| {
                        player.get_datum(&args[2]).int_value()
                    })?;
                    let value = reserve_player_ref(|player| {
                        player.get_datum(&args[3]).string_value()
                    })?;
                    if let Some((sn, _cl, _cm)) = member_ref {
                        ruffle_set_flash_property(sn, &target, prop_num, &value);
                    }
                }
                Ok(DatumRef::Void)
            }
            BuiltInSymbol::HitTest => {
                if args.len() >= 3 {
                    let member_ref = Self::resolve_flash_member(&args[0])?;
                    let x = reserve_player_ref(|player| {
                        player.get_datum(&args[1]).int_value()
                    })?;
                    let y = reserve_player_ref(|player| {
                        player.get_datum(&args[2]).int_value()
                    })?;
                    if let Some((sn, _cl, _cm)) = member_ref {
                        let result = ruffle_hit_test(sn, x as f64, y as f64);
                        return reserve_player_mut(|player| {
                            Ok(player.alloc_datum(Datum::Int(if result { 1 } else { 0 })))
                        });
                    }
                }
                Ok(DatumRef::Void)
            }
            BuiltInSymbol::TellTarget => {
                if args.len() >= 2 {
                    // tellTarget is complex; for now just log it
                    let target = reserve_player_ref(|player| {
                        player.get_datum(&args[0]).string_value()
                    })?;
                    debug!("tellTarget: target={}", target);
                }
                Ok(DatumRef::Void)
            }
            BuiltInSymbol::GetaProp => TypeHandlers::get_a_prop(args),
            BuiltInSymbol::Inside => {
                let point = &args[0];
                let rect = &args[1..].to_vec();
                PointDatumHandlers::inside(point, rect)
            }
            BuiltInSymbol::AddProp => {
                let list = &args[0];
                let args = &args[1..].to_vec();
                PropListDatumHandlers::add_prop(list, args)
            }
            BuiltInSymbol::DeleteProp => {
                let list = &args[0];
                let args = &args[1..].to_vec();
                PropListDatumHandlers::delete_prop(list, args)
            }
            BuiltInSymbol::Append => {
                let list = &args[0];
                let args = &args[1..].to_vec();
                ListDatumHandlers::append(list, args)
            }
            BuiltInSymbol::DeleteAt => reserve_player_mut(|player| {
                let list = &args[0];
                let args = &args[1..].to_vec();
                match player.get_datum(list) {
                    Datum::List(..) => ListDatumHandlers::delete_at(list, args),
                    Datum::PropList(..) => PropListDatumHandlers::delete_at(list, args),
                    _ => Err(ScriptError::new("Cannot delete at non list".to_string())),
                }
            }),
            BuiltInSymbol::DeleteOne => {
                let list = &args[0];
                let args = &args[1..].to_vec();
                ListDatumHandlers::delete_one(list, &args)
            }
            BuiltInSymbol::DeleteAll => {
                let list = &args[0];
                ListDatumHandlers::delete_all(list, &vec![])
            }
            BuiltInSymbol::GetOne => reserve_player_mut(|player| {
                let list = &args[0];
                let args = &args[1..].to_vec();
                match player.get_datum(list) {
                    Datum::List(..) => ListDatumHandlers::get_one(list, args),
                    Datum::PropList(..) => PropListDatumHandlers::get_one(list, args),
                    _ => Err(ScriptError::new("Cannot get one at non list".to_string())),
                }
            }),
            BuiltInSymbol::FindPos => reserve_player_mut(|player| {
                let list = &args[0];
                let args = &args[1..].to_vec();
                match player.get_datum(list) {
                    Datum::List(..) => ListDatumHandlers::find_pos(list, &args),
                    Datum::PropList(..) => PropListDatumHandlers::find_pos(list, &args),
                    _ => Err(ScriptError::new("Cannot findPos on non-list".to_string())),
                }
            }),
            BuiltInSymbol::SetProp => {
                let datum = &args[0];
                let datum_type = reserve_player_ref(|player| player.get_datum(datum).type_enum());
                let args = &args[1..].to_vec();
                match datum_type {
                    DatumType::PropList => PropListDatumHandlers::set_opt_prop(datum, args),
                    DatumType::ScriptInstanceRef => ScriptInstanceDatumHandlers::set_prop(datum, args),
                    _ => Err(ScriptError::new(
                        "Cannot setProp on non-prop list or child object".to_string(),
                    )),
                }
            }
            BuiltInSymbol::GetPos => reserve_player_mut(|player| {
                let list = &args[0];
                let args = &args[1..].to_vec();
                match player.get_datum(list) {
                    Datum::List(..) => ListDatumHandlers::find_pos(list, &args),
                    Datum::PropList(..) => PropListDatumHandlers::get_pos(list, &args),
                    _ => Err(ScriptError::new("Cannot getPos of non-list".to_string())),
                }
            }),
            BuiltInSymbol::SetaProp => {
                let datum = &args[0];
                let datum_type = reserve_player_ref(|player| player.get_datum(datum).type_enum());
                let args = &args[1..].to_vec();
                match datum_type {
                    DatumType::PropList => PropListDatumHandlers::set_opt_prop(datum, args),
                    DatumType::ScriptInstanceRef => ScriptInstanceDatumHandlers::set_a_prop(datum, args),
                    _ => Err(ScriptError::new(
                        "Cannot setaProp on non-prop list or child object".to_string(),
                    )),
                }
            }
            BuiltInSymbol::AddAt => {
                let list = &args[0];
                let args = &args[1..].to_vec();
                ListDatumHandlers::add_at(list, args)
            }
            BuiltInSymbol::GetNodes => Self::get_nodes(args),
            BuiltInSymbol::Duplicate => {
                let item = &args[0];
                let args = &args[1..].to_vec();
                reserve_player_mut(|player| match player.get_datum(item) {
                    Datum::List(..) => ListDatumHandlers::duplicate(item, args),
                    Datum::PropList(..) => PropListDatumHandlers::duplicate(item, args),
                    Datum::Point(vals, flags) => {
                        Ok(player.alloc_datum(Datum::Point(*vals, *flags)))
                    }
                    Datum::Rect(vals, flags) => {
                        Ok(player.alloc_datum(Datum::Rect(*vals, *flags)))
                    }
                    Datum::String(s) => Ok(player.alloc_datum(Datum::String(s.clone()))),
                    Datum::Int(i) => Ok(player.alloc_datum(Datum::Int(*i))),
                    Datum::Float(f) => Ok(player.alloc_datum(Datum::Float(*f))),
                    Datum::Symbol(s) => Ok(player.alloc_datum(Datum::Symbol(s.clone()))),
                    Datum::ColorRef(c) => Ok(player.alloc_datum(Datum::ColorRef(c.clone()))),
                    Datum::Vector(v) => Ok(player.alloc_datum(Datum::Vector(*v))),
                    Datum::Transform3d(t) => Ok(player.alloc_datum(Datum::Transform3d(*t))),
                    Datum::CastMember(r) => Ok(player.alloc_datum(Datum::CastMember(r.clone()))),
                    Datum::BitmapRef(bitmap_ref) => {
                        // `duplicate(image)` returns an independent copy of the
                        // bitmap; the result is an ephemeral so it's freed when
                        // the caller's DatumRef drops.
                        let new_bitmap = player
                            .bitmap_manager
                            .get_bitmap(*bitmap_ref)
                            .ok_or_else(|| ScriptError::new(
                                "duplicate(): source bitmap not found".to_string(),
                            ))?
                            .clone();
                        let new_ref = player.bitmap_manager.add_ephemeral_bitmap(new_bitmap);
                        Ok(player.alloc_datum(Datum::BitmapRef(new_ref)))
                    }
                    _ => Err(ScriptError::new(format!("duplicate() not implemented for type {}", player.get_datum(item).type_str()))),
                })
            }
            BuiltInSymbol::GetProp => {
                let list = &args[0];
                let args = &args[1..].to_vec();
                PropListDatumHandlers::get_prop(list, args)
            }
            BuiltInSymbol::Min => TypeHandlers::min(args),
            BuiltInSymbol::Max => TypeHandlers::max(args),
            BuiltInSymbol::Sort => TypeHandlers::sort(args),
            BuiltInSymbol::Intersect => TypeHandlers::intersect(args),
            BuiltInSymbol::Rollover => MovieHandlers::rollover(args),
            BuiltInSymbol::GetPropAt => TypeHandlers::get_prop_at(args),
            BuiltInSymbol::PuppetSound => MovieHandlers::puppet_sound(args),
            BuiltInSymbol::Pi => TypeHandlers::pi(args),
            BuiltInSymbol::Sin => TypeHandlers::sin(args),
            BuiltInSymbol::Cos => TypeHandlers::cos(args),
            BuiltInSymbol::Sqrt => TypeHandlers::sqrt(args),
            BuiltInSymbol::Tan => TypeHandlers::tan(args),
            BuiltInSymbol::Atan => TypeHandlers::atan(args),
            BuiltInSymbol::Sound => TypeHandlers::sound(args),
            BuiltInSymbol::Vector => TypeHandlers::vector(args),
            BuiltInSymbol::Transform => TypeHandlers::transform3d(args),
            BuiltInSymbol::Color => TypeHandlers::color(args),
            BuiltInSymbol::Date => TypeHandlers::date(args),
            BuiltInSymbol::KeyPressed => Self::key_pressed(args),
            BuiltInSymbol::ShowGlobals => Self::show_globals(),
            BuiltInSymbol::TellStreamStatus => Self::tell_stream_status(args),
            BuiltInSymbol::Frame => {
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::Int(player.movie.current_frame as i32)))
                })
            }
            BuiltInSymbol::Label => Self::label(args),
            BuiltInSymbol::Alert => Self::alert(args),
            BuiltInSymbol::Objectp => Self::object_p(args),
            BuiltInSymbol::SoundBusy => TypeHandlers::sound_busy(args),
            BuiltInSymbol::Delay => MovieHandlers::delay(args),
            BuiltInSymbol::Halt => MovieHandlers::halt(args),
            BuiltInSymbol::StartTimer => Self::start_timer(args),
            BuiltInSymbol::ExternalEvent => Self::external_event(args),
            BuiltInSymbol::DontPassEvent => Self::dont_pass_event(args),
            BuiltInSymbol::FrameReady => Self::frame_ready(args),
            BuiltInSymbol::Marker => Self::marker(args),
            BuiltInSymbol::Play => {
                // play member("name") - play a sound on channel 1
                if args.is_empty() {
                    return Ok(DatumRef::Void);
                }
                reserve_player_mut(|player| {
                    let channel_datum = player.alloc_datum(Datum::SoundChannel(1));
                    SoundChannelDatumHandlers::call(player, &channel_datum, Symbol::builtin(BuiltInSymbol::Play), args)
                })
            }
            BuiltInSymbol::SpriteBox => {
                // spriteBox(sprite, left, top, right, bottom)
                if args.len() < 5 {
                    return Err(ScriptError::new(
                        "spriteBox requires 5 arguments (sprite, left, top, right, bottom)".to_string(),
                    ));
                }
                reserve_player_mut(|player| {
                    let sprite_num = player.get_datum(&args[0]).to_sprite_ref()?;
                    let left = player.get_datum(&args[1]).int_value()?;
                    let top = player.get_datum(&args[2]).int_value()?;
                    let right = player.get_datum(&args[3]).int_value()?;
                    let bottom = player.get_datum(&args[4]).int_value()?;

                    // Set dimensions first so the rect computation uses the new size
                    let sprite = player.movie.score.get_sprite_mut(sprite_num);
                    sprite.width = right - left;
                    sprite.height = bottom - top;
                    sprite.has_size_changed = true;

                    // Now compute the rect with the new dimensions (scaled reg_point reflects new size)
                    let sprite = player.movie.score.get_sprite(sprite_num).unwrap();
                    let current_rect = get_concrete_sprite_rect(player, sprite);

                    // Adjust position so the displayed left/top match the desired values
                    let sprite = player.movie.score.get_sprite_mut(sprite_num);
                    sprite.loc_h += left - current_rect.left;
                    sprite.loc_v += top - current_rect.top;

                    Ok(DatumRef::Void)
                })
            }
            BuiltInSymbol::PuppetTransition => {
                log::warn!("puppetTransition is not implemented");
                Ok(DatumRef::Void)
            }
            BuiltInSymbol::Preload => {
                log::warn!("preload is not implemented");
                Ok(DatumRef::Void)
            }
            BuiltInSymbol::CharPosToLoc => {
                reserve_player_mut(|player| {
                    if args.len() < 2 {
                        return Err(ScriptError::new(
                            "charPosToLoc requires 2 arguments (member, charPos)".to_string(),
                        ));
                    }
                    let member_ref = player.get_datum(&args[0]).to_member_ref()?;
                    let char_pos = player.get_datum(&args[1]).int_value()?;

                    let member = player
                        .movie
                        .cast_manager
                        .find_member_by_ref(&member_ref)
                        .ok_or_else(|| ScriptError::new("Member not found".to_string()))?;

                    let (text, fixed_line_space, top_spacing, char_spacing, member_width, font_name, font_size, alignment, tab_stops) = match &member.member_type {
                        crate::player::cast_member::CastMemberType::Text(t) => {
                            (t.text.clone(), t.fixed_line_space, t.top_spacing, t.char_spacing as i16, t.width as i16, t.font.clone(), t.font_size, t.alignment.clone(), t.tab_stops.clone())
                        }
                        crate::player::cast_member::CastMemberType::Field(f) => {
                            (f.text.clone(), f.fixed_line_space, f.top_spacing, 0, f.width as i16, f.font.clone(), f.font_size, f.alignment.clone(), Vec::new())
                        }
                        crate::player::cast_member::CastMemberType::Button(b) => {
                            (b.field.text.clone(), b.field.fixed_line_space, b.field.top_spacing, 0, b.field.width as i16, b.field.font.clone(), b.field.font_size, b.field.alignment.clone(), Vec::new())
                        }
                        _ => {
                            return Err(ScriptError::new(
                                "charPosToLoc requires a text, field, or button member".to_string(),
                            ))
                        }
                    };

                    let align_kind: u8 = if alignment == BuiltInSymbol::Center {
                        1
                    } else if alignment == BuiltInSymbol::Right {
                        2
                    } else {
                        0
                    };

                    // Resolve the member's actual font the same way text.rs .image does.
                    // If it's a PFR/bitmap font we can measure locally via get_text_char_pos;
                    // otherwise we delegate to Canvas2D measure_text_native so the width
                    // matches what was rasterised into the member's .image.
                    let font_size_opt = if font_size > 0 { Some(font_size) } else { None };
                    let loaded_font = if !font_name.is_empty() {
                        player.font_manager.get_font_with_cast_and_bitmap(
                            &font_name,
                            &player.movie.cast_manager,
                            &mut player.bitmap_manager,
                            font_size_opt,
                            None,
                        )
                    } else {
                        None
                    };
                    let is_pfr = loaded_font.as_ref().map_or(false, |f| f.char_widths.is_some());

                    // char_pos is 1-based; convert to 0-based index. Also cap to text length.
                    let index = if char_pos > 0 { (char_pos - 1) as usize } else { 0 };

                    if !is_pfr && !font_name.is_empty() {
                        // Native Canvas2D path: measure the substring up to `index` using the
                        // member's font so the returned x matches the rasterised image width.
                        // Handle multi-line text by tracking which line `index` falls on.
                        let display_font_name = if font_name.is_empty() { "Arial".to_string() } else { font_name.clone() };
                        let display_font_size = if font_size > 0 { font_size } else { 12 };

                        let mut consumed = 0usize;
                        let mut line_idx = 0usize;
                        let mut line_start: Option<&str> = None;
                        let mut prefix_chars = 0usize;
                        let chars_vec: Vec<char> = text.chars().collect();
                        let target = index.min(chars_vec.len());

                        // Split on \r / \n; treat \r\n as single break.
                        let normalised: String = text.replace("\r\n", "\n").replace('\r', "\n");
                        for (li, line) in normalised.split('\n').enumerate() {
                            let line_len = line.chars().count();
                            if target <= consumed + line_len {
                                line_idx = li;
                                line_start = Some(line);
                                prefix_chars = target - consumed;
                                break;
                            }
                            consumed += line_len + 1; // +1 for the line break
                        }
                        let (line_ref, prefix_len) = match line_start {
                            Some(l) => (l, prefix_chars),
                            None => {
                                // target was beyond end-of-text: use last line, full length.
                                let lines: Vec<&str> = normalised.split('\n').collect();
                                let last = lines.last().copied().unwrap_or("");
                                line_idx = lines.len().saturating_sub(1);
                                (last, last.chars().count())
                            }
                        };
                        let prefix: String = line_ref.chars().take(prefix_len).collect();

                        // Measure prefix width AND full line width via Canvas2D.
                        // Full-line width is needed to apply alignment offset (center/right)
                        // so the returned x matches the rasterised image's pixel position.
                        let (prefix_w, line_w) = {
                            use wasm_bindgen::JsCast;
                            let font_str_for_log = format!("{}px {}", display_font_size, display_font_name);
                            web_sys::window()
                                .and_then(|w| w.document())
                                .and_then(|d| d.create_element("canvas").ok())
                                .and_then(|el| el.dyn_into::<web_sys::HtmlCanvasElement>().ok())
                                .and_then(|c| c.get_context("2d").ok().flatten())
                                .and_then(|c| c.dyn_into::<web_sys::CanvasRenderingContext2d>().ok())
                                .map(|ctx| {
                                    ctx.set_font(&font_str_for_log);
                                    let p = ctx.measure_text(&prefix).ok().map(|m| m.width()).unwrap_or(0.0);
                                    let l = ctx.measure_text(line_ref).ok().map(|m| m.width()).unwrap_or(0.0);
                                    (p, l)
                                })
                                .unwrap_or((0.0, 0.0))
                        };
                        let start_x = match align_kind {
                            1 if member_width > 0 => ((member_width as f64 - line_w) / 2.0).max(0.0),
                            2 if member_width > 0 => (member_width as f64 - line_w).max(0.0),
                            _ => 0.0,
                        };
                        let width = (start_x + prefix_w).round() as i32;

                        let line_step = if fixed_line_space > 0 {
                            fixed_line_space as i32
                        } else {
                            display_font_size as i32
                        };
                        let y = top_spacing as i32 + line_idx as i32 * line_step;

                        Ok(player.alloc_datum(Datum::Point([width as f64, y as f64], 0)))
                    } else {
                        let font = loaded_font
                            .or_else(|| player.font_manager.get_system_font())
                            .ok_or_else(|| ScriptError::new("No font available".to_string()))?;
                        let params = crate::player::font::DrawTextParams {
                            font: &font,
                            line_height: None,
                            line_spacing: fixed_line_space,
                            top_spacing,
                            char_spacing,
                            member_width: if member_width > 0 { Some(member_width) } else { None },
                        };
                        // Tab-aware char position. Coke Studios' userlist computes
                        // the dotted-separator bounds via two charPosToLoc calls,
                        // one landing on a char inside "Go!" (past the tab) and
                        // one on the line's last char. We need the tab to advance
                        // to its tab-stop for those positions to bracket the
                        // empty space between the roomname column and the "Go!"
                        // column — which is where the dotted line should span.
                        // When the requested char index is BEFORE any tab on its
                        // line we stay at the pre-tab x so Lingo that asks for
                        // positions inside the name column (e.g. underline draws)
                        // still returns the usual advance-based x.
                        let (x_from_zero, y) = if text.contains('\t') && !tab_stops.is_empty() {
                            let eff_lh = if font.font_size > 0 { font.font_size } else { font.char_height };
                            let line_step = fixed_line_space.max(eff_lh) as i16 + 1;
                            // Helper: width of a substring (chars only, excluding control chars).
                            let segment_width = |chars: &[char], from: usize| -> i16 {
                                let mut w: i16 = 0;
                                for c in chars.iter().skip(from) {
                                    if *c == '\t' || *c == '\r' || *c == '\n' { break; }
                                    w = w.saturating_add(
                                        font.get_char_advance(*c as u8) as i16 + 1 + char_spacing,
                                    );
                                }
                                w
                            };
                            let chars: Vec<char> = text.chars().collect();
                            let mut x: i16 = 0;
                            let mut y: i16 = top_spacing;
                            let mut current_line_tab_count: usize = 0;
                            let mut char_i: usize = 0;
                            let mut prev_was_cr = false;
                            let mut result: Option<(i16, i16)> = None;
                            while char_i < chars.len() {
                                if char_i == index {
                                    result = Some((x, y));
                                    break;
                                }
                                let c = chars[char_i];
                                if c == '\n' && prev_was_cr {
                                    prev_was_cr = false;
                                    char_i += 1;
                                    continue;
                                }
                                if c == '\r' || c == '\n' {
                                    prev_was_cr = c == '\r';
                                    x = 0;
                                    y = y.saturating_add(line_step);
                                    current_line_tab_count = 0;
                                } else if c == '\t' {
                                    prev_was_cr = false;
                                    if let Some(stop) = tab_stops.get(current_line_tab_count) {
                                        let stop_pos = stop.position as i16;
                                        // Match the renderer's flush_line tab logic
                                        // (text.rs): right/center tabs look ahead at
                                        // the next segment's width to anchor the
                                        // segment to the stop's right edge / centre.
                                        let next_seg_w = segment_width(&chars, char_i + 1);
                                        let new_x = match stop.tab_type.as_str() {
                                            "right" => (stop_pos - next_seg_w).max(x),
                                            "center" => (stop_pos - next_seg_w / 2).max(x),
                                            _ => stop_pos.max(x), // #left / #decimal
                                        };
                                        x = new_x;
                                    }
                                    current_line_tab_count += 1;
                                } else {
                                    prev_was_cr = false;
                                    let adv = font.get_char_advance(c as u8) as i16
                                        + 1 + char_spacing;
                                    x = x.saturating_add(adv);
                                }
                                char_i += 1;
                            }
                            // If we exited the loop without hitting `index` (target
                            // beyond end-of-text), result stays None and we fall back
                            // to the final (x, y).
                            if result.is_none() && char_i == index {
                                result = Some((x, y));
                            }
                            result.unwrap_or((x, y))
                        } else {
                            crate::player::font::get_text_char_pos(&text, &params, index)
                        };

                        // Apply alignment offset so the returned x matches the pixel position
                        // in the rasterised image (the bitmap render centres/right-aligns the
                        // line inside member_width; see text.rs flush_line at lines 888-892).
                        //
                        // EXCEPTION: when the line has a right or center tab, the renderer
                        // anchors segments to those stops and skips alignment (see
                        // flush_line's `has_right_tab` branch). We must skip alignment too,
                        // otherwise charPosToLoc returns positions shifted by the centring
                        // amount even though the rendered text is left-anchored. Coke
                        // Studios userlist hits this: alignment=#center inherited from the
                        // loading screen, but each row uses [#left at 18, #right at pwidth-1]
                        // tabs — Lingo's dotted-line bounds were drawn off-canvas because
                        // dotleft/dotright both got an extra ~71px centring offset.
                        let line_has_anchor_tab = !tab_stops.is_empty()
                            && tab_stops.iter().any(|t| {
                                t.tab_type == BuiltInSymbol::Right || t.tab_type == BuiltInSymbol::Center
                            });
                        let start_x = if align_kind != 0 && member_width > 0 && !line_has_anchor_tab {
                            // Compute the width of the line that `index` falls on, using the
                            // same advance-per-char sum as flush_line.
                            let normalised: String = text.replace("\r\n", "\n").replace('\r', "\n");
                            let mut consumed = 0usize;
                            let target = index.min(text.chars().count());
                            let mut hit_line: Option<String> = None;
                            for line in normalised.split('\n') {
                                let line_len = line.chars().count();
                                if target <= consumed + line_len {
                                    hit_line = Some(line.to_string());
                                    break;
                                }
                                consumed += line_len + 1;
                            }
                            let line = hit_line.unwrap_or_else(|| {
                                normalised.split('\n').last().unwrap_or("").to_string()
                            });
                            let line_width: i32 = line
                                .chars()
                                .map(|c| font.get_char_advance(c as u8) as i32 + char_spacing as i32)
                                .sum();
                            match align_kind {
                                1 => ((member_width as i32 - line_width) / 2).max(0),
                                2 => (member_width as i32 - line_width).max(0),
                                _ => 0,
                            }
                        } else {
                            0
                        };

                        let x = x_from_zero as i32 + start_x;
                        Ok(player.alloc_datum(Datum::Point([x as f64, y as f64], 0)))
                    }
                })
            }
            _ => {
                // Check if first arg is an xtra instance - if so, forward to the xtra instance handler
                if !args.is_empty() {
                    if let Some(res) = reserve_player_ref(|player| {
                        if let Ok((xtra_name, instance_id)) = player.get_datum(&args[0]).to_xtra_instance() {
                            let remaining_args = args[1..].to_vec();
                            return Some(call_xtra_instance_handler(&xtra_name, *instance_id, &name.to_string(), &remaining_args));
                        }
                        None
                    }) {
                        return res;
                    }
                }
                // Static-only Xtras (OpenURL, SysMenu, BudAPI, Curl statics).
                if let Some(res) =
                    crate::player::xtra::manager::try_call_xtra_static_handler(name, args)
                {
                    return res;
                }
                let formatted_args = reserve_player_ref(|player| {
                    let mut s = String::new();
                    for arg in args {
                        if !s.is_empty() { s.push_str(", "); }
                        s.push_str(&format_concrete_datum(&player.get_datum(arg), player));
                    }
                    Ok(s)
                })?;
                let msg = format!("No built-in handler: {}({})", name, formatted_args);
                warn!("{msg}");
                return Err(ScriptError::new(msg));
            }
        }
    }

    fn alert(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let message = player.get_datum(&args[0]).string_value()?;
            trace_output(player, &format!("Alert: {}", message));
            Ok(DatumRef::Void)
        })
    }

    fn label(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let label_name = player.get_datum(&args[0]).string_value()?;

            debug!("Searching for label: {}", label_name);

            let label_name_lower = label_name.to_lowercase();

            let label = player
                .movie
                .score
                .frame_labels
                .iter()
                .find(|label| label.label.to_lowercase() == label_name_lower);

            // Log result
            if let Some(lbl) = label {
                debug!(
                    "Found label '{}' at frame {}",
                    lbl.label, lbl.frame_num
                );
            } else {
                warn!("Label not found");
            }

            Ok(player.alloc_datum(Datum::Int(
                label.map_or(0, |label| label.frame_num as i32),
            )))
        })
    }

    fn show_globals() -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            trace_output(player, "--- Global Variables ---");
            for (name, value) in &player.globals {
                let value = format_datum(value, player);
                trace_output(player, &format!("{} = {}", name, value));
            }
        });
        Ok(DatumRef::Void)
    }

    pub fn key_pressed(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let arg_datum = player.get_datum(&args[0]);

            let key_code = if let Ok(key_str) = arg_datum.string_value() {
                // STRING: First check if it's a single character
                if key_str.len() == 1 {
                    // Single character - convert to Director key code
                    let ch = key_str
                        .chars()
                        .next()
                        .unwrap();

                    // First check for special Director characters (arrow keys, etc.)
                    // These are control characters like ASCII 28-31 for arrow keys
                    if let Some(&code) = keyboard_map::get_director_special_char_to_keycode_map().get(&ch) {
                        code
                    } else {
                        // Regular character - lowercase and look up
                        let ch_lower = ch.to_lowercase().next().unwrap();
                        *keyboard_map::get_char_to_keycode_map()
                            .get(&ch_lower)
                            .unwrap_or(&0)
                    }
                } else {
                    // Try to parse as number string (like "123")
                    if let Ok(code) = key_str.parse::<i32>() {
                        // Check if it's an ASCII letter code that needs mapping
                        let mapped_code = if (65..=90).contains(&code) || (97..=122).contains(&code)
                        {
                            let ch = (code as u8 as char).to_lowercase().next().unwrap();
                            *keyboard_map::get_char_to_keycode_map()
                                .get(&ch)
                                .unwrap_or(&(code as u16))
                        } else {
                            code as u16
                        };
                        mapped_code
                    } else {
                        return Err(ScriptError::new(format!(
                            "keyPressed: cannot parse string '{}'",
                            key_str
                        )));
                    }
                }
            } else if let Ok(code) = arg_datum.int_value() {
                // INTEGER: Check if it's an ASCII code that needs mapping
                let mapped_code = if (65..=90).contains(&code) || (97..=122).contains(&code) {
                    let ch = (code as u8 as char).to_lowercase().next().unwrap();
                    *keyboard_map::get_char_to_keycode_map()
                        .get(&ch)
                        .unwrap_or(&(code as u16))
                } else {
                    code as u16
                };
                mapped_code
            } else {
                return Err(ScriptError::new(
                    "keyPressed expects a string or integer".to_string(),
                ));
            };

            // Check if any currently pressed key matches this code
            let is_pressed = player
                .keyboard_manager
                .down_keys
                .iter()
                .any(|key| key.code == key_code);

            // Debug: log arrow key checks (per-key counters)
            if is_pressed && (key_code == 123 || key_code == 124) {
                static KP_LR: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                if KP_LR.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 5 {
                    debug!(
                        "[KEY-STEER] keyPressed({}) = TRUE ({})",
                        key_code, if key_code == 123 { "LEFT" } else { "RIGHT" }
                    );
                }
            }
            if is_pressed && (key_code == 125 || key_code == 126) {
                static KP_UD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                if KP_UD.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 3 {
                    debug!(
                        "[KEY-DRIVE] keyPressed({}) = TRUE ({})",
                        key_code, if key_code == 126 { "UP" } else { "DOWN" }
                    );
                }
            }

            Ok(player.alloc_datum(datum_bool(is_pressed)))
        })
    }

    pub fn get_nodes(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        if args.len() < 2 {
            return Err(ScriptError::new(
                "getNodes requires 2 arguments: xml_node, node_name".to_string(),
            ));
        }

        reserve_player_mut(|player| {
            let xml_node = player.get_datum(&args[0]);
            let node_name = player.get_datum(&args[1]).string_value()?;

            debug!("🔧 getNodes called for node type: {}", node_name);

            // Get the XML node ID
            let xml_id = match xml_node {
                Datum::XmlRef(id) => *id,
                _ => {
                    return Err(ScriptError::new(
                        "First argument must be an XML node reference".to_string(),
                    ));
                }
            };

            // Use XmlHelper to search for matching nodes
            let matching_nodes = XmlHelper::find_nodes_by_name(player, xml_id, &node_name);

            Ok(player.alloc_datum(Datum::List(
                crate::director::lingo::datum::DatumType::List,
                VecDeque::from(matching_nodes),
                false,
            )))
        })
    }

    fn tell_stream_status(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        if args.len() < 1 {
            return Err(ScriptError::new(
                "tellStreamStatus requires 1 argument".to_string(),
            ));
        }

        reserve_player_mut(|player| {
            let enable = player.get_datum(&args[0]).bool_value().unwrap_or(false);
            player.enable_stream_status_handler = enable;
            if enable {
                // Clear reported state so all tasks get fresh callbacks
                player.stream_status_reported.clear();
            }
            Ok(player.alloc_datum(Datum::Int(enable as i32)))
        })
    }
    
    fn void_p(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            if args.is_empty() {
                return Ok(player.alloc_datum(Datum::Int(1)));
            }
            
            let datum = player.get_datum(&args[0]);
            let is_void = matches!(datum, Datum::Void | Datum::Null);
            
            Ok(player.alloc_datum(Datum::Int(if is_void { 1 } else { 0 })))
        })
    }
    
    fn object_p(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            if args.is_empty() {
                return Ok(player.alloc_datum(Datum::Int(0)));
            }
            
            let datum = player.get_datum(&args[0]);
            
            // Director considers these as objects (not primitives)
            let is_object = matches!(
                datum,
                Datum::ScriptInstanceRef(_)
                | Datum::SpriteRef(_)
                | Datum::CastMember(_)
                | Datum::List(..)
                | Datum::PropList(..)
                | Datum::BitmapRef(_)
                | Datum::ScriptRef(_)
                | Datum::XmlRef(_)
                | Datum::Xtra(_)
                | Datum::XtraInstance(..)
                | Datum::Matte(..)
                | Datum::PlayerRef
                | Datum::MovieRef
                | Datum::MouseRef
                | Datum::Stage
                | Datum::CastLib(_)
                | Datum::DateRef(_)
                | Datum::MathRef(_)
                | Datum::SoundRef(_)
                | Datum::SoundChannel(_)
                | Datum::CursorRef(_)
                | Datum::TimeoutRef(_)
            );
            
            Ok(player.alloc_datum(Datum::Int(if is_object { 1 } else { 0 })))
        })
    }

    fn start_timer(_args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            // Reset the start_time to current time
            player.start_time = chrono::Local::now();
            Ok(DatumRef::Void)
        })
    }

    pub fn external_event(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_ref(|player| {
            let event_string = player.get_datum(&args[0]).string_value()?;
            
            debug!("🔔 externalEvent: {}", event_string);
            
            crate::js_api::JsApi::dispatch_external_event(&event_string);
            
            Ok(DatumRef::Void)
        })
    }

    fn dont_pass_event(_args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let scope_ref = player.current_scope_ref();
            if let Some(scope) = player.scopes.get_mut(scope_ref) {
                scope.passed = false;  // Set passed to false to stop propagation
            }
            Ok(DatumRef::Void)
        })
    }

    fn frame_ready(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            // Get start and end frame numbers
            let (start_frame, end_frame) = if args.is_empty() {
                // No arguments - check current frame only
                let current = player.movie.current_frame;
                (current, current)
            } else {
                let start = player.get_datum(&args[0]).int_value()? as u32;
                let end = if args.len() > 1 {
                    player.get_datum(&args[1]).int_value()? as u32
                } else {
                    start
                };
                (start, end)
            };

            debug!("frameReady checking frames {} to {}", start_frame, end_frame);

            // Check if frame range is valid
            if start_frame < 1 || end_frame < start_frame {
                return Ok(player.alloc_datum(Datum::Int(0)));
            }

            // Collect all unique cast member references used in the frame range
            let mut cast_members_to_check = std::collections::HashSet::new();
            
            for frame_num in start_frame..=end_frame {
                // Check channel initialization data for this frame
                for (frame_index, channel_index, data) in &player.movie.score.channel_initialization_data {
                    if *frame_index + 1 == frame_num {
                        // Skip empty sprites
                        if data.cast_lib > 0 && data.cast_member > 0 {
                            cast_members_to_check.insert((data.cast_lib, data.cast_member));
                        }
                    }
                }
            }

            debug!("Found {} unique cast members to check", cast_members_to_check.len());

            // Check if all cast members are loaded
            for (cast_lib, cast_member_num) in cast_members_to_check {
                if let Ok(cast) = player.movie.cast_manager.get_cast(cast_lib as u32) {
                    if let Some(cast_member) = cast.members.get(&(cast_member_num as u32)) {
                        // Check if the cast member is fully loaded based on its type
                        match &cast_member.member_type {
                            crate::player::cast_member::CastMemberType::Bitmap(bitmap_member) => {
                                // Check if bitmap is loaded by checking if it exists in bitmap_manager
                                if player.bitmap_manager.get_bitmap(bitmap_member.image_ref).is_none() {
                                    debug!("Cast member {}.{} (bitmap) not ready", cast_lib, cast_member_num);
                                    return Ok(player.alloc_datum(Datum::Int(0)));
                                }
                            }
                            crate::player::cast_member::CastMemberType::Sound(_sound_member) => {
                                // Sound members are loaded when the cast member exists
                                // No additional check needed
                            }
                            // Other types (text, field, shape, script) are generally ready immediately
                            _ => {}
                        }
                    } else {
                        // Cast member doesn't exist - this is OK in Director
                        // Missing cast members are just treated as empty/not displayed
                        debug!("Cast member {}.{} doesn't exist (skipping)", cast_lib, cast_member_num);
                    }
                } else {
                    // Cast lib doesn't exist - this is also OK
                    debug!("Cast lib {} doesn't exist (skipping)", cast_lib);
                }
            }

            debug!("All frames ready!");
            Ok(player.alloc_datum(Datum::Int(1)))
        })
    }

    fn marker(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            if args.is_empty() {
                return Err(ScriptError::new("marker requires 1 argument".to_string()));
            }

            let arg = player.get_datum(&args[0]);
            
            match arg {
                // marker(n) returns the frame number of the nth marker relative to current frame.
                // marker(0) = current marker (nearest at or before current frame)
                // marker(1) = next marker, marker(-1) = previous marker, etc.
                Datum::Int(offset) => {
                    let offset = *offset;
                    let current_frame = player.movie.current_frame as i32;
                    let labels = &player.movie.score.frame_labels;

                    // Find the index of the current marker (last marker at or before current frame)
                    let current_idx = labels.iter().rposition(|l| l.frame_num <= current_frame);

                    let target_idx = if offset >= 0 {
                        current_idx.map(|i| i as i32 + offset).unwrap_or(offset - 1)
                    } else {
                        current_idx.map(|i| i as i32 + offset).unwrap_or(-1)
                    };

                    if target_idx >= 0 && (target_idx as usize) < labels.len() {
                        Ok(player.alloc_datum(Datum::Int(labels[target_idx as usize].frame_num)))
                    } else {
                        // Out of range - return 0
                        Ok(player.alloc_datum(Datum::Int(0)))
                    }
                }
                // If argument is a string, return the frame number of that marker
                Datum::String(marker_name) => {
                    let marker_name_lower = marker_name.to_lowercase();
                    let marker = player
                        .movie
                        .score
                        .frame_labels
                        .iter()
                        .find(|label| label.label.to_lowercase() == marker_name_lower);
                    
                    Ok(player.alloc_datum(Datum::Int(
                        marker.map_or(0, |label| label.frame_num as i32),
                    )))
                }
                Datum::Symbol(symbol) => {
                    let marker_name = symbol.as_str();
                    let marker_name_lower = marker_name.to_lowercase();
                    let marker = player
                        .movie
                        .score
                        .frame_labels
                        .iter()
                        .find(|label| label.label.to_lowercase() == marker_name_lower);
                    
                    Ok(player.alloc_datum(Datum::Int(
                        marker.map_or(0, |label| label.frame_num as i32),
                    )))
                }
                _ => Err(ScriptError::new(format!(
                    "marker expects string or integer, got {}",
                    arg.type_str()
                ))),
            }
        })
    }
}

fn get_datum_script_instance_ids(
    value_ref: &DatumRef,
    player: &mut DirPlayer,
) -> Result<Vec<ScriptInstanceRef>, ScriptError> {
    let value = player.get_datum(value_ref);
    let mut instance_refs = vec![];
    match value {
        Datum::ScriptInstanceRef(instance_id) => {
            instance_refs.push(instance_id.clone());
        }
        Datum::SpriteRef(sprite_id) => {
            let fallback = player
                .movie
                .score
                .get_sprite(*sprite_id)
                .map(|sprite| sprite.script_instance_list.clone())
                .unwrap_or_default();
            instance_refs.extend(player.get_sprite_script_instance_ids(*sprite_id, fallback.as_slice()));
        }
        Datum::Int(_) => {}
        _ => {
            return Err(ScriptError::new(format!(
                "Cannot get script instance ids from datum of type: {}",
                value.type_str()
            )));
        }
    }
    Ok(instance_refs)
}
