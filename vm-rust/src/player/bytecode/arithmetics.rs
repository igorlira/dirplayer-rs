use crate::{director::{chunks::handler::Bytecode, lingo::datum::{Datum, DatumType}}, player::{datum_formatting::format_datum, datum_operations::{add_datums, subtract_datums}, reserve_player_mut, HandlerExecutionResult, HandlerExecutionResultContext, ScriptError}};

use super::handler_manager::BytecodeHandlerContext;

pub struct ArithmeticsBytecodeHandler { }

impl ArithmeticsBytecodeHandler {
  pub fn add(_: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let (left, right) = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        let right = scope.stack.pop().unwrap();
        let left = scope.stack.pop().unwrap();
        (left, right)
      };
      let right = player.get_datum(right);
      let left = player.get_datum(left);

      let result_id = {
        let result = add_datums(left.to_owned(), right.to_owned(), player)?;
        player.alloc_datum(result)
      };
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn sub(_: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let (left, right) = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        let right = scope.stack.pop().unwrap();
        let left = scope.stack.pop().unwrap();
        (left, right)
      };
      let right = player.get_datum(right);
      let left = player.get_datum(left);

      let result = subtract_datums(left.to_owned(), right.to_owned(), player)?;
      let result_id = player.alloc_datum(result);
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn mod_handler(_: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let (left, right) = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        let right = scope.stack.pop().unwrap();
        let left = scope.stack.pop().unwrap();
        (left, right)
      };
      let right = player.get_datum(right);
      let left = player.get_datum(left);

      let result = match (left, right) {
        (Datum::Int(left), Datum::Int(right)) => Datum::Int(left % right),
        (Datum::Int(left), Datum::Float(right)) => Datum::Float((*left as f32) % right),
        (Datum::Float(left), Datum::Int(right)) => Datum::Float(*left % (*right as f32)),
        (Datum::Float(left), Datum::Float(right)) => Datum::Float(left % right),
        (Datum::List(_, list, _), Datum::Float(right)) => {
          let mut new_list = vec![];
          for item in list {
            let item_datum = player.get_datum(*item);
            let result_datum = match item_datum {
              Datum::Int(n) => Datum::Int(((*n as f32) % right) as i32),
              Datum::Float(n) => Datum::Int((n % right) as i32),
              _ => return Err(ScriptError::new(format!("Modulus operator in list only works with ints and floats. Given: {}", format_datum(*item, player)))),
            };
            new_list.push(result_datum);
          }
          let mut ref_list = vec![];
          for item in new_list {
            ref_list.push(player.alloc_datum(item));
          }
          Datum::List(DatumType::List, ref_list, false)
        }
        (Datum::List(_, list, _), Datum::Int(right)) => {
          let mut new_list = vec![];
          for item in list {
            let item_datum = player.get_datum(*item);
            let result_datum = match item_datum {
              Datum::Int(n) => Datum::Int((*n % right) as i32),
              Datum::Float(n) => Datum::Int((*n % *right as f32) as i32),
              _ => return Err(ScriptError::new(format!("Modulus operator in list only works with ints and floats. Given: {}", format_datum(*item, player)))),
            };
            new_list.push(result_datum);
          }
          let mut ref_list = vec![];
          for item in new_list {
            ref_list.push(player.alloc_datum(item));
          }
          Datum::List(DatumType::List, ref_list, false)
        }
        _ => return Err(ScriptError::new(format!("Modulus operator only works with ints and floats (given {} and {})", left.type_str(), right.type_str()))),
      };
      let result_id = player.alloc_datum(result);
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn div(_: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let (left, right) = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        let right = scope.stack.pop().unwrap();
        let left = scope.stack.pop().unwrap();
        (left, right)
      };
      let right = player.get_datum(right);
      let left = player.get_datum(left);

      let result = match (left, right) {
        (Datum::Int(left), Datum::Int(right)) => Datum::Int(left / right),
        (Datum::Int(left), Datum::Float(right)) => Datum::Float((*left as f32) / right),
        (Datum::Float(left), Datum::Int(right)) => Datum::Float(*left / (*right as f32)),
        (Datum::Float(left), Datum::Float(right)) => Datum::Float(left / right),
        (Datum::Int(left), Datum::String(right)) => {
          let right = right.parse::<f32>().map_err(|_| ScriptError::new(format!("Cannot divide int by string: {}", right)))?;
          Datum::Float((*left as f32) / right)
        }
        (Datum::Float(left), Datum::String(right)) => {
          let right = right.parse::<f32>().map_err(|_| ScriptError::new(format!("Cannot divide float by string: {}", right)))?;
          Datum::Float(left / right)
        }
        (Datum::Void, _) => Datum::Int(0),
        _ => return Err(ScriptError::new(format!("Div operator only works with ints and floats (Provided: {} and {})", left.type_str(), right.type_str()))),
      };
      let result_id = player.alloc_datum(result);
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn mul(_: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let (left_ref, right_ref) = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        let right = scope.stack.pop().unwrap();
        let left = scope.stack.pop().unwrap();
        (left, right)
      };
      let right = player.get_datum(right_ref);
      let left = player.get_datum(left_ref);

      let result = match (left, right) {
        (Datum::Int(left), Datum::Int(right)) => Datum::Int(left * right),
        (Datum::Int(left), Datum::Float(right)) => Datum::Float((*left as f32) * right),
        (Datum::Float(left), Datum::Int(right)) => Datum::Float(*left * (*right as f32)),
        (Datum::Float(left), Datum::Float(right)) => Datum::Float(left * right),
        (Datum::IntRect((x1, y1, x2, y2)), Datum::Int(right)) => Datum::IntRect((x1 * *right, y1 * *right, x2 * *right, y2 * *right)),
        (Datum::IntPoint((x, y)), Datum::Int(right)) => Datum::IntPoint((x * *right, y * *right)),
        (Datum::List(_, list, _), Datum::Float(right)) => {
          let mut new_list = vec![];
          for item in list {
            let item_datum = player.get_datum(*item);
            let result_datum = match item_datum {
              Datum::Int(n) => Datum::Float((*n as f32) * right),
              Datum::Float(n) => Datum::Float(n * right),
              _ => return Err(ScriptError::new(format!("Mul operator in list only works with ints and floats. Given: {}", format_datum(*item, player)))),
            };
            new_list.push(result_datum);
          }
          let mut ref_list = vec![];
          for item in new_list {
            ref_list.push(player.alloc_datum(item));
          }
          Datum::List(DatumType::List, ref_list, false)
        }
        _ => return Err(ScriptError::new(format!("Mul operator only works with ints and floats. Given: {}, {}", format_datum(left_ref, player), format_datum(right_ref, player)))),
      };
      let result_id = player.alloc_datum(result);
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }

  pub fn inv(_: &Bytecode, ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResultContext, ScriptError> {
    reserve_player_mut(|player| {
      let value_id = {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        scope.stack.pop().unwrap()
      };
      let value = player.get_datum(value_id);
      let result = match value {
        Datum::Int(n) => Datum::Int(-n),
        Datum::Float(n) => Datum::Float(-n),
        Datum::IntPoint((x, y)) => Datum::IntPoint((-x, -y)),
        _ => return Err(ScriptError::new(format!("Cannot inv non-numeric value: {}", value.type_str()))),
      };
      let result_id = player.alloc_datum(result);
      let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
      scope.stack.push(result_id);
      Ok(HandlerExecutionResultContext { result: HandlerExecutionResult::Advance })
    })
  }
}
