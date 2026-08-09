use fxhash::FxHashMap;

use crate::director::lingo::datum::Datum;

use super::{
    cast_lib::{CastMemberRef, INVALID_CAST_MEMBER_REF},
    symbols::symbol::Symbol,
    script_ref::ScriptInstanceRef,
    DatumRef, PLAYER_OPT,
};

pub type ScopeRef = usize;

/// A value on the Lingo operand stack.
///
/// Primitive values (int/float/symbol/void) are stored INLINE, so pushing them
/// constructs neither a 64-byte `Datum` nor a `DatumRef` and never touches the
/// arena. A value is materialized into a real `DatumRef` only when something
/// actually needs one (popped/peeked by a consumer that works on `DatumRef`),
/// using the pooled fast paths (`alloc_int`/`alloc_symbol`) which return cached
/// immortal refs. Consumers that understand inline values (arithmetic, compare)
/// can read the primitive directly and skip materialization entirely.
#[derive(Clone)]
pub enum StackDatum {
    Int(i32),
    Float(f64),
    Symbol(Symbol),
    Void,
    Ref(DatumRef),
    /// A call's argument marker: its `count` arguments sit directly beneath it
    /// on the stack, and `no_ret` records whether the call discards its result.
    ///
    /// `pusharglist` used to pop the arguments and allocate a
    /// `Datum::List(ArgList, ..)` to hold them — a `VecDeque` heap allocation
    /// plus an arena allocation — purely so the very next opcode could
    /// destructure it again. Leaving the arguments in place and pushing this
    /// instead removes both. Nesting is unaffected: the marker occupies the same
    /// stack position the list did, so `foo(a, bar(b))` resolves identically.
    ArgMarker { count: u16, no_ret: bool },
}

impl StackDatum {
    /// Materialize this value into a `DatumRef` (pooled fast path for
    /// int/symbol). Requires the global player to be initialized.
    #[inline]
    pub fn into_ref(self) -> DatumRef {
        match self {
            StackDatum::Ref(dr) => dr,
            StackDatum::Void => DatumRef::Void,
            // Allocate in the ACTIVE player's arena, not PLAYER_OPT's. A nested
            // #movie runs with ACTIVE_PLAYER_ID != 0; allocating its stack values
            // in the HOST arena while DatumRef::drop frees them against the active
            // one corrupts both. Same de-globalisation gap the nested datum-leak
            // fix closed on the free side.
            StackDatum::Int(n) => {
                let player = unsafe { crate::player::player_mut() };
                player.allocator.alloc_int(n)
            }
            StackDatum::Symbol(s) => {
                let player = unsafe { crate::player::player_mut() };
                player.allocator.alloc_symbol(s)
            }
            StackDatum::Float(f) => {
                let player = unsafe { crate::player::player_mut() };
                player.alloc_datum(Datum::Float(f))
            }
            // A marker is consumed by the call opcode that follows it and is
            // never a value. Degrade to Void rather than panic if some generic
            // stack reader (debugger display, error unwinding) reaches one.
            StackDatum::ArgMarker { .. } => DatumRef::Void,
        }
    }
}

/// The Lingo operand stack. Stores `StackDatum` (inline primitives or refs) in
/// `UnsafeCell`s so inline entries can be materialized to a `DatumRef` lazily
/// in place even behind a shared `&` (sound: the stack is only reached through
/// the globally-mutable `PLAYER_OPT`, same pattern as the arena's ref-counts).
/// It presents the same `DatumRef`-based API the interpreter already used, so
/// the hundreds of existing push/pop/len/last/index call sites are unchanged.
#[derive(Default)]
pub struct OperandStack {
    items: Vec<std::cell::UnsafeCell<StackDatum>>,
}

impl Clone for OperandStack {
    fn clone(&self) -> Self {
        OperandStack {
            items: self
                .items
                .iter()
                .map(|c| std::cell::UnsafeCell::new(unsafe { (*c.get()).clone() }))
                .collect(),
        }
    }
}

impl OperandStack {
    #[inline]
    pub fn new() -> Self {
        OperandStack { items: Vec::new() }
    }

    // --- DatumRef-facing API (unchanged for existing call sites) ---
    #[inline]
    pub fn push(&mut self, dr: DatumRef) {
        self.items.push(std::cell::UnsafeCell::new(StackDatum::Ref(dr)));
    }
    #[inline]
    pub fn pop(&mut self) -> Option<DatumRef> {
        self.items.pop().map(|c| c.into_inner().into_ref())
    }
    /// Pop the top entry as a raw `StackDatum` (inline value or ref) WITHOUT
    /// materializing. Inline-aware consumers (arithmetic, compare, jmpifz) use
    /// this so an inline int/float never round-trips through the arena.
    #[inline]
    pub fn pop_value(&mut self) -> Option<StackDatum> {
        self.items.pop().map(|c| c.into_inner())
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.items.len()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    #[inline]
    pub fn clear(&mut self) {
        self.items.clear();
    }
    #[inline]
    pub fn truncate(&mut self, n: usize) {
        self.items.truncate(n);
    }
    #[inline]
    pub fn swap(&mut self, a: usize, b: usize) {
        self.items.swap(a, b);
    }
    #[inline]
    pub fn last(&self) -> Option<&DatumRef> {
        if self.items.is_empty() {
            return None;
        }
        Some(self.ensure_ref(self.items.len() - 1))
    }
    #[inline]
    pub fn last_mut(&mut self) -> Option<&mut DatumRef> {
        if self.items.is_empty() {
            return None;
        }
        let i = self.items.len() - 1;
        self.ensure_ref(i);
        match self.items[i].get_mut() {
            StackDatum::Ref(dr) => Some(dr),
            _ => unreachable!("ensure_ref guarantees Ref"),
        }
    }
    #[inline]
    pub fn get(&self, i: usize) -> Option<&DatumRef> {
        if i >= self.items.len() {
            return None;
        }
        Some(self.ensure_ref(i))
    }
    /// Discard the top `n` entries without materializing them. Dropping a
    /// `Ref` entry decrements its arena refcount exactly as moving it out and
    /// dropping the `DatumRef` would, but inline primitives (the common case
    /// for a discarded expression result) are dropped for free — no `alloc_int`
    /// round-trip just to throw the value away. Used by the `Pop` opcode.
    #[inline]
    pub fn discard(&mut self, n: usize) {
        let new_len = self.items.len().saturating_sub(n);
        self.items.truncate(new_len);
    }
    /// Move the top `n` entries out as owned `DatumRef`s (used by pop_n).
    #[inline]
    pub fn split_off_refs(&mut self, at: usize) -> Vec<DatumRef> {
        self.items
            .split_off(at)
            .into_iter()
            .map(|c| c.into_inner().into_ref())
            .collect()
    }
    /// Drain the top `n` entries directly into `buf` (materializing inline values),
    /// removing them from the stack in place. Unlike `split_off_refs`, `drain` does
    /// NOT allocate a transient `Vec` for the removed tail — the arg list is built in
    /// one pass straight into the deque the call opcode consumes. `push_arglist` runs
    /// once per Lingo call (8.1M times in the Habbo preloader), so dropping that extra
    /// per-call allocation matters. `buf` is a (typically pooled) deque, cleared first.
    #[inline]
    pub fn drain_top_into_deque(
        &mut self,
        n: usize,
        mut buf: std::collections::VecDeque<DatumRef>,
    ) -> std::collections::VecDeque<DatumRef> {
        let at = self.items.len() - n;
        buf.clear();
        buf.reserve(n);
        for c in self.items.drain(at..) {
            buf.push_back(c.into_inner().into_ref());
        }
        buf
    }
    /// Iterate the stack as `&DatumRef` (bottom to top). Materializes inline
    /// entries in place first.
    pub fn iter(&self) -> impl Iterator<Item = &DatumRef> {
        (0..self.items.len()).map(move |i| self.ensure_ref(i))
    }

    // --- Inline push fast paths (no Datum/arena) ---
    /// Push an already-formed `StackDatum` (inline value or ref) without
    /// materializing it. The IR runner operates in `StackDatum` terms and shares
    /// this stack with the interpreter, so it needs the untyped push.
    #[inline]
    pub fn push_value(&mut self, v: StackDatum) {
        self.items.push(std::cell::UnsafeCell::new(v));
    }
    #[inline]
    pub fn push_int(&mut self, n: i32) {
        self.items.push(std::cell::UnsafeCell::new(StackDatum::Int(n)));
    }
    #[inline]
    pub fn push_float(&mut self, f: f64) {
        self.items.push(std::cell::UnsafeCell::new(StackDatum::Float(f)));
    }
    #[inline]
    pub fn push_symbol(&mut self, s: Symbol) {
        self.items.push(std::cell::UnsafeCell::new(StackDatum::Symbol(s)));
    }
    #[inline]
    pub fn push_void(&mut self) {
        self.items.push(std::cell::UnsafeCell::new(StackDatum::Void));
    }

    /// Materialize the inline entry at `i` into a `Ref` in place and return it.
    /// The `UnsafeCell` makes the in-place mutation through `&self` sound.
    #[inline]
    fn ensure_ref(&self, i: usize) -> &DatumRef {
        let cell = &self.items[i];
        unsafe {
            let sd = &mut *cell.get();
            if !matches!(sd, StackDatum::Ref(_)) {
                let dr = std::mem::replace(sd, StackDatum::Void).into_ref();
                *sd = StackDatum::Ref(dr);
            }
            match &*cell.get() {
                StackDatum::Ref(dr) => dr,
                _ => unreachable!(),
            }
        }
    }
}

impl std::ops::Index<usize> for OperandStack {
    type Output = DatumRef;
    #[inline]
    fn index(&self, i: usize) -> &DatumRef {
        self.ensure_ref(i)
    }
}

// #[derive(Clone)]
pub struct Scope {
    pub scope_ref: ScopeRef,
    pub script_ref: CastMemberRef,
    pub receiver: Option<ScriptInstanceRef>,
    pub handler_name_id: u16,
    pub args: Vec<DatumRef>,
    pub bytecode_index: usize,
    /// Handler locals, indexed by DENSE SLOT — the same index the bytecode
    /// carries (`bytecode.obj / multiplier`) and the same one the register IR
    /// uses. This was an `FxHashMap<u16, DatumRef>` keyed by the SPARSE
    /// name-table id, which every call site reached by computing the slot and
    /// then mapping it through `handler.local_name_ids` purely to build a hash
    /// key; indexing by slot removes that step rather than adding one.
    pub locals: Vec<StackDatum>,
    /// Whether each slot has ever been assigned in this invocation.
    ///
    /// Load-bearing for `do`/`eval` only. The old hash map encoded "this frame
    /// has no such local" as key ABSENCE, and `eval.rs`'s resolver relies on
    /// that to fall through to `me` and then globals. A dense vector has every
    /// declared slot present from the start, so without this a declared but
    /// never-assigned local would start shadowing a same-named global —
    /// silently. Written wherever a local is written (strictly cheaper than
    /// the hash insert it replaces) and read only by that resolver.
    pub locals_assigned: Vec<bool>,
    pub loop_return_indices: Vec<usize>,
    pub return_value: DatumRef,
    pub stack: OperandStack,
    pub passed: bool,
    /// Set by the `pass` command: "The pass command branches to the next
    /// location as soon as the command runs. Any Lingo that follows the pass
    /// command in the handler does not run." (Director 11.5 Scripting
    /// Dictionary, `pass`). The bytecode loop ends the handler when it sees
    /// this, which `passed` alone must not do — `passed` is also propagated up
    /// from a nested call to drive event propagation.
    pub stop_requested: bool,
    pub generation: u64,
    /// Cached handler-level instance for get_prop/set_prop (avoids ancestor chain walk per access)
    pub cached_handler_instance: Option<ScriptInstanceRef>,
}

pub struct ScopeResult {
    pub return_value: DatumRef,
    pub passed: bool,
}

impl Scope {
    /// Pop a call's `ArgMarker` and the arguments beneath it, in stack order.
    /// Returns `(args, no_ret)`.
    ///
    /// `None` means the top of stack was not a marker, which would mean the
    /// bytecode ran a call opcode without a preceding `pusharglist`. Callers
    /// report that as a stack error rather than guessing.
    pub fn pop_call_args(&mut self) -> Option<(Vec<DatumRef>, bool)> {
        let (count, no_ret) = match self.stack.pop_value()? {
            StackDatum::ArgMarker { count, no_ret } => (count as usize, no_ret),
            // Not a marker: put nothing back — the caller errors out. Restoring
            // it would need a push and the frame is being torn down anyway.
            _ => return None,
        };
        if self.stack.len() < count {
            return None;
        }
        Some((self.pop_n(count), no_ret))
    }

    pub fn pop_n(&mut self, n: usize) -> Vec<DatumRef> {
        // Move the top `n` entries out of the stack rather than clone-then-pop.
        // `split_off` transfers ownership of the tail with zero ref-count churn,
        // where the old `to_vec()` + pop loop did 2n ref-count ops plus an extra
        // allocation. `pusharglist`/`pusharglistnoret` (the heaviest opcodes in
        // the Habbo preloader) call this on every Lingo call.
        let split_at = self.stack.len() - n;
        self.stack.split_off_refs(split_at)
    }

    pub fn default(scope_ref: ScopeRef) -> Scope {
        Scope {
            scope_ref,
            script_ref: INVALID_CAST_MEMBER_REF,
            receiver: None,
            handler_name_id: 0,
            args: vec![],
            bytecode_index: 0,
            locals: Vec::new(),
            locals_assigned: Vec::new(),
            loop_return_indices: vec![],
            return_value: DatumRef::Void,
            stack: OperandStack::new(),
            passed: false,
            stop_requested: false,
            generation: 0,
            cached_handler_instance: None,
        }
    }

    /// Size the local file for a handler. Called once per handler entry; the
    /// scope pool keeps its capacity across reuse, so after warmup this is a
    /// memset rather than an allocation.
    pub fn ensure_locals(&mut self, n_locals: usize) {
        if self.locals.len() < n_locals {
            self.locals.resize(n_locals, StackDatum::Void);
            self.locals_assigned.resize(n_locals, false);
        }
    }

    /// Read a local by slot. Out-of-range reads VOID rather than panicking:
    /// the slot comes from bytecode, and a malformed handler must not be able
    /// to take the player down.
    #[inline]
    pub fn local(&self, slot: usize) -> StackDatum {
        self.locals.get(slot).cloned().unwrap_or(StackDatum::Void)
    }

    /// Write a local by slot, growing the file if the bytecode names a slot
    /// beyond the handler's declared count.
    #[inline]
    pub fn set_local(&mut self, slot: usize, value: StackDatum) {
        if slot >= self.locals.len() {
            self.locals.resize(slot + 1, StackDatum::Void);
            self.locals_assigned.resize(slot + 1, false);
        }
        self.locals[slot] = value;
        self.locals_assigned[slot] = true;
    }

    /// Has this slot ever been assigned in this invocation? Only `do`/`eval`
    /// name resolution needs this — see `locals_assigned`.
    #[inline]
    pub fn local_is_assigned(&self, slot: usize) -> bool {
        self.locals_assigned.get(slot).copied().unwrap_or(false)
    }

    pub fn reset(&mut self) {
        // Bump the generation so the trampoline's stale-scope guard
        // (`post_gen != scope_generation`) trips for any handler still
        // suspended on this slot. The movie-change transition resets every
        // scope while a handler from the old movie can be parked across the
        // `go to movie` await; without this bump that handler would resume
        // against a reset (sentinel `script_ref`) scope and run opcodes like
        // `set homeScore` on a non-existent script (-1:-1). `push_scope`
        // overwrites the generation explicitly right after calling reset(),
        // so this is harmless on the allocation path.
        self.generation = self.generation.wrapping_add(1);
        self.script_ref = INVALID_CAST_MEMBER_REF;
        self.receiver = None;
        self.cached_handler_instance = None;
        self.handler_name_id = 0;
        self.args.clear();
        self.bytecode_index = 0;
        self.locals.clear();
        self.locals_assigned.clear();
        self.loop_return_indices.clear();
        self.return_value = DatumRef::Void;
        self.stack.clear();
        self.passed = false;
        self.stop_requested = false;
    }
}
