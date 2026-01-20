use crate::{
    console_warn,
    director::lingo::datum::Datum,
    player::{
        reserve_player_mut, reserve_player_ref, timeout::Timeout, DatumRef, DirPlayer, ScriptError,
    },
};

pub struct TimeoutDatumHandlers {}

impl TimeoutDatumHandlers {
    #[allow(dead_code, unused_variables)]
    pub fn call(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "forget" => Self::forget(datum, args),
            "setAt" => Self::set_at(datum, args),
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for timeout"
            ))),
        }
    }

    fn set_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        // TimeoutInstance needs to support setAt for #ancestor to work with Object Manager
        // We silently ignore ancestor setting since timeouts don't use ancestor chains
        reserve_player_ref(|player| {
            let key = player.get_datum(&args[0]).string_value()?;
            match key.as_str() {
                "ancestor" => {
                    // Silently accept but ignore - timeouts don't use ancestor chains
                    Ok(DatumRef::Void)
                }
                _ => Err(ScriptError::new(format!(
                    "Cannot setAt property {} on timeout", key
                ))),
            }
        })
    }

    pub fn has_async_handler(name: &String) -> bool {
        match name.as_str() {
            "new" => true,
            _ => false,
        }
    }

    pub async fn call_async(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "new" => Self::new(datum, args).await,
            "forget" => Self::forget_async(datum).await,
            _ => Err(ScriptError::new(format!(
                "No async handler {handler_name} for timeout"
            ))),
        }
    }

    pub async fn new(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        // Get the timeout name - either from the datum (TimeoutRef) or from args[0] (TimeoutFactory)
        let timeout_name = reserve_player_ref(|player| {
            let timeout_datum = player.get_datum(&datum);
            match timeout_datum {
                Datum::TimeoutFactory => {
                    // Factory call: timeout().new("name", ...)
                    if args.is_empty() {
                        return Err(ScriptError::new(
                            "timeout.new() requires at least a name argument".to_string(),
                        ));
                    }
                    player.get_datum(&args[0]).string_value()
                }
                Datum::TimeoutRef(timeout_name) => {
                    // Named call: timeout("name").new(...)
                    Ok(timeout_name.clone())
                }
                _ => Err(ScriptError::new(
                    "Cannot create timeout from non-timeout".to_string(),
                )),
            }
        })?;

        // Adjust args based on call type
        let (period_arg, handler_arg, target_arg) = reserve_player_ref(|player| {
            let timeout_datum = player.get_datum(&datum);
            match timeout_datum {
                Datum::TimeoutFactory => {
                    // Factory: timeout().new(name, period, handler, target)
                    // args[0] = name (already used), args[1] = period, args[2] = handler, args[3] = target
                    if args.len() < 4 {
                        return Err(ScriptError::new(
                            "timeout.new() requires 4 arguments: name, period, handler, target".to_string(),
                        ));
                    }
                    Ok((1, 2, 3))
                }
                Datum::TimeoutRef(_) => {
                    // Named: timeout("name").new(period, handler, target)
                    // args[0] = period, args[1] = handler, args[2] = target
                    if args.len() < 3 {
                        return Err(ScriptError::new(
                            "timeout(name).new() requires 3 arguments: period, handler, target".to_string(),
                        ));
                    }
                    Ok((0, 1, 2))
                }
                _ => Err(ScriptError::new("Invalid timeout datum".to_string())),
            }
        })?;

        // Check if this timeout name corresponds to a script in the cast
        // This is only supported in Director 10+ (dir_version >= 1000)
        // In Director 8/9 (scriptExecutionStyle 9), timeout() always creates a standard timeout
        let dir_version = reserve_player_ref(|player| player.movie.dir_version);

        if dir_version >= 1000 {
            let script_ref = reserve_player_ref(|player| {
                player
                    .movie
                    .cast_manager
                    .find_member_ref_by_name(&timeout_name)
            });

            if let Some(script_ref) = script_ref {
                // Verify it's actually a script member
                let is_script = reserve_player_ref(|player| {
                    player
                        .movie
                        .cast_manager
                        .get_script_by_ref(&script_ref)
                        .is_some()
                });

                if is_script {
                    // This is a script-based timeout (like _TIMER_)
                    // Pass ALL arguments to the script's new() handler
                    use crate::player::handlers::datum_handlers::script::ScriptDatumHandlers;
                    let script_datum = reserve_player_mut(|player| {
                        Ok(player.alloc_datum(Datum::ScriptRef(script_ref)))
                    })?;

                    // IMPORTANT: Pass the original args directly to the script's new() handler
                    // The script's new() expects: new(me, _iTimeOut, _hTargetHandler, _oTargetObject, ...)
                    let script_instance = ScriptDatumHandlers::new(&script_datum, args).await?;

                    // The script's new() handler will:
                    // 1. Set all properties (iStartTime, iTimeOut, etc.)
                    // 2. Call (the actorList).add(me)
                    // 3. Return me

                    // Wrap the script instance in a TimeoutInstance so that timeout operations
                    // like forget() work correctly
                    return reserve_player_mut(|player| {
                        Ok(player.alloc_datum(Datum::TimeoutInstance {
                            name: timeout_name,
                            duration: 0, // Script-based timeouts manage their own duration
                            callback: DatumRef::Void,
                            target: DatumRef::Void,
                            script_instance: Some(script_instance),
                        }))
                    });
                }
            }
        }

        // Not a script-based timeout - create a traditional JavaScript timeout
        // This is for backward compatibility with non-script timeouts
        let timeout_period = reserve_player_ref(|player| {
            player.get_datum(&args[period_arg]).int_value()
        })?;

        let timeout_handler = reserve_player_ref(|player| {
            match player.get_datum(&args[handler_arg]) {
                Datum::String(s) => Ok(s.clone()),
                Datum::Symbol(s) => Ok(s.clone()),
                _ => Err(ScriptError::new(
                    "Timeout handler must be a string or symbol".to_string(),
                )),
            }
        })?;

        let target_ref = args[target_arg].clone();

        reserve_player_mut(|player| {
            let mut timeout = Timeout {
                handler: timeout_handler,
                name: timeout_name.clone(),
                period: timeout_period as u32,
                target_ref: target_ref.clone(),
                is_scheduled: false,
            };
            timeout.schedule();
            player.timeout_manager.add_timeout(timeout);
            
            // Return a TimeoutInstance
            Ok(player.alloc_datum(Datum::TimeoutInstance {
                name: timeout_name,
                duration: timeout_period,
                callback: args[handler_arg].clone(),
                target: target_ref,
                script_instance: None,
            }))
        })
    }

    pub fn has_forget_async_handler(datum: &DatumRef) -> bool {
        reserve_player_ref(|player| {
            let timeout_datum = player.get_datum(datum);
            match timeout_datum {
                Datum::TimeoutInstance { script_instance: Some(_), .. } => true,
                _ => false,
            }
        })
    }

    pub async fn forget_async(datum: &DatumRef) -> Result<DatumRef, ScriptError> {
        // Check if this is a script-based timeout
        let script_instance_ref = reserve_player_ref(|player| {
            let timeout_datum = player.get_datum(datum);
            match timeout_datum {
                Datum::TimeoutInstance { script_instance: Some(ref si), .. } => Some(si.clone()),
                _ => None,
            }
        });

        if let Some(script_instance_ref) = script_instance_ref {
            // Call the script's destroy() handler to remove it from actorList
            use super::script_instance::ScriptInstanceDatumHandlers;
            if ScriptInstanceDatumHandlers::has_async_handler(&script_instance_ref, &"destroy".to_string())? {
                let _ = ScriptInstanceDatumHandlers::call_async(
                    &script_instance_ref,
                    &"destroy".to_string(),
                    &vec![],
                ).await;
            }
        }

        // Also forget from the timeout manager (for non-script timeouts or as cleanup)
        reserve_player_mut(|player| {
            let timeout_name = {
                let timeout_ref = player.get_datum(datum);
                match timeout_ref {
                    Datum::TimeoutRef(timeout_name) => Some(timeout_name.to_owned()),
                    Datum::TimeoutInstance { name, .. } => Some(name.to_owned()),
                    _ => None,
                }
            };
            if let Some(name) = timeout_name {
                player.timeout_manager.forget_timeout(&name);
            }
            Ok(DatumRef::Void)
        })
    }

    fn forget(datum: &DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let timeout_name = {
                let timeout_ref = player.get_datum(datum);
                match timeout_ref {
                    Datum::TimeoutRef(timeout_name) => Ok(timeout_name.to_owned()),
                    Datum::TimeoutInstance { name, .. } => Ok(name.to_owned()),
                    _ => Err(ScriptError::new("Cannot forget non-timeout".to_string())),
                }?
            };
            player.timeout_manager.forget_timeout(&timeout_name);
            Ok(DatumRef::Void)
        })
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop: &String,
    ) -> Result<DatumRef, ScriptError> {
        let timeout_datum = player.get_datum(datum);
        match timeout_datum {
            Datum::TimeoutRef(timeout_name) => {
                let timeout = player.timeout_manager.get_timeout(timeout_name);
                match prop.as_str() {
                    "name" => Ok(player.alloc_datum(Datum::String(timeout_name.to_owned()))),
                    "target" => Ok(timeout.map_or(DatumRef::Void, |x| x.target_ref.clone())),
                    _ => Err(ScriptError::new(format!(
                        "Cannot get timeout property {}",
                        prop
                    ))),
                }
            }
            Datum::TimeoutInstance { name, target, .. } => {
                match prop.as_str() {
                    "name" => Ok(player.alloc_datum(Datum::String(name.to_owned()))),
                    "target" => Ok(target.clone()),
                    _ => Err(ScriptError::new(format!(
                        "Cannot get timeout property {}",
                        prop
                    ))),
                }
            }
            _ => Err(ScriptError::new(
                "Cannot get prop of non-timeout".to_string(),
            )),
        }
    }

    pub fn set_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop: &String,
        value: &DatumRef,
    ) -> Result<(), ScriptError> {
        let timeout_datum = player.get_datum(datum);
        let timeout_name = match timeout_datum {
            Datum::TimeoutRef(timeout_name) => timeout_name.clone(),
            Datum::TimeoutInstance { name, .. } => name.clone(),
            _ => return Err(ScriptError::new(
                "Cannot set prop of non-timeout".to_string(),
            )),
        };
        
        let timeout = player.timeout_manager.get_timeout_mut(&timeout_name);
        match prop.as_str() {
            "target" => {
                let new_target = value;
                if let Some(timeout) = timeout {
                    timeout.target_ref = new_target.clone();
                } else {
                    return Err(ScriptError::new(
                        "Cannot set target of unscheduled timeout".to_string(),
                    ));
                }
                Ok(())
            }
            _ => Err(ScriptError::new(format!(
                "Cannot set timeout property {}",
                prop
            ))),
        }
    }
}
