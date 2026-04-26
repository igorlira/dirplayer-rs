use log::{warn, debug};
use wasm_bindgen::prelude::*;

use super::{
    cast::CastHandlers,
    datum_handlers::{
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
        DatumRef, DirPlayer, ScriptError, ScriptErrorCode, bitmap::bitmap::{Bitmap, PaletteRef, get_system_default_palette}, datum_formatting::{format_concrete_datum, format_datum}, geometry::IntRect, handlers::datum_handlers::xml::XmlHelper, keyboard_map, player_alloc_datum, player_call_script_handler, reserve_player_mut, reserve_player_ref, score::get_concrete_sprite_rect, script_ref::ScriptInstanceRef, trace_output, xtra::manager::call_xtra_instance_handler
    },
};

#[wasm_bindgen]
extern "C" {
    /// Call into Ruffle's JS API to get a Flash variable
    #[wasm_bindgen(js_name = "ruffleGetVariable", catch)]
    fn ruffle_get_variable(cast_lib: i32, cast_member: i32, path: &str) -> Result<JsValue, JsValue>;

    /// Call into Ruffle's JS API to set a Flash variable
    #[wasm_bindgen(js_name = "ruffleSetVariable", catch)]
    fn ruffle_set_variable(cast_lib: i32, cast_member: i32, path: &str, value: &str) -> Result<JsValue, JsValue>;

    /// Call a Flash function via Ruffle's JS API
    #[wasm_bindgen(js_name = "ruffleCallFunction", catch)]
    fn ruffle_call_function(cast_lib: i32, cast_member: i32, path: &str, args_xml: &str) -> Result<JsValue, JsValue>;

    /// Go to a specific frame on a Flash instance
    #[wasm_bindgen(js_name = "ruffleGoToFrame")]
    fn ruffle_goto_frame(cast_lib: i32, cast_member: i32, frame: i32);

    #[wasm_bindgen(js_name = "ruffleStop")]
    fn ruffle_stop(cast_lib: i32, cast_member: i32);

    #[wasm_bindgen(js_name = "rufflePlay")]
    fn ruffle_play(cast_lib: i32, cast_member: i32);

    #[wasm_bindgen(js_name = "ruffleRewind")]
    fn ruffle_rewind(cast_lib: i32, cast_member: i32);

    #[wasm_bindgen(js_name = "ruffleIsPlaying")]
    fn ruffle_is_playing(cast_lib: i32, cast_member: i32) -> bool;

    #[wasm_bindgen(js_name = "ruffleGetFrameCount")]
    fn ruffle_get_frame_count(cast_lib: i32, cast_member: i32) -> i32;

    #[wasm_bindgen(js_name = "ruffleGetCurrentFrame")]
    fn ruffle_get_current_frame(cast_lib: i32, cast_member: i32) -> i32;

    #[wasm_bindgen(js_name = "ruffleCallFrame")]
    fn ruffle_call_frame(cast_lib: i32, cast_member: i32, frame: i32);

    #[wasm_bindgen(js_name = "ruffleHitTest")]
    fn ruffle_hit_test(cast_lib: i32, cast_member: i32, x: f64, y: f64) -> bool;

    #[wasm_bindgen(js_name = "ruffleGetFlashProperty", catch)]
    fn ruffle_get_flash_property(cast_lib: i32, cast_member: i32, target: &str, prop_num: i32) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = "ruffleSetFlashProperty")]
    fn ruffle_set_flash_property(cast_lib: i32, cast_member: i32, target: &str, prop_num: i32, value: &str);
}

pub struct BuiltInHandlerManager {}

impl BuiltInHandlerManager {
    /// Resolve a sprite datum to (cast_lib, cast_member) for Flash bridge calls.
    fn resolve_flash_member(datum_ref: &DatumRef) -> Result<Option<(i32, i32)>, ScriptError> {
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
                Some(member_ref) => Ok(Some((member_ref.cast_lib as i32, member_ref.cast_member as i32))),
                None => Ok(None),
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

    fn get_pos_global(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        // getPos(list, value) - find position of value in list
        use crate::player::compare::datum_equals;
        reserve_player_mut(|player| {
            let list_datum = player.get_datum(&args[0]);
            let search_ref = &args[1];
            match list_datum {
                Datum::List(_, items, _) => {
                    let items = items.clone();
                    for (i, item_ref) in items.iter().enumerate() {
                        let item = player.get_datum(item_ref);
                        let search = player.get_datum(search_ref);
                        if datum_equals(item, search, &player.allocator)? {
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
                        if datum_equals(val, search, &player.allocator)? {
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
            let handler_name = handler_name.string_value()?;

            let (instance_ids, list_count) = match receiver_clone {
                Datum::PropList(prop_list, ..) => {
                    let mut instance_ids = vec![];
                    let count = prop_list.len();
                    for (_, value_ref) in prop_list {
                        instance_ids.extend(get_datum_script_instance_ids(&value_ref, player)?);
                    }
                    (Ok(Some(instance_ids)), count)
                }
                Datum::List(_, list, _) => {
                    let mut instance_ids = vec![];
                    let count = list.len();
                    for value_ref in list {
                        instance_ids.extend(get_datum_script_instance_ids(&value_ref, player)?);
                    }
                    (Ok(Some(instance_ids)), count)
                }
                _ => (Ok(None), 0),
            };

            Ok((handler_name, args, instance_ids?, list_count))
        })?;

        if instance_ids.is_none() {
            return player_call_datum_handler(&receiver_ref, &handler_name, &args).await;
        }
        let instance_refs = instance_ids.unwrap();

        let mut result = player_alloc_datum(Datum::Null);
        for instance_ref in instance_refs {
            let handler = reserve_player_ref(|player| {
                ScriptInstanceUtils::get_script_instance_handler(
                    &handler_name,
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

    pub fn has_async_handler(name: &str) -> bool {
        match name {
            "call" => true,
            "new" => true,
            "newObject" => true,
            "callAncestor" => true,
            "sendSprite" => true,
            "sendAllSprites" => true,
            "value" => true,
            "do" => true,
            "updateStage" => true,
            "go" => true,
            "nothing" => true,
            _ => false,
        }
    }

    pub async fn call_async_handler(
        name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match name {
            "call" => Self::call(args).await,
            "new" => TypeHandlers::new(args).await,
            "newObject" => TypeHandlers::new_object(args).await,
            "callAncestor" => TypeHandlers::call_ancestor(args).await,
            "sendSprite" => MovieHandlers::send_sprite(args).await,
            "sendAllSprites" => MovieHandlers::send_all_sprites(args).await,
            "value" => TypeHandlers::value(args).await,
            "do" => Self::do_command(args).await,
            "updateStage" => MovieHandlers::update_stage(args).await,
            "go" => MovieHandlers::go(args).await,
            "nothing" => MovieHandlers::nothing_async(args).await,
            _ => {
                let msg = format!("No built-in async handler: {}", name);
                return Err(ScriptError::new(msg));
            }
        }
    }

    pub fn call_handler(name: &str, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        match name.to_lowercase().as_str() {
            "castlib" => CastHandlers::cast_lib(args),
            "findempty" => CastHandlers::find_empty(args),
            "preloadnetthing" => NetHandlers::preload_net_thing(args),
            "netdone" => NetHandlers::net_done(args),
            "movetofront" | "preloadmember" | "preloadbuffer" | "unloadmember" | "beep" => Ok(DatumRef::Void),
            "puppettempo" => MovieHandlers::puppet_tempo(args),
            "objectp" => TypeHandlers::objectp(args),
            "voidp" => TypeHandlers::voidp(args),
            "listp" => TypeHandlers::listp(args),
            "symbolp" => TypeHandlers::symbolp(args),
            "stringp" => TypeHandlers::stringp(args),
            "integerp" => TypeHandlers::integerp(args),
            "floatp" => TypeHandlers::floatp(args),
            "offset" => StringHandlers::offset(args),
            "length" => StringHandlers::length(args),
            "script" => MovieHandlers::script(args),
            "void" => TypeHandlers::void(args),
            "param" => Self::param(args),
            "count" => Self::count(args),
            "getat" => Self::get_at(args),
            "getlast" => Self::get_last(args),
            "getpos" => Self::get_pos_global(args),
            "setat" => Self::set_at(args),
            "ilk" => TypeHandlers::ilk(args),
            "member" => MovieHandlers::member(args),
            "space" => StringHandlers::space(args),
            "integer" => TypeHandlers::integer(args),
            "string" => StringHandlers::string(args),
            "chartonum" => StringHandlers::char_to_num(args),
            "numtochar" => StringHandlers::num_to_char(args),
            "float" => TypeHandlers::float(args),
            "put" => Self::put(args),
            "inspect" => Self::inspect(args),
            "random" => Self::random(args),
            "bitand" => Self::bit_and(args),
            "bitor" => Self::bit_or(args),
            "bitnot" => Self::bit_not(args),
            "symbol" => TypeHandlers::symbol(args),
            "puppetsprite" => MovieHandlers::puppet_sprite(args),
            "clearglobals" => Self::clear_globals(args),
            "sprite" => MovieHandlers::sprite(args),
            "point" => TypeHandlers::point(args),
            "clickloc" => {
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::Point([player.movie.click_loc.0 as f64, player.movie.click_loc.1 as f64], 0)))
                })
            }
            "constrainh" => {
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
            "constrainv" => {
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
            // stop/play/rewind/sound on a member or channel — no-op in web player
            "stop" | "play" | "rewind" | "pause"  => Ok(DatumRef::Void),
            "cursor" => TypeHandlers::cursor(args),
            "externalparamcount" => MovieHandlers::external_param_count(args),
            "externalparamname" => MovieHandlers::external_param_name(args),
            "externalparamvalue" => MovieHandlers::external_param_value(args),
            "getnettext" => NetHandlers::get_net_text(args),
            "timeout" => TypeHandlers::timeout(args),
            "rect" => TypeHandlers::rect(args),
            "getstreamstatus" => NetHandlers::get_stream_status(args),
            "neterror" => NetHandlers::net_error(args),
            "nettextresult" => NetHandlers::net_text_result(args),
            "postnettext" => NetHandlers::post_net_text(args),
            "rgb" => TypeHandlers::rgb(args),
            "list" => TypeHandlers::list(args),
            "image" => TypeHandlers::image(args),
            "chars" => StringHandlers::chars(args),
            "paletteindex" => TypeHandlers::palette_index(args),
            "abs" => TypeHandlers::abs(args),
            "xtra" => TypeHandlers::xtra(args),
            "stopevent" => MovieHandlers::stop_event(args),
            "getpref" => MovieHandlers::get_pref(args),
            "setpref" => MovieHandlers::set_pref(args),
            "urlencode" => StringHandlers::url_encode(args),
            "gotonetpage" => MovieHandlers::go_to_net_page(args),
            "gotonetmovie" => MovieHandlers::go_to_net_movie(args),
            "pass" => MovieHandlers::pass(args),
            "union" => TypeHandlers::union(args),
            "bitxor" => TypeHandlers::bit_xor(args),
            "power" => TypeHandlers::power(args),
            "add" => TypeHandlers::add(args),
            "abort" => Err(ScriptError::new_code(ScriptErrorCode::Abort, "abort".to_string())),
            "mousedown" => {
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(datum_bool(player.movie.mouse_down)))
                })
            }
            "rightmousedown" => {
                // We don't track right mouse state separately yet — return FALSE
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(datum_bool(false)))
                })
            }
            "getrendererservices" => {
                // Return a prop list with renderer info stubs
                reserve_player_mut(|player| {
                    let make_sym = |p: &mut DirPlayer, s: &str| p.alloc_datum(Datum::Symbol(s.to_string()));
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
            "getvariable" => {
                // Flash (SWF) member interop — getVariable(sprite, path)
                if args.len() >= 2 {
                    let member_ref = Self::resolve_flash_member(&args[0])?;
                    let path = reserve_player_ref(|player| {
                        player.get_datum(&args[1]).string_value()
                    })?;
                    if let Some((cast_lib, cast_member)) = member_ref {
                        match ruffle_get_variable(cast_lib, cast_member, &path) {
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
            "setvariable" => {
                // Flash (SWF) member interop — setVariable(sprite, path, value)
                if args.len() >= 3 {
                    let member_ref = Self::resolve_flash_member(&args[0])?;
                    let path = reserve_player_ref(|player| {
                        player.get_datum(&args[1]).string_value()
                    })?;
                    let value = reserve_player_ref(|player| {
                        player.get_datum(&args[2]).string_value()
                    })?;
                    if let Some((cast_lib, cast_member)) = member_ref {
                        if let Err(e) = ruffle_set_variable(cast_lib, cast_member, &path, &value) {
                            warn!("setVariable error: {:?}", e);
                        }
                    }
                }
                Ok(DatumRef::Void)
            }
            "gotoframe" => {
                // Flash (SWF) member interop — goToFrame(sprite, frame)
                if args.len() >= 2 {
                    let member_ref = Self::resolve_flash_member(&args[0])?;
                    let frame = reserve_player_ref(|player| {
                        player.get_datum(&args[1]).int_value()
                    })?;
                    if let Some((cast_lib, cast_member)) = member_ref {
                        ruffle_goto_frame(cast_lib, cast_member, frame);
                    }
                }
                Ok(DatumRef::Void)
            }
            "callframe" => {
                if args.len() >= 2 {
                    let member_ref = Self::resolve_flash_member(&args[0])?;
                    let frame = reserve_player_ref(|player| {
                        player.get_datum(&args[1]).int_value()
                    })?;
                    if let Some((cast_lib, cast_member)) = member_ref {
                        ruffle_call_frame(cast_lib, cast_member, frame);
                    }
                }
                Ok(DatumRef::Void)
            }
            "getflashproperty" => {
                if args.len() >= 3 {
                    let member_ref = Self::resolve_flash_member(&args[0])?;
                    let target = reserve_player_ref(|player| {
                        player.get_datum(&args[1]).string_value()
                    })?;
                    let prop_num = reserve_player_ref(|player| {
                        player.get_datum(&args[2]).int_value()
                    })?;
                    if let Some((cast_lib, cast_member)) = member_ref {
                        match ruffle_get_flash_property(cast_lib, cast_member, &target, prop_num) {
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
            "setflashproperty" => {
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
                    if let Some((cast_lib, cast_member)) = member_ref {
                        ruffle_set_flash_property(cast_lib, cast_member, &target, prop_num, &value);
                    }
                }
                Ok(DatumRef::Void)
            }
            "hittest" => {
                if args.len() >= 3 {
                    let member_ref = Self::resolve_flash_member(&args[0])?;
                    let x = reserve_player_ref(|player| {
                        player.get_datum(&args[1]).int_value()
                    })?;
                    let y = reserve_player_ref(|player| {
                        player.get_datum(&args[2]).int_value()
                    })?;
                    if let Some((cast_lib, cast_member)) = member_ref {
                        let result = ruffle_hit_test(cast_lib, cast_member, x as f64, y as f64);
                        return reserve_player_mut(|player| {
                            Ok(player.alloc_datum(Datum::Int(if result { 1 } else { 0 })))
                        });
                    }
                }
                Ok(DatumRef::Void)
            }
            "telltarget" => {
                if args.len() >= 2 {
                    // tellTarget is complex; for now just log it
                    let target = reserve_player_ref(|player| {
                        player.get_datum(&args[0]).string_value()
                    })?;
                    debug!("tellTarget: target={}", target);
                }
                Ok(DatumRef::Void)
            }
            "getaprop" => TypeHandlers::get_a_prop(args),
            "inside" => {
                let point = &args[0];
                let rect = &args[1..].to_vec();
                PointDatumHandlers::inside(point, rect)
            }
            "addprop" => {
                let list = &args[0];
                let args = &args[1..].to_vec();
                PropListDatumHandlers::add_prop(list, args)
            }
            "deleteprop" => {
                let list = &args[0];
                let args = &args[1..].to_vec();
                PropListDatumHandlers::delete_prop(list, args)
            }
            "append" => {
                let list = &args[0];
                let args = &args[1..].to_vec();
                ListDatumHandlers::append(list, args)
            }
            "deleteat" => reserve_player_mut(|player| {
                let list = &args[0];
                let args = &args[1..].to_vec();
                match player.get_datum(list) {
                    Datum::List(..) => ListDatumHandlers::delete_at(list, args),
                    Datum::PropList(..) => PropListDatumHandlers::delete_at(list, args),
                    _ => Err(ScriptError::new("Cannot delete at non list".to_string())),
                }
            }),
            "deleteone" => {
                let list = &args[0];
                let args = &args[1..].to_vec();
                ListDatumHandlers::delete_one(list, &args)
            }
            "deleteall" => {
                let list = &args[0];
                ListDatumHandlers::delete_all(list, &vec![])
            }
            "getone" => reserve_player_mut(|player| {
                let list = &args[0];
                let args = &args[1..].to_vec();
                match player.get_datum(list) {
                    Datum::List(..) => ListDatumHandlers::get_one(list, args),
                    Datum::PropList(..) => PropListDatumHandlers::get_one(list, args),
                    _ => Err(ScriptError::new("Cannot get one at non list".to_string())),
                }
            }),
            "findpos" => reserve_player_mut(|player| {
                let list = &args[0];
                let args = &args[1..].to_vec();
                match player.get_datum(list) {
                    Datum::List(..) => ListDatumHandlers::find_pos(list, &args),
                    Datum::PropList(..) => PropListDatumHandlers::find_pos(list, &args),
                    _ => Err(ScriptError::new("Cannot findPos on non-list".to_string())),
                }
            }),
            "setprop" => {
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
            "getpos" => reserve_player_mut(|player| {
                let list = &args[0];
                let args = &args[1..].to_vec();
                match player.get_datum(list) {
                    Datum::List(..) => ListDatumHandlers::find_pos(list, &args),
                    Datum::PropList(..) => PropListDatumHandlers::get_pos(list, &args),
                    _ => Err(ScriptError::new("Cannot getPos of non-list".to_string())),
                }
            }),
            "setaprop" => {
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
            "addat" => {
                let list = &args[0];
                let args = &args[1..].to_vec();
                ListDatumHandlers::add_at(list, args)
            }
            "getnodes" => Self::get_nodes(args),
            "duplicate" => {
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
                    _ => Err(ScriptError::new(format!("duplicate() not implemented for type {}", player.get_datum(item).type_str()))),
                })
            }
            "getprop" => {
                let list = &args[0];
                let args = &args[1..].to_vec();
                PropListDatumHandlers::get_prop(list, args)
            }
            "min" => TypeHandlers::min(args),
            "max" => TypeHandlers::max(args),
            "sort" => TypeHandlers::sort(args),
            "intersect" => TypeHandlers::intersect(args),
            "rollover" => MovieHandlers::rollover(args),
            "getpropat" => TypeHandlers::get_prop_at(args),
            "puppetsound" => MovieHandlers::puppet_sound(args),
            "pi" => TypeHandlers::pi(args),
            "sin" => TypeHandlers::sin(args),
            "cos" => TypeHandlers::cos(args),
            "sqrt" => TypeHandlers::sqrt(args),
            "tan" => TypeHandlers::tan(args),
            "atan" => TypeHandlers::atan(args),
            "sound" => TypeHandlers::sound(args),
            "vector" => TypeHandlers::vector(args),
            "transform" => TypeHandlers::transform3d(args),
            "color" => TypeHandlers::color(args),
            "date" => TypeHandlers::date(args),
            "keypressed" => Self::key_pressed(args),
            "showglobals" => Self::show_globals(),
            "tellstreamstatus" => Self::tell_stream_status(args),
            "frame" => {
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::Int(player.movie.current_frame as i32)))
                })
            }
            "label" => Self::label(args),
            "alert" => Self::alert(args),
            "objectp" => Self::object_p(args),
            "soundbusy" => TypeHandlers::sound_busy(args),
            "delay" => MovieHandlers::delay(args),
            "halt" => MovieHandlers::halt(args),
            "starttimer" => Self::start_timer(args),
            "externalevent" => Self::external_event(args),
            "dontpassevent" => Self::dont_pass_event(args),
            "frameready" => Self::frame_ready(args),
            "marker" => Self::marker(args),
            "play" => {
                // play member("name") - play a sound on channel 1
                if args.is_empty() {
                    return Ok(DatumRef::Void);
                }
                reserve_player_mut(|player| {
                    let channel_datum = player.alloc_datum(Datum::SoundChannel(1));
                    SoundChannelDatumHandlers::call(player, &channel_datum, &"play".to_string(), args)
                })
            }
            "spritebox" => {
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
            "puppettransition" => {
                log::warn!("puppetTransition is not implemented");
                Ok(DatumRef::Void)
            }
            "preload" => {
                log::warn!("preload is not implemented");
                Ok(DatumRef::Void)
            }
            "charpostoloc" => {
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

                    let align_lower = alignment.to_lowercase();
                    let align_kind: u8 = if align_lower == "center" || align_lower == "#center" {
                        1
                    } else if align_lower == "right" || align_lower == "#right" {
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
                                t.tab_type == "right" || t.tab_type == "center"
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
                    web_sys::console::log_1(&format!(
                        "[KEY-STEER] keyPressed({}) = TRUE ({})",
                        key_code, if key_code == 123 { "LEFT" } else { "RIGHT" }
                    ).into());
                }
            }
            if is_pressed && (key_code == 125 || key_code == 126) {
                static KP_UD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                if KP_UD.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 3 {
                    web_sys::console::log_1(&format!(
                        "[KEY-DRIVE] keyPressed({}) = TRUE ({})",
                        key_code, if key_code == 126 { "UP" } else { "DOWN" }
                    ).into());
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
                Datum::String(marker_name) | Datum::Symbol(marker_name) => {
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
