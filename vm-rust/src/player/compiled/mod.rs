//! Register-IR compiler PoC (Stage 2 of the perf plan).
//!
//! Compiles a handler's pure-sync bytecode into a flat, pre-decoded `IrOp`
//! sequence executed by a tight loop that owns its operand stack and a DENSE
//! `Vec<StackDatum>` local file on the native Rust stack — eliminating, for the
//! compiled subset, the per-op `reserve_player`, the scope fetch, the locals
//! `FxHashMap`, and the operand-stack `UnsafeCell` indirection.
//!
//! This PoC is intentionally restricted to PURE opcodes (no calls, no globals,
//! no params, no strings/props) and INT operands, so the IR runner needs no
//! `player` access. Its only job is to answer the go/no-go question: does
//! compiling the basic-op cluster to this form actually beat the interpreter?
//! Measured by `run_ir_benchmark` against the same loops as the interpreter
//! bench. If it doesn't clearly win here, it won't help origins.

use crate::director::chunks::handler::HandlerDef;
use crate::director::lingo::opcode::OpCode;
use crate::player::scope::StackDatum;

/// Pre-decoded register-IR instruction. Jump targets are IR indices (already
/// remapped from bytecode `pos`).
#[derive(Clone, Debug)]
pub enum IrOp {
    PushInt(i32),
    GetLocal(u16),
    SetLocal(u16),
    Add,
    Sub,
    Mul,
    Lt,
    LtEq,
    Gt,
    GtEq,
    Eq,
    NtEq,
    Jmp(usize),
    JmpIfZero(usize),
    Pop(usize),
    Ret,
}

pub struct CompiledHandler {
    pub ops: Vec<IrOp>,
    pub n_locals: usize,
}

/// Try to compile a handler to the pure-int IR. Returns `None` (→ interpreter
/// fallback) if it contains any opcode outside the supported pure subset.
pub fn compile(handler: &HandlerDef, multiplier: u32) -> Option<CompiledHandler> {
    let bc = &handler.bytecode_array;
    let mut ops: Vec<IrOp> = Vec::with_capacity(bc.len());

    for b in bc {
        let op = match b.opcode {
            OpCode::PushInt8 | OpCode::PushInt16 | OpCode::PushInt32 => IrOp::PushInt(b.obj as i32),
            OpCode::PushZero => IrOp::PushInt(0),
            OpCode::GetLocal => IrOp::GetLocal((b.obj as u32 / multiplier) as u16),
            OpCode::SetLocal => IrOp::SetLocal((b.obj as u32 / multiplier) as u16),
            OpCode::Add => IrOp::Add,
            OpCode::Sub => IrOp::Sub,
            OpCode::Mul => IrOp::Mul,
            OpCode::Lt => IrOp::Lt,
            OpCode::LtEq => IrOp::LtEq,
            OpCode::Gt => IrOp::Gt,
            OpCode::GtEq => IrOp::GtEq,
            OpCode::Eq => IrOp::Eq,
            OpCode::NtEq => IrOp::NtEq,
            // Jump targets resolved below via bytecode_index_map.
            OpCode::Jmp => {
                let dest = (b.pos as i64 + b.obj) as usize;
                IrOp::Jmp(*handler.bytecode_index_map.get(&dest)? as usize)
            }
            OpCode::JmpIfZ => {
                let dest = (b.pos as i64 + b.obj) as usize;
                IrOp::JmpIfZero(*handler.bytecode_index_map.get(&dest)? as usize)
            }
            OpCode::EndRepeat => {
                let dest = (b.pos as i64 - b.obj) as usize;
                IrOp::Jmp(*handler.bytecode_index_map.get(&dest)? as usize)
            }
            OpCode::Pop => IrOp::Pop(b.obj as usize),
            OpCode::Ret => IrOp::Ret,
            // Anything else (calls, globals, params, strings, props, floats,
            // symbols, ...) → ineligible.
            _ => return None,
        };
        ops.push(op);
    }

    Some(CompiledHandler {
        ops,
        n_locals: handler.local_name_ids.len(),
    })
}

#[inline(always)]
fn as_int(d: &StackDatum) -> i32 {
    match d {
        StackDatum::Int(n) => *n,
        StackDatum::Void => 0,
        _ => 0, // PoC: pure-int subset; non-int shouldn't reach here.
    }
}

/// Run a compiled pure-int handler. `locals_init` seeds the dense local file
/// (the PoC bench uses it to set up loop counters). Returns the top of stack.
pub fn run(compiled: &CompiledHandler, locals_init: &[StackDatum]) -> StackDatum {
    let mut st: Vec<StackDatum> = Vec::with_capacity(32);
    let mut locals: Vec<StackDatum> = vec![StackDatum::Void; compiled.n_locals.max(locals_init.len())];
    for (i, v) in locals_init.iter().enumerate() {
        locals[i] = v.clone();
    }
    let ops = &compiled.ops;
    let mut pc = 0usize;
    loop {
        match &ops[pc] {
            IrOp::PushInt(n) => { st.push(StackDatum::Int(*n)); pc += 1; }
            IrOp::GetLocal(s) => { st.push(locals[*s as usize].clone()); pc += 1; }
            IrOp::SetLocal(s) => { locals[*s as usize] = st.pop().unwrap(); pc += 1; }
            IrOp::Add => { let b = st.pop().unwrap(); let a = st.pop().unwrap(); st.push(StackDatum::Int(as_int(&a) + as_int(&b))); pc += 1; }
            IrOp::Sub => { let b = st.pop().unwrap(); let a = st.pop().unwrap(); st.push(StackDatum::Int(as_int(&a).wrapping_sub(as_int(&b)))); pc += 1; }
            IrOp::Mul => { let b = st.pop().unwrap(); let a = st.pop().unwrap(); st.push(StackDatum::Int(as_int(&a).wrapping_mul(as_int(&b)))); pc += 1; }
            IrOp::Lt => { let b = st.pop().unwrap(); let a = st.pop().unwrap(); st.push(StackDatum::Int((as_int(&a) < as_int(&b)) as i32)); pc += 1; }
            IrOp::LtEq => { let b = st.pop().unwrap(); let a = st.pop().unwrap(); st.push(StackDatum::Int((as_int(&a) <= as_int(&b)) as i32)); pc += 1; }
            IrOp::Gt => { let b = st.pop().unwrap(); let a = st.pop().unwrap(); st.push(StackDatum::Int((as_int(&a) > as_int(&b)) as i32)); pc += 1; }
            IrOp::GtEq => { let b = st.pop().unwrap(); let a = st.pop().unwrap(); st.push(StackDatum::Int((as_int(&a) >= as_int(&b)) as i32)); pc += 1; }
            IrOp::Eq => { let b = st.pop().unwrap(); let a = st.pop().unwrap(); st.push(StackDatum::Int((as_int(&a) == as_int(&b)) as i32)); pc += 1; }
            IrOp::NtEq => { let b = st.pop().unwrap(); let a = st.pop().unwrap(); st.push(StackDatum::Int((as_int(&a) != as_int(&b)) as i32)); pc += 1; }
            IrOp::JmpIfZero(t) => { let c = st.pop().unwrap(); if as_int(&c) == 0 { pc = *t; } else { pc += 1; } }
            IrOp::Jmp(t) => { pc = *t; }
            IrOp::Pop(n) => { for _ in 0..*n { st.pop(); } pc += 1; }
            IrOp::Ret => return st.pop().unwrap_or(StackDatum::Void),
        }
    }
}
