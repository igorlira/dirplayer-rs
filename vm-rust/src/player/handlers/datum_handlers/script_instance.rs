use crate::{
    director::lingo::datum::{datum_bool, Datum},
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
                let value = player.get_datum(value).to_owned();
                match value {
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
                    _ => Err(ScriptError::new(
                        "Cannot set ancestor to non-script instance".to_string(),
                    )),
                }
            }
            _ => Err(ScriptError::new(format!(
                "Cannot setAt property {key} on script instance datum"
            ))),
        }
    }
}

impl ScriptInstanceDatumHandlers {
    pub fn has_async_handler(datum: &DatumRef, name: &String) -> Result<bool, ScriptError> {
        return reserve_player_ref(|player| {
            let handler_ref = ScriptInstanceUtils::get_handler(name, &datum, player)?;
            Ok(handler_ref.is_some())
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
            _ => Err(ScriptError::new_code(
                ScriptErrorCode::HandlerNotFound,
                format!("No handler {handler_name} for script instance datum"),
            )),
        }
    }
}
