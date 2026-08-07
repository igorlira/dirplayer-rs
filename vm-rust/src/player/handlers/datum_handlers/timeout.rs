use crate::{
    director::lingo::datum::Datum,
    player::{
        DatumRef, DirPlayer, ScriptError, reserve_player_mut, reserve_player_ref, symbols::{builtin::BuiltInSymbol, symbol::Symbol}, timeout::Timeout
    },
};

pub struct TimeoutDatumHandlers {}

impl TimeoutDatumHandlers {
    #[allow(dead_code, unused_variables)]
    pub fn call(
        datum: &DatumRef,
        handler_name: Symbol,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.into_builtin() {
            Some(BuiltInSymbol::Forget) => Self::forget(datum, args),
            Some(BuiltInSymbol::SetAt) => Self::set_at(datum, args),
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for timeout"
            ))),
        }
    }

    fn set_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        // TimeoutInstance needs to support setAt for #ancestor to work with Object Manager
        // We silently ignore ancestor setting since timeouts don't use ancestor chains
        reserve_player_ref(|player| {
            let key = player.get_datum(&args[0]).symbol_value()?;
            match key.into_builtin() {
                Some(BuiltInSymbol::Ancestor) => {
                    // Silently accept but ignore - timeouts don't use ancestor chains
                    Ok(DatumRef::Void)
                }
                _ => Err(ScriptError::new(format!(
                    "Cannot setAt property {} on timeout", key
                ))),
            }
        })
    }

    pub fn has_async_handler(name: Symbol) -> bool {
        matches!(name.into_builtin(), Some(BuiltInSymbol::New))
    }

    pub async fn call_async(
        datum: &DatumRef,
        handler_name: Symbol,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.into_builtin() {
            Some(BuiltInSymbol::New) => Self::new(datum, args).await,
            Some(BuiltInSymbol::Forget) => Self::forget_async(datum).await,
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

        // Adjust arg indices based on call type. `target` is optional in
        // Director — when omitted, the timeout fires the handler in the
        // calling script's context (Director searches movie scripts).
        // `Option<usize>` for target_arg signals "no target supplied".
        let (period_arg, handler_arg, target_arg) = reserve_player_ref(|player| {
            let timeout_datum = player.get_datum(&datum);
            match timeout_datum {
                Datum::TimeoutFactory => {
                    // Factory: timeout().new(name, period, handler {, target})
                    // args[0] = name (already used), args[1] = period, args[2] = handler, args[3] = target?
                    if args.len() < 3 {
                        return Err(ScriptError::new(
                            "timeout.new() requires at least: name, period, handler".to_string(),
                        ));
                    }
                    let tgt = if args.len() >= 4 { Some(3) } else { None };
                    Ok((1usize, 2usize, tgt))
                }
                Datum::TimeoutRef(_) => {
                    // Named: timeout("name").new(period, handler {, target})
                    // args[0] = period, args[1] = handler, args[2] = target?
                    if args.len() < 2 {
                        return Err(ScriptError::new(
                            "timeout(name).new() requires at least: period, handler".to_string(),
                        ));
                    }
                    let tgt = if args.len() >= 3 { Some(2) } else { None };
                    Ok((0usize, 1usize, tgt))
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
                        Ok(player.alloc_datum(Datum::timeout_instance(
                            timeout_name,
                            0, // Script-based timeouts manage their own duration
                            DatumRef::Void,
                            DatumRef::Void,
                            Some(script_instance),
                        )))
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
                Datum::String(s) => Ok(Symbol::from_str(s)),
                Datum::Symbol(s) => Ok(*s),
                _ => Err(ScriptError::new(
                    "Timeout handler must be a string or symbol".to_string(),
                )),
            }
        })?;

        // When the script omitted `target`, default to Datum::Void — the
        // timeout fire path in mod.rs treats that as "search movie scripts
        // for `handler_name`", matching Director's documented fallback.
        let target_ref = match target_arg {
            Some(idx) => args[idx].clone(),
            None => DatumRef::Void,
        };

        // A negative period must not wrap through `as u32` into ~49 days of silence.
        // Scripts compute periods arithmetically (AreaZero: `(motion.duration / 0.5) - 100`)
        // and a short or zero duration goes negative; Director treats a non-positive
        // period as dormant, so clamp and say so rather than fail invisibly.
        if timeout_period < 0 {
            crate::console_warn!(
                "timeout(\"{}\").new: negative period {} — clamped to 0 (dormant)",
                timeout_name, timeout_period
            );
        }
        let timeout_period = timeout_period.max(0);

        reserve_player_mut(|player| {
            let mut timeout = Timeout {
                handler: timeout_handler,
                name: timeout_name.clone(),
                period: timeout_period as u32,
                target_ref: target_ref.clone(),
                is_scheduled: false,
                next_fire_ms: 0.0,
            };
            // Retire any same-named timeout BEFORE scheduling the replacement.
            // The JS host keys interval handles by name, so scheduling first stored
            // the new handle under that name and the subsequent cancel then cleared
            // the NEW interval while leaking the old one — every re-creation of a
            // self-rescheduling timeout (AreaZero's MenuCameraNextAnimation chain)
            // killed its own replacement.
            player.timeout_manager.forget_timeout(&timeout.name);
            timeout.schedule();
            player.timeout_manager.add_timeout(timeout);
            
            // Return a TimeoutInstance
            Ok(player.alloc_datum(Datum::timeout_instance(
                timeout_name,
                timeout_period,
                args[handler_arg].clone(),
                target_ref,
                None,
            )))
        })
    }

    pub fn has_forget_async_handler(datum: &DatumRef) -> bool {
        reserve_player_ref(|player| {
            let timeout_datum = player.get_datum(datum);
            match timeout_datum {
                Datum::TimeoutInstance(ti) => ti.script_instance.is_some(),
                _ => false,
            }
        })
    }

    pub async fn forget_async(datum: &DatumRef) -> Result<DatumRef, ScriptError> {
        // Check if this is a script-based timeout
        let script_instance_ref = reserve_player_ref(|player| {
            let timeout_datum = player.get_datum(datum);
            match timeout_datum {
                Datum::TimeoutInstance(ti) => ti.script_instance.clone(),
                _ => None,
            }
        });

        if let Some(script_instance_ref) = script_instance_ref {
            // Call the script's destroy() handler to remove it from actorList
            use super::script_instance::ScriptInstanceDatumHandlers;
            if ScriptInstanceDatumHandlers::has_async_handler(&script_instance_ref, Symbol::builtin(BuiltInSymbol::Destroy))? {
                let _ = ScriptInstanceDatumHandlers::call_async(
                    &script_instance_ref,
                    Symbol::builtin(BuiltInSymbol::Destroy),
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
                    Datum::TimeoutInstance(ti) => Some(ti.name.to_owned()),
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
                    Datum::TimeoutInstance(ti) => Ok(ti.name.to_owned()),
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
        prop: Symbol,
    ) -> Result<DatumRef, ScriptError> {
        let timeout_datum = player.get_datum(datum);
        match timeout_datum {
            Datum::TimeoutRef(timeout_name) => {
                let timeout = player.timeout_manager.get_timeout(timeout_name);
                match prop.as_str() {
                    "name" => Ok(player.alloc_datum(Datum::String(timeout_name.to_owned()))),
                    "target" => Ok(timeout.map_or(DatumRef::Void, |x| x.target_ref.clone())),
                    "period" => {
                        let p = timeout.map_or(0, |t| t.period as i32);
                        Ok(player.alloc_datum(Datum::Int(p)))
                    }
                    _ => Err(ScriptError::new(format!(
                        "Cannot get timeout property {}",
                        prop
                    ))),
                }
            }
            Datum::TimeoutInstance(ti) => {
                match prop.as_str() {
                    "name" => Ok(player.alloc_datum(Datum::String(ti.name.to_owned()))),
                    "target" => Ok(ti.target.clone()),
                    "period" => {
                        let p = player
                            .timeout_manager
                            .get_timeout(&ti.name)
                            .map_or(0, |t| t.period as i32);
                        Ok(player.alloc_datum(Datum::Int(p)))
                    }
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
        prop: Symbol,
        value: &DatumRef,
    ) -> Result<(), ScriptError> {
        let timeout_datum = player.get_datum(datum);
        let timeout_name = match timeout_datum {
            Datum::TimeoutRef(timeout_name) => timeout_name.clone(),
            Datum::TimeoutInstance(ti) => ti.name.clone(),
            _ => return Err(ScriptError::new(
                "Cannot set prop of non-timeout".to_string(),
            )),
        };
        
        let timeout = player.timeout_manager.get_timeout_mut(&timeout_name);
        match prop.into_builtin() {
            Some(BuiltInSymbol::Target) => {
                let new_target = value.clone();
                if let Some(timeout) = timeout {
                    timeout.target_ref = new_target;
                    Ok(())
                } else {
                    Err(ScriptError::new(
                        "Cannot set target of unscheduled timeout".to_string(),
                    ))
                }
            }
            // `the period of timeoutObject` is read/write (Director Scripting
            // Dictionary): it is the number of ms between timeout events, and
            // setting it changes the interval. We update the period and
            // reschedule the underlying timer (a period of 0 makes it dormant).
            // Habbo's DM_CurlDetacher sets `period = 1` to start a dormant
            // timeout — that drives the cURL-download completion callback. Before
            // this arm existed, the assignment errored ("Cannot set timeout
            // property period") and, because it runs inside the cURL Xtra
            // callback (which fails silently), the download never completed and
            // ES Origins never showed its login (figuredata/external_texts
            // never loaded).
            Some(BuiltInSymbol::Period) => {
                let new_period = player.get_datum(value).int_value()?;
                let new_period = if new_period < 0 { 0 } else { new_period as u32 };
                let timeout = player.timeout_manager.get_timeout_mut(&timeout_name);
                if let Some(timeout) = timeout {
                    timeout.period = new_period;
                    timeout.schedule();
                    Ok(())
                } else {
                    Err(ScriptError::new(
                        "Cannot set period of unscheduled timeout".to_string(),
                    ))
                }
            }
            _ => Err(ScriptError::new(format!(
                "Cannot set timeout property {}",
                prop.to_string()
            ))),
        }
    }
}
