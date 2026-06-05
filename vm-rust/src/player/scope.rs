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
}

impl StackDatum {
    /// Materialize this value into a `DatumRef` (pooled fast path for
    /// int/symbol). Requires the global player to be initialized.
    #[inline]
    pub fn into_ref(self) -> DatumRef {
        match self {
            StackDatum::Ref(dr) => dr,
            StackDatum::Void => DatumRef::Void,
            StackDatum::Int(n) => {
                let player = unsafe { PLAYER_OPT.as_mut().unwrap() };
                player.allocator.alloc_int(n)
            }
            StackDatum::Symbol(s) => {
                let player = unsafe { PLAYER_OPT.as_mut().unwrap() };
                player.allocator.alloc_symbol(s)
            }
            StackDatum::Float(f) => {
                let player = unsafe { PLAYER_OPT.as_mut().unwrap() };
                player.alloc_datum(Datum::Float(f))
            }
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
    pub locals: FxHashMap<u16, DatumRef>,
    pub loop_return_indices: Vec<usize>,
    pub return_value: DatumRef,
    pub stack: OperandStack,
    pub passed: bool,
    pub generation: u64,
    /// Cached handler-level instance for get_prop/set_prop (avoids ancestor chain walk per access)
    pub cached_handler_instance: Option<ScriptInstanceRef>,
}

pub struct ScopeResult {
    pub return_value: DatumRef,
    pub passed: bool,
}

impl Scope {
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
            locals: FxHashMap::default(),
            loop_return_indices: vec![],
            return_value: DatumRef::Void,
            stack: OperandStack::new(),
            passed: false,
            generation: 0,
            cached_handler_instance: None,
        }
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
        self.loop_return_indices.clear();
        self.return_value = DatumRef::Void;
        self.stack.clear();
        self.passed = false;
    }
}
