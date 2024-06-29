use num::ToPrimitive;

use crate::{director::{chunks::handler::Bytecode, lingo::datum::{datum_bool, Datum}}, player::{compare::{datum_equals, datum_greater_than, datum_less_than}, reserve_player_mut, HandlerExecutionResult, HandlerExecutionResultContext, ScriptError}};

use super::handler_manager::BytecodeHandlerContext;

pub struct CompareBytecodeHandler { }

impl CompareBytecodeHandler {
  pub fn gt(_: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let (left, right) = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        let right = scope.stack.pop().unwrap();
        let left = scope.stack.pop().unwrap();
        (left, right)
      };
      let right = player.get_datum(&right);
      let left = player.get_datum(&left);

      let is_gt = datum_greater_than(left, right)?;

      let result_id = player.alloc_datum(datum_bool(is_gt));
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn lt(_: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let (left, right) = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        let right = scope.stack.pop().unwrap();
        let left = scope.stack.pop().unwrap();
        (left, right)
      };
      let right = player.get_datum(&right);
      let left = player.get_datum(&left);

      let is_lt = datum_less_than(left, right)?;

      let result_id = player.alloc_datum(datum_bool(is_lt));
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn lt_eq(_: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let (left, right) = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        let right = scope.stack.pop().unwrap();
        let left = scope.stack.pop().unwrap();
        (left, right)
      };
      let right = player.get_datum(&right);
      let left = player.get_datum(&left);

      let is_lt = datum_less_than(left, right)?;
      let is_eq = datum_equals(left, right, &player.datums)?;

      let result_id = player.alloc_datum(datum_bool(is_lt || is_eq));
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn gt_eq(_: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let (left, right) = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        let right = scope.stack.pop().unwrap();
        let left = scope.stack.pop().unwrap();
        (left, right)
      };
      let right = player.get_datum(&right);
      let left = player.get_datum(&left);

      let is_gt = datum_greater_than(left, right)?;
      let is_eq = datum_equals(left, right, &player.datums)?;

      let result_id = player.alloc_datum(datum_bool(is_gt || is_eq));
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);

      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn not(_: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let obj_id = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        scope.stack.pop().unwrap()
      };
      let obj = player.get_datum(&obj_id);
      let is_not = match obj {
        Datum::Void => true,
        Datum::Int(num) => *num == 0,
        Datum::Float(num) => num.to_u32().unwrap() == 0,
        _ => false,
      };
      let result_id = player.alloc_datum(datum_bool(is_not));
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn nt_eq(_: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let (left, right) = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        let right = scope.stack.pop().unwrap();
        let left = scope.stack.pop().unwrap();
        (left, right)
      };
      let right = player.get_datum(&right);
      let left = player.get_datum(&left);

      let is_eq = datum_equals(left, right, &player.datums)?;

      let result_id = player.alloc_datum(datum_bool(!is_eq));
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn and(_: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let (left, right) = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        let right = scope.stack.pop().unwrap();
        let left = scope.stack.pop().unwrap();
        (left, right)
      };
      let right = player.get_datum(&right);
      let left = player.get_datum(&left);

      let is_and = left.to_bool()? && right.to_bool()?;

      let result_id = player.alloc_datum(datum_bool(is_and));

      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn or(_: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let (left, right) = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        let right = scope.stack.pop().unwrap();
        let left = scope.stack.pop().unwrap();
        (left, right)
      };
      let right = player.get_datum(&right);
      let left = player.get_datum(&left);

      let is_or = left.to_bool()? || right.to_bool()?;

      let result_id = player.alloc_datum(datum_bool(is_or));
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn eq(_: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let (left, right) = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        let right = scope.stack.pop().unwrap();
        let left = scope.stack.pop().unwrap();
        (left, right)
      };
      let right = player.get_datum(&right);
      let left = player.get_datum(&left);

      let is_eq = datum_equals(left, right, &player.datums)?;

      let result_id = player.alloc_datum(datum_bool(is_eq));
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }
}
