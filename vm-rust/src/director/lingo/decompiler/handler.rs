// Lingo bytecode decompiler - core handler logic
// Ported from ProjectorRays

use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;
use fxhash::FxHashMap;

use crate::director::chunks::handler::HandlerDef;
use crate::director::chunks::script::ScriptChunk;
use crate::director::lingo::opcode::OpCode;
use crate::director::lingo::script::ScriptContext;
use super::ast::*;
use super::enums::*;
use super::code_writer::CodeWriter;

/// Represents a decompiled line of Lingo code
#[derive(Clone, Debug)]
pub struct DecompiledLine {
    pub text: String,
    pub bytecode_indices: Vec<usize>,
    pub indent: u32,
}

/// Result of decompiling a handler
#[derive(Clone, Debug)]
pub struct DecompiledHandler {
    pub name: String,
    pub arguments: Vec<String>,
    pub lines: Vec<DecompiledLine>,
    pub bytecode_to_line: HashMap<usize, usize>,
}

/// Tags for bytecode instructions (used for loop identification)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct BytecodeInfo {
    tag: BytecodeTag,
    owner_loop: u32,
}

/// Stack entry with bytecode tracking
struct StackEntry {
    node: Rc<AstNode>,
    bytecode_indices: Vec<usize>,
}

/// Decompiler state
struct DecompilerState<'a> {
    handler: &'a HandlerDef,
    chunk: &'a ScriptChunk,
    lctx: &'a ScriptContext,
    version: u16,
    multiplier: u32,

    // Stack for expressions with bytecode tracking
    stack: Vec<StackEntry>,

    // AST building
    root_block: Rc<RefCell<BlockNode>>,
    current_block: Rc<RefCell<BlockNode>>,
    block_stack: Vec<Rc<RefCell<BlockNode>>>,

    // Bytecode tagging
    bytecode_tags: Vec<BytecodeInfo>,

    // Position mapping
    bytecode_pos_map: FxHashMap<usize, usize>,

    // Current bytecode index being processed
    current_bytecode_index: usize,

    // Statement to bytecode indices mapping
    statement_bytecode_indices: Vec<Vec<usize>>,
}

impl<'a> DecompilerState<'a> {
    fn new(handler: &'a HandlerDef, chunk: &'a ScriptChunk, lctx: &'a ScriptContext, version: u16, multiplier: u32) -> Self {
        let root_block = Rc::new(RefCell::new(BlockNode::new()));
        let current_block = root_block.clone();

        // Build position map
        let mut bytecode_pos_map = FxHashMap::default();
        for (i, bc) in handler.bytecode_array.iter().enumerate() {
            bytecode_pos_map.insert(bc.pos, i);
        }

        // Initialize tags
        let bytecode_tags = vec![BytecodeInfo::default(); handler.bytecode_array.len()];

        Self {
            handler,
            chunk,
            lctx,
            version,
            multiplier,
            stack: Vec::new(),
            root_block,
            current_block,
            block_stack: Vec::new(),
            bytecode_tags,
            bytecode_pos_map,
            current_bytecode_index: 0,
            statement_bytecode_indices: Vec::new(),
        }
    }

    fn get_name(&self, id: i64) -> String {
        self.lctx.names.get(id as usize)
            .cloned()
            .unwrap_or_else(|| format!("UNKNOWN_{}", id))
    }

    fn get_local_name(&self, id: i64) -> String {
        let local_index = (id as u32 / self.multiplier) as usize;
        self.handler.local_name_ids
            .get(local_index)
            .and_then(|&name_id| self.lctx.names.get(name_id as usize))
            .cloned()
            .unwrap_or_else(|| format!("local_{}", local_index))
    }

    fn get_argument_name(&self, id: i64) -> String {
        let arg_index = (id as u32 / self.multiplier) as usize;
        self.handler.argument_name_ids
            .get(arg_index)
            .and_then(|&name_id| self.lctx.names.get(name_id as usize))
            .cloned()
            .unwrap_or_else(|| format!("arg_{}", arg_index))
    }

    /// Pop from stack, returning node and accumulating bytecode indices
    fn pop(&mut self) -> Rc<AstNode> {
        if let Some(entry) = self.stack.pop() {
            entry.node
        } else {
            Rc::new(AstNode::Error)
        }
    }

    /// Pop from stack and collect bytecode indices into the provided vec
    fn pop_with_indices(&mut self, indices: &mut Vec<usize>) -> Rc<AstNode> {
        if let Some(entry) = self.stack.pop() {
            indices.extend(entry.bytecode_indices);
            entry.node
        } else {
            Rc::new(AstNode::Error)
        }
    }

    fn push(&mut self, node: AstNode) {
        self.stack.push(StackEntry {
            node: Rc::new(node),
            bytecode_indices: vec![self.current_bytecode_index],
        });
    }

    fn push_with_indices(&mut self, node: AstNode, indices: Vec<usize>) {
        let mut all_indices = indices;
        all_indices.push(self.current_bytecode_index);
        self.stack.push(StackEntry {
            node: Rc::new(node),
            bytecode_indices: all_indices,
        });
    }

    fn enter_block(&mut self, block: Rc<RefCell<BlockNode>>) {
        self.block_stack.push(self.current_block.clone());
        self.current_block = block;
    }

    fn exit_block(&mut self) {
        if let Some(parent) = self.block_stack.pop() {
            self.current_block = parent;
        }
    }

    fn add_statement(&mut self, node: Rc<AstNode>, bytecode_indices: Vec<usize>) {
        self.current_block.borrow_mut().add_child(node);
        self.statement_bytecode_indices.push(bytecode_indices);
    }

    /// Tag loops in the bytecode
    fn tag_loops(&mut self) {
        let bytecode_array = &self.handler.bytecode_array;

        for start_index in 0..bytecode_array.len() {
            let jmpifz = &bytecode_array[start_index];
            if jmpifz.opcode != OpCode::JmpIfZ {
                continue;
            }

            // Calculate jump position
            let jmp_pos = jmpifz.pos + jmpifz.obj as usize;
            let end_index = match self.bytecode_pos_map.get(&jmp_pos) {
                Some(&idx) => idx,
                None => continue,
            };

            if end_index == 0 {
                continue;
            }

            let end_repeat = &bytecode_array[end_index - 1];
            if end_repeat.opcode != OpCode::EndRepeat {
                continue;
            }

            // Check if endrepeat jumps back before jmpifz
            if end_repeat.pos < end_repeat.obj as usize {
                continue;
            }
            if (end_repeat.pos - end_repeat.obj as usize) > jmpifz.pos {
                continue;
            }

            let loop_type = self.identify_loop(start_index, end_index);
            self.bytecode_tags[start_index].tag = loop_type;

            match loop_type {
                BytecodeTag::RepeatWithIn => {
                    // Tag pre-loop setup (7 instructions before jmpifz)
                    if start_index >= 7 {
                        for i in (start_index - 7)..start_index {
                            self.bytecode_tags[i].tag = BytecodeTag::Skip;
                        }
                    }
                    // Tag post-condition setup (5 instructions after jmpifz)
                    for i in (start_index + 1)..=(start_index + 5).min(bytecode_array.len() - 1) {
                        self.bytecode_tags[i].tag = BytecodeTag::Skip;
                    }
                    // Tag loop increment and end
                    if end_index >= 3 {
                        self.bytecode_tags[end_index - 3].tag = BytecodeTag::NextRepeatTarget;
                        self.bytecode_tags[end_index - 3].owner_loop = start_index as u32;
                        self.bytecode_tags[end_index - 2].tag = BytecodeTag::Skip;
                        self.bytecode_tags[end_index - 1].tag = BytecodeTag::Skip;
                        self.bytecode_tags[end_index - 1].owner_loop = start_index as u32;
                    }
                    if end_index < bytecode_array.len() {
                        self.bytecode_tags[end_index].tag = BytecodeTag::Skip;
                    }
                }
                BytecodeTag::RepeatWithTo | BytecodeTag::RepeatWithDownTo => {
                    let end_repeat = &bytecode_array[end_index - 1];
                    if let Some(&condition_start_index) = self.bytecode_pos_map.get(&(end_repeat.pos - end_repeat.obj as usize)) {
                        if condition_start_index > 0 {
                            self.bytecode_tags[condition_start_index - 1].tag = BytecodeTag::Skip;
                        }
                        self.bytecode_tags[condition_start_index].tag = BytecodeTag::Skip;
                    }
                    if start_index > 0 {
                        self.bytecode_tags[start_index - 1].tag = BytecodeTag::Skip;
                    }
                    if end_index >= 5 {
                        self.bytecode_tags[end_index - 5].tag = BytecodeTag::NextRepeatTarget;
                        self.bytecode_tags[end_index - 5].owner_loop = start_index as u32;
                        self.bytecode_tags[end_index - 4].tag = BytecodeTag::Skip;
                        self.bytecode_tags[end_index - 3].tag = BytecodeTag::Skip;
                        self.bytecode_tags[end_index - 2].tag = BytecodeTag::Skip;
                        self.bytecode_tags[end_index - 1].tag = BytecodeTag::Skip;
                        self.bytecode_tags[end_index - 1].owner_loop = start_index as u32;
                    }
                }
                BytecodeTag::RepeatWhile => {
                    self.bytecode_tags[end_index - 1].tag = BytecodeTag::NextRepeatTarget;
                    self.bytecode_tags[end_index - 1].owner_loop = start_index as u32;
                }
                _ => {}
            }
        }
    }

    fn identify_loop(&self, start_index: usize, end_index: usize) -> BytecodeTag {
        // Check for repeat with in
        if self.is_repeat_with_in(start_index, end_index) {
            return BytecodeTag::RepeatWithIn;
        }

        if start_index < 1 {
            return BytecodeTag::RepeatWhile;
        }

        let bytecode_array = &self.handler.bytecode_array;

        // Check for repeat with to/downto
        let up = match bytecode_array[start_index - 1].opcode {
            OpCode::LtEq => true,
            OpCode::GtEq => false,
            _ => return BytecodeTag::RepeatWhile,
        };

        let end_repeat = &bytecode_array[end_index - 1];
        let condition_start_pos = end_repeat.pos - end_repeat.obj as usize;
        let condition_start_index = match self.bytecode_pos_map.get(&condition_start_pos) {
            Some(&idx) => idx,
            None => return BytecodeTag::RepeatWhile,
        };

        if condition_start_index < 1 {
            return BytecodeTag::RepeatWhile;
        }

        // Verify the set/get pattern
        let set_op = bytecode_array[condition_start_index - 1].opcode;
        let get_op = match set_op {
            OpCode::SetGlobal => OpCode::GetGlobal,
            OpCode::SetGlobal2 => OpCode::GetGlobal2,
            OpCode::SetProp => OpCode::GetProp,
            OpCode::SetParam => OpCode::GetParam,
            OpCode::SetLocal => OpCode::GetLocal,
            _ => return BytecodeTag::RepeatWhile,
        };
        let var_id = bytecode_array[condition_start_index - 1].obj;

        if bytecode_array[condition_start_index].opcode != get_op
            || bytecode_array[condition_start_index].obj != var_id
        {
            return BytecodeTag::RepeatWhile;
        }

        if end_index < 5 {
            return BytecodeTag::RepeatWhile;
        }

        // Check increment pattern
        let expected_inc = if up { 1 } else { -1 };
        if bytecode_array[end_index - 5].opcode != OpCode::PushInt8
            || bytecode_array[end_index - 5].obj != expected_inc
        {
            return BytecodeTag::RepeatWhile;
        }

        if bytecode_array[end_index - 4].opcode != get_op
            || bytecode_array[end_index - 4].obj != var_id
        {
            return BytecodeTag::RepeatWhile;
        }

        if bytecode_array[end_index - 3].opcode != OpCode::Add {
            return BytecodeTag::RepeatWhile;
        }

        if bytecode_array[end_index - 2].opcode != set_op
            || bytecode_array[end_index - 2].obj != var_id
        {
            return BytecodeTag::RepeatWhile;
        }

        if up {
            BytecodeTag::RepeatWithTo
        } else {
            BytecodeTag::RepeatWithDownTo
        }
    }

    fn is_repeat_with_in(&self, start_index: usize, end_index: usize) -> bool {
        let bytecode_array = &self.handler.bytecode_array;

        if start_index < 7 || start_index + 5 >= bytecode_array.len() {
            return false;
        }

        // Check pre-jmpifz pattern
        if bytecode_array[start_index - 7].opcode != OpCode::Peek
            || bytecode_array[start_index - 7].obj != 0
        {
            return false;
        }

        if bytecode_array[start_index - 6].opcode != OpCode::PushArgList
            || bytecode_array[start_index - 6].obj != 1
        {
            return false;
        }

        if bytecode_array[start_index - 5].opcode != OpCode::ExtCall
            || self.get_name(bytecode_array[start_index - 5].obj) != "count"
        {
            return false;
        }

        if bytecode_array[start_index - 4].opcode != OpCode::PushInt8
            || bytecode_array[start_index - 4].obj != 1
        {
            return false;
        }

        if bytecode_array[start_index - 3].opcode != OpCode::Peek
            || bytecode_array[start_index - 3].obj != 0
        {
            return false;
        }

        if bytecode_array[start_index - 2].opcode != OpCode::Peek
            || bytecode_array[start_index - 2].obj != 2
        {
            return false;
        }

        if bytecode_array[start_index - 1].opcode != OpCode::LtEq {
            return false;
        }

        // Check post-jmpifz pattern
        if bytecode_array[start_index + 1].opcode != OpCode::Peek
            || bytecode_array[start_index + 1].obj != 2
        {
            return false;
        }

        if bytecode_array[start_index + 2].opcode != OpCode::Peek
            || bytecode_array[start_index + 2].obj != 1
        {
            return false;
        }

        if bytecode_array[start_index + 3].opcode != OpCode::PushArgList
            || bytecode_array[start_index + 3].obj != 2
        {
            return false;
        }

        if bytecode_array[start_index + 4].opcode != OpCode::ExtCall
            || self.get_name(bytecode_array[start_index + 4].obj) != "getAt"
        {
            return false;
        }

        let set_op = bytecode_array[start_index + 5].opcode;
        if !matches!(
            set_op,
            OpCode::SetGlobal | OpCode::SetProp | OpCode::SetParam | OpCode::SetLocal
        ) {
            return false;
        }

        // Check end pattern
        if end_index < 3 {
            return false;
        }

        if bytecode_array[end_index - 3].opcode != OpCode::PushInt8
            || bytecode_array[end_index - 3].obj != 1
        {
            return false;
        }

        if bytecode_array[end_index - 2].opcode != OpCode::Add {
            return false;
        }

        if bytecode_array[end_index].opcode != OpCode::Pop || bytecode_array[end_index].obj != 3 {
            return false;
        }

        true
    }

    fn get_var_name_from_set(&self, index: usize) -> String {
        let bytecode = &self.handler.bytecode_array[index];
        match bytecode.opcode {
            OpCode::SetGlobal | OpCode::SetGlobal2 | OpCode::SetProp => {
                self.get_name(bytecode.obj)
            }
            OpCode::SetParam => self.get_argument_name(bytecode.obj),
            OpCode::SetLocal => self.get_local_name(bytecode.obj),
            _ => "unknown".to_string(),
        }
    }

    /// Parse and translate all bytecode
    fn parse(&mut self) {
        self.tag_loops();
        self.stack.clear();

        let mut i = 0;
        while i < self.handler.bytecode_array.len() {
            let bytecode = &self.handler.bytecode_array[i];
            let pos = bytecode.pos as u32;

            // Exit blocks at their end position
            while pos == self.current_block.borrow().end_pos {
                self.exit_block();
            }

            self.current_bytecode_index = i;
            let translate_size = self.translate_bytecode(i);
            i += translate_size;
        }
    }

    fn translate_bytecode(&mut self, index: usize) -> usize {
        let tag = self.bytecode_tags[index].tag;

        // Skip tagged internal loop bytecode
        if tag == BytecodeTag::Skip || tag == BytecodeTag::NextRepeatTarget {
            return 1;
        }

        let bytecode = self.handler.bytecode_array[index].clone();
        let opcode = bytecode.opcode;
        let obj = bytecode.obj;

        let mut next_block: Option<Rc<RefCell<BlockNode>>> = None;
        let mut collected_indices: Vec<usize> = vec![index];

        let translation: Option<Rc<AstNode>> = match opcode {
            OpCode::Ret | OpCode::RetFactory => {
                if index == self.handler.bytecode_array.len() - 1 {
                    None // end of handler
                } else {
                    Some(Rc::new(AstNode::Exit))
                }
            }

            OpCode::PushZero => {
                Some(Rc::new(AstNode::Literal(Datum::int(0))))
            }

            OpCode::Mul | OpCode::Add | OpCode::Sub | OpCode::Div | OpCode::Mod |
            OpCode::JoinStr | OpCode::JoinPadStr |
            OpCode::Lt | OpCode::LtEq | OpCode::NtEq | OpCode::Eq | OpCode::Gt | OpCode::GtEq |
            OpCode::And | OpCode::Or | OpCode::ContainsStr | OpCode::Contains0Str => {
                let b = self.pop_with_indices(&mut collected_indices);
                let a = self.pop_with_indices(&mut collected_indices);
                Some(Rc::new(AstNode::BinaryOp { opcode, left: a, right: b }))
            }

            OpCode::Inv => {
                let x = self.pop_with_indices(&mut collected_indices);
                Some(Rc::new(AstNode::InverseOp(x)))
            }

            OpCode::Not => {
                let x = self.pop_with_indices(&mut collected_indices);
                Some(Rc::new(AstNode::NotOp(x)))
            }

            OpCode::GetChunk => {
                let string = self.pop_with_indices(&mut collected_indices);
                Some(self.read_chunk_ref_with_indices(string, &mut collected_indices))
            }

            OpCode::HiliteChunk => {
                let cast_id = if self.version >= 500 {
                    Some(self.pop_with_indices(&mut collected_indices))
                } else {
                    None
                };
                let field_id = self.pop_with_indices(&mut collected_indices);
                let field = Rc::new(AstNode::Member {
                    member_type: "field".to_string(),
                    member_id: field_id,
                    cast_id,
                });
                let chunk = self.read_chunk_ref_with_indices(field, &mut collected_indices);
                Some(Rc::new(AstNode::ChunkHilite(chunk)))
            }

            OpCode::OntoSpr => {
                let second = self.pop_with_indices(&mut collected_indices);
                let first = self.pop_with_indices(&mut collected_indices);
                Some(Rc::new(AstNode::SpriteIntersects { first, second }))
            }

            OpCode::IntoSpr => {
                let second = self.pop_with_indices(&mut collected_indices);
                let first = self.pop_with_indices(&mut collected_indices);
                Some(Rc::new(AstNode::SpriteWithin { first, second }))
            }

            OpCode::GetField => {
                let cast_id = if self.version >= 500 {
                    Some(self.pop_with_indices(&mut collected_indices))
                } else {
                    None
                };
                let field_id = self.pop_with_indices(&mut collected_indices);
                Some(Rc::new(AstNode::Member {
                    member_type: "field".to_string(),
                    member_id: field_id,
                    cast_id,
                }))
            }

            OpCode::StartTell => {
                let window = self.pop_with_indices(&mut collected_indices);
                let block = Rc::new(RefCell::new(BlockNode::new()));
                let tell = Rc::new(AstNode::Tell { window, block: block.clone() });
                next_block = Some(block);
                Some(tell)
            }

            OpCode::EndTell => {
                self.exit_block();
                None
            }

            OpCode::PushList => {
                let list = self.pop_with_indices(&mut collected_indices);
                if let AstNode::Literal(mut datum) = (*list).clone() {
                    datum.datum_type = DatumType::List;
                    Some(Rc::new(AstNode::Literal(datum)))
                } else {
                    Some(list)
                }
            }

            OpCode::PushPropList => {
                let list = self.pop_with_indices(&mut collected_indices);
                if let AstNode::Literal(mut datum) = (*list).clone() {
                    datum.datum_type = DatumType::PropList;
                    Some(Rc::new(AstNode::Literal(datum)))
                } else {
                    Some(list)
                }
            }

            OpCode::Swap => {
                if self.stack.len() >= 2 {
                    let len = self.stack.len();
                    self.stack.swap(len - 1, len - 2);
                }
                None
            }

            OpCode::PushInt8 | OpCode::PushInt16 | OpCode::PushInt32 => {
                Some(Rc::new(AstNode::Literal(Datum::int(obj as i32))))
            }

            OpCode::PushFloat32 => {
                let f = f32::from_bits(obj as u32);
                Some(Rc::new(AstNode::Literal(Datum::float(f as f64))))
            }

            OpCode::PushArgListNoRet => {
                let arg_count = obj as usize;
                let mut args = Vec::with_capacity(arg_count);
                for _ in 0..arg_count {
                    args.push(self.pop_with_indices(&mut collected_indices));
                }
                args.reverse();
                Some(Rc::new(AstNode::Literal(Datum::arg_list_no_ret(args))))
            }

            OpCode::PushArgList => {
                let arg_count = obj as usize;
                let mut args = Vec::with_capacity(arg_count);
                for _ in 0..arg_count {
                    args.push(self.pop_with_indices(&mut collected_indices));
                }
                args.reverse();
                Some(Rc::new(AstNode::Literal(Datum::arg_list(args))))
            }

            OpCode::PushCons => {
                let literal_id = (obj as u32 / self.multiplier) as usize;
                if let Some(literal) = self.chunk.literals.get(literal_id) {
                    let datum = match literal {
                        crate::director::lingo::datum::Datum::String(s) => Datum::string(s.clone()),
                        crate::director::lingo::datum::Datum::Int(i) => Datum::int(*i),
                        crate::director::lingo::datum::Datum::Float(f) => Datum::float(*f as f64),
                        crate::director::lingo::datum::Datum::Symbol(s) => Datum::symbol(s.clone()),
                        _ => Datum::void(),
                    };
                    Some(Rc::new(AstNode::Literal(datum)))
                } else {
                    Some(Rc::new(AstNode::Error))
                }
            }

            OpCode::PushSymb => {
                Some(Rc::new(AstNode::Literal(Datum::symbol(self.get_name(obj)))))
            }

            OpCode::PushVarRef => {
                Some(Rc::new(AstNode::Literal(Datum::var_ref(self.get_name(obj)))))
            }

            OpCode::GetGlobal | OpCode::GetGlobal2 => {
                Some(Rc::new(AstNode::Var(self.get_name(obj))))
            }

            OpCode::GetProp => {
                Some(Rc::new(AstNode::Var(self.get_name(obj))))
            }

            OpCode::GetParam => {
                Some(Rc::new(AstNode::Var(self.get_argument_name(obj))))
            }

            OpCode::GetLocal => {
                Some(Rc::new(AstNode::Var(self.get_local_name(obj))))
            }

            OpCode::SetGlobal | OpCode::SetGlobal2 => {
                let value = self.pop_with_indices(&mut collected_indices);
                let var = Rc::new(AstNode::Var(self.get_name(obj)));
                Some(Rc::new(AstNode::Assignment { variable: var, value, force_verbose: false }))
            }

            OpCode::SetProp => {
                let value = self.pop_with_indices(&mut collected_indices);
                let var = Rc::new(AstNode::Var(self.get_name(obj)));
                Some(Rc::new(AstNode::Assignment { variable: var, value, force_verbose: false }))
            }

            OpCode::SetParam => {
                let value = self.pop_with_indices(&mut collected_indices);
                let var = Rc::new(AstNode::Var(self.get_argument_name(obj)));
                Some(Rc::new(AstNode::Assignment { variable: var, value, force_verbose: false }))
            }

            OpCode::SetLocal => {
                let value = self.pop_with_indices(&mut collected_indices);
                let var = Rc::new(AstNode::Var(self.get_local_name(obj)));
                Some(Rc::new(AstNode::Assignment { variable: var, value, force_verbose: false }))
            }

            OpCode::Jmp => {
                self.translate_jmp(index, obj)
            }

            OpCode::EndRepeat => {
                // Should normally be tagged and skipped
                Some(Rc::new(AstNode::Comment("ERROR: Stray endrepeat".to_string())))
            }

            OpCode::JmpIfZ => {
                self.translate_jmpifz_with_indices(index, obj, &mut next_block, &mut collected_indices)
            }

            OpCode::LocalCall => {
                let arg_list = self.pop_with_indices(&mut collected_indices);
                let handler_name = if (obj as usize) < self.chunk.handlers.len() {
                    // Get handler name from the name table
                    let handler_def = &self.chunk.handlers[obj as usize];
                    self.lctx.names.get(handler_def.name_id as usize)
                        .cloned()
                        .unwrap_or_else(|| format!("handler_{}", obj))
                } else {
                    format!("handler_{}", obj)
                };
                Some(Rc::new(AstNode::Call { name: handler_name, args: arg_list }))
            }

            OpCode::ExtCall | OpCode::TellCall => {
                let name = self.get_name(obj);
                let arg_list = self.pop_with_indices(&mut collected_indices);
                Some(Rc::new(AstNode::Call { name, args: arg_list }))
            }

            OpCode::ObjCallV4 => {
                let arg_list = self.pop_with_indices(&mut collected_indices);
                let object = self.read_var_with_indices(obj, &mut collected_indices);
                Some(Rc::new(AstNode::ObjCallV4 { obj: object, args: arg_list }))
            }

            OpCode::Put => {
                let put_type = match ((obj >> 4) & 0xF) as u8 {
                    1 => PutType::Into,
                    2 => PutType::After,
                    3 => PutType::Before,
                    _ => PutType::Into,
                };
                let var_type = (obj & 0xF) as i64;
                let var = self.read_var_with_indices(var_type, &mut collected_indices);
                let val = self.pop_with_indices(&mut collected_indices);
                Some(Rc::new(AstNode::Put { put_type, variable: var, value: val }))
            }

            OpCode::PutChunk => {
                let put_type = match ((obj >> 4) & 0xF) as u8 {
                    1 => PutType::Into,
                    2 => PutType::After,
                    3 => PutType::Before,
                    _ => PutType::Into,
                };
                let var_type = (obj & 0xF) as i64;
                let var = self.read_var_with_indices(var_type, &mut collected_indices);
                let chunk = self.read_chunk_ref_with_indices(var, &mut collected_indices);
                let val = self.pop_with_indices(&mut collected_indices);
                Some(Rc::new(AstNode::Put { put_type, variable: chunk, value: val }))
            }

            OpCode::DeleteChunk => {
                let var = self.read_var_with_indices(obj, &mut collected_indices);
                let chunk = self.read_chunk_ref_with_indices(var, &mut collected_indices);
                Some(Rc::new(AstNode::ChunkDelete(chunk)))
            }

            OpCode::Get => {
                let prop_id = self.pop_with_indices(&mut collected_indices);
                let prop_id_val = prop_id.get_value().map(|d| d.to_int()).unwrap_or(0);
                self.read_v4_property(obj, prop_id_val)
            }

            OpCode::Set => {
                let prop_id = self.pop_with_indices(&mut collected_indices);
                let value = self.pop_with_indices(&mut collected_indices);
                let prop_id_val = prop_id.get_value().map(|d| d.to_int()).unwrap_or(0);
                let prop = self.read_v4_property(obj, prop_id_val);
                if let Some(p) = prop {
                    Some(Rc::new(AstNode::Assignment { variable: p, value, force_verbose: true }))
                } else {
                    Some(Rc::new(AstNode::Error))
                }
            }

            OpCode::GetMovieProp => {
                Some(Rc::new(AstNode::The(self.get_name(obj))))
            }

            OpCode::SetMovieProp => {
                let value = self.pop_with_indices(&mut collected_indices);
                let prop = Rc::new(AstNode::The(self.get_name(obj)));
                Some(Rc::new(AstNode::Assignment { variable: prop, value, force_verbose: false }))
            }

            OpCode::GetObjProp | OpCode::GetChainedProp => {
                let object = self.pop_with_indices(&mut collected_indices);
                Some(Rc::new(AstNode::ObjProp { obj: object, prop: self.get_name(obj) }))
            }

            OpCode::SetObjProp => {
                let value = self.pop_with_indices(&mut collected_indices);
                let object = self.pop_with_indices(&mut collected_indices);
                let prop = Rc::new(AstNode::ObjProp { obj: object, prop: self.get_name(obj) });
                Some(Rc::new(AstNode::Assignment { variable: prop, value, force_verbose: false }))
            }

            OpCode::Peek => {
                // This is either the start of a case statement or part of repeat with in
                // For now, just push a placeholder
                None
            }

            OpCode::Pop => {
                // Pop instructions are handled specially
                None
            }

            OpCode::TheBuiltin => {
                self.pop_with_indices(&mut collected_indices); // empty arglist
                Some(Rc::new(AstNode::The(self.get_name(obj))))
            }

            OpCode::ObjCall => {
                let method = self.get_name(obj);
                let arg_list = self.pop_with_indices(&mut collected_indices);
                self.translate_obj_call(&method, arg_list)
            }

            OpCode::PushChunkVarRef => {
                Some(self.read_var_with_indices(obj, &mut collected_indices))
            }

            OpCode::GetTopLevelProp => {
                Some(Rc::new(AstNode::Var(self.get_name(obj))))
            }

            OpCode::NewObj => {
                let obj_args = self.pop_with_indices(&mut collected_indices);
                Some(Rc::new(AstNode::NewObj { obj_type: self.get_name(obj), args: obj_args }))
            }

            _ => {
                // Unknown opcode
                let op_id = num::ToPrimitive::to_u16(&opcode).unwrap_or(0);
                let comment = if op_id > 0x40 {
                    format!("Unknown opcode {:02x} {}", op_id, obj)
                } else {
                    format!("Unknown opcode {:02x}", op_id)
                };
                self.stack.clear();
                Some(Rc::new(AstNode::Comment(comment)))
            }
        };

        if let Some(node) = translation {
            if node.is_expression() {
                self.push_with_indices((*node).clone(), collected_indices);
            } else {
                self.add_statement(node, collected_indices);
            }
        }

        if let Some(block) = next_block {
            self.enter_block(block);
        }

        1
    }

    fn translate_jmp(&mut self, index: usize, obj: i64) -> Option<Rc<AstNode>> {
        let bytecode_array = &self.handler.bytecode_array;
        let bytecode = &bytecode_array[index];
        let target_pos = bytecode.pos + obj as usize;

        let target_index = match self.bytecode_pos_map.get(&target_pos) {
            Some(&idx) => idx,
            None => return Some(Rc::new(AstNode::Comment("ERROR: Invalid jump target".to_string()))),
        };

        // Check for exit repeat / next repeat
        if target_index > 0 {
            let prev_bytecode = &bytecode_array[target_index - 1];
            if prev_bytecode.opcode == OpCode::EndRepeat {
                let owner_loop = self.bytecode_tags[target_index - 1].owner_loop;
                if owner_loop > 0 {
                    return Some(Rc::new(AstNode::ExitRepeat));
                }
            }
        }

        if self.bytecode_tags[target_index].tag == BytecodeTag::NextRepeatTarget {
            return Some(Rc::new(AstNode::NextRepeat));
        }

        // Check for else branch
        if index + 1 < bytecode_array.len() {
            let next_bytecode = &bytecode_array[index + 1];
            if next_bytecode.pos as u32 == self.current_block.borrow().end_pos {
                // This is the end of an if block, update the else block
                // The parent if statement should handle this
                return None;
            }
        }

        Some(Rc::new(AstNode::Comment("jmp".to_string())))
    }

    fn translate_jmpifz(&mut self, index: usize, obj: i64, next_block: &mut Option<Rc<RefCell<BlockNode>>>) -> Option<Rc<AstNode>> {
        let bytecode = &self.handler.bytecode_array[index];
        let end_pos = (bytecode.pos as i64 + obj) as u32;
        let tag = self.bytecode_tags[index].tag;

        match tag {
            BytecodeTag::RepeatWhile => {
                let condition = self.pop();
                let block = Rc::new(RefCell::new(BlockNode::new()));
                block.borrow_mut().end_pos = end_pos;
                *next_block = Some(block.clone());
                Some(Rc::new(AstNode::RepeatWhile {
                    condition,
                    block,
                    start_index: index as u32,
                }))
            }

            BytecodeTag::RepeatWithIn => {
                let list = self.pop();
                let var_name = self.get_var_name_from_set(index + 5);
                let block = Rc::new(RefCell::new(BlockNode::new()));
                block.borrow_mut().end_pos = end_pos;
                *next_block = Some(block.clone());
                Some(Rc::new(AstNode::RepeatWithIn {
                    var_name,
                    list,
                    block,
                    start_index: index as u32,
                }))
            }

            BytecodeTag::RepeatWithTo | BytecodeTag::RepeatWithDownTo => {
                let up = tag == BytecodeTag::RepeatWithTo;
                let end = self.pop();
                let start = self.pop();

                let bytecode_array = &self.handler.bytecode_array;
                let end_index = self.bytecode_pos_map.get(&(end_pos as usize)).copied().unwrap_or(index);
                let end_repeat = &bytecode_array[end_index.saturating_sub(1)];
                let condition_start_pos = end_repeat.pos.saturating_sub(end_repeat.obj as usize);
                let condition_start_index = self.bytecode_pos_map.get(&condition_start_pos).copied().unwrap_or(0);
                let var_name = if condition_start_index > 0 {
                    self.get_var_name_from_set(condition_start_index - 1)
                } else {
                    "i".to_string()
                };

                let block = Rc::new(RefCell::new(BlockNode::new()));
                block.borrow_mut().end_pos = end_pos;
                *next_block = Some(block.clone());
                Some(Rc::new(AstNode::RepeatWithTo {
                    var_name,
                    start,
                    end,
                    up,
                    block,
                    start_index: index as u32,
                }))
            }

            _ => {
                // Regular if statement
                let condition = self.pop();
                let block1 = Rc::new(RefCell::new(BlockNode::new()));
                block1.borrow_mut().end_pos = end_pos;
                let block2 = Rc::new(RefCell::new(BlockNode::new()));
                *next_block = Some(block1.clone());
                Some(Rc::new(AstNode::If {
                    condition,
                    block1,
                    block2,
                    has_else: false,
                }))
            }
        }
    }

    fn translate_jmpifz_with_indices(&mut self, index: usize, obj: i64, next_block: &mut Option<Rc<RefCell<BlockNode>>>, indices: &mut Vec<usize>) -> Option<Rc<AstNode>> {
        let bytecode = &self.handler.bytecode_array[index];
        let end_pos = (bytecode.pos as i64 + obj) as u32;
        let tag = self.bytecode_tags[index].tag;

        match tag {
            BytecodeTag::RepeatWhile => {
                let condition = self.pop_with_indices(indices);
                let block = Rc::new(RefCell::new(BlockNode::new()));
                block.borrow_mut().end_pos = end_pos;
                *next_block = Some(block.clone());
                Some(Rc::new(AstNode::RepeatWhile {
                    condition,
                    block,
                    start_index: index as u32,
                }))
            }

            BytecodeTag::RepeatWithIn => {
                let list = self.pop_with_indices(indices);
                let var_name = self.get_var_name_from_set(index + 5);
                let block = Rc::new(RefCell::new(BlockNode::new()));
                block.borrow_mut().end_pos = end_pos;
                *next_block = Some(block.clone());
                Some(Rc::new(AstNode::RepeatWithIn {
                    var_name,
                    list,
                    block,
                    start_index: index as u32,
                }))
            }

            BytecodeTag::RepeatWithTo | BytecodeTag::RepeatWithDownTo => {
                let up = tag == BytecodeTag::RepeatWithTo;
                let end = self.pop_with_indices(indices);
                let start = self.pop_with_indices(indices);

                let bytecode_array = &self.handler.bytecode_array;
                let end_index = self.bytecode_pos_map.get(&(end_pos as usize)).copied().unwrap_or(index);
                let end_repeat = &bytecode_array[end_index.saturating_sub(1)];
                let condition_start_pos = end_repeat.pos.saturating_sub(end_repeat.obj as usize);
                let condition_start_index = self.bytecode_pos_map.get(&condition_start_pos).copied().unwrap_or(0);
                let var_name = if condition_start_index > 0 {
                    self.get_var_name_from_set(condition_start_index - 1)
                } else {
                    "i".to_string()
                };

                let block = Rc::new(RefCell::new(BlockNode::new()));
                block.borrow_mut().end_pos = end_pos;
                *next_block = Some(block.clone());
                Some(Rc::new(AstNode::RepeatWithTo {
                    var_name,
                    start,
                    end,
                    up,
                    block,
                    start_index: index as u32,
                }))
            }

            _ => {
                // Regular if statement
                let condition = self.pop_with_indices(indices);
                let block1 = Rc::new(RefCell::new(BlockNode::new()));
                block1.borrow_mut().end_pos = end_pos;
                let block2 = Rc::new(RefCell::new(BlockNode::new()));
                *next_block = Some(block1.clone());
                Some(Rc::new(AstNode::If {
                    condition,
                    block1,
                    block2,
                    has_else: false,
                }))
            }
        }
    }

    fn translate_obj_call(&mut self, method: &str, arg_list: Rc<AstNode>) -> Option<Rc<AstNode>> {
        if let AstNode::Literal(datum) = arg_list.as_ref() {
            let args = &datum.list_value;
            let nargs = args.len();

            // Handle special method translations
            match method {
                "getAt" if nargs == 2 => {
                    return Some(Rc::new(AstNode::ObjBracket {
                        obj: args[0].clone(),
                        prop: args[1].clone(),
                    }));
                }
                "setAt" if nargs == 3 => {
                    let prop_expr = Rc::new(AstNode::ObjBracket {
                        obj: args[0].clone(),
                        prop: args[1].clone(),
                    });
                    return Some(Rc::new(AstNode::Assignment {
                        variable: prop_expr,
                        value: args[2].clone(),
                        force_verbose: false,
                    }));
                }
                "hilite" if nargs == 1 => {
                    return Some(Rc::new(AstNode::ChunkHilite(args[0].clone())));
                }
                "delete" if nargs == 1 => {
                    return Some(Rc::new(AstNode::ChunkDelete(args[0].clone())));
                }
                _ => {}
            }
        }

        Some(Rc::new(AstNode::ObjCall { name: method.to_string(), args: arg_list }))
    }

    fn read_var(&mut self, var_type: i64) -> Rc<AstNode> {
        let cast_id = if var_type == 0x6 && self.version >= 500 {
            Some(self.pop())
        } else {
            None
        };
        let id = self.pop();

        match var_type {
            0x1 | 0x2 | 0x3 => id, // global, property
            0x4 => {
                // argument
                if let Some(datum) = id.get_value() {
                    let name = self.get_argument_name(datum.to_int() as i64);
                    Rc::new(AstNode::Literal(Datum::var_ref(name)))
                } else {
                    id
                }
            }
            0x5 => {
                // local
                if let Some(datum) = id.get_value() {
                    let name = self.get_local_name(datum.to_int() as i64);
                    Rc::new(AstNode::Literal(Datum::var_ref(name)))
                } else {
                    id
                }
            }
            0x6 => {
                // field
                Rc::new(AstNode::Member {
                    member_type: "field".to_string(),
                    member_id: id,
                    cast_id,
                })
            }
            _ => Rc::new(AstNode::Error),
        }
    }

    fn read_var_with_indices(&mut self, var_type: i64, indices: &mut Vec<usize>) -> Rc<AstNode> {
        let cast_id = if var_type == 0x6 && self.version >= 500 {
            Some(self.pop_with_indices(indices))
        } else {
            None
        };
        let id = self.pop_with_indices(indices);

        match var_type {
            0x1 | 0x2 | 0x3 => id, // global, property
            0x4 => {
                // argument
                if let Some(datum) = id.get_value() {
                    let name = self.get_argument_name(datum.to_int() as i64);
                    Rc::new(AstNode::Literal(Datum::var_ref(name)))
                } else {
                    id
                }
            }
            0x5 => {
                // local
                if let Some(datum) = id.get_value() {
                    let name = self.get_local_name(datum.to_int() as i64);
                    Rc::new(AstNode::Literal(Datum::var_ref(name)))
                } else {
                    id
                }
            }
            0x6 => {
                // field
                Rc::new(AstNode::Member {
                    member_type: "field".to_string(),
                    member_id: id,
                    cast_id,
                })
            }
            _ => Rc::new(AstNode::Error),
        }
    }

    fn read_chunk_ref(&mut self, string: Rc<AstNode>) -> Rc<AstNode> {
        let last_line = self.pop();
        let first_line = self.pop();
        let last_item = self.pop();
        let first_item = self.pop();
        let last_word = self.pop();
        let first_word = self.pop();
        let last_char = self.pop();
        let first_char = self.pop();

        let mut result = string;

        // Build chunk expression from innermost to outermost
        if !is_zero(&first_line) {
            result = Rc::new(AstNode::ChunkExpr {
                chunk_type: ChunkExprType::Line,
                first: first_line,
                last: last_line,
                string: result,
            });
        }
        if !is_zero(&first_item) {
            result = Rc::new(AstNode::ChunkExpr {
                chunk_type: ChunkExprType::Item,
                first: first_item,
                last: last_item,
                string: result,
            });
        }
        if !is_zero(&first_word) {
            result = Rc::new(AstNode::ChunkExpr {
                chunk_type: ChunkExprType::Word,
                first: first_word,
                last: last_word,
                string: result,
            });
        }
        if !is_zero(&first_char) {
            result = Rc::new(AstNode::ChunkExpr {
                chunk_type: ChunkExprType::Char,
                first: first_char,
                last: last_char,
                string: result,
            });
        }

        result
    }

    fn read_chunk_ref_with_indices(&mut self, string: Rc<AstNode>, indices: &mut Vec<usize>) -> Rc<AstNode> {
        let last_line = self.pop_with_indices(indices);
        let first_line = self.pop_with_indices(indices);
        let last_item = self.pop_with_indices(indices);
        let first_item = self.pop_with_indices(indices);
        let last_word = self.pop_with_indices(indices);
        let first_word = self.pop_with_indices(indices);
        let last_char = self.pop_with_indices(indices);
        let first_char = self.pop_with_indices(indices);

        let mut result = string;

        // Build chunk expression from innermost to outermost
        if !is_zero(&first_line) {
            result = Rc::new(AstNode::ChunkExpr {
                chunk_type: ChunkExprType::Line,
                first: first_line,
                last: last_line,
                string: result,
            });
        }
        if !is_zero(&first_item) {
            result = Rc::new(AstNode::ChunkExpr {
                chunk_type: ChunkExprType::Item,
                first: first_item,
                last: last_item,
                string: result,
            });
        }
        if !is_zero(&first_word) {
            result = Rc::new(AstNode::ChunkExpr {
                chunk_type: ChunkExprType::Word,
                first: first_word,
                last: last_word,
                string: result,
            });
        }
        if !is_zero(&first_char) {
            result = Rc::new(AstNode::ChunkExpr {
                chunk_type: ChunkExprType::Char,
                first: first_char,
                last: last_char,
                string: result,
            });
        }

        result
    }

    fn read_v4_property(&self, property_type: i64, property_id: i32) -> Option<Rc<AstNode>> {
        match property_type {
            0x00 => {
                // Movie/system property
                let prop_name = get_movie_property_name(property_id);
                Some(Rc::new(AstNode::The(prop_name)))
            }
            0x01 => {
                // Sound property
                let sound_id = Rc::new(AstNode::Literal(Datum::int(property_id)));
                Some(Rc::new(AstNode::SoundProp { sound_id, prop: 1 }))
            }
            0x02 => {
                // Sprite property
                let sprite_id = Rc::new(AstNode::Literal(Datum::int(property_id)));
                Some(Rc::new(AstNode::SpriteProp { sprite_id, prop: 0 }))
            }
            _ => {
                Some(Rc::new(AstNode::Comment(format!("Unknown property type {} id {}", property_type, property_id))))
            }
        }
    }

    /// Generate output lines from the parsed AST
    fn generate_output(&self) -> DecompiledHandler {
        let mut code = CodeWriter::new();

        // Write handler header
        let name = self.lctx.names.get(self.handler.name_id as usize)
            .cloned()
            .unwrap_or_else(|| format!("handler_{}", self.handler.name_id));

        let args: Vec<String> = self.handler.argument_name_ids.iter()
            .filter_map(|&id| self.lctx.names.get(id as usize).cloned())
            .collect();

        // Write block contents
        self.root_block.borrow().write_script(&mut code, true, false);

        let output = code.into_string();

        // Parse output into lines and create mappings
        let mut lines = Vec::new();
        let mut bytecode_to_line = HashMap::new();

        // Flatten all bytecode indices from statements
        // Each statement contributes its bytecode indices to the corresponding output lines
        let output_lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();

        // If we have statement bytecode indices, use them
        // Otherwise fall back to sequential assignment
        if !self.statement_bytecode_indices.is_empty() && !output_lines.is_empty() {
            // Match statements to output lines, skipping closing keywords (end if, end repeat, etc.)
            // which don't correspond to any statement but appear in the output.
            let statements_count = self.statement_bytecode_indices.len();
            let mut statement_index = 0;

            for (line_idx, line) in output_lines.iter().enumerate() {
                let indent = line.chars().take_while(|c| *c == ' ').count() as u32 / 2;
                let text = line.trim().to_string();

                // Check if this is a closing keyword line (no executable bytecodes)
                let is_closing_line = text == "end if" || text == "end repeat" ||
                                      text == "end tell" || text == "end case" ||
                                      text == "else";

                let stmt_indices = if is_closing_line {
                    // Closing lines have no executable bytecodes
                    vec![]
                } else if statement_index < statements_count {
                    // Assign next statement's bytecodes to this line
                    let indices = self.statement_bytecode_indices[statement_index].clone();
                    statement_index += 1;
                    indices
                } else {
                    vec![]
                };

                // Sort indices and use them
                let mut sorted_indices = stmt_indices;
                sorted_indices.sort_unstable();

                lines.push(DecompiledLine {
                    text,
                    bytecode_indices: sorted_indices.clone(),
                    indent,
                });

                // Map each bytecode index to this line
                for &bc_idx in &sorted_indices {
                    bytecode_to_line.entry(bc_idx).or_insert(line_idx);
                }
            }
        } else {
            // Fallback: no statement tracking available
            for (line_idx, line) in output_lines.iter().enumerate() {
                let indent = line.chars().take_while(|c| *c == ' ').count() as u32 / 2;
                let text = line.trim().to_string();

                lines.push(DecompiledLine {
                    text,
                    bytecode_indices: vec![],
                    indent,
                });
            }
        }

        DecompiledHandler {
            name,
            arguments: args,
            lines,
            bytecode_to_line,
        }
    }
}

fn is_zero(node: &Rc<AstNode>) -> bool {
    if let AstNode::Literal(datum) = node.as_ref() {
        datum.datum_type == DatumType::Int && datum.int_value == 0
    } else {
        false
    }
}

fn get_movie_property_name(id: i32) -> String {
    match id {
        0x01 => "floatPrecision".to_string(),
        0x02 => "mouseDownScript".to_string(),
        0x03 => "mouseUpScript".to_string(),
        0x04 => "keyDownScript".to_string(),
        0x05 => "keyUpScript".to_string(),
        0x06 => "timeoutScript".to_string(),
        _ => format!("movieProp_{}", id),
    }
}

/// Main entry point for decompiling a handler
pub fn decompile_handler(
    handler: &HandlerDef,
    chunk: &ScriptChunk,
    lctx: &ScriptContext,
    version: u16,
    multiplier: u32,
) -> DecompiledHandler {
    let mut state = DecompilerState::new(handler, chunk, lctx, version, multiplier);
    state.parse();
    state.generate_output()
}
