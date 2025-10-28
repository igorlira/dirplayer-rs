use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{
        context_vars::{player_get_context_var, read_context_var_args},
        handlers::datum_handlers::script::ScriptDatumHandlers,
        reserve_player_mut,
        script::{get_current_script, get_current_variable_multiplier, get_name},
        DatumRef, HandlerExecutionResult, HandlerExecutionResultContext, ScriptError, PLAYER_OPT,
    },
};

use super::handler_manager::BytecodeHandlerContext;

pub struct StackBytecodeHandler {}

impl StackBytecodeHandler {
    pub fn push_int(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let datum_ref =
                player.alloc_datum(Datum::Int(player.get_ctx_current_bytecode(ctx).obj as i32));
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(datum_ref);
        });
        Ok(HandlerExecutionResult::Advance)
    }

    pub fn push_f32(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let obj_value = player.get_ctx_current_bytecode(ctx).obj as u32;
            let result = f32::from_bits(obj_value);
            let datum_ref = player.alloc_datum(Datum::Float(result));
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(datum_ref);
        });
        Ok(HandlerExecutionResult::Advance)
    }

    pub fn push_arglist(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let bytecode_obj = player.get_ctx_current_bytecode(ctx).obj;
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            if scope.stack.len() < bytecode_obj as usize {
                return Err(ScriptError::new(
                    "Not enough items in stack to create arglist".to_string(),
                ));
            }
            let items = scope.pop_n(bytecode_obj as usize);
            let datum_ref = player.alloc_datum(Datum::List(DatumType::ArgList, items, false));

            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(datum_ref);
            Ok(())
        })?;
        Ok(HandlerExecutionResult::Advance)
    }

    pub fn push_arglist_no_ret(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let bytecode_obj = player.get_ctx_current_bytecode(ctx).obj;
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            if scope.stack.len() < bytecode_obj as usize {
                return Err(ScriptError::new(
                    "Not enough items in stack to create arglist".to_string(),
                ));
            }
            let items = scope.pop_n(bytecode_obj as usize);
            let datum_ref = player.alloc_datum(Datum::List(DatumType::ArgListNoRet, items, false));

            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(datum_ref);
            Ok(())
        })?;
        Ok(HandlerExecutionResult::Advance)
    }

    pub fn push_symb(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        let player = unsafe { PLAYER_OPT.as_mut().unwrap() };
        let name_id = player.get_ctx_current_bytecode(ctx).obj;
        let symbol_name = get_name(&player, &ctx, name_id as u16).unwrap();
        let datum_ref = player.alloc_datum(Datum::Symbol(symbol_name.to_owned()));

        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        scope.stack.push(datum_ref);
        Ok(HandlerExecutionResult::Advance)
    }

    pub fn push_cons(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            // let (member_ref, handler_def) = get_current_handler_def(&player, ctx.to_owned()).unwrap();
            let script = get_current_script(&player, &ctx).unwrap();

            let literal_id = player.get_ctx_current_bytecode(ctx).obj as u32
                / get_current_variable_multiplier(player, &ctx);
            let literal = &script.chunk.literals[literal_id as usize];
            let datum_ref = player.alloc_datum(literal.clone());

            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(datum_ref);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn push_zero(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let datum_ref = player.alloc_datum(Datum::Int(0));
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(datum_ref);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn push_prop_list(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        let arg_list_ref = reserve_player_mut(|player| {
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.pop().unwrap()
        });
        reserve_player_mut(|player| {
            let arg_list = player.get_datum(&arg_list_ref).to_list()?;
            if arg_list.len() % 2 != 0 {
                return Err(ScriptError::new("argList length must be even".to_string()));
            }
            let entry_count = arg_list.len() / 2;
            let entries = (0..entry_count)
                .map(|index| {
                    let base_index = index * 2;
                    let key = arg_list[base_index].to_owned();
                    let value = arg_list[base_index + 1].to_owned();
                    (key, value)
                })
                .collect::<Vec<(DatumRef, DatumRef)>>();
            let datum_ref = player.alloc_datum(Datum::PropList(entries, false));
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(datum_ref);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn push_list(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let list_id = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.pop().unwrap()
            };
            let list = player.get_datum(&list_id).to_list()?.clone();
            let result_id = player.alloc_datum(Datum::List(DatumType::List, list, false));
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn peek(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let offset = player.get_ctx_current_bytecode(ctx).obj;
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            let stack_index = scope.stack.len() - 1 - offset as usize;
            let datum_ref = scope.stack.get(stack_index).unwrap().clone();
            scope.stack.push(datum_ref);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn pop(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let count = player.get_ctx_current_bytecode(ctx).obj;
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.pop_n(count as usize);
        });
        Ok(HandlerExecutionResult::Advance)
    }

    pub fn push_chunk_var_ref(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let bytecode_obj = player.get_ctx_current_bytecode(ctx).obj;
            let (id_ref, cast_id_ref) =
                read_context_var_args(player, bytecode_obj as u32, ctx.scope_ref);
            let value_ref = player_get_context_var(
                player,
                &id_ref,
                cast_id_ref.as_ref(),
                bytecode_obj as u32,
                ctx,
            )?;

            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(value_ref);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub async fn new_obj(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        let (script_ref, extra_args) = reserve_player_mut(|player| {
            let bytecode = player.get_ctx_current_bytecode(&ctx);
            let obj_type = get_name(player, &ctx, bytecode.obj as u16).unwrap();
            if obj_type != "script" {
                return Err(ScriptError::new(format!(
                    "Cannot create new instance of non-script: {}",
                    obj_type
                )));
            }
            let arg_list = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.pop().unwrap()
            };
            let arg_list = player.get_datum(&arg_list).to_list()?;
            let script_name = player.get_datum(&arg_list[0]).string_value()?;
            let extra_args = arg_list[1..].to_vec();

            let script_ref = player
                .movie
                .cast_manager
                .find_member_ref_by_name(&script_name)
                .unwrap();
            let script_ref = player.alloc_datum(Datum::ScriptRef(script_ref));

            Ok((script_ref, extra_args))
        })?;
        let result = ScriptDatumHandlers::new(&script_ref, &extra_args).await?;
        reserve_player_mut(|player| {
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn swap(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            let a = scope.stack.pop().unwrap();
            let b = scope.stack.pop().unwrap();
            scope.stack.push(a);
            scope.stack.push(b);
            Ok(HandlerExecutionResult::Advance)
        })
    }
}
