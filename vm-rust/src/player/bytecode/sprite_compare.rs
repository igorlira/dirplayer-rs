use std::sync::Arc;

use log::{debug, warn};

use crate::{
    director::lingo::datum::Datum,
    player::{
        bitmap::manager::BitmapRef,
        bitmap::mask::BitmapMask,
        reserve_player_mut,
        score::sprite_get_prop,
        DirPlayer,
        HandlerExecutionResult, ScriptError,
        datum_formatting::format_concrete_datum,
    },
};


use super::handler_manager::BytecodeHandlerContext;

/// Check if an ink value requires matte (pixel-level) collision
fn is_matte_ink(ink: i32) -> bool {
    ink == 8 || ink == 36 || ink == 33 || ink == 41 || ink == 7
}

/// Get the bitmap image_ref for a sprite's cast member (if it's a bitmap).
/// Returns (image_ref, bitmap_width, bitmap_height).
fn get_sprite_image_ref(player: &DirPlayer, sprite_num: i16) -> Option<(BitmapRef, u16, u16)> {
    let member_ref = player.movie.score.get_channel(sprite_num)
        .sprite.member.as_ref()?.clone();
    let member = player.movie.cast_manager.find_member_by_ref(&member_ref)?;
    let bmp = member.member_type.as_bitmap()?;
    let bitmap = player.bitmap_manager.get_bitmap(bmp.image_ref)?;
    Some((bmp.image_ref, bitmap.width, bitmap.height))
}

/// Check pixel-level collision between two sprites using their bitmap mattes.
/// Only called when AABB already overlaps and at least one sprite has matte ink.
fn check_matte_pixel_overlap(
    player: &DirPlayer,
    src_num: i16,
    tgt_num: i16,
    src_rect: (i32, i32, i32, i32),
    tgt_rect: (i32, i32, i32, i32),
    src_is_matte: bool,
    tgt_is_matte: bool,
) -> bool {
    // Compute overlap region in stage coordinates
    let overlap_left = src_rect.0.max(tgt_rect.0);
    let overlap_top = src_rect.1.max(tgt_rect.1);
    let overlap_right = src_rect.2.min(tgt_rect.2);
    let overlap_bottom = src_rect.3.min(tgt_rect.3);

    if overlap_left >= overlap_right || overlap_top >= overlap_bottom {
        return false;
    }

    // Helper: get the matte Arc for a sprite's bitmap
    let get_matte = |sprite_num: i16| -> Option<(Arc<BitmapMask>, u16, u16)> {
        let member_ref = player.movie.score.get_channel(sprite_num)
            .sprite.member.as_ref()?.clone();
        let member = player.movie.cast_manager.find_member_by_ref(&member_ref)?;
        let bmp = member.member_type.as_bitmap()?;
        let bitmap = player.bitmap_manager.get_bitmap(bmp.image_ref)?;
        let matte = bitmap.matte.as_ref()?.clone();
        Some((matte, bitmap.width, bitmap.height))
    };

    // Get matte data for matte-ink sprites
    let src_matte = if src_is_matte { get_matte(src_num) } else { None };
    let tgt_matte = if tgt_is_matte { get_matte(tgt_num) } else { None };

    // If we need matte data but it's not available (not yet rendered), fall back to AABB
    if (src_is_matte && src_matte.is_none()) || (tgt_is_matte && tgt_matte.is_none()) {
        return true;
    }

    let src_rect_w = (src_rect.2 - src_rect.0).max(1);
    let src_rect_h = (src_rect.3 - src_rect.1).max(1);
    let tgt_rect_w = (tgt_rect.2 - tgt_rect.0).max(1);
    let tgt_rect_h = (tgt_rect.3 - tgt_rect.1).max(1);

    // Check pixel overlap in the overlap region
    for stage_y in overlap_top..overlap_bottom {
        for stage_x in overlap_left..overlap_right {
            // Check source pixel opacity
            let src_opaque = if let Some((ref matte, bw, bh, ..)) = src_matte {
                let bx = ((stage_x - src_rect.0) as u32 * bw as u32 / src_rect_w as u32) as u16;
                let by = ((stage_y - src_rect.1) as u32 * bh as u32 / src_rect_h as u32) as u16;
                matte.get_bit(bx, by)
            } else {
                true // Non-matte sprite: all pixels in bounding box are opaque
            };

            if !src_opaque { continue; }

            // Check target pixel opacity
            let tgt_opaque = if let Some((ref matte, bw, bh, ..)) = tgt_matte {
                let bx = ((stage_x - tgt_rect.0) as u32 * bw as u32 / tgt_rect_w as u32) as u16;
                let by = ((stage_y - tgt_rect.1) as u32 * bh as u32 / tgt_rect_h as u32) as u16;
                matte.get_bit(bx, by)
            } else {
                true
            };

            if src_opaque && tgt_opaque {
                return true; // Found overlapping opaque pixels
            }
        }
    }

    false // No overlapping opaque pixels
}

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

            // Check if rectangles intersect (AABB test)
            let (src_left, src_top, src_right, src_bottom) = source_rect;
            let (tgt_left, tgt_top, tgt_right, tgt_bottom) = target_rect;

            let mut intersects = !(
                src_right <= tgt_left ||   // source is completely to the left
                src_left >= tgt_right ||   // source is completely to the right
                src_bottom <= tgt_top ||   // source is completely above
                src_top >= tgt_bottom      // source is completely below
            );

            // Matte ink pixel-level collision check
            // In Director, sprites with matte ink use actual pixel overlap rather
            // than just bounding box intersection.
            if intersects {
                let src_ink = player.movie.score.get_channel(source_sprite_num).sprite.ink;
                let tgt_ink = player.movie.score.get_channel(target_sprite_num).sprite.ink;

                if is_matte_ink(src_ink) || is_matte_ink(tgt_ink) {
                    // Ensure mattes are computed for the matte-ink sprites
                    if is_matte_ink(src_ink) {
                        if let Some((image_ref, _, _)) = get_sprite_image_ref(player, source_sprite_num) {
                            let palettes = player.movie.cast_manager.palettes();
                            if let Some(bmp) = player.bitmap_manager.get_bitmap_mut(image_ref) {
                                if bmp.matte.is_none() {
                                    bmp.create_matte(&palettes);
                                }
                            }
                        }
                    }
                    if is_matte_ink(tgt_ink) {
                        if let Some((image_ref, _, _)) = get_sprite_image_ref(player, target_sprite_num) {
                            let palettes = player.movie.cast_manager.palettes();
                            if let Some(bmp) = player.bitmap_manager.get_bitmap_mut(image_ref) {
                                if bmp.matte.is_none() {
                                    bmp.create_matte(&palettes);
                                }
                            }
                        }
                    }

                    intersects = check_matte_pixel_overlap(
                        player,
                        source_sprite_num,
                        target_sprite_num,
                        source_rect,
                        target_rect,
                        is_matte_ink(src_ink),
                        is_matte_ink(tgt_ink),
                    );
                }
            }

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
