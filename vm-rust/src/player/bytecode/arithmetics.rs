use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{
        datum_formatting::format_datum,
        datum_operations::{add_datums, divide_datums, multiply_datums, subtract_datums},
        reserve_player_mut, HandlerExecutionResult, ScriptError,
    },
};

use super::handler_manager::BytecodeHandlerContext;

pub struct ArithmeticsBytecodeHandler {}

impl ArithmeticsBytecodeHandler {
    pub fn add(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (left, right) = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                let right = scope.stack.pop().unwrap();
                let left = scope.stack.pop().unwrap();
                (left, right)
            };
            let right = player.get_datum(&right);
            let left = player.get_datum(&left);

            let result_id = {
                let result = add_datums(left.to_owned(), right.to_owned(), player)?;
                player.alloc_datum(result)
            };
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn sub(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (left, right) = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                let right = scope.stack.pop().unwrap();
                let left = scope.stack.pop().unwrap();
                (left, right)
            };
            let right = player.get_datum(&right);
            let left = player.get_datum(&left);

            let result = subtract_datums(left.to_owned(), right.to_owned(), player)?;
            let result_id = player.alloc_datum(result);
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_id);
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
                    let mut ref_list = vec![];
                    for item in new_list {
                        ref_list.push(player.alloc_datum(item));
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
                    let mut ref_list = vec![];
                    for item in new_list {
                        ref_list.push(player.alloc_datum(item));
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
            let (left, right) = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                let right = scope.stack.pop().unwrap();
                let left = scope.stack.pop().unwrap();
                (left, right)
            };
            let result = divide_datums(left, right, player)?;
            let result_id = player.alloc_datum(result);
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn mul(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (left_ref, right_ref) = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                let right = scope.stack.pop().unwrap();
                let left = scope.stack.pop().unwrap();
                (left, right)
            };
            let result = multiply_datums(left_ref, right_ref, player)?;
            let result_id = player.alloc_datum(result);
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_id);
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
                Datum::Point(arr) => {
                    let x_val = player.get_datum(&arr[0]).clone();
                    let y_val = player.get_datum(&arr[1]).clone();

                    let x_ref = match x_val {
                        Datum::Int(n) => player.alloc_datum(Datum::Int(-n)),
                        Datum::Float(n) => player.alloc_datum(Datum::Float(-n)),
                        _ => return Err(ScriptError::new(
                            "Point component must be Int or Float".to_string(),
                        )),
                    };

                    let y_ref = match y_val {
                        Datum::Int(n) => player.alloc_datum(Datum::Int(-n)),
                        Datum::Float(n) => player.alloc_datum(Datum::Float(-n)),
                        _ => return Err(ScriptError::new(
                            "Point component must be Int or Float".to_string(),
                        )),
                    };
                    Datum::Point([x_ref, y_ref])
                }
                Datum::List(list_type, items, sorted) => {
                    let mut negated_items = Vec::with_capacity(items.len());
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
                        negated_items.push(negated);
                    }
                    Datum::List(list_type, negated_items, sorted)
                }
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
