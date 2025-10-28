use fxhash::FxHashMap;

use super::{
    cast_lib::{CastMemberRef, INVALID_CAST_MEMBER_REF},
    script::ScriptHandlerRef,
    script_ref::ScriptInstanceRef,
    DatumRef,
};

pub type ScopeRef = usize;

// #[derive(Clone)]
pub struct Scope {
    pub scope_ref: ScopeRef,
    pub script_ref: CastMemberRef,
    pub receiver: Option<ScriptInstanceRef>,
    pub handler_name_id: u16,
    pub args: Vec<DatumRef>,
    pub bytecode_index: usize,
    pub locals: FxHashMap<String, DatumRef>,
    pub loop_return_indices: Vec<usize>,
    pub return_value: DatumRef,
    pub stack: Vec<DatumRef>,
    pub passed: bool,
}

pub struct ScopeResult {
    pub return_value: DatumRef,
    pub passed: bool,
}

impl Scope {
    pub fn pop_n(&mut self, n: usize) -> Vec<DatumRef> {
        let result = self.stack[self.stack.len() - n..].to_vec();
        for _ in 0..n {
            self.stack.pop();
        }
        result
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
            stack: vec![],
            passed: false,
        }
    }

    pub fn reset(&mut self) {
        self.script_ref = INVALID_CAST_MEMBER_REF;
        self.receiver = None;
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
