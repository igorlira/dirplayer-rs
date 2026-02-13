use std::{
    borrow::{Borrow, BorrowMut},
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
};

use async_std::task::spawn_local;
use chrono::Local;
use itertools::Itertools;
use log::debug;
use wasm_bindgen::{prelude::*, Clamped};
use web_sys::console;

use crate::js_api::safe_js_string;
use crate::{
    console_warn,
    js_api::JsApi,
    player::{
        bitmap::{
            bitmap::{self, get_system_default_palette, resolve_color_ref, Bitmap, PaletteRef},
            drawing::{should_matte_sprite, CopyPixelsParams},
            manager::BitmapManager,
            mask::BitmapMask,
            palette_map::PaletteMap,
        },
        cast_lib::CastMemberRef,
        cast_member::CastMemberType,
        geometry::IntRect,
        movie::Movie,
        reserve_player_ref,
        score::{
            get_concrete_sprite_rect, get_score, get_score_sprite, get_sprite_at, Score, ScoreRef,
        },
        sprite::{ColorRef, CursorRef, Sprite},
        DirPlayer, PLAYER_OPT,
    },
    utils::log_i,
};

use crate::player::cast_manager::CastManager;
use crate::player::font::BitmapFont;
use crate::player::font::FontManager;
use crate::player::handlers::datum_handlers::cast_member::font::{FontMemberHandlers, TextAlignment, StyledSpan, HtmlStyle};
use crate::player::score_keyframes::SpritePathKeyframes;
use crate::rendering_gpu::{DynamicRenderer, Renderer};

/// Interpolate path position between keyframes for filmloop animation.
/// Returns interpolated (x, y) position for the given frame, or None if no interpolation is needed.
fn interpolate_path_position(path_keyframes: &SpritePathKeyframes, frame: u32) -> Option<(i32, i32)> {
    let keyframes = &path_keyframes.keyframes;
    if keyframes.is_empty() {
        return None;
    }

    // Find the keyframe pair surrounding the current frame
    let prev_kf = keyframes.iter().rev().find(|kf| kf.frame <= frame);
    let next_kf = keyframes.iter().find(|kf| kf.frame > frame);

    match (prev_kf, next_kf) {
        (Some(prev), Some(next)) => {
            // Interpolate between prev and next keyframes
            let frame_range = next.frame - prev.frame;
            if frame_range == 0 {
                return Some((prev.x as i32, prev.y as i32));
            }
            let t = (frame - prev.frame) as f32 / frame_range as f32;
            let x = prev.x as f32 + (next.x as f32 - prev.x as f32) * t;
            let y = prev.y as f32 + (next.y as f32 - prev.y as f32) * t;
            Some((x as i32, y as i32))
        }
        (Some(prev), None) => {
            // Past the last keyframe - use last keyframe position
            Some((prev.x as i32, prev.y as i32))
        }
        (None, Some(_next)) => {
            // Before the first keyframe - return None to use channel_initialization_data position
            None
        }
        (None, None) => None,
    }
}

pub struct PlayerCanvasRenderer {
    pub container_element: Option<web_sys::HtmlElement>,
    pub preview_container_element: Option<web_sys::HtmlElement>,
    pub canvas: web_sys::HtmlCanvasElement,
    pub ctx2d: web_sys::CanvasRenderingContext2d,
    pub preview_canvas: web_sys::HtmlCanvasElement,
    pub preview_ctx2d: web_sys::CanvasRenderingContext2d,
    pub size: (u32, u32),
    pub preview_size: (u32, u32),
    pub preview_member_ref: Option<CastMemberRef>,
    pub debug_selected_channel_num: Option<i16>,
    pub bitmap: Bitmap,
}

fn get_or_load_font(
    font_manager: &mut FontManager,
    cast_manager: &CastManager,
    font_name: &str,
    font_size: Option<u16>,
    font_style: Option<u8>,
) -> Option<Rc<BitmapFont>> {
    return get_or_load_font_with_id(font_manager, cast_manager, font_name, font_size, font_style, None);
}

fn get_or_load_font_with_id(
    font_manager: &mut FontManager,
    cast_manager: &CastManager,
    font_name: &str,
    font_size: Option<u16>,
    font_style: Option<u8>,
    font_id: Option<u16>,
) -> Option<Rc<BitmapFont>> {
    if font_name.is_empty() || font_name == "System" {
        return font_manager.get_system_font();
    }

    let cache_key = format!(
        "{}_{}_{}",
        font_name,
        font_size.unwrap_or(0),
        font_style.unwrap_or(0)
    );

    if let Some(font) = font_manager.font_cache.get(&cache_key) {
        return Some(Rc::clone(font));
    }

    if let Some(font) = font_manager.font_cache.get(font_name) {
        return Some(Rc::clone(font));
    }

    // Try font_id-based lookup (for STXT formatting runs that reference fonts by ID)
    if let Some(id) = font_id {
        if let Some(font_ref) = font_manager.font_by_id.get(&id).copied() {
            if let Some(font) = font_manager.fonts.get(&font_ref) {
                web_sys::console::log_1(&format!(
                    "Font '{}' not found by name, but found by font_id={}", font_name, id
                ).into());
                return Some(Rc::clone(font));
            }
        }
    }

    if let Some(loaded_font) =
        font_manager.get_font_with_cast(font_name, Some(cast_manager), font_size, font_style)
    {
        return Some(loaded_font);
    }

    // Font not found by any method, attempt fallback to system font
    let system_font = font_manager.get_system_font();

    if system_font.is_some() {
        debug!(
            "Font '{}' (id={:?}) not found, using system font fallback. Available fonts: {:?}",
            font_name, font_id,
            font_manager.font_cache.keys().collect::<Vec<_>>()
        );
    } else {
        debug!(
            "Font '{}' (id={:?}) not found and system font unavailable. Available fonts: {:?}",
            font_name, font_id,
            font_manager.font_cache.keys().collect::<Vec<_>>()
        );
    }

    system_font
}

pub fn render_stage_to_bitmap(
    player: &mut DirPlayer,
    bitmap: &mut Bitmap,
    debug_sprite_num: Option<i16>,
) {
    let palettes = player.movie.cast_manager.palettes();
    render_score_to_bitmap(
        player,
        &ScoreRef::Stage,
        bitmap,
        debug_sprite_num,
        IntRect::from_size(0, 0, player.movie.rect.width(), player.movie.rect.height()),
    );
    draw_cursor(player, bitmap, &palettes);
}

pub fn render_score_to_bitmap(
    player: &mut DirPlayer,
    score_source: &ScoreRef,
    bitmap: &mut Bitmap,
    debug_sprite_num: Option<i16>,
    dest_rect: IntRect,
) {
    render_score_to_bitmap_with_offset(player, score_source, bitmap, debug_sprite_num, dest_rect, (0, 0), None);
}

/// Recompute the initial_rect for a filmloop using actual bitmap dimensions.
/// This is more accurate than the precomputed initial_rect because it uses
/// the real cast member dimensions instead of the channel_data dimensions.
fn compute_filmloop_initial_rect_with_members(
    player: &DirPlayer,
    member_ref: &CastMemberRef,
) -> Option<IntRect> {
    let member = player.movie.cast_manager.find_member_by_ref(member_ref)?;
    let film_loop = match &member.member_type {
        CastMemberType::FilmLoop(fl) => fl,
        _ => return None,
    };

    let filmloop_cast_lib = member_ref.cast_lib;
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    let mut found_any = false;

    for (_frame_idx, channel_idx, data) in film_loop.score.channel_initialization_data.iter() {
        // Skip effect channels (channels 0-5 in raw data)
        if *channel_idx < 6 {
            continue;
        }
        // Skip empty sprites
        if data.cast_member == 0 || data.cast_lib == 0 {
            continue;
        }

        // Resolve the cast member reference
        let sprite_cast_lib = if data.cast_lib == 65535 {
            filmloop_cast_lib as i32
        } else {
            data.cast_lib as i32
        };
        let sprite_member_ref = CastMemberRef {
            cast_lib: sprite_cast_lib,
            cast_member: data.cast_member as i32,
        };

        // Get actual bitmap dimensions and registration point from the cast member.
        // The registration point is the anchor used for positioning - sprite_left = pos_x - reg_x.
        let (actual_width, actual_height, reg_x, reg_y) = if let Some(sprite_member) =
            player.movie.cast_manager.find_member_by_ref(&sprite_member_ref)
        {
            match &sprite_member.member_type {
                CastMemberType::Bitmap(bm) => {
                    // Try to get actual bitmap dimensions from BitmapManager
                    let (w, h) = if let Some(bitmap) = player.bitmap_manager.get_bitmap(bm.image_ref) {
                        (bitmap.width as i32, bitmap.height as i32)
                    } else {
                        // Fall back to BitmapInfo dimensions
                        (bm.info.width as i32, bm.info.height as i32)
                    };
                    // Use the actual registration point from the bitmap
                    (w, h, bm.reg_point.0 as i32, bm.reg_point.1 as i32)
                }
                _ => (data.width as i32, data.height as i32, data.width as i32 / 2, data.height as i32 / 2),
            }
        } else {
            (data.width as i32, data.height as i32, data.width as i32 / 2, data.height as i32 / 2)
        };

        if actual_width == 0 && actual_height == 0 {
            continue;
        }

        // pos_x/pos_y is the loc (registration point position).
        // The sprite's top-left corner is: pos - reg_point
        let sprite_left = data.pos_x as i32 - reg_x;
        let sprite_top = data.pos_y as i32 - reg_y;
        let sprite_right = sprite_left + actual_width;
        let sprite_bottom = sprite_top + actual_height;

        debug!(
            "  compute_initial_rect: ch {} m {}:{} pos ({}, {}) size {}x{} reg ({}, {}) -> bounds ({}, {}, {}, {})",
            channel_idx, sprite_member_ref.cast_lib, sprite_member_ref.cast_member,
            data.pos_x, data.pos_y, actual_width, actual_height, reg_x, reg_y,
            sprite_left, sprite_top, sprite_right, sprite_bottom
        );

        if sprite_left < min_x {
            min_x = sprite_left;
        }
        if sprite_top < min_y {
            min_y = sprite_top;
        }
        if sprite_right > max_x {
            max_x = sprite_right;
        }
        if sprite_bottom > max_y {
            max_y = sprite_bottom;
        }
        found_any = true;
    }

    if !found_any {
        return None;
    }

    let result = IntRect::from(min_x, min_y, max_x, max_y);
    debug!(
        "compute_filmloop_initial_rect_with_members: FINAL rect ({}, {}, {}, {}) size {}x{}",
        result.left, result.top, result.right, result.bottom,
        result.width(), result.height()
    );
    Some(result)
}

/// Render a filmloop directly from its channel_initialization_data.
/// This is needed because filmloop Score.channels are not populated with sprite data
/// like the main stage score. Instead, we read sprite info directly from channel_initialization_data.
///
/// Director behavior: Film loop frames use the PARENT sprite's ink semantics,
/// not their own stored ink values. The parent_ink, parent_color, and parent_bg_color
/// are the properties of the sprite displaying the film loop on the stage.
fn render_filmloop_from_channel_data(
    player: &mut DirPlayer,
    member_ref: &CastMemberRef,
    bitmap: &mut Bitmap,
    dest_rect: IntRect,
    initial_rect: IntRect,
    parent_ink: u32,
    parent_color: ColorRef,
    parent_bg_color: ColorRef,
) {
    use crate::player::bitmap::drawing::CopyPixelsParams;
    use crate::player::score::get_channel_number_from_index;

    let palettes = player.movie.cast_manager.palettes();

    // The filmloop's own cast_lib is used as the default when channel data has cast_lib=65535
    let filmloop_cast_lib = member_ref.cast_lib;

    // Get filmloop data
    // Filmloops use keyframe-based animation - not every frame has explicit data.
    // We need to find the most recent keyframe data for each channel that is <= current_frame.
    // Also extract keyframes_cache for path interpolation.
    let (current_frame, channel_data, total_frames, keyframes_cache) = {
        let member = player.movie.cast_manager.find_member_by_ref(member_ref);
        if member.is_none() {
            return;
        }
        let member = member.unwrap();
        match &member.member_type {
            CastMemberType::FilmLoop(film_loop_member) => {
                let frame = film_loop_member.current_frame;
                // Frame numbers are 1-based, channel_initialization_data frame indices are 0-based
                let frame_idx_target = frame.saturating_sub(1);

                // Group data by channel, keeping only valid sprite channels
                let mut channel_map: std::collections::HashMap<u16, (u32, crate::director::chunks::score::ScoreFrameChannelData)> =
                    std::collections::HashMap::new();

                for (frame_idx, channel_idx, data) in film_loop_member.score.channel_initialization_data.iter() {
                    // Skip effect channels (0-5)
                    if *channel_idx < 6 {
                        continue;
                    }
                    // Skip empty sprites (cast_member 0 means no sprite)
                    if data.cast_member == 0 {
                        continue;
                    }
                    // Only consider frames <= current frame (keyframe interpolation)
                    if *frame_idx > frame_idx_target {
                        continue;
                    }
                    // Keep the most recent (highest frame_idx) data for each channel
                    let entry = channel_map.entry(*channel_idx);
                    entry
                        .and_modify(|(existing_frame, existing_data)| {
                            if *frame_idx > *existing_frame {
                                *existing_frame = *frame_idx;
                                *existing_data = data.clone();
                            }
                        })
                        .or_insert((*frame_idx, data.clone()));
                }

                let data: Vec<_> = channel_map
                    .into_iter()
                    .map(|(channel_idx, (frame_idx, data))| (frame_idx, channel_idx, data))
                    .collect();

                // Calculate total frames from multiple sources:
                // 1. channel_initialization_data max frame index
                let init_data_max = film_loop_member.score.channel_initialization_data
                    .iter()
                    .map(|(frame_idx, _, _)| *frame_idx + 1)
                    .max()
                    .unwrap_or(1);

                // 2. sprite_spans end frames
                let span_max = film_loop_member.score.sprite_spans
                    .iter()
                    .map(|span| span.end_frame)
                    .max()
                    .unwrap_or(1);

                // 3. path keyframes max frame
                let keyframes_max = film_loop_member.score.keyframes_cache.values()
                    .filter_map(|channel_kf| channel_kf.path.as_ref())
                    .flat_map(|path_kf| path_kf.keyframes.iter())
                    .map(|kf| kf.frame)
                    .max()
                    .unwrap_or(1);

                // Use the maximum of all sources
                let total_frames = init_data_max.max(span_max).max(keyframes_max);

                // Clone keyframes cache for path interpolation
                let keyframes_cache = film_loop_member.score.keyframes_cache.clone();

                (frame, data, total_frames, keyframes_cache)
            }
            _ => return,
        }
    };

    // Since our bitmap is sized to match initial_rect dimensions (1:1 scaling),
    // we just need to translate by subtracting initial_rect origin.
    // If we later support non-1:1 scaling, we'd use:
    //   rel_x = (pos_x - initial_rect.left) * dest_width / initial_rect.width()
    let scale_x = dest_rect.width() as f32 / initial_rect.width().max(1) as f32;
    let scale_y = dest_rect.height() as f32 / initial_rect.height().max(1) as f32;

    debug!(
        "render_filmloop_from_channel_data: frame {}/{}, {} sprites, initial_rect ({}, {}, {}, {}), scale ({:.2}, {:.2})",
        current_frame, total_frames, channel_data.len(),
        initial_rect.left, initial_rect.top, initial_rect.right, initial_rect.bottom,
        scale_x, scale_y
    );

    // Sort by channel number for consistent z-ordering
    let mut sorted_data = channel_data;
    sorted_data.sort_by_key(|(_, channel_idx, _)| *channel_idx);

    for (_frame_idx, channel_idx, data) in sorted_data {
        let channel_num = get_channel_number_from_index(channel_idx as u32);

        // Build member ref from channel data
        // cast_lib 65535 means "use the filmloop's cast library"
        let sprite_member_ref = CastMemberRef {
            cast_lib: if data.cast_lib == 65535 { filmloop_cast_lib } else { data.cast_lib as i32 },
            cast_member: data.cast_member as i32,
        };

        let member = player.movie.cast_manager.find_member_by_ref(&sprite_member_ref);
        if member.is_none() {
            web_sys::console::log_1(&format!(
                "  channel {}: member {}:{} not found",
                channel_num, sprite_member_ref.cast_lib, sprite_member_ref.cast_member
            ).into());
            continue;
        }
        let member = member.unwrap();

        // Translate sprite position relative to initial_rect origin, then scale to dest_rect
        //
        // For channels with path keyframes, interpolate between keyframes
        // Director frame numbers are 1-based, keyframes use 1-based Director frame numbers
        let (pos_x, pos_y) = if let Some(channel_keyframes) = keyframes_cache.get(&(channel_num as u16)) {
            if let Some(path_keyframes) = &channel_keyframes.path {
                // Debug: log path keyframes for this channel
                if current_frame <= 5 || current_frame >= 95 {
                    let kf_summary: Vec<String> = path_keyframes.keyframes.iter()
                        .take(5)
                        .map(|kf| format!("f{}:({},{})", kf.frame, kf.x, kf.y))
                        .collect();
                    let last_kf = path_keyframes.keyframes.last();
                    debug!(
                        "    PATH keyframes for channel {}: {} keyframes, first 5: {:?}, last: {:?}",
                        channel_num, path_keyframes.keyframes.len(), kf_summary, last_kf
                    );
                }

                // Try to get interpolated position from path keyframes
                if let Some((interp_x, interp_y)) = interpolate_path_position(path_keyframes, current_frame) {
                    (interp_x as i16, interp_y as i16)
                } else {
                    // Fall back to static keyframe position
                    (data.pos_x, data.pos_y)
                }
            } else {
                (data.pos_x, data.pos_y)
            }
        } else {
            (data.pos_x, data.pos_y)
        };

        // For bitmaps, use channel data dimensions if they're valid (non-zero),
        // otherwise fall back to member's actual dimensions.
        // Also get the actual registration point from the bitmap member.
        let (member_width, member_height, reg_x, reg_y) = match &member.member_type {
            CastMemberType::Bitmap(bm) => (
                bm.info.width as u16,
                bm.info.height as u16,
                bm.reg_point.0 as i32,
                bm.reg_point.1 as i32,
            ),
            _ => (data.width, data.height, data.width as i32 / 2, data.height as i32 / 2),
        };

        // Use channel data dimensions if valid, otherwise fall back to member dimensions
        let (use_width, use_height) = if data.width > 0 && data.height > 0 {
            (data.width, data.height)
        } else {
            (member_width, member_height)
        };

        // Coordinate transformation: translate sprite position relative to initial_rect origin.
        // The filmloop content is rendered at its NATURAL size (initial_rect dimensions),
        // not scaled to the sprite's rect.
        //
        // IMPORTANT: pos_x/pos_y are the sprite's loc (registration point position).
        // We need to subtract the registration point (the bitmap's anchor)
        // to get the sprite's top-left corner, then translate relative to initial_rect.
        // This matches how initial_rect was computed in compute_filmloop_initial_rect.
        let sprite_left = pos_x as i32 - reg_x;
        let sprite_top = pos_y as i32 - reg_y;
        let rel_x = sprite_left - initial_rect.left;
        let rel_y = sprite_top - initial_rect.top;

        // Keep sprite dimensions at natural size (no scaling)
        let rel_w = use_width as i32;
        let rel_h = use_height as i32;

        let sprite_rect = IntRect::from(
            rel_x,
            rel_y,
            rel_x + rel_w,
            rel_y + rel_h,
        );

        debug!(
            "  channel {}: member {}:{} type {:?} orig ({}, {}) interp ({}, {}) data_size {}x{} member_size {}x{} reg ({}, {}) sprite_left {} initial_left {} -> rect ({}, {}, {}, {})",
            channel_num, sprite_member_ref.cast_lib, sprite_member_ref.cast_member,
            member.member_type.member_type_id(),
            data.pos_x, data.pos_y,
            pos_x, pos_y,
            data.width, data.height,
            member_width, member_height,
            reg_x, reg_y,
            sprite_left, initial_rect.left,
            sprite_rect.left, sprite_rect.top, sprite_rect.right, sprite_rect.bottom
        );

        match &member.member_type {
            CastMemberType::Bitmap(bitmap_member) => {
                let sprite_bitmap = player
                    .bitmap_manager
                    .get_bitmap_mut(bitmap_member.image_ref);
                if sprite_bitmap.is_none() {
                    debug!(
                        "    Bitmap image_ref {} not found in bitmap_manager",
                        bitmap_member.image_ref
                    );
                    continue;
                }
                let src_bitmap = sprite_bitmap.unwrap();

                let src_rect = IntRect::from(0, 0, src_bitmap.width as i32, src_bitmap.height as i32);
                let dst_rect = sprite_rect;

                // Check if bitmap has actual data
                let has_data = !src_bitmap.data.is_empty();
                let first_pixels: Vec<u8> = src_bitmap.data.iter().take(16).copied().collect();

                debug!(
                    "    Bitmap found: {}x{} bit_depth={} image_ref={} ink={} blend={} has_data={} first_bytes={:?} src_rect=({},{},{},{}) dst_rect=({},{},{},{})",
                    src_bitmap.width, src_bitmap.height, src_bitmap.bit_depth, bitmap_member.image_ref,
                    data.ink, data.blend, has_data, first_pixels,
                    src_rect.left, src_rect.top, src_rect.right, src_rect.bottom,
                    dst_rect.left, dst_rect.top, dst_rect.right, dst_rect.bottom
                );

                // Director behavior: Film loop internal sprites use the PARENT sprite's
                // ink, color, and bgColor - not their own stored values.
                // This is because film loops are just sequences of cast members rendered
                // through the parent sprite's properties.
                let sprite_color = parent_color.clone();
                let sprite_bg_color = parent_bg_color.clone();
                let ink = parent_ink;

                // In Director, blend=0 means "default" which is fully opaque (100)
                // Only values 1-99 represent partial transparency
                // Note: blend is still read from channel data as it may vary per frame
                let blend = if data.blend == 0 { 100 } else { data.blend as i32 };

                // Only use matte mask for inks that support it:
                // - Ink 0 (copy): for trimWhiteSpace edge transparency (indexed and 16-bit)
                // - Ink 8 (matte): always uses matte (indexed only)
                // - Ink 7, 36 (color-key): do NOT use matte - they have their own
                //   bgColor-based transparency that conflicts with matte logic
                let is_indexed = src_bitmap.original_bit_depth <= 8;
                let is_16bit = src_bitmap.original_bit_depth == 16;
                let should_use_matte = (is_indexed && (ink == 0 || ink == 8))
                    || (is_16bit && ink == 0);

                let mask = if should_use_matte {
                    if src_bitmap.matte.is_none() {
                        src_bitmap.create_matte(&palettes);
                    }
                    src_bitmap.matte.as_ref()
                } else {
                    None
                };

                // Debug: resolve some representative palette indices to see what colors they would be
                let test_idx_0 = crate::player::bitmap::bitmap::resolve_color_ref(
                    &palettes,
                    &ColorRef::PaletteIndex(0),
                    &src_bitmap.palette_ref,
                    src_bitmap.original_bit_depth,
                );
                let test_idx_255 = crate::player::bitmap::bitmap::resolve_color_ref(
                    &palettes,
                    &ColorRef::PaletteIndex(255),
                    &src_bitmap.palette_ref,
                    src_bitmap.original_bit_depth,
                );
                let test_idx_128 = crate::player::bitmap::bitmap::resolve_color_ref(
                    &palettes,
                    &ColorRef::PaletteIndex(128),
                    &src_bitmap.palette_ref,
                    src_bitmap.original_bit_depth,
                );

                debug!(
                    "    Calling copy_pixels: dest_bitmap {}x{} bit_depth={} src_palette={:?} orig_bit_depth={} sprite_color={:?} sprite_bg_color={:?}",
                    bitmap.width, bitmap.height, bitmap.bit_depth,
                    src_bitmap.palette_ref, src_bitmap.original_bit_depth, sprite_color, sprite_bg_color
                );
                debug!(
                    "    Palette test: idx_0={:?} idx_128={:?} idx_255={:?}",
                    test_idx_0, test_idx_128, test_idx_255
                );

                // Debug: check matte mask status
                if let Some(m) = &mask {
                    // Count true/false bits in matte
                    let total_pixels = m.width as usize * m.height as usize;
                    let true_count = m.data.count_ones();
                    let false_count = total_pixels - true_count;
                    debug!(
                        "    Matte mask: {}x{} total={} true(opaque)={} false(transparent)={}",
                        m.width, m.height, total_pixels, true_count, false_count
                    );
                } else {
                    debug!(
                        "    Matte mask: None"
                    );
                }

                let params = CopyPixelsParams {
                    blend,
                    ink,
                    color: sprite_color,
                    bg_color: sprite_bg_color,
                    mask_image: mask.map(|m| m as &BitmapMask),
                    is_text_rendering: false,
                    rotation: 0.0,
                    skew: 0.0,
                    sprite: None,
                    original_dst_rect: Some(dst_rect.clone()),
                };

                bitmap.copy_pixels_with_params(
                    &palettes,
                    &src_bitmap,
                    dst_rect,
                    src_rect,
                    &params,
                );
            }
            CastMemberType::Shape(_shape_member) => {
                // Skip rendering shapes with tiny dimensions (blank placeholders or zero-size)
                // data.width/height from score data indicate the shape size
                // Skip if EITHER dimension is <= 1
                if data.width <= 1 || data.height <= 1 {
                    continue;
                }

                // Get sprite foreground color from channel data
                // Detect RGB mode by checking color_flag OR non-zero G/B components
                let fore_is_rgb = (data.color_flag & 0x1) != 0
                    || data.fore_color_g != 0
                    || data.fore_color_b != 0;

                let sprite_color = if fore_is_rgb {
                    ColorRef::Rgb(data.fore_color, data.fore_color_g, data.fore_color_b)
                } else {
                    ColorRef::PaletteIndex(data.fore_color)
                };

                let color = resolve_color_ref(
                    &palettes,
                    &sprite_color,
                    &PaletteRef::BuiltIn(get_system_default_palette()),
                    bitmap.original_bit_depth,
                );

                debug!(
                    "    Shape color: fore_is_rgb={} fore_color={} fore_g={} fore_b={} -> resolved ({}, {}, {})",
                    fore_is_rgb, data.fore_color, data.fore_color_g, data.fore_color_b,
                    color.0, color.1, color.2
                );

                bitmap.fill_rect(
                    sprite_rect.left,
                    sprite_rect.top,
                    sprite_rect.right,
                    sprite_rect.bottom,
                    color,
                    &palettes,
                    1.0, // Full alpha
                );
            }
            _ => {
                // Other member types not yet supported in filmloop rendering
                web_sys::console::log_1(&format!(
                    "  channel {}: unsupported member type {:?}",
                    channel_num, member.member_type.member_type_id()
                ).into());
            }
        }
    }
}

/// Parent sprite properties for film loop rendering.
/// In Director, film loop internal sprites use the parent sprite's ink semantics.
pub struct FilmLoopParentProps {
    pub ink: u32,
    pub color: ColorRef,
    pub bg_color: ColorRef,
}

/// Render a score to a bitmap with an optional coordinate offset.
/// The offset is used for filmloop rendering where sprite coordinates need to be
/// translated relative to the filmloop's initial_rect.
///
/// For film loop rendering, pass `parent_props` with the parent sprite's ink, color,
/// and bgColor. These will be applied to all internal sprites in the film loop.
pub fn render_score_to_bitmap_with_offset(
    player: &mut DirPlayer,
    score_source: &ScoreRef,
    bitmap: &mut Bitmap,
    debug_sprite_num: Option<i16>,
    dest_rect: IntRect,
    offset: (i32, i32),
    parent_props: Option<FilmLoopParentProps>,
) {
    let palettes = player.movie.cast_manager.palettes();

    // For filmloops, use transparent background so sprites composite correctly
    // onto the stage without a solid background color showing through.
    if let ScoreRef::FilmLoop(member_ref) = score_source {
        // Clear with transparent pixels for filmloop
        bitmap.clear_rect_transparent(
            dest_rect.left,
            dest_rect.top,
            dest_rect.right,
            dest_rect.bottom,
        );

        // The filmloop's rect is stored in info as:
        // - reg_point = (left, top) coordinates of the rect
        // - width = right coordinate
        // - height = bottom coordinate
        // This defines the coordinate space for internal sprite rendering.
        let initial_rect = {
            let member = player.movie.cast_manager.find_member_by_ref(member_ref);
            if let Some(member) = member {
                if let CastMemberType::FilmLoop(film_loop) = &member.member_type {
                    let rect_left = film_loop.info.reg_point.0 as i32;
                    let rect_top = film_loop.info.reg_point.1 as i32;
                    let rect_right = film_loop.info.width as i32;
                    let rect_bottom = film_loop.info.height as i32;
                    IntRect::from(rect_left, rect_top, rect_right, rect_bottom)
                } else {
                    IntRect::from(0, 0, 1, 1)
                }
            } else {
                IntRect::from(0, 0, 1, 1)
            }
        };

        // Get parent sprite properties - use defaults if not provided
        let props = parent_props.unwrap_or(FilmLoopParentProps {
            ink: 0, // Default to copy ink
            color: ColorRef::PaletteIndex(255), // Default foreground (black in most palettes)
            bg_color: ColorRef::PaletteIndex(0), // Default background (white in most palettes)
        });

        render_filmloop_from_channel_data(
            player,
            member_ref,
            bitmap,
            dest_rect,
            initial_rect,
            props.ink,
            props.color,
            props.bg_color,
        );
        return;
    }

    // For stage rendering, use the player's background color
    bitmap.clear_rect(
        dest_rect.left,
        dest_rect.top,
        dest_rect.right,
        dest_rect.bottom,
        resolve_color_ref(
            &palettes,
            &player.bg_color,
            &PaletteRef::BuiltIn(get_system_default_palette()),
            bitmap.original_bit_depth,
        ),
        &palettes,
    );

    let sorted_channel_numbers = {
        // Get the correct frame number for this score
        let frame_num = match score_source {
            ScoreRef::Stage => player.movie.current_frame,
            ScoreRef::FilmLoop(_) => unreachable!(), // Handled above
        };

        let score = match score_source {
            ScoreRef::Stage => &player.movie.score,
            ScoreRef::FilmLoop(_) => unreachable!(), // Handled above
        };
        score
            .get_sorted_channels(frame_num)
            .iter()
            .map(|x| x.number as i16)
            .collect_vec()
    };

    // Debug: log which channels are being rendered
    if matches!(score_source, ScoreRef::Stage) {
        debug!(
            "STAGE RENDER: frame {} channels {:?}",
            player.movie.current_frame, sorted_channel_numbers
        );
    }

    for channel_num in sorted_channel_numbers {
        let member_ref = {
            let score = get_score(&player.movie, score_source).unwrap();
            let sprite = score.get_sprite(channel_num);
            if sprite.is_none() {
                if matches!(score_source, ScoreRef::Stage) {
                    debug!(
                        "  STAGE channel {} SKIPPED: sprite is None",
                        channel_num
                    );
                }
                continue;
            }
            let sprite = sprite.unwrap();
            let member = sprite.member.as_ref();
            if member.is_none() {
                if matches!(score_source, ScoreRef::Stage) {
                    debug!(
                        "  STAGE channel {} SKIPPED: sprite.member is None",
                        channel_num
                    );
                }
                continue;
            }
            member.unwrap().clone()
        };
        let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
        if member.is_none() {
            if matches!(score_source, ScoreRef::Stage) {
                debug!(
                    "  STAGE channel {} SKIPPED: member {}:{} not found in cast_manager",
                    channel_num, member_ref.cast_lib, member_ref.cast_member
                );
            }
            continue;
        }
        let member = member.unwrap();

        // Debug: log each channel being rendered on stage
        if matches!(score_source, ScoreRef::Stage) {
            debug!(
                "  STAGE channel {}: member {}:{} type {:?}",
                channel_num, member_ref.cast_lib, member_ref.cast_member,
                member.member_type.member_type_id()
            );
        }

        match &member.member_type {
            CastMemberType::Bitmap(bitmap_member) => {
                let sprite_rect = {
                    let sprite =
                        get_score_sprite(&player.movie, score_source, channel_num).unwrap();
                    let rect = get_concrete_sprite_rect(player, sprite);

                    rect
                };
                let logical_rect = sprite_rect.clone();

                let sprite_bitmap = player
                    .bitmap_manager
                    .get_bitmap_mut(bitmap_member.image_ref);
                if sprite_bitmap.is_none() {
                    continue;
                }
                let src_bitmap = sprite_bitmap.unwrap();
                let sprite = get_score_sprite(&player.movie, score_source, channel_num).unwrap();
                let mask = if should_matte_sprite(sprite.ink as u32) {
                    if src_bitmap.matte.is_none() {
                        src_bitmap.create_matte(&palettes);
                    }
                    Some(src_bitmap.matte.as_ref().unwrap())
                } else {
                    None
                };

                let mut src_rect = IntRect::from(0, 0, 0, 0);

                let mut option = 0;

                if sprite.has_size_tweened || sprite.has_size_changed {
                    src_rect = IntRect::from(0, 0, src_bitmap.width as i32, src_bitmap.height as i32);
                } else if sprite.width > player.movie.rect.width() && sprite.height >  player.movie.rect.height() {
                    // sprite dimensions > movie dimensions
                    src_rect = IntRect::from(0, 0, bitmap_member.info.width as i32, bitmap_member.info.height as i32);
                    option = 1;
                } else if bitmap_member.info.width == 0 && bitmap_member.info.height == 0 {
                    // bitmap dimensions are 0
                    src_rect = IntRect::from(0, 0, sprite.width as i32, sprite.height as i32);
                    option = 2;
                } else if i32::from(bitmap_member.info.width) < sprite.width && i32::from(bitmap_member.info.height) < sprite.height || 
                    i32::from(bitmap_member.info.width) > sprite.width && i32::from(bitmap_member.info.height) > sprite.height {
                    // bitmap dimensions are < than sprite dimensions OR bitmap dimensions are > than sprite dimensions
                    src_rect = IntRect::from(0, 0, bitmap_member.info.width as i32, bitmap_member.info.height as i32);
                    option = 3;
                } else if sprite.width > i32::from(bitmap_member.info.width) || sprite.height > i32::from(bitmap_member.info.height) {
                    // sprite dimensions > bitmap dimensions
                    src_rect = IntRect::from(0, 0, bitmap_member.info.width as i32, bitmap_member.info.height as i32);
                    option = 4;
                } else {
                    src_rect = IntRect::from(0, 0, sprite.width as i32, sprite.height as i32);
                    option = 5;
                }

                let dst_rect = sprite_rect;
                let dst_rect = IntRect::from(
                    if sprite.flip_h {
                        dst_rect.right
                    } else {
                        dst_rect.left
                    },
                    if sprite.flip_v {
                        dst_rect.bottom
                    } else {
                        dst_rect.top
                    },
                    if sprite.flip_h {
                        dst_rect.left
                    } else {
                        dst_rect.right
                    },
                    if sprite.flip_v {
                        dst_rect.top
                    } else {
                        dst_rect.bottom
                    },
                );

                // 6) Params
                let mut params = CopyPixelsParams {
                    blend: sprite.blend,
                    ink: sprite.ink as u32,
                    color: sprite.color.clone(),
                    bg_color: sprite.bg_color.clone(),
                    mask_image: None,
                    is_text_rendering: false,
                    rotation: sprite.rotation,
                    skew: sprite.skew,
                    sprite: Some(&sprite.clone()),
                    original_dst_rect: Some(logical_rect),
                };

                if let Some(mask) = mask {
                    let mask_bitmap: &BitmapMask = mask.borrow();
                    params.mask_image = Some(mask_bitmap);
                }

                debug!(
                    "DRAW Sprite {} dimensions {}x{} bitmap dimensions {}x{} bitmap_member.info dimensions {}x{} Option {} src_rect: {:?}",
                    sprite.number,
                    sprite.width,
                    sprite.height,
                    bitmap.width,
                    bitmap.height,
                    bitmap_member.info.width,
                    bitmap_member.info.height,
                    option,
                    src_rect
                );

                bitmap.copy_pixels_with_params(
                    &palettes,
                    &src_bitmap,
                    dst_rect,
                    src_rect,
                    &params,
                );
            }
            CastMemberType::Shape(_) => {
                let sprite = get_score_sprite(&player.movie, score_source, channel_num).unwrap();

                // Skip rendering shapes with tiny dimensions (blank placeholders or zero-size)
                // These are used in Director as placeholder sprites that get assigned
                // different members later via scripts. Skip if EITHER dimension is <= 1.
                if sprite.width <= 1 || sprite.height <= 1 {
                    continue;
                }

                // Skip rendering shapes that use member 1:1 - this is a placeholder/empty shape
                // in Director that shouldn't be rendered visually. It's used as a dummy member
                // for sprites that will have their member changed by scripts.
                if let Some(member_ref) = &sprite.member {
                    if member_ref.cast_lib == 1 && member_ref.cast_member == 1 {
                        continue;
                    }
                }

                debug!(
                    "  SHAPE RENDER: channel {} member {:?} size {}x{} color {:?} bg {:?} ink {} blend {}",
                    channel_num, sprite.member, sprite.width, sprite.height,
                    sprite.color, sprite.bg_color, sprite.ink, sprite.blend
                );

                let rect = get_concrete_sprite_rect(player, sprite);
                // Apply offset for filmloop coordinate translation
                let sprite_rect = IntRect::from(
                    rect.left - offset.0,
                    rect.top - offset.1,
                    rect.right - offset.0,
                    rect.bottom - offset.1,
                );
                // Create a translated sprite for rendering if we have an offset
                if offset.0 != 0 || offset.1 != 0 {
                    let mut translated_sprite = sprite.clone();
                    translated_sprite.loc_h -= offset.0;
                    translated_sprite.loc_v -= offset.1;
                    bitmap.fill_shape_rect_with_sprite(&translated_sprite, sprite_rect, &palettes);
                } else {
                    bitmap.fill_shape_rect_with_sprite(sprite, sprite_rect, &palettes);
                }
            }
            CastMemberType::Field(field_member) => {
                let sprite = get_score_sprite(&player.movie, score_source, channel_num).unwrap();

                let font_opt = get_or_load_font_with_id(
                    &mut player.font_manager,
                    &player.movie.cast_manager,
                    &field_member.font,
                    Some(field_member.font_size),
                    None,
                    field_member.font_id,
                );

                if let Some(font) = font_opt {
                    let font_bitmap = player.bitmap_manager.get_bitmap(font.bitmap_ref).unwrap();

                    let params = CopyPixelsParams {
                        blend: sprite.blend as i32,
                        ink: sprite.ink as u32,
                        color: sprite.color.clone(),
                        bg_color: sprite.bg_color.clone(),
                        mask_image: None,
                        is_text_rendering: true,
                        rotation: 0.0,
                        skew: 0.0,
                        sprite: None,
                        original_dst_rect: None,
                    };

                    bitmap.draw_text(
                        &field_member.text,
                        &font,
                        font_bitmap,
                        sprite.loc_h,
                        sprite.loc_v,
                        params,
                        &palettes,
                        field_member.fixed_line_space,
                        field_member.top_spacing,
                    );

                    if player.keyboard_focus_sprite == sprite.number as i16 {
                        let cursor_x = sprite.loc_h + (sprite.width / 2);
                        let cursor_y = sprite.loc_v;
                        let cursor_width = 1;
                        let cursor_height = font.char_height as i32;

                        bitmap.fill_rect(
                            cursor_x,
                            cursor_y,
                            cursor_x + cursor_width,
                            cursor_y + cursor_height,
                            (0, 0, 0),
                            &palettes,
                            1.0,
                        );
                    }
                }
            }
            CastMemberType::Font(font_member) => {
                let sprite = get_score_sprite(&player.movie, score_source, channel_num).unwrap();

                // Get font by info with fallback
                let font: Rc<BitmapFont> =
                    if let Some(f) = player.font_manager.get_font_by_info(&font_member.font_info) {
                        Rc::new(f.clone()) // wrap &BitmapFont in Rc
                    } else if let Some(f) = player.font_manager.get_system_font() {
                        f // already Rc<BitmapFont>
                    } else {
                        continue;
                    };

                let font_bitmap: &mut bitmap::Bitmap = player
                    .bitmap_manager
                    .get_bitmap_mut(font.bitmap_ref)
                    .unwrap();

                let mask = if should_matte_sprite(sprite.ink as u32) {
                    if font_bitmap.matte.is_none() {
                        font_bitmap.create_matte_text(&palettes);
                    }
                    Some(font_bitmap.matte.as_ref().unwrap())
                } else {
                    None
                };

                let mut params = CopyPixelsParams {
                    blend: sprite.blend as i32,
                    ink: sprite.ink as u32,
                    color: sprite.color.clone(),
                    bg_color: sprite.bg_color.clone(),
                    mask_image: None,
                    is_text_rendering: true,
                    rotation: 0.0,
                    skew: 0.0,
                    sprite: None,
                    original_dst_rect: None,
                };

                if let Some(mask) = mask {
                    let mask_bitmap: &BitmapMask = mask.borrow();
                    params.mask_image = Some(mask_bitmap);
                }

                if !font_member.preview_html_spans.is_empty() {
                    if let Err(e) = FontMemberHandlers::render_html_text_to_bitmap(
                        bitmap,
                        &font_member.preview_html_spans,
                        &font,
                        font_bitmap,
                        &palettes,
                        font_member.fixed_line_space,
                        sprite.loc_h as i32,
                        sprite.loc_v as i32 + font_member.top_spacing as i32,
                        params,
                    ) {
                        console_warn!("HTML render error: {:?}", e);
                    }
                } else if !font_member.preview_text.is_empty() {
                    bitmap.draw_text(
                        &font_member.preview_text,
                        &font,
                        font_bitmap,
                        sprite.loc_h,
                        sprite.loc_v,
                        params,
                        &palettes,
                        font.char_height,
                        font_member.top_spacing,
                    );
                }
            }
            CastMemberType::Text(text_member) => {
                let sprite = get_score_sprite(&player.movie, score_source, channel_num).unwrap();
                let sprite_rect = get_concrete_sprite_rect(player, sprite);
                let draw_x = sprite_rect.left;
                let draw_y = sprite_rect.top;
                let draw_w = sprite_rect.width();
                let draw_h = sprite_rect.height();

                // Extract font properties from first styled span if available
                let (font_name, font_size, font_style) = if !text_member.html_styled_spans.is_empty() {
                    let first_style = &text_member.html_styled_spans[0].style;
                    let name = first_style.font_face.clone().unwrap_or_else(|| text_member.font.clone());
                    let size = first_style.font_size.map(|s| s as u16).unwrap_or(text_member.font_size);
                    // Convert bold/italic/underline to font_style: bit 0 = bold, bit 1 = italic, bit 2 = underline
                    let style = (if first_style.bold { 1u8 } else { 0 })
                        | (if first_style.italic { 2u8 } else { 0 })
                        | (if first_style.underline { 4u8 } else { 0 });
                    (name, size, Some(style))
                } else {
                    (text_member.font.clone(), text_member.font_size, None)
                };

                web_sys::console::log_1(&format!(
                    "🔤 Text member font request: name='{}', size={}, style={:?}",
                    font_name, font_size, font_style
                ).into());

                // Try to load font with specified style first
                let mut font_opt = get_or_load_font(
                    &mut player.font_manager,
                    &player.movie.cast_manager,
                    &font_name,
                    Some(font_size),
                    font_style,
                );

                // If not found with style, try without style
                if font_opt.is_none() && font_style.is_some() {
                    web_sys::console::log_1(&format!(
                        "⚠️ Font not found with style, trying without style..."
                    ).into());
                    font_opt = get_or_load_font(
                        &mut player.font_manager,
                        &player.movie.cast_manager,
                        &font_name,
                        Some(font_size),
                        None,
                    );
                }

                // If still not found with size, try without size
                if font_opt.is_none() {
                    web_sys::console::log_1(&format!(
                        "⚠️ Font not found with size, trying without size..."
                    ).into());
                    font_opt = get_or_load_font(
                        &mut player.font_manager,
                        &player.movie.cast_manager,
                        &font_name,
                        None,
                        None,
                    );
                }

                if let Some(ref font) = font_opt {
                    debug!(
                        "Using font: name='{}', size={}, style={}",
                        font.font_name, font.font_size, font.font_style
                    );
                } else {
                    debug!(
                        "Skipping text rendering: no font available for '{}' (size={}, style={:?}) and system font unavailable",
                        font_name, font_size, font_style
                    );
                }

                if let Some(font) = font_opt {
                    let font_bitmap = player.bitmap_manager.get_bitmap(font.bitmap_ref).unwrap();

                    let params = CopyPixelsParams {
                        blend: sprite.blend as i32,
                        ink: sprite.ink as u32,
                        color: sprite.color.clone(),
                        bg_color: sprite.bg_color.clone(),
                        mask_image: None,
                        is_text_rendering: true,
                        rotation: 0.0,
                        skew: 0.0,
                        sprite: None,
                        original_dst_rect: None,
                    };

                    // Use styled text rendering if html_styled_spans is populated
                    // BUT only use native rendering if the font is NOT a PFR bitmap font
                    // PFR fonts can't be used by Canvas2D, so we must use bitmap rendering
                    let is_pfr_font = font.char_widths.is_some();
                    if !text_member.html_styled_spans.is_empty() && !is_pfr_font {
                        // Parse alignment from text_member
                        let alignment = match text_member.alignment.to_lowercase().as_str() {
                            "center" | "#center" => TextAlignment::Center,
                            "right" | "#right" => TextAlignment::Right,
                            "justify" | "#justify" => TextAlignment::Justify,
                            _ => TextAlignment::Left,
                        };

                        let initial_span_size = text_member
                            .html_styled_spans
                            .first()
                            .and_then(|s| s.style.font_size)
                            .unwrap_or(0);
                        let should_override_span_sizes = text_member.font_size > 0
                            && (text_member.font_size as i32) != initial_span_size;

                        // Clone spans and apply text member runtime overrides when needed.
                        // The movie can set font, fontSize, fontStyle at runtime, so these
                        // should override whatever was in the original styled spans
                        let spans_with_defaults: Vec<StyledSpan> = text_member.html_styled_spans.iter().map(|span| {
                            let mut style = span.style.clone();

                            // ALWAYS use text_member's font if set (movie may have changed it)
                            if !text_member.font.is_empty() {
                                style.font_face = Some(text_member.font.clone());
                            } else if style.font_face.as_ref().map_or(true, |f| f.is_empty()) {
                                style.font_face = Some("Arial".to_string());
                            }

                            // Preserve per-span sizes unless the movie changed fontSize at runtime.
                            if should_override_span_sizes {
                                style.font_size = Some(text_member.font_size as i32);
                            } else if style.font_size.map_or(true, |s| s <= 0) {
                                style.font_size = Some(12);
                            }

                            // Use sprite color if span doesn't have color
                            if style.color.is_none() {
                                style.color = match &sprite.color {
                                    ColorRef::Rgb(r, g, b) => {
                                        Some(((*r as u32) << 16) | ((*g as u32) << 8) | (*b as u32))
                                    }
                                    ColorRef::PaletteIndex(idx) => {
                                        match *idx {
                                            0 => Some(0xFFFFFF),
                                            255 => Some(0x000000),
                                            _ => Some(0x000000),
                                        }
                                    }
                                };
                            }

                            // ALWAYS apply text_member's fontStyle (movie may have changed it)
                            if !text_member.font_style.is_empty() {
                                style.bold = text_member.font_style.iter().any(|s| s == "bold");
                                style.italic = text_member.font_style.iter().any(|s| s == "italic");
                                style.underline = text_member.font_style.iter().any(|s| s == "underline");
                            }

                            StyledSpan {
                                text: span.text.clone(),
                                style,
                            }
                        }).collect();

                        // Use native browser text rendering for smooth, anti-aliased text
                        if let Err(e) = FontMemberHandlers::render_native_text_to_bitmap(
                            bitmap,
                            &spans_with_defaults,
                            draw_x,
                            draw_y + text_member.top_spacing as i32,
                            draw_w,
                            draw_h,
                            alignment,
                            draw_w,
                            text_member.word_wrap,
                            None, // Color is now in the spans
                            text_member.fixed_line_space,
                        ) {
                            console_warn!("Native text render error for Text member: {:?}", e);
                        }
                    } else {
                        bitmap.draw_text(
                            &text_member.text,
                            &font,
                            font_bitmap,
                            draw_x,
                            draw_y,
                            params,
                            &palettes,
                            text_member.fixed_line_space,
                            text_member.top_spacing,
                        );
                    }
                }
            }
            CastMemberType::FilmLoop(film_loop) => {
                // ---- 1. Snapshot sprite data ----
                // The filmloop's rect is stored in info as:
                // - reg_point = (left, top) coordinates of the rect
                // - width = right coordinate
                // - height = bottom coordinate
                // So actual dimensions = (width - reg_point.0, height - reg_point.1)
                let info_rect_left = film_loop.info.reg_point.0 as i32;
                let info_rect_top = film_loop.info.reg_point.1 as i32;
                let info_rect_right = film_loop.info.width as i32;
                let info_rect_bottom = film_loop.info.height as i32;
                let info_initial_rect = IntRect::from(info_rect_left, info_rect_top, info_rect_right, info_rect_bottom);

                let (
                    sprite_rect,
                    blend,
                    ink,
                    color,
                    bg_color,
                    rotation,
                    skew,
                    logical_rect,
                    initial_rect,
                    current_frame,
                    info_width,
                    info_height,
                ) = {
                    let sprite = get_score_sprite(&player.movie, score_source, channel_num).unwrap();

                    let rect = get_concrete_sprite_rect(player, sprite);

                    (
                        rect.clone(),
                        sprite.blend,
                        sprite.ink as u32,
                        sprite.color.clone(),
                        sprite.bg_color.clone(),
                        sprite.rotation,
                        sprite.skew,
                        rect, // logical rect
                        info_initial_rect,
                        film_loop.current_frame,
                        film_loop.info.width as i32,
                        film_loop.info.height as i32,
                    )
                };

                // ---- 2. Create filmloop bitmap using INITIAL_RECT dimensions ----
                // The filmloop is rendered at its natural size (initial_rect).
                // This bitmap will then be positioned at sprite_rect location on the stage.
                // The content is NOT scaled even if sprite_rect is larger/smaller.
                let width = initial_rect.width().max(1);
                let height = initial_rect.height().max(1);

                debug!(
                    "Rendering FilmLoop: channel {} frame {} ink={} blend={} initial_rect ({}, {}, {}, {}) bitmap size {}x{} sprite_rect ({}, {}, {}, {}) offset ({}, {})",
                    channel_num,
                    current_frame,
                    ink, blend,
                    initial_rect.left, initial_rect.top, initial_rect.right, initial_rect.bottom,
                    width, height,
                    sprite_rect.left, sprite_rect.top, sprite_rect.right, sprite_rect.bottom,
                    initial_rect.left, initial_rect.top
                );

                let mut filmloop_bitmap = Bitmap::new(
                    width as u16,
                    height as u16,
                    32,
                    32,
                    8, // alpha_depth = 8 for transparency support
                    PaletteRef::BuiltIn(get_system_default_palette()),
                );
                // Enable alpha channel for filmloop transparency
                filmloop_bitmap.use_alpha = true;
                // Clear to fully transparent (RGBA 0,0,0,0) so only rendered sprites are visible
                filmloop_bitmap.data.fill(0);

                // ---- 3. Recursive render with coordinate offset ----
                // Render at natural size (initial_rect dimensions)
                // Pass parent sprite's ink, color, and bgColor - Director behavior is that
                // film loop internal sprites use the parent sprite's ink semantics.
                render_score_to_bitmap_with_offset(
                    player,
                    &ScoreRef::FilmLoop(member_ref.clone()),
                    &mut filmloop_bitmap,
                    debug_sprite_num,
                    IntRect::from_size(0, 0, width, height),
                    (initial_rect.left, initial_rect.top),
                    Some(FilmLoopParentProps {
                        ink,
                        color: color.clone(),
                        bg_color: bg_color.clone(),
                    }),
                );

                // ---- 4. Composite filmloop onto stage ----
                // Count total opaque pixels in filmloop bitmap to verify rendering worked
                let opaque_count = (0..filmloop_bitmap.data.len()).step_by(4)
                    .filter(|&i| i + 3 < filmloop_bitmap.data.len() && filmloop_bitmap.data[i + 3] > 0)
                    .count();
                let transparent_count = (0..filmloop_bitmap.data.len()).step_by(4)
                    .filter(|&i| i + 3 < filmloop_bitmap.data.len() && filmloop_bitmap.data[i + 3] == 0)
                    .count();
                let total_pixels = filmloop_bitmap.width as usize * filmloop_bitmap.height as usize;
                debug!(
                    "    FILMLOOP BITMAP STATS: {}x{} total={} opaque={} ({}%) transparent={} ({}%)",
                    filmloop_bitmap.width, filmloop_bitmap.height, total_pixels,
                    opaque_count, if total_pixels > 0 { opaque_count * 100 / total_pixels } else { 0 },
                    transparent_count, if total_pixels > 0 { transparent_count * 100 / total_pixels } else { 0 }
                );

                let params = CopyPixelsParams {
                    blend,
                    ink,
                    color,
                    bg_color,
                    mask_image: None,
                    is_text_rendering: false,
                    rotation,
                    skew,
                    sprite: None,
                    original_dst_rect: Some(logical_rect),
                };

                // Debug: log filmloop bitmap properties before compositing
                debug!(
                    "    FILMLOOP COMPOSITE: use_alpha={} bit_depth={} orig_bit_depth={} ink={}",
                    filmloop_bitmap.use_alpha, filmloop_bitmap.bit_depth,
                    filmloop_bitmap.original_bit_depth, ink
                );

                // Position the filmloop bitmap at sprite_rect location, but keep natural size.
                // The filmloop is NOT scaled to fill sprite_rect.
                let dst_rect = IntRect::from_size(sprite_rect.left, sprite_rect.top, width, height);
                debug!(
                    "    FILMLOOP DST_RECT: ({}, {}, {}, {}) <- sprite_rect.left={} sprite_rect.top={}",
                    dst_rect.left, dst_rect.top, dst_rect.right, dst_rect.bottom,
                    sprite_rect.left, sprite_rect.top
                );
                bitmap.copy_pixels_with_params(
                    &palettes,
                    &filmloop_bitmap,
                    dst_rect,
                    IntRect::from_size(0, 0, width, height),
                    &params,
                );
            }
            _ => {}
        }
    }

    // Draw debug rect
    if let Some(sprite) = debug_sprite_num.and_then(|x| player.movie.score.get_sprite(x)) {
        let sprite_rect = get_concrete_sprite_rect(player, sprite);
        bitmap.stroke_rect(
            sprite_rect.left,
            sprite_rect.top,
            sprite_rect.right,
            sprite_rect.bottom,
            (255, 0, 0),
            &palettes,
            1.0,
        );
        bitmap.set_pixel(sprite.loc_h, sprite.loc_v, (0, 255, 0), &palettes);
    }

    // Draw pick rect
    let is_picking_sprite = player.keyboard_manager.is_alt_down()
        && (player.keyboard_manager.is_control_down() || player.keyboard_manager.is_command_down());
    if is_picking_sprite {
        let hovered_sprite = get_sprite_at(player, player.mouse_loc.0, player.mouse_loc.1, false);
        if let Some(hovered_sprite) = hovered_sprite {
            let sprite = player
                .movie
                .score
                .get_sprite(hovered_sprite as i16)
                .unwrap();
            let sprite_rect = get_concrete_sprite_rect(player, sprite);
            bitmap.stroke_rect(
                sprite_rect.left,
                sprite_rect.top,
                sprite_rect.right,
                sprite_rect.bottom,
                (0, 255, 0),
                &palettes,
                1.0,
            );
        }
    }
}

fn draw_cursor(player: &mut DirPlayer, bitmap: &mut Bitmap, palettes: &PaletteMap) {
    let hovered_sprite = get_sprite_at(player, player.mouse_loc.0, player.mouse_loc.1, false);
    let cursor_ref = if let Some(hovered_sprite) = hovered_sprite {
        let hovered_sprite = player
            .movie
            .score
            .get_sprite(hovered_sprite as i16)
            .unwrap();
        hovered_sprite.cursor_ref.as_ref()
    } else {
        None
    };
    let cursor_ref = cursor_ref.or(Some(&player.cursor));
    let cursor_list = cursor_ref.and_then(|x| match x {
        CursorRef::Member(x) => Some(x),
        _ => None,
    });
    let cursor_bitmap_member = cursor_list
        .and_then(|x| x.first().map(|x| *x)) // TODO: what to do with other values? maybe animate?
        .and_then(|x| {
            player
                .movie
                .cast_manager
                .find_member_by_slot_number(x as u32)
        })
        .and_then(|x| x.member_type.as_bitmap());

    let cursor_bitmap_ref = cursor_bitmap_member.and_then(|x| Some(x.image_ref));

    let cursor_mask_bitmap_ref = cursor_list
        .and_then(|x| x.get(1).map(|x| *x)) // TODO: what to do with other values? maybe animate?
        .and_then(|x| {
            player
                .movie
                .cast_manager
                .find_member_by_slot_number(x as u32)
        })
        .and_then(|x| x.member_type.as_bitmap())
        .and_then(|x| Some(x.image_ref));

    if let Some(cursor_bitmap_ref) = cursor_bitmap_ref {
        let cursor_bitmap = player.bitmap_manager.get_bitmap(cursor_bitmap_ref).unwrap();
        let mask = if let Some(cursor_mask_bitmap_ref) = cursor_mask_bitmap_ref {
            let cursor_mask_bitmap = player
                .bitmap_manager
                .get_bitmap(cursor_mask_bitmap_ref)
                .unwrap();
            let mask = cursor_mask_bitmap.to_mask();
            Some(mask)
        } else {
            None
        };
        let cursor_bitmap_member = cursor_bitmap_member.unwrap();
        bitmap.copy_pixels_with_params(
            &palettes,
            cursor_bitmap,
            IntRect::from_size(
                player.mouse_loc.0 - cursor_bitmap_member.reg_point.0 as i32,
                player.mouse_loc.1 - cursor_bitmap_member.reg_point.1 as i32,
                cursor_bitmap.width as i32,
                cursor_bitmap.height as i32,
            ),
            IntRect::from_size(
                0,
                0,
                cursor_bitmap.width as i32,
                cursor_bitmap.height as i32,
            ),
            &CopyPixelsParams {
                blend: 100,
                ink: 41,
                bg_color: bitmap.get_bg_color_ref(),
                color: bitmap.get_fg_color_ref(),
                mask_image: mask.as_ref(),
                is_text_rendering: false,
                rotation: 0.0,
                skew: 0.0,
                sprite: None,
                original_dst_rect: None,
            },
        );
    }
}

impl PlayerCanvasRenderer {
    #[allow(dead_code)]
    pub fn set_size(&mut self, width: u32, height: u32) {
        self.size = (width, height);
        self.canvas.set_width(width);
        self.canvas.set_height(height);
    }

    pub fn set_preview_size(&mut self, width: u32, height: u32) {
        self.preview_size = (width, height);
        self.preview_canvas.set_width(width);
        self.preview_canvas.set_height(height);

        // console_warn!("Set preview size: {}x{}", width, height);
    }

    pub fn set_container_element(&mut self, container_element: web_sys::HtmlElement) {
        if self.canvas.parent_node().is_some() {
            self.canvas.remove();
        }
        container_element.append_child(&self.canvas).unwrap();
        self.container_element = Some(container_element);
    }

    pub fn set_preview_container_element(
        &mut self,
        container_element: Option<web_sys::HtmlElement>,
    ) {
        if self.preview_canvas.parent_node().is_some() {
            self.preview_canvas.remove();
        }
        if let Some(container_element) = container_element {
            container_element
                .append_child(&self.preview_canvas)
                .unwrap();
            self.preview_container_element = Some(container_element);
        }
    }

    pub fn draw_preview_frame(&mut self, player: &mut DirPlayer) {
        if self.preview_member_ref.is_none()
            || self.preview_container_element.is_none()
            || self.preview_ctx2d.is_null()
            || self.preview_ctx2d.is_undefined()
        {
            return;
        }

        let member_ref = self.preview_member_ref.as_ref().unwrap();
        let member = player.movie.cast_manager.find_member_by_ref(member_ref);
        if member.is_none() {
            return;
        }
        let member = member.unwrap();
        match &member.member_type {
            CastMemberType::Bitmap(sprite_member) => {
                let sprite_bitmap = player.bitmap_manager.get_bitmap(sprite_member.image_ref);
                if sprite_bitmap.is_none() {
                    return;
                }
                let sprite_bitmap = sprite_bitmap.unwrap();
                let width = sprite_bitmap.width as u32;
                let height = sprite_bitmap.height as u32;
                let mut bitmap = Bitmap::new(
                    width as u16,
                    height as u16,
                    32,
                    32,
                    0,
                    PaletteRef::BuiltIn(get_system_default_palette()),
                );
                let palettes = &player.movie.cast_manager.palettes();
                bitmap.fill_relative_rect(
                    0,
                    0,
                    0,
                    0,
                    resolve_color_ref(
                        &palettes,
                        &player.bg_color,
                        &PaletteRef::BuiltIn(get_system_default_palette()),
                        sprite_bitmap.original_bit_depth,
                    ),
                    palettes,
                    1.0,
                );
                bitmap.copy_pixels(
                    &palettes,
                    sprite_bitmap,
                    IntRect::from(
                        0,
                        0,
                        sprite_bitmap.width as i32,
                        sprite_bitmap.height as i32,
                    ),
                    IntRect::from(
                        0,
                        0,
                        sprite_bitmap.width as i32,
                        sprite_bitmap.height as i32,
                    ),
                    &HashMap::new(),
                    None,
                );
                bitmap.set_pixel(
                    sprite_member.reg_point.0 as i32,
                    sprite_member.reg_point.1 as i32,
                    (255, 0, 255),
                    palettes,
                );

                if self.preview_size.0 != bitmap.width as u32
                    || self.preview_size.1 != bitmap.height as u32
                {
                    self.set_preview_size(bitmap.width as u32, bitmap.height as u32);
                }
                let slice_data = Clamped(bitmap.data.as_slice());
                let image_data = web_sys::ImageData::new_with_u8_clamped_array_and_sh(
                    slice_data,
                    bitmap.width.into(),
                    bitmap.height.into(),
                );
                self.preview_ctx2d.set_fill_style(&safe_js_string("white"));
                match image_data {
                    Ok(image_data) => {
                        self.preview_ctx2d
                            .put_image_data(&image_data, 0.0, 0.0)
                            .unwrap();
                    }
                    _ => {}
                }
            }
            CastMemberType::FilmLoop(loop_member) => {
                let sprite =
                    get_score_sprite(&player.movie, &ScoreRef::FilmLoop(member_ref.clone()), 1)
                        .unwrap();
                let sprite_rect = get_concrete_sprite_rect(player, sprite);
                let dest_x = sprite.loc_h;
                let dest_y = sprite.loc_v;
                let width = loop_member.info.width as i32;
                let height = loop_member.info.height as i32;
                let mut bitmap = Bitmap::new(
                    width as u16,
                    height as u16,
                    32,
                    32,
                    0,
                    PaletteRef::BuiltIn(get_system_default_palette()),
                );
                render_score_to_bitmap(
                    player,
                    &ScoreRef::FilmLoop(member_ref.clone()),
                    &mut bitmap,
                    None,
                    IntRect::from_size(0, 0, width, height),
                );
                if self.preview_size.0 != bitmap.width as u32
                    || self.preview_size.1 != bitmap.height as u32
                {
                    self.set_preview_size(bitmap.width as u32, bitmap.height as u32);
                }
                let slice_data = Clamped(bitmap.data.as_slice());
                let image_data = web_sys::ImageData::new_with_u8_clamped_array_and_sh(
                    slice_data,
                    bitmap.width.into(),
                    bitmap.height.into(),
                );
                self.preview_ctx2d
                    .set_fill_style(&JsValue::from_str("white"));
                match image_data {
                    Ok(image_data) => {
                        self.preview_ctx2d
                            .put_image_data(&image_data, 0.0, 0.0)
                            .unwrap();
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    pub fn draw_frame(&mut self, player: &mut DirPlayer) {
        // let time = chrono::Local::now().timestamp_millis() as i64;
        // let time_seconds = time as f64 / 1000.0;
        // let oscillated_r = 127.0 + 255.0 * (time_seconds * 2.0 * std::f32::consts::PI as f64).sin();
        // let oscillated_g = 127.0 + 255.0 * (time_seconds * 2.0 * std::f32::consts::PI as f64 + (std::f32::consts::PI / 2.0) as f64).sin();
        // let oscillated_b = 127.0 + 255.0 * (time_seconds * 2.0 * std::f32::consts::PI as f64 + (std::f32::consts::PI) as f64).sin();

        // let color = format!("rgba({}, {}, {}, {})", oscillated_r, oscillated_g, oscillated_b, 1);
        // let bg_color = "black";

        // let (width, height) = self.size;
        // self.ctx2d.clear_rect(0.0, 0.0, width as f64, height as f64);
        // self.ctx2d.set_fill_style(&JsValue::from_str(&bg_color));
        // self.ctx2d.fill_rect(0.0, 0.0, width as f64, height as f64);

        // self.ctx2d.set_fill_style(&JsValue::from_str("black"));
        // self.ctx2d
        //     .fill_text(
        //         &format!("dir_version: {}", player.movie.dir_version),
        //         0.0,
        //         10.0,
        //     )
        //     .unwrap();

        let movie_width = player.movie.rect.width();
        let movie_height = player.movie.rect.height();

        if self.bitmap.width != movie_width as u16 || self.bitmap.height != movie_height as u16 {
            self.bitmap = Bitmap::new(
                movie_width as u16,
                movie_height as u16,
                32,
                32,
                0,
                PaletteRef::BuiltIn(get_system_default_palette()),
            );
        }
        let bitmap = &mut self.bitmap;
        render_stage_to_bitmap(player, bitmap, self.debug_selected_channel_num);

        if let Some(font) = player.font_manager.get_system_font() {
            let font_bitmap = player.bitmap_manager.get_bitmap(font.bitmap_ref).unwrap();
            let txt = format!(
                "Datum count: {}\nScript count: {}",
                player.allocator.datum_count(),
                player.allocator.script_instance_count()
            );

            let params = CopyPixelsParams {
                blend: 100,
                ink: 36,
                color: bitmap.get_fg_color_ref(),
                bg_color: bitmap.get_bg_color_ref(),
                mask_image: None,
                is_text_rendering: false,
                rotation: 0.0,
                skew: 0.0,
                sprite: None,
                original_dst_rect: None,
            };

            bitmap.draw_text(
                txt.as_str(),
                &font,
                font_bitmap,
                0,
                0,
                params,
                &player.movie.cast_manager.palettes(),
                0,
                0,
            );
        }
        let slice_data = Clamped(bitmap.data.as_slice());
        let image_data = web_sys::ImageData::new_with_u8_clamped_array_and_sh(
            slice_data,
            bitmap.width.into(),
            bitmap.height.into(),
        );
        self.ctx2d.set_fill_style(&safe_js_string("white"));
        match image_data {
            Ok(image_data) => {
                self.ctx2d.put_image_data(&image_data, 0.0, 0.0).unwrap();
            }
            _ => {}
        }
    }

    /// Get the backend name
    pub fn backend_name(&self) -> &'static str {
        "Canvas2D"
    }
}

impl Renderer for PlayerCanvasRenderer {
    fn draw_frame(&mut self, player: &mut DirPlayer) {
        PlayerCanvasRenderer::draw_frame(self, player)
    }

    fn draw_preview_frame(&mut self, player: &mut DirPlayer) {
        PlayerCanvasRenderer::draw_preview_frame(self, player)
    }

    fn set_size(&mut self, width: u32, height: u32) {
        PlayerCanvasRenderer::set_size(self, width, height)
    }

    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn backend_name(&self) -> &'static str {
        PlayerCanvasRenderer::backend_name(self)
    }

    fn canvas(&self) -> &web_sys::HtmlCanvasElement {
        &self.canvas
    }

    fn set_preview_member_ref(&mut self, member_ref: Option<CastMemberRef>) {
        self.preview_member_ref = member_ref;
    }

    fn set_preview_container_element(&mut self, container_element: Option<web_sys::HtmlElement>) {
        PlayerCanvasRenderer::set_preview_container_element(self, container_element)
    }
}

thread_local! {
    pub static RENDERER_LOCK: RefCell<Option<DynamicRenderer>> = RefCell::new(None);
}

#[allow(dead_code)]
pub fn with_renderer_ref<F, R>(f: F) -> R
where
    F: FnOnce(&DynamicRenderer) -> R,
{
    RENDERER_LOCK.with(|renderer_lock| {
        let renderer = renderer_lock.borrow();
        f(renderer.as_ref().unwrap())
    })
}

pub fn with_renderer_mut<F, R>(f: F) -> R
where
    F: FnOnce(&mut Option<DynamicRenderer>) -> R,
{
    RENDERER_LOCK.with_borrow_mut(|renderer_lock| f(renderer_lock))
}

/// Helper to access Canvas2D renderer for Canvas2D-specific operations
#[allow(dead_code)]
pub fn with_canvas2d_renderer<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&PlayerCanvasRenderer) -> R,
{
    RENDERER_LOCK.with(|renderer_lock| {
        let renderer = renderer_lock.borrow();
        if let Some(dynamic) = renderer.as_ref() {
            if let Some(canvas2d) = dynamic.as_canvas2d() {
                return Some(f(canvas2d));
            }
        }
        None
    })
}

/// Helper to access Canvas2D renderer mutably for Canvas2D-specific operations
pub fn with_canvas2d_renderer_mut<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut PlayerCanvasRenderer) -> R,
{
    RENDERER_LOCK.with_borrow_mut(|renderer_lock| {
        if let Some(dynamic) = renderer_lock {
            if let Some(canvas2d) = dynamic.as_canvas2d_mut() {
                return Some(f(canvas2d));
            }
        }
        None
    })
}

/// Legacy helper - kept for backward compatibility with existing code
/// Prefer using with_renderer_mut or with_canvas2d_renderer_mut
#[allow(dead_code)]
pub fn with_canvas_renderer_mut<F, R>(f: F) -> R
where
    F: FnOnce(&mut PlayerCanvasRenderer) -> R,
    R: Default,
{
    with_canvas2d_renderer_mut(f).unwrap_or_default()
}

#[wasm_bindgen]
pub fn player_set_preview_member_ref(cast_lib: i32, cast_num: i32) -> Result<(), JsValue> {
    use crate::rendering_gpu::Renderer;
    with_renderer_mut(|renderer_lock| {
        if let Some(dynamic) = renderer_lock {
            dynamic.set_preview_member_ref(Some(CastMemberRef {
                cast_lib,
                cast_member: cast_num,
            }));
        }
    });
    Ok(())
}

#[wasm_bindgen]
pub fn player_set_debug_selected_channel(channel_num: i16) -> Result<(), JsValue> {
    // Set on whichever renderer is active (Canvas2D or WebGL2)
    with_renderer_mut(|renderer_lock| {
        if let Some(dynamic) = renderer_lock {
            if let Some(canvas2d) = dynamic.as_canvas2d_mut() {
                canvas2d.debug_selected_channel_num = Some(channel_num);
            } else if let Some(webgl2) = dynamic.as_webgl2_mut() {
                webgl2.debug_selected_channel_num = Some(channel_num);
            }
        }
    });
    JsApi::dispatch_channel_changed(channel_num);
    Ok(())
}

#[wasm_bindgen]
pub fn player_set_preview_parent(parent_selector: &str) -> Result<(), JsValue> {
    use crate::rendering_gpu::Renderer;
    if parent_selector.is_empty() {
        with_renderer_mut(|renderer_lock| {
            if let Some(dynamic) = renderer_lock {
                dynamic.set_preview_container_element(None);
            }
        });
        return Ok(());
    }
    let parent_element = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .query_selector(parent_selector)
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()?;

    with_renderer_mut(|renderer_lock| {
        if let Some(dynamic) = renderer_lock {
            dynamic.set_preview_container_element(Some(parent_element));
        }
    });

    Ok(())
}

/// Helper to set pixel-perfect rendering styles on a canvas
fn set_pixelated_canvas_style(canvas: &web_sys::HtmlCanvasElement) {
    canvas
        .style()
        .set_property("image-rendering", "pixelated")
        .unwrap_or(());
    canvas
        .style()
        .set_property("image-rendering", "-moz-crisp-edges")
        .unwrap_or(());
    canvas
        .style()
        .set_property("image-rendering", "crisp-edges")
        .unwrap_or(());
}

/// Create a Canvas2D renderer (fallback)
fn create_canvas2d_renderer(
    canvas_size: (u32, u32),
) -> PlayerCanvasRenderer {
    let canvas = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("canvas")
        .unwrap()
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .unwrap();

    let preview_canvas = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("canvas")
        .unwrap()
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .unwrap();

    canvas.set_width(canvas_size.0);
    canvas.set_height(canvas_size.1);

    preview_canvas.set_width(1);
    preview_canvas.set_height(1);

    set_pixelated_canvas_style(&canvas);
    set_pixelated_canvas_style(&preview_canvas);

    let ctx = canvas
        .get_context("2d")
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::CanvasRenderingContext2d>()
        .unwrap();

    let preview_ctx = preview_canvas
        .get_context("2d")
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::CanvasRenderingContext2d>()
        .unwrap();

    ctx.set_image_smoothing_enabled(false);
    preview_ctx.set_image_smoothing_enabled(false);

    PlayerCanvasRenderer {
        container_element: None,
        preview_container_element: None,
        canvas,
        preview_canvas,
        ctx2d: ctx,
        preview_ctx2d: preview_ctx,
        size: canvas_size,
        preview_size: (1, 1),
        preview_member_ref: None,
        debug_selected_channel_num: None,
        bitmap: Bitmap::new(
            1,
            1,
            32,
            32,
            0,
            PaletteRef::BuiltIn(get_system_default_palette()),
        ),
    }
}

/// Try to create a WebGL2 renderer, returns None if not supported or fails
fn try_create_webgl2_renderer(
    canvas_size: (u32, u32),
) -> Option<crate::rendering_gpu::webgl2::WebGL2Renderer> {
    use crate::rendering_gpu::webgl2::WebGL2Renderer;

    // Check if WebGL2 is supported
    if !crate::rendering_gpu::is_webgl2_supported() {
        console::log_1(&"WebGL2 not supported, falling back to Canvas2D".into());
        return None;
    }

    // Create canvases for WebGL2
    let canvas = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("canvas")
        .ok()?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .ok()?;

    let preview_canvas = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("canvas")
        .ok()?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .ok()?;

    canvas.set_width(canvas_size.0);
    canvas.set_height(canvas_size.1);
    preview_canvas.set_width(1);
    preview_canvas.set_height(1);

    set_pixelated_canvas_style(&canvas);
    set_pixelated_canvas_style(&preview_canvas);

    // Try to create the WebGL2 renderer
    match WebGL2Renderer::new(canvas, preview_canvas) {
        Ok(renderer) => {
            console::log_1(&"WebGL2 renderer created successfully".into());
            Some(renderer)
        }
        Err(e) => {
            console::warn_1(&format!("Failed to create WebGL2 renderer: {:?}, falling back to Canvas2D", e).into());
            None
        }
    }
}

#[wasm_bindgen]
pub fn player_create_canvas() -> Result<(), JsValue> {
    let container_element = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .query_selector("#stage_canvas_container")
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()?;

    // Create renderer if it doesn't exist
    with_renderer_mut(|renderer_lock| {
        if renderer_lock.is_none() {
            let canvas_size = reserve_player_ref(|player| {
                (
                    player.movie.rect.width() as u32,
                    player.movie.rect.height() as u32,
                )
            });

            // Try WebGL2 first, fall back to Canvas2D
            let dynamic_renderer = if let Some(webgl2_renderer) = try_create_webgl2_renderer(canvas_size) {
                DynamicRenderer::WebGL2(webgl2_renderer)
            } else {
                DynamicRenderer::Canvas2D(create_canvas2d_renderer(canvas_size))
            };

            *renderer_lock = Some(dynamic_renderer);
            spawn_local(async {
                run_draw_loop().await;
            });
        }
    });

    // Set container element - need to handle both renderer types
    with_renderer_mut(|renderer_lock| {
        if let Some(renderer) = renderer_lock {
            match renderer {
                DynamicRenderer::Canvas2D(canvas_renderer) => {
                    canvas_renderer.set_container_element(container_element.clone());
                }
                DynamicRenderer::WebGL2(webgl_renderer) => {
                    // For WebGL2, we need to append the canvas to the container
                    if webgl_renderer.canvas().parent_node().is_some() {
                        webgl_renderer.canvas().remove();
                    }
                    container_element.append_child(webgl_renderer.canvas()).unwrap();
                }
            }
        }
    });

    Ok(())
}

fn request_animation_frame(f: &Closure<dyn FnMut()>) {
    web_sys::window()
        .unwrap()
        .request_animation_frame(f.as_ref().unchecked_ref())
        .unwrap();
}

async fn run_draw_loop() {
    let rc = Rc::new(RefCell::new(None));
    let rc_clone = rc.clone();

    let mut last_frame_ms = 0;
    let cb = Closure::<dyn FnMut()>::new(move || {
        let mut player = unsafe { PLAYER_OPT.as_mut().unwrap() };
        let draw_fps = 24;

        if Local::now().timestamp_millis() - last_frame_ms >= 1000 / draw_fps as i64 {
            last_frame_ms = Local::now().timestamp_millis();
            with_renderer_mut(|renderer_lock| {
                if let Some(renderer) = renderer_lock {
                    renderer.draw_frame(&mut player);
                    renderer.draw_preview_frame(&mut player);
                }
            });
        }

        let cb = rc.as_ref().borrow();
        let cb = cb.as_ref().unwrap();
        request_animation_frame(&cb);
    });
    rc_clone.replace(Some(cb));

    let cb = rc_clone.as_ref().borrow();
    let cb = cb.as_ref().unwrap();
    request_animation_frame(&cb);
}
