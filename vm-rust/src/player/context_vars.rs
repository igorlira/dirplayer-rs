use super::{bytecode::handler_manager::BytecodeHandlerContext, scope::ScopeRef, script::{get_current_handler_def, get_current_variable_multiplier, get_name}, DatumRef, DirPlayer, ScriptError, VOID_DATUM_REF};

pub fn read_context_var_args(player: &mut DirPlayer, var_type: u32, scope_ref: ScopeRef) -> (DatumRef, Option<DatumRef>) {
  let scope = player.scopes.get_mut(scope_ref).unwrap();
  let cast_id = if var_type == 0x6 && player.movie.dir_version >= 500 {
    // field cast ID
    Some(scope.stack.pop().unwrap())
  } else {
    None
  };
  let id = scope.stack.pop().unwrap();
  (id, cast_id)
}

pub fn player_get_context_var(
  player: &mut DirPlayer, 
  id_ref: &DatumRef,
  _cast_id_ref: Option<&DatumRef>,
  var_type: u32, 
  ctx: &BytecodeHandlerContext,
) -> Result<DatumRef, ScriptError> {
  let variable_multiplier = get_current_variable_multiplier(player, ctx);
  let id = player.get_datum(id_ref);
  let (_, handler) = get_current_handler_def(player, &ctx).unwrap();
  
  match var_type {
    // global | global | property/instance
    0x1 | 0x2 | 0x3 => Err(ScriptError::new("readVar global/prop/instance not implemented".to_string())),
    0x4 => {
      // arg
      let arg_index = (id.int_value()? / variable_multiplier as i32) as usize;
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      let arg_val_ref = scope.args.get(arg_index).unwrap();
      // let arg_name = get_name(&player, ctx.to_owned(), arg_name_ids[arg_index]).unwrap();
      Ok(arg_val_ref.clone())
    }
    0x5 => {
      // local
      let local_name_ids = &handler.local_name_ids;
      let local_name = get_name(player, &ctx, local_name_ids[id.int_value()? as usize]).unwrap().to_owned();
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      let local = scope.locals.get(&local_name).unwrap_or(&VOID_DATUM_REF);
      Ok(local.clone())
    }
    0x6 => {
      // field
      Err(ScriptError::new("readVar field not implemented".to_string()))
    }
    _ => Err(ScriptError::new(format!("Invalid context var type: {}", var_type)))
  }
}

pub fn player_set_context_var(
  player: &mut DirPlayer, 
  id_ref: &DatumRef,
  _cast_id_ref: Option<&DatumRef>,
  var_type: u32, 
  value_ref: &DatumRef, 
  ctx: &BytecodeHandlerContext, 
) -> Result<(), ScriptError> {
  let variable_multiplier = get_current_variable_multiplier(player, ctx);
  let (_, handler) = get_current_handler_def(player, &ctx).unwrap();
  let id = player.get_datum(id_ref);
  
  match var_type {
    // global | global | property/instance
    0x1 | 0x2 | 0x3 => Err(ScriptError::new("set readVar global/prop/instance not implemented".to_string())),
    0x4 => {
      // arg
      let arg_index = (id.int_value()? / variable_multiplier as i32) as usize;
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      let arg_val_ref = scope.args.get_mut(arg_index).unwrap();
      *arg_val_ref = value_ref.clone();
      Ok(())
    }
    0x5 => {
      // local
      let local_name_ids = &handler.local_name_ids;
      let local_name = get_name(player, &ctx, local_name_ids[id.int_value()? as usize]).unwrap().to_owned();
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.locals.insert(local_name, value_ref.clone());
      Ok(())
    }
    0x6 => {
      // field
      Err(ScriptError::new("set readVar field not implemented".to_string()))
    }
    _ => Err(ScriptError::new(format!("set Invalid context var type: {}", var_type)))
  }
}