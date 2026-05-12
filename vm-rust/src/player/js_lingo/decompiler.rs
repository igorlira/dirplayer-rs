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
    state.precompute_do_while();
    state.precompute_while_true();
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
    /// Stack of enclosing loop scopes. Used by Goto handling to translate
    /// bare jumps into `break;` / `continue;` when their target matches
    /// the current loop's continuation or header.
    loop_stack: Vec<LoopCtx>,
    /// Pre-computed do-while loops keyed by their header (first instruction
    /// of the body). Populated once at construction by scanning for the
    /// IFEQ + back-GOTO tail pattern.
    do_while_at: std::collections::HashMap<usize, DoWhileLoop>,
    /// Pre-computed `while (true) { ... }` loops keyed by header.
    /// Detected by finding backward unconditional GOTOs that aren't already
    /// the back-edge of a while/for or do-while loop.
    while_true_at: std::collections::HashMap<usize, WhileTrueLoop>,
}

#[derive(Debug, Clone, Copy)]
struct DoWhileLoop {
    header_idx: usize,
    /// IFEQ instruction at the loop tail; cond expression is what's on the
    /// stack when control reaches it.
    ifeq_idx: usize,
    /// Instruction index immediately following the back-GOTO (where break
    /// targets).
    cont_idx: usize,
}

#[derive(Debug, Clone, Copy)]
struct WhileTrueLoop {
    header_idx: usize,
    back_goto_idx: usize,
    cont_idx: usize,
}

#[derive(Debug, Clone, Copy)]
struct LoopCtx {
    /// Instruction index that re-runs the cond on each iteration (i.e. the
    /// loop's header). A `continue` jumps here.
    header_idx: usize,
    /// Instruction index immediately after the loop. A `break` jumps here.
    cont_idx: usize,
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
    /// `header_idx` is where `continue` jumps to (the cond expression's
    /// first instruction). `cont_idx` is where `break` jumps to (the
    /// instruction immediately following the loop).
    Loop {
        body_range: (usize, usize),
        step_range: Option<(usize, usize)>,
        header_idx: usize,
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
            loop_stack: Vec::new(),
            do_while_at: std::collections::HashMap::new(),
            while_true_at: std::collections::HashMap::new(),
        }
    }

    /// Pre-scan for backward-unconditional-GOTO patterns that form an
    /// infinite loop -- typically `while (true) { ... break; ... }`,
    /// `for (;;) { ... }`, or `do { ... } while (true)`. We treat them all
    /// as `while (true) {...}` since JS makes them equivalent.
    ///
    /// Skips back-edges that are already accounted for by:
    ///   - a do-while at the same tail (IFEQ at goto-1 forming the cond)
    ///   - a top-IFEQ while/for (IFEQ at the target with cont == goto+1)
    fn precompute_while_true(&mut self) {
        let mut found = std::collections::HashMap::new();
        for (k, ins) in self.instructions.iter().enumerate() {
            if !matches!(ins.op, JsOp::Goto | JsOp::Gotox) { continue; }
            let delta = if ins.op == JsOp::Gotox {
                super::variable_length::read_i32_operand(&ins.operand).unwrap_or(0)
            } else {
                read_i16_operand(&ins.operand).unwrap_or(0) as i32
            };
            if delta >= 0 { continue; }
            let target_off = (ins.offset as i32 + delta) as usize;
            let header_idx = match self.offset_to_idx.get(&target_off) { Some(&t) => t, None => continue };
            if header_idx > k { continue; }
            // do-while back-edge? (IFEQ at k-1 targets k+1, GOTO at k targets header)
            if k > 0 {
                let prev = &self.instructions[k - 1];
                if matches!(prev.op, JsOp::Ifeq | JsOp::Ifeqx) {
                    let prev_delta = if prev.op == JsOp::Ifeqx {
                        super::variable_length::read_i32_operand(&prev.operand).unwrap_or(0)
                    } else {
                        read_i16_operand(&prev.operand).unwrap_or(0) as i32
                    };
                    let prev_target_off = (prev.offset as i32 + prev_delta) as usize;
                    if let Some(&pt) = self.offset_to_idx.get(&prev_target_off) {
                        if pt == k + 1 { continue; }
                    }
                }
            }
            // while/for loop back-edge? Detect by walking the body and
            // looking for ANY forward IFEQ that targets cont (= k+1) — the
            // top-of-loop exit jump. If found, this is `while (cond) {...}`
            // (or `for(...; cond; ...){...}`), already handled by
            // classify_branch / Region::Loop.
            let cont_target = k + 1;
            let mut is_top_ifeq_loop = false;
            for inner_idx in header_idx..k {
                let cand = &self.instructions[inner_idx];
                if !matches!(cand.op, JsOp::Ifeq | JsOp::Ifeqx | JsOp::Ifne | JsOp::Ifnex) {
                    continue;
                }
                let cd = if matches!(cand.op, JsOp::Ifeqx | JsOp::Ifnex) {
                    super::variable_length::read_i32_operand(&cand.operand).unwrap_or(0)
                } else {
                    read_i16_operand(&cand.operand).unwrap_or(0) as i32
                };
                if cd <= 0 { continue; }
                let ct_off = (cand.offset as i32 + cd) as usize;
                if let Some(&ct_idx) = self.offset_to_idx.get(&ct_off) {
                    if ct_idx == cont_target {
                        is_top_ifeq_loop = true;
                        break;
                    }
                }
            }
            if is_top_ifeq_loop { continue; }
            // Otherwise this is a while-true back-edge. Innermost (highest
            // header_idx) wins per header.
            found.entry(header_idx).or_insert(WhileTrueLoop {
                header_idx,
                back_goto_idx: k,
                cont_idx: k + 1,
            });
        }
        self.while_true_at = found;
    }

    /// Scan instructions for the `IFEQ end; GOTO header` tail pattern that
    /// SpiderMonkey emits for `do { ... } while (cond)`. Each detected loop
    /// gets registered by its header instruction; emit_range later wraps
    /// the body in `do { ... } while (cond);`.
    fn precompute_do_while(&mut self) {
        let mut found = std::collections::HashMap::new();
        for (k, ins) in self.instructions.iter().enumerate() {
            if !matches!(ins.op, JsOp::Ifeq | JsOp::Ifeqx) { continue; }
            let next = match self.instructions.get(k + 1) { Some(n) => n, None => continue };
            if !matches!(next.op, JsOp::Goto | JsOp::Gotox) { continue; }
            let next_delta = if next.op == JsOp::Gotox {
                super::variable_length::read_i32_operand(&next.operand).unwrap_or(0)
            } else {
                read_i16_operand(&next.operand).unwrap_or(0) as i32
            };
            let next_target = (next.offset as i32 + next_delta) as usize;
            let header_idx = match self.offset_to_idx.get(&next_target) { Some(&t) => t, None => continue };
            if header_idx > k { continue; } // not backward → not a do-while
            // Only the innermost (largest header_idx) loop wins for each header.
            found.entry(header_idx).or_insert(DoWhileLoop {
                header_idx,
                ifeq_idx: k,
                cont_idx: k + 2,
            });
        }
        self.do_while_at = found;
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
    /// patterns and emitting `if`/`if-else`/`for`/`while`/`do-while` wrappers
    /// as we go.
    fn emit_range(&mut self, mut i: usize, end: usize) {
        while i < end {
            // While-true header (infinite loop with explicit breaks).
            if let Some(wt) = self.while_true_at.remove(&i) {
                if wt.cont_idx <= end + 1 {
                    self.emit_line_with_idx("while (true) {".into(), vec![]);
                    self.indent += 1;
                    self.loop_stack.push(LoopCtx {
                        header_idx: wt.header_idx,
                        cont_idx: wt.cont_idx,
                    });
                    self.emit_range(i, wt.back_goto_idx);
                    self.loop_stack.pop();
                    self.indent -= 1;
                    self.emit_line_with_idx("}".into(), vec![]);
                    i = wt.cont_idx;
                    continue;
                }
            }
            // Do-while header: wrap [i, ifeq_idx) in `do { ... } while (cond);`.
            if let Some(dw) = self.do_while_at.remove(&i) {
                if dw.cont_idx <= end + 1 {  // sanity: don't escape range
                    self.emit_line_with_idx("do {".into(), vec![]);
                    self.indent += 1;
                    self.loop_stack.push(LoopCtx {
                        header_idx: dw.header_idx,
                        cont_idx: dw.cont_idx,
                    });
                    self.emit_range(i, dw.ifeq_idx);
                    let cond = self.pop_or_undef();
                    self.loop_stack.pop();
                    self.indent -= 1;
                    let mut bc = cond.bc_idx.clone();
                    bc.push(dw.ifeq_idx);
                    self.emit_line_with_idx(format!("}} while ({});", cond.text), bc);
                    i = dw.cont_idx;
                    continue;
                }
            }
            if let Some(region) = self.classify_branch(i, end) {
                match region {
                    Region::IfElse { then_range, else_range, cont_idx } => {
                        // Ternary `?:` and `if-else` compile to the same
                        // opcode shape. Distinguish by trying to render both
                        // branches as pure expressions; if both succeed
                        // (each branch pushes one value with no statements),
                        // emit as a ternary expression on the stack rather
                        // than an if-else statement.
                        if let (Some(then_e), Some(else_e)) = (
                            self.render_branch_as_expr(then_range.0, then_range.1),
                            self.render_branch_as_expr(else_range.0, else_range.1),
                        ) {
                            let cond = self.consume_cond_at(i);
                            let text = format!(
                                "{} ? {} : {}",
                                paren_if_le(&cond, 4),
                                then_e,
                                else_e,
                            );
                            let mut bc = cond.bc_idx;
                            bc.push(i);
                            self.push_entry(text, 4, bc);
                            i = cont_idx;
                            continue;
                        }
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
                    Region::Loop { body_range, step_range, header_idx, cont_idx } => {
                        let cond = self.consume_cond_at(i);
                        let step_text = if let Some(sr) = step_range {
                            self.render_inline(sr.0, sr.1)
                        } else {
                            String::new()
                        };
                        // Only steal a preceding line as the for-loop init
                        // when we're actually going to emit a for-loop (i.e.
                        // there *is* a tail step). Otherwise this would
                        // swallow whatever statement happened to be before
                        // the while-loop.
                        let (init_text, init_bc) = if !step_text.is_empty() {
                            self.steal_for_init(body_range.0)
                        } else {
                            (String::new(), Vec::new())
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
                        self.loop_stack.push(LoopCtx { header_idx, cont_idx });
                        self.emit_range(body_range.0, body_emit_end);
                        self.loop_stack.pop();
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

    /// If `target_idx` matches a loop boundary in the enclosing loop stack,
    /// return "break" or "continue". The innermost loop wins. Returns None
    /// when the jump isn't a structured break/continue (we'd emit it as a
    /// labeled comment instead, but for our purposes the most common case
    /// is a simple `break;`).
    fn classify_loop_jump(&self, target_idx: usize) -> Option<&'static str> {
        let ctx = self.loop_stack.last()?;
        if target_idx == ctx.cont_idx {
            return Some("break");
        }
        if target_idx == ctx.header_idx {
            return Some("continue");
        }
        None
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
                        // Empty-then-block check: when IFEQ skips exactly the
                        // single trailing GOTO, the pattern is `if (cond)
                        // <goto>;` (typically `if (cond) break;`), not an
                        // if-else. The IFEQ's "else" range I'd compute here
                        // would actually be the rest of the surrounding
                        // block. Fall through to IfOnly so the GOTO appears
                        // inside the then-body where the loop-jump classifier
                        // can rename it to break/continue.
                        if prev_idx == i + 1 {
                            return Some(Region::IfOnly {
                                then_range: (i + 1, target_idx),
                                cont_idx: target_idx,
                            });
                        }
                        // Real if-else: then body is [i+1, prev_idx), else
                        // body is [target_idx, prev_target_idx). Continuation
                        // = prev_target_idx.
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
                            header_idx: prev_target_idx,
                            cont_idx: target_idx,
                        });
                    }
                    // GOTO target is outside both ranges (forward past `end`
                    // OR somewhere we don't recognise structurally). Fall
                    // through to IfOnly so the GOTO appears inside the
                    // then-body where classify_loop_jump can recognise it
                    // as a `break;` / `continue;` of an enclosing loop.
                }
            }
            // Either no GOTO before the target, or the GOTO targets outside
            // any structured region — emit as `if (cond) { ... }` with the
            // whole [i+1, target_idx) range as the then-body. Inner
            // statements (including a trailing GOTO that jumps elsewhere)
            // get processed by the recursive emit_range.
            return Some(Region::IfOnly {
                then_range: (i + 1, target_idx),
                cont_idx: target_idx,
            });
        }
        None
    }

    /// Detect the for-step expression at the tail of a loop body. The step
    /// is the LAST statement of the body, immediately preceding the
    /// back-GOTO. We scan backwards over trailing POPs to find the step's
    /// terminator (a SETNAME / SETVAR / SETARG / SETPROP / SETELEM / inc /
    /// dec), then walk back over the value-computation ops using a stack-
    /// effect tracker -- stopping precisely when the net stack effect from
    /// "step start" through the trailing POP balances to zero (i.e., this
    /// IS a self-contained statement).
    ///
    /// Earlier versions either widened too little (`r = undefined` because
    /// the value-computation wasn't captured) or too much (the whole loop
    /// body got swallowed into the step when there was no preceding POP).
    fn detect_for_step(&self, body_start: usize, body_end: usize) -> Option<(usize, usize)> {
        if body_end == 0 || body_end <= body_start { return None; }
        let mut step_end = body_end;
        while step_end > body_start && matches!(self.instructions[step_end - 1].op, JsOp::Pop) {
            step_end -= 1;
        }
        if step_end <= body_start { return None; }
        let last = &self.instructions[step_end - 1];
        let is_stepish = matches!(last.op,
            JsOp::Varinc | JsOp::Vardec | JsOp::Arginc | JsOp::Argdec
            | JsOp::Incvar | JsOp::Decvar | JsOp::Incarg | JsOp::Decarg
            | JsOp::Setvar | JsOp::Setarg | JsOp::Setname
            | JsOp::Setprop | JsOp::Setelem);
        if !is_stepish { return None; }
        // Walk back, tracking stack net effect. We start from one past the
        // trailing POP and accumulate the net effect of each op as we
        // include it. The shortest suffix where total == 0 is the
        // statement boundary.
        let mut total: i32 = 0;
        let mut step_start = step_end;
        for k in (body_start..body_end).rev() {
            let (uses, defs) = match op_stack_effect(&self.instructions[k]) {
                Some(x) => x,
                None => return None, // unknown effect; abort widening
            };
            total += defs - uses;
            if total == 0 {
                step_start = k;
                break;
            }
            if total > 0 {
                // Net positive net before reaching zero means the suffix
                // produces values that flow OUT of the step -- this can't
                // be a self-contained statement; abort.
                return None;
            }
        }
        if total != 0 { return None; }
        Some((step_start, body_end))
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

    /// Try to render a range as a single pure expression. Returns Some(text)
    /// when the range produces exactly one stack value with no emitted
    /// statements -- i.e. when it's safe to use as a ternary `?:` arm or as
    /// the right-hand side of `&&` / `||`. Returns None otherwise (range
    /// contains statements, multiple value pushes, or no value at all).
    fn render_branch_as_expr(&self, start: usize, end: usize) -> Option<String> {
        if start >= end { return None; }
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
            loop_stack: self.loop_stack.clone(),
            do_while_at: std::collections::HashMap::new(), // suppress structural detection
            while_true_at: std::collections::HashMap::new(),
        };
        let n = child.instructions.len();
        let mut i = 0usize;
        while i < n {
            i = child.step(i);
        }
        if !child.out.is_empty() {
            return None;
        }
        if child.stack.len() != 1 {
            return None;
        }
        let top = child.stack.pop().unwrap();
        if top.text.starts_with('<') { return None; } // bind/this marker, not a real expr
        Some(top.text)
    }

    /// Render an instruction range as a single JS expression. Used by &&/||
    /// and ?:. Returns the expression text — if the slice produces multiple
    /// statements or no value, returns "/* expr */".
    fn render_subexpr(&self, start: usize, end: usize) -> String {
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
            loop_stack: self.loop_stack.clone(),
            do_while_at: self.do_while_at.clone(),
            while_true_at: self.while_true_at.clone(),
        };
        let n = child.instructions.len();
        let mut i = 0usize;
        while i < n {
            i = child.step(i);
        }
        if let Some(top) = child.stack.last() {
            return top.text.clone();
        }
        if let Some(line) = child.out.last() {
            return line.text.trim_end_matches(';').to_string();
        }
        "undefined".into()
    }

    /// Render a JsFunctionAtom body as `{ ... }` for use in function-expression
    /// contexts (the value of an `Anonfunobj` push, the RHS of `var x = function() {...}`).
    fn render_function_body(&self, fa: &JsFunctionAtom) -> String {
        let dec = decompile(&fa.script, &fa.bindings);
        let mut lines = Vec::new();
        // Emit any function-local var declarations first (mirrors the
        // function-statement emitter).
        for b in fa.bindings.iter().filter(|b| b.kind != JsBindingKind::Argument) {
            lines.push(format!("  var {};", b.name));
        }
        for line in &dec.lines {
            lines.push(format!("{}{}", "  ".repeat(line.indent as usize + 1), line.text));
        }
        format!("{{\n{}\n}}", lines.join("\n"))
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
            loop_stack: self.loop_stack.clone(),
            do_while_at: self.do_while_at.clone(),
            while_true_at: self.while_true_at.clone(),
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
                // `this` placeholder. For normal CALLs (e.g. `trace(...)`)
                // CALL pops this slot and ignores it. If the caller never
                // pushed a function value, the pop-callee step ends up
                // grabbing this sentinel — which in JS is `this(args)`
                // (calling whatever `this` refers to). Either way, "this"
                // is the right text.
                self.stack.push(StackEntry { text: "this".into(), prec: 100, bc_idx: vec![i] });
                i + 1
            }
            JsOp::Bindname => {
                // BINDNAME pushes the scope OBJECT that contains `name` so a
                // later SETNAME / SETPROP / GETPROP can operate on it. We
                // remember the name as a marker; GETPROP / SETPROP / GETELEM
                // / SETELEM strip the marker when their object is one of
                // these and emit a bare name access.
                let idx = read_u16_operand(&ins.operand).unwrap_or(0) as usize;
                let name = atom_name(self.ir, idx);
                self.stack.push(StackEntry {
                    text: format!("<bind:{}>", name),
                    prec: 100,
                    bc_idx: vec![i],
                });
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
                // BINDNAME-scope read: `<bind:w>.w` is just `w`.
                let text = if is_bind_marker(&obj.text) {
                    name
                } else {
                    format!("{}.{}", paren_if_lt(&obj, 95), name)
                };
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
                let target = if is_bind_marker(&obj.text) {
                    name
                } else {
                    format!("{}.{}", paren_if_lt(&obj, 95), name)
                };
                self.push_assignment(&target, &val.text, bc);
                i + 1
            }
            JsOp::Getelem => {
                let key = self.pop_or_undef();
                let obj = self.pop_or_undef();
                let mut bc = obj.bc_idx.clone();
                bc.extend(&key.bc_idx);
                bc.push(i);
                let text = if is_bind_marker(&obj.text) {
                    // `<bind:foo>[key]` doesn't have a JS source form -- the
                    // scope-object computed indexing is internal. Fall back
                    // to showing the key alone, which approximates a global
                    // dynamic name read.
                    key.text
                } else {
                    format!("{}[{}]", paren_if_lt(&obj, 95), key.text)
                };
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
                let target = if is_bind_marker(&obj.text) {
                    key.text
                } else {
                    format!("{}[{}]", paren_if_lt(&obj, 95), key.text)
                };
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
            JsOp::Incprop | JsOp::Propinc | JsOp::Decprop | JsOp::Propdec => {
                let idx = read_u16_operand(&ins.operand).unwrap_or(0) as usize;
                let name = atom_name(self.ir, idx);
                let obj = self.pop_or_undef();
                let target = if is_bind_marker(&obj.text) {
                    name
                } else {
                    format!("{}.{}", paren_if_lt(&obj, 95), name)
                };
                let s = match ins.op {
                    JsOp::Incprop => format!("++{}", target),
                    JsOp::Propinc => format!("{}++", target),
                    JsOp::Decprop => format!("--{}", target),
                    _             => format!("{}--", target),
                };
                self.push_entry(s, 15, vec![i]); i + 1
            }
            JsOp::Incelem | JsOp::Eleminc | JsOp::Decelem | JsOp::Elemdec => {
                let key = self.pop_or_undef();
                let obj = self.pop_or_undef();
                let target = if is_bind_marker(&obj.text) {
                    key.text
                } else {
                    format!("{}[{}]", paren_if_lt(&obj, 95), key.text)
                };
                let s = match ins.op {
                    JsOp::Incelem => format!("++{}", target),
                    JsOp::Eleminc => format!("{}++", target),
                    JsOp::Decelem => format!("--{}", target),
                    _             => format!("{}--", target),
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

            // ===== switch (table / lookup / cond) =====
            //
            // SpiderMonkey emits one of three forms:
            //   TABLESWITCH default, low, high, case_off[high-low+1]
            //   LOOKUPSWITCH default, npairs, [(atom_idx, off)]*
            //   CONDSWITCH then a sequence of <val>; CASE off; … (default at end)
            //
            // Reconstructing the case bodies from these would need flow
            // analysis that knows where each case `break;`s to. For now we
            // emit the switch header as a header comment and let the body
            // ops decompile inline below — readable but not perfectly
            // structured. Better than dropping them entirely.
            JsOp::Tableswitch => {
                let disc = self.pop_or_undef();
                let low  = read_i16_operand(&ins.operand[2..]).unwrap_or(0) as i32;
                let high = read_i16_operand(&ins.operand[4..]).unwrap_or(0) as i32;
                self.emit_line_with_idx(
                    format!("switch ({}) {{  // tableswitch low={} high={}", disc.text, low, high),
                    vec![i],
                );
                i + 1
            }
            JsOp::Lookupswitch => {
                let disc = self.pop_or_undef();
                let npairs = read_u16_operand(&ins.operand[2..]).unwrap_or(0);
                self.emit_line_with_idx(
                    format!("switch ({}) {{  // lookupswitch npairs={}", disc.text, npairs),
                    vec![i],
                );
                i + 1
            }
            JsOp::Case | JsOp::Casex => {
                // The case-label "value" is below the discriminant on the
                // stack; CASE peeks-and-compares. If we're decompiling a
                // sub-region, pop the case-value entry and emit `case v:`.
                let v = self.pop_or_undef();
                self.emit_line_with_idx(format!("case {}:", v.text), vec![i]);
                i + 1
            }
            JsOp::Default | JsOp::Defaultx => {
                // DEFAULT also pops the discriminant. Emit a default: label
                // (the actual default body decompiles inline after this).
                let _disc = self.pop_or_undef();
                self.emit_line_with_idx("default:".into(), vec![i]);
                i + 1
            }
            JsOp::Condswitch => {
                // Pure marker before the CASE-chain. The switch header has
                // already been emitted by the surrounding context — drop
                // silently so we don't pollute output.
                i + 1
            }

            // ===== Function expressions (used as values, not declarations) =====
            JsOp::Anonfunobj | JsOp::Namedfunobj | JsOp::Closure => {
                let idx = read_u16_operand(&ins.operand).unwrap_or(0) as usize;
                if let Some(JsAtom::Function(fa)) = self.ir.atoms.get(idx) {
                    let body = self.render_function_body(fa);
                    let args: Vec<&str> = fa.bindings.iter()
                        .filter(|b| b.kind == JsBindingKind::Argument)
                        .map(|b| b.name.as_str())
                        .collect();
                    let name = if ins.op == JsOp::Namedfunobj || ins.op == JsOp::Closure {
                        fa.name.as_deref().unwrap_or("")
                    } else { "" };
                    let text = format!("function {}({}) {}", name, args.join(", "), body);
                    self.push_entry(text, 100, vec![i]);
                } else {
                    self.push_entry("/* function-expr */".into(), 100, vec![i]);
                }
                i + 1
            }

            // ===== Short-circuit operators (&&, ||) =====
            //
            // `a && b` compiles to: <expr a>; AND target; <expr b>; target:
            // `a || b` compiles to: <expr a>; OR  target; <expr b>; target:
            //
            // AND/OR peek (not pop) — if jump-taken, `a` stays on TOS; if not
            // taken, ops in [i+1, target) overwrite the stack top to produce
            // `b`. We rerun those ops as a child render to capture `b`, then
            // emit a single combined `(a && b)` / `(a || b)` expression and
            // skip past `target`.
            JsOp::And | JsOp::Or => {
                let delta = read_i16_operand(&ins.operand).unwrap_or(0) as i32;
                let target_off = (ins.offset as i32 + delta) as usize;
                if let Some(&target_idx) = self.offset_to_idx.get(&target_off) {
                    if target_idx > i {
                        let a = self.pop_or_undef();
                        // Render [i+1, target_idx) as an expression — should
                        // push exactly one value onto the child stack which
                        // we adopt.
                        let b_text = self.render_subexpr(i + 1, target_idx);
                        let op = if ins.op == JsOp::And { " && " } else { " || " };
                        let mut bc = a.bc_idx.clone();
                        bc.push(i);
                        self.push_entry(format!("{}{}{}", paren_if_lt(&a, 5), op, b_text), 5, bc);
                        return target_idx;
                    }
                }
                // Couldn't resolve target — fall back to comment.
                self.emit_line_with_idx(format!("/* {} */", ins.op.info().mnemonic), vec![i]);
                i + 1
            }

            // ===== Conditional (?:) =====
            //
            // `a ? b : c` compiles to: <expr a>; IFEQ else; <expr b>;
            // GOTO end; else: <expr c>; end:
            //
            // We detect a forward IFEQ whose target Y has instruction at Y-3
            // == GOTO Z > Y, AND the instructions in [i+1, Y-3] don't emit
            // any statements (just push exactly one value). Pure-expression
            // detection: if both then and else slices, when rendered, produce
            // a single expression each, we treat the whole thing as ?:.
            //
            // For simplicity we always *try* this path first when classify
            // says IfElse — render_subexpr will report failure if the slice
            // can't be expressed as a single expression.

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
                // `return;` reads cleaner than `return undefined;`.
                if e.text == "undefined" {
                    self.emit_line_with_idx("return;".into(), bc);
                } else {
                    self.emit_line_with_idx(format!("return {};", e.text), bc);
                }
                i + 1
            }
            JsOp::Dup => {
                if let Some(top) = self.stack.last().cloned() {
                    self.stack.push(top);
                }
                i + 1
            }
            JsOp::Dup2 => {
                // Duplicate the top two stack entries: [..., a, b] -> [..., a, b, a, b].
                let n = self.stack.len();
                if n >= 2 {
                    let b = self.stack[n - 1].clone();
                    let a = self.stack[n - 2].clone();
                    self.stack.push(a);
                    self.stack.push(b);
                }
                i + 1
            }
            JsOp::Swap => {
                let n = self.stack.len();
                if n >= 2 { self.stack.swap(n - 1, n - 2); }
                i + 1
            }
            // `group` is a SpiderMonkey marker tracking grouped sub-expressions
            // for the decompiler (originally for source-precedence parens).
            // Treat as a no-op for our purposes.
            JsOp::Group | JsOp::Nop => i + 1,

            // ===== Control flow =====
            JsOp::Return => {
                let e = self.pop_or_undef();
                let mut bc = e.bc_idx.clone();
                bc.push(i);
                if e.text == "undefined" {
                    self.emit_line_with_idx("return;".into(), bc);
                } else {
                    self.emit_line_with_idx(format!("return {};", e.text), bc);
                }
                i + 1
            }
            JsOp::Retrval => {
                self.emit_line_with_idx("return;".into(), vec![i]);
                i + 1
            }
            JsOp::Goto | JsOp::Gotox | JsOp::Ifeq | JsOp::Ifeqx | JsOp::Ifne | JsOp::Ifnex => {
                // Reached when classify_branch couldn't shape the jump into
                // a structured if/for/while -- typically a `break;` or
                // `continue;` inside a loop, or an unstructured forward goto.
                let delta = if matches!(ins.op, JsOp::Gotox | JsOp::Ifeqx | JsOp::Ifnex) {
                    super::variable_length::read_i32_operand(&ins.operand).unwrap_or(0)
                } else {
                    read_i16_operand(&ins.operand).unwrap_or(0) as i32
                };
                let target_off = (ins.offset as i32 + delta) as usize;
                let target_idx = self.offset_to_idx.get(&target_off).copied();
                match ins.op {
                    JsOp::Goto | JsOp::Gotox => {
                        if let Some(t) = target_idx {
                            if let Some(kw) = self.classify_loop_jump(t) {
                                self.emit_line_with_idx(format!("{};", kw), vec![i]);
                                return i + 1;
                            }
                        }
                        self.emit_line_with_idx(format!("/* goto -> {} */", target_off), vec![i]);
                    }
                    _ => {
                        let cond = self.pop_or_undef();
                        let mut bc = cond.bc_idx.clone();
                        bc.push(i);
                        // Conditional jump (IFEQ = jump-if-false; IFNE = jump-if-true).
                        // When the target is a loop boundary, render as
                        // `if (cond) break;` / `if (cond) continue;` etc.
                        let is_inverted = matches!(ins.op, JsOp::Ifne | JsOp::Ifnex);
                        if let Some(t) = target_idx {
                            if let Some(kw) = self.classify_loop_jump(t) {
                                let cond_text = if is_inverted {
                                    cond.text.clone()
                                } else {
                                    format!("!({})", cond.text)
                                };
                                self.emit_line_with_idx(format!("if ({}) {};", cond_text, kw), bc);
                                return i + 1;
                            }
                        }
                        let kw = if is_inverted { "if-true" } else { "if-false" };
                        self.emit_line_with_idx(format!("/* {} ({}) -> {} */", kw, cond.text, target_off), bc);
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

            // ===== Exception-handling markers =====
            //
            // The full try/catch shape would need to consult ir.try_notes to
            // know the covered range and the catch handler PC. For now we
            // emit visible markers so the body still appears between them,
            // and the inspector reads as `try { ... } catch (e) { ... }`.
            JsOp::Try => {
                self.emit_line_with_idx("try {".into(), vec![i]);
                self.indent += 1;
                i + 1
            }
            JsOp::Finally => {
                if self.indent > 0 { self.indent -= 1; }
                self.emit_line_with_idx("} finally {".into(), vec![i]);
                self.indent += 1;
                i + 1
            }
            JsOp::Initcatchvar => {
                // The catch-variable name is the operand atom. We close the
                // surrounding `try {`/`} finally {` block and open the
                // matching `catch (e) {`.
                let idx = read_u16_operand(&ins.operand).unwrap_or(0) as usize;
                let name = atom_name(self.ir, idx);
                if self.indent > 0 { self.indent -= 1; }
                self.emit_line_with_idx(format!("}} catch ({}) {{", name), vec![i]);
                self.indent += 1;
                // INITCATCHVAR's value-pop and `obj` peek are the engine's
                // way of binding the exception into a catch scope object.
                // Drop them so they don't leak into the source.
                let _ = self.pop_or_undef();
                let _ = self.pop_or_undef();
                i + 1
            }
            JsOp::Exception => {
                // After an unwind the exception value sits on TOS. We don't
                // generate a JS expression for it directly; the next op (a
                // SETVAR / INITCATCHVAR) names it. Stub as undefined for the
                // stack effect.
                self.push("undefined", 100, vec![i]);
                i + 1
            }
            JsOp::Gosub | JsOp::Gosubx => {
                // Internal try/finally control transfer — no source-level
                // equivalent. Drop.
                i + 1
            }
            JsOp::Retsub => {
                let _ = self.pop_or_undef();
                i + 1
            }
            JsOp::Setsp => {
                // Stack reset during exception unwind. Source-level invisible.
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

/// True if `text` is one of the BINDNAME / `this` / NEWINIT sentinels —
/// SpiderMonkey leaves these on the stack for the engine's bookkeeping but
/// they have no source-level counterpart.
fn is_bind_marker(text: &str) -> bool {
    text.starts_with("<bind:") || text == "<bind>" || text.starts_with("<newinit:")
}

/// Per-op (uses, defs) stack effect. The opcode table marks variable-arity
/// ops with `uses = -1`; we resolve those by reading the relevant operand
/// (argc for CALL/NEW). Returns None for ops whose stack effect can't be
/// statically determined (callers should fall back to a conservative
/// estimate rather than mis-walking the body).
fn op_stack_effect(ins: &DecodedIns) -> Option<(i32, i32)> {
    let info = ins.op.info();
    let uses = if info.uses >= 0 {
        info.uses as i32
    } else {
        match ins.op {
            // CALL / NEW: argc + 2 (fn slot, this slot, args).
            JsOp::Call | JsOp::New => {
                if ins.operand.len() < 2 { return None; }
                let argc = u16::from_be_bytes([ins.operand[0], ins.operand[1]]) as i32;
                argc + 2
            }
            _ => return None,
        }
    };
    Some((uses, info.defs as i32))
}
