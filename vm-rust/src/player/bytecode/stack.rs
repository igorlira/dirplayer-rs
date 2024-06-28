use crate::{director::{chunks::handler::Bytecode, lingo::datum::{Datum, DatumType}}, player::{context_vars::{player_get_context_var, read_context_var_args}, get_datum, handlers::datum_handlers::script::ScriptDatumHandlers, reserve_player_mut, script::{get_current_script, get_current_variable_multiplier, get_name}, DatumRef, HandlerExecutionResult, HandlerExecutionResultContext, ScriptError, PLAYER_LOCK}};

use super::handler_manager::BytecodeHandlerContext;

pub struct StackBytecodeHandler { }

impl StackBytecodeHandler {
  pub fn push_int(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let datum_ref = player.alloc_datum(Datum::Int(bytecode.obj as i32));
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(datum_ref);
    });
    Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
  }

  pub fn push_f32(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    let bytes = (bytecode.obj as i32).to_be_bytes();
    let result = f32::from_be_bytes(bytes);
    
    reserve_player_mut(|player| {
      let datum_ref = player.alloc_datum(Datum::Float(result));
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(datum_ref);
    });
    Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
  }

  pub fn push_arglist(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      if scope.stack.len() < bytecode.obj as usize {
        return Err(ScriptError::new("Not enough items in stack to create arglist".to_string()));
      }
      let items = scope.pop_n(bytecode.obj as usize);
      let datum_ref = player.alloc_datum(Datum::List(DatumType::ArgList, items, false));

      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(datum_ref);
      Ok(())
    })?;
    Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
  }

  pub fn push_arglist_no_ret(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      if scope.stack.len() < bytecode.obj as usize {
        return Err(ScriptError::new("Not enough items in stack to create arglist".to_string()));
      }
      let items = scope.pop_n(bytecode.obj as usize);
      let datum_ref = player.alloc_datum(Datum::List(DatumType::ArgListNoRet, items, false));

      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(datum_ref);
      Ok(())
    })?;
    Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
  }

  pub fn push_symb(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    let name_id = bytecode.obj;
    let mut player_opt = PLAYER_LOCK.try_lock().unwrap();
    let player = player_opt.as_mut().unwrap();
    let symbol_name = get_name(&player, &ctx, name_id as u16).unwrap();
    let datum_ref = player.alloc_datum(Datum::Symbol(symbol_name.to_owned()));

    let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
    scope.stack.push(datum_ref);
    Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
  }

  pub fn push_cons(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      // let (member_ref, handler_def) = get_current_handler_def(&player, ctx.to_owned()).unwrap();
      let script = get_current_script(&player, &ctx).unwrap();

      let literal_id = bytecode.obj as u32 / get_current_variable_multiplier(player, &ctx);
      let literal = &script.chunk.literals[literal_id as usize];
      let datum_ref = player.alloc_datum(literal.clone());

      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(datum_ref);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn push_zero(_: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let datum_ref = player.alloc_datum(Datum::Int(0));
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(datum_ref);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn push_prop_list(_: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    let arg_list_ref = reserve_player_mut(|player| {
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.pop().unwrap()
    });
    reserve_player_mut(|player| {
      let arg_list = get_datum(arg_list_ref, &player.datums).to_list()?;
      if arg_list.len() % 2 != 0 {
        return Err(ScriptError::new("argList length must be even".to_string()));
      }
      let entry_count = arg_list.len() / 2;
      let entries = (0..entry_count).map(|index| {
        let base_index = index * 2;
        let key = arg_list[base_index].to_owned();
        let value = arg_list[base_index + 1].to_owned();
        (key, value)
      }).collect::<Vec<(DatumRef, DatumRef)>>();
      let datum_ref = player.alloc_datum(Datum::PropList(entries, false));
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(datum_ref);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn push_list(_: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let list_id = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        scope.stack.pop().unwrap()
      };
      let list = player.get_datum(list_id).to_list()?.clone();
      let result_id = player.alloc_datum(Datum::List(DatumType::List, list, false));
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn peek(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    let offset = bytecode.obj;
    reserve_player_mut(|player| {
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      let stack_index = scope.stack.len() - 1 - offset as usize;
      let datum_ref = *scope.stack.get(stack_index).unwrap();  
      scope.stack.push(datum_ref);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn pop(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    let count = bytecode.obj;
    reserve_player_mut(|player| {
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.pop_n(count as usize);
    });
    Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
  }

  pub fn push_chunk_var_ref(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let (id_ref, cast_id_ref) = read_context_var_args(player, bytecode.obj as u32, ctx.scope_ref);
      let value_ref = player_get_context_var(player, id_ref, cast_id_ref, bytecode.obj as u32, ctx)?;
    
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(value_ref);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub async fn new_obj(bytecode: Bytecode, ctx: BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    let (script_ref, extra_args) = reserve_player_mut(|player| {
      let obj_type = get_name(player, &ctx, bytecode.obj as u16).unwrap();
      if obj_type != "script" {
        return Err(ScriptError::new(format!("Cannot create new instance of non-script: {}", obj_type)));
      }
      let arg_list = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        scope.stack.pop().unwrap()
      };
      let arg_list = player.get_datum(arg_list).to_list()?;
      let script_name = player.get_datum(arg_list[0]).string_value(&player.datums)?;
      let extra_args = arg_list[1..].to_vec();

      let script_ref = player.movie.cast_manager.find_member_ref_by_name(&script_name).unwrap();
      let script_ref = player.alloc_datum(Datum::ScriptRef(script_ref));
      
      Ok((script_ref, extra_args))
    })?;
    let result = ScriptDatumHandlers::new(script_ref, &extra_args).await?;
    reserve_player_mut(|player| {
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }
}
