use log::warn;

use super::{
    cast::CastHandlers,
    datum_handlers::{
        list_handlers::ListDatumHandlers,
        player_call_datum_handler,
        point::PointDatumHandlers,
        prop_list::PropListDatumHandlers,
        script_instance::{ScriptInstanceDatumHandlers, ScriptInstanceUtils},
    },
    movie::MovieHandlers,
    net::NetHandlers,
    string::StringHandlers,
    types::TypeHandlers,
};
use crate::{
    director::lingo::datum::{datum_bool, Datum, DatumType},
    js_api::JsApi,
    player::{
        datum_formatting::format_concrete_datum, handlers::datum_handlers::xml::XmlHelper,
        keyboard_map, player_alloc_datum, player_call_script_handler, reserve_player_mut,
        reserve_player_ref, script_ref::ScriptInstanceRef, DatumRef, DirPlayer, ScriptError,
    },
};

pub struct BuiltInHandlerManager {}

impl BuiltInHandlerManager {
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
                _ => Err(ScriptError::new(format!("Cannot get count of non-list"))),
            }
        })
    }

    fn get_at(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_ref(|player| {
            let obj = player.get_datum(&args[0]);
            let position = player.get_datum(&args[1]).int_value()?;
            let index = position - 1;
            match obj {
                Datum::List(_, list, ..) => Ok(list[index as usize].clone()),
                Datum::PropList(prop_list, ..) => Ok(prop_list[index as usize].1.clone()),
                _ => Err(ScriptError::new(format!("Cannot getAt of non-list"))),
            }
        })
    }

    fn put(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_ref(|player| {
            let mut line = String::new();
            let mut i = 0;
            for arg in args {
                if i > 0 {
                    line.push_str(" ");
                }
                let arg = player.get_datum(arg);
                line.push_str(&format_concrete_datum(&arg, player));
                i += 1;
            }
            JsApi::dispatch_debug_message(line.as_str());
            Ok(())
        })?;
        Ok(DatumRef::Void)
    }

    fn clear_globals(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            player.globals.clear();
            Ok(DatumRef::Void)
        })
    }

    fn random(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let min: i32 = 1;
            let max = player.get_datum(&args[0]).int_value()? - 1;
            if max < 0 {
                return Err(ScriptError::new(
                    "random: max must be greater than or equal to 0".to_string(),
                ));
            }
            let max = max as f64;
            let random = js_sys::Math::random() * max as f64;
            let random = random.floor() as i32;
            let random = random + min;
            Ok(player.alloc_datum(Datum::Int(random)))
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
        let (handler_name, args, instance_ids) = reserve_player_mut(|player| {
            let handler_name = player.get_datum(&args[0]);
            let receiver_clone = player.get_datum(receiver_ref).clone();
            let args = args[2..].to_vec();
            if !handler_name.is_symbol() {
                return Err(ScriptError::new(
                    "Handler name must be a symbol".to_string(),
                ));
            }
            let handler_name = handler_name.string_value()?;

            let instance_ids = match receiver_clone {
                Datum::PropList(prop_list, ..) => {
                    let mut instance_ids = vec![];
                    for (_, value_ref) in prop_list {
                        instance_ids.extend(get_datum_script_instance_ids(&value_ref, player)?);
                    }
                    Ok(Some(instance_ids))
                }
                Datum::List(_, list, _) => {
                    let mut instance_ids = vec![];
                    for value_ref in list {
                        instance_ids.extend(get_datum_script_instance_ids(&value_ref, player)?);
                    }
                    Ok(Some(instance_ids))
                }
                _ => Ok(None),
            }?;

            Ok((handler_name, args, instance_ids))
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

    pub fn has_async_handler(name: &String) -> bool {
        match name.as_str() {
            "call" => true,
            "new" => true,
            "newObject" => true,
            "callAncestor" => true,
            "sendSprite" => true,
            "sendAllSprites" => true,
            _ => false,
        }
    }

    pub async fn call_async_handler(
        name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match name.as_str() {
            "call" => Self::call(args).await,
            "new" => TypeHandlers::new(args).await,
            "newObject" => TypeHandlers::new_object(args).await,
            "callAncestor" => TypeHandlers::call_ancestor(args).await,
            "sendSprite" => MovieHandlers::send_sprite(args).await,
            "sendAllSprites" => MovieHandlers::send_all_sprites(args).await,
            _ => {
                let msg = format!("No built-in async handler: {}", name);
                return Err(ScriptError::new(msg));
            }
        }
    }

    pub fn call_handler(name: &String, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        match name.as_str() {
            "castLib" => CastHandlers::cast_lib(args),
            "preloadNetThing" => NetHandlers::preload_net_thing(args),
            "netDone" => NetHandlers::net_done(args),
            "moveToFront" => Ok(DatumRef::Void),
            "puppetTempo" => MovieHandlers::puppet_tempo(args),
            "objectp" => TypeHandlers::objectp(args),
            "voidp" => TypeHandlers::voidp(args),
            "listp" => TypeHandlers::listp(args),
            "symbolp" => TypeHandlers::symbolp(args),
            "stringp" => TypeHandlers::stringp(args),
            "integerp" => TypeHandlers::integerp(args),
            "floatp" => TypeHandlers::floatp(args),
            "offset" => StringHandlers::offset(args),
            "length" => StringHandlers::length(args),
            "value" => TypeHandlers::value(args),
            "script" => MovieHandlers::script(args),
            "void" => TypeHandlers::void(args),
            "param" => Self::param(args),
            "count" => Self::count(args),
            "getAt" => Self::get_at(args),
            "ilk" => TypeHandlers::ilk(args),
            "member" => MovieHandlers::member(args),
            "space" => StringHandlers::space(args),
            "integer" => TypeHandlers::integer(args),
            "string" => StringHandlers::string(args),
            "charToNum" => StringHandlers::char_to_num(args),
            "numToChar" => StringHandlers::num_to_char(args),
            "float" => TypeHandlers::float(args),
            "put" => Self::put(args),
            "random" => Self::random(args),
            "bitAnd" => Self::bit_and(args),
            "bitOr" => Self::bit_or(args),
            "bitNot" => Self::bit_not(args),
            "symbol" => TypeHandlers::symbol(args),
            "go" => MovieHandlers::go(args),
            "puppetSprite" => MovieHandlers::puppet_sprite(args),
            "clearGlobals" => Self::clear_globals(args),
            "sprite" => MovieHandlers::sprite(args),
            "point" => TypeHandlers::point(args),
            "cursor" => TypeHandlers::cursor(args),
            "externalParamName" => MovieHandlers::external_param_name(args),
            "externalParamValue" => MovieHandlers::external_param_value(args),
            "getNetText" => NetHandlers::get_net_text(args),
            "timeout" => TypeHandlers::timeout(args),
            "rect" => TypeHandlers::rect(args),
            "getStreamStatus" => NetHandlers::get_stream_status(args),
            "netError" => NetHandlers::net_error(args),
            "netTextresult" => NetHandlers::net_text_result(args),
            "netTextResult" => NetHandlers::net_text_result(args),
            "postNetText" => NetHandlers::post_net_text(args),
            "rgb" => TypeHandlers::rgb(args),
            "list" => TypeHandlers::list(args),
            "image" => TypeHandlers::image(args),
            "chars" => StringHandlers::chars(args),
            "paletteIndex" => TypeHandlers::palette_index(args),
            "abs" => TypeHandlers::abs(args),
            "xtra" => TypeHandlers::xtra(args),
            "stopEvent" => MovieHandlers::stop_event(args),
            "getPref" => MovieHandlers::get_pref(args),
            "setPref" => MovieHandlers::set_pref(args),
            "gotoNetPage" => MovieHandlers::go_to_net_page(args),
            "pass" => MovieHandlers::pass(args),
            "union" => TypeHandlers::union(args),
            "bitXor" => TypeHandlers::bit_xor(args),
            "power" => TypeHandlers::power(args),
            "add" => TypeHandlers::add(args),
            "nothing" => TypeHandlers::nothing(args),
            "updateStage" => MovieHandlers::update_stage(args),
            "getaProp" => TypeHandlers::get_a_prop(args),
            "inside" => {
                let point = &args[0];
                let rect = &args[1..].to_vec();
                PointDatumHandlers::inside(point, rect)
            }
            "addProp" => {
                let list = &args[0];
                let args = &args[1..].to_vec();
                PropListDatumHandlers::add_prop(list, args)
            }
            "append" => {
                let list = &args[0];
                let args = &args[1..].to_vec();
                ListDatumHandlers::append(list, args)
            }
            "deleteAt" => reserve_player_mut(|player| {
                let list = &args[0];
                let args = &args[1..].to_vec();
                match player.get_datum(list) {
                    Datum::List(..) => ListDatumHandlers::delete_at(list, args),
                    Datum::PropList(..) => PropListDatumHandlers::delete_at(list, args),
                    _ => Err(ScriptError::new("Cannot delete at non list".to_string())),
                }
            }),
            "getOne" => reserve_player_mut(|player| {
                let list = &args[0];
                let args = &args[1..].to_vec();
                match player.get_datum(list) {
                    Datum::List(..) => ListDatumHandlers::get_one(list, args),
                    Datum::PropList(..) => PropListDatumHandlers::get_one(list, args),
                    _ => Err(ScriptError::new("Cannot get one at non list".to_string())),
                }
            }),
            "setaProp" => {
                let datum = &args[0];
                let datum_type = reserve_player_ref(|player| player.get_datum(datum).type_enum());
                let args = &args[1..].to_vec();
                match datum_type {
                    DatumType::PropList => PropListDatumHandlers::set_opt_prop(datum, args),
                    DatumType::ScriptInstanceRef => {
                        ScriptInstanceDatumHandlers::set_a_prop(datum, args)
                    }
                    _ => Err(ScriptError::new(
                        "Cannot setaProp on non-prop list or child object".to_string(),
                    )),
                }
            }
            "addAt" => {
                let list = &args[0];
                let args = &args[1..].to_vec();
                ListDatumHandlers::add_at(list, args)
            }
            "getNodes" => Self::get_nodes(args),
            "duplicate" => {
                let item = &args[0];
                let args = &args[1..].to_vec();
                reserve_player_mut(|player| match player.get_datum(item) {
                    Datum::List(..) => ListDatumHandlers::duplicate(item, args),
                    Datum::PropList(..) => PropListDatumHandlers::duplicate(item, args),
                    _ => Err(ScriptError::new(
                        "duplicate() on non list not implemented".to_string(),
                    )),
                })
            }
            "getProp" => {
                let list = &args[0];
                let args = &args[1..].to_vec();
                PropListDatumHandlers::get_prop(list, args)
            }
            "min" => TypeHandlers::min(args),
            "max" => TypeHandlers::max(args),
            "sort" => TypeHandlers::sort(args),
            "intersect" => TypeHandlers::intersect(args),
            "rollover" => MovieHandlers::rollover(args),
            "getPropAt" => TypeHandlers::get_prop_at(args),
            "puppetSound" => Ok(DatumRef::Void), // TODO
            "pi" => TypeHandlers::pi(args),
            "sin" => TypeHandlers::sin(args),
            "cos" => TypeHandlers::cos(args),
            "sound" => TypeHandlers::sound(args),
            "vector" => TypeHandlers::vector(args),
            "color" => TypeHandlers::color(args),
            "keyPressed" => Self::key_pressed(args),
            _ => {
                let formatted_args = reserve_player_ref(|player| {
                    let mut formatted_args = String::new();
                    for arg in args {
                        if !formatted_args.is_empty() {
                            formatted_args.push_str(", ");
                        }
                        formatted_args
                            .push_str(&format_concrete_datum(&player.get_datum(arg), player));
                    }
                    Ok(formatted_args)
                })?;
                let msg = format!("No built-in handler: {}({})", name, formatted_args);
                warn!("{msg}");
                return Err(ScriptError::new(msg));
            }
        }
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
                        .unwrap()
                        .to_lowercase()
                        .next()
                        .unwrap();
                    let mapped_code = *keyboard_map::get_char_to_keycode_map()
                        .get(&ch)
                        .unwrap_or(&0);
                    mapped_code
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

            web_sys::console::log_1(
                &format!("🔧 getNodes called for node type: {}", node_name).into(),
            );

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
                matching_nodes,
                false,
            )))
        })
    }
}

fn get_datum_script_instance_ids(
    value_ref: &DatumRef,
    player: &DirPlayer,
) -> Result<Vec<ScriptInstanceRef>, ScriptError> {
    let value = player.get_datum(value_ref);
    let mut instance_refs = vec![];
    match value {
        Datum::ScriptInstanceRef(instance_id) => {
            instance_refs.push(instance_id.clone());
        }
        Datum::SpriteRef(sprite_id) => {
            let sprite = player.movie.score.get_sprite(*sprite_id).unwrap();
            instance_refs.extend(sprite.script_instance_list.clone());
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
