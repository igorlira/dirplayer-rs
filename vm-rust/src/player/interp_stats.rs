//! Interpreter instrumentation — counters that answer "what should we optimise
//! next?" with data instead of a guess.
//!
//! Every counter here exists to size ONE decision:
//!
//!   - Which opcodes does the interpreter actually execute? The register IR
//!     (`player::compiled`) natively handles ~20 of them and escapes the rest,
//!     so "which opcodes are worth teaching it" is a question about the tail of
//!     `INTERP_OPS`, not about which opcodes look expensive.
//!   - How much of that traffic is handlers the IR REJECTED outright
//!     (`is_worth_compiling`) versus escapes from a compiled handler? Those
//!     want different fixes.
//!   - What does `escape_needs_local_sync` actually cost? It defaults to TRUE
//!     and copies every local into `scope.locals` and back around each escape,
//!     so the cost is (events x locals), not events. These were hash
//!     insert/lookup pairs until `Scope.locals` became slot-indexed; they are
//!     plain slot copies now — same count, far cheaper each, which is why the
//!     report says "slots copied" rather than "hash ops".
//!   - How many args does a handler call carry? That sizes the per-call
//!     `Vec<DatumRef>` alloc + clone + drop in `setup_handler_frame`.
//!
//! Gated on a runtime flag, OFF by default: when disabled each hook is one
//! relaxed atomic load and a predictable branch. The e2e browser harness turns
//! it on for the whole suite, which is the point — 50 movies is a far better
//! opcode sample than any single title.
//!
//! Deliberately NOT a profiler: no timing, no call stacks. Timing under wasm
//! needs `performance.now()` per sample and would perturb the very loop being
//! measured. Counts are exact and nearly free; combine them with a DevTools
//! profile to get cost-per-op.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::director::lingo::opcode::OpCode;

/// Opcode discriminants are u8-ranged (see `OpCode`), so a flat 256-slot table
/// indexes without hashing.
const N_OPCODES: usize = 256;

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Opcodes executed by the INTERPRETER, by discriminant. Ops the IR runs
/// natively never reach here — that asymmetry is the measurement.
static INTERP_OPS: [AtomicU64; N_OPCODES] = [const { AtomicU64::new(0) }; N_OPCODES];
/// Subset of `INTERP_OPS` that escaped from a COMPILED handler, as opposed to
/// running in a handler the IR declined. Same index space.
static ESCAPED_OPS: [AtomicU64; N_OPCODES] = [const { AtomicU64::new(0) }; N_OPCODES];

macro_rules! counters {
    ($($name:ident),* $(,)?) => {
        $( static $name: AtomicU64 = AtomicU64::new(0); )*
    };
}

counters!(
    // Locals sync around IR escapes: events, and the number of local slots
    // copied. The second is the real cost — it is O(n_locals) each way.
    SYNC_OUT_EVENTS,
    SYNC_OUT_LOCALS,
    SYNC_IN_EVENTS,
    SYNC_IN_LOCALS,
    // Compile outcomes, counted once per handler (the result is cached on the
    // HandlerDef, so these are distinct handlers, not calls).
    HANDLERS_COMPILED,
    HANDLERS_REJECTED,
    // Handler invocations, split by whether the callee had usable IR. The
    // rejected-handler figure is CALL-weighted, which is what matters: one hot
    // rejected handler outranks a hundred cold ones.
    CALLS_WITH_IR,
    CALLS_WITHOUT_IR,
    // Argument traffic, to size the per-call Vec alloc + clone + drop.
    CALL_ARGS_TOTAL,
    // `do` / `eval` reaching this frame's LOCALS. These exist to answer a
    // coverage question, not a performance one: converting `Scope.locals` to a
    // dense slot-indexed Vec changes what "this local is absent" means, and the
    // only code that can observe the difference is the do/eval resolver. If
    // these are zero across the whole e2e suite the path has no coverage and
    // any behaviour change there would be silent.
    //
    // EVAL_LOCAL_HIT is the read in `eval.rs` that probes `locals` by name id
    // after a global-name-table search; CTXVAR_LOCAL_GET / _SET are the
    // `context_vars` 0x5 arms, which is how `do "x = 5"` writes a local.
    EVAL_LOCAL_HIT,
    CTXVAR_LOCAL_GET,
    CTXVAR_LOCAL_SET,
);

#[inline(always)]
pub fn record_eval_local_hit() {
    if enabled() {
        bump(&EVAL_LOCAL_HIT, 1);
    }
}

#[inline(always)]
pub fn record_ctxvar_local(is_write: bool) {
    if enabled() {
        bump(if is_write { &CTXVAR_LOCAL_SET } else { &CTXVAR_LOCAL_GET }, 1);
    }
}

#[inline(always)]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn reset() {
    for slot in INTERP_OPS.iter().chain(ESCAPED_OPS.iter()) {
        slot.store(0, Ordering::Relaxed);
    }
    for slot in [
        &SYNC_OUT_EVENTS, &SYNC_OUT_LOCALS, &SYNC_IN_EVENTS, &SYNC_IN_LOCALS,
        &HANDLERS_COMPILED, &HANDLERS_REJECTED,
        &CALLS_WITH_IR, &CALLS_WITHOUT_IR, &CALL_ARGS_TOTAL,
        &EVAL_LOCAL_HIT, &CTXVAR_LOCAL_GET, &CTXVAR_LOCAL_SET,
    ] {
        slot.store(0, Ordering::Relaxed);
    }
}

#[inline(always)]
fn bump(c: &AtomicU64, by: u64) {
    c.fetch_add(by, Ordering::Relaxed);
}

/// One opcode dispatched through the interpreter. `from_ir` distinguishes an
/// escape out of a compiled handler from an op in a handler with no IR.
#[inline(always)]
pub fn record_interp_op(opcode: OpCode, from_ir: bool) {
    if !enabled() {
        return;
    }
    let idx = opcode as usize & (N_OPCODES - 1);
    bump(&INTERP_OPS[idx], 1);
    if from_ir {
        bump(&ESCAPED_OPS[idx], 1);
    }
}

#[inline(always)]
pub fn record_sync_out(n_locals: usize) {
    if !enabled() {
        return;
    }
    bump(&SYNC_OUT_EVENTS, 1);
    bump(&SYNC_OUT_LOCALS, n_locals as u64);
}

#[inline(always)]
pub fn record_sync_in(n_locals: usize) {
    if !enabled() {
        return;
    }
    bump(&SYNC_IN_EVENTS, 1);
    bump(&SYNC_IN_LOCALS, n_locals as u64);
}

/// Outcome of the one-time compile attempt for a handler.
#[inline(always)]
pub fn record_compile_outcome(compiled: bool) {
    if !enabled() {
        return;
    }
    bump(if compiled { &HANDLERS_COMPILED } else { &HANDLERS_REJECTED }, 1);
}

#[inline(always)]
pub fn record_handler_call(has_ir: bool, n_args: usize) {
    if !enabled() {
        return;
    }
    bump(if has_ir { &CALLS_WITH_IR } else { &CALLS_WITHOUT_IR }, 1);
    bump(&CALL_ARGS_TOTAL, n_args as u64);
}

fn get(c: &AtomicU64) -> u64 {
    c.load(Ordering::Relaxed)
}

fn pct(part: u64, whole: u64) -> f64 {
    if whole == 0 { 0.0 } else { part as f64 * 100.0 / whole as f64 }
}

/// Human-readable report. Opcodes are listed most-executed first, with the
/// share that escaped from compiled handlers — the ones worth teaching the IR
/// are those high in this list AND high in `%esc`.
pub fn report() -> String {
    let mut rows: Vec<(usize, u64, u64)> = (0..N_OPCODES)
        .map(|i| (i, get(&INTERP_OPS[i]), get(&ESCAPED_OPS[i])))
        .filter(|(_, total, _)| *total > 0)
        .collect();
    rows.sort_by_key(|(_, total, _)| std::cmp::Reverse(*total));

    let total_ops: u64 = rows.iter().map(|(_, t, _)| *t).sum();
    let total_esc: u64 = rows.iter().map(|(_, _, e)| *e).sum();

    let mut out = String::new();
    out.push_str("=== interpreter stats ===\n\n");

    let calls_ir = get(&CALLS_WITH_IR);
    let calls_no_ir = get(&CALLS_WITHOUT_IR);
    let calls = calls_ir + calls_no_ir;
    let args = get(&CALL_ARGS_TOTAL);
    out.push_str(&format!(
        "handler calls      {calls}\n  with IR          {calls_ir} ({:.1}%)\n  \
         without IR       {calls_no_ir} ({:.1}%)\n  args total       {args} \
         (mean {:.2}/call)\n\n",
        pct(calls_ir, calls),
        pct(calls_no_ir, calls),
        if calls == 0 { 0.0 } else { args as f64 / calls as f64 },
    ));

    let hc = get(&HANDLERS_COMPILED);
    let hr = get(&HANDLERS_REJECTED);
    out.push_str(&format!(
        "handlers seen      {}\n  compiled         {hc} ({:.1}%)\n  rejected         {hr} ({:.1}%)\n\n",
        hc + hr,
        pct(hc, hc + hr),
        pct(hr, hc + hr),
    ));

    let so_e = get(&SYNC_OUT_EVENTS);
    let so_l = get(&SYNC_OUT_LOCALS);
    let si_e = get(&SYNC_IN_EVENTS);
    let si_l = get(&SYNC_IN_LOCALS);
    out.push_str(&format!(
        "locals sync\n  out              {so_e} events, {so_l} slots (mean {:.2})\n  \
         in               {si_e} events, {si_l} slots (mean {:.2})\n  \
         slots copied     {}\n\n",
        if so_e == 0 { 0.0 } else { so_l as f64 / so_e as f64 },
        if si_e == 0 { 0.0 } else { si_l as f64 / si_e as f64 },
        so_l + si_l,
    ));

    out.push_str(&format!(
        "interpreted ops    {total_ops}\n  from IR escape   {total_esc} ({:.1}%)\n  \
         uncompiled       {} ({:.1}%)\n\n",
        pct(total_esc, total_ops),
        total_ops - total_esc,
        pct(total_ops - total_esc, total_ops),
    ));

    out.push_str(&format!(
        "do/eval reaching locals\n  eval.rs read hit {}\n  context_vars get  {}\n  context_vars set  {}\n\n",
        get(&EVAL_LOCAL_HIT),
        get(&CTXVAR_LOCAL_GET),
        get(&CTXVAR_LOCAL_SET),
    ));

    out.push_str("opcode                       count      %all   esc%\n");
    out.push_str("------------------------------------------------------\n");
    for (idx, total, esc) in rows.iter().take(48) {
        // Via the map rather than `get_opcode_name`, which `.unwrap()`s and
        // would panic on an opcode that has no name entry. A diagnostic must
        // not be able to take the run down.
        let name = num::FromPrimitive::from_usize(*idx)
            .and_then(|op: OpCode| {
                crate::director::lingo::constants::opcode_names().get(&op).cloned()
            })
            .map(|n| n.to_string())
            .unwrap_or_else(|| format!("<0x{idx:02x}>"));
        out.push_str(&format!(
            "{name:<24} {total:>10}  {:>6.2}  {:>5.1}\n",
            pct(*total, total_ops),
            pct(*esc, *total),
        ));
    }
    out
}
