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
};

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
    let opcode = {
        let player = unsafe { PLAYER_OPT.as_ref().unwrap() };
        let scope = player.scopes.get(ctx.scope_ref).unwrap();

        let handler = unsafe { &*ctx.handler_def_ptr };
        let bytecode = &handler.bytecode_array[scope.bytecode_index];

        bytecode.opcode
    };

    if StaticBytecodeHandlerManager::has_async_handler(&opcode) {
        StaticBytecodeHandlerManager::call_async_handler(opcode, ctx).await
    } else {
        StaticBytecodeHandlerManager::call_sync_handler(opcode, ctx)
    }
}
