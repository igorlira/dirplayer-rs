use std::collections::VecDeque;
use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{
        datum_formatting::format_datum,
        datum_operations::{add_datums, divide_datums, multiply_datums, subtract_datums},
        reserve_player_mut, scope::StackDatum, HandlerExecutionResult, ScriptError,
    },
};

use super::handler_manager::BytecodeHandlerContext;

pub struct ArithmeticsBytecodeHandler {}

impl ArithmeticsBytecodeHandler {
    pub fn add(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (lv, rv) = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                let right = scope.stack.pop_value().unwrap();
                let left = scope.stack.pop_value().unwrap();
                (left, right)
            };
            // Inline int+int: no get_datum, no result alloc — push the sum
            // back inline. Mirrors add_datums' Int/Int arm exactly.
            if let (StackDatum::Int(a), StackDatum::Int(b)) = (&lv, &rv) {
                let sum = a + b;
                player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push_int(sum);
                return Ok(HandlerExecutionResult::Advance);
            }
            let right = rv.into_ref();
            let left = lv.into_ref();
            let right_d = player.get_datum(&right).to_owned();
            let left_d = player.get_datum(&left).to_owned();
            let result = add_datums(left_d, right_d, player)?;
            let result_id = player.alloc_datum(result);
            player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn sub(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (lv, rv) = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                let right = scope.stack.pop_value().unwrap();
                let left = scope.stack.pop_value().unwrap();
                (left, right)
            };
            if let (StackDatum::Int(a), StackDatum::Int(b)) = (&lv, &rv) {
                let diff = a.wrapping_sub(*b);
                player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push_int(diff);
                return Ok(HandlerExecutionResult::Advance);
            }
            let right = rv.into_ref();
            let left = lv.into_ref();
            let right_d = player.get_datum(&right).to_owned();
            let left_d = player.get_datum(&left).to_owned();
            let result = subtract_datums(left_d, right_d, player)?;
            let result_id = player.alloc_datum(result);
            player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    fn safe_mod_int(left: i32, right: i32) -> i32 {
        if right == 0 {
            0
        } else {
            left % right
        }
    }

    fn safe_mod_float(left: f64, right: f64) -> f64 {
        if right == 0.0 {
            0.0
        } else {
            left % right
        }
    }

    pub fn mod_handler(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (left, right) = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                let right = scope.stack.pop().unwrap();
                let left = scope.stack.pop().unwrap();
                (left, right)
            };
            let right = player.get_datum(&right);
            let left = player.get_datum(&left);

            // Treat Void as 0 (Director behavior)
            let left = match left {
                Datum::Void => &Datum::Int(0),
                other => other,
            };
            let right = match right {
                Datum::Void => &Datum::Int(0),
                other => other,
            };

            let result = match (left, right) {
                (Datum::Int(left), Datum::Int(right)) => {
                    Datum::Int(Self::safe_mod_int(*left, *right))
                }
                (Datum::Int(left), Datum::Float(right)) => {
                    Datum::Float(Self::safe_mod_float(*left as f64, *right))
                }
                (Datum::Float(left), Datum::Int(right)) => {
                    Datum::Float(Self::safe_mod_float(*left, *right as f64))
                }
                (Datum::Float(left), Datum::Float(right)) => {
                    Datum::Float(Self::safe_mod_float(*left, *right))
                }
                (Datum::List(_, list, _), Datum::Float(right)) => {
                    let mut new_list = vec![];
                    for item in list {
                        let item_datum = player.get_datum(item);
                        let result_datum = match item_datum {
              Datum::Int(n) => Datum::Int(Self::safe_mod_float(*n as f64, *right) as i32),
              Datum::Float(n) => Datum::Int(Self::safe_mod_float(*n, *right) as i32),
              _ => return Err(ScriptError::new(format!("Modulus operator in list only works with ints and floats. Given: {}", format_datum(item, player)))),
            };
                        new_list.push(result_datum);
                    }
                    let mut ref_list = VecDeque::new();
                    for item in new_list {
                        ref_list.push_back(player.alloc_datum(item));
                    }
                    Datum::List(DatumType::List, ref_list, false)
                }
                (Datum::List(_, list, _), Datum::Int(right)) => {
                    let mut new_list = vec![];
                    for item in list {
                        let item_datum = player.get_datum(item);
                        let result_datum = match item_datum {
              Datum::Int(n) => Datum::Int(Self::safe_mod_int(*n, *right)),
              Datum::Float(n) => Datum::Int(Self::safe_mod_float(*n, *right as f64) as i32),
              _ => return Err(ScriptError::new(format!("Modulus operator in list only works with ints and floats. Given: {}", format_datum(item, player)))),
            };
                        new_list.push(result_datum);
                    }
                    let mut ref_list = VecDeque::new();
                    for item in new_list {
                        ref_list.push_back(player.alloc_datum(item));
                    }
                    Datum::List(DatumType::List, ref_list, false)
                }
                _ => {
                    return Err(ScriptError::new(format!(
                        "Modulus operator only works with ints and floats (given {} and {})",
                        left.type_str(),
                        right.type_str()
                    )))
                }
            };
            let result_id = player.alloc_datum(result);
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn div(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (lv, rv) = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                let right = scope.stack.pop_value().unwrap();
                let left = scope.stack.pop_value().unwrap();
                (left, right)
            };
            if let (StackDatum::Int(a), StackDatum::Int(b)) = (&lv, &rv) {
                // Lingo coerces divisor 0 to 1 (matches divide_datums' Int/Int arm).
                let d = if *b == 0 { 1 } else { *b };
                let q = a / d;
                player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push_int(q);
                return Ok(HandlerExecutionResult::Advance);
            }
            let right = rv.into_ref();
            let left = lv.into_ref();
            let result = divide_datums(left, right, player)?;
            let result_id = player.alloc_datum(result);
            player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn mul(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (lv, rv) = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                let right = scope.stack.pop_value().unwrap();
                let left = scope.stack.pop_value().unwrap();
                (left, right)
            };
            if let (StackDatum::Int(a), StackDatum::Int(b)) = (&lv, &rv) {
                let prod = a * b;
                player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push_int(prod);
                return Ok(HandlerExecutionResult::Advance);
            }
            let right_ref = rv.into_ref();
            let left_ref = lv.into_ref();
            let result = multiply_datums(left_ref, right_ref, player)?;
            let result_id = player.alloc_datum(result);
            player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn inv(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let value_id = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.pop().unwrap()
            };
            let value = player.get_datum(&value_id).clone(); 
            let result_datum = match value {
                Datum::Int(n) => Datum::Int(-n),
                Datum::Float(n) => Datum::Float(-n),
                Datum::Point(vals, flags) => {
                    Datum::Point([-vals[0], -vals[1]], flags)
                }
                Datum::Vector(v) => {
                    Datum::Vector([-v[0], -v[1], -v[2]])
                }
                Datum::List(list_type, items, sorted) => {
                    let mut negated_items = VecDeque::with_capacity(items.len());
                    for item_ref in &items {
                        let item = player.get_datum(item_ref).clone();
                        let negated = match item {
                            Datum::Int(n) => player.alloc_datum(Datum::Int(-n)),
                            Datum::Float(n) => player.alloc_datum(Datum::Float(-n)),
                            _ => return Err(ScriptError::new(format!(
                                "Cannot negate list element of type: {}",
                                item.type_str()
                            ))),
                        };
                        negated_items.push_back(negated);
                    }
                    Datum::List(list_type, negated_items, sorted)
                }
                Datum::Void => Datum::Int(0),
                _ => {
                    return Err(ScriptError::new(format!(
                        "Cannot inv non-numeric value: {}",
                        value.type_str()
                    )))
                }
            };

            let result_id = player.alloc_datum(result_datum);
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }
}
