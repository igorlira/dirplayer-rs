use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{
        HandlerExecutionResult, PLAYER_OPT, ScriptError, compare::datum_is_zero, datum_formatting::format_datum, datum_ref::DatumRef, handlers::datum_handlers::{
            player_call_datum_handler, script_instance::ScriptInstanceUtils,
        }, player_call_script_handler_raw_args, player_ext_call, player_handle_scope_return, reserve_player_mut, reserve_player_ref, script::{get_current_handler_def, get_current_script, get_name}
    },
};

use super::handler_manager::BytecodeHandlerContext;

pub struct FlowControlBytecodeHandler {}

impl FlowControlBytecodeHandler {
    pub fn ret(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.return_value = DatumRef::Void;
            scope.stack.clear();
        });
        Ok(HandlerExecutionResult::Stop)
    }

    pub async fn ext_call(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        // let script = get_current_script(player.to_owned(), ctx.to_owned());
        let (name, arg_ref_list, is_no_ret) = {
            let player = unsafe { crate::player::player_mut() };
            let player_cell = &player;

            let name_id = player.get_ctx_current_bytecode(&ctx).obj as u16;

            let name = get_name(player_cell, &ctx, name_id).unwrap().to_owned();
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            let arg_list_datum_ref = match scope.stack.pop() {
                Some(datum_ref) => datum_ref,
                None => {
                    return Err(ScriptError::new(format!(
                        "ext_call '{}': operand stack is empty (scope_ref={}, bytecode_index={})",
                        name, ctx.scope_ref, scope.bytecode_index
                    )));
                }
            };
            let arg_list_datum = player.get_datum(&arg_list_datum_ref);

            if let Datum::List(list_type, list, _) = arg_list_datum {
                let is_no_ret = match list_type {
                    DatumType::ArgListNoRet => true,
                    _ => false,
                };
                (name, Vec::from(list.to_owned()), is_no_ret)
            } else {
                return Err(ScriptError::new(format!(
                    "ext_call '{}': expected arg list on stack",
                    name
                )));
            }
        };

        let (result_ctx, return_value) =
            player_ext_call(name.clone(), &arg_ref_list, ctx.scope_ref).await;
        if !is_no_ret {
            reserve_player_mut(|player| {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.push(return_value);
            });
        }
        return Ok(result_ctx);
    }

    /// `tell <target>` — pop the target and record which context the enclosed
    /// `tellcall`s dispatch to. `tell sprite(#movieSprite)` targets that nested
    /// sub-player (the loader→game command bridge); other targets run on THIS
    /// player. Stack is a Vec so `tell` blocks can nest.
    pub fn start_tell(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            let target_ref = scope.stack.pop().ok_or_else(|| {
                ScriptError::new("starttell: operand stack is empty".to_string())
            })?;
            let target = player.get_datum(&target_ref).clone();
            let nested = match target {
                Datum::SpriteRef(n) => player
                    .movie
                    .score
                    .get_sprite(n as i16)
                    .and_then(|s| s.member.clone())
                    .and_then(|m| crate::player::nested_player_id(&m)),
                Datum::CastMember(ref m) => crate::player::nested_player_id(m),
                _ => None,
            };
            player.tell_target_stack.push(nested);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    /// `end tell` — pop the current tell target.
    pub fn end_tell(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        let _ = ctx;
        reserve_player_mut(|player| {
            player.tell_target_stack.pop();
        });
        Ok(HandlerExecutionResult::Advance)
    }

    /// `tellcall` — like `ext_call` but dispatches the command into the current
    /// `tell` target. For a nested `#movie` target the args are marshaled into
    /// the sub-player, the command runs there (with the active id pinned across
    /// awaits), and the result is marshaled back. No nested target → runs here.
    pub async fn tell_call(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        let (name, arg_ref_list, is_no_ret, target) = {
            let player = unsafe { crate::player::player_mut() };
            let name_id = player.get_ctx_current_bytecode(&ctx).obj as u16;
            let name = get_name(&player, &ctx, name_id).unwrap().to_owned();
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            let arg_list_ref = scope.stack.pop().ok_or_else(|| {
                ScriptError::new(format!("tell_call '{}': operand stack is empty", name))
            })?;
            let arg_list_datum = player.get_datum(&arg_list_ref);
            let (args, is_no_ret) = if let Datum::List(list_type, list, _) = arg_list_datum {
                (
                    Vec::from(list.to_owned()),
                    matches!(list_type, DatumType::ArgListNoRet),
                )
            } else {
                return Err(ScriptError::new(format!(
                    "tell_call '{}': expected arg list on stack",
                    name
                )));
            };
            let target = player.tell_target_stack.last().copied().flatten();
            (name, args, is_no_ret, target)
        };

        let nested_id = match target {
            Some(id) => id,
            None => {
                // No nested target — behave like ext_call on this player.
                let (result_ctx, return_value) =
                    player_ext_call(name.clone(), &arg_ref_list, ctx.scope_ref).await;
                if !is_no_ret {
                    reserve_player_mut(|player| {
                        player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push(return_value);
                    });
                }
                return Ok(result_ctx);
            }
        };

        // Marshal args from the host into the nested sub-player's allocator, and
        // PAUSE the sub's own frame loop for the duration of the tell dispatch.
        // Otherwise the sub's frame loop (a separate async task) runs its scripts
        // concurrently with the tell's `sendAllSprites` — both push/pop scopes on
        // the same player between awaits, corrupting its scope stack (observed as
        // a hang). `command_handler_yielding` is the engine's existing "a command
        // handler is running, don't advance the frame loop" gate.
        let nested_args: Vec<DatumRef> = unsafe {
            let host = crate::player::player_mut();
            match crate::player::NESTED_PLAYERS
                .get_mut(nested_id - 1)
                .and_then(|o| o.as_mut())
            {
                Some(sub) => {
                    sub.command_handler_yielding = true;
                    arg_ref_list
                        .iter()
                        .map(|r| crate::player::marshal_datum(host, sub, r))
                        .collect()
                }
                None => return Ok(HandlerExecutionResult::Advance),
            }
        };

        // Run the command inside the nested player, active id pinned across awaits.
        let (_result_ctx, nested_return) = crate::player::with_active_player(
            nested_id,
            player_ext_call(name.clone(), &nested_args, ctx.scope_ref),
        )
        .await;

        // Resume the sub's frame loop.
        unsafe {
            if let Some(sub) = crate::player::NESTED_PLAYERS
                .get_mut(nested_id - 1)
                .and_then(|o| o.as_mut())
            {
                sub.command_handler_yielding = false;
            }
        }

        if !is_no_ret {
            let host_ret = unsafe {
                let host = crate::player::player_mut();
                match crate::player::NESTED_PLAYERS
                    .get(nested_id - 1)
                    .and_then(|o| o.as_ref())
                {
                    Some(sub) => crate::player::marshal_datum(sub, host, &nested_return),
                    None => DatumRef::Void,
                }
            };
            reserve_player_mut(|player| {
                player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push(host_ret);
            });
        }
        Ok(HandlerExecutionResult::Advance)
    }

    pub async fn local_call(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        let (handler_ref, is_no_ret, args, receiver) = reserve_player_mut(|player| {
            let arg_list_id = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                match scope.stack.pop() {
                    Some(v) => v,
                    None => {
                        let current_handler_name = ctx.get_name(scope.handler_name_id);
                        return Err(ScriptError::new(format!(
                            "local_call: stack underflow in handler '{}' (script={}:{}, scope_ref={}, bytecode_index={})",
                            current_handler_name, scope.script_ref.cast_lib, scope.script_ref.cast_member, ctx.scope_ref, scope.bytecode_index
                        )));
                    }
                }
            };
            let script = get_current_script(&player, &ctx).unwrap();

            let arg_list_datum = player.get_datum(&arg_list_id);
            let is_no_ret = match arg_list_datum {
                Datum::List(DatumType::ArgListNoRet, _, _) => true,
                _ => false,
            };
            let args: Vec<DatumRef> = arg_list_datum.to_list()?.iter().cloned().collect();

            let handler_index = player.get_ctx_current_bytecode(&ctx).obj as usize;
            let mut handler_ref = match script.get_own_handler_ref_at(handler_index) {
                Some(h) => h,
                None => {
                    return Err(ScriptError::new(format!(
                        "local_call: no own handler at index {} (script={}:{}, has {} handlers)",
                        handler_index,
                        script.member_ref.cast_lib,
                        script.member_ref.cast_member,
                        script.handler_names.len()
                    )));
                }
            };
            let handler_name = &handler_ref.1;

            // if first arg is a script or script instance and has a handler by the same name
            // use that handler instead
            let mut receiver;
            let receiver_handler =
                ScriptInstanceUtils::get_handler_from_first_arg(&args, handler_name);
            if receiver_handler.is_some() {
                let handler_pair = receiver_handler.unwrap();
                receiver = handler_pair.0;
                handler_ref = handler_pair.1;
            } else {
                receiver = reserve_player_ref(|player| {
                    let scope = player.scopes.get(ctx.scope_ref).unwrap();
                    scope.receiver.clone()
                });
            }
            Ok((handler_ref, is_no_ret, args, receiver))
        })?;
        let scope = player_call_script_handler_raw_args(receiver, handler_ref, &args, true).await?;
        player_handle_scope_return(&scope);
        let result = scope.return_value;
        if !is_no_ret {
            reserve_player_mut(|player| {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.push(result);
            });
        }
        Ok(HandlerExecutionResult::Advance)
    }

    pub fn jmp_if_zero(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let value_id = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                match scope.stack.pop() {
                    Some(v) => v,
                    None => {
                        let current_handler_name = ctx.get_name(scope.handler_name_id);
                        return Err(ScriptError::new(format!(
                            "jmp_if_zero: stack underflow in handler '{}' (script={}:{}, scope_ref={}, bytecode_index={})",
                            current_handler_name, scope.script_ref.cast_lib, scope.script_ref.cast_member, ctx.scope_ref, scope.bytecode_index
                        )));
                    }
                }
            };

            let datum = player.get_datum(&value_id);
            let bytecode = player.get_ctx_current_bytecode(&ctx);
            let position = bytecode.pos as i32;
            let offset = bytecode.obj as i32;

            if datum_is_zero(datum, &player.allocator)? {
                let new_bytecode_index = {
                    let handler = get_current_handler_def(player, &ctx);
                    let dest_pos = (position as i32 + offset) as usize;
                    handler.bytecode_index_map[&dest_pos] as usize
                };
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.bytecode_index = new_bytecode_index;
                Ok(HandlerExecutionResult::Jump)
            } else {
                Ok(HandlerExecutionResult::Advance)
            }
        })
    }

    pub fn jmp(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let bytecode = player.get_ctx_current_bytecode(ctx);
            let new_bytecode_index = {
                let handler = get_current_handler_def(player, &ctx);
                let dest_pos = (bytecode.pos as i32 + bytecode.obj as i32) as usize;
                handler.bytecode_index_map[&dest_pos] as usize
            };
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.bytecode_index = new_bytecode_index;
            Ok(HandlerExecutionResult::Jump)
        })
    }

    pub async fn obj_call(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        // let token = start_profiling("_obj_call_prepare".to_string());
        let (obj_ref, handler_name, args, is_no_ret) = reserve_player_mut(|player| {
            let bytecode = player.get_ctx_current_bytecode(&ctx);
            let target_handler_name = get_name(
                &player,
                &ctx,
                bytecode.obj as u16,
            )
            .map(|s| s.to_owned())
            .unwrap_or_else(|| "?".to_owned());
            let arg_list_id = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                match scope.stack.pop() {
                    Some(v) => v,
                    None => {
                        let current_handler_name = ctx.get_name(scope.handler_name_id);
                        return Err(ScriptError::new(format!(
                            "obj_call '{}': stack underflow in handler '{}' (script={}:{}, scope_ref={}, bytecode_index={})",
                            target_handler_name, current_handler_name, scope.script_ref.cast_lib, scope.script_ref.cast_member, ctx.scope_ref, scope.bytecode_index
                        )));
                    }
                }
            };
            let arg_list_datum = player.get_datum(&arg_list_id);
            let is_no_ret = match arg_list_datum {
                Datum::List(DatumType::ArgListNoRet, _, _) => true,
                _ => false,
            };
            let arg_list = arg_list_datum.to_list()?;
            let obj = arg_list[0].clone();
            let args: Vec<DatumRef> = arg_list.iter().skip(1).cloned().collect();

            Ok((obj, target_handler_name, args, is_no_ret))
        })?;
        // end_profiling(token);
        // let token = start_profiling(handler_name.clone());
        let result = player_call_datum_handler(&obj_ref, &handler_name, &args).await?;
        // end_profiling(token);
        // let token = start_profiling("_obj_call_push_result".to_string());
        reserve_player_mut(|player| {
            player.last_handler_result = result.clone();
            if !is_no_ret {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.push(result);
            };
        });
        // end_profiling(token);
        Ok(HandlerExecutionResult::Advance)
    }

    pub async fn obj_call_v4(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        // ObjCallV4 is like ObjCall but the handler name comes from the stack
        // (pushed by PushVarRef) instead of the bytecode operand.
        // In Director 4 syntax, `handlerName(objectSymbol)` calls a handler on the
        // object referenced by the symbol. The first arg is typically a symbol that
        // needs to be resolved to a global variable to find the actual object.
        // Stack: [..., ArgList([receiver, args...]), Symbol(handlerName)]
        let (obj_ref, handler_name, args, is_no_ret, route_to_global) = reserve_player_mut(|player| {
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            let handler_name_ref = scope.stack.pop().ok_or_else(|| ScriptError::new("obj_call_v4: stack underflow (handler name)".to_string()))?;
            let arg_list_ref = scope.stack.pop().ok_or_else(|| ScriptError::new("obj_call_v4: stack underflow (arg list)".to_string()))?;

            let handler_name = player.get_datum(&handler_name_ref).symbol_value()?;

            let arg_list_datum = player.get_datum(&arg_list_ref);
            let is_no_ret = match arg_list_datum {
                Datum::List(DatumType::ArgListNoRet, _, _) => true,
                _ => false,
            };
            let arg_list = arg_list_datum.to_list()?;
            let mut obj = arg_list[0].clone();
            let args: Vec<DatumRef> = arg_list.iter().skip(1).cloned().collect();

            // In Director 4 calling convention, the receiver is often passed as a
            // symbol (e.g. #oTrackControl). Resolve it by looking up the symbol
            // name in globals to get the actual script instance.
            if let Datum::Symbol(sym_name) = player.get_datum(&obj) {
                if let Some(global_ref) = player.globals.get(sym_name) {
                    obj = global_ref.clone();
                }
            }

            // Decide whether this is a real method call (receiver is a script
            // object) or the D4 `name(receiver, ..)` form of a MOVIE HANDLER
            // call where the first arg just happens to be the receiver. Director
            // gives a movie handler priority when the receiver isn't an object
            // with that method. hackey's `vector(HERE, there)` compiles to
            // ObjCallV4 with HERE (a list) as receiver, but `vector` is a movie
            // handler (`on vector HERE, there`) — calling it as a list method
            // failed with "No handler vector for list datum".
            let is_object_receiver = matches!(
                player.get_datum(&obj),
                Datum::ScriptInstanceRef(_) | Datum::ScriptRef(_)
            );
            let route_to_global = !is_object_receiver
                && crate::player::player_global_handler_exists(player, &handler_name);

            Ok((obj, handler_name, args, is_no_ret, route_to_global))
        })?;
        let result = if route_to_global {
            // Movie-handler call: pass the receiver as the first argument so
            // `on vector HERE, there` receives (HERE, there).
            let mut full_args = Vec::with_capacity(args.len() + 1);
            full_args.push(obj_ref.clone());
            full_args.extend(args.iter().cloned());
            crate::player::player_call_global_handler(&handler_name, &full_args).await?
        } else {
            player_call_datum_handler(&obj_ref, &handler_name, &args).await?
        };
        reserve_player_mut(|player| {
            player.last_handler_result = result.clone();
            if !is_no_ret {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.push(result);
            };
        });
        Ok(HandlerExecutionResult::Advance)
    }

    pub fn end_repeat(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let new_index = {
                let bytecode = player.get_ctx_current_bytecode(ctx);
                let handler = get_current_handler_def(player, &ctx);
                let return_pos = bytecode.pos - bytecode.obj as usize;
                handler.bytecode_index_map[&return_pos] as usize
            };
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.bytecode_index = new_index;
            Ok(HandlerExecutionResult::Jump)
        })
    }

    pub fn call_javascript(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            let arg1 = scope.stack.pop().ok_or_else(|| ScriptError::new("call_javascript: stack underflow (arg1)".to_string()))?;
            let arg2 = scope.stack.pop().ok_or_else(|| ScriptError::new("call_javascript: stack underflow (arg2)".to_string()))?;
            let arg1_formatted = format_datum(&arg1, player);
            let arg2_formatted = format_datum(&arg2, player);

            log::warn!("TODO: call_javascript with args: {}, {}", arg1_formatted, arg2_formatted);
            Ok(HandlerExecutionResult::Advance)
        })
    }
}
