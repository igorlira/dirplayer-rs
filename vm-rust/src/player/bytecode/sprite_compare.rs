use log::{debug, warn};

use crate::{
    director::lingo::datum::Datum,
    player::{
        reserve_player_mut,
        score::sprite_get_prop,
        HandlerExecutionResult, ScriptError,
        datum_formatting::format_concrete_datum,
    },
};

use wasm_bindgen::JsValue;

use super::handler_manager::BytecodeHandlerContext;

pub struct SpriteCompareBytecodeHandler {}

impl SpriteCompareBytecodeHandler {
    /// ontospr - Check if one sprite intersects with another sprite
    /// Pops two values from stack:
    /// - First pop: target sprite (from sprite() call or sprite number)
    /// - Second pop: source sprite number
    /// Pushes 1 if sprites intersect, 0 if they don't
    pub fn onto_sprite(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            // Pop the target sprite (result from sprite() call)
            let target_sprite_ref = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.pop().unwrap()
            };
            
            // Pop the source sprite number
            let source_sprite_ref = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.pop().unwrap()
            };

            // Get sprite numbers - handle both sprite refs and plain integers
            let get_sprite_num = |datum_ref: &crate::player::DatumRef| -> Result<i16, ScriptError> {
                let datum = player.get_datum(datum_ref);
                
                // Try to_sprite_ref first (proper sprite reference)
                if let Ok(num) = datum.to_sprite_ref() {
                    return Ok(num);
                }
                
                // Fall back to int_value for plain integers
                if let Ok(num) = datum.int_value() {
                    return Ok(num as i16);
                }
                
                Err(ScriptError::new(format!(
                    "Expected sprite reference or integer, got {}",
                    format_concrete_datum(datum, player)
                )))
            };

            let source_sprite_num = get_sprite_num(&source_sprite_ref)?;
            let target_sprite_num = get_sprite_num(&target_sprite_ref)?;

            debug!("ontospr: Comparing sprite {} with sprite {}", source_sprite_num, target_sprite_num);
            debug!("  source_sprite_ref datum: {}", format_concrete_datum(&player.get_datum(&source_sprite_ref), player));
            debug!("  target_sprite_ref datum: {}", format_concrete_datum(&player.get_datum(&target_sprite_ref), player));

            // Helper function to get rect bounds
            let mut get_rect_bounds = |sprite_num: i16| -> Result<(i32, i32, i32, i32), ScriptError> {
                let rect_datum = sprite_get_prop(player, sprite_num, "rect")?;
                
                debug!("  sprite {} rect datum: {}", sprite_num, format_concrete_datum(&rect_datum, player));
                
                // Extract rect coordinates - rect is stored as Datum::Rect([left, top, right, bottom])
                match rect_datum {
                    Datum::Rect(coords) => {
                        // Rect is an array of 4 DatumRefs: [left, top, right, bottom]
                        let left = player.get_datum(&coords[0]).int_value()?;
                        let top = player.get_datum(&coords[1]).int_value()?;
                        let right = player.get_datum(&coords[2]).int_value()?;
                        let bottom = player.get_datum(&coords[3]).int_value()?;
                        debug!("  sprite {} rect: [{}, {}, {}, {}]", sprite_num, left, top, right, bottom);
                        Ok((left, top, right, bottom))
                    }
                    Datum::List(_, coords, _) => {
                        // Also support list format [left, top, right, bottom] just in case
                        if coords.len() != 4 {
                            return Err(ScriptError::new(format!(
                                "Sprite {} rect has invalid format (length {})",
                                sprite_num, coords.len()
                            )));
                        }
                        let left = player.get_datum(&coords[0]).int_value()?;
                        let top = player.get_datum(&coords[1]).int_value()?;
                        let right = player.get_datum(&coords[2]).int_value()?;
                        let bottom = player.get_datum(&coords[3]).int_value()?;
                        debug!("  sprite {} rect: [{}, {}, {}, {}]", sprite_num, left, top, right, bottom);
                        Ok((left, top, right, bottom))
                    }
                    _ => {
                        Err(ScriptError::new(format!(
                            "Sprite {} rect is not a rect or list: {}",
                            sprite_num, format_concrete_datum(&rect_datum, player)
                        )))
                    }
                }
            };

            // Get rectangles for both sprites
            let source_rect = match get_rect_bounds(source_sprite_num) {
                Ok(rect) => rect,
                Err(e) => {
                    warn!("WARNING: Failed to get rect for source sprite {}: {:?}", source_sprite_num, e);
                    // Sprite doesn't exist or has no rect, return 0 (no collision)
                    let result_ref = player.alloc_datum(Datum::Int(0));
                    let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                    scope.stack.push(result_ref);
                    return Ok(HandlerExecutionResult::Advance);
                }
            };

            let target_rect = match get_rect_bounds(target_sprite_num) {
                Ok(rect) => rect,
                Err(e) => {
                    warn!("WARNING: Failed to get rect for target sprite {}: {:?}", target_sprite_num, e);
                    // Sprite doesn't exist or has no rect, return 0 (no collision)
                    let result_ref = player.alloc_datum(Datum::Int(0));
                    let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                    scope.stack.push(result_ref);
                    return Ok(HandlerExecutionResult::Advance);
                }
            };

            // Check if rectangles intersect
            // Rectangles DON'T intersect if one is completely to the side of the other
            let (src_left, src_top, src_right, src_bottom) = source_rect;
            let (tgt_left, tgt_top, tgt_right, tgt_bottom) = target_rect;
            
            let intersects = !(
                src_right <= tgt_left ||   // source is completely to the left
                src_left >= tgt_right ||   // source is completely to the right
                src_bottom <= tgt_top ||   // source is completely above
                src_top >= tgt_bottom      // source is completely below
            );

            // Debug logging
            debug!("ontospr: sprite {} [{},{},{},{}] vs sprite {} [{},{},{},{}] => {}",
                source_sprite_num, src_left, src_top, src_right, src_bottom,
                target_sprite_num, tgt_left, tgt_top, tgt_right, tgt_bottom,
                if intersects { "INTERSECT" } else { "no collision" }
            );

            // Push result (1 for true, 0 for false)
            let result = if intersects { 1 } else { 0 };
            let result_ref = player.alloc_datum(Datum::Int(result));

            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_ref);

            Ok(HandlerExecutionResult::Advance)
        })
    }

    /// intospr - Check if one sprite is completely within another sprite
    /// Pops two values from stack:
    /// - First pop: target sprite (the container)
    /// - Second pop: source sprite number (the sprite to check if within)
    /// Pushes 1 if source is completely within target, 0 otherwise
    pub fn into_sprite(
        ctx: &BytecodeHandlerContext,
    ) -> Result<HandlerExecutionResult, ScriptError> {
        reserve_player_mut(|player| {
            // Pop the target sprite (the container)
            let target_sprite_ref = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.pop().unwrap()
            };

            // Pop the source sprite number (the one to check if within)
            let source_sprite_ref = {
                let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                scope.stack.pop().unwrap()
            };

            // Get sprite numbers - handle both sprite refs and plain integers
            let get_sprite_num = |datum_ref: &crate::player::DatumRef| -> Result<i16, ScriptError> {
                let datum = player.get_datum(datum_ref);

                // Try to_sprite_ref first (proper sprite reference)
                if let Ok(num) = datum.to_sprite_ref() {
                    return Ok(num);
                }

                // Fall back to int_value for plain integers
                if let Ok(num) = datum.int_value() {
                    return Ok(num as i16);
                }

                Err(ScriptError::new(format!(
                    "Expected sprite reference or integer, got {}",
                    format_concrete_datum(datum, player)
                )))
            };

            let source_sprite_num = get_sprite_num(&source_sprite_ref)?;
            let target_sprite_num = get_sprite_num(&target_sprite_ref)?;

            debug!("intospr: Checking if sprite {} is within sprite {}", source_sprite_num, target_sprite_num);

            // Helper function to get rect bounds
            let mut get_rect_bounds = |sprite_num: i16| -> Result<(i32, i32, i32, i32), ScriptError> {
                let rect_datum = sprite_get_prop(player, sprite_num, "rect")?;

                match rect_datum {
                    Datum::Rect(coords) => {
                        let left = player.get_datum(&coords[0]).int_value()?;
                        let top = player.get_datum(&coords[1]).int_value()?;
                        let right = player.get_datum(&coords[2]).int_value()?;
                        let bottom = player.get_datum(&coords[3]).int_value()?;
                        Ok((left, top, right, bottom))
                    }
                    Datum::List(_, coords, _) => {
                        if coords.len() != 4 {
                            return Err(ScriptError::new(format!(
                                "Sprite {} rect has invalid format (length {})",
                                sprite_num, coords.len()
                            )));
                        }
                        let left = player.get_datum(&coords[0]).int_value()?;
                        let top = player.get_datum(&coords[1]).int_value()?;
                        let right = player.get_datum(&coords[2]).int_value()?;
                        let bottom = player.get_datum(&coords[3]).int_value()?;
                        Ok((left, top, right, bottom))
                    }
                    _ => {
                        Err(ScriptError::new(format!(
                            "Sprite {} rect is not a rect or list: {}",
                            sprite_num, format_concrete_datum(&rect_datum, player)
                        )))
                    }
                }
            };

            // Get rectangles for both sprites
            let source_rect = match get_rect_bounds(source_sprite_num) {
                Ok(rect) => rect,
                Err(e) => {
                    warn!("WARNING: Failed to get rect for source sprite {}: {:?}", source_sprite_num, e);
                    let result_ref = player.alloc_datum(Datum::Int(0));
                    let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                    scope.stack.push(result_ref);
                    return Ok(HandlerExecutionResult::Advance);
                }
            };

            let target_rect = match get_rect_bounds(target_sprite_num) {
                Ok(rect) => rect,
                Err(e) => {
                    warn!("WARNING: Failed to get rect for target sprite {}: {:?}", target_sprite_num, e);
                    let result_ref = player.alloc_datum(Datum::Int(0));
                    let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
                    scope.stack.push(result_ref);
                    return Ok(HandlerExecutionResult::Advance);
                }
            };

            // Check if source is completely within target
            // Source is within target if all edges of source are inside target
            let (src_left, src_top, src_right, src_bottom) = source_rect;
            let (tgt_left, tgt_top, tgt_right, tgt_bottom) = target_rect;

            let is_within =
                src_left >= tgt_left &&
                src_top >= tgt_top &&
                src_right <= tgt_right &&
                src_bottom <= tgt_bottom;

            debug!("intospr: sprite {} [{},{},{},{}] within sprite {} [{},{},{},{}] => {}",
                source_sprite_num, src_left, src_top, src_right, src_bottom,
                target_sprite_num, tgt_left, tgt_top, tgt_right, tgt_bottom,
                if is_within { "WITHIN" } else { "not within" }
            );

            // Push result (1 for true, 0 for false)
            let result = if is_within { 1 } else { 0 };
            let result_ref = player.alloc_datum(Datum::Int(result));

            let scope = player.scopes.get_mut(ctx.scope_ref).unwrap();
            scope.stack.push(result_ref);

            Ok(HandlerExecutionResult::Advance)
        })
    }
}
