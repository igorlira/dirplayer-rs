// Lingo decompiler AST nodes
// Ported from ProjectorRays

use std::rc::Rc;
use std::cell::RefCell;
use crate::director::lingo::opcode::OpCode;
use super::enums::{ChunkExprType, PutType, DatumType, CaseExpect};
use super::code_writer::CodeWriter;

/// Maximum recursion depth for write_script to prevent stack overflow
const MAX_WRITE_DEPTH: usize = 100;

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
                let s = format!("{:.4}", self.float_value);
                // Remove trailing zeros but keep at least one decimal place
                let s = s.trim_end_matches('0');
                let s = if s.ends_with('.') { format!("{}0", s) } else { s.to_string() };
                code.write(&s);
            }
            DatumType::String => {
                code.write("\"");
                code.write(&escape_string(&self.string_value));
                code.write("\"");
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

fn escape_string(s: &str) -> String {
    let mut result = String::new();
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            _ => result.push(c),
        }
    }
    result
}

/// Block node for containing statements
#[derive(Clone, Debug)]
pub struct BlockNode {
    pub children: Vec<Rc<AstNode>>,
    pub end_pos: u32,
    pub current_case_label: Option<Rc<RefCell<CaseLabelNode>>>,
}

impl BlockNode {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            end_pos: u32::MAX,
            current_case_label: None,
        }
    }

    pub fn add_child(&mut self, child: Rc<AstNode>) {
        self.children.push(child);
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
            child.write_script_with_depth(code, dot, sum, depth + 1);
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
    If { condition: Rc<AstNode>, block1: Rc<RefCell<BlockNode>>, block2: Rc<RefCell<BlockNode>>, has_else: bool },
    RepeatWhile { condition: Rc<AstNode>, block: Rc<RefCell<BlockNode>>, start_index: u32 },
    RepeatWithIn { var_name: String, list: Rc<AstNode>, block: Rc<RefCell<BlockNode>>, start_index: u32 },
    RepeatWithTo { var_name: String, start: Rc<AstNode>, end: Rc<AstNode>, up: bool, block: Rc<RefCell<BlockNode>>, start_index: u32 },
    Tell { window: Rc<AstNode>, block: Rc<RefCell<BlockNode>> },
    Case { value: Rc<AstNode>, first_label: Option<Rc<RefCell<CaseLabelNode>>>, otherwise: Option<Rc<RefCell<OtherwiseNode>>>, end_pos: i32, potential_otherwise_pos: i32 },
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
                    code.write("set ");
                    variable.write_script_with_depth(code, dot, sum, depth + 1);
                    code.write(" to ");
                    value.write_script_with_depth(code, dot, sum, depth + 1);
                }
            }
            AstNode::BinaryOp { opcode, left, right } => {
                write_binary_op_with_depth(code, *opcode, left, right, dot, sum, depth);
            }
            AstNode::InverseOp(operand) => {
                code.write("-");
                let needs_parens = matches!(operand.as_ref(), AstNode::BinaryOp { .. });
                if needs_parens { code.write("("); }
                operand.write_script_with_depth(code, dot, sum, depth + 1);
                if needs_parens { code.write(")"); }
            }
            AstNode::NotOp(operand) => {
                code.write("not ");
                let needs_parens = matches!(operand.as_ref(), AstNode::BinaryOp { .. });
                if needs_parens { code.write("("); }
                operand.write_script_with_depth(code, dot, sum, depth + 1);
                if needs_parens { code.write(")"); }
            }
            AstNode::ChunkExpr { chunk_type, first, last, string } => {
                let chunk_name = chunk_type.name();
                // Check if first == last for single chunk reference
                let is_single = match (first.as_ref(), last.as_ref()) {
                    (AstNode::Literal(d1), AstNode::Literal(d2)) => {
                        d1.datum_type == DatumType::Int && d2.datum_type == DatumType::Int && d1.int_value == d2.int_value
                    }
                    _ => false,
                };
                if is_single {
                    code.write(chunk_name);
                    code.write(" ");
                    first.write_script_with_depth(code, dot, sum, depth + 1);
                    code.write(" of ");
                    string.write_script_with_depth(code, dot, sum, depth + 1);
                } else {
                    code.write(chunk_name);
                    code.write(" ");
                    first.write_script_with_depth(code, dot, sum, depth + 1);
                    code.write(" to ");
                    last.write_script_with_depth(code, dot, sum, depth + 1);
                    code.write(" of ");
                    string.write_script_with_depth(code, dot, sum, depth + 1);
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
                code.write("sprite ");
                first.write_script_with_depth(code, dot, sum, depth + 1);
                code.write(" intersects ");
                second.write_script_with_depth(code, dot, sum, depth + 1);
            }
            AstNode::SpriteWithin { first, second } => {
                code.write("sprite ");
                first.write_script_with_depth(code, dot, sum, depth + 1);
                code.write(" within ");
                second.write_script_with_depth(code, dot, sum, depth + 1);
            }
            AstNode::Member { member_type, member_id, cast_id } => {
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
                    member_id.write_script_with_depth(code, dot, sum, depth + 1);
                    if let Some(cast) = cast_id {
                        code.write(" of castLib ");
                        cast.write_script_with_depth(code, dot, sum, depth + 1);
                    }
                }
            }
            AstNode::The(prop) => {
                code.write("the ");
                code.write(prop);
            }
            AstNode::TheProp { obj, prop } => {
                code.write("the ");
                code.write(prop);
                code.write(" of ");
                obj.write_script_with_depth(code, dot, sum, depth + 1);
            }
            AstNode::ObjProp { obj, prop } => {
                if dot {
                    obj.write_script_with_depth(code, true, sum, depth + 1);
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
                obj.write_script_with_depth(code, dot, sum, depth + 1);
                code.write("[");
                prop.write_script_with_depth(code, dot, sum, depth + 1);
                code.write("]");
            }
            AstNode::ObjPropIndex { obj, prop, index, index2 } => {
                obj.write_script_with_depth(code, dot, sum, depth + 1);
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
                code.write("the last ");
                code.write(chunk_type.name());
                code.write(" of ");
                obj.write_script_with_depth(code, dot, sum, depth + 1);
            }
            AstNode::StringChunkCount { chunk_type, obj } => {
                code.write("the number of ");
                code.write(chunk_type.name());
                code.write("s in ");
                obj.write_script_with_depth(code, dot, sum, depth + 1);
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
                sprite_id.write_script_with_depth(code, dot, sum, depth + 1);
            }
            AstNode::Call { name, args } => {
                write_call_with_depth(code, name, args, dot, sum, depth);
            }
            AstNode::ObjCall { name, args } => {
                if let AstNode::Literal(arg_list) = args.as_ref() {
                    if !arg_list.list_value.is_empty() {
                        let obj = &arg_list.list_value[0];
                        if dot {
                            obj.write_script_with_depth(code, true, sum, depth + 1);
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
                variable.write_script_with_depth(code, dot, sum, depth + 1);
            }
            AstNode::If { condition, block1, block2, has_else } => {
                code.write("if ");
                condition.write_script_with_depth(code, dot, sum, depth + 1);
                code.write(" then");
                code.end_line();
                code.indent();
                block1.borrow().write_script_with_depth(code, dot, sum, depth + 1);
                code.unindent();
                if *has_else && !block2.borrow().children.is_empty() {
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
                let mut current_label = first_label.clone();
                while let Some(label) = current_label {
                    let label_ref = label.borrow();
                    label_ref.write_script_with_depth(code, dot, sum, depth + 1);
                    current_label = label_ref.next_label.clone();
                }

                // Write otherwise
                if let Some(ow) = otherwise {
                    ow.borrow().write_script_with_depth(code, dot, sum, depth + 1);
                }

                code.unindent();
                code.write("end case");
            }
            AstNode::NewObj { obj_type, args } => {
                code.write("new(");
                code.write(obj_type);
                if let AstNode::Literal(arg_list) = args.as_ref() {
                    if !arg_list.list_value.is_empty() {
                        code.write(", ");
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
                code.write("play");
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
    let precedence = get_precedence(opcode);

    let left_needs_parens = match left.as_ref() {
        AstNode::BinaryOp { opcode: left_op, .. } => get_precedence(*left_op) < precedence,
        _ => false,
    };

    let right_needs_parens = match right.as_ref() {
        AstNode::BinaryOp { opcode: right_op, .. } => get_precedence(*right_op) <= precedence,
        _ => false,
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
        OpCode::ContainsStr | OpCode::Contains0Str => "contains",
        _ => "???",
    }
}

fn get_precedence(opcode: OpCode) -> u32 {
    match opcode {
        OpCode::Or => 1,
        OpCode::And => 2,
        OpCode::ContainsStr | OpCode::Contains0Str => 3,
        OpCode::Lt | OpCode::LtEq | OpCode::NtEq | OpCode::Eq | OpCode::Gt | OpCode::GtEq => 4,
        OpCode::JoinStr | OpCode::JoinPadStr => 5,
        OpCode::Add | OpCode::Sub => 6,
        OpCode::Mul | OpCode::Div | OpCode::Mod => 7,
        _ => 0,
    }
}

fn write_call_with_depth(code: &mut CodeWriter, name: &str, args: &Rc<AstNode>, dot: bool, sum: bool, depth: usize) {
    if depth > MAX_WRITE_DEPTH {
        code.write("/* MAX DEPTH */");
        return;
    }
    // Check if this is a special no-parens command (statement-style)
    let no_parens = matches!(name.to_lowercase().as_str(),
        "go" | "play" | "playaccelerator" | "pause" | "stop" | "halt" | "pass" | "continue" |
        "alert" | "beep" | "updatestage" | "puppetsprite" | "puppetsound" | "puppetpalette" |
        "puppettempo" | "puppettransition" | "sound" | "printwithout" | "tell" | "return" |
        "nothing" | "put"
    );

    if let AstNode::Literal(arg_list) = args.as_ref() {
        let is_statement = arg_list.datum_type == DatumType::ArgListNoRet;

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

fn get_sprite_prop_name(prop: u32) -> String {
    match prop {
        0x01 => "type".to_string(),
        0x02 => "backColor".to_string(),
        0x03 => "bottom".to_string(),
        0x04 => "castNum".to_string(),
        0x05 => "constraint".to_string(),
        0x06 => "cursor".to_string(),
        0x07 => "foreColor".to_string(),
        0x08 => "height".to_string(),
        0x09 => "immediate".to_string(),
        0x0a => "ink".to_string(),
        0x0b => "left".to_string(),
        0x0c => "lineSize".to_string(),
        0x0d => "locH".to_string(),
        0x0e => "locV".to_string(),
        0x0f => "moveableSprite".to_string(),
        0x10 => "pattern".to_string(),
        0x11 => "puppet".to_string(),
        0x12 => "right".to_string(),
        0x13 => "scriptNum".to_string(),
        0x14 => "stretch".to_string(),
        0x15 => "top".to_string(),
        0x16 => "trails".to_string(),
        0x17 => "visible".to_string(),
        0x18 => "width".to_string(),
        0x19 => "blend".to_string(),
        0x1a => "scriptInstanceList".to_string(),
        0x1b => "loc".to_string(),
        0x1c => "rect".to_string(),
        0x1d => "member".to_string(),
        _ => format!("spriteProp_{}", prop),
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
