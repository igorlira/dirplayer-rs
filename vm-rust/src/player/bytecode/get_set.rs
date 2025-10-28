use crate::{
    director::lingo::{
        constants::{
            get_anim_prop_name, get_sprite_prop_name, movie_prop_names, sprite_prop_names,
        },
        datum::{Datum, StringChunkType},
    },
    player::{
        allocator::DatumAllocatorTrait,
        handlers::datum_handlers::string_chunk::StringChunkUtils,
        reserve_player_mut,
        score::{sprite_get_prop, sprite_set_prop},
        script::{
            get_current_handler_def, get_current_variable_multiplier, get_name, get_obj_prop,
            player_set_obj_prop, script_get_prop, script_get_static_prop, script_set_prop,
            script_set_static_prop,
        },
        DatumRef, DirPlayer, HandlerExecutionResult, ScriptError, PLAYER_OPT,
    },
};

use super::handler_manager::BytecodeHandlerContext;
use crate::player::handlers::datum_handlers::list_handlers::ListDatumHandlers;

pub struct GetSetBytecodeHandler {}
pub struct GetSetUtils {}

impl GetSetUtils {
    pub fn get_the_built_in_prop(
        player: &mut DirPlayer,
        ctx: &BytecodeHandlerContext,
        prop_name: &str,
    ) -> Result<DatumRef, ScriptError> {
        match prop_name {
            "paramCount" => Ok(player.alloc_datum(Datum::Int(
                player.scopes.get(ctx.scope_ref).unwrap().args.len() as i32,
            ))),
            "result" => Ok(player.last_handler_result.clone()),
            _ => player.get_movie_prop(prop_name),
        }
    }

    pub fn set_the_built_in_prop(
        player: &mut DirPlayer,
        _ctx: &BytecodeHandlerContext,
        prop_name: &str,
        value: Datum,
    ) -> Result<(), ScriptError> {
        match prop_name {
            _ => player.set_movie_prop(prop_name, value),
        }
    }
}

impl GetSetBytecodeHandler {
    pub fn get_prop(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        let player = unsafe { PLAYER_OPT.as_mut().unwrap() };
        let (receiver, script_ref) = {
            let scope = player.scopes.get(ctx.scope_ref).unwrap();
            (scope.receiver.clone(), scope.script_ref.clone())
        };
        let prop_name = get_name(
            &player,
            &ctx,
            player.get_ctx_current_bytecode(ctx).obj as u16,
        )
        .unwrap()
        .clone();

        let result = if let Some(instance_ref) = receiver {
            script_get_prop(player, &instance_ref, &prop_name)?
        } else {
            script_get_static_prop(player, &script_ref, &prop_name)?
        };
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        scope.stack.push(result);
        return Ok(HandlerExecutionResult::Advance);
    }

    pub fn set_prop(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (value_ref, receiver, script_ref, prop_name) = {
                let current_obj = player.get_ctx_current_bytecode(ctx).obj as u16;
                let prop_name = get_name(&player, &ctx, current_obj)
                    .ok_or_else(|| ScriptError::new("Failed to get property name".to_string()))?
                    .to_owned();
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                let value_ref = scope.stack.pop().unwrap();
                (
                    value_ref,
                    scope.receiver.clone(),
                    scope.script_ref.clone(),
                    prop_name,
                )
            };

            match receiver {
                Some(instance_ref) => {
                    if *instance_ref == 0 {
                        return Err(ScriptError::new(format!(
                            "Can't set prop {} of Void",
                            prop_name
                        )));
                    }
                    script_set_prop(player, &instance_ref, &prop_name, &value_ref, false)?;
                    Ok(HandlerExecutionResult::Advance)
                }
                None => {
                    script_set_static_prop(player, &script_ref, &prop_name, &value_ref, true)?;
                    Ok(HandlerExecutionResult::Advance)
                }
            }
        })
    }

    pub async fn set_obj_prop(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        let (value, obj_datum_ref, prop_name) = reserve_player_mut(|player| {
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            let value = scope.stack.pop().unwrap();
            let obj_datum_ref = scope.stack.pop().unwrap();
            let prop_name = get_name(
                &player,
                &ctx,
                player.get_ctx_current_bytecode(&ctx).obj as u16,
            )
            .unwrap()
            .to_owned();
            Ok((value, obj_datum_ref, prop_name))
        })?;
        player_set_obj_prop(&obj_datum_ref, &prop_name, &value).await?;
        Ok(HandlerExecutionResult::Advance)
    }

    pub fn get_obj_prop(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            // Pop the object reference from the stack
            let obj_datum_ref = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.pop().unwrap()
            };

            let prop_name = get_name(
                &player,
                &ctx,
                player.get_ctx_current_bytecode(ctx).obj as u16,
            )
            .unwrap()
            .clone();

            let result_ref = get_obj_prop(player, &obj_datum_ref, &prop_name)?;
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_ref);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn get_movie_prop(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let prop_name = get_name(
                &player,
                &ctx,
                player.get_ctx_current_bytecode(ctx).obj as u16,
            )
            .unwrap()
            .clone();
            let result_ref = player.get_movie_prop(&prop_name)?;
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_ref);
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
                        Err(ScriptError::new(format!(
                            "Invalid propertyType/propertyID for kOpSet: {}",
                            property_type
                        )))
                    }
                }
                0x06 => {
                    let prop_name = get_sprite_prop_name(property_id as u16);
                    let datum_ref = {
                        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                        scope.stack.pop().unwrap()
                    };
                    let sprite_num = player.get_datum(&datum_ref).int_value()?;
                    sprite_set_prop(sprite_num as i16, prop_name, value)?;
                    Ok(HandlerExecutionResult::Advance)
                }
                0x07 => {
                    let prop_name = get_anim_prop_name(property_id as u16);
                    player.set_movie_prop(&prop_name, value)?;
                    Ok(HandlerExecutionResult::Advance)
                }
                _ => Err(ScriptError::new(format!(
                    "Invalid propertyType/propertyID for kOpSet: {}",
                    property_type
                ))),
            }
        })
    }

    pub fn get_global(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let value_ref = {
                let prop_name = get_name(
                    &player,
                    &ctx,
                    player.get_ctx_current_bytecode(ctx).obj as u16,
                )
                .unwrap();
                player
                    .globals
                    .get(prop_name)
                    .unwrap_or(&DatumRef::Void)
                    .clone()
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
            let prop_name = get_name(
                &player,
                &ctx,
                player.get_ctx_current_bytecode(ctx).obj as u16,
            )
            .unwrap();
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

            let field_value = player.movie.cast_manager.get_field_value_by_identifiers(
                field_name_or_num,
                Some(cast_id),
                &player.allocator,
            )?;
            let result_id = player.alloc_datum(Datum::String(field_value));

            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn get_local(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let name_int = player.get_ctx_current_bytecode(ctx).obj as u32
                / get_current_variable_multiplier(player, &ctx);
            let handler = get_current_handler_def(player, &ctx);
            let name_id = handler.local_name_ids[name_int as usize];

            let var_name = get_name(&player, &ctx, name_id).unwrap();

            let scope = player.scopes.get(ctx.scope_ref).unwrap();
            let value_ref = scope
                .locals
                .get(var_name)
                .unwrap_or(&DatumRef::Void)
                .clone();

            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(value_ref);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn set_local(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let name_int = player.get_ctx_current_bytecode(ctx).obj as u32
                / get_current_variable_multiplier(player, &ctx);
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
            let param_number = player.get_ctx_current_bytecode(ctx).obj as u32
                / get_current_variable_multiplier(player, ctx);
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            let result = scope
                .args
                .get(param_number as usize)
                .unwrap_or(&DatumRef::Void)
                .clone();
            scope.stack.push(result);
        });
        Ok(HandlerExecutionResult::Advance)
    }

    pub fn set_param(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let bytecode_obj = player.get_ctx_current_bytecode(ctx).obj as u32
                / get_current_variable_multiplier(player, ctx);
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            let arg_count = scope.args.len();
            let arg_index = bytecode_obj as usize;
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

    pub fn set_movie_prop(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let prop_name = get_name(
                &player,
                &ctx,
                player.get_ctx_current_bytecode(ctx).obj as u16,
            )
            .unwrap()
            .to_owned();
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            let value_ref = scope.stack.pop().unwrap();
            let value = player.get_datum(&value_ref).clone();
            player.set_movie_prop(&prop_name, value)?;
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn the_built_in(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let prop_name = get_name(
                &player,
                &ctx,
                player.get_ctx_current_bytecode(ctx).obj as u16,
            )
            .unwrap();
            let result_id = GetSetUtils::get_the_built_in_prop(player, ctx, &prop_name.clone())?;

            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.pop(); // empty arglist
            scope.stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn get_chained_prop(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let obj_ref = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.pop().unwrap_or(DatumRef::Void)
            };
            let prop_name = get_name(
                &player,
                &ctx,
                player.get_ctx_current_bytecode(ctx).obj as u16,
            )
            .unwrap()
            .clone();

            // Clone the datum type first
            let obj_type = player.get_datum(&obj_ref).type_enum();

            // Check if prop_name is a numeric index
            let is_numeric_index = prop_name.parse::<i32>().is_ok();

            let result_ref = match obj_type {
                crate::director::lingo::datum::DatumType::SpriteRef => {
                    // Handle sprite references - resolve to script instance first
                    let sprite_num = player.get_datum(&obj_ref).to_sprite_ref()?;
                    let sprite = player.movie.score.get_sprite(sprite_num);

                    if let Some(sprite) = sprite {
                        // Clone the script instance list to avoid borrow issues
                        let instance_refs = sprite.script_instance_list.clone();

                        // Try to get the property from the first script instance
                        for instance_ref in instance_refs {
                            if let Ok(result) = crate::player::script::script_get_prop(
                                player,
                                &instance_ref,
                                &prop_name,
                            ) {
                                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                                scope.stack.push(result);
                                return Ok(HandlerExecutionResult::Advance);
                            }
                        }

                        // If not found in script instances, try built-in sprite properties
                        match crate::player::score::sprite_get_prop(
                            player,
                            sprite_num as i16,
                            &prop_name,
                        ) {
                            Ok(datum) => {
                                let result = player.alloc_datum(datum);
                                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                                scope.stack.push(result);
                                return Ok(HandlerExecutionResult::Advance);
                            }
                            Err(_) => {
                                // Property not found anywhere
                                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                                scope.stack.push(DatumRef::Void);
                                return Ok(HandlerExecutionResult::Advance);
                            }
                        }
                    } else {
                        return Err(ScriptError::new(format!(
                            "Sprite {} does not exist",
                            sprite_num
                        )));
                    }
                }
                crate::director::lingo::datum::DatumType::String => match prop_name.as_str() {
                    "char" => {
                        use crate::director::lingo::datum::{StringChunkExpr, StringChunkType};
                        let (s_len, s_clone) = if let Datum::String(s) = player.get_datum(&obj_ref)
                        {
                            (s.len() as i32, s.clone())
                        } else {
                            unreachable!()
                        };
                        let chunk_expr = StringChunkExpr {
                            chunk_type: StringChunkType::Char,
                            start: 1,
                            end: s_len,
                            item_delimiter: player.movie.item_delimiter,
                        };
                        player.alloc_datum(Datum::StringChunk(
                            crate::director::lingo::datum::StringChunkSource::Datum(
                                obj_ref.clone(),
                            ),
                            chunk_expr,
                            s_clone,
                        ))
                    }
                    _ => get_obj_prop(player, &obj_ref, &prop_name.to_owned())?,
                },
                crate::director::lingo::datum::DatumType::List => {
                    // Handle numeric indices for lists
                    if is_numeric_index {
                        let index = prop_name.parse::<i32>().unwrap();
                        if let Datum::List(_, list, _) = player.get_datum(&obj_ref) {
                            // Lingo uses 1-based indexing
                            let zero_based_index = (index - 1) as usize;
                            if zero_based_index < list.len() {
                                list[zero_based_index].clone()
                            } else {
                                return Err(ScriptError::new(format!(
                                    "List index {} out of bounds (list has {} items)",
                                    index,
                                    list.len()
                                )));
                            }
                        } else {
                            unreachable!()
                        }
                    } else {
                        // Route all property access to ListDatumHandlers
                        ListDatumHandlers::get_prop(player, &obj_ref, &prop_name)?
                    }
                }
                crate::director::lingo::datum::DatumType::PropList => {
                    // Route all property access to get_obj_prop which handles PropList properly
                    get_obj_prop(player, &obj_ref, &prop_name.to_owned())?
                }
                crate::director::lingo::datum::DatumType::ScriptInstanceRef => {
                    // If it's a numeric index, try to find a default indexable property
                    if is_numeric_index {
                        // Common indexable properties in Director scripts
                        let indexable_property_names = vec!["aSquares", "list", "items", "data"];

                        let mut found_indexable = None;
                        for prop in indexable_property_names {
                            if let Ok(prop_ref) = get_obj_prop(player, &obj_ref, &prop.to_string())
                            {
                                // Check if this property is a list
                                if let Datum::List(_, _, _) = player.get_datum(&prop_ref) {
                                    found_indexable = Some(prop_ref);
                                    break;
                                }
                            }
                        }

                        if let Some(list_ref) = found_indexable {
                            // Now index into the list
                            let index = prop_name.parse::<i32>().unwrap();
                            if let Datum::List(_, list, _) = player.get_datum(&list_ref) {
                                // Lingo uses 1-based indexing
                                let zero_based_index = (index - 1) as usize;
                                if zero_based_index < list.len() {
                                    list[zero_based_index].clone()
                                } else {
                                    return Err(ScriptError::new(format!(
                                        "List index {} out of bounds (list has {} items)",
                                        index,
                                        list.len()
                                    )));
                                }
                            } else {
                                return Err(ScriptError::new(format!(
                                    "Internal error: Property was a list but now isn't"
                                )));
                            }
                        } else {
                            return Err(ScriptError::new(format!(
                "Cannot use numeric index '{}' on script instance - no indexable property found (tried: aSquares, list, items, data)",
                prop_name
              )));
                        }
                    } else {
                        // Regular property access
                        get_obj_prop(player, &obj_ref, &prop_name.to_owned())?
                    }
                }
                _ => get_obj_prop(player, &obj_ref, &prop_name.to_owned())?,
            };

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
                let last_chunk = StringChunkUtils::resolve_last_chunk(
                    &string,
                    chunk_type,
                    player.movie.item_delimiter,
                )?;

                Ok(player.alloc_datum(Datum::String(last_chunk)))
            } else if prop_type == 0x06 {
                // sprite prop
                let prop_name = sprite_prop_names().get(&(prop_id as u16));
                if prop_name.is_some() {
                    let datum_ref = {
                        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                        scope.stack.pop().unwrap()
                    };
                    let sprite_num = player.get_datum(&datum_ref).int_value()?;
                    let result = sprite_get_prop(player, sprite_num as i16, prop_name.unwrap())?;
                    Ok(player.alloc_datum(result))
                } else {
                    Err(ScriptError::new(format!(
                        "kOpGet sprite prop {} not implemented",
                        prop_id
                    )))
                }
            } else if prop_type == 0x07 {
                // anim prop
                Ok(player.alloc_datum(player.get_anim_prop(prop_id as u16)?))
            } else if prop_type == 0x08 {
                // anim2 prop
                let datum = if prop_id == 0x02 && player.movie.dir_version >= 500 {
                    // the number of castMembers supports castLib selection from Director 5.0
                    let cast_lib_id = {
                        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                        scope.stack.pop().unwrap()
                    };
                    let cast_lib_id = player.get_datum(&cast_lib_id);
                    let bypass_castlib_selection =
                        cast_lib_id.is_int() && cast_lib_id.int_value()? == 0;
                    if bypass_castlib_selection {
                        player.get_anim2_prop(prop_id as u16)?
                    } else {
                        let cast = {
                            if cast_lib_id.is_string() {
                                player
                                    .movie
                                    .cast_manager
                                    .get_cast_by_name(&cast_lib_id.string_value()?)
                            } else {
                                player
                                    .movie
                                    .cast_manager
                                    .get_cast_or_null(cast_lib_id.int_value()? as u32)
                            }
                        };
                        match cast {
                            Some(cast) => Datum::Int(cast.max_member_id() as i32),
                            None => return Err(ScriptError::new(format!("kOpSet cast not found"))),
                        }
                    }
                } else {
                    player.get_anim2_prop(prop_id as u16)?
                };
                Ok(player.alloc_datum(datum))
            } else if prop_type == 0x01 {
                // number of chunks
                let string_id = {
                    let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                    scope.stack.pop().unwrap()
                };
                let string = player.get_datum(&string_id).string_value()?;
                let chunk_type = StringChunkType::from(&prop_id);
                let chunks = StringChunkUtils::resolve_chunk_list(
                    &string,
                    chunk_type,
                    player.movie.item_delimiter,
                )?;
                Ok(player.alloc_datum(Datum::Int(chunks.len() as i32)))
            } else {
                Err(ScriptError::new(format!(
                    "OpCode.kOpGet call not implemented propertyID={} propertyType={}",
                    prop_id, prop_type
                )))
            }?;

            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn get_top_level_prop(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let prop_name = get_name(
                &player,
                &ctx,
                player.get_ctx_current_bytecode(ctx).obj as u16,
            )
            .unwrap()
            .clone();
            let result = match prop_name.as_str() {
                "_player" => Ok(Datum::PlayerRef),
                "_movie" => Ok(Datum::MovieRef),
                _ => Err(ScriptError::new(format!(
                    "Invalid top level prop: {}",
                    prop_name
                ))),
            }?;
            let result_id = player.alloc_datum(result);
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }
}
