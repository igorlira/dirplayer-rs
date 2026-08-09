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
    /// Run this bytecode through the interpreter and come back.
    ///
    /// Because `compile` emits exactly ONE `IrOp` per bytecode, the IR pc IS the
    /// bytecode index — so the escape carries no index, and resuming is just
    /// reading `scope.bytecode_index` back after the interpreter advanced it.
    /// That invariant also keeps `bytecode_index_map` valid as an IR jump table.
    ///
    /// The IR and the interpreter share ONE local file (`scope.locals`), so an
    /// escape hands over no state and needs no sync — it just returns.
    Escape,
}

pub struct CompiledHandler {
    pub ops: Vec<IrOp>,
    pub n_locals: usize,
}

/// How a run of the IR ended.
pub enum IrExit {
    /// `Ret`, or the op array ran out.
    Done,
    /// An `Escape`: `scope.bytecode_index` is set to the op the interpreter must
    /// run. The driver executes it, advances the index as usual, then re-enters.
    Escape,
    /// A backward jump the DRIVER must see. `scope.bytecode_index` is the loop
    /// target; the driver runs its `HandlerExecutionResult::Jump` handling and
    /// re-enters.
    ///
    /// Without this the IR would swallow loops whole and never hand control
    /// back, and the driver's cooperative yield — the thing that lets a
    /// `repeat while keyPressed(" ")` ever see the key-up — would never fire,
    /// hanging the tab. The runaway-loop watchdog counts these too.
    BackJump,
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
            // Anything else runs through the interpreter in place. The hot
            // arithmetic/branch/local cluster around it still gets the IR's
            // tight loop, which is the whole point — a handler is no longer
            // rejected outright because it contains one `getchunk` or `put`.
            _ => IrOp::Escape,
        };
        ops.push(op);
    }
    debug_assert_eq!(
        ops.len(),
        bc.len(),
        "IR must stay 1:1 with bytecode — pc doubles as the bytecode index and \
         bytecode_index_map is reused as the jump table"
    );

    Some(CompiledHandler {
        ops,
        n_locals: handler.local_name_ids.len(),
    })
}

/// Largest share of escaped ops (percent) a handler may have and still be
/// worth compiling.
///
/// An escape is no longer free, just cheap: the driver re-enters
/// `run_handler_resumable` ONCE PER ESCAPE, and each re-entry pays a
/// `reserve_player_mut`, an `ensure_locals` check and loop setup. A handler
/// that is nothing but escapes therefore runs the IR loop only to bounce
/// straight back out, which is strictly slower than interpreting it.
///
/// It used to be 50% (`escapes * 2 < ops.len()`), chosen when an escape ALSO
/// dragged the whole local file into and out of a hash map. With that gone the
/// break-even sits much higher: the IR saves the driver's whole per-op path
/// (debugger check, generation read, opcode decode, the two dispatch matches,
/// result match) on every natively-executed op, and costs only re-entry on
/// each escape.
///
/// This is deliberately a tunable constant rather than a folded-in `* 2`,
/// because the right value is an empirical question — raise it, then check
/// `interpreted ops` in the E2E_INTERP_STATS report. That total counts only
/// ops reaching the interpreter, so it falls as the IR takes over, and it is
/// the direct measure of whether a change to this number helped.
const MAX_ESCAPE_PERCENT: usize = 65;

/// Is compiling this handler likely to pay for itself? Requires a real op
/// stream, and escapes below `MAX_ESCAPE_PERCENT` of it.
pub fn is_worth_compiling(c: &CompiledHandler) -> bool {
    if c.ops.len() < 8 {
        return false;
    }
    let escapes = c.ops.iter().filter(|o| matches!(o, IrOp::Escape)).count();
    escapes * 100 < c.ops.len() * MAX_ESCAPE_PERCENT
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
            IrOp::Escape { .. } => unreachable!("the pure-int bench runner compiles no escapes"),
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
    match run_handler_resumable(compiled, scope_ref)? {
        IrExit::Done => Ok(()),
        IrExit::Escape | IrExit::BackJump => Err(ScriptError::new(
            "run_handler: handler escaped; use run_handler_resumable".to_string(),
        )),
    }
}

/// Hand a backward jump back to the driver when it needs to see it: an
/// input-polling iteration (so the cooperative yield can run) or every 4096
/// iterations (so the runaway watchdog still counts). Returns `None` to stay in
/// the IR, which is the overwhelmingly common case for compute loops.
#[inline]
fn back_jump(
    scope_ptr: *mut crate::player::scope::Scope,
    target: usize,
    backjumps: &mut u32,
) -> Option<IrExit> {
    *backjumps = backjumps.wrapping_add(1);
    let polled = reserve_player_ref(|player| player.input_polled);
    if polled || (*backjumps & 0xFFF) == 0 {
        unsafe { (*scope_ptr).bytecode_index = target };
        return Some(IrExit::BackJump);
    }
    None
}

/// The IR loop, entered at `scope.bytecode_index` and returning how it stopped.
///
/// Operands live on the REAL `scope.stack` rather than a runner-owned Vec. That
/// is what makes an escape free: the interpreter op it hands off to reads and
/// writes the very same stack, so nothing has to be copied across. The measured
/// justification is that `OperandStack` push/pop benches at 0.5 ns/op — the same
/// as a plain Vec — so sharing it costs the IR nothing.
///
/// `locals` is the caller-owned dense file; it survives across escapes so a
/// resumed run continues with the same values.
pub fn run_handler_resumable(
    compiled: &CompiledHandler,
    scope_ref: ScopeRef,
) -> Result<IrExit, ScriptError> {
    // `scopes` is a fixed, pre-filled pool that never reallocates (see
    // `push_scope`), so this slot address is stable for the run.
    let scope_ptr: *mut crate::player::scope::Scope =
        reserve_player_mut(|player| &mut player.scopes[scope_ref] as *mut _);
    // Locals live in the scope, shared with the interpreter. Sized once here so
    // the op loop can index without a bounds-grow on every write.
    unsafe { (*scope_ptr).ensure_locals(compiled.n_locals) };
    // Reads and writes go through the same raw pointer the operand stack uses.
    // Nothing borrows across an escape: the Escape arm RETURNS, so the
    // interpreter op that follows has exclusive access.
    macro_rules! lc_get {
        ($s:expr) => {
            unsafe { (*scope_ptr).local($s as usize) }
        };
    }
    macro_rules! lc_set {
        ($s:expr, $v:expr) => {
            unsafe { (*scope_ptr).set_local($s as usize, $v) }
        };
    }
    macro_rules! st_push {
        ($v:expr) => {
            unsafe { (*scope_ptr).stack.push_value($v) }
        };
    }
    macro_rules! st_pop {
        () => {
            unsafe { (*scope_ptr).stack.pop_value() }.unwrap_or(StackDatum::Void)
        };
    }

    let ops = &compiled.ops;
    let mut pc = unsafe { (*scope_ptr).bytecode_index };
    let mut backjumps: u32 = 0;
    loop {
        if pc >= ops.len() {
            unsafe { (*scope_ptr).bytecode_index = pc };
            return Ok(IrExit::Done);
        }
        match &ops[pc] {
            IrOp::Escape => {
                // Hand the pc to the interpreter as a bytecode index (they are
                // the same number by construction) and let the driver advance
                // it. No locals handover: both sides read the same storage.
                unsafe { (*scope_ptr).bytecode_index = pc };
                return Ok(IrExit::Escape);
            }
            _ => {}
        }
        match &ops[pc] {
            IrOp::PushInt(n) => { st_push!(StackDatum::Int(*n)); pc += 1; }
            IrOp::GetLocal(s) => { st_push!(lc_get!(*s)); pc += 1; }
            IrOp::SetLocal(s) => { let v = cow_on_assign(st_pop!()); lc_set!(*s, v); pc += 1; }
            IrOp::GetParam(s) => {
                let s = *s as usize;
                let dr = reserve_player_ref(|player| {
                    player.scopes.get(scope_ref).unwrap().args.get(s).cloned().unwrap_or(DatumRef::Void)
                });
                st_push!(StackDatum::Ref(dr));
                pc += 1;
            }
            IrOp::Add => { let b = st_pop!(); let a = st_pop!(); st_push!(ir_add(a, b)?); pc += 1; }
            IrOp::Sub => { let b = st_pop!(); let a = st_pop!(); st_push!(ir_sub(a, b)?); pc += 1; }
            IrOp::Mul => { let b = st_pop!(); let a = st_pop!(); st_push!(ir_mul(a, b)?); pc += 1; }
            IrOp::Lt => { let b = st_pop!(); let a = st_pop!(); st_push!(ir_cmp(a, b, 0)?); pc += 1; }
            IrOp::LtEq => { let b = st_pop!(); let a = st_pop!(); st_push!(ir_cmp(a, b, 1)?); pc += 1; }
            IrOp::Gt => { let b = st_pop!(); let a = st_pop!(); st_push!(ir_cmp(a, b, 2)?); pc += 1; }
            IrOp::GtEq => { let b = st_pop!(); let a = st_pop!(); st_push!(ir_cmp(a, b, 3)?); pc += 1; }
            IrOp::Eq => { let b = st_pop!(); let a = st_pop!(); st_push!(ir_cmp(a, b, 4)?); pc += 1; }
            IrOp::NtEq => { let b = st_pop!(); let a = st_pop!(); st_push!(ir_cmp(a, b, 5)?); pc += 1; }
            IrOp::JmpIfZero(t) => {
                let c = st_pop!();
                if ir_is_zero(&c)? {
                    let t = *t;
                    if t <= pc { if let Some(e) = back_jump(scope_ptr, t, &mut backjumps) { return Ok(e); } }
                    pc = t;
                } else { pc += 1; }
            }
            IrOp::Jmp(t) => {
                let t = *t;
                if t <= pc { if let Some(e) = back_jump(scope_ptr, t, &mut backjumps) { return Ok(e); } }
                pc = t;
            }
            IrOp::Pop(n) => { for _ in 0..*n { let _ = st_pop!(); } pc += 1; }
            IrOp::Escape => unreachable!("handled before this match"),
            IrOp::Ret => {
                // EXACTLY `FlowControlBytecodeHandler::ret`: VOID, and clear the
                // stack. A handler's real return value comes from the `return`
                // BUILTIN (`ExtCall("return")`), which writes `return_value` and
                // stops the handler — so the `Ret` opcode is only ever reached by
                // falling off the end, which yields VOID in Director.
                //
                // The PoC runner popped the stack top as the return value. Once
                // the IR was actually wired into dispatch that handed callers a
                // leftover operand instead of VOID, sending movies down the wrong
                // branch (nintendo/rollcall rendered the instructions screen where
                // the title belonged). Not clearing the stack also leaked operands
                // into the pooled scope for its next use.
                let scope = unsafe { &mut *scope_ptr };
                scope.return_value = DatumRef::Void;
                scope.stack.clear();
                scope.bytecode_index = pc;
                return Ok(IrExit::Done);
            }
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::player::symbols::symbol_table::init_symbol_table;
    use crate::player::testing::{run_test, TestPlayer};

    /// Top of the scope's operand stack, as an int. The IR shares the scope
    /// stack with the interpreter, so a handler's computed value is read from
    /// there — `Ret` itself yields VOID, exactly as the interpreter's does.
    fn stack_top_int(scope_ref: ScopeRef) -> i32 {
        reserve_player_ref(|player| {
            let scope = player.scopes.get(scope_ref).unwrap();
            let top = scope.stack.iter().last().cloned().unwrap_or(DatumRef::Void);
            player.get_datum(&top).int_value().unwrap()
        })
    }

    #[allow(dead_code)]
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
                ops: vec![IrOp::GetParam(0), IrOp::PushInt(1), IrOp::Add],
                n_locals: 0,
            };
            run_handler(&compiled, scope_ref).unwrap();
            assert_eq!(stack_top_int(scope_ref), 6);
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
                IrOp::GetLocal(0),                         // 17  leave sum on the stack
            ];
            let compiled = CompiledHandler { ops, n_locals: 2 };
            run_handler(&compiled, scope_ref).unwrap();
            assert_eq!(stack_top_int(scope_ref), 55);
            reserve_player_mut(|player| player.pop_scope());
        });
    }

    /// An unsupported opcode must become an `Escape` that stops the IR at the
    /// RIGHT bytecode index, and a resumed run must continue with the dense
    /// local file intact. This is the whole premise of Stage 3: a handler is no
    /// longer rejected because it contains one interpreter-only op.
    #[test]
    fn compile_escapes_unsupported_opcode_and_resumes() {
        use crate::director::chunks::handler::{Bytecode, HandlerDef};
        init_symbol_table();
        run_test(async {
            let _player = TestPlayer::new();

            // local0 = 5 ; <getchunk: interpreter-only> ; return local0
            let handler = HandlerDef {
                name_id: 0,
                bytecode_array: vec![
                    Bytecode::new(OpCode::PushInt8, 5, 0),
                    Bytecode::new(OpCode::SetLocal, 0, 1),
                    Bytecode::new(OpCode::GetChunk, 0, 2),
                    // No trailing Ret: `Ret` yields VOID and clears the stack
                    // (Director semantics), so the computed value is asserted
                    // from the stack, which the IR shares with the interpreter.
                    Bytecode::new(OpCode::GetLocal, 0, 3),
                ],
                bytecode_index_map: fxhash::FxHashMap::default(),
                argument_name_ids: vec![],
                local_name_ids: vec![0],
                global_name_ids: vec![],
                compiled_ir: std::cell::RefCell::new(None),
            };
            let compiled = compile(&handler, 1).expect("escapes, never rejects");
            assert_eq!(compiled.ops.len(), handler.bytecode_array.len(), "IR must be 1:1");
            assert!(matches!(compiled.ops[2], IrOp::Escape));

            let scope_ref = reserve_player_mut(|player| player.push_scope());

            // First run stops AT the escaped op (pc == its bytecode index).
            match run_handler_resumable(&compiled, scope_ref).unwrap() {
                IrExit::Escape => {}
                other => panic!("expected an escape, got {}", match other {
                    IrExit::Done => "Done", IrExit::BackJump => "BackJump", _ => "?" }),
            }

            // The local written before the escape is visible in the SCOPE, not
            // in any IR-private copy — that is what makes the sync unnecessary.
            assert!(
                matches!(
                    reserve_player_ref(|p| p.scopes.get(scope_ref).unwrap().local(0)),
                    StackDatum::Int(5)
                ),
                "the IR must write locals straight into the scope"
            );
            assert!(
                reserve_player_ref(|p| p.scopes.get(scope_ref).unwrap().local_is_assigned(0)),
                "an IR write must mark the slot assigned, or do/eval resolution breaks"
            );
            assert_eq!(
                reserve_player_ref(|p| p.scopes.get(scope_ref).unwrap().bytecode_index),
                2,
                "escape must leave bytecode_index on the op the interpreter runs"
            );

            // The driver would run that op and advance; do just the advance.
            reserve_player_mut(|player| {
                player.scopes.get_mut(scope_ref).unwrap().bytecode_index = 3;
            });

            // Resuming continues with locals intact and returns 5.
            match run_handler_resumable(&compiled, scope_ref).unwrap() {
                IrExit::Done => {}
                IrExit::Escape => panic!("expected completion, got Escape"),
                IrExit::BackJump => panic!("expected completion, got BackJump"),
            }
            assert_eq!(stack_top_int(scope_ref), 5, "dense locals must survive the escape");
            reserve_player_mut(|player| player.pop_scope());
        });
    }
}
