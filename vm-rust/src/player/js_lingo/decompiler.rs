// JavaScript decompiler for JsScriptIR.
//
// Translates SpiderMonkey 1.5 bytecode back into readable JS source.
// We don't try to reproduce the *exact* author source — comments and
// whitespace are gone — but we recover statements, expressions, and the
// most common control-flow shapes. Output goes into a `DecompiledScript`
// that mirrors the structure existing `decompiler::DecompiledHandler`
// produces for Lingo, so the cast inspector reuses the same renderer.
//
// Reference: jsdmx/src/jsopcode.c::Decompile (the same approach in C —
// a "sprint stack" of source-expression strings + per-opcode rewrite
// rules). We don't yet consume source notes, so we miss some hints
// (semicolons placement, comma operator), but the visible structure
// is unambiguous from the bytecode + atom map alone.

use super::opcodes::{JsOp, JsOpFormat};
use super::variable_length::{read_i16_operand, read_u16_operand};
use super::xdr::{iter_ops, JsAtom, JsBindingKind, JsFunctionAtom, JsInstruction, JsScriptIR};

/// A single decoded statement line plus the bytecode offsets it came from
/// (so the existing breakpoint UI can map line ↔ bc index).
#[derive(Debug, Clone)]
pub struct DecompLine {
    pub text: String,
    pub indent: u32,
    /// Indices into the iter_ops() walk that produced this line.
    pub bytecode_indices: Vec<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct DecompiledScript {
    pub lines: Vec<DecompLine>,
    /// bytecode_index → line_index
    pub bytecode_to_line: Vec<(usize, usize)>,
}

/// Decompile a script body. `bindings` is the function's local/arg binding
/// list (empty for top-level program). Argument names are surfaced for
/// `Getarg`, locals for `Getvar` / `Setvar`.
pub fn decompile(ir: &JsScriptIR, bindings: &[super::xdr::JsFunctionBinding]) -> DecompiledScript {
    let mut state = DecompState::new(ir, bindings);
    state.run();
    state.into_result()
}

// ===== Sprint-stack decompiler =====

struct DecompState<'a> {
    ir: &'a JsScriptIR,
    bindings: &'a [super::xdr::JsFunctionBinding],
    /// Pre-decoded instruction stream (with offsets) so we can walk
    /// forwards/backwards for control-flow recognition.
    instructions: Vec<DecodedIns>,
    /// Map from bytecode offset → index in `instructions`. Used to resolve
    /// jump targets to instruction indices.
    offset_to_idx: std::collections::HashMap<usize, usize>,
    /// Operand-expression stack — pushes hold strings (the source form of
    /// a JS expression). Each entry tracks which instruction(s) produced it.
    stack: Vec<StackEntry>,
    /// Emitted statements.
    out: Vec<DecompLine>,
    /// Indentation depth (in 2-space units).
    indent: u32,
    /// Map of bytecode-index → emitted-line-index.
    bc_to_line: Vec<(usize, usize)>,
    /// Pre-computed: at which instruction indices does a block end?
    /// (For if/else/while/for recognition.) Filled lazily.
    block_ends: std::collections::HashMap<usize, BlockKind>,
}

#[derive(Debug, Clone)]
struct DecodedIns {
    offset: usize,
    op: JsOp,
    operand: Vec<u8>,
    length: usize,
}

#[derive(Debug, Clone)]
struct StackEntry {
    text: String,
    /// Operator precedence — higher = tighter binding. Used to decide
    /// when to add parentheses around sub-expressions.
    prec: u8,
    /// Bytecode indices that contributed to this expression.
    bc_idx: Vec<usize>,
}

#[derive(Debug, Clone, Copy)]
enum BlockKind {
    IfThen,
    Else,
    Loop,
}

/// Result of analysing a forward branch (IFEQ / IFNE) for structured control
/// flow. Indices are into `instructions[]`.
#[derive(Debug, Clone)]
enum Region {
    /// `if (cond) { then }` -- terminates at `cont_idx` (no else block).
    IfOnly { then_range: (usize, usize), cont_idx: usize },
    /// `if (cond) { then } else { else }` -- the `then` ends with a GOTO that
    /// jumps past the else block to `cont_idx`.
    IfElse {
        then_range: (usize, usize),
        else_range: (usize, usize),
        cont_idx: usize,
    },
    /// `for (init; cond; step) { body }` or `while (cond) { body }`. Body is
    /// `body_range`; if `step_range` is Some, it's the trailing expression
    /// of the body that's emitted in the for-header instead of inline.
    Loop {
        body_range: (usize, usize),
        step_range: Option<(usize, usize)>,
        cont_idx: usize,
    },
}

impl<'a> DecompState<'a> {
    fn new(ir: &'a JsScriptIR, bindings: &'a [super::xdr::JsFunctionBinding]) -> Self {
        let mut instructions = Vec::new();
        let mut offset_to_idx = std::collections::HashMap::new();
        for (i, ins) in iter_ops(&ir.bytecode).enumerate() {
            match ins {
                Ok(JsInstruction { offset, op, operand, length }) => {
                    offset_to_idx.insert(offset, i);
                    instructions.push(DecodedIns { offset, op, operand: operand.to_vec(), length });
                }
                Err(_) => break,
            }
        }
        // Sentinel: a jump that targets one byte past the last instruction is
        // really `end of script`. Map that virtual offset to instructions.len().
        let end_offset = instructions.last().map(|i| i.offset + i.length).unwrap_or(0);
        offset_to_idx.insert(end_offset, instructions.len());
        Self {
            ir,
            bindings,
            instructions,
            offset_to_idx,
            stack: Vec::new(),
            out: Vec::new(),
            indent: 0,
            bc_to_line: Vec::new(),
            block_ends: std::collections::HashMap::new(),
        }
    }

    fn into_result(self) -> DecompiledScript {
        DecompiledScript {
            lines: self.out,
            bytecode_to_line: self.bc_to_line,
        }
    }

    fn run(&mut self) {
        // First emit function declarations and var declarations from the
        // prologue range so the source reads top-down.
        let prolog_end = self.ir.prolog_length as usize;
        let mut i = 0usize;
        while i < self.instructions.len() {
            if self.instructions[i].offset >= prolog_end {
                break;
            }
            i = self.step(i);
        }
        // Body: emit instructions in [i, len) with structured control flow.
        let end = self.instructions.len();
        self.emit_range(i, end);

        // Drain any leftover expressions on stack as expression-statements.
        while let Some(e) = self.stack.pop() {
            if !e.text.starts_with('<') {
                self.emit_line_with_idx(format!("{};", e.text), e.bc_idx);
            }
        }
    }

    /// Emit instructions in [start, end), recognising structured control-flow
    /// patterns and emitting `if`/`if-else`/`for`/`while` wrappers as we go.
    fn emit_range(&mut self, mut i: usize, end: usize) {
        while i < end {
            if let Some(region) = self.classify_branch(i, end) {
                match region {
                    Region::IfElse { then_range, else_range, cont_idx } => {
                        let cond = self.consume_cond_at(i);
                        self.emit_line_with_idx(format!("if ({}) {{", cond.text), cond.bc_idx);
                        self.indent += 1;
                        self.emit_range(then_range.0, then_range.1);
                        self.indent -= 1;
                        self.emit_line_with_idx("} else {".into(), vec![]);
                        self.indent += 1;
                        self.emit_range(else_range.0, else_range.1);
                        self.indent -= 1;
                        self.emit_line_with_idx("}".into(), vec![]);
                        i = cont_idx;
                    }
                    Region::IfOnly { then_range, cont_idx } => {
                        let cond = self.consume_cond_at(i);
                        self.emit_line_with_idx(format!("if ({}) {{", cond.text), cond.bc_idx);
                        self.indent += 1;
                        self.emit_range(then_range.0, then_range.1);
                        self.indent -= 1;
                        self.emit_line_with_idx("}".into(), vec![]);
                        i = cont_idx;
                    }
                    Region::Loop { body_range, step_range, cont_idx } => {
                        // The loop body precedes the IFEQ. We've already
                        // emitted the loop's "init" as plain statements (the
                        // SETVAR before pc i). Now we need to recognise the
                        // for-loop init pattern: the immediately-preceding
                        // emitted lines that look like `<localvar> = <expr>;`.
                        let cond = self.consume_cond_at(i);
                        let (init_text, init_bc) = self.steal_for_init(body_range.0);
                        let step_text = if let Some(sr) = step_range {
                            // Render the step expression(s) without emitting them inline.
                            self.render_inline(sr.0, sr.1)
                        } else {
                            String::new()
                        };
                        let header = if !init_text.is_empty() && !step_text.is_empty() {
                            format!("for ({}; {}; {}) {{", init_text, cond.text, step_text)
                        } else if !step_text.is_empty() {
                            format!("for (; {}; {}) {{", cond.text, step_text)
                        } else {
                            format!("while ({}) {{", cond.text)
                        };
                        let mut bc = init_bc;
                        bc.extend(cond.bc_idx);
                        self.emit_line_with_idx(header, bc);
                        self.indent += 1;
                        let body_emit_end = match step_range {
                            Some((s, _)) => s,
                            None => body_range.1,
                        };
                        self.emit_range(body_range.0, body_emit_end);
                        self.indent -= 1;
                        self.emit_line_with_idx("}".into(), vec![]);
                        i = cont_idx;
                    }
                }
            } else {
                // Skip trailing NOPs at the very end of the function — they
                // are SpiderMonkey's bytecode-alignment padding and aren't
                // part of the source.
                if matches!(self.instructions[i].op, JsOp::Nop) && self.only_nops_remain(i, end) {
                    break;
                }
                i = self.step(i);
            }
        }
    }

    fn only_nops_remain(&self, start: usize, end: usize) -> bool {
        (start..end).all(|i| matches!(self.instructions[i].op, JsOp::Nop))
    }

    /// Look at the branch at instruction index `i` and decide whether it
    /// shapes into an `if-else`, `if`-only, or a loop. Returns None for an
    /// unrecognised branch (in which case `step` falls back to the comment
    /// representation).
    fn classify_branch(&self, i: usize, end: usize) -> Option<Region> {
        let ins = &self.instructions[i];
        let (is_if, _is_inverted) = match ins.op {
            JsOp::Ifeq | JsOp::Ifeqx => (true, false),
            JsOp::Ifne | JsOp::Ifnex => (true, true),
            _ => (false, false),
        };
        if !is_if { return None; }

        let delta = if matches!(ins.op, JsOp::Ifeqx | JsOp::Ifnex) {
            super::variable_length::read_i32_operand(&ins.operand).ok()?
        } else {
            read_i16_operand(&ins.operand).ok()? as i32
        };
        let target_off = (ins.offset as i32 + delta) as usize;
        let target_idx = *self.offset_to_idx.get(&target_off)?;

        if target_idx <= i || target_idx > end {
            return None; // backward branch or out of range
        }

        // Look at the instruction immediately before the target (if any).
        if target_idx >= 1 {
            let prev_idx = target_idx - 1;
            let prev = &self.instructions[prev_idx];
            if matches!(prev.op, JsOp::Goto | JsOp::Gotox) {
                let prev_delta = if prev.op == JsOp::Gotox {
                    super::variable_length::read_i32_operand(&prev.operand).ok()?
                } else {
                    read_i16_operand(&prev.operand).ok()? as i32
                };
                let prev_target_off = (prev.offset as i32 + prev_delta) as usize;
                if let Some(&prev_target_idx) = self.offset_to_idx.get(&prev_target_off) {
                    if prev_target_idx > target_idx && prev_target_idx <= end {
                        // if-else: then body is [i+1, prev_idx), else body is
                        // [target_idx, prev_target_idx). Continuation = prev_target_idx.
                        return Some(Region::IfElse {
                            then_range: (i + 1, prev_idx),
                            else_range: (target_idx, prev_target_idx),
                            cont_idx: prev_target_idx,
                        });
                    } else if prev_target_idx <= i {
                        // Loop: body is [i+1, prev_idx). The GOTO at prev_idx
                        // jumps back to the loop header (where the cond starts).
                        // Try to detect a for-step: the trailing statement(s)
                        // of the body before the GOTO that contain an inc/dec
                        // of a variable used in the cond.
                        let step_range = self.detect_for_step(i + 1, prev_idx);
                        return Some(Region::Loop {
                            body_range: (i + 1, prev_idx),
                            step_range,
                            cont_idx: target_idx,
                        });
                    }
                }
            }
            // No preceding GOTO — plain if (no else, no loop).
            return Some(Region::IfOnly {
                then_range: (i + 1, target_idx),
                cont_idx: target_idx,
            });
        }
        None
    }

    /// Detect the for-step expression at the tail of a loop body. A typical
    /// SpiderMonkey emission is `... <step expr>; pop; pop` immediately before
    /// the back-GOTO, where the step is a single statement.
    ///
    /// We approximate: walk backwards from `end` over Pops; if the preceding
    /// op is a varinc/vardec/arginc/argdec or a SETVAR/SETARG/SETNAME, treat
    /// the run [step_start, end) as the step. Range stays empty for `while`.
    fn detect_for_step(&self, start: usize, end: usize) -> Option<(usize, usize)> {
        if end == 0 || end <= start { return None; }
        let mut e = end;
        // Trim trailing POPs.
        while e > start && matches!(self.instructions[e - 1].op, JsOp::Pop) {
            e -= 1;
        }
        if e == 0 || e <= start { return None; }
        let last = &self.instructions[e - 1];
        let is_stepish = matches!(last.op,
            JsOp::Varinc | JsOp::Vardec | JsOp::Arginc | JsOp::Argdec
            | JsOp::Incvar | JsOp::Decvar | JsOp::Incarg | JsOp::Decarg
            | JsOp::Setvar | JsOp::Setarg | JsOp::Setname);
        if !is_stepish { return None; }
        // Conservative: the step is just the single trailing op (plus pops). For
        // simple post-inc the entire step is one 3-byte op.
        Some((e - 1, end))
    }

    /// Consume an `if (cond)` test by running the decompiler over the bytecode
    /// that produced the cond value, then taking the IFEQ off the stack.
    ///
    /// The cond expression was emitted just before the IFEQ — it lives on
    /// `self.stack` as the latest entry. We pop it. The instructions that
    /// produced the cond have already been processed by emit_range / step,
    /// so we don't need to walk them again; we just consume the IFEQ
    /// instruction itself.
    fn consume_cond_at(&mut self, ifeq_idx: usize) -> StackEntry {
        let cond = self.pop_or_undef();
        let mut bc = cond.bc_idx.clone();
        bc.push(ifeq_idx);
        StackEntry { text: cond.text, prec: cond.prec, bc_idx: bc }
    }

    /// Steal the most-recently-emitted line as a for-loop init statement.
    /// Returns its text (without trailing `;`) and bytecode indices.
    fn steal_for_init(&mut self, body_start_idx: usize) -> (String, Vec<usize>) {
        // The init lives as a single `name = expr;` line emitted by the
        // SETVAR before the loop header.
        if let Some(last) = self.out.last() {
            if last.text.ends_with(';') && last.indent == self.indent {
                let _ = body_start_idx;
                let line = self.out.pop().unwrap();
                // Strip the trailing semicolon. Convert plain `name = expr;`
                // to a var-bearing init if it matches one of the function's
                // local bindings.
                let text = line.text.trim_end_matches(';').to_string();
                let init = if let Some(eq) = text.find(" = ") {
                    let name = &text[..eq];
                    let is_local = self.bindings.iter().any(|b| {
                        matches!(b.kind, JsBindingKind::Variable | JsBindingKind::Constant)
                            && b.name == name
                    });
                    if is_local { format!("var {}", text) } else { text }
                } else {
                    text
                };
                return (init, line.bytecode_indices);
            }
        }
        (String::new(), Vec::new())
    }

    /// Render the source for an instruction range without emitting it as
    /// statements. We re-run a fresh DecompState over the slice and join the
    /// resulting lines.
    fn render_inline(&self, start: usize, end: usize) -> String {
        // Build a child state borrowing the same ir + bindings, but limited
        // to the instructions slice.
        let mut child = DecompState {
            ir: self.ir,
            bindings: self.bindings,
            instructions: self.instructions[start..end].to_vec(),
            offset_to_idx: self.offset_to_idx.clone(),
            stack: Vec::new(),
            out: Vec::new(),
            indent: 0,
            bc_to_line: Vec::new(),
            block_ends: std::collections::HashMap::new(),
        };
        let len = child.instructions.len();
        let mut i = 0usize;
        while i < len {
            i = child.step(i);
        }
        // The "step" lines we just emitted include the trailing semicolon.
        // For an inline expression we want a single ;-joined fragment.
        child.out.into_iter().map(|l| l.text.trim_end_matches(';').to_string())
            .collect::<Vec<_>>().join(", ")
    }

    /// Process one instruction, return the next instruction index.
    fn step(&mut self, i: usize) -> usize {
        let ins = self.instructions[i].clone();
        match ins.op {
            // ===== Function and variable declarations =====
            JsOp::Defvar | JsOp::Defconst => {
                let idx = read_u16_operand(&ins.operand).unwrap_or(0) as usize;
                let name = atom_name(self.ir, idx);
                let kw = if ins.op == JsOp::Defconst { "const" } else { "var" };
                self.emit_line_with_idx(format!("{} {};", kw, name), vec![i]);
                i + 1
            }
            JsOp::Deffun => {
                let idx = read_u16_operand(&ins.operand).unwrap_or(0) as usize;
                if let Some(JsAtom::Function(fa)) = self.ir.atoms.get(idx) {
                    self.emit_function_decl(fa, vec![i]);
                } else {
                    self.emit_line_with_idx("/* deffun: unknown atom */".into(), vec![i]);
                }
                i + 1
            }
            JsOp::Deflocalfun => {
                let slot = read_u16_operand(&ins.operand[..2]).unwrap_or(0);
                let atom_idx = read_u16_operand(&ins.operand[2..]).unwrap_or(0) as usize;
                if let Some(JsAtom::Function(fa)) = self.ir.atoms.get(atom_idx) {
                    self.emit_function_decl(fa, vec![i]);
                    let _ = slot;
                }
                i + 1
            }

            // ===== Pure pushes =====
            JsOp::Zero  => { self.push("0", 100, vec![i]); i + 1 }
            JsOp::One   => { self.push("1", 100, vec![i]); i + 1 }
            JsOp::Null  => { self.push("null", 100, vec![i]); i + 1 }
            JsOp::This  => { self.push("this", 100, vec![i]); i + 1 }
            JsOp::True  => { self.push("true", 100, vec![i]); i + 1 }
            JsOp::False => { self.push("false", 100, vec![i]); i + 1 }
            JsOp::Push  => { self.push("undefined", 100, vec![i]); i + 1 }
            JsOp::Uint16 => {
                let v = read_u16_operand(&ins.operand).unwrap_or(0);
                self.push(&v.to_string(), 100, vec![i]); i + 1
            }
            JsOp::String => {
                let idx = read_u16_operand(&ins.operand).unwrap_or(0) as usize;
                let s = match self.ir.atoms.get(idx) {
                    Some(JsAtom::String(s)) => format!("{:?}", s),
                    other => format!("/* string #{} = {:?} */", idx, other),
                };
                self.push(&s, 100, vec![i]); i + 1
            }
            JsOp::Number => {
                let idx = read_u16_operand(&ins.operand).unwrap_or(0) as usize;
                let s = match self.ir.atoms.get(idx) {
                    Some(JsAtom::Int(v)) => v.to_string(),
                    Some(JsAtom::Double(v)) => format!("{}", v),
                    other => format!("/* number #{} = {:?} */", idx, other),
                };
                self.push(&s, 100, vec![i]); i + 1
            }

            // ===== Local / arg / scope reads =====
            JsOp::Getvar => {
                let slot = read_u16_operand(&ins.operand).unwrap_or(0);
                self.push(&local_name(self.bindings, slot, false), 100, vec![i]);
                i + 1
            }
            JsOp::Getarg => {
                let slot = read_u16_operand(&ins.operand).unwrap_or(0);
                self.push(&local_name(self.bindings, slot, true), 100, vec![i]);
                i + 1
            }
            JsOp::Name => {
                let idx = read_u16_operand(&ins.operand).unwrap_or(0) as usize;
                self.push(&atom_name(self.ir, idx), 100, vec![i]);
                i + 1
            }
            JsOp::Pushobj => {
                // `this` placeholder for the upcoming CALL — leave a sentinel
                // we can drop in Call.
                self.stack.push(StackEntry { text: "<this>".into(), prec: 100, bc_idx: vec![i] });
                i + 1
            }
            JsOp::Bindname => {
                // LVALUE marker — the matching SETNAME does the work. Leave a
                // sentinel so we know not to consume the matching expression
                // ahead of time.
                self.stack.push(StackEntry { text: "<bind>".into(), prec: 100, bc_idx: vec![i] });
                i + 1
            }
            JsOp::Setvar => {
                let slot = read_u16_operand(&ins.operand).unwrap_or(0);
                let target = local_name(self.bindings, slot, false);
                let val = self.pop_or_undef();
                let mut bc = val.bc_idx.clone();
                bc.push(i);
                self.push_assignment(&target, &val.text, bc);
                i + 1
            }
            JsOp::Setarg => {
                let slot = read_u16_operand(&ins.operand).unwrap_or(0);
                let target = local_name(self.bindings, slot, true);
                let val = self.pop_or_undef();
                let mut bc = val.bc_idx.clone();
                bc.push(i);
                self.push_assignment(&target, &val.text, bc);
                i + 1
            }
            JsOp::Setname => {
                let idx = read_u16_operand(&ins.operand).unwrap_or(0) as usize;
                let name = atom_name(self.ir, idx);
                let val = self.pop_or_undef();
                let _lhs = self.pop_or_undef(); // bindname marker
                let mut bc = val.bc_idx.clone();
                bc.push(i);
                self.push_assignment(&name, &val.text, bc);
                i + 1
            }

            // ===== Property / element access =====
            JsOp::Getprop => {
                let idx = read_u16_operand(&ins.operand).unwrap_or(0) as usize;
                let name = atom_name(self.ir, idx);
                let obj = self.pop_or_undef();
                let mut bc = obj.bc_idx.clone();
                bc.push(i);
                let text = format!("{}.{}", paren_if_lt(&obj, 95), name);
                self.push_entry(text, 95, bc);
                i + 1
            }
            JsOp::Setprop => {
                let idx = read_u16_operand(&ins.operand).unwrap_or(0) as usize;
                let name = atom_name(self.ir, idx);
                let val = self.pop_or_undef();
                let obj = self.pop_or_undef();
                let mut bc = obj.bc_idx.clone();
                bc.extend(&val.bc_idx);
                bc.push(i);
                let target = format!("{}.{}", paren_if_lt(&obj, 95), name);
                self.push_assignment(&target, &val.text, bc);
                i + 1
            }
            JsOp::Getelem => {
                let key = self.pop_or_undef();
                let obj = self.pop_or_undef();
                let mut bc = obj.bc_idx.clone();
                bc.extend(&key.bc_idx);
                bc.push(i);
                let text = format!("{}[{}]", paren_if_lt(&obj, 95), key.text);
                self.push_entry(text, 95, bc);
                i + 1
            }
            JsOp::Setelem => {
                let val = self.pop_or_undef();
                let key = self.pop_or_undef();
                let obj = self.pop_or_undef();
                let mut bc = obj.bc_idx.clone();
                bc.extend(&key.bc_idx);
                bc.extend(&val.bc_idx);
                bc.push(i);
                let target = format!("{}[{}]", paren_if_lt(&obj, 95), key.text);
                self.push_assignment(&target, &val.text, bc);
                i + 1
            }

            // ===== Binary operators =====
            JsOp::Add => { self.binop(" + ", 13, i); i + 1 }
            JsOp::Sub => { self.binop(" - ", 13, i); i + 1 }
            JsOp::Mul => { self.binop(" * ", 14, i); i + 1 }
            JsOp::Div => { self.binop(" / ", 14, i); i + 1 }
            JsOp::Mod => { self.binop(" % ", 14, i); i + 1 }
            JsOp::Eq  => { self.binop(" == ", 10, i); i + 1 }
            JsOp::Ne  => { self.binop(" != ", 10, i); i + 1 }
            JsOp::Lt  => { self.binop(" < ", 11, i); i + 1 }
            JsOp::Le  => { self.binop(" <= ", 11, i); i + 1 }
            JsOp::Gt  => { self.binop(" > ", 11, i); i + 1 }
            JsOp::Ge  => { self.binop(" >= ", 11, i); i + 1 }
            JsOp::NewEq => { self.binop(" === ", 10, i); i + 1 }
            JsOp::NewNe => { self.binop(" !== ", 10, i); i + 1 }
            JsOp::Bitor  => { self.binop(" | ", 7, i); i + 1 }
            JsOp::Bitxor => { self.binop(" ^ ", 8, i); i + 1 }
            JsOp::Bitand => { self.binop(" & ", 9, i); i + 1 }
            JsOp::Lsh    => { self.binop(" << ", 12, i); i + 1 }
            JsOp::Rsh    => { self.binop(" >> ", 12, i); i + 1 }
            JsOp::Ursh   => { self.binop(" >>> ", 12, i); i + 1 }

            // ===== Unary operators =====
            JsOp::Neg => { self.unop("-", 15, i); i + 1 }
            JsOp::Pos => { self.unop("+", 15, i); i + 1 }
            JsOp::Not => { self.unop("!", 15, i); i + 1 }
            JsOp::Bitnot => { self.unop("~", 15, i); i + 1 }
            JsOp::Typeof => { self.unop_named("typeof ", 15, i); i + 1 }
            JsOp::Void => { self.unop_named("void ", 15, i); i + 1 }

            // ===== Increment / decrement =====
            JsOp::Incarg | JsOp::Incvar => {
                let slot = read_u16_operand(&ins.operand).unwrap_or(0);
                let n = local_name(self.bindings, slot, ins.op == JsOp::Incarg);
                self.push_entry(format!("++{}", n), 15, vec![i]); i + 1
            }
            JsOp::Decarg | JsOp::Decvar => {
                let slot = read_u16_operand(&ins.operand).unwrap_or(0);
                let n = local_name(self.bindings, slot, ins.op == JsOp::Decarg);
                self.push_entry(format!("--{}", n), 15, vec![i]); i + 1
            }
            JsOp::Arginc | JsOp::Varinc => {
                let slot = read_u16_operand(&ins.operand).unwrap_or(0);
                let n = local_name(self.bindings, slot, ins.op == JsOp::Arginc);
                self.push_entry(format!("{}++", n), 15, vec![i]); i + 1
            }
            JsOp::Argdec | JsOp::Vardec => {
                let slot = read_u16_operand(&ins.operand).unwrap_or(0);
                let n = local_name(self.bindings, slot, ins.op == JsOp::Argdec);
                self.push_entry(format!("{}--", n), 15, vec![i]); i + 1
            }
            JsOp::Incname | JsOp::Nameinc | JsOp::Decname | JsOp::Namedec => {
                let idx = read_u16_operand(&ins.operand).unwrap_or(0) as usize;
                let n = atom_name(self.ir, idx);
                let s = match ins.op {
                    JsOp::Incname => format!("++{}", n),
                    JsOp::Nameinc => format!("{}++", n),
                    JsOp::Decname => format!("--{}", n),
                    _             => format!("{}--", n),
                };
                self.push_entry(s, 15, vec![i]); i + 1
            }

            // ===== Calls =====
            JsOp::Call => {
                let argc = read_u16_operand(&ins.operand).unwrap_or(0) as usize;
                let mut args: Vec<StackEntry> = Vec::with_capacity(argc);
                for _ in 0..argc {
                    args.push(self.pop_or_undef());
                }
                args.reverse();
                let _this_marker = self.pop_or_undef();
                let callee = self.pop_or_undef();
                let mut bc = callee.bc_idx.clone();
                for a in &args { bc.extend(&a.bc_idx); }
                bc.push(i);
                let args_str = args.iter().map(|a| a.text.as_str()).collect::<Vec<_>>().join(", ");
                let text = format!("{}({})", paren_if_lt(&callee, 95), args_str);
                self.push_entry(text, 95, bc);
                i + 1
            }
            JsOp::New => {
                let argc = read_u16_operand(&ins.operand).unwrap_or(0) as usize;
                let mut args: Vec<StackEntry> = Vec::with_capacity(argc);
                for _ in 0..argc {
                    args.push(self.pop_or_undef());
                }
                args.reverse();
                let _this = self.pop_or_undef();
                let callee = self.pop_or_undef();
                let mut bc = callee.bc_idx.clone();
                for a in &args { bc.extend(&a.bc_idx); }
                bc.push(i);
                let args_str = args.iter().map(|a| a.text.as_str()).collect::<Vec<_>>().join(", ");
                let text = format!("new {}({})", paren_if_lt(&callee, 95), args_str);
                self.push_entry(text, 95, bc);
                i + 1
            }

            // ===== Object / array literals =====
            JsOp::Newinit => {
                // Decide kind by what's at stack[-2] (the constructor NAME).
                let kind = if self.stack.len() >= 2 {
                    let nm = &self.stack[self.stack.len() - 2].text;
                    if nm == "Array" { "[]" } else { "{}" }
                } else {
                    "{}"
                };
                // Replace the constructor + `this` marker with the literal seed.
                self.stack.push(StackEntry {
                    text: format!("<newinit:{}>", kind),
                    prec: 100,
                    bc_idx: vec![i],
                });
                i + 1
            }
            JsOp::Initprop => {
                let idx = read_u16_operand(&ins.operand).unwrap_or(0) as usize;
                let key = atom_name(self.ir, idx);
                let val = self.pop_or_undef();
                // Append into the topmost <newinit:...> marker.
                if let Some(top) = self.stack.last_mut() {
                    let body = top.text.trim_start_matches("<newinit:{}>")
                                       .trim_start_matches("<newinit:[]>")
                                       .to_string();
                    top.text = if body.is_empty() {
                        format!("<newinit:{{}}>{}: {}", key, val.text)
                    } else {
                        format!("<newinit:{{}}>{}, {}: {}", body, key, val.text)
                    };
                    top.bc_idx.extend(&val.bc_idx);
                    top.bc_idx.push(i);
                }
                i + 1
            }
            JsOp::Initelem => {
                let val = self.pop_or_undef();
                let _key = self.pop_or_undef(); // numeric index — order is implied
                if let Some(top) = self.stack.last_mut() {
                    let body = top.text.trim_start_matches("<newinit:[]>")
                                       .trim_start_matches("<newinit:{}>")
                                       .to_string();
                    top.text = if body.is_empty() {
                        format!("<newinit:[]>{}", val.text)
                    } else {
                        format!("<newinit:[]>{}, {}", body, val.text)
                    };
                    top.bc_idx.extend(&val.bc_idx);
                    top.bc_idx.push(i);
                }
                i + 1
            }
            JsOp::Endinit => {
                if let Some(mut top) = self.stack.pop() {
                    // The <newinit:...> marker stays; we just finalize the
                    // surrounding brackets.
                    if let Some(rest) = top.text.strip_prefix("<newinit:[]>") {
                        top.text = format!("[{}]", rest);
                    } else if let Some(rest) = top.text.strip_prefix("<newinit:{}>") {
                        top.text = format!("{{{}}}", rest);
                    }
                    top.bc_idx.push(i);
                    // Drop the residual constructor + this markers under it.
                    // SpiderMonkey leaves them on the stack for the duration
                    // of the literal; in source they have no role.
                    if self.stack.len() >= 2 {
                        // peek last two
                        let n = self.stack.len();
                        let drop_this = self.stack[n - 1].text == "<this>" || self.stack[n - 1].text == "<bind>";
                        if drop_this {
                            self.stack.pop();
                            self.stack.pop();
                        }
                    }
                    self.stack.push(top);
                }
                i + 1
            }

            // ===== Stack management =====
            JsOp::Pop => {
                // Statement boundary. The expression on TOS is consumed as a
                // standalone statement.
                if let Some(e) = self.stack.pop() {
                    if !e.text.starts_with('<') {
                        let mut bc = e.bc_idx.clone();
                        bc.push(i);
                        self.emit_line_with_idx(format!("{};", e.text), bc);
                    }
                }
                i + 1
            }
            JsOp::Popv | JsOp::Setrval => {
                let e = self.pop_or_undef();
                let mut bc = e.bc_idx.clone();
                bc.push(i);
                self.emit_line_with_idx(format!("return {};", e.text), bc);
                i + 1
            }
            JsOp::Dup => {
                if let Some(top) = self.stack.last().cloned() {
                    self.stack.push(top);
                }
                i + 1
            }
            JsOp::Swap => {
                let n = self.stack.len();
                if n >= 2 { self.stack.swap(n - 1, n - 2); }
                i + 1
            }
            JsOp::Group | JsOp::Nop | JsOp::Pushobj if false => i + 1, // unreachable arm; kept for parity

            // ===== Control flow =====
            JsOp::Return => {
                let e = self.pop_or_undef();
                let mut bc = e.bc_idx.clone();
                bc.push(i);
                self.emit_line_with_idx(format!("return {};", e.text), bc);
                i + 1
            }
            JsOp::Retrval => {
                self.emit_line_with_idx("return;".into(), vec![i]);
                i + 1
            }
            JsOp::Goto | JsOp::Gotox | JsOp::Ifeq | JsOp::Ifeqx | JsOp::Ifne | JsOp::Ifnex => {
                // Phase 1 decompiler: just emit a labeled goto/if line as a
                // comment line. Real if/while/for reconstruction needs a
                // control-flow analysis pass which lands incrementally.
                let delta = if matches!(ins.op, JsOp::Gotox | JsOp::Ifeqx | JsOp::Ifnex) {
                    super::variable_length::read_i32_operand(&ins.operand).unwrap_or(0)
                } else {
                    read_i16_operand(&ins.operand).unwrap_or(0) as i32
                };
                let target = ins.offset as i32 + delta;
                match ins.op {
                    JsOp::Goto | JsOp::Gotox => {
                        self.emit_line_with_idx(format!("/* goto -> {} */", target), vec![i]);
                    }
                    _ => {
                        let cond = self.pop_or_undef();
                        let mut bc = cond.bc_idx.clone();
                        bc.push(i);
                        let kw = if matches!(ins.op, JsOp::Ifne | JsOp::Ifnex) { "if-true" } else { "if-false" };
                        self.emit_line_with_idx(format!("/* {} ({}) -> {} */", kw, cond.text, target), bc);
                    }
                }
                i + 1
            }
            JsOp::Throw => {
                let e = self.pop_or_undef();
                let mut bc = e.bc_idx.clone();
                bc.push(i);
                self.emit_line_with_idx(format!("throw {};", e.text), bc);
                i + 1
            }

            // ===== Anything we haven't covered: emit a comment so the line
            // mapping is preserved without crashing. =====
            other => {
                let info = other.info();
                let operand_str = match info.format {
                    JsOpFormat::Const => format!(" #{}", read_u16_operand(&ins.operand).unwrap_or(0)),
                    JsOpFormat::Uint16 | JsOpFormat::Qarg | JsOpFormat::Qvar | JsOpFormat::Local => {
                        format!(" {}", read_u16_operand(&ins.operand).unwrap_or(0))
                    }
                    JsOpFormat::Jump => format!(" {:+}", read_i16_operand(&ins.operand).unwrap_or(0)),
                    _ => String::new(),
                };
                self.emit_line_with_idx(format!("/* {}{} */", info.mnemonic, operand_str), vec![i]);
                i + 1
            }
        }
    }

    // ===== Helpers =====

    fn push(&mut self, text: &str, prec: u8, bc_idx: Vec<usize>) {
        self.stack.push(StackEntry { text: text.to_string(), prec, bc_idx });
    }

    fn push_entry(&mut self, text: String, prec: u8, bc_idx: Vec<usize>) {
        self.stack.push(StackEntry { text, prec, bc_idx });
    }

    fn pop_or_undef(&mut self) -> StackEntry {
        self.stack.pop().unwrap_or(StackEntry {
            text: "undefined".into(),
            prec: 100,
            bc_idx: Vec::new(),
        })
    }

    fn binop(&mut self, op: &str, prec: u8, i: usize) {
        let b = self.pop_or_undef();
        let a = self.pop_or_undef();
        let mut bc = a.bc_idx.clone();
        bc.extend(&b.bc_idx);
        bc.push(i);
        let text = format!("{}{}{}", paren_if_lt(&a, prec), op, paren_if_le(&b, prec));
        self.push_entry(text, prec, bc);
    }

    fn unop(&mut self, op: &str, prec: u8, i: usize) {
        let a = self.pop_or_undef();
        let mut bc = a.bc_idx.clone();
        bc.push(i);
        let text = format!("{}{}", op, paren_if_lt(&a, prec));
        self.push_entry(text, prec, bc);
    }

    fn unop_named(&mut self, op: &str, prec: u8, i: usize) {
        let a = self.pop_or_undef();
        let mut bc = a.bc_idx.clone();
        bc.push(i);
        let text = format!("{}{}", op, paren_if_lt(&a, prec));
        self.push_entry(text, prec, bc);
    }

    /// `name = expr` lands on the stack as the assignment expression's value;
    /// the matching POP turns it into a statement.
    fn push_assignment(&mut self, target: &str, value: &str, bc: Vec<usize>) {
        self.push_entry(format!("{} = {}", target, value), 3, bc);
    }

    fn emit_line_with_idx(&mut self, text: String, bc_indices: Vec<usize>) {
        let line_idx = self.out.len();
        for bc in &bc_indices {
            self.bc_to_line.push((*bc, line_idx));
        }
        self.out.push(DecompLine {
            text,
            indent: self.indent,
            bytecode_indices: bc_indices,
        });
    }

    fn emit_function_decl(&mut self, fa: &JsFunctionAtom, bc_indices: Vec<usize>) {
        let args: Vec<&str> = fa.bindings.iter()
            .filter(|b| b.kind == JsBindingKind::Argument)
            .map(|b| b.name.as_str())
            .collect();
        let name = fa.name.as_deref().unwrap_or("");
        self.emit_line_with_idx(format!("function {}({}) {{", name, args.join(", ")), bc_indices.clone());
        self.indent += 1;
        // Synthesise `var <name>;` lines for every function-local binding so the
        // body that follows reads idiomatically (SpiderMonkey hoists var decls
        // to the function prologue; the inner SETVAR ops then look like plain
        // assignments).
        for b in fa.bindings.iter().filter(|b| b.kind != JsBindingKind::Argument) {
            let kw = if b.kind == JsBindingKind::Constant { "const" } else { "var" };
            self.out.push(DecompLine {
                text: format!("{} {};", kw, b.name),
                indent: self.indent,
                bytecode_indices: Vec::new(),
            });
        }
        let sub = decompile(&fa.script, &fa.bindings);
        for line in sub.lines {
            self.out.push(DecompLine {
                text: line.text,
                indent: self.indent + line.indent,
                bytecode_indices: Vec::new(),
            });
        }
        self.indent -= 1;
        self.emit_line_with_idx("}".into(), bc_indices);
    }
}

fn atom_name(ir: &JsScriptIR, idx: usize) -> String {
    match ir.atoms.get(idx) {
        Some(JsAtom::String(s)) => s.clone(),
        Some(JsAtom::Function(f)) => f.name.clone().unwrap_or_else(|| format!("fn{}", idx)),
        _ => format!("atom{}", idx),
    }
}

fn local_name(bindings: &[super::xdr::JsFunctionBinding], slot: u16, is_arg: bool) -> String {
    // Slot number = position in bindings filtered by kind. `short_id` in the
    // XDR binding record is a per-property hash bucket id (not a slot number)
    // and shouldn't be matched against the bytecode operand.
    let want = if is_arg { JsBindingKind::Argument } else { JsBindingKind::Variable };
    let mut idx = 0u16;
    for b in bindings {
        if b.kind == want {
            if idx == slot {
                return b.name.clone();
            }
            idx += 1;
        } else if !is_arg && matches!(b.kind, JsBindingKind::Constant) {
            // CONST slots share the same numbering as VAR slots in the
            // SpiderMonkey emitter — both go through getvar/setvar.
            if idx == slot {
                return b.name.clone();
            }
            idx += 1;
        }
    }
    if is_arg { format!("arg{}", slot) } else { format!("v{}", slot) }
}

fn paren_if_lt(e: &StackEntry, parent_prec: u8) -> String {
    if e.prec < parent_prec { format!("({})", e.text) } else { e.text.clone() }
}

fn paren_if_le(e: &StackEntry, parent_prec: u8) -> String {
    if e.prec <= parent_prec { format!("({})", e.text) } else { e.text.clone() }
}
