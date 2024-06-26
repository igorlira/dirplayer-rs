use crate::{director::{chunks::handler::Bytecode, lingo::{constants::{get_anim_prop_name, MOVIE_PROP_NAMES}, datum::{Datum, StringChunkType}}}, player::{get_datum, handlers::datum_handlers::string_chunk::StringChunkUtils, reserve_player_mut, script::{get_current_handler_def, get_current_variable_multiplier, get_name, get_obj_prop, player_set_obj_prop, script_get_prop, script_set_prop}, DatumRef, DirPlayer, HandlerExecutionResult, HandlerExecutionResultContext, ScriptError, PLAYER_LOCK, VOID_DATUM_REF}};

use super::handler_manager::BytecodeHandlerContext;


pub struct GetSetBytecodeHandler { }
pub struct GetSetUtils { }

impl GetSetUtils {
  pub fn get_the_built_in_prop(
      player: &mut DirPlayer,
      ctx: &BytecodeHandlerContext,
      prop_name: &String,
  ) -> Result<DatumRef, ScriptError> {
      match prop_name.as_str() {
        "paramCount" => Ok(player.alloc_datum(Datum::Int(player.scopes.get(ctx.scope_ref).unwrap().args.len() as i32))),
        "result" => Ok(player.last_handler_result),
        _ => Ok(player.alloc_datum(player.get_movie_prop(prop_name)?))
      }
  }

  pub fn set_the_built_in_prop(
      player: &mut DirPlayer,
      _ctx: &BytecodeHandlerContext,
      prop_name: &String,
      value: Datum,
  ) -> Result<(), ScriptError> {
      match prop_name.as_str() {
        _ => player.set_movie_prop(prop_name, value)
      }
  }
}

impl GetSetBytecodeHandler {
  pub fn get_prop(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    let mut player_opt = PLAYER_LOCK.try_lock().unwrap();
    let player = player_opt.as_mut().unwrap();
    let receiver ={
      let scope = player.scopes.get(ctx.scope_ref).unwrap();
      scope.receiver
    };
    let prop_name = get_name(&player, &ctx, bytecode.obj as u16).unwrap().clone();
    
    if let Some(instance_id) = receiver {
      let result = script_get_prop(player, instance_id, &prop_name)?;
      
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result);
      return Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance });
    } else {
      Err(ScriptError::new(format!("No receiver to get prop {}", prop_name)))
    }
  }

  pub fn set_prop(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let (value_ref, receiver) = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        let value_red = scope.stack.pop().unwrap();
        (value_red, scope.receiver)
      };
      let prop_name = get_name(&player, &ctx, bytecode.obj as u16).unwrap();
      
      match receiver {
        Some(0) => Err(ScriptError::new(format!("Can't set prop {} of Void", prop_name))),
        Some(instance_id) => {
          script_set_prop(player, instance_id, &prop_name.to_owned(), value_ref, false)?;
          Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
        },
        None => Err(ScriptError::new(format!("No receiver to set prop {}", prop_name)))
      }
    })
  }

  pub async fn set_obj_prop(bytecode: Bytecode, ctx: BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    let (value, obj_datum_ref, prop_name) = reserve_player_mut(|player| {
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      let value = scope.stack.pop().unwrap();
      let obj_datum_ref = scope.stack.pop().unwrap();
      let prop_name = get_name(&player, &ctx, bytecode.obj as u16).unwrap().to_owned();
      Ok((value, obj_datum_ref, prop_name))
    })?;
    player_set_obj_prop(obj_datum_ref, &prop_name, value).await?;
    Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
  }

  pub fn get_obj_prop(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    let obj_datum_ref = reserve_player_mut(|player| {
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      Ok(scope.stack.pop().unwrap())
    })?;
    reserve_player_mut(|player| {
      let prop_name = get_name(&player, &ctx, bytecode.obj as u16).unwrap();
      let result_ref = get_obj_prop(player, obj_datum_ref, &prop_name.to_owned())?;
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_ref);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn get_movie_prop(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let prop_name = get_name(&player, &ctx, bytecode.obj as u16).unwrap();
      let value = player.get_movie_prop(&prop_name)?;
      let result_id = player.alloc_datum(value);
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn set(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      let property_id_ref = scope.stack.pop().unwrap();
      let property_id = get_datum(property_id_ref, &player.datums).int_value(&player.datums)?;
      let value_ref = scope.stack.pop().unwrap();
      let value = get_datum(value_ref, &player.datums).clone();

      let property_type = bytecode.obj;
      match property_type {
        0x00 => {
          if property_id <= 0x0b { 
            // movie prop
            let prop_name = MOVIE_PROP_NAMES.get(&(property_id as u16)).unwrap();
            GetSetUtils::set_the_built_in_prop(player, ctx, prop_name, value)?;
            Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
          } else { 
            // last chunk
            Err(ScriptError::new(format!("Invalid propertyType/propertyID for kOpSet: {}", property_type)))
          }
        }
        0x07 => {
          let prop_name = get_anim_prop_name(property_id as u16);
          player.set_movie_prop(&prop_name, value)?;
          Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
        },
        _ => Err(ScriptError::new(format!("Invalid propertyType/propertyID for kOpSet: {}", property_type))),
      }
    })
  }

  pub fn get_global(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let value_ref = {
        let prop_name = get_name(&player, &ctx, bytecode.obj as u16).unwrap();
        *player.globals.get(prop_name).unwrap_or(&VOID_DATUM_REF)
      };
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(value_ref);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn set_global(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      let value_ref = scope.stack.pop().unwrap();
      let prop_name = get_name(&player, &ctx, bytecode.obj as u16).unwrap();
      player.globals.insert(prop_name.to_owned(), value_ref);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn get_field(_: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let cast_id = if player.movie.dir_version >= 500 {
        let cast_id_ref = {
          let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
          scope.stack.pop().unwrap()
        };
        get_datum(cast_id_ref, &player.datums)
      } else {
        &Datum::Int(0)
      };
      let field_name_or_num_ref = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        scope.stack.pop().unwrap()
      };
      let field_name_or_num = get_datum(field_name_or_num_ref, &player.datums);
      let field_value = player.movie.cast_manager.get_field_value_by_identifiers(field_name_or_num, Some(cast_id), &player.datums)?;
      let result_id = player.alloc_datum(Datum::String(field_value));

      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn get_local(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let name_int = bytecode.obj as u32 / get_current_variable_multiplier(player, &ctx);
      let (_, handler) = get_current_handler_def(&player, &ctx).unwrap();
      let name_id = handler.local_name_ids[name_int as usize];
      
      let var_name = get_name(&player, &ctx, name_id).unwrap();

      let scope = player.scopes.get(ctx.scope_ref).unwrap();
      let value_ref = *scope.locals.get(var_name).unwrap_or(&VOID_DATUM_REF);

      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(value_ref);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn set_local(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let name_int = bytecode.obj as u32 / get_current_variable_multiplier(player, &ctx);
      let (_, handler) = get_current_handler_def(&player, &ctx).unwrap();
      let name_id = handler.local_name_ids[name_int as usize];
      let var_name = get_name(&player, &ctx, name_id).unwrap().to_owned();

      let value_ref = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        scope.stack.pop().unwrap()
      };

      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.locals.insert(var_name.to_owned(), value_ref);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn get_param(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    let param_number = bytecode.obj as usize;
    reserve_player_mut(|player| {
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      let result = scope.args.get(param_number).unwrap_or(&VOID_DATUM_REF);
      scope.stack.push(*result);
    });
    Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
  }

  pub fn set_param(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      let arg_count = scope.args.len();
      let arg_index = bytecode.obj as usize;
      let value_ref = scope.stack.pop().unwrap();

      if arg_index < scope.args.len() {
        scope.args[arg_index] = value_ref;
        Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
      } else {
        scope.args.resize(arg_count.max(arg_index), VOID_DATUM_REF);
        scope.args.insert(arg_index, value_ref);
        Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
      }
    })
  }

  pub fn set_movie_prop(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let prop_name = get_name(&player, &ctx, bytecode.obj as u16).unwrap().to_owned();
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      let value_ref = scope.stack.pop().unwrap();
      let value = get_datum(value_ref, &player.datums).clone();
      player.set_movie_prop(&prop_name, value)?;
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn the_built_in(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let prop_name = get_name(&player, &ctx, bytecode.obj as u16).unwrap();
      let result_id = GetSetUtils::get_the_built_in_prop(player, ctx, &prop_name.clone())?;

      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.pop(); // empty arglist
      scope.stack.push(result_id);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }
  
  pub fn get_chained_prop(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let obj_ref = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        scope.stack.pop().unwrap()
      };
      let prop_name = get_name(&player, &ctx, bytecode.obj as u16).unwrap();
      let result_ref = get_obj_prop(player, obj_ref, &prop_name.to_owned())?;
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_ref);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn get(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let prop_id = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        scope.stack.pop().unwrap()
      };
      let prop_id = player.get_datum(prop_id).int_value(&player.datums)?;
      let prop_type = bytecode.obj;
      let max_movie_prop_id = *MOVIE_PROP_NAMES.keys().max().unwrap();

      let result = if prop_type == 0 && prop_id <= max_movie_prop_id as i32 {
        // movie prop
        let prop_name = MOVIE_PROP_NAMES.get(&(prop_id as u16)).unwrap();
        GetSetUtils::get_the_built_in_prop(player, ctx, prop_name)
      } else if prop_type == 0 {
        // last chunk
        let string_id = {
          let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
          scope.stack.pop().unwrap()
        };
        let string = player.get_datum(string_id).string_value(&player.datums)?;
        let chunk_type = StringChunkType::from(&(prop_id - 0x0b));
        let last_chunk = StringChunkUtils::resolve_last_chunk(&string, chunk_type, &player.movie.item_delimiter)?;

        Ok(player.alloc_datum(Datum::String(last_chunk)))
      } else if prop_type == 0x07 {
        // anim prop
        Ok(player.alloc_datum(player.get_anim_prop(prop_id as u16)?))
      } else if prop_type == 0x08 {
        // anim2 prop
        let result = if prop_id == 0x02 && player.movie.dir_version >= 500 {
          // the number of castMembers supports castLib selection from Director 5.0
          let cast_lib_id = {
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.pop().unwrap()
          };
          let cast_lib_id = player.get_datum(cast_lib_id);
          let cast = if cast_lib_id.is_string() {
            player.movie.cast_manager.get_cast_by_name(&cast_lib_id.string_value(&player.datums)?)
          } else {
            player.movie.cast_manager.get_cast_or_null(cast_lib_id.int_value(&player.datums)? as u32)
          };
          match cast {
            Some(cast) => Ok(Datum::Int(cast.max_member_id() as i32)),
            None => Err(ScriptError::new(format!("kOpSet cast not found")))
          }
        } else {
          player.get_anim2_prop(prop_id as u16)
        }?;
        Ok(player.alloc_datum(result))
      } else if prop_type == 0x01 {
        // number of chunks
        let string_id = {
          let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
          scope.stack.pop().unwrap()
        };
        let string = player.get_datum(string_id).string_value(&player.datums)?;
        let chunk_type = StringChunkType::from(&prop_id);
        let chunks = StringChunkUtils::resolve_chunk_list(&string, chunk_type, &player.movie.item_delimiter)?;
        Ok(player.alloc_datum(Datum::Int(chunks.len() as i32)))
      } else {
        Err(ScriptError::new(format!("OpCode.kOpGet call not implemented propertyID={} propertyType={}", prop_id, prop_type)))
      }?;

      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn get_top_level_prop(bytecode: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let prop_name = get_name(&player, &ctx, bytecode.obj as u16).unwrap().clone();
      let result = match prop_name.as_str() {
        "_player" => Ok(Datum::PlayerRef),
        "_movie" => Ok(Datum::MovieRef),
        _ => Err(ScriptError::new(format!("Invalid top level prop: {}", prop_name)))
      }?;
      let result_id = player.alloc_datum(result);
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }
}