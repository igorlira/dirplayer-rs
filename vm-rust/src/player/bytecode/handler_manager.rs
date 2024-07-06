use std::{future::Future, pin::Pin};

use crate::{
    director::{
        chunks::handler::Bytecode,
        lingo::{constants::get_opcode_name, opcode::OpCode},
    },
    player::{
        allocator::player_run_allocator_cycle, bytecode::{
            arithmetics::ArithmeticsBytecodeHandler, flow_control::FlowControlBytecodeHandler,
            stack::StackBytecodeHandler,
        }, scope::ScopeRef, HandlerExecutionResultContext, ScriptError, PLAYER_LOCK
    },
};

use super::{compare::CompareBytecodeHandler, get_set::GetSetBytecodeHandler, string::StringBytecodeHandler};

#[derive(Clone)]
pub struct BytecodeHandlerContext {
    pub scope_ref: ScopeRef,
    // pub player: RefCell<&'a DirPlayer>,
}
pub type BytecodeHandlerFunctionSync =
    fn(&Bytecode, &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError>;
pub type BytecodeHandlerFunctionAsync = Box<
    dyn Fn(
        Bytecode,
        BytecodeHandlerContext,
    )
        -> Pin<Box<dyn Future<Output = Result<HandlerExecutionResultContext, ScriptError>>>>,
>;
pub struct StaticBytecodeHandlerManager {}
impl StaticBytecodeHandlerManager {
    pub fn get_sync_handler(&self, opcode: &OpCode) -> Option<BytecodeHandlerFunctionSync> {
        match opcode {
            OpCode::Add => Some(ArithmeticsBytecodeHandler::add),
            OpCode::PushInt8 => Some(StackBytecodeHandler::push_int),
            OpCode::PushInt16 => Some(StackBytecodeHandler::push_int),
            OpCode::PushInt32 => Some(StackBytecodeHandler::push_int),
            OpCode::PushArgList => Some(StackBytecodeHandler::push_arglist),
            OpCode::PushArgListNoRet => Some(StackBytecodeHandler::push_arglist_no_ret),
            OpCode::PushSymb => Some(StackBytecodeHandler::push_symb),
            OpCode::GetProp => Some(GetSetBytecodeHandler::get_prop),
            OpCode::GetObjProp => Some(GetSetBytecodeHandler::get_obj_prop),
            OpCode::GetMovieProp => Some(GetSetBytecodeHandler::get_movie_prop),
            OpCode::Set => Some(GetSetBytecodeHandler::set),
            OpCode::Ret => Some(FlowControlBytecodeHandler::ret),
            OpCode::JmpIfZ => Some(FlowControlBytecodeHandler::jmp_if_zero),
            OpCode::Jmp => Some(FlowControlBytecodeHandler::jmp),
            OpCode::GetGlobal => Some(GetSetBytecodeHandler::get_global),
            OpCode::SetGlobal => Some(GetSetBytecodeHandler::set_global),
            OpCode::PushCons => Some(StackBytecodeHandler::push_cons),
            OpCode::PushZero => Some(StackBytecodeHandler::push_zero),
            OpCode::GetField => Some(GetSetBytecodeHandler::get_field),
            OpCode::GetLocal => Some(GetSetBytecodeHandler::get_local),
            OpCode::SetLocal => Some(GetSetBytecodeHandler::set_local),
            OpCode::GetParam => Some(GetSetBytecodeHandler::get_param),
            OpCode::SetMovieProp => Some(GetSetBytecodeHandler::set_movie_prop),
            OpCode::PushPropList => Some(StackBytecodeHandler::push_prop_list),
            OpCode::Gt => Some(CompareBytecodeHandler::gt),
            OpCode::Lt => Some(CompareBytecodeHandler::lt),
            OpCode::GtEq => Some(CompareBytecodeHandler::gt_eq),
            OpCode::LtEq => Some(CompareBytecodeHandler::lt_eq),
            OpCode::Sub => Some(ArithmeticsBytecodeHandler::sub),
            OpCode::EndRepeat => Some(FlowControlBytecodeHandler::end_repeat),
            OpCode::SetProp => Some(GetSetBytecodeHandler::set_prop),
            OpCode::PushList => Some(StackBytecodeHandler::push_list),
            OpCode::Not => Some(CompareBytecodeHandler::not),
            OpCode::NtEq => Some(CompareBytecodeHandler::nt_eq),
            OpCode::TheBuiltin => Some(GetSetBytecodeHandler::the_built_in),
            OpCode::Peek => Some(StackBytecodeHandler::peek),
            OpCode::Pop => Some(StackBytecodeHandler::pop),
            OpCode::And => Some(CompareBytecodeHandler::and),
            OpCode::Eq => Some(CompareBytecodeHandler::eq),
            OpCode::SetParam => Some(GetSetBytecodeHandler::set_param),
            OpCode::GetChainedProp => Some(GetSetBytecodeHandler::get_chained_prop),
            OpCode::ContainsStr => Some(StringBytecodeHandler::contains_str),
            OpCode::Contains0Str => Some(StringBytecodeHandler::contains_0str),
            OpCode::JoinPadStr => Some(StringBytecodeHandler::join_pad_str),
            OpCode::JoinStr => Some(StringBytecodeHandler::join_str),
            OpCode::Get => Some(GetSetBytecodeHandler::get),
            OpCode::Mod => Some(ArithmeticsBytecodeHandler::mod_handler),
            OpCode::GetChunk => Some(StringBytecodeHandler::get_chunk),
            OpCode::Put => Some(StringBytecodeHandler::put),
            OpCode::Or => Some(CompareBytecodeHandler::or),
            OpCode::Inv => Some(ArithmeticsBytecodeHandler::inv),
            OpCode::Div => Some(ArithmeticsBytecodeHandler::div),
            OpCode::PushFloat32 => Some(StackBytecodeHandler::push_f32),
            OpCode::Mul => Some(ArithmeticsBytecodeHandler::mul),
            OpCode::PushChunkVarRef => Some(StackBytecodeHandler::push_chunk_var_ref),
            OpCode::DeleteChunk => Some(StringBytecodeHandler::delete_chunk),
            OpCode::GetTopLevelProp => Some(GetSetBytecodeHandler::get_top_level_prop),
            _ => None,
        }
    }
    pub fn get_async_handler(&self, opcode: &OpCode) -> Option<BytecodeHandlerFunctionAsync> {
        match opcode {
            OpCode::NewObj => Some(Box::new(|a, b| {
                Box::pin(StackBytecodeHandler::new_obj(a, b))
            })),
            OpCode::ExtCall => Some(Box::new(|a, b| {
                Box::pin(FlowControlBytecodeHandler::ext_call(a, b))
            })),
            OpCode::ObjCall => Some(Box::new(|a, b| {
                Box::pin(FlowControlBytecodeHandler::obj_call(a, b))
            })),
            OpCode::LocalCall => Some(Box::new(|a, b| {
                Box::pin(FlowControlBytecodeHandler::local_call(a, b))
            })),
            OpCode::SetObjProp => Some(Box::new(|a, b| {
                Box::pin(GetSetBytecodeHandler::set_obj_prop(a, b))
            })),
            _ => None,
        }
    }
}

pub async fn player_execute_bytecode<'a>(
    // handler_manager: &'a BytecodeHandlerManager<'a>,
    // player: RefCell<&'a DirPlayer<'a>>,
    // player: &mut DirPlayer,
    bytecode: &Bytecode,
    ctx: &BytecodeHandlerContext,
) -> Result<HandlerExecutionResultContext, ScriptError> {
    let (sync_opt, async_opt) = {
        let player_opt = PLAYER_LOCK.try_lock().unwrap();
        let player = player_opt.as_ref().unwrap();
        let handler_manager = &player.bytecode_handler_manager;

        let sync_handler_opt = handler_manager.get_sync_handler(&bytecode.opcode);
        let async_handler_opt = handler_manager.get_async_handler(&bytecode.opcode);

        (sync_handler_opt, async_handler_opt)
    };

    let result = if let Some(sync_handler) = sync_opt {
        sync_handler(bytecode, ctx)
    } else if let Some(async_handler) = async_opt {
        async_handler(bytecode.to_owned(), ctx.to_owned()).await
    } else {
        return Err(ScriptError::new(
            format_args!(
                "No handler for opcode {} ({:#04x})",
                get_opcode_name(&bytecode.opcode),
                num::ToPrimitive::to_u16(&bytecode.opcode).unwrap()
            )
            .to_string(),
        ))
    };

    player_run_allocator_cycle();
    result
}
