use crate::{
    director::lingo::datum::{datum_bool, Datum},
    player::{
        allocator::ScriptInstanceAllocatorTrait,
        cast_lib::CastMemberRef,
        player_call_script_handler, player_handle_scope_return, reserve_player_mut,
        reserve_player_ref,
        script::{get_lctx_for_script, ScriptInstance},
        script_ref::ScriptInstanceRef,
        DatumRef, ScriptError, ScriptErrorCode,
    },
};
pub struct ScriptDatumHandlers {}

impl ScriptDatumHandlers {
    pub fn has_async_handler(obj_ref: &DatumRef, name: &String) -> bool {
        match name.as_str() {
            "new" => true,
            "handler" => false,
            _ => {
                // Check if the script has a handler with this name
                reserve_player_ref(|player| {
                    if let Datum::ScriptRef(script_ref) = player.get_datum(obj_ref) {
                        if let Some(script_rc) =
                            player.movie.cast_manager.get_script_by_ref(script_ref)
                        {
                            let script = script_rc.as_ref();
                            return script.get_own_handler(name).is_some();
                        }
                    }
                    false
                })
            }
        }
    }

    pub async fn call_async(
        obj_ref: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "new" => Self::new(obj_ref, args).await,
            _ => {
                // Try to call a handler defined in the script itself
                let handler_ref = reserve_player_ref(|player| {
                    let script_ref = match player.get_datum(obj_ref) {
                        Datum::ScriptRef(script_ref) => script_ref.clone(),
                        _ => return Err(ScriptError::new("Expected script reference".to_string())),
                    };
                    Ok::<_, ScriptError>((script_ref, handler_name.clone()))
                })?;

                // Check if the script actually has this handler
                let has_handler = reserve_player_ref(|player| {
                    if let Datum::ScriptRef(script_ref) = player.get_datum(obj_ref) {
                        if let Some(script_rc) =
                            player.movie.cast_manager.get_script_by_ref(script_ref)
                        {
                            let script = script_rc.as_ref();
                            return script.get_own_handler(handler_name).is_some();
                        }
                    }
                    false
                });

                if !has_handler {
                    return Err(ScriptError::new_code(
                        ScriptErrorCode::HandlerNotFound,
                        format!("No handler {} for script datum", handler_name),
                    ));
                }

                // Call with no receiver (None) - the script itself becomes "me"
                let result = player_call_script_handler(None, handler_ref, args).await?;
                Ok(result.return_value)
            }
        }
    }

    pub fn call(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "handler" => Self::handler(datum, args),
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for script datum"
            ))),
        }
    }

    pub fn handler(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let name = player.get_datum(&args[0]).string_value()?;
            let script_ref = match player.get_datum(datum) {
                Datum::ScriptRef(script_ref) => script_ref,
                _ => {
                    return Err(ScriptError::new(
                        "Cannot create new instance of non-script".to_string(),
                    ))
                }
            };
            let script = player
                .movie
                .cast_manager
                .get_script_by_ref(script_ref)
                .unwrap();
            let own_handler = script.get_own_handler(&name);
            Ok(player.alloc_datum(datum_bool(own_handler.is_some())))
        })
    }

    pub fn create_script_instance(script_ref: &CastMemberRef) -> (ScriptInstanceRef, DatumRef) {
        reserve_player_mut(|player| {
            let instance_id = player.allocator.get_free_script_instance_id();
            let script = player
                .movie
                .cast_manager
                .get_script_by_ref(script_ref)
                .unwrap();

            let lctx_ptr: *const crate::director::lingo::script::ScriptContext =
                get_lctx_for_script(player, script).unwrap() as *const _;

            let instance = ScriptInstance::new(
                instance_id,
                script_ref.to_owned(),
                script,
                unsafe { &*lctx_ptr }, // safe because lctx_ptr is still valid
            );

            let instance_ref = player.allocator.alloc_script_instance(instance);
            let datum_ref = player.alloc_datum(Datum::ScriptInstanceRef(instance_ref.clone()));
            (instance_ref, datum_ref)
        })
    }

    pub async fn new(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        let (script_ref, new_handler_ref, expected_param_count, script_name) =
            reserve_player_mut(|player| {
                let script_ref = match player.get_datum(datum) {
                    Datum::ScriptRef(script_ref) => script_ref,
                    _ => {
                        return Err(ScriptError::new(
                            "Cannot create new instance of non-script".to_string(),
                        ))
                    }
                };
                let script = player
                    .movie
                    .cast_manager
                    .get_script_by_ref(script_ref)
                    .unwrap();
                let new_handler_ref = script.get_own_handler_ref(&"new".to_string());

                let param_count = if let Some(_) = &new_handler_ref {
                    let handler_def = script.get_own_handler(&"new".to_string()).unwrap();
                    handler_def.argument_name_ids.len()
                } else {
                    0
                };

                Ok((
                    script_ref.clone(),
                    new_handler_ref,
                    param_count,
                    script.name.clone(),
                ))
            })?;

        let (instance_ref, datum_ref) = Self::create_script_instance(&script_ref);

        if let Some(new_handler_ref) = new_handler_ref {
            let mut padded_args = args.clone();
            while padded_args.len() < expected_param_count {
                padded_args.push(DatumRef::Void);
            }

            let result_scope =
                match player_call_script_handler(Some(instance_ref), new_handler_ref, &padded_args)
                    .await
                {
                    Ok(scope) => scope,
                    Err(err) => {
                        web_sys::console::log_1(
                            &format!("❌ Error in {}.new(): {}", script_name, err.message).into(),
                        );
                        return Err(err);
                    }
                };

            player_handle_scope_return(&result_scope);
            return Ok(result_scope.return_value);
        } else {
            return Ok(datum_ref);
        }
    }
}
