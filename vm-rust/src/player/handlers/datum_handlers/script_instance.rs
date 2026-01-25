use crate::{
    director::lingo::datum::{datum_bool, Datum, DatumType},
    player::{
        allocator::ScriptInstanceAllocatorTrait,
        cast_lib::CastMemberRef,
        handlers::types::TypeUtils,
        player_call_script_handler, player_handle_scope_return, reserve_player_mut,
        reserve_player_ref,
        script::{script_get_prop, script_set_prop, Script, ScriptHandlerRef},
        script_ref::ScriptInstanceRef,
        DatumRef, DirPlayer, ScriptError, ScriptErrorCode,
    },
};

pub struct ScriptInstanceDatumHandlers {}
pub struct ScriptInstanceUtils {}

impl ScriptInstanceUtils {
    pub fn get_script<'a>(
        datum: &DatumRef,
        player: &'a DirPlayer,
    ) -> Result<(ScriptInstanceRef, &'a Script), ScriptError> {
        let datum = player.get_datum(datum);
        match datum {
            Datum::ScriptInstanceRef(instance_ref) => {
                let instance_id = **instance_ref;
                let instance = player
                    .allocator
                    .get_script_instance_opt(&instance_ref)
                    .ok_or(ScriptError::new(format!(
                        "Script instance {instance_id} not found"
                    )))?;
                let script = player
                    .movie
                    .cast_manager
                    .get_script_by_ref(&instance.script)
                    .ok_or(ScriptError::new(format!("Script not found")))?;
                Ok((instance_ref.clone(), script))
            }
            _ => Err(ScriptError::new(format!(
                "Cannot get script from non-script instance ({})",
                datum.type_str()
            ))),
        }
    }

    #[allow(dead_code)]
    pub fn get_instance_script_def<'a>(
        instance_ref: &ScriptInstanceRef,
        player: &'a DirPlayer,
    ) -> &'a Script {
        let script_ref = player
            .allocator
            .get_script_instance(&instance_ref)
            .script
            .to_owned();
        let script = player
            .movie
            .cast_manager
            .get_script_by_ref(&script_ref)
            .unwrap();
        script
    }

    pub fn get_handler(
        name: &String,
        datum: &DatumRef,
        player: &DirPlayer,
    ) -> Result<Option<ScriptHandlerRef>, ScriptError> {
        // let script = ScriptInstanceUtils::get_script(datum, player)?;
        // Self::get_script_instance_handler(name, script, player)
        let datum = player.get_datum(datum);
        match datum {
            Datum::ScriptInstanceRef(instance_ref) => {
                Self::get_script_instance_handler(name, instance_ref, player)
            }
            _ => Err(ScriptError::new(format!(
                "Cannot get handler from non-script instance ({})",
                datum.type_str()
            ))),
        }
    }

    pub fn get_script_instance_handler(
        name: &String,
        instance_ref: &ScriptInstanceRef,
        player: &DirPlayer,
    ) -> Result<Option<ScriptHandlerRef>, ScriptError> {
        let instance = player.allocator.get_script_instance(instance_ref);
        let script = player
            .movie
            .cast_manager
            .get_script_by_ref(&instance.script)
            .unwrap();
        let own_handler = script.get_own_handler_ref(name);
        if let Some(own_handler) = own_handler {
            return Ok(Some(own_handler));
        }
        let script_instance = player.allocator.get_script_instance(instance_ref);
        let ancestor_instance_id = &script_instance.ancestor;
        if let Some(ancestor_instance_ref) = ancestor_instance_id {
            ScriptInstanceUtils::get_script_instance_handler(name, &ancestor_instance_ref, player)
        } else {
            Ok(None)
        }
    }

    pub fn get_handler_from_first_arg(
        args: &Vec<DatumRef>,
        handler_name: &String,
    ) -> Option<(Option<ScriptInstanceRef>, (CastMemberRef, String))> {
        reserve_player_mut(|player| {
            let receiver_handler = args
                .first()
                .and_then(|first_arg| Some(player.get_datum(first_arg)))
                .map(|first_arg| match first_arg {
                    Datum::ScriptRef(script_ref) => {
                        let script = player
                            .movie
                            .cast_manager
                            .get_script_by_ref(&script_ref)
                            .unwrap();
                        script
                            .get_own_handler_ref(&handler_name)
                            .map(|handler| (None, handler))
                    }
                    Datum::ScriptInstanceRef(script_instance_ref) => {
                        ScriptInstanceUtils::get_script_instance_handler(
                            handler_name,
                            &script_instance_ref,
                            player,
                        )
                        .ok()
                        .flatten()
                        .map(|handler| (Some(script_instance_ref.clone()), handler))
                    }
                    Datum::SpriteRef(sprite_num) => {
                        // When the first arg is a sprite, look for the handler in the sprite's behaviors
                        let channel = player.movie.score.get_channel(*sprite_num);
                        // Search through the sprite's behavior instances for a handler
                        for instance_ref in &channel.sprite.script_instance_list {
                            if let Ok(Some(handler)) = ScriptInstanceUtils::get_script_instance_handler(
                                handler_name,
                                instance_ref,
                                player,
                            ) {
                                return Some((Some(instance_ref.clone()), handler));
                            }
                        }
                        None
                    }
                    _ => None,
                })
                .flatten();
            receiver_handler
        })
    }

    pub fn set_at(
        datum: &DatumRef,
        key: &String,
        value: &DatumRef,
        player: &mut DirPlayer,
    ) -> Result<(), ScriptError> {
        let self_instance_id = match player.get_datum(datum) {
            Datum::ScriptInstanceRef(instance_ref) => instance_ref.clone(),
            _ => {
                return Err(ScriptError::new(
                    "Cannot set ancestor on non-script instance".to_string(),
                ))
            }
        };
        match key.as_str() {
            "ancestor" => {
                let value_datum = player.get_datum(value).to_owned();
                match value_datum {
                    Datum::Void => {
                        // FIXME: Setting ancestor to void seems to be a no-op.
                        Ok(())
                    }
                    Datum::ScriptInstanceRef(ancestor_instance_id) => {
                        let script_instance =
                            player.allocator.get_script_instance_mut(&self_instance_id);
                        script_instance.ancestor = Some(ancestor_instance_id);
                        Ok(())
                    }
                    // For non-ScriptInstanceRef ancestors (like TimeoutInstance),
                    // store in properties map so method calls can be delegated
                    _ => {
                        let script_instance =
                            player.allocator.get_script_instance_mut(&self_instance_id);
                        script_instance.properties.insert("ancestor".to_string(), value.clone());
                        Ok(())
                    }
                }
            }
            _ => Err(ScriptError::new(format!(
                "Cannot setAt property {key} on script instance datum"
            ))),
        }
    }
}

impl ScriptInstanceDatumHandlers {
    /// Find a non-ScriptInstance ancestor (like TimeoutInstance) in the properties
    fn find_non_script_ancestor(datum: &DatumRef, player: &DirPlayer) -> Option<DatumRef> {
        let instance_ref = match player.get_datum(datum) {
            Datum::ScriptInstanceRef(ref r) => r.clone(),
            _ => return None,
        };

        // Walk the ancestor chain looking for non-ScriptInstance ancestors in properties
        let mut current_instance_ref = Some(instance_ref);
        let mut depth = 0;
        while let Some(ref inst_ref) = current_instance_ref {
            depth += 1;
            if depth > 100 {
                break;
            }

            let instance = player.allocator.get_script_instance(inst_ref);

            // Check if this instance has a non-ScriptInstance ancestor in properties
            if let Some(ancestor_prop_ref) = instance.properties.get("ancestor") {
                let ancestor_datum = player.get_datum(ancestor_prop_ref);
                match ancestor_datum {
                    // If ancestor is not a ScriptInstanceRef, return it for delegation
                    Datum::ScriptInstanceRef(ref next_ref) => {
                        current_instance_ref = Some(next_ref.clone());
                        continue;
                    }
                    Datum::Void | Datum::Int(0) => {
                        // No ancestor - continue to struct field
                    }
                    _ => {
                        // Non-ScriptInstance ancestor (e.g., TimeoutInstance)
                        return Some(ancestor_prop_ref.clone());
                    }
                }
            }

            // Check the struct field for ScriptInstance ancestors
            if let Some(ref ancestor_ref) = instance.ancestor {
                current_instance_ref = Some(ancestor_ref.clone());
            } else {
                break;
            }
        }

        None
    }

    pub fn has_async_handler(datum: &DatumRef, name: &String) -> Result<bool, ScriptError> {
        return reserve_player_ref(|player| {
            let handler_ref = ScriptInstanceUtils::get_handler(name, &datum, player)?;
            if handler_ref.is_some() {
                return Ok(true);
            }
            // Check if there's a non-ScriptInstance ancestor that might handle this
            if let Some(ancestor_ref) = Self::find_non_script_ancestor(datum, player) {
                let ancestor_datum = player.get_datum(&ancestor_ref);
                // For TimeoutInstance, check if the method is a timeout method
                if let Datum::TimeoutInstance { .. } = ancestor_datum {
                    if name == "forget" || name == "new" {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        });
    }

    pub async fn call_async(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let (instance_id, handler_ref) = reserve_player_ref(|player| {
            let handler_ref = ScriptInstanceUtils::get_handler(handler_name, datum, player);
            let datum = player.get_datum(datum);
            let instance_ref = match datum {
                Datum::ScriptInstanceRef(instance_id) => Some(instance_id),
                _ => None,
            }
            .unwrap();
            (instance_ref.clone(), handler_ref)
        });
        if let Some(handler_ref) = handler_ref? {
            let result_scope =
                player_call_script_handler(Some(instance_id), handler_ref, args).await?;
            player_handle_scope_return(&result_scope);
            Ok(result_scope.return_value)
        } else {
            // No handler found in script - check for special handlers first

            // getPropertyDescriptionList returns empty prop list if not implemented
            if handler_name == "getPropertyDescriptionList" {
                return reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::PropList(vec![], false)))
                });
            }

            // Director system events should be silently ignored if not implemented
            match handler_name.as_str() {
                "exitFrame" | "enterFrame" | "prepareFrame" | "idle" | "stepFrame" |
                "mouseDown" | "mouseUp" | "mouseEnter" | "mouseLeave" | "mouseWithin" |
                "keyDown" | "keyUp" | "beginSprite" | "endSprite" | "prepareMovie" |
                "startMovie" | "stopMovie" | "activate" | "deactivate" => {
                    return Ok(DatumRef::Void);
                }
                _ => {}
            }

            // Check for non-ScriptInstance ancestor to delegate to
            let (ancestor_ref, ancestor_type) = reserve_player_ref(|player| {
                if let Some(ancestor) = Self::find_non_script_ancestor(datum, player) {
                    let datum_type = player.get_datum(&ancestor).type_enum();
                    (Some(ancestor), Some(datum_type))
                } else {
                    (None, None)
                }
            });
            if let (Some(ancestor_ref), Some(ancestor_type)) = (ancestor_ref, ancestor_type) {
                // Delegate to the appropriate handler based on ancestor type
                use crate::director::lingo::datum::DatumType;
                use super::timeout::TimeoutDatumHandlers;

                match ancestor_type {
                    DatumType::TimeoutRef | DatumType::TimeoutInstance | DatumType::TimeoutFactory => {
                        // For timeouts, use the sync call handler which handles forget
                        // We avoid call_async to prevent async recursion issues
                        return TimeoutDatumHandlers::call(&ancestor_ref, handler_name, args);
                    }
                    _ => {
                        // Other datum types not yet supported for delegation
                    }
                }
            }
            Err(ScriptError::new(format!(
                "No async handler {handler_name} for script instance datum"
            )))
        }
    }

    fn get_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let key = player.get_datum(&args[0]).string_value()?;
            match key.as_str() {
                "ancestor" => {
                    let datum = player.get_datum(datum);
                    let script_instance = player
                        .allocator
                        .get_script_instance(datum.to_script_instance_ref()?);
                    Ok(
                        player.alloc_datum(if let Some(ancestor) = &script_instance.ancestor {
                            Datum::ScriptInstanceRef(ancestor.clone())
                        } else {
                            Datum::Int(0)
                        }),
                    )
                }
                _ => Self::get_a_prop(datum, args),
            }
        })
    }

    fn set_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let key = player.get_datum(&args[0]).string_value()?;
            let value_ref = &args[1];

            ScriptInstanceUtils::set_at(datum, &key, &value_ref, player)?;
            Ok(DatumRef::Void)
        })
    }

    pub fn set_a_prop(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let prop_name = player.get_datum(&args[0]).string_value()?;
            let value_ref = &args[1];

            let instance_ref = match player.get_datum(datum) {
                Datum::ScriptInstanceRef(instance_ref) => instance_ref.clone(),
                _ => {
                    return Err(ScriptError::new(
                        "Cannot set property on non-script instance".to_string(),
                    ))
                }
            };
            script_set_prop(player, &instance_ref, &prop_name, &value_ref, false)
                .map(|_| DatumRef::Void)
        })
    }

    pub fn get_prop(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let list_prop_name_ref = &args[1];

            let local_prop_name = player.get_datum(&args[0]).string_value()?;
            let instance_ref = match player.get_datum(datum) {
                Datum::ScriptInstanceRef(instance_ref) => instance_ref.clone(),
                _ => {
                    return Err(ScriptError::new(
                        "Cannot get property on non-script instance".to_string(),
                    ))
                }
            };

            let local_prop_ref = script_get_prop(player, &instance_ref, &local_prop_name)?;
            let result = TypeUtils::get_sub_prop(&local_prop_ref, &list_prop_name_ref, player)?;
            Ok(result)
        })
    }

    pub fn set_prop(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let list_prop_name_ref = &args[1];
            let value_ref = &args[2];

            let local_prop_name = player.get_datum(&args[0]).string_value()?;
            let instance_ref = match player.get_datum(datum) {
                Datum::ScriptInstanceRef(instance_ref) => instance_ref.clone(),
                _ => {
                    return Err(ScriptError::new(
                        "Cannot set property on non-script instance".to_string(),
                    ))
                }
            };

            let local_prop_ref = script_get_prop(player, &instance_ref, &local_prop_name)?;
            TypeUtils::set_sub_prop(&local_prop_ref, &list_prop_name_ref, &value_ref, player)?;

            Ok(DatumRef::Void)
        })
    }

    pub fn handler(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let name = player.get_datum(&args[0]).string_value()?;
            let (_, script) = ScriptInstanceUtils::get_script(datum, player)?;
            let own_handler = script.get_own_handler(&name);
            Ok(player.alloc_datum(datum_bool(own_handler.is_some())))
        })
    }

    pub fn count(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let instance_ref = match player.get_datum(datum) {
                Datum::ScriptInstanceRef(instance_ref) => instance_ref.clone(),
                _ => {
                    return Err(ScriptError::new(
                        "Cannot count non-script instance".to_string(),
                    ))
                }
            };
            let prop_name = player.get_datum(&args[0]).string_value()?;
            let prop_value = script_get_prop(player, &instance_ref, &prop_name)?;
            let prop_value_datum = player.get_datum(&prop_value);
            let count = match prop_value_datum {
                Datum::List(_, list, _) => list.len(),
                Datum::PropList(prop_list, ..) => prop_list.len(),
                _ => {
                    return Err(ScriptError::new(
                        "Cannot count non-list property".to_string(),
                    ))
                }
            };
            Ok(player.alloc_datum(Datum::Int(count as i32)))
        })
    }

    pub fn get_a_prop(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let prop_name = player.get_datum(&args[0]).string_value()?;
            let instance_ref = match player.get_datum(datum) {
                Datum::ScriptInstanceRef(instance_ref) => instance_ref.clone(),
                _ => {
                    return Err(ScriptError::new(
                        "Cannot get property on non-script instance".to_string(),
                    ))
                }
            };
            let prop_value = script_get_prop(player, &instance_ref, &prop_name)?;
            Ok(prop_value)
        })
    }

    pub fn handlers(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let script_instance_ref = match player.get_datum(datum) {
                Datum::ScriptInstanceRef(instance_ref) => instance_ref.clone(),
                _ => {
                    return Err(ScriptError::new(
                        "Cannot get handlers of non-script instance".to_string(),
                    ))
                }
            };
            let script_instance = player.allocator.get_script_instance(&script_instance_ref);
            let script = player
                .movie
                .cast_manager
                .get_script_by_ref(&script_instance.script)
                .unwrap();
            let handler_names = script.handler_names.clone();
            let handler_name_datums = handler_names
                .iter()
                .map(|name| player.alloc_datum(Datum::Symbol(name.clone())))
                .collect();
            Ok(player.alloc_datum(Datum::List(DatumType::List, handler_name_datums, false)))
        })
    }

    pub fn call(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "setAt" => Self::set_at(datum, args),
            "handler" => Self::handler(datum, args),
            "setaProp" => Self::set_a_prop(datum, args),
            "setProp" => Self::set_prop(datum, args),
            "getProp" => Self::get_prop(datum, args),
            "getPropRef" => Self::get_prop(datum, args),
            "getaProp" => Self::get_a_prop(datum, args),
            "getAt" => Self::get_at(datum, args),
            "count" => Self::count(datum, args),
            "handlers" => Self::handlers(datum, args),
            // getPropertyDescriptionList returns empty prop list if not implemented
            "getPropertyDescriptionList" => {
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::PropList(vec![], false)))
                })
            }
            // Director system events that should be silently ignored if not implemented
            "exitFrame" | "enterFrame" | "prepareFrame" | "idle" | "stepFrame" |
            "mouseDown" | "mouseUp" | "mouseEnter" | "mouseLeave" | "mouseWithin" |
            "keyDown" | "keyUp" | "beginSprite" | "endSprite" | "prepareMovie" |
            "startMovie" | "stopMovie" | "activate" | "deactivate" |
            // forget is called on wrapper objects that may not have it - silently ignore
            "forget" => {
                Ok(DatumRef::Void)
            }
            _ => {
                // Check for non-ScriptInstance ancestor to delegate to (e.g., TimeoutInstance)
                let (ancestor_ref, ancestor_type, script_name) = reserve_player_ref(|player| {
                    let script_name = if let Datum::ScriptInstanceRef(ref inst_ref) = player.get_datum(datum) {
                        let instance = player.allocator.get_script_instance(inst_ref);
                        player.movie.cast_manager.get_script_by_ref(&instance.script)
                            .map(|s| s.name.clone())
                            .unwrap_or_else(|| "unknown".to_string())
                    } else {
                        "not-script-instance".to_string()
                    };

                    if let Some(ancestor) = Self::find_non_script_ancestor(datum, player) {
                        let datum_type = player.get_datum(&ancestor).type_enum();
                        (Some(ancestor), Some(datum_type), script_name)
                    } else {
                        (None, None, script_name)
                    }
                });

                if let (Some(ancestor_ref), Some(ancestor_type)) = (ancestor_ref, ancestor_type) {
                    use crate::director::lingo::datum::DatumType;
                    use super::timeout::TimeoutDatumHandlers;

                    match ancestor_type {
                        DatumType::TimeoutRef | DatumType::TimeoutInstance | DatumType::TimeoutFactory => {
                            return TimeoutDatumHandlers::call(&ancestor_ref, handler_name, args);
                        }
                        _ => {}
                    }
                }

                // Log once when we hit this error for debugging
                static LOGGED_ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                if !LOGGED_ONCE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    crate::console_warn!("ScriptInstance call error: handler={}, script={}", handler_name, script_name);
                }
                Err(ScriptError::new_code(
                    ScriptErrorCode::HandlerNotFound,
                    format!("No handler {handler_name} for script instance datum"),
                ))
            }
        }
    }

    /// Director's forget() method removes the script instance from the actorList.
    /// This is commonly used with script-based timeouts (like _TIMER_) that are stored in actorList.
    fn forget(datum: &DatumRef, _args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            // Get the actorList
            let actor_list_ref = player.globals.get("actorList").cloned();

            if let Some(actor_list_ref) = actor_list_ref {
                let actor_list = player.get_datum(&actor_list_ref).clone();
                if let Datum::List(dtype, items, sorted) = actor_list {
                    // Get the instance ID we're looking for
                    let target_id = match player.get_datum(datum) {
                        Datum::ScriptInstanceRef(ref instance_ref) => Some(**instance_ref),
                        _ => None,
                    };

                    if let Some(target_id) = target_id {
                        // Find and remove the instance from the list
                        let new_items: Vec<DatumRef> = items.iter()
                            .filter(|item| {
                                match player.get_datum(item) {
                                    Datum::ScriptInstanceRef(ref item_ref) => **item_ref != target_id,
                                    _ => true, // Keep non-script-instance items
                                }
                            })
                            .cloned()
                            .collect();

                        // Update the actorList with the filtered list
                        let new_list = Datum::List(dtype, new_items, sorted);
                        let new_list_ref = player.alloc_datum(new_list);
                        player.globals.insert("actorList".to_string(), new_list_ref);
                    }
                }
            }

            Ok(DatumRef::Void)
        })
    }
}
