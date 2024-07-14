use std::rc::Rc;

use fxhash::FxHashMap;

use crate::director::chunks::handler::HandlerDef;

use super::{cast_lib::CastMemberRef, script::{Script, ScriptHandlerRef}, script_ref::ScriptInstanceRef, DatumRef};

pub type ScopeRef = usize;

// #[derive(Clone)]
pub struct Scope {
  pub scope_ref: ScopeRef,
  pub script_ref: CastMemberRef,
  pub receiver: Option<ScriptInstanceRef>,
  pub handler_ref: ScriptHandlerRef,
  pub handler_name_id: u16,
  pub args: Vec<DatumRef>,
  pub bytecode_index: usize,
  pub locals: FxHashMap<String, DatumRef>,
  pub loop_return_indices: Vec<usize>,
  pub return_value: DatumRef,
  pub stack: Vec<DatumRef>,
  pub passed: bool,
  pub script_rc: Rc<Script>,
  pub handler_rc: Rc<HandlerDef>,
}

impl Scope {
  pub fn pop_n(&mut self, n: usize) -> Vec<DatumRef> {
    let result = self.stack[self.stack.len() - n..].to_vec();
    for _ in 0..n {
      self.stack.pop();
    }
    result
  }

  pub fn new(
    scope_ref: ScopeRef,
    script_ref: CastMemberRef, 
    receiver: Option<ScriptInstanceRef>, 
    handler_ref: ScriptHandlerRef, 
    handler_name_id: u16,
    args: Vec<DatumRef>,
    script_rc: Rc<Script>,
    handler_rc: Rc<HandlerDef>,
  ) -> Scope {
    Scope {
      scope_ref,
      script_ref,
      receiver,
      handler_ref,
      handler_name_id,
      args,
      bytecode_index: 0,
      locals: FxHashMap::default(),
      loop_return_indices: vec![],
      return_value: DatumRef::Void,
      stack: vec![],
      passed: false,
      script_rc,
      handler_rc,
    }
  }
}