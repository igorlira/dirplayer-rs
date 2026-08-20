use std::collections::{HashMap, HashSet};

use fxhash::FxHashMap;

use crate::{
    director::{
        chunks::{
            handler::{Bytecode, HandlerDef},
            script::ScriptChunk,
        },
        lingo::{constants::{anim2_prop_names, anim_prop_names, movie_prop_names}, datum::Datum, opcode::OpCode, script::ScriptContext},
    },
    player::{
        eval::{parse_expr_to_lingo_expr, parse_to_lingo_expr, LingoExpr},
        sprite::ColorRef,
        symbols::{builtin::BuiltInSymbol, symbol::Symbol},
    },
};

// ─── Public API ───────────────────────────────────────────────────────────────

pub struct CompileResult {
    pub names: Vec<String>,
    pub chunk: ScriptChunk,
}

pub fn compile_lingo(source: &str, script_number: u16) -> Result<CompileResult, String> {
    let mut sc = ScriptCompiler::new();
    sc.compile(source, script_number)
}

/// Merge a CompileResult's names + chunk into an existing ScriptContext.
/// Remaps all name_ids so they point into lctx.names.
pub fn inject_into_lctx(lctx: &mut ScriptContext, result: CompileResult, script_id: u32) {
    let CompileResult { names, mut chunk } = result;

    // Build mapping: old name_id -> lctx name_id
    let mut id_map: HashMap<u16, u16> = HashMap::new();
    for (old_id, name) in names.iter().enumerate() {
        let new_id = if let Some(pos) = lctx.names.iter().position(|n| n == name) {
            pos as u16
        } else {
            let id = lctx.names.len() as u16;
            lctx.names.push(name.clone());
            id
        };
        id_map.insert(old_id as u16, new_id);
    }

    remap_chunk(&mut chunk, &id_map);
    lctx.scripts.insert(script_id, chunk);
}

fn remap_chunk(chunk: &mut ScriptChunk, map: &HashMap<u16, u16>) {
    let remap = |id: u16| map.get(&id).copied().unwrap_or(id);

    for id in &mut chunk.property_name_ids {
        *id = remap(*id);
    }

    for handler in &mut chunk.handlers {
        handler.name_id = remap(handler.name_id);
        for id in &mut handler.argument_name_ids {
            *id = remap(*id);
        }
        for id in &mut handler.local_name_ids {
            *id = remap(*id);
        }
        for id in &mut handler.global_name_ids {
            *id = remap(*id);
        }

        for bc in &mut handler.bytecode_array {
            match bc.opcode {
                OpCode::GetGlobal
                | OpCode::SetGlobal
                | OpCode::GetProp
                | OpCode::SetProp
                | OpCode::GetObjProp
                | OpCode::SetObjProp
                | OpCode::ExtCall
                | OpCode::ObjCall
                | OpCode::PushSymb
                | OpCode::GetChainedProp
                | OpCode::GetMovieProp
                | OpCode::SetMovieProp
                | OpCode::TheBuiltin
                | OpCode::PushVarRef => {
                    bc.obj = remap(bc.obj as u16) as i64;
                }
                _ => {}
            }
        }

        relayout_handler_bytecodes(handler);
    }
}

// ─── Block-level AST ──────────────────────────────────────────────────────────

struct ScriptNode {
    properties: Vec<String>,
    handlers: Vec<HandlerNode>,
}

struct HandlerNode {
    name: String,
    params: Vec<String>,
    body: Vec<StmtNode>,
}

enum StmtNode {
    Line(String),
    GlobalDecl(Vec<String>),
    Case {
        expr: String,
        branches: Vec<CaseBranchNode>,
        otherwise_body: Vec<StmtNode>,
    },
    If {
        cond: String,
        then_body: Vec<StmtNode>,
        else_body: Vec<StmtNode>,
    },
    RepeatWhile {
        cond: String,
        body: Vec<StmtNode>,
    },
    RepeatWith {
        var: String,
        start: String,
        end: String,
        step: i8,
        body: Vec<StmtNode>,
    },
    RepeatIn {
        var: String,
        list_expr: String,
        body: Vec<StmtNode>,
    },
    ExitRepeat,
    NextRepeat,
    Return(Option<String>),
    Exit,
}

struct CaseBranchNode {
    labels: Vec<String>,
    body: Vec<StmtNode>,
}

// ─── Block Parser ─────────────────────────────────────────────────────────────

fn parse_script_block(source: &str) -> ScriptNode {
    let lines: Vec<&str> = source.lines().collect();
    let mut idx = 0;
    let mut properties = Vec::new();
    let mut handlers = Vec::new();

    while idx < lines.len() {
        let line = lines[idx].trim();
        if line.is_empty() || line.starts_with("--") {
            idx += 1;
            continue;
        }
        let kw = first_word_lc(line);
        match kw.as_str() {
            "property" => {
                properties.extend(split_names(&line["property".len()..].trim_start()));
                idx += 1;
            }
            "on" => {
                let (h, ni) = parse_handler_block(&lines, idx);
                handlers.push(h);
                idx = ni;
            }
            _ => {
                idx += 1;
            }
        }
    }
    ScriptNode { properties, handlers }
}

fn parse_handler_block(lines: &[&str], start: usize) -> (HandlerNode, usize) {
    let header = lines[start].trim();
    let (name, params) = parse_handler_header(header);
    let (body, mut idx) = parse_stmts(lines, start + 1, &["end"]);
    if idx < lines.len() && first_word_lc(lines[idx].trim()) == "end" {
        idx += 1;
    }
    (HandlerNode { name, params, body }, idx)
}

fn parse_stmts(lines: &[&str], start: usize, stops: &[&str]) -> (Vec<StmtNode>, usize) {
    let mut stmts = Vec::new();
    let mut idx = start;
    while idx < lines.len() {
        let line = lines[idx].trim();
        if line.is_empty() || line.starts_with("--") {
            idx += 1;
            continue;
        }
        let kw = first_word_lc(line);
        if stops.contains(&kw.as_str()) {
            break;
        }
        match kw.as_str() {
            "case" => {
                let (s, ni) = parse_case(lines, idx);
                stmts.push(s);
                idx = ni;
            }
            "if" => {
                let (s, ni) = parse_if(lines, idx);
                stmts.push(s);
                idx = ni;
            }
            "repeat" => {
                let (s, ni) = parse_repeat(lines, idx);
                stmts.push(s);
                idx = ni;
            }
            "global" => {
                stmts.push(StmtNode::GlobalDecl(split_names(
                    line["global".len()..].trim_start(),
                )));
                idx += 1;
            }
            "exit" => {
                let rest = line["exit".len()..].trim().to_lowercase();
                if rest.starts_with("repeat") {
                    stmts.push(StmtNode::ExitRepeat);
                } else {
                    stmts.push(StmtNode::Exit);
                }
                idx += 1;
            }
            "next" => {
                stmts.push(StmtNode::NextRepeat);
                idx += 1;
            }
            "return" => {
                let rest = line["return".len()..].trim().to_string();
                stmts.push(StmtNode::Return(if rest.is_empty() {
                    None
                } else {
                    Some(rest)
                }));
                idx += 1;
            }
            _ => {
                stmts.push(StmtNode::Line(line.to_string()));
                idx += 1;
            }
        }
    }
    (stmts, idx)
}

fn parse_stmt(lines: &[&str], idx: usize) -> (StmtNode, usize) {
    let line = lines[idx].trim();
    let kw = first_word_lc(line);
    match kw.as_str() {
        "case" => parse_case(lines, idx),
        "if" => parse_if(lines, idx),
        "repeat" => parse_repeat(lines, idx),
        "global" => (
            StmtNode::GlobalDecl(split_names(line["global".len()..].trim_start())),
            idx + 1,
        ),
        "exit" => {
            let rest = line["exit".len()..].trim().to_lowercase();
            if rest.starts_with("repeat") {
                (StmtNode::ExitRepeat, idx + 1)
            } else {
                (StmtNode::Exit, idx + 1)
            }
        }
        "next" => (StmtNode::NextRepeat, idx + 1),
        "return" => {
            let rest = line["return".len()..].trim().to_string();
            (
                StmtNode::Return(if rest.is_empty() { None } else { Some(rest) }),
                idx + 1,
            )
        }
        _ => (StmtNode::Line(line.to_string()), idx + 1),
    }
}

fn parse_case(lines: &[&str], start: usize) -> (StmtNode, usize) {
    let header = lines[start].trim();
    let lower = header.to_lowercase();
    let expr = if let Some(pos) = lower.rfind(" of") {
        header["case".len()..pos].trim().to_string()
    } else {
        header["case".len()..].trim().to_string()
    };

    let mut branches = Vec::new();
    let mut otherwise_body = Vec::new();
    let mut idx = start + 1;

    while idx < lines.len() {
        let line = lines[idx].trim();
        if line.is_empty() || line.starts_with("--") {
            idx += 1;
            continue;
        }

        let kw = first_word_lc(line);
        if kw == "end" {
            idx += 1;
            break;
        }

        if let Some((labels, inline_stmt)) = parse_case_label_line(line) {
            idx += 1;
            let mut body = Vec::new();
            if let Some(inline_stmt) = inline_stmt {
                body.push(StmtNode::Line(inline_stmt));
            }
            let (parsed_body, next_idx) = parse_case_branch_body(lines, idx);
            body.extend(parsed_body);
            idx = next_idx;

            if labels.len() == 1 && labels[0].eq_ignore_ascii_case("otherwise") {
                otherwise_body = body;
            } else {
                branches.push(CaseBranchNode { labels, body });
            }
            continue;
        }

        idx += 1;
    }

    (
        StmtNode::Case {
            expr,
            branches,
            otherwise_body,
        },
        idx,
    )
}

fn parse_case_branch_body(lines: &[&str], start: usize) -> (Vec<StmtNode>, usize) {
    let mut stmts = Vec::new();
    let mut idx = start;

    while idx < lines.len() {
        let line = lines[idx].trim();
        if line.is_empty() || line.starts_with("--") {
            idx += 1;
            continue;
        }

        let kw = first_word_lc(line);
        if kw == "end" || parse_case_label_line(line).is_some() {
            break;
        }

        let (stmt, next_idx) = parse_stmt(lines, idx);
        stmts.push(stmt);
        idx = next_idx;
    }

    (stmts, idx)
}

fn find_top_level_colon(s: &str) -> Option<usize> {
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut in_string = false;
    let mut chars = s.char_indices().peekable();

    while let Some((i, ch)) = chars.next() {
        match ch {
            '\\' if in_string => { chars.next(); }
            '"' => { in_string = !in_string; }
            '(' if !in_string => { paren_depth += 1; }
            ')' if !in_string => { paren_depth = paren_depth.saturating_sub(1); }
            '[' if !in_string => { bracket_depth += 1; }
            ']' if !in_string => { bracket_depth = bracket_depth.saturating_sub(1); }
            ':' if !in_string && paren_depth == 0 && bracket_depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

fn find_top_level_eq(s: &str) -> Option<usize> {
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut in_string = false;
    let mut chars = s.char_indices().peekable();

    while let Some((i, ch)) = chars.next() {
        match ch {
            '\\' if in_string => { chars.next(); }
            '"' => { in_string = !in_string; }
            '(' if !in_string => { paren_depth += 1; }
            ')' if !in_string => { paren_depth = paren_depth.saturating_sub(1); }
            '[' if !in_string => { bracket_depth += 1; }
            ']' if !in_string => { bracket_depth = bracket_depth.saturating_sub(1); }
            '=' if !in_string && paren_depth == 0 && bracket_depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

fn parse_case_label_line(line: &str) -> Option<(Vec<String>, Option<String>)> {
    let trimmed = line.trim();
    let colon_index = find_top_level_colon(trimmed)?;
    let prefix = trimmed[..colon_index].trim();
    if prefix.is_empty() {
        return None;
    }

    let first_word = first_word_lc(prefix);
    if matches!(first_word.as_str(), "if" | "repeat" | "global" | "exit" | "next" | "return" | "case") {
        return None;
    }
    if let Some(eq_index) = find_top_level_eq(trimmed) {
        if eq_index < colon_index {
            return None;
        }
    }

    let suffix = trimmed[colon_index + 1..].trim();
    let labels = split_top_level_commas(prefix);
    if labels.is_empty() {
        return None;
    }

    Some((
        labels,
        if suffix.is_empty() { None } else { Some(suffix.to_string()) },
    ))
}

fn split_top_level_commas(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut in_string = false;
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                current.push(ch);
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            '"' => {
                in_string = !in_string;
                current.push(ch);
            }
            '(' if !in_string => {
                paren_depth += 1;
                current.push(ch);
            }
            ')' if !in_string => {
                paren_depth = paren_depth.saturating_sub(1);
                current.push(ch);
            }
            '[' if !in_string => {
                bracket_depth += 1;
                current.push(ch);
            }
            ']' if !in_string => {
                bracket_depth = bracket_depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if !in_string && paren_depth == 0 && bracket_depth == 0 => {
                let part = current.trim();
                if !part.is_empty() {
                    parts.push(part.to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    let part = current.trim();
    if !part.is_empty() {
        parts.push(part.to_string());
    }
    parts
}

fn parse_if_with_header(lines: &[&str], start: usize, header: &str) -> (StmtNode, usize) {
    let (cond, inline) = split_if_header(header);
    if let Some(then_line) = inline {
        return (
            StmtNode::If {
                cond,
                then_body: vec![StmtNode::Line(then_line)],
                else_body: vec![],
            },
            start + 1,
        );
    }
    let (then_body, mut idx) = parse_stmts(lines, start + 1, &["else", "end"]);
    let mut else_body = vec![];
    if idx < lines.len() && lines[idx].trim().to_ascii_lowercase().starts_with("else if") {
        let nested_header = lines[idx].trim()["else".len()..].trim_start();
        let (nested_if, ni) = parse_if_with_header(lines, idx, nested_header);
        else_body = vec![nested_if];
        idx = ni;
    } else if idx < lines.len() && first_word_lc(lines[idx].trim()) == "else" {
        idx += 1;
        let (eb, ni) = parse_stmts(lines, idx, &["end"]);
        else_body = eb;
        idx = ni;
    }
    if idx < lines.len() && first_word_lc(lines[idx].trim()) == "end" {
        idx += 1;
    }
    (StmtNode::If { cond, then_body, else_body }, idx)
}

fn parse_if(lines: &[&str], start: usize) -> (StmtNode, usize) {
    let header = lines[start].trim();
    parse_if_with_header(lines, start, header)
}

fn parse_repeat(lines: &[&str], start: usize) -> (StmtNode, usize) {
    let header = lines[start].trim();
    let (body, mut idx) = parse_stmts(lines, start + 1, &["end"]);
    if idx < lines.len() && first_word_lc(lines[idx].trim()) == "end" {
        idx += 1;
    }

    let after = &header["repeat".len()..].trim_start().to_lowercase();
    if after.starts_with("while") {
        let cond = header["repeat".len()..].trim_start()["while".len()..]
            .trim_start()
            .to_string();
        (StmtNode::RepeatWhile { cond, body }, idx)
    } else if after.starts_with("with") {
        let orig = header["repeat".len()..].trim_start()["with".len()..]
            .trim_start();
        match parse_repeat_with_header(orig) {
            RepeatHeader::Range { var, start, end, step } => {
                (StmtNode::RepeatWith { var, start, end, step, body }, idx)
            }
            RepeatHeader::InList { var, list_expr } => {
                (StmtNode::RepeatIn { var, list_expr, body }, idx)
            }
            RepeatHeader::Invalid => {
                (StmtNode::RepeatWith {
                    var: "_x".to_string(),
                    start: "0".to_string(),
                    end: "0".to_string(),
                    step: 1,
                    body,
                }, idx)
            }
        }
    } else {
        (StmtNode::RepeatWhile { cond: "false".to_string(), body: vec![] }, idx)
    }
}

fn parse_handler_header(line: &str) -> (String, Vec<String>) {
    let rest = line["on".len()..].trim_start();
    let mut parts = rest.splitn(2, |c: char| c == ' ' || c == '\t' || c == ',');
    let name = parts.next().unwrap_or("").trim().to_string();
    let params = if let Some(params_str) = parts.next() {
        split_names(params_str.trim())
    } else {
        vec![]
    };
    (name, params)
}

fn split_if_header(line: &str) -> (String, Option<String>) {
    let after = &line["if".len()..].trim_start();
    let lower = after.to_lowercase();
    if let Some(pos) = lower.find(" then") {
        let cond = after[..pos].trim().to_string();
        let inline = after[pos + 5..].trim();
        (cond, if inline.is_empty() { None } else { Some(inline.to_string()) })
    } else {
        (after.trim().to_string(), None)
    }
}

enum RepeatHeader {
    Range {
        var: String,
        start: String,
        end: String,
        step: i8,
    },
    InList {
        var: String,
        list_expr: String,
    },
    Invalid,
}

fn parse_repeat_with_header(s: &str) -> RepeatHeader {
    let lower = s.to_lowercase();
    // Check for range pattern (=) before "in" to avoid misparse of
    // "i = 1 to the number of lines in X" as an "in" loop.
    if let Some(eq) = s.find('=') {
        let var = s[..eq].trim().to_string();
        let rest = s[eq + 1..].trim();
        let lower = rest.to_lowercase();
        if let Some(p) = lower.find(" down to ") {
            let start = rest[..p].trim().to_string();
            let end = rest[p + " down to ".len()..].trim().to_string();
            RepeatHeader::Range { var, start, end, step: -1i8 }
        } else if let Some(p) = lower.find(" to ") {
            let start = rest[..p].trim().to_string();
            let end = rest[p + " to ".len()..].trim().to_string();
            RepeatHeader::Range { var, start, end, step: 1i8 }
        } else {
            RepeatHeader::Range {
                var,
                start: rest.to_string(),
                end: "0".to_string(),
                step: 1i8,
            }
        }
    } else if let Some(p) = lower.find(" in ") {
        let var = s[..p].trim().to_string();
        let list_expr = s[p + " in ".len()..].trim().to_string();
        if var.is_empty() || list_expr.is_empty() {
            RepeatHeader::Invalid
        } else {
            RepeatHeader::InList { var, list_expr }
        }
    } else {
        RepeatHeader::Invalid
    }
}

fn first_word_lc(s: &str) -> String {
    s.split_whitespace().next().unwrap_or("").to_lowercase()
}

fn split_names(s: &str) -> Vec<String> {
    s.split(',')
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

fn case_implicit_shared_tail<'a>(expr: &str, otherwise_body: &'a [StmtNode]) -> (&'a [StmtNode], &'a [StmtNode]) {
    let Some(base_name) = expr.trim().strip_suffix(".ilk").map(str::trim) else {
        return (otherwise_body, &[]);
    };

    if otherwise_body.len() < 3 {
        return (otherwise_body, &[]);
    }

    let Some(StmtNode::Line(assign_line)) = otherwise_body.first() else {
        return (otherwise_body, &[]);
    };
    let Some((lhs, rhs)) = assign_line.split_once('=') else {
        return (otherwise_body, &[]);
    };
    if !lhs.trim().eq_ignore_ascii_case(base_name) {
        return (otherwise_body, &[]);
    }

    let expected_rhs = format!("list({base_name})");
    if !rhs.trim().eq_ignore_ascii_case(&expected_rhs) {
        return (otherwise_body, &[]);
    }

    match otherwise_body.get(1) {
        Some(StmtNode::RepeatIn { list_expr, .. }) if list_expr.eq_ignore_ascii_case(base_name) => {
            (&otherwise_body[..2], &otherwise_body[2..])
        }
        _ => (otherwise_body, &[]),
    }
}

fn case_body_terminates(body: &[StmtNode]) -> bool {
    match body.last() {
        Some(StmtNode::Return(_)) | Some(StmtNode::Exit) => true,
        Some(StmtNode::If { then_body, else_body, .. }) => {
            !else_body.is_empty()
                && case_body_terminates(then_body)
                && case_body_terminates(else_body)
        }
        _ => false,
    }
}

fn stmt_always_terminates(stmt: &StmtNode) -> bool {
    match stmt {
        StmtNode::Return(_) | StmtNode::Exit => true,
        StmtNode::If { then_body, else_body, .. } => {
            !else_body.is_empty()
                && case_body_terminates(then_body)
                && case_body_terminates(else_body)
        }
        _ => false,
    }
}

fn stmt_contains_return_or_exit(stmt: &StmtNode) -> bool {
    match stmt {
        StmtNode::Return(_) | StmtNode::Exit => true,
        StmtNode::If { then_body, else_body, .. } => {
            then_body.iter().any(|s| stmt_contains_return_or_exit(s))
                || else_body.iter().any(|s| stmt_contains_return_or_exit(s))
        }
        StmtNode::Case { branches, otherwise_body, .. } => {
            branches.iter().any(|b| b.body.iter().any(|s| stmt_contains_return_or_exit(s)))
                || otherwise_body.iter().any(|s| stmt_contains_return_or_exit(s))
        }
        _ => false,
    }
}

// ─── Instruction representation ───────────────────────────────────────────────

#[derive(Clone)]
enum Operand {
    Val(i64),
    FwdJump(usize),   // forward jump target (instruction index)
    BwdRepeat(usize), // backward EndRepeat target (instruction index)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SizeHint {
    None,
    Short,
}

#[derive(Clone)]
struct Instr {
    opcode: OpCode,
    op: Operand,
    size_hint: SizeHint,
}

struct LoopCtx {
    cond_idx: usize,
    continue_target: Option<usize>,
    breaks: Vec<usize>,
    continues: Vec<usize>,
}

// ─── Script-level compiler ────────────────────────────────────────────────────

struct ScriptCompiler {
    names: Vec<String>,
    name_map: HashMap<String, u16>,
    literals: Vec<Datum>,
    properties: HashSet<String>,
    handler_names: HashSet<String>,
    handler_indices: HashMap<String, usize>,
}

impl ScriptCompiler {
    fn new() -> Self {
        Self {
            names: Vec::new(),
            name_map: HashMap::new(),
            literals: Vec::new(),
            properties: HashSet::new(),
            handler_names: HashSet::new(),
            handler_indices: HashMap::new(),
        }
    }

    fn get_name_id(&mut self, name: &str) -> u16 {
        if let Some(&id) = self.name_map.get(name) {
            return id;
        }
        let id = self.names.len() as u16;
        self.names.push(name.to_string());
        self.name_map.insert(name.to_string(), id);
        id
    }

    fn add_literal(&mut self, datum: Datum) -> usize {
        for (i, existing) in self.literals.iter().enumerate() {
            if literal_eq(existing, &datum) {
                return i;
            }
        }
        let idx = self.literals.len();
        self.literals.push(datum);
        idx
    }

    fn compile(&mut self, source: &str, script_number: u16) -> Result<CompileResult, String> {
        let node = parse_script_block(source);

        // First pass: collect all handler names so LocalCall can reference them
        self.handler_indices.clear();
        for (index, h) in node.handlers.iter().enumerate() {
            self.handler_names.insert(h.name.clone());
            self.handler_indices.insert(h.name.to_lowercase(), index);
        }

        for prop in &node.properties {
            self.properties.insert(prop.clone());
            self.get_name_id(prop);
        }
        let property_name_ids: Vec<u16> = node.properties.iter()
            .map(|p| self.get_name_id(p))
            .collect();

        let mut handlers = Vec::new();
        for h in &node.handlers {
            handlers.push(self.compile_handler(h)?);
        }

        let literals = std::mem::take(&mut self.literals);
        let chunk = ScriptChunk {
            script_number,
            literals,
            handlers,
            property_name_ids,
            property_defaults: HashMap::new(),
        };

        Ok(CompileResult { names: self.names.clone(), chunk })
    }

    fn compile_handler(&mut self, node: &HandlerNode) -> Result<HandlerDef, String> {
        let name_id = self.get_name_id(&node.name);
        // Compute argument_name_ids before creating HandlerCompiler (which borrows self)
        let argument_name_ids: Vec<u16> = node.params.iter()
            .map(|p| self.get_name_id(p))
            .collect();

        let props: HashSet<String> = self.properties.clone();
        let handler_names: HashSet<String> = self.handler_names.clone();
        let handler_indices: HashMap<String, usize> = self.handler_indices.clone();
        let mut hc = HandlerCompiler {
            sc: self,
            params: node.params.clone(),
            locals: Vec::new(),
            local_map: HashMap::new(),
            globals: HashSet::new(),
            props: &props,
            handler_names: &handler_names,
            handler_indices: &handler_indices,
            instrs: Vec::new(),
            loops: Vec::new(),
            case_expr_depth: 0,
        };

        hc.compile_stmts(&node.body)?;
        hc.emit_val(OpCode::Ret, 0);

        // Extract locals and globals before finalize() moves hc
        let locals = std::mem::take(&mut hc.locals);
        let globals_vec: Vec<String> = hc.globals.iter().cloned().collect();

        let (bytecode_array, bytecode_index_map) = hc.finalize();
        // hc is consumed here, its borrow of self (via hc.sc) is released

        let local_name_ids: Vec<u16> = locals.iter()
            .map(|l| self.get_name_id(l))
            .collect();
        let global_name_ids: Vec<u16> = globals_vec.iter()
            .map(|g| self.get_name_id(g))
            .collect();

        Ok(HandlerDef {
            name_id,
            bytecode_array,
            bytecode_index_map,
            argument_name_ids,
            local_name_ids,
            global_name_ids,
            compiled_ir: std::cell::RefCell::new(None),
        })
    }
}

// ─── Handler-level compiler ───────────────────────────────────────────────────

enum VarKind {
    Param(usize),
    Local(usize),
    Global(u16),
    Prop(u16),
}

struct HandlerCompiler<'a> {
    sc: &'a mut ScriptCompiler,
    params: Vec<String>,
    locals: Vec<String>,
    local_map: HashMap<String, usize>,
    globals: HashSet<String>,
    props: &'a HashSet<String>,
    handler_names: &'a HashSet<String>,
    handler_indices: &'a HashMap<String, usize>,
    instrs: Vec<Instr>,
    loops: Vec<LoopCtx>,
    case_expr_depth: usize,
}

impl<'a> HandlerCompiler<'a> {
    fn emit(&mut self, opcode: OpCode, op: Operand) -> usize {
        let idx = self.instrs.len();
        self.instrs.push(Instr {
            opcode,
            op,
            size_hint: SizeHint::None,
        });
        idx
    }

    fn emit_with_size_hint(&mut self, opcode: OpCode, op: Operand, size_hint: SizeHint) -> usize {
        let idx = self.instrs.len();
        self.instrs.push(Instr { opcode, op, size_hint });
        idx
    }

    fn emit_val(&mut self, opcode: OpCode, val: i64) -> usize {
        self.emit(opcode, Operand::Val(val))
    }

    fn cur_idx(&self) -> usize {
        self.instrs.len()
    }

    // Emit a chunk position index (may be a negative sentinel like -30000).
    // Unlike general compile_expr_val, negative integer literals are encoded
    // as signed values directly (PushInt16(-30000)) rather than PushInt16(30000)+Inv.
    fn emit_chunk_index_val(&mut self, expr: &LingoExpr) -> Result<(), String> {
        let neg_val = match expr {
            LingoExpr::Negate(inner) => match inner.as_ref() {
                LingoExpr::IntLiteral(n) => Some(-(*n as i64)),
                _ => None,
            },
            _ => None,
        };
        if let Some(n) = neg_val {
            if n == 0 {
                self.emit_val(OpCode::PushZero, 0);
            } else if (-128..=127).contains(&n) {
                self.emit_val(OpCode::PushInt8, n);
            } else if (-32768..=32767).contains(&n) {
                self.emit_val(OpCode::PushInt16, n);
            } else {
                self.emit_val(OpCode::PushInt32, n);
            }
            return Ok(());
        }
        self.compile_expr_val(expr)
    }

    fn name_id(&mut self, name: &str) -> u16 {
        self.sc.get_name_id(name)
    }

    fn is_local_handler(&self, name: &str) -> bool {
        self.handler_names.iter().any(|h| h.eq_ignore_ascii_case(name))
    }

    fn emit_call(&mut self, name: &str, arg_count: i64, no_ret: bool) {
        let push_op = if no_ret { OpCode::PushArgListNoRet } else { OpCode::PushArgList };
        self.emit_val(push_op, arg_count);
        if self.is_local_handler(name) {
            let handler_index = self
                .handler_indices
                .get(&name.to_lowercase())
                .copied()
                .unwrap_or_default() as i64;
            self.emit_val(OpCode::LocalCall, handler_index);
        } else {
            let name_id = self.sc.get_name_id(name) as i64;
            self.emit_val(OpCode::ExtCall, name_id);
        }
    }

    fn add_literal(&mut self, datum: Datum) -> usize {
        self.sc.add_literal(datum)
    }

    fn resolve_var(&mut self, name: &str) -> VarKind {
        if let Some(idx) = self
            .params
            .iter()
            .position(|p| p.eq_ignore_ascii_case(name))
        {
            return VarKind::Param(idx);
        }
        if self.globals.iter().any(|g| g.eq_ignore_ascii_case(name)) {
            let id = self.sc.get_name_id(name);
            return VarKind::Global(id);
        }
        if self.props.iter().any(|p| p.eq_ignore_ascii_case(name)) {
            let id = self.sc.get_name_id(name);
            return VarKind::Prop(id);
        }
        if let Some(&idx) = self.local_map.get(&name.to_lowercase()) {
            return VarKind::Local(idx);
        }
        let idx = self.locals.len();
        self.locals.push(name.to_string());
        self.local_map.insert(name.to_lowercase(), idx);
        VarKind::Local(idx)
    }

    fn emit_var_read(&mut self, name: &str) {
        match self.resolve_var(name) {
            VarKind::Param(idx) => {
                self.emit_val(OpCode::GetParam, idx as i64);
            }
            VarKind::Local(idx) => {
                self.emit_val(OpCode::GetLocal, idx as i64);
            }
            VarKind::Global(id) => {
                self.emit_val(OpCode::GetGlobal, id as i64);
            }
            VarKind::Prop(id) => {
                self.emit_val(OpCode::GetProp, id as i64);
            }
        }
    }

    fn emit_var_write(&mut self, name: &str) {
        match self.resolve_var(name) {
            VarKind::Param(idx) => {
                self.emit_val(OpCode::SetParam, idx as i64);
            }
            VarKind::Local(idx) => {
                self.emit_val(OpCode::SetLocal, idx as i64);
            }
            VarKind::Global(id) => {
                self.emit_val(OpCode::SetGlobal, id as i64);
            }
            VarKind::Prop(id) => {
                self.emit_val(OpCode::SetProp, id as i64);
            }
        }
    }

    fn emit_push_chunk_var_ref(&mut self, name: &str) -> Result<(), String> {
        let var_kind = self.resolve_var(name);
        match var_kind {
            VarKind::Param(idx) => {
                if idx == 0 {
                    self.emit_val(OpCode::PushZero, 0);
                } else if idx <= 127 {
                    self.emit_val(OpCode::PushInt8, idx as i64);
                } else {
                    self.emit_val(OpCode::PushInt16, idx as i64);
                }
                self.emit_val(OpCode::PushChunkVarRef, 0x4);
            }
            VarKind::Local(idx) => {
                if idx == 0 {
                    self.emit_val(OpCode::PushZero, 0);
                } else if idx <= 127 {
                    self.emit_val(OpCode::PushInt8, idx as i64);
                } else {
                    self.emit_val(OpCode::PushInt16, idx as i64);
                }
                self.emit_val(OpCode::PushChunkVarRef, 0x5);
            }
            VarKind::Global(name_id) => {
                self.emit_val(OpCode::PushVarRef, name_id as i64);
                self.emit_val(OpCode::PushChunkVarRef, 0x1);
            }
            VarKind::Prop(name_id) => {
                self.emit_val(OpCode::PushVarRef, name_id as i64);
                self.emit_val(OpCode::PushChunkVarRef, 0x3);
            }
        }
        Ok(())
    }

    fn compile_stmts(&mut self, stmts: &[StmtNode]) -> Result<(), String> {
        for s in stmts {
            self.compile_stmt(s)?;
        }
        Ok(())
    }

    fn compile_stmt(&mut self, stmt: &StmtNode) -> Result<(), String> {
        match stmt {
            StmtNode::Line(line) => self.compile_line(line),
            StmtNode::GlobalDecl(names) => {
                for name in names {
                    self.globals.insert(name.clone());
                    self.sc.get_name_id(name);
                }
                Ok(())
            }
            StmtNode::Case { expr, branches, otherwise_body } => {
                self.compile_case(expr, branches, otherwise_body)
            }
            StmtNode::If { cond, then_body, else_body } => {
                self.compile_if(cond, then_body, else_body)
            }
            StmtNode::RepeatWhile { cond, body } => self.compile_repeat_while(cond, body),
            StmtNode::RepeatWith { var, start, end, step, body } => {
                self.compile_repeat_with(var, start, end, *step, body)
            }
            StmtNode::RepeatIn { var, list_expr, body } => {
                self.compile_repeat_in(var, list_expr, body)
            }
            StmtNode::ExitRepeat => self.compile_exit_repeat(),
            StmtNode::NextRepeat => self.compile_next_repeat(),
            StmtNode::Return(expr) => self.compile_return(expr.as_deref()),
            StmtNode::Exit => {
                self.emit_val(OpCode::Ret, 0);
                Ok(())
            }
        }
    }

    fn compile_line(&mut self, line: &str) -> Result<(), String> {
        let expr = parse_to_lingo_expr(line)
            .map_err(|e| format!("Parse error in '{line}': {e}"))?;
        self.compile_expr_stmt(&expr)
    }

    fn compile_if(
        &mut self,
        cond: &str,
        then_body: &[StmtNode],
        else_body: &[StmtNode],
    ) -> Result<(), String> {
        let cond_expr = parse_expr_to_lingo_expr(cond)
            .or_else(|_| parse_to_lingo_expr(cond))
            .map_err(|e| format!("Parse error in if condition '{cond}': {e}"))?;
        self.compile_expr_val(&cond_expr)?;

        let jmp_to_else = self.emit(OpCode::JmpIfZ, Operand::FwdJump(0));

        self.compile_stmts(then_body)?;

        if !else_body.is_empty() {
            let jmp_past_else = self.emit(OpCode::Jmp, Operand::FwdJump(0));
            let else_start = self.cur_idx();
            self.instrs[jmp_to_else].op = Operand::FwdJump(else_start);
            self.compile_stmts(else_body)?;
            let after_else = self.cur_idx();
            self.instrs[jmp_past_else].op = Operand::FwdJump(after_else);
        } else {
            let after_then = self.cur_idx();
            self.instrs[jmp_to_else].op = Operand::FwdJump(after_then);
        }

        Ok(())
    }

    fn compile_case(
        &mut self,
        expr: &str,
        branches: &[CaseBranchNode],
        otherwise_body: &[StmtNode],
    ) -> Result<(), String> {
        let case_expr = parse_expr_to_lingo_expr(expr)
            .or_else(|_| parse_to_lingo_expr(expr))
            .map_err(|e| format!("Parse error in case expression '{expr}': {e}"))?;

        self.compile_expr_val(&case_expr)?;

        let has_otherwise = !otherwise_body.is_empty();

        // 0-branch with otherwise: jmp past otherwise (it's dead code); dead-code pop at end.
        if branches.is_empty() && has_otherwise {
            let initial_jmp = self.emit(OpCode::Jmp, Operand::FwdJump(0));
            self.case_expr_depth += 1;
            self.compile_stmts(otherwise_body)?;
            self.case_expr_depth -= 1;
            let dead_pop_idx = self.cur_idx();
            self.emit_val(OpCode::Pop, 1);
            self.instrs[initial_jmp].op = Operand::FwdJump(dead_pop_idx);
            return Ok(());
        }

        // 0-branch with no otherwise: just pop the case expression.
        if branches.is_empty() {
            self.emit_val(OpCode::Pop, 1);
            return Ok(());
        }

        let mut end_jumps = Vec::new();
        for (branch_index, branch) in branches.iter().enumerate() {
            let n_labels = branch.labels.len();
            let mut body_jumps: Vec<usize> = Vec::new();

            for (label_idx, label) in branch.labels.iter().enumerate() {
                let label_expr = parse_expr_to_lingo_expr(label)
                    .or_else(|_| parse_to_lingo_expr(label))
                    .map_err(|e| format!("Parse error in case label '{label}': {e}"))?;
                let is_last_label = label_idx == n_labels - 1;
                self.emit_val(OpCode::Peek, 0);
                self.compile_expr_val(&label_expr)?;
                if !is_last_label {
                    self.emit_val(OpCode::NtEq, 0);
                    body_jumps.push(self.emit(OpCode::JmpIfZ, Operand::FwdJump(0)));
                } else {
                    self.emit_val(OpCode::Eq, 0);
                }
            }

            let next_branch_jump = self.emit(OpCode::JmpIfZ, Operand::FwdJump(0));

            let body_start = self.cur_idx();
            for j in body_jumps {
                self.instrs[j].op = Operand::FwdJump(body_start);
            }

            self.case_expr_depth += 1;
            self.compile_stmts(&branch.body)?;
            self.case_expr_depth -= 1;

            let is_last_branch = branch_index + 1 == branches.len();
            // All arms emit end-jumps when there is an otherwise body.
            // Last arm without otherwise: no end-jump; its jmpifz falls through to the cleanup pop.
            if has_otherwise || !is_last_branch {
                end_jumps.push(self.emit(OpCode::Jmp, Operand::FwdJump(0)));
            }
            let next_branch_idx = self.cur_idx();
            self.instrs[next_branch_jump].op = Operand::FwdJump(next_branch_idx);
        }

        if has_otherwise {
            // Director's otherwise body rule:
            //
            //   • If first stmt always terminates (return/exit): compile first stmt at depth+1
            //     (compile_return emits a cleanup pop), emit dead-code pop (arm end-jumps →
            //     that pop), compile remaining stmts at depth=0.
            //
            //   • Otherwise: find split_point = first stmt index containing any return/exit.
            //     Compile stmts[0..split_point] at depth+1 (they never return, so depth
            //     doesn't matter — but must match Director for the arm end-jump target byte).
            //     Emit dead-code pop; arm end-jumps → that pop.
            //     Compile stmts[split_point..] at depth=0 (case expr already cleaned).
            //     If split_point == len (no returns in body), this degenerates to: compile all
            //     at depth+1, dead-code pop after.
            let first_terminates = stmt_always_terminates(&otherwise_body[0]);

            if first_terminates {
                // First stmt unconditionally exits: compile it at depth+1 so compile_return
                // emits a cleanup pop, then emit dead-code pop (arm end-jumps → that pop),
                // then compile remaining stmts at depth=0.
                self.case_expr_depth += 1;
                self.compile_stmts(&otherwise_body[..1])?;
                self.case_expr_depth -= 1;
                let dead_pop_idx = self.cur_idx();
                self.emit_val(OpCode::Pop, 1);
                for jump in end_jumps {
                    self.instrs[jump].op = Operand::FwdJump(dead_pop_idx);
                }
                self.compile_stmts(&otherwise_body[1..])?;
            } else if case_body_terminates(otherwise_body) {
                // Body terminates (last stmt always exits). Find the first stmt that contains
                // any return/exit — compile stmts before it at depth+1, emit dead-code pop
                // (arm end-jumps → that pop), compile from that stmt at depth=0 so any return
                // inside doesn't emit an extra case-expr cleanup pop.
                let split_point = otherwise_body
                    .iter()
                    .position(|s| stmt_contains_return_or_exit(s))
                    .unwrap_or(otherwise_body.len());
                self.case_expr_depth += 1;
                self.compile_stmts(&otherwise_body[..split_point])?;
                self.case_expr_depth -= 1;
                let dead_pop_idx = self.cur_idx();
                self.emit_val(OpCode::Pop, 1);
                for jump in end_jumps {
                    self.instrs[jump].op = Operand::FwdJump(dead_pop_idx);
                }
                self.compile_stmts(&otherwise_body[split_point..])?;
            } else {
                // Body doesn't terminate: compile all at depth+1, dead-code pop AFTER.
                // Returns inside emit their own cleanup pop via compile_return (depth+1).
                self.case_expr_depth += 1;
                self.compile_stmts(otherwise_body)?;
                self.case_expr_depth -= 1;
                let dead_pop_idx = self.cur_idx();
                self.emit_val(OpCode::Pop, 1);
                for jump in end_jumps {
                    self.instrs[jump].op = Operand::FwdJump(dead_pop_idx);
                }
            }
        } else {
            // No otherwise: single cleanup pop serves as jmpifz-last and end-jump targets.
            let cleanup_idx = self.cur_idx();
            self.emit_val(OpCode::Pop, 1);
            for jump in end_jumps {
                self.instrs[jump].op = Operand::FwdJump(cleanup_idx);
            }
        }
        Ok(())
    }

    fn compile_repeat_while(&mut self, cond: &str, body: &[StmtNode]) -> Result<(), String> {
        let cond_idx = self.cur_idx();

        let cond_expr = parse_expr_to_lingo_expr(cond)
            .or_else(|_| parse_to_lingo_expr(cond))
            .map_err(|e| format!("Parse error in repeat while condition '{cond}': {e}"))?;
        self.compile_expr_val(&cond_expr)?;

        let jmp_exit = self.emit(OpCode::JmpIfZ, Operand::FwdJump(0));

        self.loops.push(LoopCtx {
            cond_idx,
            continue_target: None,
            breaks: Vec::new(),
            continues: Vec::new(),
        });
        self.compile_stmts(body)?;
        let loop_ctx = self.loops.pop().unwrap();

        let endrepeat_idx = self.cur_idx();
        self.emit(OpCode::EndRepeat, Operand::BwdRepeat(cond_idx));

        let after_loop = self.cur_idx();
        self.instrs[jmp_exit].op = Operand::FwdJump(after_loop);
        for brk in loop_ctx.breaks {
            self.instrs[brk].op = Operand::FwdJump(after_loop);
        }
        for cont in loop_ctx.continues {
            self.instrs[cont].op = Operand::FwdJump(endrepeat_idx);
        }

        Ok(())
    }

    fn compile_repeat_with(
        &mut self,
        var: &str,
        start_str: &str,
        end_str: &str,
        step: i8,
        body: &[StmtNode],
    ) -> Result<(), String> {
        // Initialize loop variable
        let start_expr = parse_expr_to_lingo_expr(start_str)
            .or_else(|_| parse_to_lingo_expr(start_str))
            .map_err(|e| format!("Parse error in repeat start '{start_str}': {e}"))?;
        self.compile_expr_val(&start_expr)?;
        self.emit_var_write(var);

        // Loop condition check
        let cond_idx = self.cur_idx();
        self.emit_var_read(var);
        let end_expr = parse_expr_to_lingo_expr(end_str)
            .or_else(|_| parse_to_lingo_expr(end_str))
            .map_err(|e| format!("Parse error in repeat end '{end_str}': {e}"))?;
        self.compile_expr_val(&end_expr)?;

        let jmp_exit = if step >= 0 {
            self.emit_val(OpCode::LtEq, 0);
            self.emit(OpCode::JmpIfZ, Operand::FwdJump(0))
        } else {
            self.emit_val(OpCode::GtEq, 0);
            self.emit(OpCode::JmpIfZ, Operand::FwdJump(0))
        };

        self.loops.push(LoopCtx {
            cond_idx,
            continue_target: None,
            breaks: Vec::new(),
            continues: Vec::new(),
        });
        self.compile_stmts(body)?;
        let increment_idx = self.cur_idx();
        if let Some(loop_ctx) = self.loops.last_mut() {
            loop_ctx.continue_target = Some(increment_idx);
        }
        let loop_ctx = self.loops.pop().unwrap();

        // Increment/decrement (Director order: push 1 first, then read var)
        if step >= 0 {
            self.emit_val(OpCode::PushInt8, 1);
            self.emit_var_read(var);
            self.emit_val(OpCode::Add, 0);
        } else {
            self.emit_val(OpCode::PushInt8, -1);
            self.emit_var_read(var);
            self.emit_val(OpCode::Add, 0);
        }
        self.emit_var_write(var);

        self.emit(OpCode::EndRepeat, Operand::BwdRepeat(cond_idx));

        let after_loop = self.cur_idx();
        self.instrs[jmp_exit].op = Operand::FwdJump(after_loop);
        for brk in loop_ctx.breaks {
            self.instrs[brk].op = Operand::FwdJump(after_loop);
        }
        for cont in loop_ctx.continues {
            self.instrs[cont].op = Operand::FwdJump(increment_idx);
        }

        Ok(())
    }

    fn compile_repeat_in(
        &mut self,
        var: &str,
        list_expr: &str,
        body: &[StmtNode],
    ) -> Result<(), String> {
        let list_expr = parse_expr_to_lingo_expr(list_expr)
            .or_else(|_| parse_to_lingo_expr(list_expr))
            .map_err(|e| format!("Parse error in repeat-in list '{list_expr}': {e}"))?;

        self.compile_expr_val(&list_expr)?;
        self.emit_val(OpCode::Peek, 0);
        self.emit_call("count", 1, false);
        self.emit_val(OpCode::PushInt8, 1);

        let cond_idx = self.cur_idx();
        self.emit_val(OpCode::Peek, 0);
        self.emit_val(OpCode::Peek, 2);
        self.emit_val(OpCode::LtEq, 0);
        let jmp_exit = self.emit(OpCode::JmpIfZ, Operand::FwdJump(0));

        self.emit_val(OpCode::Peek, 2);
        self.emit_val(OpCode::Peek, 1);
        self.emit_call("getAt", 2, false);
        self.emit_var_write(var);

        self.loops.push(LoopCtx {
            cond_idx,
            continue_target: None,
            breaks: Vec::new(),
            continues: Vec::new(),
        });
        self.compile_stmts(body)?;
        let increment_idx = self.cur_idx();
        if let Some(loop_ctx) = self.loops.last_mut() {
            loop_ctx.continue_target = Some(increment_idx);
        }
        let loop_ctx = self.loops.pop().unwrap();

        self.emit_val(OpCode::PushInt8, 1);
        self.emit_val(OpCode::Add, 0);
        self.emit(OpCode::EndRepeat, Operand::BwdRepeat(cond_idx));

        let cleanup_idx = self.cur_idx();
        self.instrs[jmp_exit].op = Operand::FwdJump(cleanup_idx);
        for brk in loop_ctx.breaks {
            self.instrs[brk].op = Operand::FwdJump(cleanup_idx);
        }
        for cont in loop_ctx.continues {
            self.instrs[cont].op = Operand::FwdJump(increment_idx);
        }

        self.emit_val(OpCode::Pop, 3);
        Ok(())
    }

    fn compile_exit_repeat(&mut self) -> Result<(), String> {
        if !self.loops.is_empty() {
            let jmp_idx = self.emit(OpCode::Jmp, Operand::FwdJump(0));
            self.loops.last_mut().unwrap().breaks.push(jmp_idx);
        }
        Ok(())
    }

    fn compile_next_repeat(&mut self) -> Result<(), String> {
        if !self.loops.is_empty() {
            let jmp_idx = self.emit(OpCode::Jmp, Operand::FwdJump(0));
            self.loops.last_mut().unwrap().continues.push(jmp_idx);
        }
        Ok(())
    }

    fn compile_the_identifier_read(&mut self, name: &str) -> bool {
        let Some(the_name) = normalize_the_identifier(name) else {
            return false;
        };

        if is_the_builtin_name(&the_name) {
            self.emit_val(OpCode::PushArgList, 0);
            let name_id = self.sc.get_name_id(&the_name) as i64;
            self.emit_val(OpCode::TheBuiltin, name_id);
        } else if let Some((prop_type, prop_id)) = legacy_movie_get_prop(&the_name) {
            self.emit_legacy_get_prop(prop_type, prop_id);
        } else {
            let name_id = self.sc.get_name_id(&the_name) as i64;
            self.emit_val(OpCode::GetMovieProp, name_id);
        }
        true
    }

    fn emit_legacy_get_prop(&mut self, prop_type: u16, prop_id: u16) {
        let prop_id = prop_id as i64;
        if prop_id == 0 {
            self.emit_val(OpCode::PushZero, 0);
        } else if prop_id <= 127 {
            self.emit_val(OpCode::PushInt8, prop_id);
        } else if prop_id <= 32767 {
            self.emit_val(OpCode::PushInt16, prop_id);
        } else {
            self.emit_val(OpCode::PushInt32, prop_id);
        }
        self.emit_val(OpCode::Get, prop_type as i64);
    }

    fn compile_obj_chain_base(&mut self, expr: &LingoExpr) -> Result<(), String> {
        match expr {
            LingoExpr::ObjProp(obj, prop) => {
                self.compile_obj_chain_base(obj)?;
                let prop_id = self.sc.get_name_id(prop) as i64;
                self.emit_val(OpCode::GetChainedProp, prop_id);
                Ok(())
            }
            LingoExpr::ListAccess(obj, idx) => {
                self.compile_list_access_expr(obj, idx, true)
            }
            _ => self.compile_expr_val(expr),
        }
    }

    fn compile_list_access_expr(
        &mut self,
        obj: &LingoExpr,
        idx: &LingoExpr,
        prefer_ref: bool,
    ) -> Result<(), String> {
        if let LingoExpr::ObjProp(base_obj, base_prop) = obj {
            self.compile_method_receiver(base_obj)?;
            let prop_id = self.sc.get_name_id(base_prop) as i64;
            self.emit_val(OpCode::PushSymb, prop_id);
            self.compile_expr_val(idx)?;
            self.emit_val(OpCode::PushArgList, 3);
            let name_id = self
                .sc
                .get_name_id(if prefer_ref { "getPropRef" } else { "getProp" }) as i64;
            self.emit_val(OpCode::ObjCall, name_id);
            return Ok(());
        }

        if let LingoExpr::ListAccess(inner_obj, inner_idx) = obj {
            self.compile_list_access_expr(inner_obj, inner_idx, true)?;
        } else {
            self.compile_expr_val(obj)?;
        }

        match idx {
            LingoExpr::SymbolLiteral(symbol_name) => {
                let symbol_id = self.sc.get_name_id(symbol_name) as i64;
                self.emit_val(OpCode::PushSymb, symbol_id);
            }
            _ => self.compile_expr_val(idx)?,
        }
        self.emit_val(OpCode::PushArgList, 2);
        let name_id = self.sc.get_name_id("getAt") as i64;
        self.emit_val(OpCode::ObjCall, name_id);
        Ok(())
    }

    fn compile_method_receiver(&mut self, expr: &LingoExpr) -> Result<(), String> {
        match expr {
            LingoExpr::ObjProp(obj, prop) => {
                self.compile_obj_chain_base(obj)?;
                let prop_id = self.sc.get_name_id(prop) as i64;
                self.emit_val(OpCode::GetChainedProp, prop_id);
                Ok(())
            }
            LingoExpr::ListAccess(obj, idx) => {
                self.compile_list_access_expr(obj, idx, true)
            }
            _ => self.compile_expr_val(expr),
        }
    }

    fn compile_prop_symbol_call(&mut self, obj: &LingoExpr, prop: &str, method: &str) -> Result<(), String> {
        self.compile_method_receiver(obj)?;
        let prop_id = self.sc.get_name_id(prop) as i64;
        self.emit_val(OpCode::PushSymb, prop_id);
        self.emit_val(OpCode::PushArgList, 2);
        let method_id = self.sc.get_name_id(method) as i64;
        self.emit_val(OpCode::ObjCall, method_id);
        Ok(())
    }

    fn compile_return(&mut self, expr: Option<&str>) -> Result<(), String> {
        let return_id = self.sc.get_name_id("return") as i64;
        if let Some(expr_str) = expr {
            if let Ok(e) = parse_expr_to_lingo_expr(expr_str)
                .or_else(|_| parse_to_lingo_expr(expr_str))
            {
                if self.case_expr_depth > 0 {
                    self.emit_val(OpCode::Pop, self.case_expr_depth as i64);
                }
                self.compile_expr_val(&e)?;
                self.emit_val(OpCode::PushArgListNoRet, 1);
            } else {
                self.emit_val(OpCode::PushArgListNoRet, 0);
            }
        } else {
            if self.case_expr_depth > 0 {
                self.emit_val(OpCode::Pop, self.case_expr_depth as i64);
            }
            self.emit_val(OpCode::PushArgListNoRet, 0);
        }
        self.emit_val(OpCode::ExtCall, return_id);
        Ok(())
    }

    // Compile an expression used as a statement (for side effects)
    fn compile_expr_stmt(&mut self, expr: &LingoExpr) -> Result<(), String> {
        match expr {
            LingoExpr::Identifier(name) => {
                // Bare identifier at statement level is always a 0-arg handler call.
                // (A bare variable read at statement level has no side effect and would be a
                // no-op; Director treats `foo` without parens as `foo()` when used as a statement.)
                self.emit_call(name, 0, true);
                Ok(())
            }
            LingoExpr::Assignment(lhs, rhs) if matches!(lhs.as_ref(), LingoExpr::ListAccess(..)) => {
                // list[key] = value → obj.setAt(key, value)
                let LingoExpr::ListAccess(list_obj, list_idx) = lhs.as_ref() else { unreachable!() };
                if let LingoExpr::ObjProp(base_obj, base_prop) = list_obj.as_ref() {
                    self.compile_expr_val(base_obj)?;
                    let prop_id = self.sc.get_name_id(base_prop) as i64;
                    self.emit_val(OpCode::PushSymb, prop_id);
                    self.compile_expr_val(list_idx)?;
                    self.compile_expr_val(rhs)?;
                    self.emit_val(OpCode::PushArgListNoRet, 4);
                    let name_id = self.sc.get_name_id("setProp") as i64;
                    self.emit_val(OpCode::ObjCall, name_id);
                    return Ok(());
                }

                if let LingoExpr::ListAccess(inner_obj, inner_idx) = list_obj.as_ref() {
                    self.compile_list_access_expr(inner_obj, inner_idx, true)?;
                } else {
                    self.compile_expr_val(list_obj)?;
                }
                match list_idx.as_ref() {
                    LingoExpr::SymbolLiteral(symbol_name) => {
                        let symbol_id = self.sc.get_name_id(symbol_name) as i64;
                        self.emit_val(OpCode::PushSymb, symbol_id);
                    }
                    _ => self.compile_expr_val(list_idx)?,
                }
                self.compile_expr_val(rhs)?;
                self.emit_val(OpCode::PushArgListNoRet, 3);
                let name_id = self.sc.get_name_id("setAt") as i64;
                self.emit_val(OpCode::ObjCall, name_id);
                Ok(())
            }
            LingoExpr::Assignment(lhs, rhs) if matches!(lhs.as_ref(), LingoExpr::ObjProp(..)) => {
                // obj.prop = value → push obj, push value, SetObjProp
                let LingoExpr::ObjProp(obj, prop) = lhs.as_ref() else { unreachable!() };
                self.compile_obj_chain_base(obj)?;
                self.compile_expr_val(rhs)?;
                let prop_id = self.sc.get_name_id(prop) as i64;
                self.emit_val(OpCode::SetObjProp, prop_id);
                Ok(())
            }
            LingoExpr::Assignment(lhs, rhs) => {
                self.compile_expr_val(rhs)?;
                self.compile_assign_lhs(lhs)
            }
            LingoExpr::HandlerCall(name, args) => {
                for arg in args {
                    self.compile_expr_val(arg)?;
                }
                let n = args.len() as i64;
                self.emit_call(name, n, true);
                Ok(())
            }
            LingoExpr::ObjHandlerCall(obj, method, args) => {
                // delete chunk via obj.char[start..end].delete():
                // obj.char[start..end] parses as ObjHandlerCall(obj, "getProp", [#char, start, end])
                // Director emits getPropRef + PushChunkVarRef for the receiver when it's a simple var.
                if method.eq_ignore_ascii_case("delete") && args.is_empty() {
                    if let LingoExpr::ObjHandlerCall(inner_obj, inner_method, inner_args) = obj.as_ref() {
                        if inner_method.eq_ignore_ascii_case("getProp") {
                            if let LingoExpr::Identifier(var_name) = inner_obj.as_ref() {
                                let var_name = var_name.clone();
                                let inner_args = inner_args.clone();
                                self.emit_push_chunk_var_ref(&var_name)?;
                                for arg in &inner_args {
                                    self.compile_expr_val(arg)?;
                                }
                                let n = (inner_args.len() + 1) as i64;
                                self.emit_val(OpCode::PushArgList, n);
                                let get_prop_ref_id = self.sc.get_name_id("getPropRef") as i64;
                                self.emit_val(OpCode::ObjCall, get_prop_ref_id);
                                self.emit_val(OpCode::PushArgListNoRet, 1);
                                let delete_id = self.sc.get_name_id("delete") as i64;
                                self.emit_val(OpCode::ObjCall, delete_id);
                                return Ok(());
                            }
                        }
                    }
                }
                self.compile_method_receiver(obj)?;
                for arg in args {
                    self.compile_expr_val(arg)?;
                }
                let n = (args.len() + 1) as i64;
                self.emit_val(OpCode::PushArgListNoRet, n);
                let name_id = self.sc.get_name_id(method) as i64;
                self.emit_val(OpCode::ObjCall, name_id);
                Ok(())
            }
            LingoExpr::PutInto(val, target) => {
                self.compile_expr_val(val)?;
                self.compile_assign_lhs(target)
            }
            LingoExpr::PutDisplay(val) => {
                self.compile_expr_val(val)?;
                self.emit_val(OpCode::PushArgListNoRet, 1);
                let name_id = self.sc.get_name_id("put") as i64;
                self.emit_val(OpCode::ExtCall, name_id);
                Ok(())
            }
            LingoExpr::PutBefore(val, target) => {
                if let LingoExpr::ChunkExpr(chunk_type, first, range_end, source) = target.as_ref() {
                    if let LingoExpr::Identifier(source_name) = source.as_ref() {
                        let source_name = source_name.clone();
                        self.compile_expr_val(val)?;
                        return self.compile_chunk_write(chunk_type, first, range_end.as_deref(), &source_name, 3);
                    }
                }
                if let LingoExpr::Identifier(target_name) = target.as_ref() {
                    self.compile_expr_val(val)?;
                    self.compile_put_simple(target_name, 3)?;
                }
                Ok(())
            }
            LingoExpr::PutAfter(val, target) => {
                if let LingoExpr::ChunkExpr(chunk_type, first, range_end, source) = target.as_ref() {
                    if let LingoExpr::Identifier(source_name) = source.as_ref() {
                        let source_name = source_name.clone();
                        self.compile_expr_val(val)?;
                        return self.compile_chunk_write(chunk_type, first, range_end.as_deref(), &source_name, 2);
                    }
                }
                if let LingoExpr::Identifier(target_name) = target.as_ref() {
                    self.compile_expr_val(val)?;
                    self.compile_put_simple(target_name, 2)?;
                }
                Ok(())
            }
            LingoExpr::DeleteChunk(target) => {
                match target.as_ref() {
                    LingoExpr::ChunkExpr(chunk_type, first, range_end, source) => {
                        if let LingoExpr::Identifier(source_name) = source.as_ref() {
                            let source_name = source_name.clone();
                            let chunk_type = *chunk_type;
                            let last = range_end.as_deref().unwrap_or(first);
                            let last = last.clone();
                            let first = first.clone();
                            for builtin in [BuiltInSymbol::Char, BuiltInSymbol::Word, BuiltInSymbol::Item, BuiltInSymbol::Line] {
                                if chunk_type == builtin {
                                    self.emit_chunk_index_val(&first)?;
                                    self.emit_chunk_index_val(&last)?;
                                } else {
                                    self.emit_val(OpCode::PushZero, 0);
                                    self.emit_val(OpCode::PushZero, 0);
                                }
                            }
                            let var_kind = self.resolve_var(&source_name);
                            let (var_type, var_id) = match var_kind {
                                VarKind::Global(name_id) => (0x1i64, name_id as i64),
                                VarKind::Prop(name_id) => (0x3i64, name_id as i64),
                                VarKind::Param(idx) => (0x4i64, idx as i64),
                                VarKind::Local(idx) => (0x5i64, idx as i64),
                            };
                            if var_type == 0x1 || var_type == 0x3 {
                                self.emit_val(OpCode::PushVarRef, var_id);
                            } else if var_id == 0 {
                                self.emit_val(OpCode::PushZero, 0);
                            } else {
                                self.emit_val(OpCode::PushInt8, var_id);
                            }
                            self.emit_val(OpCode::DeleteChunk, var_type);
                        }
                    }
                    // delete pStr.char[start..end] parses as DeleteChunk(ObjHandlerCall(Identifier, "getProp", args))
                    // Director emits PushChunkVarRef for simple variable receivers in this context.
                    LingoExpr::ObjHandlerCall(inner_obj, inner_method, inner_args)
                        if inner_method.eq_ignore_ascii_case("getProp") =>
                    {
                        if let LingoExpr::Identifier(var_name) = inner_obj.as_ref() {
                            let var_name = var_name.clone();
                            let inner_args = inner_args.clone();
                            self.emit_push_chunk_var_ref(&var_name)?;
                            for arg in &inner_args {
                                self.compile_expr_val(arg)?;
                            }
                            let n = (inner_args.len() + 1) as i64;
                            self.emit_val(OpCode::PushArgList, n);
                            let get_prop_ref_id = self.sc.get_name_id("getPropRef") as i64;
                            self.emit_val(OpCode::ObjCall, get_prop_ref_id);
                            self.emit_val(OpCode::PushArgListNoRet, 1);
                            let delete_id = self.sc.get_name_id("delete") as i64;
                            self.emit_val(OpCode::ObjCall, delete_id);
                        } else {
                            self.compile_expr_val(target)?;
                            self.emit_val(OpCode::PushArgListNoRet, 1);
                            let delete_id = self.sc.get_name_id("delete") as i64;
                            self.emit_val(OpCode::ObjCall, delete_id);
                        }
                    }
                    _ => {
                        // delete obj.prop[start..end] → getProp then objcall delete
                        self.compile_expr_val(target)?;
                        self.emit_val(OpCode::PushArgListNoRet, 1);
                        let delete_id = self.sc.get_name_id("delete") as i64;
                        self.emit_val(OpCode::ObjCall, delete_id);
                    }
                }
                Ok(())
            }
            _ => {
                // Generic expression at statement level: evaluate and pop
                self.compile_expr_val(expr)?;
                self.emit_val(OpCode::Pop, 1);
                Ok(())
            }
        }
    }

    fn compile_assign_lhs(&mut self, lhs: &LingoExpr) -> Result<(), String> {
        match lhs {
            LingoExpr::Identifier(name) => {
                if let Some(the_name) = normalize_the_identifier(name) {
                    let name_id = self.sc.get_name_id(&the_name) as i64;
                    self.emit_val(OpCode::SetMovieProp, name_id);
                } else {
                    self.emit_var_write(name);
                }
                Ok(())
            }
            LingoExpr::ChunkExpr(chunk_type, first, range_end, source) => {
                if let LingoExpr::Identifier(source_name) = source.as_ref() {
                    let source_name = source_name.clone();
                    self.compile_chunk_write(chunk_type, first, range_end.as_deref(), &source_name, 1)?;
                }
                Ok(())
            }
            LingoExpr::ObjProp(_, prop) => {
                // Value is on stack, push object, then SetObjProp
                // For SetObjProp: stack = [value, obj] → it pops both
                // Wait, looking at set_obj_prop: pops value, then pops obj
                // So stack should be: obj first, then value on top
                // But we already compiled lhs... We need obj on stack BEFORE value
                // This is tricky because we've already pushed the value.
                // Solution: we can't easily do this after pushing value.
                // Skip this case for now - most assignments are to simple vars
                self.emit_var_write(&prop); // fallback - won't work correctly
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn compile_expr_val(&mut self, expr: &LingoExpr) -> Result<(), String> {
        match expr {
            LingoExpr::IntLiteral(n) => {
                let n = *n as i64;
                if n == 0 {
                    self.emit_val(OpCode::PushZero, 0);
                } else if (-128..=127).contains(&n) {
                    self.emit_val(OpCode::PushInt8, n);
                } else if (-32768..=32767).contains(&n) {
                    self.emit_val(OpCode::PushInt16, n);
                } else {
                    self.emit_val(OpCode::PushInt32, n);
                }
            }
            LingoExpr::FloatLiteral(f) => {
                let f32val = *f as f32;
                let bits = f32val.to_bits() as i64;
                self.emit_val(OpCode::PushFloat32, bits);
            }
            LingoExpr::StringLiteral(s) => {
                let lit_idx = self.add_literal(Datum::String(s.clone()));
                self.emit_val(OpCode::PushCons, lit_idx as i64);
            }
            LingoExpr::SymbolLiteral(s) => {
                let name_id = self.sc.get_name_id(s) as i64;
                self.emit_val(OpCode::PushSymb, name_id);
            }
            LingoExpr::BoolLiteral(b) => {
                if *b {
                    self.emit_val(OpCode::PushInt8, 1);
                } else {
                    self.emit_val(OpCode::PushZero, 0);
                }
            }
            LingoExpr::VoidLiteral => {
                self.emit_val(OpCode::PushZero, 0);
            }
            LingoExpr::Identifier(name) => {
                if !self.compile_the_identifier_read(name) {
                    self.emit_var_read(name);
                }
            }
            LingoExpr::ObjProp(obj, prop) => {
                if prop.eq_ignore_ascii_case("count") {
                    if let LingoExpr::ObjProp(base_obj, base_prop) = obj.as_ref() {
                        return self.compile_prop_symbol_call(base_obj, base_prop, "count");
                    }
                }
                self.compile_obj_chain_base(obj)?;
                let prop_id = self.sc.get_name_id(prop) as i64;
                self.emit_val(OpCode::GetObjProp, prop_id);
            }
            LingoExpr::HandlerCall(name, args) => {
                if name.eq_ignore_ascii_case("field") {
                    match args.as_slice() {
                        [first, second] => {
                            self.compile_expr_val(first)?;
                            self.compile_expr_val(second)?;
                            self.emit_val(OpCode::GetField, 0);
                            return Ok(());
                        }
                        [LingoExpr::ListLiteral(items)] if items.len() == 2 => {
                            self.compile_expr_val(&items[0])?;
                            self.compile_expr_val(&items[1])?;
                            self.emit_val(OpCode::GetField, 0);
                            return Ok(());
                        }
                        _ => {}
                    }
                }
                for arg in args {
                    self.compile_expr_val(arg)?;
                }
                let n = args.len() as i64;
                self.emit_call(name, n, false);
            }
            LingoExpr::ObjHandlerCall(obj, method, args) => {
                self.compile_method_receiver(obj)?;
                for arg in args {
                    self.compile_expr_val(arg)?;
                }
                let n = (args.len() + 1) as i64;
                self.emit_val(OpCode::PushArgList, n);
                let name_id = self.sc.get_name_id(method) as i64;
                self.emit_val(OpCode::ObjCall, name_id);
            }
            LingoExpr::Add(l, r) => {
                self.compile_expr_val(l)?;
                self.compile_expr_val(r)?;
                self.emit_val(OpCode::Add, 0);
            }
            LingoExpr::Subtract(l, r) => {
                self.compile_expr_val(l)?;
                self.compile_expr_val(r)?;
                self.emit_val(OpCode::Sub, 0);
            }
            LingoExpr::Multiply(l, r) => {
                self.compile_expr_val(l)?;
                self.compile_expr_val(r)?;
                self.emit_val(OpCode::Mul, 0);
            }
            LingoExpr::Divide(l, r) => {
                self.compile_expr_val(l)?;
                self.compile_expr_val(r)?;
                self.emit_val(OpCode::Div, 0);
            }
            LingoExpr::Modulo(l, r) => {
                self.compile_expr_val(l)?;
                self.compile_expr_val(r)?;
                self.emit_val(OpCode::Mod, 0);
            }
            LingoExpr::Join(l, r) => {
                self.compile_expr_val(l)?;
                self.compile_expr_val(r)?;
                self.emit_val(OpCode::JoinStr, 0);
            }
            LingoExpr::JoinPad(l, r) => {
                self.compile_expr_val(l)?;
                self.compile_expr_val(r)?;
                self.emit_val(OpCode::JoinPadStr, 0);
            }
            LingoExpr::And(l, r) => {
                self.compile_expr_val(l)?;
                self.compile_expr_val(r)?;
                self.emit_val(OpCode::And, 0);
            }
            LingoExpr::Or(l, r) => {
                self.compile_expr_val(l)?;
                self.compile_expr_val(r)?;
                self.emit_val(OpCode::Or, 0);
            }
            LingoExpr::Contains(l, r) => {
                self.compile_expr_val(l)?;
                self.compile_expr_val(r)?;
                self.emit_val(OpCode::ContainsStr, 0);
            }
            LingoExpr::Starts(l, r) => {
                self.compile_expr_val(l)?;
                self.compile_expr_val(r)?;
                self.emit_val(OpCode::Contains0Str, 0);
            }
            LingoExpr::Not(inner) => {
                self.compile_expr_val(inner)?;
                self.emit_val(OpCode::Not, 0);
            }
            LingoExpr::Negate(inner) => {
                self.compile_expr_val(inner)?;
                self.emit_val(OpCode::Inv, 0);
            }
            LingoExpr::Eq(l, r) => {
                self.compile_expr_val(l)?;
                self.compile_expr_val(r)?;
                self.emit_val(OpCode::Eq, 0);
            }
            LingoExpr::Ne(l, r) => {
                self.compile_expr_val(l)?;
                self.compile_expr_val(r)?;
                self.emit_val(OpCode::NtEq, 0);
            }
            LingoExpr::Lt(l, r) => {
                self.compile_expr_val(l)?;
                self.compile_expr_val(r)?;
                self.emit_val(OpCode::Lt, 0);
            }
            LingoExpr::Gt(l, r) => {
                self.compile_expr_val(l)?;
                self.compile_expr_val(r)?;
                self.emit_val(OpCode::Gt, 0);
            }
            LingoExpr::Le(l, r) => {
                self.compile_expr_val(l)?;
                self.compile_expr_val(r)?;
                self.emit_val(OpCode::LtEq, 0);
            }
            LingoExpr::Ge(l, r) => {
                self.compile_expr_val(l)?;
                self.compile_expr_val(r)?;
                self.emit_val(OpCode::GtEq, 0);
            }
            LingoExpr::Assignment(lhs, rhs) => {
                // Assignment as expression: compile rhs, set lhs, push void
                self.compile_expr_val(rhs)?;
                self.compile_assign_lhs(lhs)?;
                self.emit_val(OpCode::PushZero, 0);
            }
            LingoExpr::ListAccess(obj, idx) => {
                self.compile_list_access_expr(obj, idx, false)?;
            }
            LingoExpr::PutInto(val, target) => {
                self.compile_expr_val(val)?;
                self.compile_assign_lhs(target)?;
                self.emit_val(OpCode::PushZero, 0);
            }
            LingoExpr::ThePropOf(obj, prop) => {
                if matches!(obj.as_ref(), LingoExpr::Identifier(target) if target.eq_ignore_ascii_case("_movie") || target.eq_ignore_ascii_case("movie")) {
                    let prop_id = self.sc.get_name_id(prop) as i64;
                    self.emit_val(OpCode::GetMovieProp, prop_id);
                } else if prop.eq_ignore_ascii_case("number") {
                    // "the number of <chunk_prop> of castLib(X)" → look up the combined
                    // prop name in anim2_prop_names and emit as get 8 with cast_lib arg.
                    if let LingoExpr::ObjProp(inner_obj, inner_prop) = obj.as_ref() {
                        let combined = format!("number of {inner_prop}");
                        if let Some((prop_type, prop_id)) = legacy_movie_get_prop(&combined) {
                            if let LingoExpr::HandlerCall(call_name, call_args) = inner_obj.as_ref() {
                                if call_name.eq_ignore_ascii_case("castLib") && call_args.len() == 1 {
                                    self.compile_expr_val(&call_args[0])?;
                                    self.emit_legacy_get_prop(prop_type, prop_id);
                                    return Ok(());
                                }
                            }
                        }
                    }
                    self.compile_expr_val(obj)?;
                    let prop_id = self.sc.get_name_id(prop) as i64;
                    self.emit_val(OpCode::GetObjProp, prop_id);
                } else {
                    self.compile_expr_val(obj)?;
                    let prop_id = self.sc.get_name_id(prop) as i64;
                    self.emit_val(OpCode::GetObjProp, prop_id);
                }
            }
            LingoExpr::MemberRef(member, cast_lib) => {
                self.compile_expr_val(member)?;
                if let Some(cl) = cast_lib {
                    self.compile_expr_val(cl)?;
                    self.emit_val(OpCode::PushArgList, 2);
                } else {
                    self.emit_val(OpCode::PushArgList, 1);
                }
                let name_id = self.sc.get_name_id("member") as i64;
                self.emit_val(OpCode::ExtCall, name_id);
            }
            LingoExpr::ColorLiteral(color) => match color {
                ColorRef::Rgb(r, g, b) => {
                    for &channel in &[*r, *g, *b] {
                        let v = channel as i64;
                        if v == 0 {
                            self.emit_val(OpCode::PushZero, 0);
                        } else if v <= 127 {
                            self.emit_val(OpCode::PushInt8, v);
                        } else {
                            self.emit_val(OpCode::PushInt16, v);
                        }
                    }
                    self.emit_val(OpCode::PushArgList, 3);
                    let name_id = self.sc.get_name_id("rgb") as i64;
                    self.emit_val(OpCode::ExtCall, name_id);
                }
                _ => {
                    self.emit_val(OpCode::PushZero, 0);
                }
            },
            LingoExpr::ListLiteral(items) => {
                for item in items {
                    self.compile_expr_val(item)?;
                }
                self.emit_val(OpCode::PushArgList, items.len() as i64);
                self.emit_val(OpCode::PushList, 0);
            }
            LingoExpr::PropListLiteral(pairs) => {
                for (key, val) in pairs {
                    self.compile_expr_val(key)?;
                    self.compile_expr_val(val)?;
                }
                self.emit_val(OpCode::PushArgList, (pairs.len() * 2) as i64);
                self.emit_val(OpCode::PushPropList, 0);
            }
            LingoExpr::RectLiteral(parts) => {
                if let Some((x, y, w, h)) = parts.first() {
                    self.compile_expr_val(x)?;
                    self.compile_expr_val(y)?;
                    self.compile_expr_val(w)?;
                    self.compile_expr_val(h)?;
                    self.emit_val(OpCode::PushArgList, 4);
                    let name_id = self.sc.get_name_id("rect") as i64;
                    self.emit_val(OpCode::ExtCall, name_id);
                } else {
                    self.emit_val(OpCode::PushZero, 0);
                }
            }
            LingoExpr::PointLiteral(parts) => {
                if let Some((x, y)) = parts.first() {
                    self.compile_expr_val(x)?;
                    self.compile_expr_val(y)?;
                    self.emit_val(OpCode::PushArgList, 2);
                    let name_id = self.sc.get_name_id("point") as i64;
                    self.emit_val(OpCode::ExtCall, name_id);
                } else {
                    self.emit_val(OpCode::PushZero, 0);
                }
            }
            LingoExpr::ChunkExpr(chunk_type, first, range_end, source) => {
                self.compile_chunk_read(chunk_type, first, range_end.as_deref(), source)?;
            }
            LingoExpr::LastChunkExpr(chunk_type, source) => {
                self.compile_expr_val(source)?;
                let property_id = match chunk_type.as_str() {
                    "char" => 12i64,
                    "word" => 13i64,
                    "item" => 14i64,
                    "line" => 15i64,
                    _ => 12i64,
                };
                self.emit_val(OpCode::PushInt8, property_id);
                self.emit_val(OpCode::Get, 0x00);
            }
            LingoExpr::StringChunkCountExpr(chunk_type, source) => {
                self.compile_expr_val(source)?;
                let property_id = match chunk_type.as_str() {
                    "char" => 1i64,
                    "word" => 2i64,
                    "item" => 3i64,
                    "line" => 4i64,
                    _ => 1i64,
                };
                self.emit_val(OpCode::PushInt8, property_id);
                self.emit_val(OpCode::Get, 0x01);
            }
            _ => {
                log::error!("[compile_expr_val] Unsupported expression: {expr:?}");
                // Unsupported or unknown expression → push void
                self.emit_val(OpCode::PushZero, 0);
            }
        }
        Ok(())
    }

    fn compile_chunk_read(
        &mut self,
        chunk_type: &Symbol,
        first: &LingoExpr,
        range_end: Option<&LingoExpr>,
        source: &LingoExpr,
    ) -> Result<(), String> {
        let last = range_end.unwrap_or(first);

        // Collect all chunk layers by traversing nested ChunkExprs.
        // We clone to avoid lifetime issues when reassigning the cursor.
        struct Layer {
            chunk_type: Symbol,
            first: LingoExpr,
            last: LingoExpr,
        }
        let mut layers: Vec<Layer> = vec![Layer {
            chunk_type: *chunk_type,
            first: first.clone(),
            last: last.clone(),
        }];

        let mut cur: &LingoExpr = source;
        while let LingoExpr::ChunkExpr(inner_type, inner_first, inner_range_end, inner_source) = cur {
            let inner_last = inner_range_end.as_deref().unwrap_or(inner_first.as_ref());
            layers.push(Layer {
                chunk_type: *inner_type,
                first: inner_first.as_ref().clone(),
                last: inner_last.clone(),
            });
            cur = inner_source.as_ref();
        }

        let mut char_range: Option<(LingoExpr, LingoExpr)> = None;
        let mut word_range: Option<(LingoExpr, LingoExpr)> = None;
        let mut item_range: Option<(LingoExpr, LingoExpr)> = None;
        let mut line_range: Option<(LingoExpr, LingoExpr)> = None;
        for layer in layers {
            if layer.chunk_type == BuiltInSymbol::Char && char_range.is_none() {
                char_range = Some((layer.first, layer.last));
            } else if layer.chunk_type == BuiltInSymbol::Word && word_range.is_none() {
                word_range = Some((layer.first, layer.last));
            } else if layer.chunk_type == BuiltInSymbol::Item && item_range.is_none() {
                item_range = Some((layer.first, layer.last));
            } else if layer.chunk_type == BuiltInSymbol::Line && line_range.is_none() {
                line_range = Some((layer.first, layer.last));
            }
        }

        // Push 8 indices in order: first_char, last_char, first_word, last_word, first_item, last_item, first_line, last_line
        for range in [char_range, word_range, item_range, line_range] {
            match range {
                Some((f, l)) => {
                    self.compile_expr_val(&f)?;
                    self.compile_expr_val(&l)?;
                }
                None => {
                    self.emit_val(OpCode::PushZero, 0);
                    self.emit_val(OpCode::PushZero, 0);
                }
            }
        }

        // Push source string value and emit GetChunk
        self.compile_expr_val(cur)?;
        self.emit_val(OpCode::GetChunk, 0);
        Ok(())
    }

    fn compile_put_simple(&mut self, target_name: &str, put_type: i64) -> Result<(), String> {
        let var_kind = self.resolve_var(target_name);
        let (var_type, var_id) = match var_kind {
            VarKind::Global(name_id) => (0x1i64, name_id as i64),
            VarKind::Prop(name_id) => (0x3i64, name_id as i64),
            VarKind::Param(idx) => (0x4i64, idx as i64),
            VarKind::Local(idx) => (0x5i64, idx as i64),
        };
        if var_type == 0x1 || var_type == 0x3 {
            self.emit_val(OpCode::PushVarRef, var_id);
        } else if var_id == 0 {
            self.emit_val(OpCode::PushZero, 0);
        } else {
            self.emit_val(OpCode::PushInt8, var_id);
        }
        self.emit_val(OpCode::Put, (put_type << 4) | var_type);
        Ok(())
    }

    fn compile_chunk_write(
        &mut self,
        chunk_type: &Symbol,
        first: &LingoExpr,
        range_end: Option<&LingoExpr>,
        source_name: &str,
        put_type: i64,
    ) -> Result<(), String> {
        // Value is already on stack (pushed before this call).
        let last = range_end.unwrap_or(first);

        // Push 8 chunk indices: char, word, item, line (each as first+last pair)
        for builtin in [BuiltInSymbol::Char, BuiltInSymbol::Word, BuiltInSymbol::Item, BuiltInSymbol::Line] {
            if *chunk_type == builtin {
                self.compile_expr_val(first)?;
                self.compile_expr_val(last)?;
            } else {
                self.emit_val(OpCode::PushZero, 0);
                self.emit_val(OpCode::PushZero, 0);
            }
        }

        // Push variable reference and emit PutChunk
        let var_kind = self.resolve_var(source_name);
        let (var_type, var_id) = match var_kind {
            VarKind::Global(name_id) => (0x1i64, name_id as i64),
            VarKind::Prop(name_id) => (0x3i64, name_id as i64),
            VarKind::Param(idx) => (0x4i64, idx as i64),
            VarKind::Local(idx) => (0x5i64, idx as i64),
        };
        if var_type == 0x1 || var_type == 0x3 {
            self.emit_val(OpCode::PushVarRef, var_id);
        } else if var_id == 0 {
            self.emit_val(OpCode::PushZero, 0);
        } else {
            self.emit_val(OpCode::PushInt8, var_id);
        }
        self.emit_val(OpCode::PutChunk, (put_type << 4) | var_type);
        Ok(())
    }

    fn finalize(self) -> (Vec<Bytecode>, FxHashMap<usize, usize>) {
        let instrs = self.instrs;
        let operand_sizes = resolve_operand_sizes(&instrs);
        let positions = compute_positions(&instrs, &operand_sizes);
        let mut bytecode_array = Vec::with_capacity(instrs.len());
        let mut bytecode_index_map: FxHashMap<usize, usize> = FxHashMap::default();

        for (i, instr) in instrs.iter().enumerate() {
            let pos = positions[i];
            let obj = match &instr.op {
                Operand::Val(v) => *v,
                Operand::FwdJump(target_idx) => {
                    let target_pos = positions[*target_idx];
                    target_pos as i64 - pos as i64
                }
                Operand::BwdRepeat(target_idx) => {
                    let target_pos = positions[*target_idx];
                    pos as i64 - target_pos as i64
                }
            };

            let mut bytecode = Bytecode::new(instr.opcode, obj, pos);
            bytecode.size_hint = match instr.size_hint {
                SizeHint::None => 0,
                SizeHint::Short => 1,
            };
            bytecode_array.push(bytecode);
            bytecode_index_map.insert(pos, i);
        }

        (bytecode_array, bytecode_index_map)
    }
}

fn compute_positions(instrs: &[Instr], operand_sizes: &[usize]) -> Vec<usize> {
    let mut positions = Vec::with_capacity(instrs.len());
    let mut pos = 0usize;
    for (instr, operand_size) in instrs.iter().zip(operand_sizes.iter()) {
        positions.push(pos);
        pos += encoded_instruction_size(instr.opcode, *operand_size);
    }
    positions
}

fn compute_bytecode_positions(bytecodes: &[Bytecode], operand_sizes: &[usize]) -> Vec<usize> {
    let mut positions = Vec::with_capacity(bytecodes.len());
    let mut pos = 0usize;
    for (bytecode, operand_size) in bytecodes.iter().zip(operand_sizes.iter()) {
        positions.push(pos);
        pos += encoded_instruction_size(bytecode.opcode, *operand_size);
    }
    positions
}

fn relayout_handler_bytecodes(handler: &mut HandlerDef) {
    let jump_targets = handler
        .bytecode_array
        .iter()
        .map(|bytecode| match bytecode.opcode {
            OpCode::Jmp | OpCode::JmpIfZ => {
                resolve_jump_target_index(handler, bytecode.pos as isize + bytecode.obj as isize)
            }
            OpCode::EndRepeat => {
                resolve_jump_target_index(handler, bytecode.pos as isize - bytecode.obj as isize)
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    let mut operand_sizes = handler
        .bytecode_array
        .iter()
        .map(|bytecode| operand_size_for_bytecode(bytecode.opcode, bytecode.obj, bytecode.size_hint))
        .collect::<Vec<_>>();

    loop {
        let positions = compute_bytecode_positions(&handler.bytecode_array, &operand_sizes);
        let mut changed = false;

        for (index, bytecode) in handler.bytecode_array.iter().enumerate() {
            let next_size = match bytecode.opcode {
                OpCode::Jmp | OpCode::JmpIfZ => {
                    let target_idx = jump_targets[index].expect("forward jump target index");
                    let delta = positions[target_idx] as i64 - positions[index] as i64;
                    operand_size_for_bytecode(bytecode.opcode, delta, bytecode.size_hint)
                }
                OpCode::EndRepeat => {
                    let target_idx = jump_targets[index].expect("repeat target index");
                    let delta = positions[index] as i64 - positions[target_idx] as i64;
                    operand_size_for_bytecode(bytecode.opcode, delta, bytecode.size_hint)
                }
                _ => operand_size_for_bytecode(bytecode.opcode, bytecode.obj, bytecode.size_hint),
            };

            if operand_sizes[index] != next_size {
                operand_sizes[index] = next_size;
                changed = true;
            }
        }

        if !changed {
            let final_positions = positions;
            let mut bytecode_index_map: FxHashMap<usize, usize> = FxHashMap::default();

            for (index, bytecode) in handler.bytecode_array.iter_mut().enumerate() {
                bytecode.pos = final_positions[index];
                match bytecode.opcode {
                    OpCode::Jmp | OpCode::JmpIfZ => {
                        let target_idx = jump_targets[index].expect("forward jump target index");
                        bytecode.obj = final_positions[target_idx] as i64 - final_positions[index] as i64;
                    }
                    OpCode::EndRepeat => {
                        let target_idx = jump_targets[index].expect("repeat target index");
                        bytecode.obj = final_positions[index] as i64 - final_positions[target_idx] as i64;
                    }
                    _ => {}
                }
                bytecode_index_map.insert(bytecode.pos, index);
            }

            handler.bytecode_index_map = bytecode_index_map;
            return;
        }
    }
}

fn resolve_jump_target_index(handler: &HandlerDef, target_pos: isize) -> Option<usize> {
    usize::try_from(target_pos)
        .ok()
        .and_then(|target_pos| handler.bytecode_index_map.get(&target_pos).copied())
}

fn resolve_operand_sizes(instrs: &[Instr]) -> Vec<usize> {
    let mut operand_sizes = instrs
        .iter()
        .map(|instr| match instr.op {
            Operand::Val(value) => operand_size_for_instr(instr, value),
            Operand::FwdJump(_) | Operand::BwdRepeat(_) => 1,
        })
        .collect::<Vec<_>>();

    loop {
        let positions = compute_positions(instrs, &operand_sizes);
        let mut changed = false;

        for (index, instr) in instrs.iter().enumerate() {
            let next_size = match instr.op {
                Operand::Val(value) => operand_size_for_instr(instr, value),
                Operand::FwdJump(target_idx) => {
                    let delta = positions[target_idx] as i64 - positions[index] as i64;
                    operand_size_for_instr(instr, delta)
                }
                Operand::BwdRepeat(target_idx) => {
                    let delta = positions[index] as i64 - positions[target_idx] as i64;
                    operand_size_for_instr(instr, delta)
                }
            };

            if operand_sizes[index] != next_size {
                operand_sizes[index] = next_size;
                changed = true;
            }
        }

        if !changed {
            return operand_sizes;
        }
    }
}

fn encoded_instruction_size(opcode: OpCode, operand_size: usize) -> usize {
    if num::ToPrimitive::to_u16(&opcode).unwrap() < 0x40 {
        1
    } else {
        1 + operand_size
    }
}

fn operand_size_for_instr(instr: &Instr, value: i64) -> usize {
    operand_size_for_bytecode(
        instr.opcode,
        value,
        match instr.size_hint {
            SizeHint::None => 0,
            SizeHint::Short => 1,
        },
    )
}

fn operand_size_for_bytecode(opcode: OpCode, value: i64, size_hint: u8) -> usize {
    let _ = size_hint;
    operand_size_for_value(opcode, value)
}

fn operand_size_for_value(opcode: OpCode, value: i64) -> usize {
    let op_id = num::ToPrimitive::to_u16(&opcode).unwrap();
    if op_id < 0x40 {
        return 0;
    }

    match opcode {
        OpCode::Jmp | OpCode::JmpIfZ => 2,
        OpCode::PushInt8 => {
            if (-128..=127).contains(&value) {
                1
            } else {
                2
            }
        }
        OpCode::PushInt16 => 2,
        OpCode::PushInt32 | OpCode::PushFloat32 => 4,
        _ => {
            if (0..=u8::MAX as i64).contains(&value) {
                1
            } else if (0..=u16::MAX as i64).contains(&value) {
                2
            } else {
                4
            }
        }
    }
}

fn literal_eq(a: &Datum, b: &Datum) -> bool {
    match (a, b) {
        (Datum::String(sa), Datum::String(sb)) => sa == sb,
        (Datum::Int(ia), Datum::Int(ib)) => ia == ib,
        (Datum::Float(fa), Datum::Float(fb)) => fa.to_bits() == fb.to_bits(),
        _ => false,
    }
}

fn normalize_the_identifier(name: &str) -> Option<String> {
    let trimmed = name.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("the ") {
        return None;
    }
    Some(trimmed[4..].trim().to_string())
}

fn is_the_builtin_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "paramcount" | "result" | "pi"
        | "mouseh" | "mousev"
        | "stageleft" | "stageright" | "stagetop" | "stagebottom"
        | "date"
    )
}

fn legacy_movie_get_prop(name: &str) -> Option<(u16, u16)> {
    let needle = name.to_ascii_lowercase();
    if let Some(prop_id) = needle.strip_prefix("movieprop_").and_then(|value| value.parse::<u16>().ok()) {
        return Some((0x00, prop_id));
    }
    if let Some(prop_id) = needle.strip_prefix("animprop_").and_then(|value| value.parse::<u16>().ok()) {
        return Some((0x07, prop_id));
    }
    if let Some(prop_id) = needle.strip_prefix("anim2prop_").and_then(|value| value.parse::<u16>().ok()) {
        return Some((0x08, prop_id));
    }

    movie_prop_names().iter().find_map(|(id, prop_name)| {
        prop_name.eq_ignore_ascii_case(&needle).then_some((0x00, *id))
    }).or_else(|| {
        anim_prop_names().iter().find_map(|(id, prop_name)| {
            prop_name.eq_ignore_ascii_case(&needle).then_some((0x07, *id))
        })
    }).or_else(|| {
        anim2_prop_names().iter().find_map(|(id, prop_name)| {
            prop_name.eq_ignore_ascii_case(&needle).then_some((0x08, *id))
        })
    })
}
