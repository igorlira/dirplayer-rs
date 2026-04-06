// Lingo bytecode decompiler - core handler logic
// Ported from ProjectorRays

use std::collections::HashMap;
use std::rc::Rc;
use std::cell::{Cell, RefCell};
use rustc_hash::FxHashMap;

use crate::director::chunks::handler::HandlerDef;
use crate::director::chunks::script::ScriptChunk;
use crate::director::lingo::opcode::OpCode;
use crate::director::lingo::script::ScriptContext;
use super::ast::*;
use super::enums::*;
use super::code_writer::CodeWriter;
use super::tokenizer::{tokenize_line, Span};

/// Represents a decompiled line of Lingo code
#[derive(Clone, Debug)]
pub struct DecompiledLine {
    pub text: String,
    pub bytecode_indices: Vec<usize>,
    pub indent: u32,
    pub spans: Vec<Span>,
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

/// Tracks what type of statement owns a block, for ancestor handling on block exit
#[derive(Clone)]
enum BlockContext {
    Root,
    IfBlock1(Rc<AstNode>),
    IfBlock2,
    CaseLabel,
    CaseOtherwise,
    Loop { start_index: u32 },
    Tell,
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
    block_context_stack: Vec<BlockContext>,

    // Bytecode tagging
    bytecode_tags: Vec<BytecodeInfo>,

    // Position mapping
    bytecode_pos_map: FxHashMap<usize, usize>,

    // Current bytecode index being processed
    current_bytecode_index: usize,

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
            block_context_stack: Vec::new(),
            bytecode_tags,
            bytecode_pos_map,
            current_bytecode_index: 0,
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

    fn enter_block(&mut self, block: Rc<RefCell<BlockNode>>, context: BlockContext) {
        self.block_stack.push(self.current_block.clone());
        self.block_context_stack.push(context);
        self.current_block = block;
    }

    fn exit_block(&mut self) -> Option<BlockContext> {
        if let Some(parent) = self.block_stack.pop() {
            self.current_block = parent;
            self.block_context_stack.pop()
        } else {
            None
        }
    }

    fn ancestor_loop_start_index(&self) -> Option<u32> {
        for ctx in self.block_context_stack.iter().rev() {
            match ctx {
                BlockContext::Loop { start_index } => return Some(*start_index),
                _ => {}
            }
        }
        None
    }

    fn ancestor_statement_context(&self) -> Option<&BlockContext> {
        self.block_context_stack.last()
    }

    fn add_statement(&mut self, node: Rc<AstNode>, bytecode_indices: Vec<usize>) {
        self.current_block.borrow_mut().add_child(node, bytecode_indices);
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

            // Exit blocks at their end position, handling ancestor statements
            while pos == self.current_block.borrow().end_pos {
                let context = self.exit_block();
                match context {
                    Some(BlockContext::IfBlock1(if_node)) => {
                        // If this block belongs to an if statement with an else branch,
                        // enter block2
                        if let AstNode::If { has_else, block2, .. } = if_node.as_ref() {
                            if has_else.get() {
                                self.enter_block(block2.clone(), BlockContext::IfBlock2);
                            }
                        }
                    }
                    Some(BlockContext::CaseLabel) => {
                        // Check if case label expects otherwise
                        let case_label = self.current_block.borrow().current_case_label.clone();
                        if let Some(label) = case_label {
                            let expect = label.borrow().expect;
                            match expect {
                                CaseExpect::Otherwise => {
                                    self.current_block.borrow_mut().current_case_label = None;
                                    // Find the ancestor case statement and add otherwise
                                    // We need to find the case statement in the current block's children
                                    let children = self.current_block.borrow().children.clone();
                                    for child in children.iter().rev() {
                                        if let AstNode::Case { otherwise, potential_otherwise_pos, .. } = child.node.as_ref() {
                                            let ow = Rc::new(RefCell::new(OtherwiseNode::new()));
                                            otherwise.borrow_mut().replace(ow.clone());
                                            // Tag the otherwise position
                                            let ow_pos = potential_otherwise_pos.get();
                                            if ow_pos >= 0 {
                                                if let Some(&ow_index) = self.bytecode_pos_map.get(&(ow_pos as usize)) {
                                                    self.bytecode_tags[ow_index].tag = BytecodeTag::EndCase;
                                                }
                                            }
                                            self.enter_block(ow.borrow().block.clone(), BlockContext::CaseOtherwise);
                                            break;
                                        }
                                    }
                                }
                                CaseExpect::End => {
                                    self.current_block.borrow_mut().current_case_label = None;
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
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

        let mut next_block: Option<(Rc<RefCell<BlockNode>>, BlockContext)> = None;
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
                next_block = Some((block, BlockContext::Tell));
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
                self.translate_jmp(index, obj, &mut next_block)
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
                // This op denotes a case statement (repeat with in is handled via tags)
                return self.translate_peek(index);
            }

            OpCode::Pop => {
                if self.bytecode_tags[index].tag == BytecodeTag::EndCase {
                    // End of case statement, already recognized
                    return 1;
                }
                if obj == 1 && self.stack.len() == 1 {
                    // Unused value on stack - end of case statement with no labels
                    let value = self.pop_with_indices(&mut collected_indices);
                    Some(Rc::new(AstNode::Case {
                        value,
                        first_label: RefCell::new(None),
                        otherwise: RefCell::new(None),
                        end_pos: Cell::new(-1),
                        potential_otherwise_pos: Cell::new(-1),
                    }))
                } else {
                    // Pop before return within case statement, no translation needed
                    return 1;
                }
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

        if let Some((block, context)) = next_block {
            self.enter_block(block, context);
        }

        1
    }

    fn translate_jmp(&mut self, index: usize, obj: i64, next_block: &mut Option<(Rc<RefCell<BlockNode>>, BlockContext)>) -> Option<Rc<AstNode>> {
        let bytecode_array = &self.handler.bytecode_array;
        let bytecode = &bytecode_array[index];
        let target_pos = bytecode.pos + obj as usize;

        let target_index = match self.bytecode_pos_map.get(&target_pos) {
            Some(&idx) => idx,
            None => return Some(Rc::new(AstNode::Comment("ERROR: Invalid jump target".to_string()))),
        };

        // Check for exit repeat / next repeat
        if let Some(ancestor_loop_start) = self.ancestor_loop_start_index() {
            if target_index > 0 {
                let prev_bytecode = &bytecode_array[target_index - 1];
                if prev_bytecode.opcode == OpCode::EndRepeat
                    && self.bytecode_tags[target_index - 1].owner_loop == ancestor_loop_start
                {
                    return Some(Rc::new(AstNode::ExitRepeat));
                }
            }

            if self.bytecode_tags[target_index].tag == BytecodeTag::NextRepeatTarget
                && self.bytecode_tags[target_index].owner_loop == ancestor_loop_start
            {
                return Some(Rc::new(AstNode::NextRepeat));
            }
        }

        // Check for else branch or case statement jmp
        if index + 1 < bytecode_array.len() {
            let next_bytecode = &bytecode_array[index + 1];
            if next_bytecode.pos as u32 == self.current_block.borrow().end_pos {
                // Check ancestor statement context
                if let Some(ctx) = self.ancestor_statement_context().cloned() {
                    match ctx {
                        BlockContext::IfBlock1(ref if_node) => {
                            // Set up else branch
                            if let AstNode::If { has_else, block2, .. } = if_node.as_ref() {
                                has_else.set(true);
                                block2.borrow_mut().end_pos = target_pos as u32;
                            }
                            return None; // if statement amended, nothing to push
                        }
                        BlockContext::CaseLabel => {
                            // Case statement jmp - find ancestor case to set end position
                            // The case statement is in the grandparent block (block_stack[-2])
                            if self.block_stack.len() >= 2 {
                                let grandparent = &self.block_stack[self.block_stack.len() - 2];
                                let children = grandparent.borrow().children.clone();
                                for child in children.iter().rev() {
                                    if let AstNode::Case { end_pos, potential_otherwise_pos, .. } = child.node.as_ref() {
                                        potential_otherwise_pos.set(bytecode.pos as i32);
                                        end_pos.set(target_pos as i32);
                                        self.bytecode_tags[target_index].tag = BytecodeTag::EndCase;
                                        return None;
                                    }
                                }
                            }
                            return None;
                        }
                        _ => {}
                    }
                }
            }
        }

        // Check for case statement starting with 'otherwise'
        if target_index < bytecode_array.len() {
            let target_bytecode = &bytecode_array[target_index];
            if target_bytecode.opcode == OpCode::Pop && target_bytecode.obj == 1 {
                let value = self.pop();
                let case_stmt = Rc::new(AstNode::Case {
                    value,
                    first_label: RefCell::new(None),
                    otherwise: RefCell::new(None),
                    end_pos: Cell::new(target_pos as i32),
                    potential_otherwise_pos: Cell::new(-1),
                });
                self.bytecode_tags[target_index].tag = BytecodeTag::EndCase;
                // Add otherwise
                let ow = Rc::new(RefCell::new(OtherwiseNode::new()));
                if let AstNode::Case { otherwise, .. } = case_stmt.as_ref() {
                    otherwise.borrow_mut().replace(ow.clone());
                }
                *next_block = Some((ow.borrow().block.clone(), BlockContext::CaseOtherwise));
                return Some(case_stmt);
            }
        }

        Some(Rc::new(AstNode::Comment("ERROR: Could not identify jmp".to_string())))
    }

    fn translate_jmpifz_with_indices(&mut self, index: usize, obj: i64, next_block: &mut Option<(Rc<RefCell<BlockNode>>, BlockContext)>, indices: &mut Vec<usize>) -> Option<Rc<AstNode>> {
        let bytecode = &self.handler.bytecode_array[index];
        let end_pos = (bytecode.pos as i64 + obj) as u32;
        let tag = self.bytecode_tags[index].tag;

        match tag {
            BytecodeTag::RepeatWhile => {
                let condition = self.pop_with_indices(indices);
                let block = Rc::new(RefCell::new(BlockNode::new()));
                block.borrow_mut().end_pos = end_pos;
                *next_block = Some((block.clone(), BlockContext::Loop { start_index: index as u32 }));
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
                *next_block = Some((block.clone(), BlockContext::Loop { start_index: index as u32 }));
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
                *next_block = Some((block.clone(), BlockContext::Loop { start_index: index as u32 }));
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
                let if_node = Rc::new(AstNode::If {
                    condition,
                    block1: block1.clone(),
                    block2,
                    has_else: Cell::new(false),
                });
                *next_block = Some((block1, BlockContext::IfBlock1(if_node.clone())));
                Some(if_node)
            }
        }
    }

    /// Handle the Peek opcode - case statement processing
    /// This recursively processes bytecodes until the comparison (eq/nteq) is found,
    /// following the ProjectorRays C++ implementation.
    fn translate_peek(&mut self, index: usize) -> usize {
        let prev_label = self.current_block.borrow().current_case_label.clone();
        let original_stack_size = self.stack.len();

        // Process bytecodes until we find the eq/nteq comparison
        // This follows the C++ do-while pattern: process a bytecode, then check the next one
        let mut curr_index = index + 1;
        loop {
            if curr_index >= self.handler.bytecode_array.len() {
                break;
            }
            self.current_bytecode_index = curr_index;
            let consumed = self.translate_bytecode(curr_index);
            curr_index += consumed;
            // Check if the next bytecode is the comparison
            if curr_index < self.handler.bytecode_array.len() {
                let next_bc = &self.handler.bytecode_array[curr_index];
                if self.stack.len() == original_stack_size + 1
                    && (next_bc.opcode == OpCode::Eq || next_bc.opcode == OpCode::NtEq)
                {
                    break;
                }
            }
        }

        if curr_index >= self.handler.bytecode_array.len() {
            let error = Rc::new(AstNode::Comment("ERROR: Expected eq or nteq!".to_string()));
            self.add_statement(error, vec![index]);
            return curr_index - index + 1;
        }

        // Check if the comparison is <> (equivalent case) or = (new case)
        let not_eq = self.handler.bytecode_array[curr_index].opcode == OpCode::NtEq;
        let case_value = self.pop(); // The case value to compare against

        curr_index += 1;
        if curr_index >= self.handler.bytecode_array.len()
            || self.handler.bytecode_array[curr_index].opcode != OpCode::JmpIfZ
        {
            let error = Rc::new(AstNode::Comment("ERROR: Expected jmpifz!".to_string()));
            self.add_statement(error, vec![index]);
            return curr_index - index + 1;
        }

        let jmpifz = &self.handler.bytecode_array[curr_index];
        let jmp_pos = (jmpifz.pos as i64 + jmpifz.obj) as usize;
        let target_index = self.bytecode_pos_map.get(&jmp_pos).copied().unwrap_or(0);

        let expect = if not_eq {
            CaseExpect::Or
        } else if target_index < self.handler.bytecode_array.len()
            && self.handler.bytecode_array[target_index].opcode == OpCode::Peek
        {
            CaseExpect::Next
        } else if target_index < self.handler.bytecode_array.len()
            && self.handler.bytecode_array[target_index].opcode == OpCode::Pop
            && self.handler.bytecode_array[target_index].obj == 1
            && (target_index == 0
                || self.handler.bytecode_array[target_index - 1].opcode != OpCode::Jmp
                || (self.handler.bytecode_array[target_index - 1].pos as i64
                    + self.handler.bytecode_array[target_index - 1].obj)
                    == self.handler.bytecode_array[target_index].pos as i64)
        {
            CaseExpect::End
        } else {
            CaseExpect::Otherwise
        };

        let curr_label = Rc::new(RefCell::new(CaseLabelNode::new(case_value, expect)));
        self.current_block.borrow_mut().current_case_label = Some(curr_label.clone());

        if prev_label.is_none() {
            // First case label - create the case statement
            let peeked_value = self.pop();
            let case_stmt = Rc::new(AstNode::Case {
                value: peeked_value,
                first_label: RefCell::new(Some(curr_label.clone())),
                otherwise: RefCell::new(None),
                end_pos: Cell::new(-1),
                potential_otherwise_pos: Cell::new(-1),
            });
            self.add_statement(case_stmt, vec![index]);
        } else if let Some(ref prev) = prev_label {
            let prev_expect = prev.borrow().expect;
            if prev_expect == CaseExpect::Or {
                prev.borrow_mut().next_or = Some(curr_label.clone());
            } else if prev_expect == CaseExpect::Next {
                prev.borrow_mut().next_label = Some(curr_label.clone());
            }
        }

        // Create a block for the case label body (unless expecting another equivalent case)
        if expect != CaseExpect::Or {
            let block = Rc::new(RefCell::new(BlockNode::new()));
            block.borrow_mut().end_pos = jmp_pos as u32;
            curr_label.borrow_mut().block = block.clone();
            self.enter_block(block, BlockContext::CaseLabel);
        }

        curr_index - index + 1
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
                "getProp" | "getPropRef" if (nargs == 3 || nargs == 4) => {
                    if let Some(datum) = args[1].get_value() {
                        if datum.datum_type == DatumType::Symbol {
                            let obj = args[0].clone();
                            let prop_name = datum.string_value.clone();
                            let i = args[2].clone();
                            let i2 = if nargs == 4 { Some(args[3].clone()) } else { None };
                            return Some(Rc::new(AstNode::ObjPropIndex {
                                obj,
                                prop: prop_name,
                                index: i,
                                index2: i2,
                            }));
                        }
                    }
                }
                "setProp" if (nargs == 4 || nargs == 5) => {
                    if let Some(datum) = args[1].get_value() {
                        if datum.datum_type == DatumType::Symbol {
                            let obj = args[0].clone();
                            let prop_name = datum.string_value.clone();
                            let i = args[2].clone();
                            let i2 = if nargs == 5 { Some(args[3].clone()) } else { None };
                            let prop_expr = Rc::new(AstNode::ObjPropIndex {
                                obj,
                                prop: prop_name,
                                index: i,
                                index2: i2,
                            });
                            let val = args[nargs - 1].clone();
                            return Some(Rc::new(AstNode::Assignment {
                                variable: prop_expr,
                                value: val,
                                force_verbose: false,
                            }));
                        }
                    }
                }
                "count" if nargs == 2 => {
                    if let Some(datum) = args[1].get_value() {
                        if datum.datum_type == DatumType::Symbol {
                            let obj = args[0].clone();
                            let prop_name = datum.string_value.clone();
                            let prop_expr = Rc::new(AstNode::ObjProp { obj, prop: prop_name });
                            return Some(Rc::new(AstNode::ObjProp { obj: prop_expr, prop: "count".to_string() }));
                        }
                    }
                }
                "setContents" | "setContentsAfter" | "setContentsBefore" if nargs == 2 => {
                    let put_type = match method {
                        "setContents" => PutType::Into,
                        "setContentsAfter" => PutType::After,
                        _ => PutType::Before,
                    };
                    return Some(Rc::new(AstNode::Put {
                        put_type,
                        variable: args[0].clone(),
                        value: args[1].clone(),
                    }));
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

    fn read_v4_property(&mut self, property_type: i64, property_id: i32) -> Option<Rc<AstNode>> {
        match property_type {
            0x00 => {
                if property_id <= 0x0b {
                    // Movie/system property
                    let prop_name = get_movie_property_name(property_id);
                    Some(Rc::new(AstNode::The(prop_name)))
                } else {
                    // Last chunk
                    let string = self.pop();
                    let chunk_type = match property_id - 0x0b {
                        1 => ChunkExprType::Char,
                        2 => ChunkExprType::Word,
                        3 => ChunkExprType::Item,
                        4 => ChunkExprType::Line,
                        _ => ChunkExprType::Char,
                    };
                    Some(Rc::new(AstNode::LastStringChunk { chunk_type, obj: string }))
                }
            }
            0x01 => {
                // Number of chunks
                let string = self.pop();
                let chunk_type = match property_id {
                    1 => ChunkExprType::Char,
                    2 => ChunkExprType::Word,
                    3 => ChunkExprType::Item,
                    4 => ChunkExprType::Line,
                    _ => ChunkExprType::Char,
                };
                Some(Rc::new(AstNode::StringChunkCount { chunk_type, obj: string }))
            }
            0x02 => {
                // Menu property
                let menu_id = self.pop();
                Some(Rc::new(AstNode::MenuProp { menu_id, prop: property_id as u32 }))
            }
            0x03 => {
                // Menu item property
                let menu_id = self.pop();
                let item_id = self.pop();
                Some(Rc::new(AstNode::MenuItemProp { menu_id, item_id, prop: property_id as u32 }))
            }
            0x04 => {
                // Sound property
                let sound_id = self.pop();
                Some(Rc::new(AstNode::SoundProp { sound_id, prop: property_id as u32 }))
            }
            0x05 => {
                // Resource property - unused
                Some(Rc::new(AstNode::Comment("ERROR: Resource property".to_string())))
            }
            0x06 => {
                // Sprite property
                let sprite_id = self.pop();
                Some(Rc::new(AstNode::SpriteProp { sprite_id, prop: property_id as u32 }))
            }
            0x07 => {
                // Animation property
                let prop_name = get_animation_property_name(property_id);
                Some(Rc::new(AstNode::The(prop_name)))
            }
            0x08 => {
                // Animation 2 property
                let prop_name = get_animation2_property_name(property_id);
                if property_id == 0x02 && self.version >= 500 {
                    let cast_lib = self.pop();
                    // Check if castLib is non-zero
                    let is_zero = if let AstNode::Literal(d) = cast_lib.as_ref() {
                        d.datum_type == DatumType::Int && d.int_value == 0
                    } else {
                        false
                    };
                    if !is_zero {
                        let cast_lib_node = Rc::new(AstNode::Member {
                            member_type: "castLib".to_string(),
                            member_id: cast_lib,
                            cast_id: None,
                        });
                        return Some(Rc::new(AstNode::TheProp { obj: cast_lib_node, prop: prop_name }));
                    }
                }
                Some(Rc::new(AstNode::The(prop_name)))
            }
            0x09..=0x15 => {
                // Member properties (generic cast member, chunk of cast member, field, etc.)
                let prop_name = get_member_property_name(property_id);
                let cast_id = if self.version >= 500 {
                    Some(self.pop())
                } else {
                    None
                };
                let member_id = self.pop();
                let prefix = if property_type == 0x0b || property_type == 0x0c {
                    "field"
                } else if property_type == 0x14 || property_type == 0x15 {
                    "script"
                } else if self.version >= 500 {
                    "member"
                } else {
                    "cast"
                };
                let member = Rc::new(AstNode::Member {
                    member_type: prefix.to_string(),
                    member_id,
                    cast_id,
                });
                let entity = if property_type == 0x0a || property_type == 0x0c || property_type == 0x15 {
                    self.read_chunk_ref(member)
                } else {
                    member
                };
                Some(Rc::new(AstNode::TheProp { obj: entity, prop: prop_name }))
            }
            _ => {
                Some(Rc::new(AstNode::Comment(format!("ERROR: Unknown property type {}", property_type))))
            }
        }
    }

    /// Generate output lines from the parsed AST by walking the tree in render order.
    /// This ensures bytecode indices are correctly associated with the lines they belong to.
    fn generate_output(&self) -> DecompiledHandler {
        let name = self.lctx.names.get(self.handler.name_id as usize)
            .cloned()
            .unwrap_or_else(|| format!("handler_{}", self.handler.name_id));

        let args: Vec<String> = self.handler.argument_name_ids.iter()
            .filter_map(|&id| self.lctx.names.get(id as usize).cloned())
            .collect();

        let dot = self.version >= 500;
        let mut lines = Vec::new();
        let mut bytecode_to_line = HashMap::new();

        Self::collect_block_lines(&self.root_block.borrow(), dot, 0, &mut lines, &mut bytecode_to_line);

        DecompiledHandler {
            name,
            arguments: args,
            lines,
            bytecode_to_line,
        }
    }

    /// Render a line of text from an AST node (just the text, no newline)
    fn render_node_text(node: &AstNode, dot: bool) -> String {
        let mut code = CodeWriter::new();
        node.write_script(&mut code, dot, false);
        code.into_string()
    }

    /// Add a line with bytecode index tracking
    fn push_line(text: String, indices: Vec<usize>, indent: u32, lines: &mut Vec<DecompiledLine>, bytecode_to_line: &mut HashMap<usize, usize>) {
        let line_idx = lines.len();
        let mut sorted_indices = indices;
        sorted_indices.sort_unstable();
        let spans = tokenize_line(&text);
        for &bc_idx in &sorted_indices {
            bytecode_to_line.entry(bc_idx).or_insert(line_idx);
        }
        lines.push(DecompiledLine {
            text,
            bytecode_indices: sorted_indices,
            indent,
            spans,
        });
    }

    /// Walk a block's children in render order, collecting lines with correct indices
    fn collect_block_lines(block: &BlockNode, dot: bool, indent: u32, lines: &mut Vec<DecompiledLine>, bytecode_to_line: &mut HashMap<usize, usize>) {
        for child in &block.children {
            Self::collect_statement_lines(&child.node, &child.bytecode_indices, dot, indent, lines, bytecode_to_line);
        }
    }

    /// Walk a single statement, rendering its header line with its indices,
    /// recursing into inner blocks, and adding structural closing lines
    fn collect_statement_lines(node: &AstNode, indices: &[usize], dot: bool, indent: u32, lines: &mut Vec<DecompiledLine>, bytecode_to_line: &mut HashMap<usize, usize>) {
        match node {
            AstNode::If { condition, block1, block2, has_else } => {
                // "if <condition> then"
                let mut code = CodeWriter::new();
                code.write("if ");
                condition.write_script(&mut code, dot, false);
                code.write(" then");
                Self::push_line(code.into_string(), indices.to_vec(), indent, lines, bytecode_to_line);

                // Block1 contents
                Self::collect_block_lines(&block1.borrow(), dot, indent + 1, lines, bytecode_to_line);

                // Else
                if has_else.get() && !block2.borrow().children.is_empty() {
                    Self::push_line("else".to_string(), vec![], indent, lines, bytecode_to_line);
                    Self::collect_block_lines(&block2.borrow(), dot, indent + 1, lines, bytecode_to_line);
                }

                // End if
                Self::push_line("end if".to_string(), vec![], indent, lines, bytecode_to_line);
            }

            AstNode::RepeatWhile { condition, block, .. } => {
                let mut code = CodeWriter::new();
                code.write("repeat while ");
                condition.write_script(&mut code, dot, false);
                Self::push_line(code.into_string(), indices.to_vec(), indent, lines, bytecode_to_line);
                Self::collect_block_lines(&block.borrow(), dot, indent + 1, lines, bytecode_to_line);
                Self::push_line("end repeat".to_string(), vec![], indent, lines, bytecode_to_line);
            }

            AstNode::RepeatWithIn { var_name, list, block, .. } => {
                let mut code = CodeWriter::new();
                code.write("repeat with ");
                code.write(var_name);
                code.write(" in ");
                list.write_script(&mut code, dot, false);
                Self::push_line(code.into_string(), indices.to_vec(), indent, lines, bytecode_to_line);
                Self::collect_block_lines(&block.borrow(), dot, indent + 1, lines, bytecode_to_line);
                Self::push_line("end repeat".to_string(), vec![], indent, lines, bytecode_to_line);
            }

            AstNode::RepeatWithTo { var_name, start, end, up, block, .. } => {
                let mut code = CodeWriter::new();
                code.write("repeat with ");
                code.write(var_name);
                code.write(" = ");
                start.write_script(&mut code, dot, false);
                if *up {
                    code.write(" to ");
                } else {
                    code.write(" down to ");
                }
                end.write_script(&mut code, dot, false);
                Self::push_line(code.into_string(), indices.to_vec(), indent, lines, bytecode_to_line);
                Self::collect_block_lines(&block.borrow(), dot, indent + 1, lines, bytecode_to_line);
                Self::push_line("end repeat".to_string(), vec![], indent, lines, bytecode_to_line);
            }

            AstNode::Tell { window, block } => {
                let mut code = CodeWriter::new();
                code.write("tell ");
                window.write_script(&mut code, dot, false);
                Self::push_line(code.into_string(), indices.to_vec(), indent, lines, bytecode_to_line);
                Self::collect_block_lines(&block.borrow(), dot, indent + 1, lines, bytecode_to_line);
                Self::push_line("end tell".to_string(), vec![], indent, lines, bytecode_to_line);
            }

            AstNode::Case { value, first_label, otherwise, .. } => {
                // "case <value> of"
                let mut code = CodeWriter::new();
                code.write("case ");
                value.write_script(&mut code, dot, false);
                code.write(" of");
                Self::push_line(code.into_string(), indices.to_vec(), indent, lines, bytecode_to_line);

                // Case labels
                let mut current_label = first_label.borrow().clone();
                while let Some(label) = current_label {
                    let label_ref = label.borrow();
                    Self::collect_case_label_lines(&label_ref, dot, indent + 1, lines, bytecode_to_line);
                    current_label = label_ref.next_label.clone();
                }

                // Otherwise
                if let Some(ow) = &*otherwise.borrow() {
                    let ow_ref = ow.borrow();
                    Self::push_line("otherwise:".to_string(), vec![], indent + 1, lines, bytecode_to_line);
                    Self::collect_block_lines(&ow_ref.block.borrow(), dot, indent + 2, lines, bytecode_to_line);
                }

                Self::push_line("end case".to_string(), vec![], indent, lines, bytecode_to_line);
            }

            // Simple statements (non-compound) - render as a single line
            _ => {
                let text = Self::render_node_text(node, dot);
                Self::push_line(text, indices.to_vec(), indent, lines, bytecode_to_line);
            }
        }
    }

    /// Collect lines for a case label
    fn collect_case_label_lines(label: &CaseLabelNode, dot: bool, indent: u32, lines: &mut Vec<DecompiledLine>, bytecode_to_line: &mut HashMap<usize, usize>) {
        // Render the label value(s)
        let mut code = CodeWriter::new();
        label.value.write_script(&mut code, dot, false);

        // Chained "or" values
        let mut current_or = label.next_or.clone();
        while let Some(or_label) = current_or {
            code.write(", ");
            or_label.borrow().value.write_script(&mut code, dot, false);
            current_or = or_label.borrow().next_or.clone();
        }

        code.write(":");
        Self::push_line(code.into_string(), vec![], indent, lines, bytecode_to_line);

        // Case label block contents
        Self::collect_block_lines(&label.block.borrow(), dot, indent + 1, lines, bytecode_to_line);
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
        0x07 => "short time".to_string(),
        0x08 => "abbr time".to_string(),
        0x09 => "long time".to_string(),
        0x0a => "short date".to_string(),
        0x0b => "abbr date".to_string(),
        _ => format!("movieProp_{}", id),
    }
}

fn get_animation_property_name(id: i32) -> String {
    match id {
        0x01 => "beepOn".to_string(),
        0x02 => "buttonStyle".to_string(),
        0x03 => "centerStage".to_string(),
        0x04 => "checkBoxAccess".to_string(),
        0x05 => "checkBoxType".to_string(),
        0x06 => "colorDepth".to_string(),
        0x07 => "colorQD".to_string(),
        0x08 => "exitLock".to_string(),
        0x09 => "fixStageSize".to_string(),
        0x0a => "fullColorPermit".to_string(),
        0x0b => "imageDirect".to_string(),
        0x0c => "doubleClick".to_string(),
        _ => format!("animProp_{}", id),
    }
}

fn get_animation2_property_name(id: i32) -> String {
    match id {
        0x01 => "the number of castMembers".to_string(),
        0x02 => "the number of castMembers".to_string(),
        0x03 => "the number of menus".to_string(),
        _ => format!("anim2Prop_{}", id),
    }
}

fn get_member_property_name(id: i32) -> String {
    match id {
        0x01 => "name".to_string(),
        0x02 => "text".to_string(),
        0x03 => "textStyle".to_string(),
        0x04 => "textFont".to_string(),
        0x05 => "textHeight".to_string(),
        0x06 => "textAlign".to_string(),
        0x07 => "textSize".to_string(),
        0x08 => "picture".to_string(),
        0x09 => "hilite".to_string(),
        0x0a => "number".to_string(),
        0x0b => "size".to_string(),
        0x0c => "loop".to_string(),
        0x0d => "duration".to_string(),
        0x0e => "controller".to_string(),
        0x0f => "directToStage".to_string(),
        0x10 => "sound".to_string(),
        0x11 => "foreColor".to_string(),
        0x12 => "backColor".to_string(),
        _ => format!("memberProp_{}", id),
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
