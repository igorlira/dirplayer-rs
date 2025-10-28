use super::{
    bytecode::handler_manager::BytecodeHandlerContext,
    scope::ScopeRef,
    script::{get_current_handler_def, get_current_variable_multiplier, get_name},
    DatumRef, DirPlayer, ScriptError,
};
use crate::director::lingo::datum::Datum;
use crate::player::bytecode::string::PutType;
use crate::player::cast_member::CastMemberType;
use web_sys::console;

pub fn read_context_var_args(
    player: &mut DirPlayer,
    var_type: u32,
    scope_ref: ScopeRef,
) -> (DatumRef, Option<DatumRef>) {
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
    let handler = get_current_handler_def(player, &ctx);

    match var_type {
        // global | global | property/instance
        0x1 | 0x2 | 0x3 => Err(ScriptError::new(
            "readVar global/prop/instance not implemented".to_string(),
        )),
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
            let local_name = get_name(player, &ctx, local_name_ids[id.int_value()? as usize])
                .unwrap()
                .to_owned();
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            let void = DatumRef::Void;
            let local = scope.locals.get(&local_name).unwrap_or(&void);
            Ok(local.clone())
        }
        0x6 => {
            // field
            // `id_ref` is the field's identifier (e.g. member number)
            // `_cast_id_ref` might indicate which cast lib to search in
            let id_datum = player.get_datum(id_ref);

            let cast_id_datum = _cast_id_ref.map(|r| player.get_datum(r));
            let cast_id_datum_opt = cast_id_datum.as_ref().map(|d| *d);

            let text = player.movie.cast_manager.get_field_value_by_identifiers(
                id_datum,
                cast_id_datum_opt,
                &player.allocator,
            )?;

            Ok(player.alloc_datum(Datum::String(text)))
        }
        _ => Err(ScriptError::new(format!(
            "Invalid context var type: {}",
            var_type
        ))),
    }
}

pub fn player_set_context_var(
    player: &mut DirPlayer,
    id_ref: &DatumRef,
    cast_id_ref: Option<&DatumRef>,
    var_type: u32,
    value_ref: &DatumRef,
    put_type: PutType,
    ctx: &BytecodeHandlerContext,
) -> Result<(), ScriptError> {
    let variable_multiplier = get_current_variable_multiplier(player, ctx);
    let handler = get_current_handler_def(player, ctx);
    let id_datum = player.get_datum(id_ref);

    match var_type {
        // global | global | property/instance
        0x1 | 0x2 | 0x3 => Err(ScriptError::new(
            "set readVar global/prop/instance not implemented".to_string(),
        )),
        0x4 => {
            // argument
            let arg_index = (id_datum.int_value()? / variable_multiplier as i32) as usize;
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            let arg_val_ref = scope.args.get_mut(arg_index).unwrap();
            *arg_val_ref = value_ref.clone();
            Ok(())
        }
        0x5 => {
            // local
            let local_name_ids = &handler.local_name_ids;
            let local_name = get_name(player, ctx, local_name_ids[id_datum.int_value()? as usize])
                .unwrap()
                .to_owned();
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.locals.insert(local_name, value_ref.clone());
            Ok(())
        }
        0x6 => {
            // FIELD variable

            // Get the value to write
            let new_value = player.get_datum(value_ref).string_value()?;

            // Map cast_id_ref to Datum if provided
            let cast_id_opt: Option<&Datum> = cast_id_ref.map(|r| player.get_datum(r));

            // Attempt to find the member reference
            let member_ref_opt = player
                .movie
                .cast_manager
                .find_member_ref_by_identifiers(
                    &player.get_datum(id_ref),
                    cast_id_opt,
                    &player.allocator,
                )
                .map_err(|e| ScriptError::new(format!("Error finding member: {:#?}", e)))?;

            let member_ref = match member_ref_opt {
                Some(r) => r,
                None => {
                    console::log_1(&"❌ Field member not found by identifiers".into());
                    return Err(ScriptError::new("Field member not found".to_string()));
                }
            };

            // Now safely mutate the member
            if let Some(member) = player
                .movie
                .cast_manager
                .find_mut_member_by_ref(&member_ref)
            {
                match &mut member.member_type {
                    CastMemberType::Field(field) => {
                        // Apply the put_type operation
                        match put_type {
                            PutType::Into => field.text = new_value,
                            PutType::Before => {
                                let mut combined = new_value;
                                combined.push_str(&field.text);
                                field.text = combined;
                            }
                            PutType::After => {
                                field.text.push_str(&new_value);
                            }
                        }
                        Ok(())
                    }
                    other => {
                        console::log_1(
                            &format!("❌ Member exists but is not a Field: {:?}", other).into(),
                        );
                        Err(ScriptError::new(
                            "Cast member exists but is not a Field".to_string(),
                        ))
                    }
                }
            } else {
                console::log_1(
                    &format!(
                        "❌ Member reference found but no mutable member exists: {:?}",
                        member_ref
                    )
                    .into(),
                );
                Err(ScriptError::new(
                    "Field member not found in cast_manager".to_string(),
                ))
            }
        }
        _ => Err(ScriptError::new(format!(
            "set Invalid context var type: {}",
            var_type
        ))),
    }
}
