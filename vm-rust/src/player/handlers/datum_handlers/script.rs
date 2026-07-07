use std::collections::VecDeque;
use log::error;
use crate::{
    director::lingo::datum::{datum_bool, Datum, DatumType},
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
    pub fn has_async_handler(obj_ref: &DatumRef, name: &str) -> bool {
        match name {
            "new" => true,
            "rawNew" => false,
            "handler" => false,
            _ => {
                reserve_player_ref(|player| {
                    if let Datum::ScriptRef(script_ref) = player.get_datum(obj_ref) {
                        if let Some(script_rc) =
                            player.movie.cast_manager.get_script_by_ref(script_ref)
                        {
                            if script_rc.get_own_handler(name).is_some() {
                                return true;
                            }
                        }
                        if crate::player::virtual_scripts::VirtualScriptRegistry::has_script_handler(player, script_ref, name) {
                            return true;
                        }
                    }
                    false
                })
            }
        }
    }

    pub async fn call_async(
        obj_ref: &DatumRef,
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name {
            "new" => Self::new(obj_ref, args).await,
            "rawNew" => Self::raw_new(obj_ref),
            _ => {
                // Try to call a handler defined in the script itself
                let handler_ref = reserve_player_ref(|player| {
                    let script_ref = match player.get_datum(obj_ref) {
                        Datum::ScriptRef(script_ref) => script_ref.clone(),
                        _ => return Err(ScriptError::new("Expected script reference".to_string())),
                    };
                    Ok::<_, ScriptError>((script_ref, handler_name.to_owned()))
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
                    let virtual_result = reserve_player_mut(|player| {
                        let script_ref = match player.get_datum(obj_ref) {
                            Datum::ScriptRef(script_ref) => script_ref.clone(),
                            _ => return Ok(None),
                        };
                        crate::player::virtual_scripts::VirtualScriptRegistry::try_call_handler(player, &script_ref, None, handler_name, args)
                    });
                    match virtual_result {
                        Ok(Some(result)) => return Ok(result),
                        Err(e) => return Err(e),
                        Ok(None) => {}
                    }

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
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name {
            "rawNew" => Self::raw_new(datum),
            "handler" => Self::handler(datum, args),
            "handlers" => Self::handlers(datum, args),
            // A movie script's static properties are addressable through the
            // script reference (Neopets DGS uses `script("globals")` as a global
            // data store: `g.levellist = []`, `g.levellist.add(...)`, etc.).
            // getPropRef returns the property's shared DatumRef so in-place list
            // mutation persists, mirroring the ScriptInstance handler.
            "getProp" | "getPropRef" | "getaProp" => reserve_player_mut(|player| {
                let script_ref = match player.get_datum(datum) {
                    Datum::ScriptRef(s) => s.clone(),
                    _ => return Err(ScriptError::new("Expected script reference".to_string())),
                };
                let prop_name = player.get_datum(&args[0]).string_value()?;
                let prop_ref =
                    crate::player::script::script_get_static_prop(player, &script_ref, &prop_name)?;
                if args.len() >= 2 {
                    // `g.prop[index]` — the bytecode passes (script, #prop, index),
                    // so index into the property value (e.g. list element). Without
                    // this the whole property was returned, ignoring the index.
                    crate::player::handlers::types::TypeUtils::get_sub_prop(
                        &prop_ref, &args[1], player,
                    )
                } else {
                    Ok(prop_ref)
                }
            }),
            "setProp" | "setaProp" => reserve_player_mut(|player| {
                let script_ref = match player.get_datum(datum) {
                    Datum::ScriptRef(s) => s.clone(),
                    _ => return Err(ScriptError::new("Expected script reference".to_string())),
                };
                let prop_name = player.get_datum(&args[0]).string_value()?;
                if args.len() >= 3 {
                    // `g.prop[index] = value`
                    let prop_ref = crate::player::script::script_get_static_prop(
                        player, &script_ref, &prop_name,
                    )?;
                    crate::player::handlers::types::TypeUtils::set_sub_prop(
                        &prop_ref, &args[1], &args[2], player,
                    )?;
                    Ok(args[2].clone())
                } else {
                    crate::player::script::script_set_static_prop(
                        player, &script_ref, &prop_name, &args[1], false,
                    )?;
                    Ok(args[1].clone())
                }
            }),
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for script datum"
            ))),
        }
    }

    pub fn handlers(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let script_ref = match player.get_datum(datum) {
                Datum::ScriptRef(script_ref) => script_ref,
                _ => {
                    return Err(ScriptError::new(
                        "Cannot get handlers of non-script".to_string(),
                    ))
                }
            };
            let script = player
                .movie
                .cast_manager
                .get_script_by_ref(script_ref)
                .unwrap();
            let handler_names = script.handler_names.clone();
            let handler_name_datums: VecDeque<_> = handler_names
                .iter()
                .map(|name| player.alloc_datum(Datum::Symbol(name.clone())))
                .collect();
            Ok(player.alloc_datum(Datum::List(DatumType::List, handler_name_datums, false)))
        })
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

    pub fn create_script_instance(script_ref: &CastMemberRef) -> Result<(ScriptInstanceRef, DatumRef), ScriptError> {
        reserve_player_mut(|player| {
            let instance_id = player.allocator.get_free_script_instance_id();
            let script = player
                .movie
                .cast_manager
                .get_script_by_ref(script_ref)
                .ok_or_else(|| ScriptError::new(format!("Script not found: {:?}", script_ref)))?;

            let lctx_opt = get_lctx_for_script(player, script);

            if let Some(lctx) = lctx_opt {
                let lctx_ptr: *const crate::director::lingo::script::ScriptContext = lctx as *const _;
                let instance = ScriptInstance::new(
                    instance_id,
                    script_ref.to_owned(),
                    script,
                    unsafe { &*lctx_ptr },
                );
                let instance_ref = player.allocator.alloc_script_instance(instance);
                let datum_ref = player.alloc_datum(Datum::ScriptInstanceRef(instance_ref.clone()));
                Ok((instance_ref, datum_ref))
            } else {
                Ok(crate::player::virtual_scripts::VirtualScriptRegistry::create_instance(player, script_ref))
            }
        })
    }

    fn create_uninit_instance(datum: &DatumRef) -> Result<(CastMemberRef, ScriptInstanceRef, DatumRef), ScriptError> {
        let script_ref = reserve_player_mut(|player| {
            let script_ref = match player.get_datum(datum) {
                Datum::ScriptRef(script_ref) => script_ref,
                _ => {
                    return Err(ScriptError::new(
                        "Cannot create new instance of non-script".to_string(),
                    ))
                }
            };

            Ok(script_ref.clone())
        })?;

        let (script_instance_ref, datum_ref) = match Self::create_script_instance(&script_ref) {
            Ok((instance_ref, datum_ref)) => (instance_ref, datum_ref),
            Err(e) => {
                error!("Failed to create script instance: {}", e.message);
                return Err(e); // Return the error
            }
        };

        Ok((script_ref, script_instance_ref, datum_ref))
    }

    pub fn raw_new(datum: &DatumRef) -> Result<DatumRef, ScriptError> {
        Ok(Self::create_uninit_instance(datum)?.2)
    }

    pub async fn new(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        let (script_ref, script_instance_ref, datum_ref) = Self::create_uninit_instance(datum)?;

        let (new_handler_ref, expected_param_count, script_name) =
            reserve_player_mut(|player| {
                let script = player
                    .movie
                    .cast_manager
                    .get_script_by_ref(&script_ref)
                    .unwrap();
                let new_handler_ref = script.get_own_handler_ref(&"new".to_string());

                let param_count = if let Some(_) = &new_handler_ref {
                    let handler_def = script.get_own_handler(&"new".to_string()).unwrap();
                    handler_def.argument_name_ids.len()
                } else {
                    0
                };

                Ok((
                    new_handler_ref,
                    param_count,
                    script.name.clone(),
                ))
            })?;

        let virtual_new_result = reserve_player_mut(|player| {
            crate::player::virtual_scripts::VirtualScriptRegistry::try_call_handler(player, &script_ref, Some(&script_instance_ref), "new", args)
        });
        match virtual_new_result {
            Ok(Some(_)) => return Ok(datum_ref),
            Err(e) => return Err(e),
            Ok(None) => {}
        }

        if let Some(new_handler_ref) = new_handler_ref {
            let mut padded_args = args.clone();
            while padded_args.len() < expected_param_count {
                padded_args.push(DatumRef::Void);
            }

            let result_scope =
                match player_call_script_handler(Some(script_instance_ref), new_handler_ref, &padded_args)
                    .await
                {
                    Ok(scope) => scope,
                    Err(err) => {
                        error!("❌ Error in {}.new(): {}", script_name, err.message);
                        return Err(err);
                    }
                };

            player_handle_scope_return(&result_scope);
            // Director's `new()` returns the new child instance. The `on new`
            // handler conventionally ends with `return me`, but if it falls off
            // the end without returning a value (VOID), Director still returns the
            // instance — NOT VOID. Only an explicit non-void return overrides.
            // (SpongeBob "JellyFishin'" nav object: `on new me, targetMovie`
            // has no `return me`, so navMovieObj was VOID and gotoExitPage /
            // gotoMainMovieAgain dispatched on Void.)
            if matches!(result_scope.return_value, DatumRef::Void) {
                return Ok(datum_ref);
            }
            return Ok(result_scope.return_value);
        } else {
            return Ok(datum_ref);
        }
    }
}
