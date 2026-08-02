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
use crate::director::lingo::datum::Datum;
use crate::director::lingo::opcode::OpCode;
use crate::player::compare::{datum_equals, datum_greater_than, datum_is_zero, datum_less_than};
use crate::player::datum_operations::{add_datums, multiply_datums, subtract_datums};
use crate::player::scope::{ScopeRef, StackDatum};
use crate::player::{reserve_player_mut, reserve_player_ref, DatumRef, ScriptError};

/// Pre-decoded register-IR instruction. Jump targets are IR indices (already
/// remapped from bytecode `pos`).
#[derive(Clone, Debug)]
pub enum IrOp {
    PushInt(i32),
    GetLocal(u16),
    SetLocal(u16),
    GetParam(u16),
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
            OpCode::GetParam => IrOp::GetParam((b.obj as u32 / multiplier) as u16),
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
            IrOp::GetParam(_) => unreachable!("GetParam not used by the pure-int bench runner"),
            IrOp::Ret => return st.pop().unwrap_or(StackDatum::Void),
        }
    }
}

// ---- Stage 2A: real-context runner for fully-pure handlers ----
//
// Runs a fully-compiled handler with the actual call context: params from
// `scope.args`, a dense native local file, the int fast paths, and the SAME
// datum_operations / compare functions the interpreter uses for non-int values
// (so results are identical). Writes the handler's return value into the scope.
// NOT yet wired into dispatch — exercised only by unit tests until Stage 2B.

#[inline]
fn ir_add(a: StackDatum, b: StackDatum) -> Result<StackDatum, ScriptError> {
    if let (StackDatum::Int(x), StackDatum::Int(y)) = (&a, &b) {
        return Ok(StackDatum::Int(x.wrapping_add(*y)));
    }
    let (ar, br) = (a.into_ref(), b.into_ref());
    reserve_player_mut(|player| {
        let ad = player.get_datum(&ar).clone();
        let bd = player.get_datum(&br).clone();
        let r = add_datums(ad, bd, player)?;
        Ok(StackDatum::Ref(player.alloc_datum(r)))
    })
}

#[inline]
fn ir_sub(a: StackDatum, b: StackDatum) -> Result<StackDatum, ScriptError> {
    if let (StackDatum::Int(x), StackDatum::Int(y)) = (&a, &b) {
        return Ok(StackDatum::Int(x.wrapping_sub(*y)));
    }
    let (ar, br) = (a.into_ref(), b.into_ref());
    reserve_player_mut(|player| {
        let ad = player.get_datum(&ar).clone();
        let bd = player.get_datum(&br).clone();
        let r = subtract_datums(ad, bd, player)?;
        Ok(StackDatum::Ref(player.alloc_datum(r)))
    })
}

#[inline]
fn ir_mul(a: StackDatum, b: StackDatum) -> Result<StackDatum, ScriptError> {
    if let (StackDatum::Int(x), StackDatum::Int(y)) = (&a, &b) {
        return Ok(StackDatum::Int(x.wrapping_mul(*y)));
    }
    let (ar, br) = (a.into_ref(), b.into_ref());
    reserve_player_mut(|player| {
        let r = multiply_datums(ar, br, player)?;
        Ok(StackDatum::Ref(player.alloc_datum(r)))
    })
}

/// Comparison via the interpreter's datum predicates (so non-int compares match
/// exactly). `kind`: 0=Lt 1=LtEq 2=Gt 3=GtEq 4=Eq 5=NtEq.
#[inline]
fn ir_cmp(a: StackDatum, b: StackDatum, kind: u8) -> Result<StackDatum, ScriptError> {
    if let (StackDatum::Int(x), StackDatum::Int(y)) = (&a, &b) {
        let r = match kind {
            0 => x < y, 1 => x <= y, 2 => x > y, 3 => x >= y, 4 => x == y, _ => x != y,
        };
        return Ok(StackDatum::Int(r as i32));
    }
    let (ar, br) = (a.into_ref(), b.into_ref());
    reserve_player_ref(|player| {
        let l = player.get_datum(&ar);
        let r = player.get_datum(&br);
        let res = match kind {
            0 => datum_less_than(l, r, &player.allocator)?,
            1 => datum_less_than(l, r, &player.allocator)? || datum_equals(l, r, &player.allocator)?,
            2 => datum_greater_than(l, r, &player.allocator)?,
            3 => datum_greater_than(l, r, &player.allocator)? || datum_equals(l, r, &player.allocator)?,
            4 => datum_equals(l, r, &player.allocator)?,
            _ => !datum_equals(l, r, &player.allocator)?,
        };
        Ok(StackDatum::Int(res as i32))
    })
}

#[inline]
fn ir_is_zero(v: &StackDatum) -> Result<bool, ScriptError> {
    match v {
        StackDatum::Int(n) => Ok(*n == 0),
        StackDatum::Void => Ok(true),
        other => {
            let r = other.clone().into_ref();
            reserve_player_ref(|player| datum_is_zero(player.get_datum(&r), &player.allocator))
        }
    }
}

/// Lingo strings are value types: copy on assignment. Mirrors `set_local`.
/// Inline primitives (the common case) are never strings, so they skip this.
#[inline]
fn cow_on_assign(v: StackDatum) -> StackDatum {
    if let StackDatum::Ref(dr) = &v {
        let dr = dr.clone();
        reserve_player_mut(|player| match player.get_datum(&dr) {
            Datum::String(s) => {
                let s = s.clone();
                StackDatum::Ref(player.alloc_datum(Datum::String(s)))
            }
            _ => StackDatum::Ref(dr.clone()),
        })
    } else {
        v
    }
}

/// Run a fully-pure compiled handler against `scope_ref`. Returns Ok on the
/// handler's `ret`; the caller's teardown reads `scope.return_value`.
pub fn run_handler(compiled: &CompiledHandler, scope_ref: ScopeRef) -> Result<(), ScriptError> {
    let mut st: Vec<StackDatum> = Vec::with_capacity(32);
    let mut locals: Vec<StackDatum> = vec![StackDatum::Void; compiled.n_locals];
    let ops = &compiled.ops;
    let mut pc = 0usize;
    loop {
        match &ops[pc] {
            IrOp::PushInt(n) => { st.push(StackDatum::Int(*n)); pc += 1; }
            IrOp::GetLocal(s) => { st.push(locals[*s as usize].clone()); pc += 1; }
            IrOp::SetLocal(s) => { let v = cow_on_assign(st.pop().unwrap()); locals[*s as usize] = v; pc += 1; }
            IrOp::GetParam(s) => {
                let s = *s as usize;
                let dr = reserve_player_ref(|player| {
                    player.scopes.get(scope_ref).unwrap().args.get(s).cloned().unwrap_or(DatumRef::Void)
                });
                st.push(StackDatum::Ref(dr));
                pc += 1;
            }
            IrOp::Add => { let b = st.pop().unwrap(); let a = st.pop().unwrap(); st.push(ir_add(a, b)?); pc += 1; }
            IrOp::Sub => { let b = st.pop().unwrap(); let a = st.pop().unwrap(); st.push(ir_sub(a, b)?); pc += 1; }
            IrOp::Mul => { let b = st.pop().unwrap(); let a = st.pop().unwrap(); st.push(ir_mul(a, b)?); pc += 1; }
            IrOp::Lt => { let b = st.pop().unwrap(); let a = st.pop().unwrap(); st.push(ir_cmp(a, b, 0)?); pc += 1; }
            IrOp::LtEq => { let b = st.pop().unwrap(); let a = st.pop().unwrap(); st.push(ir_cmp(a, b, 1)?); pc += 1; }
            IrOp::Gt => { let b = st.pop().unwrap(); let a = st.pop().unwrap(); st.push(ir_cmp(a, b, 2)?); pc += 1; }
            IrOp::GtEq => { let b = st.pop().unwrap(); let a = st.pop().unwrap(); st.push(ir_cmp(a, b, 3)?); pc += 1; }
            IrOp::Eq => { let b = st.pop().unwrap(); let a = st.pop().unwrap(); st.push(ir_cmp(a, b, 4)?); pc += 1; }
            IrOp::NtEq => { let b = st.pop().unwrap(); let a = st.pop().unwrap(); st.push(ir_cmp(a, b, 5)?); pc += 1; }
            IrOp::JmpIfZero(t) => { let c = st.pop().unwrap(); if ir_is_zero(&c)? { pc = *t; } else { pc += 1; } }
            IrOp::Jmp(t) => { pc = *t; }
            IrOp::Pop(n) => { for _ in 0..*n { st.pop(); } pc += 1; }
            IrOp::Ret => {
                let rv = st.pop().map(|v| v.into_ref()).unwrap_or(DatumRef::Void);
                reserve_player_mut(|player| {
                    player.scopes.get_mut(scope_ref).unwrap().return_value = rv;
                });
                return Ok(());
            }
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::player::symbols::symbol_table::init_symbol_table;
    use crate::player::testing::{run_test, TestPlayer};

    fn ret_int(scope_ref: ScopeRef) -> i32 {
        reserve_player_ref(|player| {
            let rv = player.scopes.get(scope_ref).unwrap().return_value.clone();
            player.get_datum(&rv).int_value().unwrap()
        })
    }

    #[test]
    fn run_handler_param_plus_one() {
        init_symbol_table();
        run_test(async {
            let _p = TestPlayer::new();
            let scope_ref = reserve_player_mut(|player| {
                let s = player.push_scope();
                let arg = player.alloc_datum(Datum::Int(5));
                player.scopes.get_mut(s).unwrap().args.push(arg);
                s
            });
            // return param(0) + 1
            let compiled = CompiledHandler {
                ops: vec![IrOp::GetParam(0), IrOp::PushInt(1), IrOp::Add, IrOp::Ret],
                n_locals: 0,
            };
            run_handler(&compiled, scope_ref).unwrap();
            assert_eq!(ret_int(scope_ref), 6);
            reserve_player_mut(|player| player.pop_scope());
        });
    }

    #[test]
    fn run_handler_counted_loop_sum() {
        init_symbol_table();
        run_test(async {
            let _p = TestPlayer::new();
            let scope_ref = reserve_player_mut(|player| player.push_scope());
            // sum=0; repeat with j=1 to 10 { sum = sum + j }; return sum  => 55
            // locals: 0=sum, 1=j
            let ops = vec![
                IrOp::PushInt(0), IrOp::SetLocal(0),       // 0,1  sum = 0
                IrOp::PushInt(1), IrOp::SetLocal(1),       // 2,3  j = 1
                IrOp::GetLocal(1), IrOp::PushInt(10), IrOp::LtEq, IrOp::JmpIfZero(17), // 4-7 cond -> exit at 17
                IrOp::GetLocal(0), IrOp::GetLocal(1), IrOp::Add, IrOp::SetLocal(0),    // 8-11 sum+=j
                IrOp::GetLocal(1), IrOp::PushInt(1), IrOp::Add, IrOp::SetLocal(1),     // 12-15 j+=1
                IrOp::Jmp(4),                              // 16  loop
                IrOp::GetLocal(0), IrOp::Ret,              // 17,18  return sum
            ];
            let compiled = CompiledHandler { ops, n_locals: 2 };
            run_handler(&compiled, scope_ref).unwrap();
            assert_eq!(ret_int(scope_ref), 55);
            reserve_player_mut(|player| player.pop_scope());
        });
    }
}
