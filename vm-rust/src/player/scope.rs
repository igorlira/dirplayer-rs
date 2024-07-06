use std::collections::HashMap;

use super::{cast_lib::CastMemberRef, script::ScriptHandlerRef, script_ref::ScriptInstanceRef, DatumRef, VOID_DATUM_REF};

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
  pub locals: HashMap<String, DatumRef>,
  pub loop_return_indices: Vec<usize>,
  pub return_value: DatumRef,
  pub stack: Vec<DatumRef>,
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

  pub fn new(
    scope_ref: ScopeRef,
    script_ref: CastMemberRef, 
    receiver: Option<ScriptInstanceRef>, 
    handler_ref: ScriptHandlerRef, 
    handler_name_id: u16,
    args: Vec<DatumRef>
  ) -> Scope {
    Scope {
      scope_ref,
      script_ref,
      receiver,
      handler_ref,
      handler_name_id,
      args,
      bytecode_index: 0,
      locals: HashMap::new(),
      loop_return_indices: vec![],
      return_value: VOID_DATUM_REF.clone(),
      stack: vec![],
      passed: false,
    }
  }
}