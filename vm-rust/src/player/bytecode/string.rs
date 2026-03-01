use crate::{
    director::lingo::datum::{
        datum_bool, Datum, StringChunkExpr, StringChunkSource, StringChunkType,
    },
    player::{
        context_vars::{player_get_context_var, player_set_context_var, read_context_var_args},
        datum_formatting::{format_concrete_datum, datum_to_string_for_concat},
        datum_ref::DatumRef,
        handlers::datum_handlers::string_chunk::StringChunkUtils,
        reserve_player_mut, DirPlayer, HandlerExecutionResult, ScriptError,
    },
};

use super::handler_manager::BytecodeHandlerContext;

pub enum PutType {
    Into,
    After,
    Before,
}

impl From<u8> for PutType {
    fn from(val: u8) -> Self {
        match val {
            0x01 => PutType::Into,
            0x02 => PutType::After,
            0x03 => PutType::Before,
            _ => panic!("Invalid put type"),
        }
    }
}

pub struct StringBytecodeHandler {}

impl StringBytecodeHandler {
    fn get_datum_concat_value(datum: &Datum, player: &DirPlayer) -> Result<String, ScriptError> {
        match datum {
            Datum::String(s) => Ok(s.clone()),
            Datum::StringChunk(..) => datum.string_value(),
            Datum::Int(i) => Ok(i.to_string()),
            Datum::Symbol(s) => Ok(s.to_string()),
            Datum::Void => Ok("".to_string()),
            _ => Ok(format_concrete_datum(datum, &player)),
        }
    }

    pub fn concat_datums(
        left: DatumRef,
        right: DatumRef,
        player: &mut DirPlayer,
        pad: bool,
    ) -> Result<DatumRef, ScriptError> {
        let right = player.get_datum(&right);
        let left = player.get_datum(&left);

        let right = Self::get_datum_concat_value(right, &player)?;
        let left = Self::get_datum_concat_value(left, &player)?;

        let result = if pad {
            format!("{} {}", left, right)
        } else {
            format!("{}{}", left, right)
        };
        Ok(player.alloc_datum(Datum::String(result)))
    }

    pub fn contains_str(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (search_in, search_str) = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                let search_str = scope.stack.pop().unwrap();
                let search_in = scope.stack.pop().unwrap();
                (search_in, search_str)
            };
            let search_str = player.get_datum(&search_str).string_value()?;
            let search_in = player.get_datum(&search_in);

            // Director's `contains` operator is case-insensitive
            let search_str_lower = search_str.to_ascii_lowercase();
            let contains = if search_in.is_list() {
                let search_list = search_in.to_list()?;
                let mut contains = false;
                for item in search_list {
                    let item = player.get_datum(item);
                    if item.is_string() {
                        let item = item.string_value()?;
                        if item.to_ascii_lowercase().contains(search_str_lower.as_str()) {
                            contains = true;
                            break;
                        }
                    }
                }
                Ok(contains)
            } else if search_in.is_string() {
                let search_in = search_in.string_value()?;
                Ok(search_in.to_ascii_lowercase().contains(search_str_lower.as_str()))
            } else if search_in.is_symbol() {
                Ok(false)
            } else if search_in.is_number() {
                Ok(false)
            } else {
                Err(ScriptError::new(
                    "kOpContainsStr invalid search subject".to_string(),
                ))
            }?;

            let result_id = player.alloc_datum(datum_bool(contains));
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn join_pad_str(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (left_id, right_id) = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                let right = scope.stack.pop().unwrap();
                let left = scope.stack.pop().unwrap();
                (left, right)
            };

            let result_id = Self::concat_datums(left_id, right_id, player, true)?;
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn join_str(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (left_ref, right_ref) = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                let right_ref = scope.stack.pop().unwrap();
                let left_ref = scope.stack.pop().unwrap();
                (left_ref, right_ref)
            };
            
            // Get the actual datums
            let left = player.get_datum(&left_ref);
            let right = player.get_datum(&right_ref);
            
            let left_str = datum_to_string_for_concat(left, player);
            let right_str = datum_to_string_for_concat(right, player);
            
            // Concatenate
            let result = Datum::String(format!("{}{}", left_str, right_str));
            let result_ref = player.alloc_datum(result);
            
            // Push result back to stack
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_ref);
            
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn put(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let bytecode = player.get_ctx_current_bytecode(ctx);
            let put_type = PutType::from(((bytecode.obj >> 4) & 0xF) as u8);
            let var_type = (bytecode.obj & 0xF) as u32;
            let (id_ref, cast_id_ref) = read_context_var_args(player, var_type, ctx.scope_ref);
            let value_ref = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.pop().unwrap()
            };

            match put_type {
                PutType::Into => player_set_context_var(
                    player,
                    &id_ref,
                    cast_id_ref.as_ref(),
                    var_type,
                    &value_ref,
                    put_type,
                    &ctx,
                )?,
                PutType::Before => {
                    let curr_string_id = player_get_context_var(
                        player,
                        &id_ref,
                        cast_id_ref.as_ref(),
                        var_type,
                        &ctx,
                    )?;
                    let curr_string = player.get_datum(&curr_string_id);
                    let curr_string = curr_string.string_value()?;
                    let value = player.get_datum(&value_ref);

                    let mut new_string = String::new();
                    new_string.push_str(value.string_value()?.as_str());
                    new_string.push_str(curr_string.as_str());
                    let new_string = player.alloc_datum(Datum::String(new_string));
                    // Already built the complete string, use Into to replace.
                    player_set_context_var(
                        player,
                        &id_ref,
                        cast_id_ref.as_ref(),
                        var_type,
                        &new_string,
                        PutType::Into,
                        &ctx,
                    )?;
                }
                PutType::After => {
                    let curr_string_id = player_get_context_var(
                        player,
                        &id_ref,
                        cast_id_ref.as_ref(),
                        var_type,
                        &ctx,
                    )?;
                    let curr_string = player.get_datum(&curr_string_id);
                    let curr_string = curr_string.string_value()?;
                    let value = player.get_datum(&value_ref);

                    let mut new_string = String::new();
                    new_string.push_str(curr_string.as_str());
                    new_string.push_str(value.string_value()?.as_str());
                    let new_string = player.alloc_datum(Datum::String(new_string));
                    // Already built the complete string, use Into to replace.
                    player_set_context_var(
                        player,
                        &id_ref,
                        cast_id_ref.as_ref(),
                        var_type,
                        &new_string,
                        PutType::Into,
                        &ctx,
                    )?;
                }
            }

            Ok(HandlerExecutionResult::Advance)
        })
    }

    fn read_single_chunk_ref(player: &mut DirPlayer, ctx: &BytecodeHandlerContext) -> Result<StringChunkExpr, ScriptError> {
        let (last_line, first_line, last_item, first_item, last_word, first_word, last_char, first_char) = {
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            let last_line = scope.stack.pop().unwrap();
            let first_line = scope.stack.pop().unwrap();
            let last_item = scope.stack.pop().unwrap();
            let first_item = scope.stack.pop().unwrap();
            let last_word = scope.stack.pop().unwrap();
            let first_word = scope.stack.pop().unwrap();
            let last_char = scope.stack.pop().unwrap();
            let first_char = scope.stack.pop().unwrap();
            (last_line, first_line, last_item, first_item, last_word, first_word, last_char, first_char)
        };
        
        let last_line = player.get_datum(&last_line).int_value()?;
        let first_line = player.get_datum(&first_line).int_value()?;
        let last_item = player.get_datum(&last_item).int_value()?;
        let first_item = player.get_datum(&first_item).int_value()?;
        let last_word = player.get_datum(&last_word).int_value()?;
        let first_word = player.get_datum(&first_word).int_value()?;
        let last_char = player.get_datum(&last_char).int_value()?;
        let first_char = player.get_datum(&first_char).int_value()?;
        
        if first_line != 0 || last_line != 0 {
            Ok(StringChunkExpr {
                chunk_type: StringChunkType::Line,
                start: first_line,
                end: last_line,
                item_delimiter: player.movie.item_delimiter.to_owned(),
            })
        } else if first_item != 0 || last_item != 0 {
            Ok(StringChunkExpr {
                chunk_type: StringChunkType::Item,
                start: first_item,
                end: last_item,
                item_delimiter: player.movie.item_delimiter.to_owned(),
            })
        } else if first_word != 0 || last_word != 0 {
            Ok(StringChunkExpr {
                chunk_type: StringChunkType::Word,
                start: first_word,
                end: last_word,
                item_delimiter: player.movie.item_delimiter.to_owned(),
            })
        } else if first_char != 0 || last_char != 0 {
            Ok(StringChunkExpr {
                chunk_type: StringChunkType::Char,
                start: first_char,
                end: last_char,
                item_delimiter: player.movie.item_delimiter.to_owned(),
            })
        } else {
            Err(ScriptError::new("getChunk: invalid chunk range".to_string()))
        }
    }

    fn read_all_chunks(player: &mut DirPlayer,ctx: &BytecodeHandlerContext) -> Result<Vec<StringChunkExpr>, ScriptError> {
        let (last_line, first_line, last_item, first_item, last_word, first_word, last_char, first_char) = {
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            let last_line = scope.stack.pop().unwrap();
            let first_line = scope.stack.pop().unwrap();
            let last_item = scope.stack.pop().unwrap();
            let first_item = scope.stack.pop().unwrap();
            let last_word = scope.stack.pop().unwrap();
            let first_word = scope.stack.pop().unwrap();
            let last_char = scope.stack.pop().unwrap();
            let first_char = scope.stack.pop().unwrap();
            (last_line, first_line, last_item, first_item, last_word, first_word, last_char, first_char)
        };
        
        let last_line = player.get_datum(&last_line).int_value()?;
        let first_line = player.get_datum(&first_line).int_value()?;
        let last_item = player.get_datum(&last_item).int_value()?;
        let first_item = player.get_datum(&first_item).int_value()?;
        let last_word = player.get_datum(&last_word).int_value()?;
        let first_word = player.get_datum(&first_word).int_value()?;
        let last_char = player.get_datum(&last_char).int_value()?;
        let first_char = player.get_datum(&first_char).int_value()?;
        
        let mut chunks = Vec::new();
        
        // Add chunks in the order they should be applied
        if first_line != 0 || last_line != 0 {
            chunks.push(StringChunkExpr {
                chunk_type: StringChunkType::Line,
                start: first_line,
                end: last_line,
                item_delimiter: player.movie.item_delimiter.to_owned(),
            });
        }
        if first_item != 0 || last_item != 0 {
            chunks.push(StringChunkExpr {
                chunk_type: StringChunkType::Item,
                start: first_item,
                end: last_item,
                item_delimiter: player.movie.item_delimiter.to_owned(),
            });
        }
        if first_word != 0 || last_word != 0 {
            chunks.push(StringChunkExpr {
                chunk_type: StringChunkType::Word,
                start: first_word,
                end: last_word,
                item_delimiter: player.movie.item_delimiter.to_owned(),
            });
        }
        if first_char != 0 || last_char != 0 {
            chunks.push(StringChunkExpr {
                chunk_type: StringChunkType::Char,
                start: first_char,
                end: last_char,
                item_delimiter: player.movie.item_delimiter.to_owned(),
            });
        }
        
        if chunks.is_empty() {
            return Err(ScriptError::new("getChunk: no valid chunks specified".to_string()));
        }
        
        Ok(chunks)
    }

    pub fn get_chunk(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let string_ref = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.pop().unwrap()
            };
            
            // Read all chunk parameters
            let chunks = Self::read_all_chunks(player, ctx)?;
            
            // Apply chunks sequentially
            let mut result = player.get_datum(&string_ref).string_value()?;
            for chunk_expr in chunks {
                result = StringChunkUtils::resolve_chunk_expr_string(&result, &chunk_expr)?;
            }
            
            let result_datum = Datum::String(result);
            let result_ref = player.alloc_datum(result_datum);
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_ref);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn delete_chunk(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let bytecode_obj = player.get_ctx_current_bytecode(ctx).obj;
            let (id_ref, cast_id_ref) =
                read_context_var_args(player, bytecode_obj as u32, ctx.scope_ref);
            let string_ref = player_get_context_var(
                player,
                &id_ref,
                cast_id_ref.as_ref(),
                bytecode_obj as u32,
                ctx,
            )?;
            // let string = player.get_datum(string_ref);
            let chunk_expr = Self::read_single_chunk_ref(player, ctx)?;

            StringChunkUtils::delete(player, &StringChunkSource::Datum(string_ref), &chunk_expr)?;

            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn contains_0str(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (search_str_ref, search_in_ref) = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                let search_str_ref = scope.stack.pop().unwrap();
                let search_in_ref = scope.stack.pop().unwrap();
                (search_str_ref, search_in_ref)
            };
            let search_in = player.get_datum(&search_in_ref);
            let result = if search_in.is_void() {
                false
            } else {
                // Director's `starts` operator is case-insensitive
                let search_str = player.get_datum(&search_str_ref).string_value()?;
                let search_in = search_in.string_value()?;
                search_in.to_ascii_lowercase().starts_with(search_str.to_ascii_lowercase().as_str())
            };
            let result = player.alloc_datum(datum_bool(result));
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn put_chunk(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let bytecode = player.get_ctx_current_bytecode(ctx);
            let put_type = PutType::from(((bytecode.obj >> 4) & 0xF) as u8);
            let var_type = (bytecode.obj & 0xF) as u32;
            
            // Read the target variable
            let (id_ref, cast_id_ref) = read_context_var_args(player, var_type, ctx.scope_ref);
            
            // Pop the value to put from the stack
            let value_ref = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.pop().unwrap()
            };
            
            // Read the chunk expression from the stack
            let chunk_expr = Self::read_single_chunk_ref(player, ctx)?;
            
            // Get the current value of the variable
            let string_ref = player_get_context_var(
                player,
                &id_ref,
                cast_id_ref.as_ref(),
                var_type,
                ctx,
            )?;
            
            let current_string = player.get_datum(&string_ref).string_value()?;
            let value_string = player.get_datum(&value_ref).string_value()?;
            
            // Apply the chunk operation based on put type
            let new_string = match put_type {
                PutType::Into => {
                    StringChunkUtils::string_by_putting_into_chunk(&current_string, &chunk_expr, &value_string)?
                }
                PutType::Before => {
                    StringChunkUtils::string_by_putting_before_chunk(&current_string, &chunk_expr, &value_string)?
                }
                PutType::After => {
                    StringChunkUtils::string_by_putting_after_chunk(&current_string, &chunk_expr, &value_string)?
                }
            };
            
            let new_string_ref = player.alloc_datum(Datum::String(new_string));
            // The chunk operation already built the complete result string,
            // so always use Into to replace rather than append/prepend again.
            player_set_context_var(
                player,
                &id_ref,
                cast_id_ref.as_ref(),
                var_type,
                &new_string_ref,
                PutType::Into,
                ctx,
            )?;
            
            Ok(HandlerExecutionResult::Advance)
        })
    }
}
