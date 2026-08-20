// Lingo decompiler AST nodes
// Ported from ProjectorRays

use std::rc::Rc;
use std::cell::{Cell, RefCell};
use crate::director::lingo::opcode::OpCode;
use super::enums::{ChunkExprType, PutType, DatumType, CaseExpect};
use super::code_writer::CodeWriter;

/// Bound on write_script recursion, to stop a malformed tree overflowing the
/// stack. It has to clear real scripts: a string built from a chain of `&`
/// nests one level per concatenation, and a 102-part chain was being truncated
/// to `/* MAX DEPTH */`, losing 3KB of text.
const MAX_WRITE_DEPTH: usize = 2000;

/// Datum represents values in the decompiler
#[derive(Clone, Debug)]
pub struct Datum {
    pub datum_type: DatumType,
    pub int_value: i32,
    pub float_value: f64,
    pub string_value: String,
    pub list_value: Vec<Rc<AstNode>>,
}

impl Datum {
    pub fn void() -> Self {
        Self {
            datum_type: DatumType::Void,
            int_value: 0,
            float_value: 0.0,
            string_value: String::new(),
            list_value: Vec::new(),
        }
    }

    pub fn int(val: i32) -> Self {
        Self {
            datum_type: DatumType::Int,
            int_value: val,
            float_value: 0.0,
            string_value: String::new(),
            list_value: Vec::new(),
        }
    }

    pub fn float(val: f64) -> Self {
        Self {
            datum_type: DatumType::Float,
            int_value: 0,
            float_value: val,
            string_value: String::new(),
            list_value: Vec::new(),
        }
    }

    pub fn string(val: String) -> Self {
        Self {
            datum_type: DatumType::String,
            int_value: 0,
            float_value: 0.0,
            string_value: val,
            list_value: Vec::new(),
        }
    }

    pub fn symbol(val: String) -> Self {
        Self {
            datum_type: DatumType::Symbol,
            int_value: 0,
            float_value: 0.0,
            string_value: val,
            list_value: Vec::new(),
        }
    }

    pub fn var_ref(val: String) -> Self {
        Self {
            datum_type: DatumType::VarRef,
            int_value: 0,
            float_value: 0.0,
            string_value: val,
            list_value: Vec::new(),
        }
    }

    pub fn list(items: Vec<Rc<AstNode>>) -> Self {
        Self {
            datum_type: DatumType::List,
            int_value: 0,
            float_value: 0.0,
            string_value: String::new(),
            list_value: items,
        }
    }

    pub fn arg_list(items: Vec<Rc<AstNode>>) -> Self {
        Self {
            datum_type: DatumType::ArgList,
            int_value: 0,
            float_value: 0.0,
            string_value: String::new(),
            list_value: items,
        }
    }

    pub fn arg_list_no_ret(items: Vec<Rc<AstNode>>) -> Self {
        Self {
            datum_type: DatumType::ArgListNoRet,
            int_value: 0,
            float_value: 0.0,
            string_value: String::new(),
            list_value: items,
        }
    }

    pub fn prop_list(items: Vec<Rc<AstNode>>) -> Self {
        Self {
            datum_type: DatumType::PropList,
            int_value: 0,
            float_value: 0.0,
            string_value: String::new(),
            list_value: items,
        }
    }

    pub fn to_int(&self) -> i32 {
        match self.datum_type {
            DatumType::Int => self.int_value,
            DatumType::Float => self.float_value as i32,
            _ => 0,
        }
    }

    pub fn write_script(&self, code: &mut CodeWriter, dot: bool, sum: bool) {
        self.write_script_with_depth(code, dot, sum, 0);
    }

    fn write_script_with_depth(&self, code: &mut CodeWriter, dot: bool, _sum: bool, depth: usize) {
        if depth > MAX_WRITE_DEPTH {
            code.write("/* MAX DEPTH */");
            return;
        }
        match self.datum_type {
            DatumType::Void => code.write("VOID"),
            DatumType::Int => code.write(&self.int_value.to_string()),
            DatumType::Float => {
                // Shortest representation that reparses to the same double. A
                // fixed 4-decimal format silently truncates: 0.00001 would come
                // back as 0.0, and 1.23456 as 1.2346.
                let mut s = format!("{}", self.float_value);
                if !s.contains('.') && !s.contains('e') && !s.contains("inf") && !s.contains("NaN") {
                    s.push_str(".0");
                }
                code.write(&s);
            }
            DatumType::String => {
                code.write(&write_string_literal(&self.string_value));
            }
            DatumType::Symbol => {
                code.write("#");
                code.write(&self.string_value);
            }
            DatumType::VarRef => {
                code.write(&self.string_value);
            }
            DatumType::List => {
                code.write("[");
                for (i, item) in self.list_value.iter().enumerate() {
                    if i > 0 {
                        code.write(", ");
                    }
                    item.write_script_with_depth(code, dot, false, depth + 1);
                }
                code.write("]");
            }
            DatumType::ArgList | DatumType::ArgListNoRet => {
                for (i, item) in self.list_value.iter().enumerate() {
                    if i > 0 {
                        code.write(", ");
                    }
                    item.write_script_with_depth(code, dot, false, depth + 1);
                }
            }
            DatumType::PropList => {
                code.write("[");
                let mut i = 0;
                while i + 1 < self.list_value.len() {
                    if i > 0 {
                        code.write(", ");
                    }
                    self.list_value[i].write_script_with_depth(code, dot, false, depth + 1);
                    code.write(": ");
                    self.list_value[i + 1].write_script_with_depth(code, dot, false, depth + 1);
                    i += 2;
                }
                if self.list_value.is_empty() {
                    code.write(":");
                }
                code.write("]");
            }
        }
    }
}

/// Whether a node renders as something containing spaces, which must be
/// parenthesized before a `.property` access. Mirrors ProjectorRays'
/// `hasSpaces`: literals, variables, calls and index expressions are compact;
/// `the ...` forms, chunk expressions and operators are not.
fn renders_with_spaces(node: &Rc<AstNode>, dot: bool) -> bool {
    // Mirrors ProjectorRays' `hasSpaces`, which is a blacklist: compact forms
    // (literals, variables, calls, index expressions) need no parentheses and
    // everything else — `the ...` forms, chunk expressions, sprite and menu
    // properties, operators — does.
    match node.as_ref() {
        AstNode::Literal(_) | AstNode::Var(_) => false,
        AstNode::Call { name, args } => {
            // `(member "x").line[1]` — verbose member syntax needs wrapping.
            let arg_count = match args.as_ref() {
                AstNode::Literal(list) => list.list_value.len(),
                _ => 0,
            };
            !dot && is_member_expr_call(name, arg_count)
        }
        AstNode::ObjCall { .. } | AstNode::ObjCallV4 { .. } => false,
        AstNode::ObjBracket { .. } | AstNode::ObjPropIndex { .. } => false,
        AstNode::ObjProp { .. } | AstNode::Member { .. } => !dot,
        AstNode::Comment(_) => false,
        _ => true,
    }
}

/// The string a chunk expression reads from is always written verbosely and
/// without parentheses — `char 1 of the platform`, not `char 1 of (the
/// platform)`.
fn write_chunk_source(
    code: &mut CodeWriter,
    string: &Rc<AstNode>,
    _chunk_type: ChunkExprType,
    _dot: bool,
    sum: bool,
    depth: usize,
) {
    string.write_script_with_depth(code, false, sum, depth + 1);
}

/// Whether a call is really a member expression — `member(1, 1)` for
/// `member 1 of castLib 1`. These are rewritten to their real syntax in verbose
/// mode, which makes them render with spaces.
fn is_member_expr_call(name: &str, arg_count: usize) -> bool {
    match name {
        "cast" | "member" | "script" => arg_count == 1 || arg_count == 2,
        "castLib" | "window" => arg_count == 1,
        _ => false,
    }
}

/// Director encodes "no value given" for an optional numeric operand as the
/// literal 0 — a cast library that was never named, or a chunk range that
/// covers a single chunk.
fn is_literal_zero(node: &Rc<AstNode>) -> bool {
    match node.as_ref() {
        AstNode::Literal(datum) => {
            datum.datum_type == DatumType::Int && datum.int_value == 0
        }
        _ => false,
    }
}

/// The Lingo constant naming a character that cannot appear inside a string
/// literal. Lingo has no escape sequences — these are spelled as constants and
/// concatenated, so `"a" & RETURN & "b"` is the only way to write a line break.
fn string_constant_for(c: char) -> Option<&'static str> {
    match c {
        '\x03' => Some("ENTER"),
        '\x08' => Some("BACKSPACE"),
        '\t' => Some("TAB"),
        '\r' | '\n' => Some("RETURN"),
        '"' => Some("QUOTE"),
        _ => None,
    }
}

/// Render a string literal as Lingo source.
///
/// An empty string is `EMPTY`, a lone special character is its constant, and a
/// string mixing text with special characters becomes a `&` concatenation.
fn write_string_literal(s: &str) -> String {
    if s.is_empty() {
        return "EMPTY".to_string();
    }

    let mut chars = s.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if let Some(name) = string_constant_for(c) {
            return name.to_string();
        }
    }

    if !s.chars().any(|c| string_constant_for(c).is_some()) {
        return format!("\"{}\"", s);
    }

    let mut parts: Vec<String> = Vec::new();
    let mut literal = String::new();
    for c in s.chars() {
        match string_constant_for(c) {
            Some(name) => {
                if !literal.is_empty() {
                    parts.push(format!("\"{}\"", literal));
                    literal.clear();
                }
                parts.push(name.to_string());
            }
            None => literal.push(c),
        }
    }
    if !literal.is_empty() {
        parts.push(format!("\"{}\"", literal));
    }
    parts.join(" & ")
}

/// A child statement within a block, pairing the AST node with its bytecode indices
#[derive(Clone, Debug)]
pub struct BlockChild {
    pub node: Rc<AstNode>,
    pub bytecode_indices: Vec<usize>,
}

/// Block node for containing statements
#[derive(Clone, Debug)]
pub struct BlockNode {
    pub children: Vec<BlockChild>,
    pub end_pos: u32,
    pub current_case_label: Option<Rc<RefCell<CaseLabelNode>>>,
    /// The case statement being built in this block, so a label block entered
    /// from here knows which case owns it. Without this the owning case has to
    /// be guessed by scanning siblings, which picks the wrong one when cases
    /// are nested.
    pub current_case_stmt: Option<Rc<AstNode>>,
}

impl BlockNode {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            end_pos: u32::MAX,
            current_case_label: None,
            current_case_stmt: None,
        }
    }

    pub fn add_child(&mut self, child: Rc<AstNode>, bytecode_indices: Vec<usize>) {
        self.children.push(BlockChild { node: child, bytecode_indices });
    }

    pub fn write_script(&self, code: &mut CodeWriter, dot: bool, sum: bool) {
        self.write_script_with_depth(code, dot, sum, 0);
    }

    fn write_script_with_depth(&self, code: &mut CodeWriter, dot: bool, sum: bool, depth: usize) {
        if depth > MAX_WRITE_DEPTH {
            code.write("-- MAX DEPTH EXCEEDED");
            code.end_line();
            return;
        }
        for child in &self.children {
            child.node.write_script_with_depth(code, dot, sum, depth + 1);
            code.end_line();
        }
    }
}

impl Default for BlockNode {
    fn default() -> Self {
        Self::new()
    }
}

/// Main AST node enum
#[derive(Clone, Debug)]
pub enum AstNode {
    Error,
    Comment(String),
    Literal(Datum),
    Block(BlockNode),
    Var(String),
    Assignment { variable: Rc<AstNode>, value: Rc<AstNode>, force_verbose: bool },
    BinaryOp { opcode: OpCode, left: Rc<AstNode>, right: Rc<AstNode> },
    InverseOp(Rc<AstNode>),
    NotOp(Rc<AstNode>),
    ChunkExpr { chunk_type: ChunkExprType, first: Rc<AstNode>, last: Rc<AstNode>, string: Rc<AstNode> },
    ChunkHilite(Rc<AstNode>),
    ChunkDelete(Rc<AstNode>),
    SpriteIntersects { first: Rc<AstNode>, second: Rc<AstNode> },
    SpriteWithin { first: Rc<AstNode>, second: Rc<AstNode> },
    Member { member_type: String, member_id: Rc<AstNode>, cast_id: Option<Rc<AstNode>> },
    The(String),
    TheProp { obj: Rc<AstNode>, prop: String },
    ObjProp { obj: Rc<AstNode>, prop: String },
    ObjBracket { obj: Rc<AstNode>, prop: Rc<AstNode> },
    ObjPropIndex { obj: Rc<AstNode>, prop: String, index: Rc<AstNode>, index2: Option<Rc<AstNode>> },
    LastStringChunk { chunk_type: ChunkExprType, obj: Rc<AstNode> },
    StringChunkCount { chunk_type: ChunkExprType, obj: Rc<AstNode> },
    MenuProp { menu_id: Rc<AstNode>, prop: u32 },
    MenuItemProp { menu_id: Rc<AstNode>, item_id: Rc<AstNode>, prop: u32 },
    SoundProp { sound_id: Rc<AstNode>, prop: u32 },
    SpriteProp { sprite_id: Rc<AstNode>, prop: u32 },
    Call { name: String, args: Rc<AstNode> },
    ObjCall { name: String, args: Rc<AstNode> },
    ObjCallV4 { obj: Rc<AstNode>, args: Rc<AstNode> },
    Exit,
    ExitRepeat,
    NextRepeat,
    Put { put_type: PutType, variable: Rc<AstNode>, value: Rc<AstNode> },
    If { condition: Rc<AstNode>, block1: Rc<RefCell<BlockNode>>, block2: Rc<RefCell<BlockNode>>, has_else: Cell<bool> },
    RepeatWhile { condition: Rc<AstNode>, block: Rc<RefCell<BlockNode>>, start_index: u32 },
    RepeatWithIn { var_name: String, list: Rc<AstNode>, block: Rc<RefCell<BlockNode>>, start_index: u32 },
    RepeatWithTo { var_name: String, start: Rc<AstNode>, end: Rc<AstNode>, up: bool, block: Rc<RefCell<BlockNode>>, start_index: u32 },
    Tell { window: Rc<AstNode>, block: Rc<RefCell<BlockNode>> },
    Case { value: Rc<AstNode>, first_label: RefCell<Option<Rc<RefCell<CaseLabelNode>>>>, otherwise: RefCell<Option<Rc<RefCell<OtherwiseNode>>>>, end_pos: Cell<i32>, potential_otherwise_pos: Cell<i32> },
    NewObj { obj_type: String, args: Rc<AstNode> },
    When { event: i32, script: String },
    SoundCmd { cmd: String, args: Rc<AstNode> },
    PlayCmd { args: Rc<AstNode> },
}

impl AstNode {
    pub fn is_expression(&self) -> bool {
        match self {
            AstNode::Literal(_) |
            AstNode::Var(_) |
            AstNode::BinaryOp { .. } |
            AstNode::InverseOp(_) |
            AstNode::NotOp(_) |
            AstNode::ChunkExpr { .. } |
            AstNode::Member { .. } |
            AstNode::The(_) |
            AstNode::TheProp { .. } |
            AstNode::ObjProp { .. } |
            AstNode::ObjBracket { .. } |
            AstNode::ObjPropIndex { .. } |
            AstNode::LastStringChunk { .. } |
            AstNode::StringChunkCount { .. } |
            AstNode::MenuProp { .. } |
            AstNode::MenuItemProp { .. } |
            AstNode::SoundProp { .. } |
            AstNode::SpriteProp { .. } |
            AstNode::SpriteIntersects { .. } |
            AstNode::SpriteWithin { .. } |
            AstNode::NewObj { .. } => true,

            // Call/ObjCall/ObjCallV4 are expressions if arg list is NOT ArgListNoRet
            AstNode::Call { args, .. } |
            AstNode::ObjCall { args, .. } |
            AstNode::ObjCallV4 { args, .. } => {
                if let AstNode::Literal(datum) = args.as_ref() {
                    datum.datum_type != DatumType::ArgListNoRet
                } else {
                    true
                }
            }

            _ => false,
        }
    }

    pub fn is_statement(&self) -> bool {
        match self {
            AstNode::Assignment { .. } |
            AstNode::Exit |
            AstNode::ExitRepeat |
            AstNode::NextRepeat |
            AstNode::Put { .. } |
            AstNode::If { .. } |
            AstNode::RepeatWhile { .. } |
            AstNode::RepeatWithIn { .. } |
            AstNode::RepeatWithTo { .. } |
            AstNode::Tell { .. } |
            AstNode::Case { .. } |
            AstNode::ChunkHilite(_) |
            AstNode::ChunkDelete(_) |
            AstNode::When { .. } |
            AstNode::SoundCmd { .. } |
            AstNode::PlayCmd { .. } => true,

            // Call/ObjCall/ObjCallV4 are statements if arg list IS ArgListNoRet
            AstNode::Call { args, .. } |
            AstNode::ObjCall { args, .. } |
            AstNode::ObjCallV4 { args, .. } => {
                if let AstNode::Literal(datum) = args.as_ref() {
                    datum.datum_type == DatumType::ArgListNoRet
                } else {
                    false
                }
            }

            _ => false,
        }
    }

    pub fn get_value(&self) -> Option<&Datum> {
        match self {
            AstNode::Literal(d) => Some(d),
            _ => None,
        }
    }

    pub fn has_spaces(&self, dot: bool) -> bool {
        match self {
            AstNode::Literal(d) => d.datum_type != DatumType::String && d.datum_type != DatumType::Int && d.datum_type != DatumType::Float,
            AstNode::Var(_) => true,
            AstNode::Member { cast_id, .. } => cast_id.is_some() || !dot,
            AstNode::ObjProp { .. } => !dot,
            AstNode::ObjBracket { .. } => !dot,
            AstNode::ObjPropIndex { .. } => !dot,
            AstNode::Call { args, .. } => {
                if let AstNode::Literal(d) = args.as_ref() {
                    d.list_value.is_empty()
                } else {
                    true
                }
            }
            AstNode::ObjCall { .. } => !dot,
            AstNode::ObjCallV4 { .. } => !dot,
            AstNode::Error => false,
            _ => true,
        }
    }

    pub fn write_script(&self, code: &mut CodeWriter, dot: bool, sum: bool) {
        self.write_script_with_depth(code, dot, sum, 0);
    }

    fn write_script_with_depth(&self, code: &mut CodeWriter, dot: bool, sum: bool, depth: usize) {
        if depth > MAX_WRITE_DEPTH {
            code.write("/* MAX DEPTH */");
            return;
        }
        match self {
            AstNode::Error => code.write("ERROR"),
            AstNode::Comment(text) => {
                code.write("-- ");
                code.write(text);
            }
            AstNode::Literal(datum) => datum.write_script_with_depth(code, dot, sum, depth),
            AstNode::Block(block) => block.write_script_with_depth(code, dot, sum, depth),
            AstNode::Var(name) => code.write(name),
            AstNode::Assignment { variable, value, force_verbose } => {
                if dot && !*force_verbose {
                    variable.write_script_with_depth(code, dot, sum, depth + 1);
                    code.write(" = ");
                    value.write_script_with_depth(code, dot, sum, depth + 1);
                } else {
                    // The destination of a `set` is always written verbosely,
                    // so `set the blend of sprite the spriteNum of me to 0`
                    // rather than mixing in dot syntax for the sprite id.
                    code.write("set ");
                    variable.write_script_with_depth(code, false, sum, depth + 1);
                    code.write(" to ");
                    value.write_script_with_depth(code, dot, sum, depth + 1);
                }
            }
            AstNode::BinaryOp { opcode, left, right } => {
                write_binary_op_with_depth(code, *opcode, left, right, dot, sum, depth);
            }
            AstNode::InverseOp(operand) => {
                code.write("-");
                // `-(the maxInteger) / 2` — negating a `the ...` form needs the
                // parentheses or the `/ 2` binds to the wrong thing.
                let needs_parens = matches!(operand.as_ref(), AstNode::BinaryOp { .. })
                    || renders_with_spaces(operand, dot);
                if needs_parens { code.write("("); }
                operand.write_script_with_depth(code, dot, sum, depth + 1);
                if needs_parens { code.write(")"); }
            }
            AstNode::NotOp(operand) => {
                code.write("not ");
                let needs_parens = matches!(operand.as_ref(), AstNode::BinaryOp { .. })
                    || renders_with_spaces(operand, dot);
                if needs_parens { code.write("("); }
                operand.write_script_with_depth(code, dot, sum, depth + 1);
                if needs_parens { code.write(")"); }
            }
            AstNode::ChunkExpr { chunk_type, first, last, string } => {
                let chunk_name = chunk_type.name();
                // A `last` of literal 0 means "same chunk as `first`" — the
                // reference is to a single chunk, whatever `first` evaluates to.
                let is_single = is_literal_zero(last);
                if is_single {
                    code.write(chunk_name);
                    code.write(" ");
                    first.write_script_with_depth(code, dot, sum, depth + 1);
                    code.write(" of ");
                    write_chunk_source(code, string, *chunk_type, dot, sum, depth);
                } else {
                    code.write(chunk_name);
                    code.write(" ");
                    first.write_script_with_depth(code, dot, sum, depth + 1);
                    code.write(" to ");
                    last.write_script_with_depth(code, dot, sum, depth + 1);
                    code.write(" of ");
                    write_chunk_source(code, string, *chunk_type, dot, sum, depth);
                }
            }
            AstNode::ChunkHilite(chunk) => {
                code.write("hilite ");
                chunk.write_script_with_depth(code, dot, sum, depth + 1);
            }
            AstNode::ChunkDelete(chunk) => {
                code.write("delete ");
                chunk.write_script_with_depth(code, dot, sum, depth + 1);
            }
            AstNode::SpriteIntersects { first, second } => {
                // Either sprite id may be an operator expression, which needs
                // parentheses: `sprite a intersects (b + i)`.
                code.write("sprite ");
                let paren = matches!(first.as_ref(), AstNode::BinaryOp { .. });
                if paren { code.write("("); }
                first.write_script_with_depth(code, dot, sum, depth + 1);
                if paren { code.write(")"); }
                code.write(" intersects ");
                let paren = matches!(second.as_ref(), AstNode::BinaryOp { .. });
                if paren { code.write("("); }
                second.write_script_with_depth(code, dot, sum, depth + 1);
                if paren { code.write(")"); }
            }
            AstNode::SpriteWithin { first, second } => {
                // Either sprite id may be an operator expression, which needs
                // parentheses: `sprite a intersects (b + i)`.
                code.write("sprite ");
                let paren = matches!(first.as_ref(), AstNode::BinaryOp { .. });
                if paren { code.write("("); }
                first.write_script_with_depth(code, dot, sum, depth + 1);
                if paren { code.write(")"); }
                code.write(" within ");
                let paren = matches!(second.as_ref(), AstNode::BinaryOp { .. });
                if paren { code.write("("); }
                second.write_script_with_depth(code, dot, sum, depth + 1);
                if paren { code.write(")"); }
            }
            AstNode::Member { member_type, member_id, cast_id } => {
                // A cast ID of literal 0 means "unspecified" — `field("x", 0)`
                // is how `field("x")` compiles, and writing the 0 back changes
                // which cast library the expression names.
                let cast_id = cast_id.as_ref().filter(|cast| !is_literal_zero(cast));
                if dot {
                    code.write(member_type);
                    code.write("(");
                    member_id.write_script_with_depth(code, dot, sum, depth + 1);
                    if let Some(cast) = cast_id {
                        code.write(", ");
                        cast.write_script_with_depth(code, dot, sum, depth + 1);
                    }
                    code.write(")");
                } else {
                    code.write(member_type);
                    code.write(" ");
                    // `field ("name" & n)` — an operator expression as the id
                    // needs parentheses or it swallows what follows.
                    let paren = matches!(member_id.as_ref(), AstNode::BinaryOp { .. });
                    if paren { code.write("("); }
                    member_id.write_script_with_depth(code, dot, sum, depth + 1);
                    if paren { code.write(")"); }
                    if let Some(cast) = cast_id {
                        code.write(" of castLib ");
                        let paren = matches!(cast.as_ref(), AstNode::BinaryOp { .. });
                        if paren { code.write("("); }
                        cast.write_script_with_depth(code, dot, sum, depth + 1);
                        if paren { code.write(")"); }
                    }
                }
            }
            AstNode::The(prop) => {
                code.write("the ");
                code.write(prop);
            }
            AstNode::TheProp { obj, prop } => {
                // `the number of castMembers of castLib X` — the object of a
                // verbose `the ... of ...` is spelled verbosely too, so a member
                // reference reads `castLib X`, not `castLib(X)`.
                code.write("the ");
                code.write(prop);
                code.write(" of ");
                obj.write_script_with_depth(code, false, sum, depth + 1);
            }
            AstNode::ObjProp { obj, prop } => {
                if dot {
                    // `the stage.rect` reads as `the (stage.rect)`; the object
                    // needs parentheses whenever it renders with spaces.
                    let paren = renders_with_spaces(obj, dot);
                    if paren { code.write("("); }
                    obj.write_script_with_depth(code, true, sum, depth + 1);
                    if paren { code.write(")"); }
                    code.write(".");
                    code.write(prop);
                } else {
                    code.write("the ");
                    code.write(prop);
                    code.write(" of ");
                    obj.write_script_with_depth(code, dot, sum, depth + 1);
                }
            }
            AstNode::ObjBracket { obj, prop } => {
                // `(the desktopRectList)[1]` — an indexed object that renders
                // with spaces needs parentheses to stay unambiguous.
                let paren = renders_with_spaces(obj, dot);
                if paren { code.write("("); }
                obj.write_script_with_depth(code, dot, sum, depth + 1);
                if paren { code.write(")"); }
                code.write("[");
                prop.write_script_with_depth(code, dot, sum, depth + 1);
                code.write("]");
            }
            AstNode::ObjPropIndex { obj, prop, index, index2 } => {
                let paren = renders_with_spaces(obj, dot);
                if paren { code.write("("); }
                obj.write_script_with_depth(code, dot, sum, depth + 1);
                if paren { code.write(")"); }
                code.write(".");
                code.write(prop);
                code.write("[");
                index.write_script_with_depth(code, dot, sum, depth + 1);
                if let Some(i2) = index2 {
                    code.write("..");
                    i2.write_script_with_depth(code, dot, sum, depth + 1);
                }
                code.write("]");
            }
            AstNode::LastStringChunk { chunk_type, obj } => {
                // Director spells this `the last char in X`, not `of X`.
                code.write("the last ");
                code.write(chunk_type.name());
                code.write(" in ");
                obj.write_script_with_depth(code, false, sum, depth + 1);
            }
            AstNode::StringChunkCount { chunk_type, obj } => {
                code.write("the number of ");
                code.write(chunk_type.name());
                code.write("s in ");
                obj.write_script_with_depth(code, false, sum, depth + 1);
            }
            AstNode::MenuProp { menu_id, prop } => {
                code.write("the ");
                code.write(&get_menu_prop_name(*prop));
                code.write(" of menu ");
                menu_id.write_script_with_depth(code, dot, sum, depth + 1);
            }
            AstNode::MenuItemProp { menu_id, item_id, prop } => {
                code.write("the ");
                code.write(&get_menu_item_prop_name(*prop));
                code.write(" of menuItem ");
                item_id.write_script_with_depth(code, dot, sum, depth + 1);
                code.write(" of menu ");
                menu_id.write_script_with_depth(code, dot, sum, depth + 1);
            }
            AstNode::SoundProp { sound_id, prop } => {
                code.write("the ");
                code.write(&get_sound_prop_name(*prop));
                code.write(" of sound ");
                sound_id.write_script_with_depth(code, dot, sum, depth + 1);
            }
            AstNode::SpriteProp { sprite_id, prop } => {
                code.write("the ");
                code.write(&get_sprite_prop_name(*prop));
                code.write(" of sprite ");
                // `the locV of sprite (n + 1)` — without parentheses the `+ 1`
                // reads as part of the enclosing expression.
                let paren = matches!(sprite_id.as_ref(), AstNode::BinaryOp { .. });
                if paren { code.write("("); }
                sprite_id.write_script_with_depth(code, dot, sum, depth + 1);
                if paren { code.write(")"); }
            }
            AstNode::Call { name, args } => {
                write_call_with_depth(code, name, args, dot, sum, depth);
            }
            AstNode::ObjCall { name, args } => {
                if let AstNode::Literal(arg_list) = args.as_ref() {
                    if !arg_list.list_value.is_empty() {
                        let obj = &arg_list.list_value[0];
                        if dot {
                            let paren = renders_with_spaces(obj, dot);
                            if paren { code.write("("); }
                            obj.write_script_with_depth(code, true, sum, depth + 1);
                            if paren { code.write(")"); }
                            code.write(".");
                            code.write(name);
                            code.write("(");
                            for (i, arg) in arg_list.list_value.iter().skip(1).enumerate() {
                                if i > 0 { code.write(", "); }
                                arg.write_script_with_depth(code, true, sum, depth + 1);
                            }
                            code.write(")");
                        } else {
                            code.write(name);
                            code.write("(");
                            for (i, arg) in arg_list.list_value.iter().enumerate() {
                                if i > 0 { code.write(", "); }
                                arg.write_script_with_depth(code, dot, sum, depth + 1);
                            }
                            code.write(")");
                        }
                    } else {
                        code.write(name);
                    }
                } else {
                    code.write(name);
                    code.write("(");
                    args.write_script_with_depth(code, dot, sum, depth + 1);
                    code.write(")");
                }
            }
            AstNode::ObjCallV4 { obj, args } => {
                obj.write_script_with_depth(code, dot, sum, depth + 1);
                code.write("(");
                args.write_script_with_depth(code, dot, sum, depth + 1);
                code.write(")");
            }
            AstNode::Exit => code.write("exit"),
            AstNode::ExitRepeat => code.write("exit repeat"),
            AstNode::NextRepeat => code.write("next repeat"),
            AstNode::Put { put_type, variable, value } => {
                code.write("put ");
                value.write_script_with_depth(code, dot, sum, depth + 1);
                code.write(" ");
                code.write(put_type.name());
                code.write(" ");
                // The destination is always written verbosely: `into field "x"`.
                variable.write_script_with_depth(code, false, sum, depth + 1);
            }
            AstNode::If { condition, block1, block2, has_else } => {
                code.write("if ");
                condition.write_script_with_depth(code, dot, sum, depth + 1);
                code.write(" then");
                code.end_line();
                code.indent();
                block1.borrow().write_script_with_depth(code, dot, sum, depth + 1);
                code.unindent();
                if has_else.get() && !block2.borrow().children.is_empty() {
                    code.write("else");
                    code.end_line();
                    code.indent();
                    block2.borrow().write_script_with_depth(code, dot, sum, depth + 1);
                    code.unindent();
                }
                code.write("end if");
            }
            AstNode::RepeatWhile { condition, block, .. } => {
                code.write("repeat while ");
                condition.write_script_with_depth(code, dot, sum, depth + 1);
                code.end_line();
                code.indent();
                block.borrow().write_script_with_depth(code, dot, sum, depth + 1);
                code.unindent();
                code.write("end repeat");
            }
            AstNode::RepeatWithIn { var_name, list, block, .. } => {
                code.write("repeat with ");
                code.write(var_name);
                code.write(" in ");
                list.write_script_with_depth(code, dot, sum, depth + 1);
                code.end_line();
                code.indent();
                block.borrow().write_script_with_depth(code, dot, sum, depth + 1);
                code.unindent();
                code.write("end repeat");
            }
            AstNode::RepeatWithTo { var_name, start, end, up, block, .. } => {
                code.write("repeat with ");
                code.write(var_name);
                code.write(" = ");
                start.write_script_with_depth(code, dot, sum, depth + 1);
                if *up {
                    code.write(" to ");
                } else {
                    code.write(" down to ");
                }
                end.write_script_with_depth(code, dot, sum, depth + 1);
                code.end_line();
                code.indent();
                block.borrow().write_script_with_depth(code, dot, sum, depth + 1);
                code.unindent();
                code.write("end repeat");
            }
            AstNode::Tell { window, block } => {
                code.write("tell ");
                window.write_script_with_depth(code, dot, sum, depth + 1);
                code.end_line();
                code.indent();
                block.borrow().write_script_with_depth(code, dot, sum, depth + 1);
                code.unindent();
                code.write("end tell");
            }
            AstNode::Case { value, first_label, otherwise, .. } => {
                code.write("case ");
                value.write_script_with_depth(code, dot, sum, depth + 1);
                code.write(" of");
                code.end_line();
                code.indent();

                // Write case labels
                let mut current_label = first_label.borrow().clone();
                while let Some(label) = current_label {
                    let label_ref = label.borrow();
                    label_ref.write_script_with_depth(code, dot, sum, depth + 1);
                    current_label = label_ref.next_label.clone();
                }

                // Write otherwise
                if let Some(ow) = &*otherwise.borrow() {
                    ow.borrow().write_script_with_depth(code, dot, sum, depth + 1);
                }

                code.unindent();
                code.write("end case");
            }
            AstNode::NewObj { obj_type, args } => {
                // `new xtra("fileio")`, not `new(xtra, "fileio")` — the latter
                // reads as a call to `new` with the type as its first argument.
                code.write("new ");
                code.write(obj_type);
                code.write("(");
                if let AstNode::Literal(arg_list) = args.as_ref() {
                    if !arg_list.list_value.is_empty() {
                        for (i, arg) in arg_list.list_value.iter().enumerate() {
                            if i > 0 { code.write(", "); }
                            arg.write_script_with_depth(code, dot, sum, depth + 1);
                        }
                    }
                }
                code.write(")");
            }
            AstNode::When { event, script } => {
                code.write("when ");
                code.write(&get_event_name(*event));
                code.write(" then ");
                code.write(script);
            }
            AstNode::SoundCmd { cmd, args } => {
                code.write("sound ");
                code.write(cmd);
                if let AstNode::Literal(arg_list) = args.as_ref() {
                    if !arg_list.list_value.is_empty() {
                        code.write(" ");
                        for (i, arg) in arg_list.list_value.iter().enumerate() {
                            if i > 0 { code.write(", "); }
                            arg.write_script_with_depth(code, dot, sum, depth + 1);
                        }
                    }
                }
            }
            AstNode::PlayCmd { args } => {
                // `play` has its own syntax: `play done`, `play frame X`,
                // `play frame X of movie Y`.
                code.write("play");
                if let AstNode::Literal(arg_list) = args.as_ref() {
                    let items = &arg_list.list_value;
                    if items.is_empty() {
                        code.write(" done");
                    } else if items.len() == 1 {
                        code.write(" frame ");
                        items[0].write_script_with_depth(code, dot, sum, depth + 1);
                    } else {
                        let frame_is_one = matches!(
                            items[0].as_ref(),
                            AstNode::Literal(d)
                                if d.datum_type == DatumType::Int && d.int_value == 1
                        );
                        if !frame_is_one {
                            code.write(" frame ");
                            items[0].write_script_with_depth(code, dot, sum, depth + 1);
                            code.write(" of");
                        }
                        code.write(" movie ");
                        items[1].write_script_with_depth(code, dot, sum, depth + 1);
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct CaseLabelNode {
    pub value: Rc<AstNode>,
    pub expect: CaseExpect,
    pub next_or: Option<Rc<RefCell<CaseLabelNode>>>,
    pub next_label: Option<Rc<RefCell<CaseLabelNode>>>,
    pub block: Rc<RefCell<BlockNode>>,
}

impl CaseLabelNode {
    pub fn new(value: Rc<AstNode>, expect: CaseExpect) -> Self {
        Self {
            value,
            expect,
            next_or: None,
            next_label: None,
            block: Rc::new(RefCell::new(BlockNode::new())),
        }
    }

    pub fn write_script(&self, code: &mut CodeWriter, dot: bool, sum: bool) {
        self.write_script_with_depth(code, dot, sum, 0);
    }

    fn write_script_with_depth(&self, code: &mut CodeWriter, dot: bool, sum: bool, depth: usize) {
        if depth > MAX_WRITE_DEPTH {
            code.write("-- MAX DEPTH EXCEEDED");
            code.end_line();
            return;
        }
        // Write value(s)
        self.value.write_script_with_depth(code, dot, sum, depth + 1);

        // Write chained "or" values
        let mut current_or = self.next_or.clone();
        while let Some(or_label) = current_or {
            code.write(", ");
            or_label.borrow().value.write_script_with_depth(code, dot, sum, depth + 1);
            current_or = or_label.borrow().next_or.clone();
        }

        code.write(":");
        code.end_line();
        code.indent();
        self.block.borrow().write_script_with_depth(code, dot, sum, depth + 1);
        code.unindent();
    }
}

#[derive(Clone, Debug)]
pub struct OtherwiseNode {
    pub block: Rc<RefCell<BlockNode>>,
}

impl OtherwiseNode {
    pub fn new() -> Self {
        Self {
            block: Rc::new(RefCell::new(BlockNode::new())),
        }
    }

    pub fn write_script(&self, code: &mut CodeWriter, dot: bool, sum: bool) {
        self.write_script_with_depth(code, dot, sum, 0);
    }

    fn write_script_with_depth(&self, code: &mut CodeWriter, dot: bool, sum: bool, depth: usize) {
        if depth > MAX_WRITE_DEPTH {
            code.write("-- MAX DEPTH EXCEEDED");
            code.end_line();
            return;
        }
        code.write("otherwise:");
        code.end_line();
        code.indent();
        self.block.borrow().write_script_with_depth(code, dot, sum, depth + 1);
        code.unindent();
    }
}

impl Default for OtherwiseNode {
    fn default() -> Self {
        Self::new()
    }
}

fn write_binary_op_with_depth(code: &mut CodeWriter, opcode: OpCode, left: &Rc<AstNode>, right: &Rc<AstNode>, dot: bool, sum: bool, depth: usize) {
    if depth > MAX_WRITE_DEPTH {
        code.write("/* MAX DEPTH */");
        return;
    }
    // Parenthesize the way Director's own decompiler does: a left operand only
    // when it binds differently, a right operand whenever it is itself a binary
    // operation (the AST is already correctly nested, but `a - (b - c)` must not
    // reprint as `a - b - c`). Operators with no precedence entry never take
    // parens, which keeps `a & b & c` flat.
    let precedence = get_precedence(opcode);
    let (left_needs_parens, right_needs_parens) = if precedence == 0 {
        (false, false)
    } else {
        let left_parens = match left.as_ref() {
            AstNode::BinaryOp { opcode: left_op, .. } => get_precedence(*left_op) != precedence,
            _ => false,
        };
        let right_parens = matches!(right.as_ref(), AstNode::BinaryOp { .. });
        (left_parens, right_parens)
    };

    if left_needs_parens { code.write("("); }
    left.write_script_with_depth(code, dot, sum, depth + 1);
    if left_needs_parens { code.write(")"); }

    code.write(" ");
    code.write(get_op_string(opcode));
    code.write(" ");

    if right_needs_parens { code.write("("); }
    right.write_script_with_depth(code, dot, sum, depth + 1);
    if right_needs_parens { code.write(")"); }
}

fn get_op_string(opcode: OpCode) -> &'static str {
    match opcode {
        OpCode::Mul => "*",
        OpCode::Add => "+",
        OpCode::Sub => "-",
        OpCode::Div => "/",
        OpCode::Mod => "mod",
        OpCode::JoinStr => "&",
        OpCode::JoinPadStr => "&&",
        OpCode::Lt => "<",
        OpCode::LtEq => "<=",
        OpCode::NtEq => "<>",
        OpCode::Eq => "=",
        OpCode::Gt => ">",
        OpCode::GtEq => ">=",
        OpCode::And => "and",
        OpCode::Or => "or",
        OpCode::ContainsStr => "contains",
        // `contains0` is the `starts` operator, not another spelling of contains.
        OpCode::Contains0Str => "starts",
        _ => "???",
    }
}

/// Tightest-binding operators score lowest; 0 means "no precedence", which
/// suppresses parentheses entirely (string joins and `contains`).
fn get_precedence(opcode: OpCode) -> u32 {
    match opcode {
        OpCode::Mul | OpCode::Div | OpCode::Mod => 1,
        OpCode::Add | OpCode::Sub => 2,
        OpCode::Lt | OpCode::LtEq | OpCode::NtEq | OpCode::Eq | OpCode::Gt | OpCode::GtEq => 3,
        OpCode::And => 4,
        OpCode::Or => 5,
        _ => 0,
    }
}

fn write_call_with_depth(code: &mut CodeWriter, name: &str, args: &Rc<AstNode>, dot: bool, sum: bool, depth: usize) {
    if depth > MAX_WRITE_DEPTH {
        code.write("/* MAX DEPTH */");
        return;
    }
    // Only `put` and `return` are written without parentheses, matching how
    // Director itself renders these calls; everything else keeps them.
    let no_parens = matches!(name.to_lowercase().as_str(), "put" | "return");

    if let AstNode::Literal(arg_list) = args.as_ref() {
        let is_statement = arg_list.datum_type == DatumType::ArgListNoRet;

        // Member expressions such as `member 1 of castLib 1` compile to the call
        // `member(1, 1)`, which pre-dot-syntax Director cannot parse. In verbose
        // mode they are written back in their real syntax.
        if !dot && !is_statement {
            let nargs = arg_list.list_value.len();
            let member_expr = is_member_expr_call(name, nargs);
            if member_expr {
                code.write(name);
                code.write(" ");
                let id = &arg_list.list_value[0];
                let paren = matches!(id.as_ref(), AstNode::BinaryOp { .. });
                if paren { code.write("("); }
                id.write_script_with_depth(code, dot, sum, depth + 1);
                if paren { code.write(")"); }
                if nargs == 2 {
                    code.write(" of castLib ");
                    let cast_id = &arg_list.list_value[1];
                    let paren = matches!(cast_id.as_ref(), AstNode::BinaryOp { .. });
                    if paren { code.write("("); }
                    cast_id.write_script_with_depth(code, dot, sum, depth + 1);
                    if paren { code.write(")"); }
                }
                return;
            }
        }

        // These constants compile to zero-argument calls; write them back as
        // the constants they were, since `void()` and friends are not Lingo.
        if arg_list.list_value.is_empty() && !is_statement {
            match name {
                "pi" => return code.write("PI"),
                "space" => return code.write("SPACE"),
                "void" => return code.write("VOID"),
                _ => {}
            }
        }

        if arg_list.list_value.is_empty() {
            // Empty argument list - only omit parens for no-parens statement commands
            if no_parens && is_statement {
                code.write(name);
            } else {
                code.write(name);
                code.write("()");
            }
        } else if no_parens && is_statement {
            // No-parens statement with arguments: "return x" instead of "return(x)"
            code.write(name);
            code.write(" ");
            for (i, arg) in arg_list.list_value.iter().enumerate() {
                if i > 0 { code.write(", "); }
                arg.write_script_with_depth(code, dot, sum, depth + 1);
            }
        } else {
            // Normal function call with parentheses
            code.write(name);
            code.write("(");
            for (i, arg) in arg_list.list_value.iter().enumerate() {
                if i > 0 { code.write(", "); }
                arg.write_script_with_depth(code, dot, sum, depth + 1);
            }
            code.write(")");
        }
    } else {
        code.write(name);
        code.write("(");
        args.write_script_with_depth(code, dot, sum, depth + 1);
        code.write(")");
    }
}

fn get_menu_prop_name(prop: u32) -> String {
    match prop {
        0x01 => "name".to_string(),
        0x02 => "number".to_string(),
        _ => format!("menuProp_{}", prop),
    }
}

fn get_menu_item_prop_name(prop: u32) -> String {
    match prop {
        0x01 => "name".to_string(),
        0x02 => "checkMark".to_string(),
        0x03 => "enabled".to_string(),
        0x04 => "script".to_string(),
        _ => format!("menuItemProp_{}", prop),
    }
}

fn get_sound_prop_name(prop: u32) -> String {
    match prop {
        0x01 => "volume".to_string(),
        _ => format!("soundProp_{}", prop),
    }
}

/// Sprite property names come from `constants::sprite_prop_names`, the same
/// table the compiler uses. A second hand-maintained copy here had drifted:
/// it omitted movieRate, movieTime, startTime, stopTime and volume, so every
/// id from 0x0f up resolved to the wrong property (`the width of sprite` read
/// back as `the loc of sprite`).
fn get_sprite_prop_name(prop: u32) -> String {
    match crate::director::lingo::constants::sprite_prop_names().get(&(prop as u16)) {
        Some(symbol) => symbol.as_str().to_string(),
        None => format!("spriteProp_{}", prop),
    }
}

fn get_event_name(event: i32) -> String {
    match event {
        1 => "mouseDown".to_string(),
        2 => "mouseUp".to_string(),
        3 => "keyDown".to_string(),
        4 => "keyUp".to_string(),
        5 => "timeout".to_string(),
        _ => format!("event_{}", event),
    }
}
