use crate::{
    director::{
        chunks::handler::Bytecode,
        lingo::datum::{datum_bool, Datum, StringChunkExpr, StringChunkSource, StringChunkType},
    },
    player::{
        context_vars::{player_get_context_var, player_set_context_var, read_context_var_args},
        datum_formatting::format_concrete_datum,
        handlers::datum_handlers::string_chunk::StringChunkUtils,
        reserve_player_mut, DirPlayer, HandlerExecutionResult, HandlerExecutionResultContext,
        ScriptError,
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
            Datum::Float(f) => Ok(f.to_string()), // TODO how to format this?
            Datum::Symbol(s) => Ok(s.to_string()),
            Datum::Void => Ok("".to_string()),
            _ => Ok(format_concrete_datum(datum, &player)),
        }
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

            let contains = if search_in.is_list() {
                let search_list = search_in.to_list()?;
                let mut contains = false;
                for item in search_list {
                    let item = player.get_datum(item);
                    if item.is_string() {
                        let item = item.string_value()?;
                        if item.contains(search_str.as_str()) {
                            contains = true;
                            break;
                        }
                    }
                }
                Ok(contains)
            } else if search_in.is_string() {
                let search_in = search_in.string_value()?;
                Ok(search_in.contains(search_str.as_str()))
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
            let right = player.get_datum(&right_id);
            let left = player.get_datum(&left_id);

            let right = Self::get_datum_concat_value(right, &player)?;
            let left = Self::get_datum_concat_value(left, &player)?;

            let join_str = format!("{} {}", left, right);

            let result_id = player.alloc_datum(Datum::String(join_str));
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn join_str(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (left_id, right_id) = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                let right = scope.stack.pop().unwrap();
                let left = scope.stack.pop().unwrap();
                (left, right)
            };
            let right = player.get_datum(&right_id);
            let left = player.get_datum(&left_id);

            let right = Self::get_datum_concat_value(right, &player)?;
            let left = Self::get_datum_concat_value(left, &player)?;

            let join_str = format!("{}{}", left, right);

            let result_id = player.alloc_datum(Datum::String(join_str));
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_id);
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
                    player_set_context_var(
                        player,
                        &id_ref,
                        cast_id_ref.as_ref(),
                        var_type,
                        &new_string,
                        put_type,
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
                    player_set_context_var(
                        player,
                        &id_ref,
                        cast_id_ref.as_ref(),
                        var_type,
                        &new_string,
                        put_type,
                        &ctx,
                    )?;
                }
            }

            Ok(HandlerExecutionResult::Advance)
        })
    }

    fn read_chunk_ref(
        player: &mut DirPlayer,
        ctx: &BytecodeHandlerContext,
    ) -> Result<StringChunkExpr, ScriptError> {
        let (
            last_line,
            first_line,
            last_item,
            first_item,
            last_word,
            first_word,
            last_char,
            first_char,
        ) = {
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            let last_line = scope.stack.pop().unwrap();
            let first_line = scope.stack.pop().unwrap();
            let last_item = scope.stack.pop().unwrap();
            let first_item = scope.stack.pop().unwrap();
            let last_word = scope.stack.pop().unwrap();
            let first_word = scope.stack.pop().unwrap();
            let last_char = scope.stack.pop().unwrap();
            let first_char = scope.stack.pop().unwrap();
            (
                last_line, first_line, last_item, first_item, last_word, first_word, last_char,
                first_char,
            )
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
            Err(ScriptError::new(
                "getChunk: invalid chunk range".to_string(),
            ))
        }
    }

    pub fn get_chunk(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let string = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.pop().unwrap()
            };
            let chunk_expr = Self::read_chunk_ref(player, ctx)?;
            let string_value = player.get_datum(&string).string_value()?;
            let resolved_str =
                StringChunkUtils::resolve_chunk_expr_string(&string_value, &chunk_expr)?;

            let result = Datum::String(resolved_str);
            let result_ref = player.alloc_datum(result);
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
            let chunk_expr = Self::read_chunk_ref(player, ctx)?;

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
                let search_str = player.get_datum(&search_str_ref).string_value()?;
                let search_in = search_in.string_value()?;
                search_in.starts_with(search_str.as_str())
            };
            let result = player.alloc_datum(datum_bool(result));
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result);
            Ok(HandlerExecutionResult::Advance)
        })
    }
}
