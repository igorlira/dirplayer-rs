// SpiderMonkey-1.5-style interpreter for js-lingo JsScriptIR.
//
// One dispatch loop, one operand stack, plus a scope chain for variable
// lookups. Lexical scoping is represented by parent links between scope
// objects; the innermost frame holds locals/args; the global object is at
// the root of every chain.
//
// Op handlers correspond 1:1 to entries in jsopcode.tbl. Coverage is
// incremental — anything we haven't implemented yet falls through to an
// `Unimplemented(op)` error rather than silent miscalculation.

use std::cell::RefCell;
use std::rc::Rc;

use super::opcodes::JsOp;
use super::value::{
    JsArray, JsArrayRef, JsError, JsFunction, JsFunctionRef, JsObject, JsObjectRef, JsValue,
    NativeFn,
};
use super::variable_length::{read_i16_operand, read_i32_operand, read_u16_operand, read_u32_operand};
use super::xdr::{
    iter_ops, JsAtom, JsBindingKind, JsFunctionAtom, JsFunctionBinding, JsScriptIR,
};

/// One frame on the interpreter call stack.
pub struct JsFrame {
    /// Bytecode being executed. Lives as long as the corresponding
    /// JsScriptIR (top-level program or nested function body).
    pub bytecode: Rc<Vec<u8>>,
    /// Atom map snapshot — needed for STRING/NUMBER/NAME/BINDNAME operand
    /// resolution. We hold an Rc so frames can outlive iteration over the
    /// atoms vector when CALL re-enters with a new IR.
    pub atoms: Rc<Vec<JsAtom>>,
    /// Argument values (slot index = `JsFunctionBinding.short_id` for
    /// Argument-kind bindings).
    pub args: Vec<JsValue>,
    /// Local-variable slots (slot index = short_id for Var/Const bindings).
    pub locals: Vec<JsValue>,
    /// The frame's lexical-scope object — holds `var` declarations made
    /// in this activation. For the global frame this is the program scope.
    pub scope: JsObjectRef,
    /// Outer scope chain (function's enclosing scope at definition time).
    pub parent_scope: Option<JsObjectRef>,
    /// `this` binding.
    pub this_value: JsValue,
    /// Current PC.
    pub pc: usize,
    /// Operand stack (per-frame).
    pub stack: Vec<JsValue>,
    /// Holding pen for RETRVAL — set by SETRVAL, returned by RETRVAL when
    /// the script falls off the end without an explicit RETURN.
    pub rval: JsValue,
    /// Mapping atom-index → binding kind/slot so NAME/SETNAME can quickly
    /// route to args / locals when the atom names a function parameter or
    /// local. Atom indices not present here resolve via the scope chain.
    pub atom_to_slot: Vec<Option<(JsBindingKind, usize)>>,
}

/// Result of running a frame to completion (or trying to advance one step).
pub enum StepOutcome {
    Continue,
    /// Function returned to the caller with this value.
    Return(JsValue),
    /// Unrecoverable error.
    Error(JsError),
}

pub struct JsRuntime {
    /// Top-level program scope (acts as the global object).
    pub global: JsObjectRef,
    /// Director runtime bridge. Held behind RefCell so Native closures can
    /// borrow it mutably during a call (single-threaded — no contention).
    pub bridge: std::rc::Rc<std::cell::RefCell<dyn super::host_bridge::JsHostBridge>>,
    /// Current call depth. Frames push and pop this around invoke() so we
    /// can refuse runaway recursion before the actual Rust stack overflows.
    call_depth: std::cell::Cell<u32>,
    /// Instruction budget shared across an entire host-level invocation
    /// (run_program or call_function). When zero, the dispatch loop bails
    /// out with an error rather than freezing the browser.
    instruction_budget: std::cell::Cell<u64>,
}

/// Hard limits that mirror Director's runtime constraints loosely. Big-enough
/// to run real movies; small-enough that a runaway loop in a JS handler
/// surfaces as a clear error instead of a hung tab.
const MAX_CALL_DEPTH: u32 = 256;
const MAX_INSTRUCTIONS_PER_INVOCATION: u64 = 50_000_000;

impl JsRuntime {
    /// New runtime with the ECMA-262 stdlib (Math, parseInt, etc.) installed.
    pub fn with_stdlib() -> Self {
        let rt = Self::new();
        super::builtins::install(&rt);
        rt
    }

    pub fn new() -> Self {
        let rt = JsRuntime {
            global: Rc::new(RefCell::new(JsObject::new())),
            bridge: std::rc::Rc::new(std::cell::RefCell::new(super::host_bridge::StubBridge)),
            call_depth: std::cell::Cell::new(0),
            instruction_budget: std::cell::Cell::new(MAX_INSTRUCTIONS_PER_INVOCATION),
        };
        // Built-in constructors: NAME "Array"/"Object" needs to resolve to
        // something at script load. The values themselves don't need to be
        // callable for array/object literal compilation -- NEWINIT does the
        // real work -- but they must exist so NAME doesn't error.
        rt.define_native("Array", |args| {
            // JS Array constructor semantics:
            //   new Array()          -> []
            //   new Array(n)         -> length-n array of undefined (n must
            //                            be a non-negative integer)
            //   new Array(a, b, c)   -> [a, b, c]
            let arr = Rc::new(RefCell::new(JsArray::new()));
            if args.len() == 1 {
                let len_hint = match &args[0] {
                    JsValue::Int(i) if *i >= 0 => Some(*i as usize),
                    JsValue::Number(n) if *n >= 0.0 && *n == n.trunc() && n.is_finite() => Some(*n as usize),
                    _ => None,
                };
                if let Some(n) = len_hint {
                    arr.borrow_mut().items.resize(n, JsValue::Undefined);
                    return Ok(JsValue::Array(arr));
                }
            }
            for v in args {
                arr.borrow_mut().items.push(v.clone());
            }
            Ok(JsValue::Array(arr))
        });
        rt.define_native("Object", |_args| {
            Ok(JsValue::Object(Rc::new(RefCell::new(JsObject::new()))))
        });
        rt
    }

    /// Install a native callable as a global property.
    pub fn define_native(
        &self,
        name: &'static str,
        f: impl Fn(&[JsValue]) -> Result<JsValue, JsError> + 'static,
    ) {
        let native = NativeFn { name, call: Box::new(f) };
        self.global.borrow_mut().set_own(name, JsValue::Native(Rc::new(native)));
    }

    /// Swap in a real Director bridge. Call before `run_program` for the
    /// bridge to be available to handlers.
    pub fn set_bridge(&mut self, bridge: std::rc::Rc<std::cell::RefCell<dyn super::host_bridge::JsHostBridge>>) {
        self.bridge = bridge;
    }

    /// Wire Director globals into the runtime. After this, `trace`, `sprite`,
    /// `member`, `go`, `puppetSprite`, and `updateStage` all resolve via the
    /// host bridge.
    pub fn install_director_globals(&self) {
        let bridge = self.bridge.clone();
        let b = bridge.clone();
        self.define_native("trace", move |args| {
            b.borrow_mut().trace(args);
            Ok(JsValue::Undefined)
        });
        let b = bridge.clone();
        self.define_native("put", move |args| {
            // `put` is Lingo's trace; the JS source code generated by
            // Director sometimes emits both names.
            b.borrow_mut().trace(args);
            Ok(JsValue::Undefined)
        });
        let b = bridge.clone();
        self.define_native("sprite", move |args| {
            let channel = args.get(0).map(|v| v.to_int32()).unwrap_or(0);
            Ok(b.borrow_mut().sprite(channel))
        });
        let b = bridge.clone();
        self.define_native("member", move |args| {
            Ok(b.borrow_mut().member(args))
        });
        let b = bridge.clone();
        self.define_native("castLib", move |args| {
            Ok(b.borrow_mut().cast_lib(args))
        });
        let b = bridge.clone();
        self.define_native("go", move |args| b.borrow_mut().go(args));
        let b = bridge.clone();
        self.define_native("puppetSprite", move |args| b.borrow_mut().puppet_sprite(args));
        let b = bridge.clone();
        self.define_native("updateStage", move |_args| b.borrow_mut().update_stage());
    }

    /// Run the top-level program of an IR. `function`-declared atoms get
    /// hoisted into the global object so they can be looked up as functions
    /// later via NAME atom_idx.
    pub fn run_program(&mut self, ir: &Rc<JsScriptIR>) -> Result<JsValue, JsError> {
        self.reset_invocation_budget();
        let frame = self.build_program_frame(ir);
        self.run_frame(frame)
    }

    /// Invoke a JsFunctionRef with the given args and `this`. Used by CALL,
    /// NEW (with proper construction wrapping), and the public host-call
    /// entry point.
    pub fn call_function(
        &mut self,
        f: &JsFunctionRef,
        args: Vec<JsValue>,
        this_value: JsValue,
    ) -> Result<JsValue, JsError> {
        self.reset_invocation_budget();
        let frame = build_function_frame(f, args, this_value, self.global.clone());
        self.run_frame(frame)
    }

    fn reset_invocation_budget(&self) {
        self.instruction_budget.set(MAX_INSTRUCTIONS_PER_INVOCATION);
        self.call_depth.set(0);
    }

    fn build_program_frame(&self, ir: &Rc<JsScriptIR>) -> JsFrame {
        let atoms = Rc::new(ir.atoms.clone());
        // Hoist DEFFUN atoms into the global scope so callers can resolve
        // function references without re-walking bytecode. This mirrors
        // js_NewScopeObject + DEFFUN semantics in SpiderMonkey.
        for a in atoms.iter() {
            if let JsAtom::Function(fa) = a {
                if let Some(name) = &fa.name {
                    let f = JsValue::Function(Rc::new(JsFunction { atom: Rc::new((**fa).clone()) }));
                    self.global.borrow_mut().set_own(name, f);
                }
            }
        }
        JsFrame {
            bytecode: Rc::new(ir.bytecode.clone()),
            atoms,
            args: Vec::new(),
            locals: Vec::new(),
            scope: self.global.clone(),
            parent_scope: None,
            this_value: JsValue::Object(self.global.clone()),
            pc: 0,
            stack: Vec::new(),
            rval: JsValue::Undefined,
            atom_to_slot: Vec::new(),
        }
    }

    /// Dispatch loop. Walks the current frame until it returns, errors,
    /// or hits an unimplemented op. CALL is the only op that pushes a
    /// new frame — implemented inline so we don't need a true frame stack.
    fn run_frame(&mut self, mut frame: JsFrame) -> Result<JsValue, JsError> {
        loop {
            // Instruction budget: a runaway loop in JS otherwise freezes the
            // wasm thread with no clue why. Decrement each step; abort cleanly
            // when the per-invocation quota is exhausted.
            let remaining = self.instruction_budget.get();
            if remaining == 0 {
                return Err(JsError::new(format!(
                    "execution limit exceeded after {} instructions — likely infinite loop",
                    MAX_INSTRUCTIONS_PER_INVOCATION
                )));
            }
            self.instruction_budget.set(remaining - 1);

            let pc = frame.pc;
            if pc >= frame.bytecode.len() {
                return Ok(std::mem::replace(&mut frame.rval, JsValue::Undefined));
            }
            let byte = frame.bytecode[pc];
            let op = match JsOp::from_byte(byte) {
                Some(o) => o,
                None => return Err(JsError::new(format!("unknown opcode 0x{:02x} at pc={}", byte, pc))),
            };
            let info = op.info();
            let op_len = if info.length < 0 {
                super::variable_length::variable_op_length(op, &frame.bytecode[pc..])
                    .map_err(JsError::new)?
            } else {
                info.length as usize
            };
            let operand_slice_start = pc + 1;
            let operand_slice_end = pc + op_len;
            // Borrow operand bytes into a local slice — careful, frame.bytecode is Rc so this is fine.
            let operand: &[u8] = unsafe {
                // We need a separate non-overlapping borrow over the bytecode for the operand.
                // Since we mutate frame.pc and frame.stack but not frame.bytecode, this is safe.
                std::slice::from_raw_parts(frame.bytecode.as_ptr().add(operand_slice_start), operand_slice_end - operand_slice_start)
            };
            // Default advance — overridden by jumps / CALL.
            frame.pc = operand_slice_end;
            match self.dispatch(&mut frame, op, operand, pc)? {
                StepOutcome::Continue => {}
                StepOutcome::Return(v) => return Ok(v),
                StepOutcome::Error(e) => return Err(e),
            }
        }
    }

    fn dispatch(
        &mut self,
        frame: &mut JsFrame,
        op: JsOp,
        operand: &[u8],
        op_pc: usize,
    ) -> Result<StepOutcome, JsError> {
        match op {
            // ===== Nullary / no-effect =====
            JsOp::Nop | JsOp::Group => Ok(StepOutcome::Continue),
            JsOp::Popv => {
                // POPV pops TOS into the script's return value slot.
                let v = pop(frame)?;
                frame.rval = v;
                Ok(StepOutcome::Continue)
            }
            JsOp::Pop => {
                let _ = pop(frame)?;
                Ok(StepOutcome::Continue)
            }
            JsOp::Dup => {
                let v = peek(frame)?.clone();
                frame.stack.push(v);
                Ok(StepOutcome::Continue)
            }
            JsOp::Setrval => {
                let v = pop(frame)?;
                frame.rval = v;
                Ok(StepOutcome::Continue)
            }
            JsOp::Retrval => {
                if frame.parent_scope.is_none() {
                    // Top-level RETRVAL: don't abort the script init.
                    return Ok(StepOutcome::Continue);
                }
                Ok(StepOutcome::Return(std::mem::replace(&mut frame.rval, JsValue::Undefined)))
            }
            JsOp::Return => {
                // SpiderMonkey 1.5 normally aborts the script on top-level
                // RETURN, but Director's port of leemon BigInt relies on
                // top-level `return` being treated like an expression
                // statement -- it stashes the value as the script's return
                // value and CONTINUES executing so the rest of the
                // initialisers run. We mirror that: at the top-level frame
                // (no parent scope), promote RETURN to SETRVAL.
                if frame.parent_scope.is_none() {
                    let v = pop(frame)?;
                    frame.rval = v;
                    return Ok(StepOutcome::Continue);
                }
                Ok(StepOutcome::Return(pop(frame)?))
            }

            // ===== Constant pushes =====
            JsOp::Push => { frame.stack.push(JsValue::Undefined); Ok(StepOutcome::Continue) }
            JsOp::Zero => { frame.stack.push(JsValue::Int(0)); Ok(StepOutcome::Continue) }
            JsOp::One => { frame.stack.push(JsValue::Int(1)); Ok(StepOutcome::Continue) }
            JsOp::Null => { frame.stack.push(JsValue::Null); Ok(StepOutcome::Continue) }
            JsOp::This => { frame.stack.push(frame.this_value.clone()); Ok(StepOutcome::Continue) }
            JsOp::False => { frame.stack.push(JsValue::Bool(false)); Ok(StepOutcome::Continue) }
            JsOp::True => { frame.stack.push(JsValue::Bool(true)); Ok(StepOutcome::Continue) }
            JsOp::String => {
                let idx = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let atom = frame.atoms.get(idx).ok_or_else(|| JsError::new("string atom oob"))?;
                match atom {
                    JsAtom::String(s) => frame.stack.push(JsValue::String(Rc::new(s.clone()))),
                    other => return Err(JsError::new(format!("STRING atom is not a string: {:?}", other))),
                }
                Ok(StepOutcome::Continue)
            }
            JsOp::Number => {
                let idx = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let atom = frame.atoms.get(idx).ok_or_else(|| JsError::new("number atom oob"))?;
                match atom {
                    JsAtom::Int(i) => frame.stack.push(JsValue::Int(*i)),
                    JsAtom::Double(d) => frame.stack.push(JsValue::Number(*d)),
                    other => return Err(JsError::new(format!("NUMBER atom is not numeric: {:?}", other))),
                }
                Ok(StepOutcome::Continue)
            }
            JsOp::Uint16 => {
                let v = read_u16_operand(operand).map_err(JsError::new)? as i32;
                frame.stack.push(JsValue::Int(v));
                Ok(StepOutcome::Continue)
            }

            // ===== Arithmetic =====
            JsOp::Add => {
                let b = pop(frame)?;
                let a = pop(frame)?;
                // JS + is overloaded: any string operand → string concat.
                let result = match (&a, &b) {
                    (JsValue::String(_), _) | (_, JsValue::String(_)) => {
                        JsValue::String(Rc::new(format!("{}{}", a.to_string(), b.to_string())))
                    }
                    _ => {
                        let na = a.to_number();
                        let nb = b.to_number();
                        let r = na + nb;
                        if r == r.trunc() && r.abs() < i32::MAX as f64 {
                            JsValue::Int(r as i32)
                        } else {
                            JsValue::Number(r)
                        }
                    }
                };
                frame.stack.push(result);
                Ok(StepOutcome::Continue)
            }
            JsOp::Sub => binop_num(frame, |a, b| a - b),
            JsOp::Mul => binop_num(frame, |a, b| a * b),
            JsOp::Div => binop_num(frame, |a, b| a / b),
            JsOp::Mod => binop_num(frame, |a, b| a % b),
            JsOp::Neg => {
                let a = pop(frame)?.to_number();
                frame.stack.push(JsValue::Number(-a));
                Ok(StepOutcome::Continue)
            }
            JsOp::Pos => {
                let a = pop(frame)?.to_number();
                frame.stack.push(JsValue::Number(a));
                Ok(StepOutcome::Continue)
            }
            JsOp::Not => {
                let a = pop(frame)?.to_bool();
                frame.stack.push(JsValue::Bool(!a));
                Ok(StepOutcome::Continue)
            }
            JsOp::Bitnot => {
                let v = pop(frame)?.to_int32();
                frame.stack.push(JsValue::Int(!v));
                Ok(StepOutcome::Continue)
            }
            JsOp::Bitor => binop_int(frame, |a, b| a | b),
            JsOp::Bitxor => binop_int(frame, |a, b| a ^ b),
            JsOp::Bitand => binop_int(frame, |a, b| a & b),
            JsOp::Lsh => binop_int(frame, |a, b| a.wrapping_shl((b & 31) as u32)),
            JsOp::Rsh => binop_int(frame, |a, b| a.wrapping_shr((b & 31) as u32)),
            JsOp::Ursh => {
                let b = (pop(frame)?.to_int32() & 31) as u32;
                let a = pop(frame)?.to_int32() as u32;
                frame.stack.push(JsValue::Int(a.wrapping_shr(b) as i32));
                Ok(StepOutcome::Continue)
            }

            // ===== Comparison =====
            JsOp::Eq => cmpop(frame, |o| matches!(o, std::cmp::Ordering::Equal)),
            JsOp::Ne => cmpop(frame, |o| !matches!(o, std::cmp::Ordering::Equal)),
            JsOp::Lt => cmpop(frame, |o| matches!(o, std::cmp::Ordering::Less)),
            JsOp::Le => cmpop(frame, |o| !matches!(o, std::cmp::Ordering::Greater)),
            JsOp::Gt => cmpop(frame, |o| matches!(o, std::cmp::Ordering::Greater)),
            JsOp::Ge => cmpop(frame, |o| !matches!(o, std::cmp::Ordering::Less)),
            JsOp::NewEq => cmpop(frame, |o| matches!(o, std::cmp::Ordering::Equal)),
            JsOp::NewNe => cmpop(frame, |o| !matches!(o, std::cmp::Ordering::Equal)),

            // ===== Control flow =====
            JsOp::Goto => {
                let delta = read_i16_operand(operand).map_err(JsError::new)? as i32;
                frame.pc = (op_pc as i32 + delta) as usize;
                Ok(StepOutcome::Continue)
            }
            JsOp::Ifeq => {
                let v = pop(frame)?;
                let delta = read_i16_operand(operand).map_err(JsError::new)? as i32;
                if !v.to_bool() {
                    frame.pc = (op_pc as i32 + delta) as usize;
                }
                Ok(StepOutcome::Continue)
            }
            JsOp::Ifne => {
                let v = pop(frame)?;
                let delta = read_i16_operand(operand).map_err(JsError::new)? as i32;
                if v.to_bool() {
                    frame.pc = (op_pc as i32 + delta) as usize;
                }
                Ok(StepOutcome::Continue)
            }
            JsOp::And => {
                // short-circuit and — jump if falsy, else pop
                let v = peek(frame)?.clone();
                let delta = read_i16_operand(operand).map_err(JsError::new)? as i32;
                if !v.to_bool() {
                    frame.pc = (op_pc as i32 + delta) as usize;
                } else {
                    frame.stack.pop();
                }
                Ok(StepOutcome::Continue)
            }
            JsOp::Or => {
                let v = peek(frame)?.clone();
                let delta = read_i16_operand(operand).map_err(JsError::new)? as i32;
                if v.to_bool() {
                    frame.pc = (op_pc as i32 + delta) as usize;
                } else {
                    frame.stack.pop();
                }
                Ok(StepOutcome::Continue)
            }

            // ===== Variable access (args / locals) =====
            JsOp::Getarg => {
                let slot = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let v = frame.args.get(slot).cloned().unwrap_or(JsValue::Undefined);
                frame.stack.push(v);
                Ok(StepOutcome::Continue)
            }
            JsOp::Setarg => {
                let slot = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let v = peek(frame)?.clone();
                if slot < frame.args.len() {
                    frame.args[slot] = v;
                }
                Ok(StepOutcome::Continue)
            }
            JsOp::Getvar => {
                let slot = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let v = frame.locals.get(slot).cloned().unwrap_or(JsValue::Undefined);
                frame.stack.push(v);
                Ok(StepOutcome::Continue)
            }
            JsOp::Setvar => {
                let slot = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let v = peek(frame)?.clone();
                if slot < frame.locals.len() {
                    frame.locals[slot] = v;
                }
                Ok(StepOutcome::Continue)
            }

            // ===== Scope-chain lookups (NAME / BINDNAME / SETNAME) =====
            JsOp::Name => {
                let idx = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let name = atom_string(&frame.atoms, idx)?;
                let v = match resolve_name(frame, &name) {
                    Some(v) => v,
                    None => {
                        // Fallback: unknown name. Director's Lingo runtime has
                        // hundreds of built-in globals (gotoNetPage, getNetText,
                        // puppetTempo, count, the * properties …) — too many to
                        // pre-bind one-by-one. We hand back a "deferred-call"
                        // Native that, when invoked, routes through the host
                        // bridge's `call_global`. If JS just reads the name
                        // without calling it (rare), the value is callable but
                        // not callable as anything else; we surface that as
                        // `undefined: <name>` only on actual non-call use later.
                        let bridge = self.bridge.clone();
                        let captured_name: Rc<String> = Rc::new(name);
                        let cn_for_native = captured_name.clone();
                        let native = super::value::NativeFn {
                            name: "<host-call>",
                            call: Box::new(move |args| {
                                bridge.borrow_mut().call_global(&cn_for_native, args)
                            }),
                        };
                        JsValue::Native(Rc::new(native))
                    }
                };
                frame.stack.push(v);
                Ok(StepOutcome::Continue)
            }
            JsOp::Bindname => {
                // SpiderMonkey BINDNAME pushes the scope OBJECT that contains
                // `name` so a subsequent GETPROP / SETPROP / SETNAME knows
                // where to read / write. Director's BigInt port emits
                // `BINDNAME bi_bpe; BINDNAME bi_bpe; GETPROP bi_bpe; …`
                // for `bi_bpe = bi_bpe >> 1` -- the inner GETPROP reads the
                // var THROUGH the scope object. Previously we pushed
                // `Undefined`, so the GETPROP returned undefined and the
                // assignment ended up storing 0. Push the program scope
                // (global object) instead; that's where top-level vars and
                // function decls live, and store_name walks the chain so
                // the matching SETNAME still works for nested scopes too.
                frame.stack.push(JsValue::Object(self.global.clone()));
                Ok(StepOutcome::Continue)
            }
            JsOp::Setname => {
                let idx = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let name = atom_string(&frame.atoms, idx)?;
                let v = pop(frame)?; // RHS
                let _lhs_marker = pop(frame)?; // BINDNAME marker
                store_name(frame, &name, v.clone(), &self.global);
                frame.stack.push(v);
                Ok(StepOutcome::Continue)
            }

            // ===== Declarations =====
            JsOp::Defvar => {
                let idx = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let name = atom_string(&frame.atoms, idx)?;
                // Bind on current scope (the program scope for top-level vars).
                let mut s = frame.scope.borrow_mut();
                if !s.has_own(&name) {
                    s.set_own(&name, JsValue::Undefined);
                }
                Ok(StepOutcome::Continue)
            }
            JsOp::Deffun => {
                let idx = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let atom = frame.atoms.get(idx).cloned()
                    .ok_or_else(|| JsError::new("DEFFUN atom oob"))?;
                if let JsAtom::Function(fa) = atom {
                    if let Some(name) = fa.name.clone() {
                        let f = JsValue::Function(Rc::new(JsFunction { atom: Rc::new((*fa).clone()) }));
                        self.global.borrow_mut().set_own(&name, f);
                    }
                }
                Ok(StepOutcome::Continue)
            }
            JsOp::Defconst => {
                let idx = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let name = atom_string(&frame.atoms, idx)?;
                let mut s = frame.scope.borrow_mut();
                if !s.has_own(&name) {
                    s.set_own(&name, JsValue::Undefined);
                }
                Ok(StepOutcome::Continue)
            }

            // ===== Property access =====
            JsOp::Getprop => {
                let idx = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let name = atom_string(&frame.atoms, idx)?;
                let obj = pop(frame)?;
                let v = get_property(&obj, &name);
                frame.stack.push(v);
                Ok(StepOutcome::Continue)
            }
            JsOp::Setprop => {
                let idx = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let name = atom_string(&frame.atoms, idx)?;
                let value = pop(frame)?;
                let obj = pop(frame)?;
                set_property(&obj, &name, value.clone())?;
                frame.stack.push(value);
                Ok(StepOutcome::Continue)
            }
            JsOp::Getelem => {
                let key = pop(frame)?;
                let obj = pop(frame)?;
                let v = get_element(&obj, &key);
                frame.stack.push(v);
                Ok(StepOutcome::Continue)
            }
            JsOp::Setelem => {
                let value = pop(frame)?;
                let key = pop(frame)?;
                let obj = pop(frame)?;
                set_element(&obj, &key, value.clone())?;
                frame.stack.push(value);
                Ok(StepOutcome::Continue)
            }

            // ===== Object / array initialization =====
            JsOp::Newinit => {
                // SpiderMonkey 1.5 emits array literals as
                //   NAME "Array"; PUSHOBJ; NEWINIT; INITELEM*; ENDINIT
                // and object literals as
                //   NAME "Object"; PUSHOBJ; NEWINIT; INITPROP*; ENDINIT
                // The constructor "fn" and "this" stay on the stack as leftovers
                // (CALL convention placeholders); NEWINIT itself just pushes a
                // fresh container. We pick container kind by looking at the
                // Function value that's still sitting below the PUSHOBJ marker.
                let kind_is_array = match peek_at(frame, 1) {
                    Ok(JsValue::Function(f)) => f.atom.name.as_deref() == Some("Array"),
                    Ok(JsValue::Native(f)) => f.name == "Array",
                    _ => false,
                };
                let v = if kind_is_array {
                    JsValue::Array(Rc::new(RefCell::new(JsArray::new())))
                } else {
                    JsValue::Object(Rc::new(RefCell::new(JsObject::new())))
                };
                frame.stack.push(v);
                Ok(StepOutcome::Continue)
            }
            JsOp::Initprop => {
                let idx = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let name = atom_string(&frame.atoms, idx)?;
                let value = pop(frame)?;
                let obj = peek(frame)?.clone();
                set_property(&obj, &name, value)?;
                Ok(StepOutcome::Continue)
            }
            JsOp::Initelem => {
                let value = pop(frame)?;
                let key = pop(frame)?;
                let obj = peek(frame)?.clone();
                set_element(&obj, &key, value)?;
                Ok(StepOutcome::Continue)
            }
            JsOp::Endinit => Ok(StepOutcome::Continue),

            // ===== Calls =====
            JsOp::Call => {
                // SpiderMonkey CALL window layout (bottom→top): [fn, this, arg0, ..., argN-1].
                // Pop args, then `this`, then `fn` (jsinterp.c GET_ARGC + vp[0]=fn).
                let argc = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let mut args = Vec::with_capacity(argc);
                for _ in 0..argc {
                    args.push(pop(frame)?);
                }
                args.reverse();
                let this_value = pop(frame)?;
                let callee = pop(frame)?;
                let result = self.invoke(&callee, args, this_value).map_err(|e| {
                    // Annotate the error with the previous instruction's
                    // name so users can tell which lookup produced the
                    // undefined callee. Best-effort: walk back from this
                    // op's PC and try to identify a NAME / GETPROP / GETELEM.
                    if e.message.starts_with("not callable") {
                        let hint = guess_callee_source(&frame.bytecode, &frame.atoms, op_pc);
                        JsError::new(format!("{} (callee was {})", e.message, hint))
                    } else { e }
                })?;
                frame.stack.push(result);
                Ok(StepOutcome::Continue)
            }
            JsOp::Pushobj => {
                // PUSHOBJ pushes a `this` placeholder. For unqualified calls
                // (e.g. trace("x")), `this` is the global object.
                frame.stack.push(JsValue::Object(self.global.clone()));
                Ok(StepOutcome::Continue)
            }
            JsOp::New => {
                let argc = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let mut args = Vec::with_capacity(argc);
                for _ in 0..argc {
                    args.push(pop(frame)?);
                }
                args.reverse();
                let _ignored_this = pop(frame)?; // NEW also has a `this` slot in the call window, but it's unused
                let ctor = pop(frame)?;
                let new_this = JsValue::Object(Rc::new(RefCell::new(JsObject::new())));
                let ret = self.invoke(&ctor, args, new_this.clone())?;
                match ret {
                    JsValue::Object(_) | JsValue::Array(_) => frame.stack.push(ret),
                    _ => frame.stack.push(new_this),
                }
                Ok(StepOutcome::Continue)
            }

            // ===== Stack management =====
            JsOp::Swap => {
                let b = pop(frame)?;
                let a = pop(frame)?;
                frame.stack.push(b);
                frame.stack.push(a);
                Ok(StepOutcome::Continue)
            }
            JsOp::Dup2 => {
                let top = peek_at(frame, 0)?.clone();
                let nx = peek_at(frame, 1)?.clone();
                frame.stack.push(nx);
                frame.stack.push(top);
                Ok(StepOutcome::Continue)
            }

            // ===== Increment / decrement (name and slot variants) =====
            JsOp::Incname | JsOp::Nameinc => incdec_name(frame, &self.global, operand, 1, op == JsOp::Nameinc),
            JsOp::Decname | JsOp::Namedec => incdec_name(frame, &self.global, operand, -1, op == JsOp::Namedec),
            JsOp::Incarg | JsOp::Arginc   => incdec_slot(frame, operand, 1,  /*arg*/ true,  /*post*/ op == JsOp::Arginc),
            JsOp::Decarg | JsOp::Argdec   => incdec_slot(frame, operand, -1, /*arg*/ true,  /*post*/ op == JsOp::Argdec),
            JsOp::Incvar | JsOp::Varinc   => incdec_slot(frame, operand, 1,  /*arg*/ false, /*post*/ op == JsOp::Varinc),
            JsOp::Decvar | JsOp::Vardec   => incdec_slot(frame, operand, -1, /*arg*/ false, /*post*/ op == JsOp::Vardec),
            JsOp::Incprop | JsOp::Propinc => incdec_prop(frame, operand, 1,  /*post*/ op == JsOp::Propinc),
            JsOp::Decprop | JsOp::Propdec => incdec_prop(frame, operand, -1, /*post*/ op == JsOp::Propdec),
            JsOp::Incelem | JsOp::Eleminc => incdec_elem(frame, 1,  /*post*/ op == JsOp::Eleminc),
            JsOp::Decelem | JsOp::Elemdec => incdec_elem(frame, -1, /*post*/ op == JsOp::Elemdec),

            // ===== Misc =====
            JsOp::Typeof => {
                let v = pop(frame)?;
                frame.stack.push(JsValue::String(Rc::new(v.type_of().into())));
                Ok(StepOutcome::Continue)
            }
            JsOp::Void => {
                let _ = pop(frame)?;
                frame.stack.push(JsValue::Undefined);
                Ok(StepOutcome::Continue)
            }
            JsOp::In => {
                let obj = pop(frame)?;
                let key = pop(frame)?;
                let name = key.to_string();
                let present = match &obj {
                    JsValue::Object(o) => o.borrow().has_own(&name),
                    JsValue::Array(a) => name == "length"
                        || name.parse::<usize>().map_or(false, |i| i < a.borrow().items.len()),
                    _ => false,
                };
                frame.stack.push(JsValue::Bool(present));
                Ok(StepOutcome::Continue)
            }
            JsOp::Instanceof => {
                let _ctor = pop(frame)?;
                let _val = pop(frame)?;
                // SpiderMonkey 1.5 walks the prototype chain. Lacking real
                // ctors right now we always say false; correctness when the
                // movie actually needs `instanceof` lands with the prototype
                // chain implementation.
                frame.stack.push(JsValue::Bool(false));
                Ok(StepOutcome::Continue)
            }
            JsOp::Delprop => {
                let idx = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let name = atom_string(&frame.atoms, idx)?;
                let obj = pop(frame)?;
                let removed = match &obj {
                    JsValue::Object(o) => {
                        let mut b = o.borrow_mut();
                        let before = b.props.len();
                        b.props.retain(|(k, _)| k != &name);
                        before != b.props.len()
                    }
                    _ => false,
                };
                frame.stack.push(JsValue::Bool(removed));
                Ok(StepOutcome::Continue)
            }
            JsOp::Delname => {
                let idx = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let name = atom_string(&frame.atoms, idx)?;
                let mut b = self.global.borrow_mut();
                let before = b.props.len();
                b.props.retain(|(k, _)| k != &name);
                let removed = before != b.props.len();
                drop(b);
                frame.stack.push(JsValue::Bool(removed));
                Ok(StepOutcome::Continue)
            }
            JsOp::Delelem => {
                let key = pop(frame)?;
                let obj = pop(frame)?;
                let name = key.to_string();
                let removed = match &obj {
                    JsValue::Object(o) => {
                        let mut b = o.borrow_mut();
                        let before = b.props.len();
                        b.props.retain(|(k, _)| k != &name);
                        before != b.props.len()
                    }
                    JsValue::Array(a) => {
                        if let Ok(i) = name.parse::<usize>() {
                            let mut b = a.borrow_mut();
                            if i < b.items.len() {
                                b.items[i] = JsValue::Undefined;
                                true
                            } else { false }
                        } else { false }
                    }
                    _ => false,
                };
                frame.stack.push(JsValue::Bool(removed));
                Ok(StepOutcome::Continue)
            }

            // ===== Extended (x-suffix) jumps: 4-byte offsets =====
            JsOp::Gotox => {
                let delta = read_i32_operand(operand).map_err(JsError::new)?;
                frame.pc = (op_pc as i32 + delta) as usize;
                Ok(StepOutcome::Continue)
            }
            JsOp::Ifeqx => {
                let v = pop(frame)?;
                let delta = read_i32_operand(operand).map_err(JsError::new)?;
                if !v.to_bool() { frame.pc = (op_pc as i32 + delta) as usize; }
                Ok(StepOutcome::Continue)
            }
            JsOp::Ifnex => {
                let v = pop(frame)?;
                let delta = read_i32_operand(operand).map_err(JsError::new)?;
                if v.to_bool() { frame.pc = (op_pc as i32 + delta) as usize; }
                Ok(StepOutcome::Continue)
            }
            JsOp::Andx => {
                let v = peek(frame)?.clone();
                let delta = read_i32_operand(operand).map_err(JsError::new)?;
                if !v.to_bool() { frame.pc = (op_pc as i32 + delta) as usize; }
                else { frame.stack.pop(); }
                Ok(StepOutcome::Continue)
            }
            JsOp::Orx => {
                let v = peek(frame)?.clone();
                let delta = read_i32_operand(operand).map_err(JsError::new)?;
                if v.to_bool() { frame.pc = (op_pc as i32 + delta) as usize; }
                else { frame.stack.pop(); }
                Ok(StepOutcome::Continue)
            }

            // ===== Switch =====
            JsOp::Condswitch => Ok(StepOutcome::Continue), // pure marker, no effect
            JsOp::Default => {
                let _ = pop(frame)?;
                let delta = read_i16_operand(operand).map_err(JsError::new)? as i32;
                frame.pc = (op_pc as i32 + delta) as usize;
                Ok(StepOutcome::Continue)
            }
            JsOp::Defaultx => {
                let _ = pop(frame)?;
                let delta = read_i32_operand(operand).map_err(JsError::new)?;
                frame.pc = (op_pc as i32 + delta) as usize;
                Ok(StepOutcome::Continue)
            }
            JsOp::Case => {
                // NEW_EQUALITY_OP: pop b, compare to PEEK a (don't pop). If
                // equal, pop a too and jump; else keep a, advance to next case.
                let b = pop(frame)?;
                let a = peek(frame)?.clone();
                let delta = read_i16_operand(operand).map_err(JsError::new)? as i32;
                if loose_equal(&a, &b) {
                    let _ = pop(frame)?;
                    frame.pc = (op_pc as i32 + delta) as usize;
                }
                Ok(StepOutcome::Continue)
            }
            JsOp::Casex => {
                let b = pop(frame)?;
                let a = peek(frame)?.clone();
                let delta = read_i32_operand(operand).map_err(JsError::new)?;
                if loose_equal(&a, &b) {
                    let _ = pop(frame)?;
                    frame.pc = (op_pc as i32 + delta) as usize;
                }
                Ok(StepOutcome::Continue)
            }
            JsOp::Tableswitch => {
                let disc = pop(frame)?;
                let i = if let JsValue::Int(v) = disc { v }
                        else if let JsValue::Number(d) = disc {
                            if d == d.trunc() && d.is_finite() { d as i32 } else { return Ok(StepOutcome::Continue); }
                        } else { return Ok(StepOutcome::Continue); }; // non-int falls through default
                let default_delta = read_i16_operand(&operand[0..]).map_err(JsError::new)? as i32;
                let low = read_i16_operand(&operand[2..]).map_err(JsError::new)? as i32;
                let high = read_i16_operand(&operand[4..]).map_err(JsError::new)? as i32;
                let idx = i - low;
                if idx >= 0 && idx <= high - low {
                    let case_off_pos = 6 + (idx as usize) * 2;
                    let case_delta = read_i16_operand(&operand[case_off_pos..]).map_err(JsError::new)? as i32;
                    if case_delta != 0 {
                        frame.pc = (op_pc as i32 + case_delta) as usize;
                        return Ok(StepOutcome::Continue);
                    }
                }
                frame.pc = (op_pc as i32 + default_delta) as usize;
                Ok(StepOutcome::Continue)
            }
            JsOp::Tableswitchx => {
                let disc = pop(frame)?;
                let i = if let JsValue::Int(v) = disc { v }
                        else if let JsValue::Number(d) = disc {
                            if d == d.trunc() && d.is_finite() { d as i32 } else { return Ok(StepOutcome::Continue); }
                        } else { return Ok(StepOutcome::Continue); };
                let default_delta = read_i32_operand(&operand[0..]).map_err(JsError::new)?;
                let low = read_i32_operand(&operand[4..]).map_err(JsError::new)?;
                let high = read_i32_operand(&operand[8..]).map_err(JsError::new)?;
                let idx = i - low;
                if idx >= 0 && idx <= high - low {
                    let case_off_pos = 12 + (idx as usize) * 4;
                    let case_delta = read_i32_operand(&operand[case_off_pos..]).map_err(JsError::new)?;
                    if case_delta != 0 {
                        frame.pc = (op_pc as i32 + case_delta) as usize;
                        return Ok(StepOutcome::Continue);
                    }
                }
                frame.pc = (op_pc as i32 + default_delta) as usize;
                Ok(StepOutcome::Continue)
            }
            JsOp::Lookupswitch => {
                let disc = pop(frame)?;
                let default_delta = read_i16_operand(&operand[0..]).map_err(JsError::new)? as i32;
                let npairs = read_u16_operand(&operand[2..]).map_err(JsError::new)? as usize;
                let mut pos = 4;
                for _ in 0..npairs {
                    let atom_idx = read_u16_operand(&operand[pos..]).map_err(JsError::new)? as usize;
                    let case_delta = read_i16_operand(&operand[pos + 2..]).map_err(JsError::new)? as i32;
                    let atom_val = atom_to_value(&frame.atoms, atom_idx)?;
                    if loose_equal(&disc, &atom_val) {
                        frame.pc = (op_pc as i32 + case_delta) as usize;
                        return Ok(StepOutcome::Continue);
                    }
                    pos += 4;
                }
                frame.pc = (op_pc as i32 + default_delta) as usize;
                Ok(StepOutcome::Continue)
            }
            JsOp::Lookupswitchx => {
                let disc = pop(frame)?;
                let default_delta = read_i32_operand(&operand[0..]).map_err(JsError::new)?;
                let npairs = read_u32_operand(&operand[4..]).map_err(JsError::new)? as usize;
                let mut pos = 8;
                for _ in 0..npairs {
                    let atom_idx = read_u32_operand(&operand[pos..]).map_err(JsError::new)? as usize;
                    let case_delta = read_i32_operand(&operand[pos + 4..]).map_err(JsError::new)?;
                    let atom_val = atom_to_value(&frame.atoms, atom_idx)?;
                    if loose_equal(&disc, &atom_val) {
                        frame.pc = (op_pc as i32 + case_delta) as usize;
                        return Ok(StepOutcome::Continue);
                    }
                    pos += 8;
                }
                frame.pc = (op_pc as i32 + default_delta) as usize;
                Ok(StepOutcome::Continue)
            }

            // ===== Exception handling =====
            // Phase 2b lays the groundwork — Throw/Try/Exception/Initcatchvar
            // are all stack-shape-correct, but proper unwind through try_notes
            // ties into the runtime in Phase 6. Until then THROW just propagates
            // a JsError.
            JsOp::Try | JsOp::Finally => Ok(StepOutcome::Continue),
            JsOp::Throw => {
                let v = pop(frame)?;
                Err(JsError::new(format!("uncaught: {}", v.to_string())))
            }
            JsOp::Exception => {
                // After unwind, the exception value would be on top. With our
                // current model we shouldn't reach here without an unwind.
                frame.stack.push(JsValue::Undefined);
                Ok(StepOutcome::Continue)
            }
            JsOp::Initcatchvar => {
                let idx = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let name = atom_string(&frame.atoms, idx)?;
                let value = pop(frame)?;
                let obj = peek(frame)?.clone();
                set_property(&obj, &name, value)?;
                Ok(StepOutcome::Continue)
            }
            JsOp::Gosub => {
                // Push return-PC and jump. The matching RETSUB pops it.
                let delta = read_i16_operand(operand).map_err(JsError::new)? as i32;
                let return_pc = frame.pc as i32; // already advanced past this op
                frame.stack.push(JsValue::Int(return_pc));
                frame.pc = (op_pc as i32 + delta) as usize;
                Ok(StepOutcome::Continue)
            }
            JsOp::Gosubx => {
                let delta = read_i32_operand(operand).map_err(JsError::new)?;
                let return_pc = frame.pc as i32;
                frame.stack.push(JsValue::Int(return_pc));
                frame.pc = (op_pc as i32 + delta) as usize;
                Ok(StepOutcome::Continue)
            }
            JsOp::Retsub => {
                let target = pop(frame)?.to_int32();
                frame.pc = target as usize;
                Ok(StepOutcome::Continue)
            }
            JsOp::Setsp => {
                // Reset the stack to a given depth (used by try-catch unwinding).
                let depth = read_u16_operand(operand).map_err(JsError::new)? as usize;
                frame.stack.truncate(depth);
                Ok(StepOutcome::Continue)
            }

            // ===== Closures and function expressions =====
            JsOp::Closure | JsOp::Anonfunobj | JsOp::Namedfunobj => {
                let idx = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let atom = frame.atoms.get(idx).cloned()
                    .ok_or_else(|| JsError::new("function atom oob"))?;
                let JsAtom::Function(fa) = atom else {
                    return Err(JsError::new("function-atom-op expected JsAtom::Function"));
                };
                let f = JsValue::Function(Rc::new(JsFunction {
                    atom: Rc::new((*fa).clone()),
                }));
                if op == JsOp::Closure {
                    // CLOSURE additionally installs the function on the current
                    // scope under its name.
                    if let JsAtom::Function(ref fa2) = frame.atoms[idx] {
                        if let Some(name) = &fa2.name {
                            frame.scope.borrow_mut().set_own(name, f.clone());
                        }
                    }
                }
                frame.stack.push(f);
                Ok(StepOutcome::Continue)
            }
            JsOp::Deflocalfun => {
                // DEFLOCALFUN: VARNO_LEN + ATOM_INDEX_LEN. var slot, then atom idx.
                let slot = read_u16_operand(&operand[0..]).map_err(JsError::new)? as usize;
                let atom_idx = read_u16_operand(&operand[2..]).map_err(JsError::new)? as usize;
                let atom = frame.atoms.get(atom_idx).cloned()
                    .ok_or_else(|| JsError::new("DEFLOCALFUN atom oob"))?;
                if let JsAtom::Function(fa) = atom {
                    let f = JsValue::Function(Rc::new(JsFunction { atom: Rc::new((*fa).clone()) }));
                    if slot < frame.locals.len() { frame.locals[slot] = f; }
                }
                Ok(StepOutcome::Continue)
            }

            // ===== `with` =====
            JsOp::Enterwith => {
                let v = pop(frame)?;
                // SpiderMonkey wraps the value in a scope object whose proto
                // points at the previous scope. We mimic this with a fresh
                // JsObject whose proto chain extends to the current scope.
                let wrapper = match v {
                    JsValue::Object(o) => o,
                    _ => Rc::new(RefCell::new(JsObject::new())),
                };
                wrapper.borrow_mut().proto = Some(frame.scope.clone());
                frame.scope = wrapper;
                Ok(StepOutcome::Continue)
            }
            JsOp::Leavewith => {
                let parent = frame.scope.borrow().proto.clone();
                if let Some(p) = parent { frame.scope = p; }
                Ok(StepOutcome::Continue)
            }

            // ===== `for..in` iteration =====
            // Phase 2b stubs: keys-list is computed eagerly into a JsArray,
            // and the FOR* ops walk it as a counter (no live iterator object).
            // Correctness is good enough for unsorted enumeration; ordering
            // matches insertion-order (matches SpiderMonkey for non-numeric keys).
            JsOp::Forarg => Err(JsError::new("for-in (forarg) needs runtime iter state — Phase 6")),
            JsOp::Forvar => Err(JsError::new("for-in (forvar) needs runtime iter state — Phase 6")),
            JsOp::Forname => Err(JsError::new("for-in (forname) needs runtime iter state — Phase 6")),
            JsOp::Forprop => Err(JsError::new("for-in (forprop) needs runtime iter state — Phase 6")),
            JsOp::Forelem => Err(JsError::new("for-in (forelem) needs runtime iter state — Phase 6")),
            JsOp::Enumelem => Err(JsError::new("for-in (enumelem) needs runtime iter state — Phase 6")),

            // ===== Stack reset / misc =====
            JsOp::Setconst => {
                let idx = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let name = atom_string(&frame.atoms, idx)?;
                let v = peek(frame)?.clone();
                frame.scope.borrow_mut().set_own(&name, v);
                Ok(StepOutcome::Continue)
            }
            JsOp::Toobject => {
                // Coerce TOS to object; for objects/arrays/functions it's a no-op,
                // for primitives we'd wrap (Phase 3 — Boolean/Number/String wrapper
                // classes). For now leave value unchanged.
                Ok(StepOutcome::Continue)
            }

            // ===== Arguments object — Phase 6 (needs an `arguments` reflection) =====
            JsOp::Arguments | JsOp::Argsub | JsOp::Argcnt => Err(JsError::new(format!(
                "arguments-object op {:?} needs Phase 6 reflection", op
            ))),

            // ===== Module system (Mozilla-only, never seen in DCRs) =====
            JsOp::Exportall | JsOp::Exportname | JsOp::Importall
            | JsOp::Importprop | JsOp::Importelem => Ok(StepOutcome::Continue),

            // ===== Object literal accessors / getters/setters =====
            JsOp::Getter | JsOp::Setter => {
                // Define an accessor on the initializing object. Phase 2b stub:
                // pop the function value and discard (we don't yet honour
                // getter/setter semantics on lookup).
                let _ = pop(frame)?;
                Ok(StepOutcome::Continue)
            }

            // ===== Sharp variables (#1=...) — extremely rare, no-op =====
            JsOp::Defsharp | JsOp::Usesharp => Ok(StepOutcome::Continue),

            // ===== Regex / object literal atom =====
            JsOp::Object => {
                let idx = read_u16_operand(operand).map_err(JsError::new)? as usize;
                let atom = frame.atoms.get(idx).cloned()
                    .ok_or_else(|| JsError::new("Object atom oob"))?;
                let v = match atom {
                    JsAtom::Function(fa) => JsValue::Function(Rc::new(JsFunction { atom: Rc::new((*fa).clone()) })),
                    _ => JsValue::Object(Rc::new(RefCell::new(JsObject::new()))),
                };
                frame.stack.push(v);
                Ok(StepOutcome::Continue)
            }

            // ===== Eval — too complex for an embedded interpreter without a
            // parser; treat the eval source as a string and warn. =====
            JsOp::Eval => {
                let _ = pop(frame)?; // pop the source string (or function value)
                // CALL semantics swallow trailing args; we already are past those
                // (eval is dispatched after argument push by the surrounding CALL).
                frame.stack.push(JsValue::Undefined);
                Ok(StepOutcome::Continue)
            }

            // ===== Misc internals =====
            JsOp::Setcall => Ok(StepOutcome::Continue), // optimisation marker
            JsOp::Trap | JsOp::Debugger => Ok(StepOutcome::Continue),

            // ===== Backpatch markers — compile-time placeholders that the
            // emitter rewrites before script serialisation. They must not
            // appear at runtime; if one does, it's a compiler bug. =====
            JsOp::Backpatch | JsOp::BackpatchPop | JsOp::BackpatchPush => Err(JsError::new(format!(
                "compile-time backpatch op {:?} should never execute", op
            ))),

            // Catch-all for anything we missed.
            other => Err(JsError::new(format!("unimplemented opcode {:?} (0x{:02x}) at pc={}", other, byte_of(other), op_pc))),
        }
    }

    /// Call any callable JsValue. Routes to JsFunction (interpreted) or
    /// NativeFn (Rust closure).
    pub fn invoke(
        &mut self,
        callee: &JsValue,
        args: Vec<JsValue>,
        this_value: JsValue,
    ) -> Result<JsValue, JsError> {
        let depth = self.call_depth.get();
        if depth >= MAX_CALL_DEPTH {
            return Err(JsError::new(format!(
                "call depth {} exceeds limit ({}) — likely infinite recursion",
                depth, MAX_CALL_DEPTH
            )));
        }
        self.call_depth.set(depth + 1);
        let result = match callee {
            JsValue::Function(f) => {
                let frame = build_function_frame(f, args, this_value, self.global.clone());
                self.run_frame(frame)
            }
            JsValue::Native(f) => (f.call)(&args),
            other => Err(JsError::new(format!("not callable: {:?}", other))),
        };
        self.call_depth.set(depth);
        result
    }
}

fn byte_of(op: JsOp) -> u8 { op as u8 }

/// Walk the bytecode forwards from PC=0 to call_pc, tracking stack depth so
/// we can identify the precise op that produced the callee for the failing
/// CALL. For `a(b(c))` the naive "most recent lookup" heuristic would pick
/// the innermost call's args, not the outer callee — this version handles
/// nesting correctly.
fn guess_callee_source(bytecode: &[u8], atoms: &[JsAtom], call_pc: usize) -> String {
    use super::variable_length::read_u16_operand;

    // For each forward-emitted op, remember (offset, op, stack_depth_after).
    // After we finish walking, look at the entry whose depth_after equals
    // depth_at_call_pc - argc - 2 + 1 (i.e. the moment the callee push
    // landed). The op AT that record is the callee producer.
    let mut history: Vec<(usize, JsOp, i32)> = Vec::new();
    let mut depth: i32 = 0;
    let mut scan = 0usize;
    let mut argc_at_call: u16 = 0;
    while scan < bytecode.len() {
        let byte = bytecode[scan];
        let op = match JsOp::from_byte(byte) { Some(o) => o, None => { scan += 1; continue; } };
        let info = op.info();
        let len = if info.length > 0 { info.length as usize } else { 1 };
        let uses = if info.uses >= 0 {
            info.uses as i32
        } else if matches!(op, JsOp::Call | JsOp::New) {
            if scan + 3 <= bytecode.len() {
                read_u16_operand(&bytecode[scan + 1..]).unwrap_or(0) as i32 + 2
            } else { 2 }
        } else { 0 };
        let defs = info.defs as i32;
        depth = depth - uses + defs;
        history.push((scan, op, depth));
        if scan == call_pc && matches!(op, JsOp::Call | JsOp::New) {
            argc_at_call = read_u16_operand(&bytecode[scan + 1..]).unwrap_or(0);
            break;
        }
        scan += len.max(1);
    }

    // depth_at_call_pc (after) = before_depth - (argc + 2) + 1.
    // The callee was the value at stack position `before_depth - (argc + 2)`
    // i.e. the FIRST of the (callee, this, args) trio. Going forward, that
    // value came from the op whose `depth_after` equals
    // `depth_at_call_pc - 1 + 1 = depth_at_call_pc`... actually the op that
    // produced the callee left the stack at depth = base + 1, where base is
    // the call window's bottom. We want the LATEST entry whose depth equals
    // `(depth_after_call - 1) + 1 = depth_after_call`. Hmm — the result of
    // CALL replaces the whole call window with one value, so depth_after_call
    // equals base + 1. The op that produced the callee also left depth =
    // base + 1. So search history for the latest record (before call_pc)
    // whose depth_after == depth_after_call AND is a lookup op.
    let depth_after_call = history.last().map(|e| e.2).unwrap_or(0);
    let target_depth = depth_after_call;
    let _ = argc_at_call;

    // Find the latest lookup op (before call_pc) whose recorded depth_after
    // equals target_depth.
    let mut found: Option<(JsOp, usize)> = None;
    for (off, op, d) in history.iter().rev() {
        if *off >= call_pc { continue; }
        if *d != target_depth { continue; }
        if matches!(op, JsOp::Name | JsOp::Getprop | JsOp::Getelem
            | JsOp::Getarg | JsOp::Getvar | JsOp::This | JsOp::Bindname) {
            found = Some((*op, *off));
            break;
        }
    }

    match found {
        Some((JsOp::Name, off)) | Some((JsOp::Bindname, off)) => {
            let idx = read_u16_operand(&bytecode[off + 1..]).unwrap_or(0) as usize;
            let name = match atoms.get(idx) {
                Some(JsAtom::String(s)) => s.as_str(),
                _ => "?",
            };
            format!("NAME {:?} at pc {}", name, off)
        }
        Some((JsOp::Getprop, off)) => {
            let idx = read_u16_operand(&bytecode[off + 1..]).unwrap_or(0) as usize;
            let name = match atoms.get(idx) {
                Some(JsAtom::String(s)) => s.as_str(),
                _ => "?",
            };
            format!("GETPROP .{} at pc {}", name, off)
        }
        Some((JsOp::Getelem, off)) => format!("GETELEM obj[key] at pc {}", off),
        Some((JsOp::Getarg, off)) => {
            let slot = read_u16_operand(&bytecode[off + 1..]).unwrap_or(0);
            format!("GETARG #{} at pc {}", slot, off)
        }
        Some((JsOp::Getvar, off)) => {
            let slot = read_u16_operand(&bytecode[off + 1..]).unwrap_or(0);
            format!("GETVAR #{} at pc {}", slot, off)
        }
        Some((JsOp::This, off)) => format!("THIS at pc {}", off),
        _ => format!("<unknown, call at pc {}>", call_pc),
    }
}

fn pop(frame: &mut JsFrame) -> Result<JsValue, JsError> {
    frame.stack.pop().ok_or_else(|| JsError::new("stack underflow"))
}

fn peek(frame: &JsFrame) -> Result<&JsValue, JsError> {
    frame.stack.last().ok_or_else(|| JsError::new("stack underflow (peek)"))
}

fn peek_at(frame: &JsFrame, depth: usize) -> Result<&JsValue, JsError> {
    let n = frame.stack.len();
    if depth + 1 > n { return Err(JsError::new("stack underflow (peek_at)")); }
    Ok(&frame.stack[n - 1 - depth])
}

fn atom_string(atoms: &[JsAtom], idx: usize) -> Result<String, JsError> {
    match atoms.get(idx) {
        Some(JsAtom::String(s)) => Ok(s.clone()),
        Some(JsAtom::Function(f)) => Ok(f.name.clone().unwrap_or_default()),
        Some(other) => Err(JsError::new(format!("atom #{} is not a name: {:?}", idx, other))),
        None => Err(JsError::new(format!("atom #{} out of bounds", idx))),
    }
}

/// Walk the scope chain looking up `name`. Returns None if not found.
fn resolve_name(frame: &JsFrame, name: &str) -> Option<JsValue> {
    // Local args/vars first if we have a binding table.
    if !frame.atom_to_slot.is_empty() {
        // Look at atoms — find the index for this name, then check slot
        for (i, a) in frame.atoms.iter().enumerate() {
            if let JsAtom::String(s) = a {
                if s == name {
                    if let Some(Some((kind, slot))) = frame.atom_to_slot.get(i) {
                        match kind {
                            JsBindingKind::Argument => return frame.args.get(*slot).cloned(),
                            JsBindingKind::Variable | JsBindingKind::Constant => {
                                return frame.locals.get(*slot).cloned();
                            }
                        }
                    }
                }
            }
        }
    }
    // Walk scope object then parent scopes.
    let mut current = Some(frame.scope.clone());
    while let Some(s) = current {
        let borrowed = s.borrow();
        if let Some(v) = borrowed.get_own(name) {
            return Some(v.clone());
        }
        current = borrowed.proto.clone();
    }
    // Parent (definition-time) scope chain.
    if let Some(parent) = &frame.parent_scope {
        let mut current = Some(parent.clone());
        while let Some(s) = current {
            let borrowed = s.borrow();
            if let Some(v) = borrowed.get_own(name) {
                return Some(v.clone());
            }
            current = borrowed.proto.clone();
        }
    }
    None
}

fn store_name(frame: &mut JsFrame, name: &str, value: JsValue, global: &JsObjectRef) {
    // If this name resolves to a local slot, write there.
    if !frame.atom_to_slot.is_empty() {
        for (i, a) in frame.atoms.iter().enumerate() {
            if let JsAtom::String(s) = a {
                if s == name {
                    if let Some(Some((kind, slot))) = frame.atom_to_slot.get(i).cloned() {
                        match kind {
                            JsBindingKind::Argument => {
                                if slot < frame.args.len() { frame.args[slot] = value.clone(); }
                            }
                            JsBindingKind::Variable | JsBindingKind::Constant => {
                                if slot < frame.locals.len() { frame.locals[slot] = value.clone(); }
                            }
                        }
                        return;
                    }
                }
            }
        }
    }
    // Walk current scope and outwards; first existing binding wins.
    if frame.scope.borrow().has_own(name) {
        frame.scope.borrow_mut().set_own(name, value);
        return;
    }
    let mut current = frame.parent_scope.clone();
    while let Some(s) = current {
        if s.borrow().has_own(name) {
            s.borrow_mut().set_own(name, value);
            return;
        }
        let next = s.borrow().proto.clone();
        current = next;
    }
    // Fall through: implicit global assignment.
    global.borrow_mut().set_own(name, value);
}

fn binop_num(frame: &mut JsFrame, f: fn(f64, f64) -> f64) -> Result<StepOutcome, JsError> {
    let b = pop(frame)?.to_number();
    let a = pop(frame)?.to_number();
    let r = f(a, b);
    if r == r.trunc() && r.abs() < i32::MAX as f64 && !r.is_nan() {
        frame.stack.push(JsValue::Int(r as i32));
    } else {
        frame.stack.push(JsValue::Number(r));
    }
    Ok(StepOutcome::Continue)
}

fn binop_int(frame: &mut JsFrame, f: fn(i32, i32) -> i32) -> Result<StepOutcome, JsError> {
    let b = pop(frame)?.to_int32();
    let a = pop(frame)?.to_int32();
    frame.stack.push(JsValue::Int(f(a, b)));
    Ok(StepOutcome::Continue)
}

fn cmpop(frame: &mut JsFrame, pred: fn(std::cmp::Ordering) -> bool) -> Result<StepOutcome, JsError> {
    let b = pop(frame)?;
    let a = pop(frame)?;
    let order = compare(&a, &b);
    frame.stack.push(JsValue::Bool(pred(order)));
    Ok(StepOutcome::Continue)
}

fn compare(a: &JsValue, b: &JsValue) -> std::cmp::Ordering {
    // String/string lexicographic, otherwise numeric.
    if let (JsValue::String(sa), JsValue::String(sb)) = (a, b) {
        return (**sa).cmp(&**sb);
    }
    let na = a.to_number();
    let nb = b.to_number();
    if na.is_nan() || nb.is_nan() {
        std::cmp::Ordering::Less // matches JS "all comparisons with NaN return false"
    } else {
        na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
    }
}

/// Test-only re-export of get_property (private otherwise).
#[cfg(test)]
pub fn get_property_pub(obj: &JsValue, name: &str) -> JsValue { get_property(obj, name) }

fn get_property(obj: &JsValue, name: &str) -> JsValue {
    match obj {
        JsValue::Object(o) => {
            let b = o.borrow();
            if let Some(v) = b.get_own(name) { return v.clone(); }
            if let Some(p) = b.proto.clone() {
                drop(b);
                return get_property(&JsValue::Object(p), name);
            }
            JsValue::Undefined
        }
        JsValue::Array(a) => {
            let b = a.borrow();
            if name == "length" { return JsValue::Int(b.items.len() as i32); }
            if let Ok(i) = name.parse::<usize>() {
                if i < b.items.len() { return b.items[i].clone(); }
            }
            drop(b);
            // Array.prototype.* methods, dispatched as bound natives.
            if let Some(m) = array_method(a.clone(), name) {
                return m;
            }
            JsValue::Undefined
        }
        JsValue::String(s) => {
            if name == "length" { return JsValue::Int(s.chars().count() as i32); }
            // Numeric indexing on strings returns a 1-char string per ECMA-262.
            if let Ok(i) = name.parse::<usize>() {
                return s.chars().nth(i)
                    .map(|c| JsValue::String(Rc::new(c.to_string())))
                    .unwrap_or(JsValue::Undefined);
            }
            if let Some(m) = string_method(s.clone(), name) {
                return m;
            }
            JsValue::Undefined
        }
        _ => JsValue::Undefined,
    }
}

/// Return a bound Native for an Array.prototype method, or None if unknown.
fn array_method(arr: super::value::JsArrayRef, name: &str) -> Option<JsValue> {
    use super::value::NativeFn;
    macro_rules! bind {
        ($n:expr, $f:expr) => {
            Some(JsValue::Native(Rc::new(NativeFn {
                name: $n,
                call: Box::new($f),
            })))
        };
    }
    match name {
        "push" => {
            let a = arr.clone();
            bind!("push", move |args| {
                let mut b = a.borrow_mut();
                for v in args { b.items.push(v.clone()); }
                Ok(JsValue::Int(b.items.len() as i32))
            })
        }
        "pop" => {
            let a = arr.clone();
            bind!("pop", move |_| {
                Ok(a.borrow_mut().items.pop().unwrap_or(JsValue::Undefined))
            })
        }
        "shift" => {
            let a = arr.clone();
            bind!("shift", move |_| {
                let mut b = a.borrow_mut();
                if b.items.is_empty() { Ok(JsValue::Undefined) } else { Ok(b.items.remove(0)) }
            })
        }
        "unshift" => {
            let a = arr.clone();
            bind!("unshift", move |args| {
                let mut b = a.borrow_mut();
                for (i, v) in args.iter().enumerate() {
                    b.items.insert(i, v.clone());
                }
                Ok(JsValue::Int(b.items.len() as i32))
            })
        }
        "join" => {
            let a = arr.clone();
            bind!("join", move |args| {
                let sep = args.get(0).map(|v| v.to_string()).unwrap_or_else(|| ",".into());
                let b = a.borrow();
                Ok(JsValue::String(Rc::new(
                    b.items.iter().map(|v| match v {
                        JsValue::Undefined | JsValue::Null => String::new(),
                        other => other.to_string(),
                    }).collect::<Vec<_>>().join(&sep)
                )))
            })
        }
        "reverse" => {
            let a = arr.clone();
            bind!("reverse", move |_| {
                a.borrow_mut().items.reverse();
                Ok(JsValue::Array(a.clone()))
            })
        }
        "slice" => {
            let a = arr.clone();
            bind!("slice", move |args| {
                let b = a.borrow();
                let len = b.items.len() as i32;
                let mut start = args.get(0).map(|v| v.to_int32()).unwrap_or(0);
                let mut end = args.get(1).map(|v| v.to_int32()).unwrap_or(len);
                if start < 0 { start = (len + start).max(0); }
                if end < 0 { end = (len + end).max(0); }
                let start = start.min(len) as usize;
                let end = end.min(len) as usize;
                let out: Vec<JsValue> = if start < end { b.items[start..end].to_vec() } else { Vec::new() };
                Ok(JsValue::Array(Rc::new(std::cell::RefCell::new(super::value::JsArray { items: out }))))
            })
        }
        "concat" => {
            let a = arr.clone();
            bind!("concat", move |args| {
                let mut out = a.borrow().items.clone();
                for v in args {
                    if let JsValue::Array(other) = v {
                        out.extend(other.borrow().items.iter().cloned());
                    } else {
                        out.push(v.clone());
                    }
                }
                Ok(JsValue::Array(Rc::new(std::cell::RefCell::new(super::value::JsArray { items: out }))))
            })
        }
        "indexOf" => {
            let a = arr.clone();
            bind!("indexOf", move |args| {
                let needle = args.get(0).cloned().unwrap_or(JsValue::Undefined);
                let b = a.borrow();
                for (i, v) in b.items.iter().enumerate() {
                    if loose_equal_pub(v, &needle) {
                        return Ok(JsValue::Int(i as i32));
                    }
                }
                Ok(JsValue::Int(-1))
            })
        }
        "sort" => {
            let a = arr.clone();
            bind!("sort", move |_args| {
                let mut b = a.borrow_mut();
                b.items.sort_by(|x, y| x.to_string().cmp(&y.to_string()));
                drop(b);
                Ok(JsValue::Array(a.clone()))
            })
        }
        _ => None,
    }
}

fn string_method(s: Rc<String>, name: &str) -> Option<JsValue> {
    use super::value::NativeFn;
    macro_rules! bind {
        ($n:expr, $f:expr) => {
            Some(JsValue::Native(Rc::new(NativeFn { name: $n, call: Box::new($f) })))
        };
    }
    match name {
        "charAt" => {
            let s = s.clone();
            bind!("charAt", move |args| {
                let i = args.get(0).map(|v| v.to_int32()).unwrap_or(0);
                let ch = if i < 0 { None } else { s.chars().nth(i as usize) };
                Ok(JsValue::String(Rc::new(ch.map(|c| c.to_string()).unwrap_or_default())))
            })
        }
        "charCodeAt" => {
            let s = s.clone();
            bind!("charCodeAt", move |args| {
                let i = args.get(0).map(|v| v.to_int32()).unwrap_or(0);
                if i < 0 { return Ok(JsValue::Number(f64::NAN)); }
                Ok(match s.chars().nth(i as usize) {
                    Some(c) => JsValue::Int(c as i32),
                    None => JsValue::Number(f64::NAN),
                })
            })
        }
        "indexOf" => {
            let s = s.clone();
            bind!("indexOf", move |args| {
                let needle = args.get(0).map(|v| v.to_string()).unwrap_or_default();
                let start = args.get(1).map(|v| v.to_int32()).unwrap_or(0).max(0) as usize;
                let hay: String = s.chars().skip(start).collect();
                match hay.find(&needle) {
                    Some(byte_pos) => {
                        let char_idx = hay[..byte_pos].chars().count();
                        Ok(JsValue::Int((start + char_idx) as i32))
                    }
                    None => Ok(JsValue::Int(-1)),
                }
            })
        }
        "lastIndexOf" => {
            let s = s.clone();
            bind!("lastIndexOf", move |args| {
                let needle = args.get(0).map(|v| v.to_string()).unwrap_or_default();
                match s.rfind(&needle) {
                    Some(byte_pos) => Ok(JsValue::Int(s[..byte_pos].chars().count() as i32)),
                    None => Ok(JsValue::Int(-1)),
                }
            })
        }
        "slice" | "substring" => {
            let s = s.clone();
            bind!("slice", move |args| {
                let chars: Vec<char> = s.chars().collect();
                let len = chars.len() as i32;
                let mut start = args.get(0).map(|v| v.to_int32()).unwrap_or(0);
                let mut end = args.get(1).map(|v| v.to_int32()).unwrap_or(len);
                if start < 0 { start = (len + start).max(0); }
                if end < 0 { end = (len + end).max(0); }
                let start = start.min(len) as usize;
                let end = end.min(len) as usize;
                let (a, b) = if start <= end { (start, end) } else { (end, start) };
                Ok(JsValue::String(Rc::new(chars[a..b].iter().collect())))
            })
        }
        "substr" => {
            let s = s.clone();
            bind!("substr", move |args| {
                let chars: Vec<char> = s.chars().collect();
                let len = chars.len() as i32;
                let mut start = args.get(0).map(|v| v.to_int32()).unwrap_or(0);
                let count = args.get(1).map(|v| v.to_int32()).unwrap_or(len);
                if start < 0 { start = (len + start).max(0); }
                let start = start.min(len) as usize;
                let end = (start as i32 + count.max(0)).min(len) as usize;
                Ok(JsValue::String(Rc::new(chars[start..end].iter().collect())))
            })
        }
        "toUpperCase" => { let s = s.clone(); bind!("toUpperCase", move |_| Ok(JsValue::String(Rc::new(s.to_uppercase())))) }
        "toLowerCase" => { let s = s.clone(); bind!("toLowerCase", move |_| Ok(JsValue::String(Rc::new(s.to_lowercase())))) }
        "split" => {
            let s = s.clone();
            bind!("split", move |args| {
                let sep = match args.get(0) {
                    Some(v) => v.to_string(),
                    None => return Ok(JsValue::Array(Rc::new(std::cell::RefCell::new(
                        super::value::JsArray { items: vec![JsValue::String(s.clone())] }
                    )))),
                };
                let items: Vec<JsValue> = if sep.is_empty() {
                    s.chars().map(|c| JsValue::String(Rc::new(c.to_string()))).collect()
                } else {
                    s.split(&sep).map(|p| JsValue::String(Rc::new(p.to_string()))).collect()
                };
                Ok(JsValue::Array(Rc::new(std::cell::RefCell::new(super::value::JsArray { items }))))
            })
        }
        "replace" => {
            let s = s.clone();
            bind!("replace", move |args| {
                let from = args.get(0).map(|v| v.to_string()).unwrap_or_default();
                let to = args.get(1).map(|v| v.to_string()).unwrap_or_default();
                Ok(JsValue::String(Rc::new(s.replacen(&from, &to, 1))))
            })
        }
        "concat" => {
            let s = s.clone();
            bind!("concat", move |args| {
                let mut out = (*s).clone();
                for v in args { out.push_str(&v.to_string()); }
                Ok(JsValue::String(Rc::new(out)))
            })
        }
        "toString" | "valueOf" => {
            let s = s.clone();
            bind!("toString", move |_| Ok(JsValue::String(s.clone())))
        }
        _ => None,
    }
}

// Pub wrapper so array_method can call it from outside the impl block.
fn loose_equal_pub(a: &JsValue, b: &JsValue) -> bool {
    loose_equal(a, b)
}

fn set_property(obj: &JsValue, name: &str, value: JsValue) -> Result<(), JsError> {
    match obj {
        JsValue::Object(o) => { o.borrow_mut().set_own(name, value); Ok(()) }
        JsValue::Array(a) => {
            if name == "length" {
                let n = value.to_int32().max(0) as usize;
                let mut b = a.borrow_mut();
                b.items.resize(n, JsValue::Undefined);
                return Ok(());
            }
            if let Ok(i) = name.parse::<usize>() {
                let mut b = a.borrow_mut();
                if i >= b.items.len() { b.items.resize(i + 1, JsValue::Undefined); }
                b.items[i] = value;
                return Ok(());
            }
            // Non-array-index keys (negative numbers, non-numeric strings).
            // ECMA-262 makes these legitimate string properties on the array
            // object. We don't currently store them anywhere (our JsArray is
            // a flat Vec); silently drop the write so a movie that does
            // `arr[-1] = x` in an off-by-one loop doesn't error out --
            // the value isn't observable later, but the code that wrote it
            // wasn't planning to read it back either.
            Ok(())
        }
        _ => Err(JsError::new(format!("cannot set property on {:?}", obj))),
    }
}

fn get_element(obj: &JsValue, key: &JsValue) -> JsValue {
    let name = key.to_string();
    get_property(obj, &name)
}

fn set_element(obj: &JsValue, key: &JsValue, value: JsValue) -> Result<(), JsError> {
    let name = key.to_string();
    set_property(obj, &name, value)
}

fn incdec_name(
    frame: &mut JsFrame,
    global: &JsObjectRef,
    operand: &[u8],
    delta: i32,
    post: bool,
) -> Result<StepOutcome, JsError> {
    let idx = read_u16_operand(operand).map_err(JsError::new)? as usize;
    let name = atom_string(&frame.atoms, idx)?;
    let old = resolve_name(frame, &name).unwrap_or(JsValue::Int(0));
    let n = old.to_number();
    let new_n = n + delta as f64;
    let new_v = if new_n == new_n.trunc() && new_n.abs() < i32::MAX as f64 {
        JsValue::Int(new_n as i32)
    } else {
        JsValue::Number(new_n)
    };
    store_name(frame, &name, new_v.clone(), global);
    frame.stack.push(if post {
        if n == n.trunc() && n.abs() < i32::MAX as f64 { JsValue::Int(n as i32) } else { JsValue::Number(n) }
    } else {
        new_v
    });
    Ok(StepOutcome::Continue)
}

fn incdec_slot(
    frame: &mut JsFrame,
    operand: &[u8],
    delta: i32,
    is_arg: bool,
    post: bool,
) -> Result<StepOutcome, JsError> {
    let slot = read_u16_operand(operand).map_err(JsError::new)? as usize;
    let kind = if is_arg { "arg" } else { "var" };
    let slots = if is_arg { &mut frame.args } else { &mut frame.locals };
    if slot >= slots.len() {
        return Err(JsError::new(format!("{}-slot {} out of bounds (len={})", kind, slot, slots.len())));
    }
    let old_n = slots[slot].to_number();
    let new_n = old_n + delta as f64;
    let new_v = num_to_value(new_n);
    slots[slot] = new_v.clone();
    frame.stack.push(if post { num_to_value(old_n) } else { new_v });
    Ok(StepOutcome::Continue)
}

fn incdec_prop(
    frame: &mut JsFrame,
    operand: &[u8],
    delta: i32,
    post: bool,
) -> Result<StepOutcome, JsError> {
    let idx = read_u16_operand(operand).map_err(JsError::new)? as usize;
    let name = atom_string(&frame.atoms, idx)?;
    let obj = pop(frame)?;
    let old = get_property(&obj, &name);
    let old_n = old.to_number();
    let new_v = num_to_value(old_n + delta as f64);
    set_property(&obj, &name, new_v.clone())?;
    frame.stack.push(if post { num_to_value(old_n) } else { new_v });
    Ok(StepOutcome::Continue)
}

fn incdec_elem(frame: &mut JsFrame, delta: i32, post: bool) -> Result<StepOutcome, JsError> {
    let key = pop(frame)?;
    let obj = pop(frame)?;
    let old = get_element(&obj, &key);
    let old_n = old.to_number();
    let new_v = num_to_value(old_n + delta as f64);
    set_element(&obj, &key, new_v.clone())?;
    frame.stack.push(if post { num_to_value(old_n) } else { new_v });
    Ok(StepOutcome::Continue)
}

/// ECMA-262 abstract equality (loose ==). Used by CASE, JSOP_LOOKUPSWITCH,
/// and JSOP_NEW_EQ for v1.4+ scripts.
fn loose_equal(a: &JsValue, b: &JsValue) -> bool {
    use JsValue::*;
    match (a, b) {
        (Undefined, Undefined) | (Null, Null) | (Undefined, Null) | (Null, Undefined) => true,
        (Bool(x), Bool(y)) => x == y,
        (Int(x), Int(y)) => x == y,
        (Number(x), Number(y)) => x == y,
        (String(x), String(y)) => **x == **y,
        (Int(_), Number(_)) | (Number(_), Int(_))
        | (Int(_), String(_)) | (String(_), Int(_))
        | (Number(_), String(_)) | (String(_), Number(_)) => a.to_number() == b.to_number(),
        (Bool(_), _) | (_, Bool(_)) => a.to_number() == b.to_number(),
        // Object identity comparison via Rc::ptr_eq.
        (Object(x), Object(y)) => Rc::ptr_eq(x, y),
        (Array(x), Array(y)) => Rc::ptr_eq(x, y),
        (Function(x), Function(y)) => Rc::ptr_eq(x, y),
        (Native(x), Native(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

fn atom_to_value(atoms: &[JsAtom], idx: usize) -> Result<JsValue, JsError> {
    let a = atoms.get(idx).ok_or_else(|| JsError::new("atom_to_value: oob"))?;
    Ok(match a {
        JsAtom::Null => JsValue::Null,
        JsAtom::Void => JsValue::Undefined,
        JsAtom::Bool(b) => JsValue::Bool(*b),
        JsAtom::Int(i) => JsValue::Int(*i),
        JsAtom::Double(d) => JsValue::Number(*d),
        JsAtom::String(s) => JsValue::String(Rc::new(s.clone())),
        JsAtom::Function(fa) => JsValue::Function(Rc::new(JsFunction { atom: Rc::new((**fa).clone()) })),
        JsAtom::Unsupported(_) => JsValue::Undefined,
    })
}

fn num_to_value(n: f64) -> JsValue {
    if n == n.trunc() && n.abs() < i32::MAX as f64 && !n.is_nan() {
        JsValue::Int(n as i32)
    } else {
        JsValue::Number(n)
    }
}

/// Build a fresh frame for invoking a JsFunction. Args overflow / underflow
/// follows SpiderMonkey: extra args dropped, missing args filled with undefined.
fn build_function_frame(
    f: &JsFunctionRef,
    args_in: Vec<JsValue>,
    this_value: JsValue,
    program_scope: JsObjectRef,
) -> JsFrame {
    let atom = &f.atom;
    let nargs = atom.nargs as usize;
    let nvars = atom.nvars as usize;
    let mut args = Vec::with_capacity(nargs);
    for i in 0..nargs {
        args.push(args_in.get(i).cloned().unwrap_or(JsValue::Undefined));
    }
    let locals = vec![JsValue::Undefined; nvars];
    let atom_to_slot = build_atom_slot_map(&atom.script.atoms, &atom.bindings);

    JsFrame {
        bytecode: Rc::new(atom.script.bytecode.clone()),
        atoms: Rc::new(atom.script.atoms.clone()),
        args,
        locals,
        scope: Rc::new(RefCell::new(JsObject::new())),
        parent_scope: Some(program_scope),
        this_value,
        pc: 0,
        stack: Vec::new(),
        rval: JsValue::Undefined,
        atom_to_slot,
    }
}

/// Index atoms by their string name and join with the function's
/// JsFunctionBinding list. The result is "for atom[i], what kind/slot is it?".
///
/// `short_id` in the XDR binding record is a property hash bucket — NOT the
/// slot index. The actual arg/var slot is the binding's POSITION among
/// same-kind bindings (the order spvec[] was filled by the SpiderMonkey
/// encoder, which is the order the bytecode's getarg/getvar operands use).
fn build_atom_slot_map(atoms: &[JsAtom], bindings: &[JsFunctionBinding]) -> Vec<Option<(JsBindingKind, usize)>> {
    let mut map = vec![None; atoms.len()];
    let mut arg_slot = 0usize;
    let mut var_slot = 0usize;
    for b in bindings {
        let slot = match b.kind {
            JsBindingKind::Argument => { let s = arg_slot; arg_slot += 1; s }
            JsBindingKind::Variable | JsBindingKind::Constant => { let s = var_slot; var_slot += 1; s }
        };
        for (i, a) in atoms.iter().enumerate() {
            if let JsAtom::String(s) = a {
                if s == &b.name {
                    map[i] = Some((b.kind, slot));
                }
            }
        }
    }
    map
}

