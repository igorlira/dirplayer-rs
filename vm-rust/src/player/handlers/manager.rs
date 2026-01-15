use log::{warn, debug, error};

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
        datum_formatting::{format_concrete_datum, format_datum},
        handlers::datum_handlers::xml::XmlHelper,
        keyboard_map, player_alloc_datum, player_call_script_handler, reserve_player_mut,
        reserve_player_ref, player_call_global_handler,
        script_ref::ScriptInstanceRef,
        DatumRef, DirPlayer, ScriptError,
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
        reserve_player_mut(|player| {  // Changed to reserve_player_mut
            let obj = player.get_datum(&args[0]);
            let position = player.get_datum(&args[1]).int_value()?;
            let index = (position - 1) as usize;
            
            debug!(
                "getAt: list={}, index={}", 
                format_concrete_datum(obj, player),
                position
            );
            
            match obj {
                Datum::Point(arr) => {
                    if index >= 2 {
                        return Err(ScriptError::new(format!(
                            "point index {} out of bounds",
                            position
                        )));
                    }
                    Ok(arr[index].clone())
                }

                Datum::Rect(arr) => {
                    if index >= 4 {
                        return Err(ScriptError::new(format!(
                            "rect index {} out of bounds",
                            position
                        )));
                    }
                    Ok(arr[index].clone())
                }
                Datum::List(_, list, ..) => {
                    let index = (position - 1) as usize;
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

    fn set_at(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let list_ref = &args[0];
            let position = player.get_datum(&args[1]).int_value()?;
            let new_value = args[2].clone();
            let index = (position - 1) as usize;
            
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
                Datum::Point(arr) => {
                    if index >= 2 {
                        return Err(ScriptError::new(format!(
                            "point index {} out of bounds",
                            position
                        )));
                    }

                    // Validate that it's an Int
                    match new_value_datum {
                        Datum::Int(_) => arr[index] = new_value,
                        other => {
                            return Err(ScriptError::new(format!(
                                "Point component must be Int, got {}",
                                other.type_str()
                            )))
                        }
                    }

                    Ok(())
                }
                Datum::Rect(arr) => {
                    if index >= 4 {
                        return Err(ScriptError::new(format!(
                            "rect index {} out of bounds",
                            position
                        )));
                    }

                    match new_value_datum {
                        Datum::Int(_) => arr[index] = new_value,
                        other => {
                            return Err(ScriptError::new(format!(
                                "Rect component must be Int, got {}",
                                other.type_str()
                            )))
                        }
                    }

                    Ok(())
                }
                Datum::List(_, ref mut list, ..) => {
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
                Datum::PropList(ref mut prop_list, ..) => {
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
                JsApi::dispatch_debug_message("--");
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
            
            JsApi::dispatch_debug_message(&format!("-- {}", output));
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
                return Err(ScriptError::new(
                    "random: argument must be greater than 0".to_string(),
                ));
            }
            
            // Director's random(n) returns a value from 1 to n (inclusive)
            let random_value = js_sys::Math::random() * (max as f64);
            let random_int = random_value.floor() as i32 + 1;
            
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

    async fn do_command(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        // Get the code string from the first argument
        let code = reserve_player_ref(|player| player.get_datum(&args[0]).string_value())?;
        debug!("do: executing code: {}", code);

        let code = code.trim();

        // Determine handler name and argument references
        let (handler_name, arg_refs) = if let Some(paren_pos) = code.find('(') {
            let handler_name = code[..paren_pos].trim().to_string();
            let args_str = &code[paren_pos + 1..];

            if let Some(close_paren) = args_str.rfind(')') {
                let args_str = &args_str[..close_paren];
                let arg_refs = if args_str.trim().is_empty() {
                    vec![]
                } else {
                    reserve_player_mut(|player| {
                        args_str.split(',')
                            .map(|arg| {
                                let arg = arg.trim();
                                if let Ok(i) = arg.parse::<i32>() {
                                    player.alloc_datum(Datum::Int(i))
                                } else if let Ok(f) = arg.parse::<f64>() {
                                    player.alloc_datum(Datum::Float(f))
                                } else if arg.starts_with('"') && arg.ends_with('"') {
                                    player.alloc_datum(Datum::String(arg[1..arg.len()-1].to_string()))
                                } else if arg.starts_with('#') {
                                    player.alloc_datum(Datum::Symbol(arg[1..].to_string()))
                                } else {
                                    player.alloc_datum(Datum::String(arg.to_string()))
                                }
                            })
                            .collect()
                    })
                };
                (handler_name, arg_refs)
            } else {
                (code.to_string(), vec![])
            }
        } else {
            (code.to_string(), vec![])
        };

        // Call the global handler
        let result = player_call_global_handler(&handler_name, &arg_refs).await;

        // Log the result
        match &result {
            Ok(datum_ref) => {
                let formatted_res: Result<String, ScriptError> = reserve_player_ref(|player| {
                    Ok(format_concrete_datum(player.get_datum(datum_ref), player))
                });

                match formatted_res {
                    Ok(formatted) => {
                        debug!("do completed: {}", formatted);
                    }
                    Err(_) => {
                        debug!("do completed: <formatting failed>");
                    }
                }
            }
            Err(e) => {
                error!("do failed: {}", e.message);
            }
        }

        result
    }

    pub fn has_async_handler(name: &String) -> bool {
        match name.as_str() {
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
            "value" => TypeHandlers::value(args).await,
            "do" => Self::do_command(args).await,
            "updateStage" => MovieHandlers::update_stage(args).await,
            "go" => MovieHandlers::go(args).await,
            _ => {
                let msg = format!("No built-in async handler: {}", name);
                return Err(ScriptError::new(msg));
            }
        }
    }

    pub fn call_handler(name: &String, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        match name.as_str().to_lowercase().as_str() {
            "castlib" => CastHandlers::cast_lib(args),
            "preloadnetthing" => NetHandlers::preload_net_thing(args),
            "netdone" => NetHandlers::net_done(args),
            "movetofront" => Ok(DatumRef::Void),
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
            "random" => Self::random(args),
            "bitand" => Self::bit_and(args),
            "bitor" => Self::bit_or(args),
            "bitnot" => Self::bit_not(args),
            "symbol" => TypeHandlers::symbol(args),
            "puppetsprite" => MovieHandlers::puppet_sprite(args),
            "clearglobals" => Self::clear_globals(args),
            "sprite" => MovieHandlers::sprite(args),
            "point" => TypeHandlers::point(args),
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
            "gotonetpage" => MovieHandlers::go_to_net_page(args),
            "pass" => MovieHandlers::pass(args),
            "union" => TypeHandlers::union(args),
            "bitxor" => TypeHandlers::bit_xor(args),
            "power" => TypeHandlers::power(args),
            "add" => TypeHandlers::add(args),
            "nothing" => TypeHandlers::nothing(args),
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
            "getone" => reserve_player_mut(|player| {
                let list = &args[0];
                let args = &args[1..].to_vec();
                match player.get_datum(list) {
                    Datum::List(..) => ListDatumHandlers::get_one(list, args),
                    Datum::PropList(..) => PropListDatumHandlers::get_one(list, args),
                    _ => Err(ScriptError::new("Cannot get one at non list".to_string())),
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
                    Datum::Point(arr) => {
                        let val0 = player.get_datum(&arr[0]).clone();
                        let val1 = player.get_datum(&arr[1]).clone();
                        
                        let new_arr: [DatumRef; 2] = [
                            player.alloc_datum(match val0 {
                                Datum::Int(n) => Datum::Int(n),
                                Datum::Float(f) => Datum::Float(f),
                                other => return Err(ScriptError::new(format!(
                                    "Point component must be numeric, got {}",
                                    other.type_str()
                                ))),
                            }),
                            player.alloc_datum(match val1 {
                                Datum::Int(n) => Datum::Int(n),
                                Datum::Float(f) => Datum::Float(f),
                                other => return Err(ScriptError::new(format!(
                                    "Point component must be numeric, got {}",
                                    other.type_str()
                                ))),
                            })
                        ];

                        Ok(player.alloc_datum(Datum::Point(new_arr)))
                    }
                    Datum::Rect(arr) => {
                        let val0 = player.get_datum(&arr[0]).clone();
                        let val1 = player.get_datum(&arr[1]).clone();
                        let val2 = player.get_datum(&arr[2]).clone();
                        let val3 = player.get_datum(&arr[3]).clone();
                        
                        let new_arr: [DatumRef; 4] = [
                            player.alloc_datum(match val0 {
                                Datum::Int(n) => Datum::Int(n),
                                Datum::Float(f) => Datum::Float(f),
                                other => return Err(ScriptError::new(format!(
                                    "Rect component must be numeric, got {}",
                                    other.type_str()
                                ))),
                            }),
                            player.alloc_datum(match val1 {
                                Datum::Int(n) => Datum::Int(n),
                                Datum::Float(f) => Datum::Float(f),
                                other => return Err(ScriptError::new(format!(
                                    "Rect component must be numeric, got {}",
                                    other.type_str()
                                ))),
                            }),
                            player.alloc_datum(match val2 {
                                Datum::Int(n) => Datum::Int(n),
                                Datum::Float(f) => Datum::Float(f),
                                other => return Err(ScriptError::new(format!(
                                    "Rect component must be numeric, got {}",
                                    other.type_str()
                                ))),
                            }),
                            player.alloc_datum(match val3 {
                                Datum::Int(n) => Datum::Int(n),
                                Datum::Float(f) => Datum::Float(f),
                                other => return Err(ScriptError::new(format!(
                                    "Rect component must be numeric, got {}",
                                    other.type_str()
                                ))),
                            }),
                        ];

                        Ok(player.alloc_datum(Datum::Rect(new_arr)))
                    }
                    Datum::String(s) => Ok(player.alloc_datum(Datum::String(s.clone()))),
                    Datum::Int(i) => Ok(player.alloc_datum(Datum::Int(*i))),
                    Datum::Float(f) => Ok(player.alloc_datum(Datum::Float(*f))),
                    Datum::Symbol(s) => Ok(player.alloc_datum(Datum::Symbol(s.clone()))),
                    _ => Err(ScriptError::new("duplicate() on non list not implemented".to_string())),
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
            "atan" => TypeHandlers::atan(args),
            "sound" => TypeHandlers::sound(args),
            "vector" => TypeHandlers::vector(args),
            "color" => TypeHandlers::color(args),
            "keypressed" => Self::key_pressed(args),
            "showglobals" => Self::show_globals(),
            "tellstreamstatus" => Self::tell_stream_status(args),
            "label" => Self::label(args),
            "alert" => Self::alert(args),
            "objectp" => Self::object_p(args),
            "soundbusy" => TypeHandlers::sound_busy(args),
            "halt" => MovieHandlers::halt(args),
            "starttimer" => Self::start_timer(args),
            "externalevent" => Self::external_event(args),
            "dontpassevent" => Self::dont_pass_event(args),
            "frameready" => Self::frame_ready(args),
            "marker" => Self::marker(args),
            _ => {
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
            JsApi::dispatch_debug_message(&format!("Alert: {}", message));
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
            JsApi::dispatch_debug_message("--- Global Variables ---");
            for (name, value) in &player.globals {
                let value = format_datum(value, player);
                JsApi::dispatch_debug_message(&format!("{} = {}", name, value));
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

    fn tell_stream_status(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        if args.len() < 1 {
            return Err(ScriptError::new(
                "tellStreamStatus requires 1 argument".to_string(),
            ));
        }

        reserve_player_mut(|player| {
            player.enable_stream_status_handler =
                player.get_datum(&args[0]).bool_value().unwrap_or(false);
            Ok(player.alloc_datum(Datum::Int(player.enable_stream_status_handler as i32)))
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
            
            web_sys::console::log_1(&format!("🔔 externalEvent: {}", event_string).into());
            
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
                // If argument is an integer, return the marker name at that frame
                Datum::Int(frame_num) => {
                    let frame_num = *frame_num as i32;
                    let marker = player
                        .movie
                        .score
                        .frame_labels
                        .iter()
                        .find(|label| label.frame_num == frame_num);
                    
                    if let Some(label) = marker {
                        Ok(player.alloc_datum(Datum::String(label.label.clone())))
                    } else {
                        Ok(player.alloc_datum(Datum::String(String::new())))
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
