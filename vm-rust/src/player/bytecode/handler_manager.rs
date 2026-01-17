use async_recursion::async_recursion;

use crate::{
    director::{
        chunks::handler::HandlerDef,
        lingo::{constants::get_opcode_name, opcode::OpCode},
    },
    player::{
        bytecode::{
            arithmetics::ArithmeticsBytecodeHandler, flow_control::FlowControlBytecodeHandler,
            stack::StackBytecodeHandler,
        },
        scope::ScopeRef,
        script::Script,
        HandlerExecutionResult, ScriptError, PLAYER_OPT,
    },
};

use super::{
    compare::CompareBytecodeHandler, get_set::GetSetBytecodeHandler, string::StringBytecodeHandler,
    sprite_compare::SpriteCompareBytecodeHandler, expression_tracker::StackExpressionTracker,
};

thread_local! {
    pub static EXPRESSION_TRACKER: std::cell::RefCell<StackExpressionTracker> = 
        std::cell::RefCell::new(StackExpressionTracker::new());
}

fn trace_output(message: &str, trace_log_file: &str) {
    use crate::js_api::JsApi;
    
    if trace_log_file.is_empty() {
        // Use the same output as 'put' command, but without the "-- " prefix
        // since trace messages already have their own prefixes (-->, ==, etc.)
        JsApi::dispatch_debug_message(message);
    } else {
        // TODO: Append to file
        // For now, output to message window with file indicator
        JsApi::dispatch_debug_message(&format!("[{}] {}", trace_log_file, message));
    }
}

#[derive(Clone)]
pub struct BytecodeHandlerContext {
    pub scope_ref: ScopeRef,
    pub handler_def_ptr: *const HandlerDef,
    pub script_ptr: *const Script,
}
pub struct StaticBytecodeHandlerManager {}
impl StaticBytecodeHandlerManager {
    #[inline(always)]
    pub fn call_sync_handler(
        opcode: OpCode,
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        match opcode {
            OpCode::Add => ArithmeticsBytecodeHandler::add(ctx),
            OpCode::PushInt8 => StackBytecodeHandler::push_int(ctx),
            OpCode::PushInt16 => StackBytecodeHandler::push_int(ctx),
            OpCode::PushInt32 => StackBytecodeHandler::push_int(ctx),
            OpCode::PushArgList => StackBytecodeHandler::push_arglist(ctx),
            OpCode::PushArgListNoRet => StackBytecodeHandler::push_arglist_no_ret(ctx),
            OpCode::PushSymb => StackBytecodeHandler::push_symb(ctx),
            OpCode::Swap => StackBytecodeHandler::swap(ctx),
            OpCode::GetProp => GetSetBytecodeHandler::get_prop(ctx),
            OpCode::GetObjProp => GetSetBytecodeHandler::get_obj_prop(ctx),
            OpCode::GetMovieProp => GetSetBytecodeHandler::get_movie_prop(ctx),
            OpCode::Set => GetSetBytecodeHandler::set(ctx),
            OpCode::Ret => FlowControlBytecodeHandler::ret(ctx),
            OpCode::JmpIfZ => FlowControlBytecodeHandler::jmp_if_zero(ctx),
            OpCode::Jmp => FlowControlBytecodeHandler::jmp(ctx),
            OpCode::GetGlobal => GetSetBytecodeHandler::get_global(ctx),
            OpCode::SetGlobal => GetSetBytecodeHandler::set_global(ctx),
            OpCode::PushCons => StackBytecodeHandler::push_cons(ctx),
            OpCode::PushZero => StackBytecodeHandler::push_zero(ctx),
            OpCode::GetField => GetSetBytecodeHandler::get_field(ctx),
            OpCode::GetLocal => GetSetBytecodeHandler::get_local(ctx),
            OpCode::SetLocal => GetSetBytecodeHandler::set_local(ctx),
            OpCode::GetParam => GetSetBytecodeHandler::get_param(ctx),
            OpCode::SetMovieProp => GetSetBytecodeHandler::set_movie_prop(ctx),
            OpCode::PushPropList => StackBytecodeHandler::push_prop_list(ctx),
            OpCode::Gt => CompareBytecodeHandler::gt(ctx),
            OpCode::Lt => CompareBytecodeHandler::lt(ctx),
            OpCode::GtEq => CompareBytecodeHandler::gt_eq(ctx),
            OpCode::LtEq => CompareBytecodeHandler::lt_eq(ctx),
            OpCode::Sub => ArithmeticsBytecodeHandler::sub(ctx),
            OpCode::EndRepeat => FlowControlBytecodeHandler::end_repeat(ctx),
            OpCode::SetProp => GetSetBytecodeHandler::set_prop(ctx),
            OpCode::PushList => StackBytecodeHandler::push_list(ctx),
            OpCode::Not => CompareBytecodeHandler::not(ctx),
            OpCode::NtEq => CompareBytecodeHandler::nt_eq(ctx),
            OpCode::TheBuiltin => GetSetBytecodeHandler::the_built_in(ctx),
            OpCode::Peek => StackBytecodeHandler::peek(ctx),
            OpCode::Pop => StackBytecodeHandler::pop(ctx),
            OpCode::And => CompareBytecodeHandler::and(ctx),
            OpCode::Eq => CompareBytecodeHandler::eq(ctx),
            OpCode::SetParam => GetSetBytecodeHandler::set_param(ctx),
            OpCode::GetChainedProp => GetSetBytecodeHandler::get_chained_prop(ctx),
            OpCode::ContainsStr => StringBytecodeHandler::contains_str(ctx),
            OpCode::Contains0Str => StringBytecodeHandler::contains_0str(ctx),
            OpCode::JoinPadStr => StringBytecodeHandler::join_pad_str(ctx),
            OpCode::JoinStr => StringBytecodeHandler::join_str(ctx),
            OpCode::Get => GetSetBytecodeHandler::get(ctx),
            OpCode::Mod => ArithmeticsBytecodeHandler::mod_handler(ctx),
            OpCode::GetChunk => StringBytecodeHandler::get_chunk(ctx),
            OpCode::Put => StringBytecodeHandler::put(ctx),
            OpCode::Or => CompareBytecodeHandler::or(ctx),
            OpCode::Inv => ArithmeticsBytecodeHandler::inv(ctx),
            OpCode::Div => ArithmeticsBytecodeHandler::div(ctx),
            OpCode::PushFloat32 => StackBytecodeHandler::push_f32(ctx),
            OpCode::Mul => ArithmeticsBytecodeHandler::mul(ctx),
            OpCode::PushChunkVarRef => StackBytecodeHandler::push_chunk_var_ref(ctx),
            OpCode::DeleteChunk => StringBytecodeHandler::delete_chunk(ctx),
            OpCode::GetTopLevelProp => GetSetBytecodeHandler::get_top_level_prop(ctx),
            OpCode::PutChunk => StringBytecodeHandler::put_chunk(ctx),
            OpCode::OntoSpr => SpriteCompareBytecodeHandler::onto_sprite(ctx),
            OpCode::IntoSpr => SpriteCompareBytecodeHandler::into_sprite(ctx),
            _ => {
                let prim = num::ToPrimitive::to_u16(&opcode).unwrap();
                let name = get_opcode_name(opcode);
                let fmt = format!("No handler for opcode {name} ({prim:#04x})");
                Err(ScriptError::new(fmt))
            }
        }
    }

    #[inline(always)]
    pub fn has_async_handler(opcode: &OpCode) -> bool {
        match opcode {
            OpCode::NewObj => true,
            OpCode::ExtCall => true,
            OpCode::ObjCall => true,
            OpCode::LocalCall => true,
            OpCode::SetObjProp => true,
            _ => false,
        }
    }

    #[inline(always)]
    pub async fn call_async_handler(
        opcode: OpCode,
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        match opcode {
            OpCode::NewObj => StackBytecodeHandler::new_obj(&ctx).await,
            OpCode::ExtCall => FlowControlBytecodeHandler::ext_call(&ctx).await,
            OpCode::ObjCall => FlowControlBytecodeHandler::obj_call(&ctx).await,
            OpCode::LocalCall => FlowControlBytecodeHandler::local_call(&ctx).await,
            OpCode::SetObjProp => GetSetBytecodeHandler::set_obj_prop(&ctx).await,
            _ => {
                let prim = num::ToPrimitive::to_u16(&opcode).unwrap();
                let name = get_opcode_name(opcode);
                let fmt = format!("No handler for opcode {name} ({prim:#04x})");
                Err(ScriptError::new(fmt))
            }
        }
    }
}

#[async_recursion(?Send)]
#[inline(always)]
pub async fn player_execute_bytecode<'a>(
    ctx: &BytecodeHandlerContext,
) -> Result<HandlerExecutionResult, ScriptError> {
    let (opcode, bytecode_text, should_trace) = {
        let player = unsafe { PLAYER_OPT.as_ref().unwrap() };
        let scope = player.scopes.get(ctx.scope_ref).unwrap();

        let handler = unsafe { &*ctx.handler_def_ptr };
        let script = unsafe { &*ctx.script_ptr };
        let bytecode = &handler.bytecode_array[scope.bytecode_index];

        let should_trace = player.movie.trace_script;
        let bytecode_text = if should_trace {
            let cast = player.movie.cast_manager
                .get_cast(script.member_ref.cast_lib as u32)
                .unwrap();
            let lctx = cast.lctx.as_ref().unwrap();
            let multiplier = crate::director::file::get_variable_multiplier(
                cast.capital_x,
                cast.dir_version
            );

            // Generate annotation using expression tracker
            let annotation = EXPRESSION_TRACKER.with(|tracker| {
                let mut tracker = tracker.borrow_mut();
                
                // Get literals from script
                let script = unsafe { &*ctx.script_ptr };
                let literals = &script.chunk.literals;
                
                tracker.process_bytecode(bytecode, lctx, handler, multiplier, literals)
            });

            // Format like LASM
            let op_name = crate::director::lingo::constants::get_opcode_name(bytecode.opcode);
            let mut text = format!("[{:3}] {}", bytecode.pos, op_name);
            
            // Add operand for some opcodes
            match bytecode.opcode {
                OpCode::SetLocal | OpCode::GetLocal | OpCode::SetParam | OpCode::GetParam => {
                    // These show the variable name in the opcode part
                }
                _ if bytecode.obj != 0 => {
                    text.push_str(&format!(" {}", bytecode.obj));
                }
                _ => {}
            }
            
            // Pad with dots
            let current_len = text.len();
            let target_len = 42;
            if current_len < target_len {
                text.push(' ');
                text.push_str(&".".repeat(target_len - current_len));
            }
            
            // Add annotation
            if !annotation.is_empty() {
                text.push(' ');
                text.push_str(&annotation);
            }
            
            text
        } else {
            String::new()
        };

        (bytecode.opcode, bytecode_text, should_trace)
    };

    // Trace bytecode execution before running
    if should_trace {
        let trace_file = {
            let player = unsafe { PLAYER_OPT.as_ref().unwrap() };
            player.movie.trace_log_file.clone()
        };
        
        let msg = format!("--> {}", bytecode_text);
        trace_output(&msg, &trace_file);
    }

    // Execute the bytecode
    let result = if StaticBytecodeHandlerManager::has_async_handler(&opcode) {
        StaticBytecodeHandlerManager::call_async_handler(opcode, ctx).await
    } else {
        StaticBytecodeHandlerManager::call_sync_handler(opcode, ctx)
    };

    // Trace assignment results after execution (for specific opcodes)
    if should_trace && result.is_ok() {
        match opcode {
            OpCode::SetLocal | OpCode::SetGlobal | OpCode::SetParam => {
                let (trace_file, var_name, value) = {
                    let player = unsafe { PLAYER_OPT.as_ref().unwrap() };
                    let scope = player.scopes.get(ctx.scope_ref).unwrap();
                    let handler = unsafe { &*ctx.handler_def_ptr };
                    let script = unsafe { &*ctx.script_ptr };
                    let bytecode = &handler.bytecode_array[scope.bytecode_index];
                    
                    // Get lingo_context and multiplier from the cast
                    let cast = player.movie.cast_manager
                        .get_cast(script.member_ref.cast_lib as u32)
                        .unwrap();
                    let lctx = cast.lctx.as_ref().unwrap();
                    let multiplier = crate::director::file::get_variable_multiplier(
                        cast.capital_x,
                        cast.dir_version
                    );
                    
                    let var_name = match opcode {
                        OpCode::SetLocal => {
                            let local_index = (bytecode.obj as u32 / multiplier) as usize;
                            handler.local_name_ids
                                .get(local_index)
                                .and_then(|&name_id| lctx.names.get(name_id as usize))
                                .map(|s| s.as_str())
                                .unwrap_or("UNKNOWN")
                                .to_string()
                        }
                        OpCode::SetGlobal => {
                            lctx.names
                                .get(bytecode.obj as usize)
                                .map(|s| s.as_str())
                                .unwrap_or("UNKNOWN")
                                .to_string()
                        }
                        OpCode::SetParam => {
                            let param_index = (bytecode.obj as u32 / multiplier) as usize;
                            handler.argument_name_ids
                                .get(param_index)
                                .and_then(|&name_id| lctx.names.get(name_id as usize))
                                .map(|s| s.as_str())
                                .unwrap_or("UNKNOWN")
                                .to_string()
                        }
                        _ => "UNKNOWN".to_string()
                    };
                    
                    // Get the value that was just set (should be on top of stack or stored)
                    let value_str = if scope.stack.len() > 0 {
                        use crate::player::datum_formatting::format_datum;
                        let value_ref = &scope.stack[scope.stack.len() - 1];
                        format_datum(value_ref, player)
                    } else {
                        "void".to_string()
                    };
                    
                    let trace_file = player.movie.trace_log_file.clone();
                    (trace_file, var_name, value_str)
                };
                
                let msg = format!("== {} = {}", var_name, value);
                trace_output(&msg, &trace_file);
            }
            _ => {}
        }
    }

    result
}
