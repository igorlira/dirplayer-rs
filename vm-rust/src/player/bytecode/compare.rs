use num::ToPrimitive;

use crate::{
    director::lingo::datum::{datum_bool, Datum},
    player::{
        compare::{datum_equals, datum_greater_than, datum_less_than},
        reserve_player_mut, scope::StackDatum, HandlerExecutionResult, ScriptError,
    },
};

use super::handler_manager::BytecodeHandlerContext;

pub struct CompareBytecodeHandler {}

impl CompareBytecodeHandler {
    pub fn gt(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (lv, rv) = Self::pop2(player, ctx, "gt")?;
            if let (StackDatum::Int(a), StackDatum::Int(b)) = (&lv, &rv) {
                let r = (*a > *b) as i32;
                player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push_int(r);
                return Ok(HandlerExecutionResult::Advance);
            }
            let right = rv.into_ref();
            let left = lv.into_ref();
            let is_gt = datum_greater_than(player.get_datum(&left), player.get_datum(&right), &player.allocator)?;
            let result_id = player.alloc_datum(datum_bool(is_gt));
            player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn lt(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (lv, rv) = Self::pop2(player, ctx, "lt")?;
            if let (StackDatum::Int(a), StackDatum::Int(b)) = (&lv, &rv) {
                let r = (*a < *b) as i32;
                player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push_int(r);
                return Ok(HandlerExecutionResult::Advance);
            }
            let right = rv.into_ref();
            let left = lv.into_ref();
            let is_lt = datum_less_than(player.get_datum(&left), player.get_datum(&right), &player.allocator)?;
            let result_id = player.alloc_datum(datum_bool(is_lt));
            player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn lt_eq(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (lv, rv) = Self::pop2(player, ctx, "lt_eq")?;
            if let (StackDatum::Int(a), StackDatum::Int(b)) = (&lv, &rv) {
                let r = (*a <= *b) as i32;
                player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push_int(r);
                return Ok(HandlerExecutionResult::Advance);
            }
            let right = rv.into_ref();
            let left = lv.into_ref();
            let l = player.get_datum(&left);
            let rgt = player.get_datum(&right);
            let is_lt = datum_less_than(l, rgt, &player.allocator)?;
            let is_eq = datum_equals(l, rgt, &player.allocator)?;
            let result_id = player.alloc_datum(datum_bool(is_lt || is_eq));
            player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn gt_eq(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (lv, rv) = Self::pop2(player, ctx, "gt_eq")?;
            if let (StackDatum::Int(a), StackDatum::Int(b)) = (&lv, &rv) {
                let r = (*a >= *b) as i32;
                player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push_int(r);
                return Ok(HandlerExecutionResult::Advance);
            }
            let right = rv.into_ref();
            let left = lv.into_ref();
            let l = player.get_datum(&left);
            let rgt = player.get_datum(&right);
            let is_gt = datum_greater_than(l, rgt, &player.allocator)?;
            let is_eq = datum_equals(l, rgt, &player.allocator)?;
            let result_id = player.alloc_datum(datum_bool(is_gt || is_eq));
            player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    /// Pop two operands as raw `StackDatum`s (left, right) without
    /// materializing — the int/int fast path in each comparison reads them
    /// directly. `op` names the opcode for the underflow message.
    #[inline]
    fn pop2(
        player: &mut crate::player::DirPlayer,
        ctx: &BytecodeHandlerContext,
        op: &str,
    ) -> Result<(StackDatum, StackDatum), ScriptError> {
        let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
        let right = scope
            .stack
            .pop_value()
            .ok_or_else(|| ScriptError::new(format!("{}: stack underflow (right)", op)))?;
        let left = scope
            .stack
            .pop_value()
            .ok_or_else(|| ScriptError::new(format!("{}: stack underflow (left)", op)))?;
        Ok((left, right))
    }

    pub fn not(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let obj_id = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.pop().ok_or_else(|| ScriptError::new("not: stack underflow".to_string()))?
            };
            let obj = player.get_datum(&obj_id);
            let is_not = match obj {
                Datum::Void => true,
                Datum::Int(num) => *num == 0,
                Datum::Float(num) => num.to_u64().unwrap() == 0,
                _ => false,
            };
            let result_id = player.alloc_datum(datum_bool(is_not));
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn nt_eq(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (lv, rv) = Self::pop2(player, ctx, "nt_eq")?;
            if let (StackDatum::Int(a), StackDatum::Int(b)) = (&lv, &rv) {
                let r = (*a != *b) as i32;
                player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push_int(r);
                return Ok(HandlerExecutionResult::Advance);
            }
            let right = rv.into_ref();
            let left = lv.into_ref();
            let is_eq = datum_equals(player.get_datum(&left), player.get_datum(&right), &player.allocator)?;
            let result_id = player.alloc_datum(datum_bool(!is_eq));
            player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn and(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (left, right) = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                let right = scope.stack.pop().ok_or_else(|| ScriptError::new("and: stack underflow (right)".to_string()))?;
                let left = scope.stack.pop().ok_or_else(|| ScriptError::new("and: stack underflow (left)".to_string()))?;
                (left, right)
            };
            let right = player.get_datum(&right);
            let left = player.get_datum(&left);

            let is_and = left.to_bool()? && right.to_bool()?;

            let result_id = player.alloc_datum(datum_bool(is_and));

            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn or(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (left, right) = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                let right = scope.stack.pop().ok_or_else(|| ScriptError::new("or: stack underflow (right)".to_string()))?;
                let left = scope.stack.pop().ok_or_else(|| ScriptError::new("or: stack underflow (left)".to_string()))?;
                (left, right)
            };
            let right = player.get_datum(&right);
            let left = player.get_datum(&left);

            let is_or = left.to_bool()? || right.to_bool()?;

            let result_id = player.alloc_datum(datum_bool(is_or));
            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }

    pub fn eq(ctx: &BytecodeHandlerContext) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            let (lv, rv) = Self::pop2(player, ctx, "eq")?;
            if let (StackDatum::Int(a), StackDatum::Int(b)) = (&lv, &rv) {
                let r = (*a == *b) as i32;
                player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push_int(r);
                return Ok(HandlerExecutionResult::Advance);
            }
            let right = rv.into_ref();
            let left = lv.into_ref();
            let is_eq = datum_equals(player.get_datum(&left), player.get_datum(&right), &player.allocator)?;
            let result_id = player.alloc_datum(datum_bool(is_eq));
            player.scopes.get_mut(ctx.scope_ref).unwrap().stack.push(result_id);
            Ok(HandlerExecutionResult::Advance)
        })
    }
}
