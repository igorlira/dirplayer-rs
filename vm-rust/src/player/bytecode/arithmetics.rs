use std::collections::VecDeque;
use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{
        datum_formatting::format_datum,
        datum_operations::{add_datums, divide_datums, multiply_datums, subtract_datums},
        reserve_player_mut, DatumRef, DirPlayer, HandlerExecutionResult, ScriptError,
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

    /// Director's `mod` is integer-only: "performs the arithmetic modulus
    /// operation on two integer expressions… The resulting value of the entire
    /// expression is the integer remainder of the division" (11.5 Scripting
    /// Dictionary, `mod`). A float operand is coerced the same way `integer()`
    /// coerces one — by ROUNDING to the nearest whole number, not truncating.
    ///
    /// Returning a float remainder instead silently broke Heatwave Racing's
    /// terrain lookup. It picks the heightmap cell with
    /// `gx = integer(integer(pX) + tracksizex/2) / stepsizex + 1` (rounded) but
    /// the interpolation weight with
    /// `subx = (pX + tracksizex/2) mod stepsizex / float(stepsizey)`. At
    /// pX = 2789.8 the rounded index lands on the NEXT cell while an unrounded
    /// remainder still reports 99.8% across the previous one, so the sample
    /// jumped a whole cell ahead: ground 126.8 instead of 114.6 (the mesh is at
    /// 114.7). That 12-unit step fed `zm = min(8, z - lz)` and launched the car
    /// into the air. Rounding both makes the two agree.
    fn to_mod_int(value: f64) -> i32 {
        value.round() as i32
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
            let result = Self::modulo_datums(&left, &right, player)?;
            let result_id = player.alloc_datum(result);
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    /// `left mod right`, shared by the bytecode handler and the message-window
    /// evaluator so both report the same thing.
    pub fn modulo_datums(
        left: &DatumRef,
        right: &DatumRef,
        player: &mut DirPlayer,
    ) -> Result<Datum, ScriptError> {
        {
            let right = player.get_datum(right);
            let left = player.get_datum(left);

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
                    Datum::Int(Self::safe_mod_int(*left, Self::to_mod_int(*right)))
                }
                (Datum::Float(left), Datum::Int(right)) => {
                    Datum::Int(Self::safe_mod_int(Self::to_mod_int(*left), *right))
                }
                (Datum::Float(left), Datum::Float(right)) => {
                    Datum::Int(Self::safe_mod_int(
                        Self::to_mod_int(*left),
                        Self::to_mod_int(*right),
                    ))
                }
                (Datum::List(_, list, _), Datum::Float(right)) => {
                    let mut new_list = vec![];
                    for item in list {
                        let item_datum = player.get_datum(item);
                        let result_datum = match item_datum {
              Datum::Int(n) => Datum::Int(Self::safe_mod_int(*n, Self::to_mod_int(*right))),
              Datum::Float(n) => Datum::Int(Self::safe_mod_int(Self::to_mod_int(*n), Self::to_mod_int(*right))),
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
              Datum::Float(n) => Datum::Int(Self::safe_mod_int(Self::to_mod_int(*n), *right)),
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
            Ok(result)
        }
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
