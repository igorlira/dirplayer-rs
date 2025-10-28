use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{
        compare::datum_is_zero,
        handlers::datum_handlers::{
            player_call_datum_handler, script_instance::ScriptInstanceUtils,
        },
        player_call_script_handler_raw_args, player_ext_call, player_handle_scope_return,
        reserve_player_mut, reserve_player_ref,
        script::{get_current_handler_def, get_current_script, get_name},
        HandlerExecutionResult, ScriptError, PLAYER_OPT,
    },
};

use super::handler_manager::BytecodeHandlerContext;

pub struct FlowControlBytecodeHandler {}

impl FlowControlBytecodeHandler {
    pub fn ret(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.clear();
        });
        Ok(HandlerExecutionResult::Stop)
    }

    pub async fn ext_call(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        // let script = get_current_script(player.to_owned(), ctx.to_owned());
        let (name, arg_ref_list, is_no_ret) = {
            let player = unsafe { PLAYER_OPT.as_mut().unwrap() };
            let player_cell = &player;

            let name_id = player.get_ctx_current_bytecode(&ctx).obj as u16;

            let name = get_name(player_cell, &ctx, name_id).unwrap().to_owned();
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            let arg_list_datum_ref = scope.stack.pop().unwrap();
            let arg_list_datum = player.get_datum(&arg_list_datum_ref);

            if let Datum::List(list_type, list, _) = arg_list_datum {
                let is_no_ret = match list_type {
                    DatumType::ArgListNoRet => true,
                    _ => false,
                };
                (name, list.to_owned(), is_no_ret)
            } else {
                panic!("ext_call was not passed a list");
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

    pub async fn local_call(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        let (handler_ref, is_no_ret, args, receiver) = reserve_player_mut(|player| {
            let arg_list_id = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.pop().unwrap()
            };
            let script = get_current_script(&player, &ctx).unwrap();

            let arg_list_datum = player.get_datum(&arg_list_id);
            let is_no_ret = match arg_list_datum {
                Datum::List(DatumType::ArgListNoRet, _, _) => true,
                _ => false,
            };
            let args = arg_list_datum.to_list()?.clone();

            let mut handler_ref = script
                .get_own_handler_ref_at(player.get_ctx_current_bytecode(&ctx).obj as usize)
                .unwrap();
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
                scope.loop_return_indices.push(scope.bytecode_index);
                scope.stack.pop().unwrap()
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
            let arg_list_id = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.pop().unwrap()
            };
            let arg_list_datum = player.get_datum(&arg_list_id);
            let is_no_ret = match arg_list_datum {
                Datum::List(DatumType::ArgListNoRet, _, _) => true,
                _ => false,
            };
            let arg_list = arg_list_datum.to_list()?;
            let obj = arg_list[0].clone();
            let args = arg_list[1..].to_vec();
            let handler_name = get_name(
                &player,
                &ctx,
                player.get_ctx_current_bytecode(&ctx).obj as u16,
            )
            .unwrap()
            .to_owned();

            Ok((obj, handler_name, args, is_no_ret))
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
}
