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
use crate::player::font::bitmap_font_copy_char;
use crate::player::handlers::datum_handlers::cast_member::font::{FontMemberHandlers, TextAlignment, StyledSpan, HtmlStyle};
use crate::director::lingo::datum::Datum;
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
    pub preview_font_size: Option<u16>,
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

    // Try case-insensitive match in font cache before falling back to system font
    let font_name_lower = font_name.to_lowercase();
    for (key, font) in font_manager.font_cache.iter() {
        if key.to_lowercase() == font_name_lower
            || key.to_lowercase().starts_with(&format!("{}_", font_name_lower))
        {
            debug!(
                "Font '{}' (id={:?}) not found by exact match, using cache entry '{}'",
                font_name, font_id, key
            );
            return Some(Rc::clone(font));
        }
    }

    // PFR canonical fallback: prefer an embedded PFR font over system font / Canvas2D native
    if !font_name.is_empty() {
        use crate::player::font::FontManager;
        let canon = FontManager::canonical_font_name(font_name);
        if !canon.is_empty() {
            for (_key, font) in font_manager.font_cache.iter() {
                if font.char_widths.is_some() && FontManager::canonical_font_name(&font.font_name) == canon {
                    return Some(Rc::clone(font));
                }
            }
        }
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

/// Render a preview bitmap for a cast member. Returns `None` if the member type
/// is not previewable or required data (fonts, bitmaps) is unavailable.
pub fn render_preview_bitmap(
    player: &mut DirPlayer,
    member_ref: &CastMemberRef,
    preview_font_size: Option<u16>,
) -> Option<Bitmap> {
    let member = player.movie.cast_manager.find_member_by_ref(member_ref)?;
    match &member.member_type {
        CastMemberType::Bitmap(sprite_member) => {
            let image_ref = sprite_member.image_ref;
            let reg_point = sprite_member.reg_point;
            let sprite_bitmap = player.bitmap_manager.get_bitmap(image_ref)?;
            let width = sprite_bitmap.width;
            let height = sprite_bitmap.height;
            let original_bit_depth = sprite_bitmap.original_bit_depth;

            let mut bitmap = Bitmap::new(
                width,
                height,
                32,
                32,
                0,
                PaletteRef::BuiltIn(get_system_default_palette()),
            );
            let palettes = &player.movie.cast_manager.palettes();
            bitmap.fill_relative_rect(
                0, 0, 0, 0,
                resolve_color_ref(
                    &palettes,
                    &player.bg_color,
                    &PaletteRef::BuiltIn(get_system_default_palette()),
                    original_bit_depth,
                ),
                palettes,
                1.0,
            );
            let sprite_bitmap = player.bitmap_manager.get_bitmap(image_ref)?;
            bitmap.copy_pixels(
                &palettes,
                sprite_bitmap,
                IntRect::from(0, 0, width as i32, height as i32),
                IntRect::from(0, 0, width as i32, height as i32),
                &HashMap::new(),
                None,
            );
            bitmap.set_pixel(
                reg_point.0 as i32,
                reg_point.1 as i32,
                (255, 0, 255),
                palettes,
            );
            Some(bitmap)
        }
        CastMemberType::FilmLoop(loop_member) => {
            let width = loop_member.info.width as i32;
            let height = loop_member.info.height as i32;
            let member_ref = member_ref.clone();

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
                &ScoreRef::FilmLoop(member_ref),
                &mut bitmap,
                None,
                IntRect::from_size(0, 0, width, height),
            );
            Some(bitmap)
        }
        CastMemberType::Font(font_member) => {
            let font_name = font_member.font_info.name.clone();
            let font_style = font_member.font_info.style;

            // Resolve font: if a size override is requested and the member has PFR data,
            // re-rasterize from THIS member's PFR data at the requested size.
            // Otherwise use the member's own bitmap_ref directly.
            let pfr_data = font_member.pfr_data.clone();
            let pfr_parsed = font_member.pfr_parsed.clone();
            let bitmap_ref = font_member.bitmap_ref;
            let member_char_width = font_member.char_width;
            let member_char_height = font_member.char_height;
            let member_grid_columns = font_member.grid_columns;
            let member_grid_rows = font_member.grid_rows;
            let member_first_char_num = font_member.first_char_num;
            let member_char_widths = font_member.char_widths.clone();
            let member_font_size = font_member.font_info.size;
            let font_info = font_member.font_info.clone();

            let font: Rc<BitmapFont> =
                if let (Some(req_size), Some(ref raw), Some(ref parsed)) = (preview_font_size, &pfr_data, &pfr_parsed) {
                    // PFR font with size override — rasterize this member's data at requested size
                    if let Some(f) = player.font_manager.rasterize_pfr_at_size(
                        raw, parsed, &font_name, font_style, req_size, &mut player.bitmap_manager,
                    ) {
                        f
                    } else if let Some(br) = bitmap_ref {
                        // Rasterization failed — fall back to member's existing bitmap
                        Rc::new(BitmapFont {
                            bitmap_ref: br,
                            char_width: member_char_width.unwrap_or(8),
                            char_height: member_char_height.unwrap_or(12),
                            grid_columns: member_grid_columns.unwrap_or(16),
                            grid_rows: member_grid_rows.unwrap_or(8),
                            grid_cell_width: member_char_width.unwrap_or(8),
                            grid_cell_height: member_char_height.unwrap_or(12),
                            first_char_num: member_first_char_num.unwrap_or(32),
                            char_offset_x: 0,
                            char_offset_y: 0,
                            font_name: font_name.clone(),
                            font_size: member_font_size,
                            font_style,
                            char_widths: member_char_widths.clone(),
                            pfr_native_size: 0,
                        })
                    } else {
                        return None;
                    }
                } else if let Some(br) = bitmap_ref {
                    // Use member's own bitmap (no size override or no PFR data)
                    Rc::new(BitmapFont {
                        bitmap_ref: br,
                        char_width: member_char_width.unwrap_or(8),
                        char_height: member_char_height.unwrap_or(12),
                        grid_columns: member_grid_columns.unwrap_or(16),
                        grid_rows: member_grid_rows.unwrap_or(8),
                        grid_cell_width: member_char_width.unwrap_or(8),
                        grid_cell_height: member_char_height.unwrap_or(12),
                        first_char_num: member_first_char_num.unwrap_or(32),
                        char_offset_x: 0,
                        char_offset_y: 0,
                        font_name: font_name.clone(),
                        font_size: member_font_size,
                        font_style,
                        char_widths: member_char_widths.clone(),
                        pfr_native_size: 0,
                    })
                } else {
                    // No bitmap_ref, no PFR — try font manager lookup
                    if let Some(f) = player.font_manager.get_font_by_info(&font_info) {
                        Rc::new(f.clone())
                    } else if let Some(f) = player.font_manager.get_system_font() {
                        f
                    } else {
                        return None;
                    }
                };

            // Get system font for rendering character code labels
            let system_font = player.font_manager.get_system_font();

            let font_bitmap = player.bitmap_manager.get_bitmap(font.bitmap_ref)?;

            let cols = 16u16;
            let rows = 16u16;
            let cell_w = font.char_width.max(1);
            let cell_h = font.char_height.max(1);

            // Layout: each cell = grid_line(1px) + label_height + glyph_height
            let label_h: u16 = if system_font.is_some() { 10 } else { 0 };
            let grid_w: u16 = 1;
            let total_cell_w = cell_w + grid_w;
            let total_cell_h = cell_h + label_h + grid_w;
            let width = cols * total_cell_w + grid_w;
            let height = rows * total_cell_h + grid_w;

            let palettes = &player.movie.cast_manager.palettes();
            let mut bitmap = Bitmap::new(
                width, height, 32, 32, 0,
                PaletteRef::BuiltIn(get_system_default_palette()),
            );
            bitmap.fill_relative_rect(0, 0, 0, 0, (255, 255, 255), palettes, 1.0);

            let grid_color = (200, 200, 200);

            // Draw horizontal grid lines
            for row in 0..=rows {
                let y = (row * total_cell_h) as i32;
                bitmap.fill_rect(0, y, width as i32, y + grid_w as i32, grid_color, palettes, 1.0);
            }
            // Draw vertical grid lines
            for col in 0..=cols {
                let x = (col * total_cell_w) as i32;
                bitmap.fill_rect(x, 0, x + grid_w as i32, height as i32, grid_color, palettes, 1.0);
            }

            let draw_params = CopyPixelsParams {
                blend: 100,
                ink: 0,
                color: ColorRef::PaletteIndex(255),
                bg_color: ColorRef::PaletteIndex(0),
                mask_image: None,
                is_text_rendering: true,
                rotation: 0.0,
                skew: 0.0,
                sprite: None,
                original_dst_rect: None,
            };

            for char_code in 0u16..256 {
                let grid_col = char_code % cols;
                let grid_row = char_code / cols;
                let cell_x = (grid_col * total_cell_w + grid_w) as i32;
                let cell_y = (grid_row * total_cell_h + grid_w) as i32;

                // Draw character code label using system font
                if let Some(ref sys_font) = system_font {
                    if let Some(sys_bitmap) = player.bitmap_manager.get_bitmap(sys_font.bitmap_ref) {
                        let label = format!("{}", char_code);
                        let label_params = CopyPixelsParams {
                            blend: 100,
                            ink: 0,
                            color: ColorRef::PaletteIndex(255),
                            bg_color: ColorRef::PaletteIndex(0),
                            mask_image: None,
                            is_text_rendering: true,
                            rotation: 0.0,
                            skew: 0.0,
                            sprite: None,
                            original_dst_rect: None,
                        };
                        bitmap.draw_text(
                            &label,
                            sys_font,
                            sys_bitmap,
                            cell_x,
                            cell_y,
                            label_params,
                            palettes,
                            0,
                            0,
                        );
                    }
                }

                // Draw the character glyph below the label
                let glyph_y = cell_y + label_h as i32;
                bitmap_font_copy_char(
                    &font,
                    font_bitmap,
                    char_code as u8,
                    &mut bitmap,
                    cell_x,
                    glyph_y,
                    palettes,
                    &draw_params,
                );
            }
            Some(bitmap)
        }
        _ => None,
    }
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

/// Get the filmloop's info rect (the authoritative viewport from the Director file).
/// The info rect is stored in the filmloop's member-specific data as:
///   reg_point = (left, top), width = right, height = bottom
/// Returns None if the member isn't a filmloop or the rect has zero dimensions.
pub fn get_filmloop_info_rect(player: &DirPlayer, member_ref: &CastMemberRef) -> Option<IntRect> {
    let member = player.movie.cast_manager.find_member_by_ref(member_ref)?;
    let film_loop = match &member.member_type {
        CastMemberType::FilmLoop(fl) => fl,
        _ => return None,
    };
    let rect = IntRect::from(
        film_loop.info.reg_point.0 as i32,
        film_loop.info.reg_point.1 as i32,
        film_loop.info.width as i32,
        film_loop.info.height as i32,
    );
    if rect.width() > 0 && rect.height() > 0 {
        Some(rect)
    } else {
        None
    }
}

/// Recompute the initial_rect for a filmloop using actual bitmap dimensions.
/// This is more accurate than the precomputed initial_rect because it uses
/// the real cast member dimensions instead of the channel_data dimensions.
pub fn compute_filmloop_initial_rect_with_members(
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

        // Get actual member dimensions and registration point from the cast member.
        // Must match compute_filmloop_initial_rect_with_members which also uses
        // actual bitmap reg_point and dimensions for computing the bounding box.
        let (member_width, member_height, reg_x, reg_y) = match &member.member_type {
            CastMemberType::Bitmap(bm) => {
                let (w, h) = if let Some(bitmap) = player.bitmap_manager.get_bitmap(bm.image_ref) {
                    (bitmap.width as u16, bitmap.height as u16)
                } else {
                    (bm.info.width as u16, bm.info.height as u16)
                };
                (w, h, bm.reg_point.0 as i32, bm.reg_point.1 as i32)
            }
            // For shapes and other non-bitmap members, pos_x/pos_y is the top-left corner.
            // Use (0,0) registration — shapes don't have a registration point like bitmaps.
            _ => (data.width, data.height, 0, 0),
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
        //
        // When bitmap dimensions differ from display dimensions (channel data),
        // scale the registration point proportionally from bitmap space to display space.
        // This matches how Director handles stretched sprites within filmloops.
        let (scaled_reg_x, scaled_reg_y) = if member_width > 0 && member_height > 0
            && (member_width != use_width || member_height != use_height)
        {
            (
                reg_x * use_width as i32 / member_width as i32,
                reg_y * use_height as i32 / member_height as i32,
            )
        } else {
            (reg_x, reg_y)
        };
        let sprite_left = pos_x as i32 - scaled_reg_x;
        let sprite_top = pos_y as i32 - scaled_reg_y;
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
                // Use the display dimensions (from channel data) for dst_rect.
                // When bitmap dimensions differ from display dimensions (e.g. thin strips
                // that Director stretches to fill the display rect), copy_pixels will
                // handle the scaling automatically via its scale_x/scale_y computation.
                let dst_rect = sprite_rect.clone();

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
            CastMemberType::Shape(shape_member) => {
                // Skip rendering shapes with tiny dimensions (blank placeholders or zero-size)
                if data.width <= 1 || data.height <= 1 {
                    continue;
                }

                // Get sprite foreground color from channel data
                let fore_is_rgb = (data.color_flag & 0x1) != 0
                    || data.fore_color_g != 0
                    || data.fore_color_b != 0;

                let sprite_color = if fore_is_rgb {
                    ColorRef::Rgb(data.fore_color, data.fore_color_g, data.fore_color_b)
                } else {
                    ColorRef::PaletteIndex(data.fore_color)
                };

                // Build a temporary sprite for draw_shape_with_sprite
                let mut temp_sprite = Sprite::new(channel_num as usize);
                temp_sprite.color = sprite_color;
                temp_sprite.ink = ((data.ink & 0x7F) / 5) as i32;
                temp_sprite.blend = if data.blend == 255 { 100 } else {
                    ((255.0 - data.blend as f32) * 100.0 / 255.0) as i32
                };

                bitmap.draw_shape_with_sprite(&temp_sprite, &shape_member.shape_info, sprite_rect, &palettes);
            }
            CastMemberType::VectorShape(vector_member) => {
                if data.width <= 1 || data.height <= 1 {
                    continue;
                }
                let mut temp_sprite = Sprite::new(channel_num as usize);
                temp_sprite.ink = ((data.ink & 0x7F) / 5) as i32;
                temp_sprite.blend = if data.blend == 255 { 100 } else {
                    ((255.0 - data.blend as f32) * 100.0 / 255.0) as i32
                };
                let alpha = (temp_sprite.blend as f32 / 100.0).clamp(0.0, 1.0);
                bitmap.draw_vector_shape(vector_member, sprite_rect, &palettes, alpha);
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

        // Use the filmloop's info rect (from the Director file) as the authoritative viewport.
        // This is the rect that Director uses for the filmloop's coordinate space.
        // Fall back to computed rect if info rect is unavailable.
        let initial_rect = get_filmloop_info_rect(player, member_ref)
            .or_else(|| compute_filmloop_initial_rect_with_members(player, member_ref))
            .unwrap_or_else(|| {
                // Fall back to load-time computed initial_rect
                let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                if let Some(member) = member {
                    if let CastMemberType::FilmLoop(film_loop) = &member.member_type {
                        film_loop.initial_rect.clone()
                    } else {
                        IntRect::from(0, 0, 1, 1)
                    }
                } else {
                    IntRect::from(0, 0, 1, 1)
                }
            });

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

    // Composite accumulated trails bitmap onto the cleared stage
    if let Some(trails_bmp) = &player.trails_bitmap {
        if trails_bmp.width == bitmap.width && trails_bmp.height == bitmap.height {
            // Copy non-transparent pixels from trails bitmap onto stage
            let w = bitmap.width as usize;
            let h = bitmap.height as usize;
            for y in 0..h {
                for x in 0..w {
                    let idx = (y * w + x) * 4;
                    if idx + 3 < trails_bmp.data.len() {
                        let alpha = trails_bmp.data[idx + 3];
                        if alpha > 0 {
                            bitmap.data[idx] = trails_bmp.data[idx];
                            bitmap.data[idx + 1] = trails_bmp.data[idx + 1];
                            bitmap.data[idx + 2] = trails_bmp.data[idx + 2];
                            bitmap.data[idx + 3] = trails_bmp.data[idx + 3];
                        }
                    }
                }
            }
        }
    }

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
            CastMemberType::Shape(shape_member) => {
                let sprite = get_score_sprite(&player.movie, score_source, channel_num).unwrap();

                // Skip rendering shapes with tiny dimensions (blank placeholders or zero-size)
                if sprite.width <= 1 || sprite.height <= 1 {
                    continue;
                }

                // Skip rendering shapes that use member 1:1 (placeholder)
                if let Some(member_ref) = &sprite.member {
                    if member_ref.cast_lib == 1 && member_ref.cast_member == 1 {
                        continue;
                    }
                }

                debug!(
                    "  SHAPE RENDER: channel {} member {:?} type {:?} size {}x{} color {:?} bg {:?} ink {} blend {} filled={} lineThick={}",
                    channel_num, sprite.member, shape_member.shape_info.shape_type,
                    sprite.width, sprite.height,
                    sprite.color, sprite.bg_color, sprite.ink, sprite.blend,
                    shape_member.shape_info.fill_type, shape_member.shape_info.line_thickness
                );

                let shape_info = &shape_member.shape_info;
                let rect = get_concrete_sprite_rect(player, sprite);
                let sprite_rect = IntRect::from(
                    rect.left - offset.0,
                    rect.top - offset.1,
                    rect.right - offset.0,
                    rect.bottom - offset.1,
                );
                if offset.0 != 0 || offset.1 != 0 {
                    let mut translated_sprite = sprite.clone();
                    translated_sprite.loc_h -= offset.0;
                    translated_sprite.loc_v -= offset.1;
                    bitmap.draw_shape_with_sprite(&translated_sprite, shape_info, sprite_rect, &palettes);
                } else {
                    bitmap.draw_shape_with_sprite(sprite, shape_info, sprite_rect, &palettes);
                }
            }
            CastMemberType::VectorShape(vector_member) => {
                let sprite = get_score_sprite(&player.movie, score_source, channel_num).unwrap();

                if sprite.width <= 1 || sprite.height <= 1 {
                    continue;
                }

                if let Some(member_ref) = &sprite.member {
                    if member_ref.cast_lib == 1 && member_ref.cast_member == 1 {
                        continue;
                    }
                }

                let rect = get_concrete_sprite_rect(player, sprite);
                let sprite_rect = IntRect::from(
                    rect.left - offset.0,
                    rect.top - offset.1,
                    rect.right - offset.0,
                    rect.bottom - offset.1,
                );
                if offset.0 != 0 || offset.1 != 0 {
                    let mut translated_sprite = sprite.clone();
                    translated_sprite.loc_h -= offset.0;
                    translated_sprite.loc_v -= offset.1;
                    bitmap.draw_vector_shape_with_sprite(&translated_sprite, vector_member, sprite_rect, &palettes);
                } else {
                    bitmap.draw_vector_shape_with_sprite(sprite, vector_member, sprite_rect, &palettes);
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
            CastMemberType::Button(button_member) => {
                let sprite = get_score_sprite(&player.movie, score_source, channel_num).unwrap();
                let sprite_rect = get_concrete_sprite_rect(player, sprite);
                let draw_x = sprite_rect.left;
                let draw_y = sprite_rect.top;
                let draw_w = sprite_rect.width();
                let draw_h = sprite_rect.height();

                if draw_w <= 0 || draw_h <= 0 {
                    continue;
                }

                let button_type = &button_member.button_type;
                let hilite = button_member.hilite;
                let field = &button_member.field;

                // Determine colors based on hilite state
                // Only push buttons invert everything; radio/checkbox keep black text
                let is_push = matches!(button_type, crate::player::cast_member::ButtonType::PushButton);
                let (frame_color, fill_color, text_color_rgb): ((u8,u8,u8),(u8,u8,u8),(u8,u8,u8)) = if hilite && is_push {
                    ((255,255,255), (0,0,0), (255,255,255))
                } else {
                    ((0,0,0), (255,255,255), (0,0,0))
                };

                // Render button to intermediate bitmap, then composite with ink.
                // This handles all ink modes (bgTransparent, addPin, etc.) uniformly.
                let mut temp = Bitmap::new(
                    draw_w as u16, draw_h as u16,
                    32, 32, 0,
                    PaletteRef::BuiltIn(get_system_default_palette()),
                );
                temp.data.fill(0); // Start fully transparent
                temp.use_alpha = true;

                // For push buttons: draw text FIRST onto transparent bitmap to preserve
                // AA alpha, then fill remaining transparent pixels with the fill color,
                // then draw the border on top. This prevents native text AA from blending
                // with the white fill (which would create near-white pixels that survive
                // ink 36 color-keying).
                // For checkbox/radio: draw chrome first (no fill behind text area).

                // Draw non-pushbutton chrome first (checkbox/radio don't have fill behind text)
                // Chrome is positioned at the top-left, aligned with the first line of text
                if !is_push {
                    match button_type {
                        crate::player::cast_member::ButtonType::CheckBox => {
                            let box_y = 0;
                            temp.fill_rect(0, box_y, 10, box_y + 1, (0, 0, 0), &palettes, 1.0);
                            temp.fill_rect(0, box_y + 9, 10, box_y + 10, (0, 0, 0), &palettes, 1.0);
                            temp.fill_rect(0, box_y, 1, box_y + 10, (0, 0, 0), &palettes, 1.0);
                            temp.fill_rect(9, box_y, 10, box_y + 10, (0, 0, 0), &palettes, 1.0);
                            temp.fill_rect(1, box_y + 1, 9, box_y + 9, (255, 255, 255), &palettes, 1.0);
                            if hilite {
                                for i in 1..9 {
                                    temp.fill_rect(i, box_y + i, i + 1, box_y + i + 1, (0, 0, 0), &palettes, 1.0);
                                    temp.fill_rect(9 - i, box_y + i, 10 - i, box_y + i + 1, (0, 0, 0), &palettes, 1.0);
                                }
                            }
                        }
                        crate::player::cast_member::ButtonType::RadioButton => {
                            let base_y = 0;
                            let circle_points: &[(i32, i32)] = &[
                                (4, 0), (5, 0), (6, 0),
                                (3, 1), (7, 1),
                                (2, 2), (8, 2),
                                (1, 3), (9, 3),
                                (0, 4), (10, 4),
                                (0, 5), (10, 5),
                                (0, 6), (10, 6),
                                (1, 7), (9, 7),
                                (2, 8), (8, 8),
                                (3, 9), (7, 9),
                                (4, 10), (5, 10), (6, 10),
                            ];
                            for &(px, py) in circle_points {
                                temp.fill_rect(px, base_y + py, px + 1, base_y + py + 1, (0, 0, 0), &palettes, 1.0);
                            }
                            if hilite {
                                temp.fill_rect(4, base_y + 3, 7, base_y + 4, (0, 0, 0), &palettes, 1.0);
                                temp.fill_rect(3, base_y + 4, 8, base_y + 5, (0, 0, 0), &palettes, 1.0);
                                temp.fill_rect(3, base_y + 5, 8, base_y + 6, (0, 0, 0), &palettes, 1.0);
                                temp.fill_rect(3, base_y + 6, 8, base_y + 7, (0, 0, 0), &palettes, 1.0);
                                temp.fill_rect(4, base_y + 7, 7, base_y + 8, (0, 0, 0), &palettes, 1.0);
                            }
                        }
                        _ => {}
                    }
                }

                // Draw text label into temp bitmap
                let chrome_offset_x = match button_type {
                    crate::player::cast_member::ButtonType::CheckBox => 13, // 10px box + 3px gap
                    crate::player::cast_member::ButtonType::RadioButton => 14, // 11px circle + 3px gap
                    _ => 0,
                };

                let font_opt = get_or_load_font_with_id(
                    &mut player.font_manager,
                    &player.movie.cast_manager,
                    &field.font,
                    Some(field.font_size),
                    None,
                    field.font_id,
                );

                let text_area_x = chrome_offset_x;
                let text_area_w = draw_w - chrome_offset_x;

                let is_pfr_font = font_opt.as_ref().map_or(false, |f| f.char_widths.is_some());

                if let (true, Some(font)) = (is_pfr_font, &font_opt) {
                    let font_bitmap = player.bitmap_manager.get_bitmap(font.bitmap_ref).unwrap();

                    let wrapped_lines = Bitmap::wrap_text_lines(&field.text, font, text_area_w);
                    let line_h = font.char_height as i32 + field.fixed_line_space as i32;
                    let total_text_h = (wrapped_lines.len() as i32) * line_h;
                    // Push buttons center text vertically; radio/checkbox start at top
                    let text_y = if is_push {
                        ((draw_h - total_text_h) / 2).max(0)
                    } else {
                        0
                    };

                    let text_params = CopyPixelsParams {
                        blend: 100,
                        ink: 36, // bg transparent for text onto temp
                        color: ColorRef::Rgb(text_color_rgb.0, text_color_rgb.1, text_color_rgb.2),
                        bg_color: ColorRef::Rgb(255, 255, 255),
                        mask_image: None,
                        is_text_rendering: true,
                        rotation: 0.0,
                        skew: 0.0,
                        sprite: None,
                        original_dst_rect: None,
                    };

                    temp.draw_text_wrapped(
                        &field.text,
                        font,
                        font_bitmap,
                        text_area_x,
                        text_y,
                        text_area_w,
                        &field.alignment,
                        text_params,
                        &palettes,
                        field.fixed_line_space,
                        field.top_spacing,
                    );
                } else {
                    // Native Canvas2D rendering for system fonts (Arial etc.)
                    let font_name = if field.font.is_empty() { "Arial".to_string() } else { field.font.clone() };
                    let font_size = if field.font_size > 0 { field.font_size as i32 } else { 12 };
                    let text_color = ((text_color_rgb.0 as u32) << 16)
                        | ((text_color_rgb.1 as u32) << 8)
                        | (text_color_rgb.2 as u32);

                    let span = StyledSpan {
                        text: field.text.clone(),
                        style: HtmlStyle {
                            font_face: Some(font_name),
                            font_size: Some(font_size),
                            color: Some(text_color),
                            ..HtmlStyle::default()
                        },
                    };

                    let alignment = match field.alignment.to_lowercase().as_str() {
                        "center" | "#center" => TextAlignment::Center,
                        "right" | "#right" => TextAlignment::Right,
                        _ => TextAlignment::Left,
                    };

                    // Push buttons center text vertically; radio/checkbox start at top
                    let text_y = if is_push {
                        ((draw_h - font_size) / 2).max(0)
                    } else {
                        0
                    };

                    if let Err(e) = FontMemberHandlers::render_native_text_to_bitmap(
                        &mut temp,
                        &[span],
                        text_area_x,
                        text_y,
                        text_area_w,
                        draw_h,
                        alignment,
                        text_area_w,
                        true,
                        None,
                        field.fixed_line_space,
                        field.top_spacing,
                        0,
                    ) {
                        console_warn!("Native text render error for Button: {:?}", e);
                    }
                }

                // For push buttons: fill background and draw border.
                // For matte-like inks (bgTransparent 36, Matte 8, Not Ghost 7),
                // skip the fill and rely on the alpha channel instead of color-keying.
                // This avoids white fringe from AA text pixels that don't exactly match bgColor.
                let use_alpha_matte = sprite.ink == 36 || sprite.ink == 8 || sprite.ink == 7;

                if is_push {
                    if !use_alpha_matte {
                        // Fill transparent interior pixels with fill color (inset 1px for border)
                        for y in 1..(draw_h - 1) {
                            for x in 1..(draw_w - 1) {
                                let idx = ((y * draw_w + x) * 4) as usize;
                                if idx + 3 < temp.data.len() && temp.data[idx + 3] == 0 {
                                    temp.data[idx] = fill_color.0;
                                    temp.data[idx + 1] = fill_color.1;
                                    temp.data[idx + 2] = fill_color.2;
                                    temp.data[idx + 3] = 255;
                                }
                            }
                        }
                    }
                    // Border on top
                    temp.fill_rect(2, 0, draw_w - 2, 1, frame_color, &palettes, 1.0);
                    temp.fill_rect(2, draw_h - 1, draw_w - 2, draw_h, frame_color, &palettes, 1.0);
                    temp.fill_rect(0, 2, 1, draw_h - 2, frame_color, &palettes, 1.0);
                    temp.fill_rect(draw_w - 1, 2, draw_w, draw_h - 2, frame_color, &palettes, 1.0);
                    // Corner pixels
                    temp.fill_rect(1, 0, 2, 1, frame_color, &palettes, 1.0);
                    temp.fill_rect(draw_w - 2, 0, draw_w - 1, 1, frame_color, &palettes, 1.0);
                    temp.fill_rect(0, 1, 1, 2, frame_color, &palettes, 1.0);
                    temp.fill_rect(draw_w - 1, 1, draw_w, 2, frame_color, &palettes, 1.0);
                    temp.fill_rect(1, draw_h - 1, 2, draw_h, frame_color, &palettes, 1.0);
                    temp.fill_rect(draw_w - 2, draw_h - 1, draw_w - 1, draw_h, frame_color, &palettes, 1.0);
                    temp.fill_rect(0, draw_h - 2, 1, draw_h - 1, frame_color, &palettes, 1.0);
                    temp.fill_rect(draw_w - 1, draw_h - 2, draw_w, draw_h - 1, frame_color, &palettes, 1.0);
                }

                // Composite temp bitmap onto stage with sprite's ink.
                // For matte-like inks, use Matte (ink 8) to composite via alpha channel
                // instead of color-keying, which avoids AA fringe artifacts.
                let compositing_ink = if use_alpha_matte { 8 } else { sprite.ink as i32 };

                let mut ink_params = HashMap::new();
                ink_params.insert("blend".into(), Datum::Int(sprite.blend as i32));
                ink_params.insert("ink".into(), Datum::Int(compositing_ink));
                ink_params.insert("color".into(), Datum::ColorRef(sprite.color.clone()));
                ink_params.insert("bgColor".into(), Datum::ColorRef(sprite.bg_color.clone()));

                bitmap.copy_pixels(
                    &palettes,
                    &temp,
                    IntRect::from(draw_x, draw_y, draw_x + draw_w, draw_y + draw_h),
                    IntRect::from_tuple((0, 0, draw_w, draw_h)),
                    &ink_params,
                    None,
                );
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
                            draw_y,
                            draw_w,
                            draw_h,
                            alignment,
                            draw_w,
                            text_member.word_wrap,
                            None, // Color is now in the spans
                            text_member.fixed_line_space,
                            text_member.top_spacing,
                            text_member.bottom_spacing,
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
                // Use the computed initial_rect (bounding box of all sprites across all frames)
                // instead of the info header rect. ScummVM also recomputes the initial rect
                // from actual sprite data rather than trusting the header values.
                let computed_initial_rect = film_loop.initial_rect.clone();

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
                        computed_initial_rect,
                        film_loop.current_frame,
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

    // Accumulate trails sprites into the persistent trails bitmap
    if matches!(score_source, ScoreRef::Stage) {
        // Check if any sprite in the current frame has trails
        let mut has_trails = false;
        let frame_num = player.movie.current_frame;
        let channels: Vec<i16> = player.movie.score
            .get_sorted_channels(frame_num)
            .iter()
            .map(|x| x.number as i16)
            .collect();
        for &ch in &channels {
            if let Some(sprite) = player.movie.score.get_sprite(ch) {
                if sprite.trails {
                    has_trails = true;
                    break;
                }
            }
        }

        if has_trails {
            // Ensure trails bitmap exists and is the right size
            let bw = bitmap.width;
            let bh = bitmap.height;
            if player.trails_bitmap.is_none()
                || player.trails_bitmap.as_ref().unwrap().width != bw
                || player.trails_bitmap.as_ref().unwrap().height != bh
            {
                let mut trails_bmp = Bitmap::new(
                    bw,
                    bh,
                    32,
                    32,
                    0,
                    PaletteRef::BuiltIn(get_system_default_palette()),
                );
                // Start fully transparent
                for pixel in trails_bmp.data.chunks_exact_mut(4) {
                    pixel[0] = 0;
                    pixel[1] = 0;
                    pixel[2] = 0;
                    pixel[3] = 0;
                }
                player.trails_bitmap = Some(trails_bmp);
            }

            // For each trails sprite, copy its bounding rect from the rendered bitmap to trails_bitmap
            for &ch in &channels {
                if let Some(sprite) = player.movie.score.get_sprite(ch) {
                    if sprite.trails && sprite.member.is_some() {
                        let sprite_rect = get_concrete_sprite_rect(player, sprite);
                        let x1 = sprite_rect.left.max(0) as usize;
                        let y1 = sprite_rect.top.max(0) as usize;
                        let x2 = (sprite_rect.right as usize).min(bw as usize);
                        let y2 = (sprite_rect.bottom as usize).min(bh as usize);
                        let w = bw as usize;

                        if let Some(trails_bmp) = &mut player.trails_bitmap {
                            for y in y1..y2 {
                                for x in x1..x2 {
                                    let idx = (y * w + x) * 4;
                                    if idx + 3 < bitmap.data.len() {
                                        trails_bmp.data[idx] = bitmap.data[idx];
                                        trails_bmp.data[idx + 1] = bitmap.data[idx + 1];
                                        trails_bmp.data[idx + 2] = bitmap.data[idx + 2];
                                        trails_bmp.data[idx + 3] = 255; // Mark as opaque
                                    }
                                }
                            }
                        }
                    }
                }
            }
        } else {
            // No trails sprites in this frame - clear the trails bitmap
            player.trails_bitmap = None;
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
    let is_picking_sprite = player.picking_mode
        || (player.keyboard_manager.is_alt_down()
            && (player.keyboard_manager.is_control_down() || player.keyboard_manager.is_command_down()));
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
                ink: 0,
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

        let member_ref = self.preview_member_ref.as_ref().unwrap().clone();
        let bitmap = render_preview_bitmap(player, &member_ref, self.preview_font_size);
        if let Some(bitmap) = bitmap {
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
            if let Ok(image_data) = image_data {
                let _ = self.preview_ctx2d.put_image_data(&image_data, 0.0, 0.0);
            }
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

    fn set_preview_font_size(&mut self, size: Option<u16>) {
        self.preview_font_size = size;
    }

    fn preview_font_size(&self) -> Option<u16> {
        self.preview_font_size
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
pub fn player_set_preview_font_size(size: u16) -> Result<(), JsValue> {
    use crate::rendering_gpu::Renderer;
    with_renderer_mut(|renderer_lock| {
        if let Some(dynamic) = renderer_lock {
            dynamic.set_preview_font_size(if size > 0 { Some(size) } else { None });
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
    let style = canvas.style();
    // Nearest-neighbor scaling when CSS size differs from canvas resolution
    style.set_property("image-rendering", "pixelated").unwrap_or(());
    style.set_property("image-rendering", "-moz-crisp-edges").unwrap_or(());
    style.set_property("image-rendering", "crisp-edges").unwrap_or(());
    // Disable ClearType / subpixel font smoothing - force grayscale AA
    style.set_property("-webkit-font-smoothing", "none").unwrap_or(());
    style.set_property("-moz-osx-font-smoothing", "grayscale").unwrap_or(());
    style.set_property("font-smooth", "never").unwrap_or(());
    // Disable text anti-aliasing optimizations
    style.set_property("text-rendering", "optimizeSpeed").unwrap_or(());
    // Force no backface-visibility optimization (can cause blurring on some GPUs)
    style.set_property("backface-visibility", "hidden").unwrap_or(());
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
        preview_font_size: None,
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
