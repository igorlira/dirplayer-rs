use std::collections::VecDeque;
use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{
        DatumRef, HandlerExecutionResult, PLAYER_OPT, ScriptError, context_vars::player_get_context_var, handlers::datum_handlers::script::ScriptDatumHandlers, reserve_player_mut, script::{get_current_handler_def, get_current_script, get_name}, symbols::builtin::BuiltInSymbol
    },
};

use super::handler_manager::BytecodeHandlerContext;

pub struct StackBytecodeHandler {}

impl StackBytecodeHandler {
    pub fn push_int(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let n = player.get_ctx_current_bytecode(ctx).obj as i32;
            // Inline: store the int directly on the stack — no Datum, no DatumRef,
            // no arena. Materialized to a DatumRef only if/when a consumer needs one.
            player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push_int(n);
        });
        Ok(HandlerExecutionResult::Advance)
    }

    pub fn push_f32(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let obj_value = player.get_ctx_current_bytecode(ctx).obj as u32;
            
            // Interpret the 32 bits as f32, THEN convert to f64
            let float_f32 = f32::from_bits(obj_value);
            let float_f64 = float_f32 as f64;
            
            player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push_float(float_f64);
        });
        Ok(HandlerExecutionResult::Advance)
    }

    /// Mark the top `n` stack entries as a call's arguments.
    ///
    /// This used to drain them into a `VecDeque` and allocate a
    /// `Datum::List(ArgList, ..)` — a heap allocation plus an arena allocation —
    /// for the sole benefit of the call opcode that immediately follows, which
    /// took the list straight back apart. `pusharglistnoret` and `pusharglist`
    /// together are ~17% of Agent Free Ride's profile and the most expensive ops
    /// in the interpreter bench (85-91 ns/op). Leave the arguments where they
    /// are and push a marker instead: no allocation at all.
    pub fn push_arglist(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        Self::push_arg_marker(ctx, false)
    }

    fn push_arg_marker(
        ctx: &BytecodeHandlerContext,
        no_ret: bool,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let count = player.get_ctx_current_bytecode(ctx).obj as usize;
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            if scope.stack.len() < count {
                return Err(ScriptError::new(
                    "Not enough items in stack to create arglist".to_string(),
                ));
            }
            scope.stack.push_value(crate::player::scope::StackDatum::ArgMarker {
                count: count as u16,
                no_ret,
            });
            Ok(())
        })?;
        Ok(HandlerExecutionResult::Advance)
    }

    /// As `push_arglist`, for a call whose result is discarded.
    pub fn push_arglist_no_ret(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        Self::push_arg_marker(ctx, true)
    }

    pub fn push_symb(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        let player = unsafe { crate::player::player_mut() };
        let name_id = player.get_ctx_current_bytecode(ctx).obj;
        // ctx.get_name indexes ctx.names_ptr directly — no per-op get_cast lookup
        // (the free get_name() re-resolves the cast's name table every call).
        let symbol_name = ctx.get_name(name_id as u16);
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        scope.stack.push_symbol(symbol_name);
        Ok(HandlerExecutionResult::Advance)
    }

    pub fn push_var_ref(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        let player = unsafe { crate::player::player_mut() };
        let name_id = player.get_ctx_current_bytecode(ctx).obj;
        let symbol_name = ctx.get_name(name_id as u16);
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        scope.stack.push_symbol(symbol_name);
        Ok(HandlerExecutionResult::Advance)
    }

    pub fn push_cons(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let literal_id = (player.get_ctx_current_bytecode(ctx).obj as u32 / ctx.multiplier) as usize;
            // SAFETY: script_ptr is valid for the whole handler (same assumption
            // as get_current_script). Tying the borrow to ctx rather than player
            // lets us call the &mut player fast-path allocators below.
            let script = unsafe { &*ctx.script_ptr };
            let literal = &script.chunk.literals[literal_id];
            // Fast paths: int/symbol literals build the DatumRef directly (no
            // 64-byte Datum clone + move). Other literal types clone as before.
            // Inline primitive literals (no Datum clone/alloc); other literal
            // types allocate and push a ref as before.
            match literal {
                Datum::Int(n) => { let n = *n; player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push_int(n); }
                Datum::Float(f) => { let f = *f; player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push_float(f); }
                Datum::Symbol(s) => { let s = *s; player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push_symbol(s); }
                Datum::Void => { player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push_void(); }
                other => {
                    let dr = player.alloc_datum(other.clone());
                    player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push(dr);
                }
            }
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn push_zero(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push_int(0);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn push_prop_list(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        // `[#a: 1, #b: 2]` compiles to `pusharglist N` + `pushproplist`, so the
        // key/value pairs arrive as a call-style arg marker.
        let arg_list: Vec<DatumRef> = reserve_player_mut(|player| {
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.pop_call_args().map(|(a, _)| a).ok_or_else(|| {
                ScriptError::new("push_prop_list: expected arg marker on stack".to_string())
            })
        })?;
        reserve_player_mut(|player| {
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
                .collect::<VecDeque<(DatumRef, DatumRef)>>();
            let datum_ref = player.alloc_datum(Datum::PropList(entries, false));
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(datum_ref);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn push_list(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            // `[a, b, c]` compiles to `pusharglist N` + `pushlist`, so the
            // elements arrive as a call-style arg marker, not a list datum.
            let (items, _) = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.pop_call_args().ok_or_else(|| {
                    ScriptError::new("push_list: expected arg marker on stack".to_string())
                })?
            };
            let list: VecDeque<DatumRef> = items.into();
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
            // The Pop opcode throws the values away — discard inline entries
            // without an alloc_int materialization round-trip.
            scope.stack.discard(count as usize);
        });
        Ok(HandlerExecutionResult::Advance)
    }

    pub fn push_chunk_var_ref(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        // PushChunkVarRef uses RAW (non-multiplied) variable indices,
        // unlike getparam/setparam/deletechunk which use multiplied indices.
        // e.g. for handler(me, t): deletechunk uses pushint8 8 for t (8/8=1),
        // but pushchunkvarref uses pushint8 1 for t (raw index 1).
        reserve_player_mut(|player| {
            let var_type = player.get_ctx_current_bytecode(ctx).obj as u32;
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            let id_ref = scope.stack.pop().unwrap();
            let id = player.get_datum(&id_ref).int_value()?;

            let value_ref = match var_type {
                0x4 => {
                    // argument - raw index, no variable multiplier
                    let arg_index = id as usize;
                    let scope = player.scopes.get(ctx.scope_ref).unwrap();
                    scope.args.get(arg_index).cloned().unwrap_or(DatumRef::Void)
                }
                0x5 => {
                    // local - raw index, no variable multiplier. `id` is the
                    // dense slot directly.
                    let scope = player.scopes.get(ctx.scope_ref).unwrap();
                    scope.local(id as usize).into_ref()
                }
                _ => {
                    // For other var types (field etc.), fall back to the standard path
                    let cast_id_ref = if var_type == 0x6 && player.movie.dir_version >= 500 {
                        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                        Some(scope.stack.pop().unwrap())
                    } else {
                        None
                    };
                    player_get_context_var(
                        player,
                        &id_ref,
                        cast_id_ref.as_ref(),
                        var_type,
                        ctx,
                    )?
                }
            };

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
            if !obj_type.eq_ignore_ascii_case("script") {
                return Err(ScriptError::new(format!(
                    "Cannot create new instance of non-script: {}",
                    obj_type
                )));
            }
            // `new(script "x", ..)` compiles to `pusharglist N` + `newobj`, so
            // the script and its constructor args arrive as an arg marker.
            let mut all_args = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.pop_call_args().map(|(a, _)| a).ok_or_else(|| {
                    ScriptError::new("new_obj: expected arg marker on stack".to_string())
                })?
            };
            if all_args.is_empty() {
                return Err(ScriptError::new("new_obj: arg list has no script".to_string()));
            }
            let script_arg_ref = all_args.remove(0);
            let extra_args: Vec<DatumRef> = all_args;
            let script_arg = player.get_datum(&script_arg_ref);
            let script_ref = match script_arg {
                Datum::String(script_name) => {
                    if let Some(script_ref) = player
                        .movie
                        .cast_manager
                        .find_member_ref_by_name(&script_name) {
                        player.alloc_datum(Datum::ScriptRef(script_ref))
                    } else {
                        return Err(ScriptError::new(format!(
                            "No script found with name {}",
                            script_name
                        )));
                    }
                }
                Datum::CastMember(member_ref) => {
                    player.alloc_datum(Datum::ScriptRef(member_ref.clone()))
                }
                _ => {
                    return Err(ScriptError::new(
                        "First argument to new script must be script name or CastMember".to_string(),
                    ))
                }
            };

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
