use crate::{console_warn, director::{chunks::handler::Bytecode, lingo::{constants::{get_anim_prop_name, movie_prop_names}, datum::{Datum, StringChunkType}}}, player::{allocator::DatumAllocatorTrait, handlers::datum_handlers::string_chunk::StringChunkUtils, reserve_player_mut, script::{get_current_handler_def, get_current_variable_multiplier, get_name, get_obj_prop, player_set_obj_prop, script_get_prop, script_set_prop}, DatumRef, DirPlayer, HandlerExecutionResult, HandlerExecutionResultContext, ScriptError, PLAYER_OPT}};

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
        "result" => Ok(player.last_handler_result.clone()),
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
  pub fn get_prop(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
    let player = unsafe { PLAYER_OPT.as_mut().unwrap() };
    let receiver = {
      let scope = player.scopes.get(ctx.scope_ref).unwrap();
      scope.receiver.clone()
    };
    let prop_name = get_name(&player, &ctx, player.get_ctx_current_bytecode(ctx).obj as u16).unwrap().clone();
    
    if let Some(instance_ref) = receiver {
      let result = script_get_prop(player, &instance_ref, &prop_name)?;
      
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result);
      return Ok(HandlerExecutionResult::Advance);
    } else {
      Err(ScriptError::new(format!("No receiver to get prop {}", prop_name)))
    }
  }

  pub fn set_prop(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
    reserve_player_mut(|player| {
      let (value_ref, receiver) = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        let value_red = scope.stack.pop().unwrap();
        (value_red, scope.receiver.clone())
      };
      let prop_name = get_name(&player, &ctx, player.get_ctx_current_bytecode(ctx).obj as u16).unwrap();
      
      match receiver {
        Some(instance_ref) => {
          if *instance_ref == 0 {
            return Err(ScriptError::new(format!("Can't set prop {} of Void", prop_name)));
          }
          script_set_prop(player, &instance_ref, &prop_name.to_owned(), &value_ref, false)?;
          Ok(HandlerExecutionResult::Advance)
        },
        None => Err(ScriptError::new(format!("No receiver to set prop {}", prop_name)))
      }
    })
  }

  pub async fn set_obj_prop(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
    let (value, obj_datum_ref, prop_name) = reserve_player_mut(|player| {
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      let value = scope.stack.pop().unwrap();
      let obj_datum_ref = scope.stack.pop().unwrap();
      let prop_name = get_name(&player, &ctx, player.get_ctx_current_bytecode(&ctx).obj as u16).unwrap().to_owned();
      Ok((value, obj_datum_ref, prop_name))
    })?;
    player_set_obj_prop(&obj_datum_ref, &prop_name, &value).await?;
    Ok(HandlerExecutionResult::Advance)
  }

  pub fn get_obj_prop(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
    let obj_datum_ref = reserve_player_mut(|player| {
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      Ok(scope.stack.pop().unwrap())
    })?;
    reserve_player_mut(|player| {
      let prop_name = get_name(&player, &ctx, player.get_ctx_current_bytecode(ctx).obj as u16).unwrap();
      let result_ref = get_obj_prop(player, &obj_datum_ref, &prop_name.to_owned())?;
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_ref);
      Ok(HandlerExecutionResult::Advance)
    })
  }

  pub fn get_movie_prop(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
    reserve_player_mut(|player| {
      let prop_name = get_name(&player, &ctx, player.get_ctx_current_bytecode(ctx).obj as u16).unwrap();
      let value = player.get_movie_prop(&prop_name)?;
      let result_id = player.alloc_datum(value);
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResult::Advance)
    })
  }

  pub fn set(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
    reserve_player_mut(|player| {
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      let property_id_ref = scope.stack.pop().unwrap();
      let property_id = player.allocator.get_datum(&property_id_ref).int_value()?;
      let value_ref = scope.stack.pop().unwrap();
      let value = player.get_datum(&value_ref).clone();

      let property_type = player.get_ctx_current_bytecode(ctx).obj;
      match property_type {
        0x00 => {
          if property_id <= 0x0b { 
            // movie prop
            let prop_name = movie_prop_names().get(&(property_id as u16)).unwrap();
            GetSetUtils::set_the_built_in_prop(player, ctx, prop_name, value)?;
            Ok(HandlerExecutionResult::Advance)
          } else { 
            // last chunk
            Err(ScriptError::new(format!("Invalid propertyType/propertyID for kOpSet: {}", property_type)))
          }
        }
        0x07 => {
          let prop_name = get_anim_prop_name(property_id as u16);
          player.set_movie_prop(&prop_name, value)?;
          Ok(HandlerExecutionResult::Advance)
        },
        _ => Err(ScriptError::new(format!("Invalid propertyType/propertyID for kOpSet: {}", property_type))),
      }
    })
  }

  pub fn get_global(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
    reserve_player_mut(|player| {
      let value_ref = {
        let prop_name = get_name(&player, &ctx, player.get_ctx_current_bytecode(ctx).obj as u16).unwrap();
        player.globals.get(prop_name).unwrap_or(&DatumRef::Void).clone()
      };
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(value_ref);
      Ok(HandlerExecutionResult::Advance)
    })
  }

  pub fn set_global(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
    reserve_player_mut(|player| {
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      let value_ref = scope.stack.pop().unwrap();
      let prop_name = get_name(&player, &ctx, player.get_ctx_current_bytecode(ctx).obj as u16).unwrap();
      player.globals.insert(prop_name.to_owned(), value_ref);
      Ok(HandlerExecutionResult::Advance)
    })
  }

  pub fn get_field(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
    reserve_player_mut(|player| {
      let cast_id_ref = if player.movie.dir_version >= 500 {
        let cast_id_ref = {
          let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
          scope.stack.pop().unwrap()
        };
        Some(cast_id_ref)
      } else {
        None
      };
      let field_name_or_num_ref = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        scope.stack.pop().unwrap()
      };
      let cast_id = if let Some(cast_id_ref) = cast_id_ref {
        player.get_datum(&cast_id_ref)
      } else {
        &Datum::Int(0)
      };
      let field_name_or_num = player.get_datum(&field_name_or_num_ref);

      let field_value = player.movie.cast_manager.get_field_value_by_identifiers(field_name_or_num, Some(cast_id), &player.allocator)?;
      let result_id = player.alloc_datum(Datum::String(field_value));

      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResult::Advance)
    })
  }

  pub fn get_local(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
    reserve_player_mut(|player| {
      let name_int = player.get_ctx_current_bytecode(ctx).obj as u32 / get_current_variable_multiplier(player, &ctx);
      let handler = get_current_handler_def(player, &ctx);
      let name_id = handler.local_name_ids[name_int as usize];
      
      let var_name = get_name(&player, &ctx, name_id).unwrap();

      let scope = player.scopes.get(ctx.scope_ref).unwrap();
      let value_ref = scope.locals.get(var_name).unwrap_or(&DatumRef::Void).clone();

      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(value_ref);
      Ok(HandlerExecutionResult::Advance)
    })
  }

  pub fn set_local(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
    reserve_player_mut(|player| {
      let name_int = player.get_ctx_current_bytecode(ctx).obj as u32 / get_current_variable_multiplier(player, &ctx);
      let handler = get_current_handler_def(player, &ctx);
      let name_id = handler.local_name_ids[name_int as usize];

      let value_ref = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        scope.stack.pop().unwrap()
      };

      let var_name = get_name(&player, &ctx, name_id).unwrap().to_owned();
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.locals.insert(var_name, value_ref);
      Ok(HandlerExecutionResult::Advance)
    })
  }

  pub fn get_param(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
    reserve_player_mut(|player| {
      let param_number = player.get_ctx_current_bytecode(ctx).obj as usize;
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      let result = scope.args.get(param_number).unwrap_or(&DatumRef::Void).clone();
      scope.stack.push(result);
    });
    Ok(HandlerExecutionResult::Advance)
  }

  pub fn set_param(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
    reserve_player_mut(|player| {
      let bytecode_obj = player.get_ctx_current_bytecode(ctx).obj as usize;
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      let arg_count = scope.args.len();
      let arg_index = bytecode_obj;
      let value_ref = scope.stack.pop().unwrap();

      if arg_index < scope.args.len() {
        scope.args[arg_index] = value_ref;
        Ok(HandlerExecutionResult::Advance)
      } else {
        scope.args.resize(arg_count.max(arg_index), DatumRef::Void);
        scope.args.insert(arg_index, value_ref);
        Ok(HandlerExecutionResult::Advance)
      }
    })
  }

  pub fn set_movie_prop(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
    reserve_player_mut(|player| {
      let prop_name = get_name(&player, &ctx, player.get_ctx_current_bytecode(ctx).obj as u16).unwrap().to_owned();
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      let value_ref = scope.stack.pop().unwrap();
      let value = player.get_datum(&value_ref).clone();
      player.set_movie_prop(&prop_name, value)?;
      Ok(HandlerExecutionResult::Advance)
    })
  }

  pub fn the_built_in(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
    reserve_player_mut(|player| {
      let prop_name = get_name(&player, &ctx, player.get_ctx_current_bytecode(ctx).obj as u16).unwrap();
      let result_id = GetSetUtils::get_the_built_in_prop(player, ctx, &prop_name.clone())?;

      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.pop(); // empty arglist
      scope.stack.push(result_id);
      Ok(HandlerExecutionResult::Advance)
    })
  }
  
  pub fn get_chained_prop(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
    reserve_player_mut(|player| {
      let obj_ref = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        scope.stack.pop().unwrap()
      };
      let prop_name = get_name(&player, &ctx, player.get_ctx_current_bytecode(ctx).obj as u16).unwrap();
      let result_ref = get_obj_prop(player, &obj_ref, &prop_name.to_owned())?;
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_ref);
      Ok(HandlerExecutionResult::Advance)
    })
  }

  pub fn get(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
    reserve_player_mut(|player| {
      let prop_id = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        scope.stack.pop().unwrap()
      };
      let prop_id = player.get_datum(&prop_id).int_value()?;
      let prop_type = player.get_ctx_current_bytecode(ctx).obj;
      let max_movie_prop_id = *movie_prop_names().keys().max().unwrap();

      let result = if prop_type == 0 && prop_id <= max_movie_prop_id as i32 {
        // movie prop
        let prop_name = movie_prop_names().get(&(prop_id as u16)).unwrap();
        GetSetUtils::get_the_built_in_prop(player, ctx, prop_name)
      } else if prop_type == 0 {
        // last chunk
        let string_id = {
          let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
          scope.stack.pop().unwrap()
        };
        let string = player.get_datum(&string_id).string_value()?;
        let chunk_type = StringChunkType::from(&(prop_id - 0x0b));
        let last_chunk = StringChunkUtils::resolve_last_chunk(&string, chunk_type, player.movie.item_delimiter)?;

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
          let cast_lib_id = player.get_datum(&cast_lib_id);
          let cast = if cast_lib_id.is_string() {
            player.movie.cast_manager.get_cast_by_name(&cast_lib_id.string_value()?)
          } else {
            player.movie.cast_manager.get_cast_or_null(cast_lib_id.int_value()? as u32)
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
        let string = player.get_datum(&string_id).string_value()?;
        let chunk_type = StringChunkType::from(&prop_id);
        let chunks = StringChunkUtils::resolve_chunk_list(&string, chunk_type, player.movie.item_delimiter)?;
        Ok(player.alloc_datum(Datum::Int(chunks.len() as i32)))
      } else {
        Err(ScriptError::new(format!("OpCode.kOpGet call not implemented propertyID={} propertyType={}", prop_id, prop_type)))
      }?;

      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result);
      Ok(HandlerExecutionResult::Advance)
    })
  }

  pub fn get_top_level_prop(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
    reserve_player_mut(|player| {
      let prop_name = get_name(&player, &ctx, player.get_ctx_current_bytecode(ctx).obj as u16).unwrap().clone();
      let result = match prop_name.as_str() {
        "_player" => Ok(Datum::PlayerRef),
        "_movie" => Ok(Datum::MovieRef),
        _ => Err(ScriptError::new(format!("Invalid top level prop: {}", prop_name)))
      }?;
      let result_id = player.alloc_datum(result);
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResult::Advance)
    })
  }
}