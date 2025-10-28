use std::{
    borrow::{Borrow, BorrowMut},
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
};

use async_std::task::spawn_local;
use chrono::Local;
use itertools::Itertools;
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
        sprite::{CursorRef, Sprite},
        DirPlayer, PLAYER_OPT,
    },
    utils::log_i,
};

use crate::player::cast_manager::CastManager;
use crate::player::font::BitmapFont;
use crate::player::font::FontManager;
use crate::player::handlers::datum_handlers::cast_member::font::FontMemberHandlers;

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
    if font_name.is_empty() || font_name == "System" {
        return font_manager.get_system_font();
    }

    let cache_key = format!(
        "{}_{}_{}",
        font_name,
        font_size.unwrap_or(0),
        font_style.unwrap_or(0)
    );

    console::log_1(
        &format!(
            "🔍 Looking for font: '{}' (key: '{}')",
            font_name, cache_key
        )
        .into(),
    );

    if let Some(font) = font_manager.font_cache.get(&cache_key) {
        console::log_1(&format!("✓ Found in cache: '{}'", cache_key).into());
        return Some(Rc::clone(font));
    }

    if let Some(font) = font_manager.font_cache.get(font_name) {
        console::log_1(&format!("✓ Found by name: '{}'", font_name).into());
        return Some(Rc::clone(font));
    }

    console::log_1(
        &format!(
            "⚠️  Font '{}' not in cache, attempting to load from cast...",
            font_name
        )
        .into(),
    );

    if let Some(loaded_font) =
        font_manager.get_font_with_cast(font_name, Some(cast_manager), font_size, font_style)
    {
        console::log_1(&format!("✅ Loaded font '{}' from cast", font_name).into());
        return Some(loaded_font);
    }

    console::log_1(
        &format!(
            "❌ Could not find font '{}', falling back to system font",
            font_name
        )
        .into(),
    );

    font_manager.get_system_font()
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
    let palettes = player.movie.cast_manager.palettes();
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
        let score = match score_source {
            ScoreRef::Stage => &player.movie.score,
            ScoreRef::FilmLoop(member_ref) => {
                let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                if member.is_none() {
                    return;
                }
                let member = member.unwrap();
                match &member.member_type {
                    CastMemberType::FilmLoop(film_loop_member) => &film_loop_member.score,
                    _ => return,
                }
            }
        };
        score
            .get_sorted_channels()
            .iter()
            .map(|x| x.number as i16)
            .collect_vec()
    };

    for channel_num in sorted_channel_numbers {
        let member_ref = {
            let score = get_score(&player.movie, score_source).unwrap();
            let sprite = score.get_sprite(channel_num).unwrap();
            sprite.member.as_ref().unwrap().clone()
        };
        let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
        if member.is_none() {
            continue;
        }
        let member = member.unwrap();
        match &member.member_type {
            CastMemberType::Bitmap(bitmap_member) => {
                let sprite_rect = {
                    let sprite =
                        get_score_sprite(&player.movie, score_source, channel_num).unwrap();
                    get_concrete_sprite_rect(player, sprite).clone()
                };
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
                let src_rect = IntRect::from(0, 0, sprite.width as i32, sprite.height as i32);
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

                let mut params = CopyPixelsParams {
                    blend: sprite.blend as i32,
                    ink: sprite.ink as u32,
                    color: sprite.color.clone(),
                    bg_color: sprite.bg_color.clone(),
                    mask_image: None,
                };
                if let Some(mask) = mask {
                    let mask_bitmap: &BitmapMask = mask.borrow();
                    params.mask_image = Some(mask_bitmap);
                }
                bitmap.copy_pixels_with_params(&palettes, &src_bitmap, dst_rect, src_rect, &params);
            }
            CastMemberType::Shape(_) => {
                let sprite = get_score_sprite(&player.movie, score_source, channel_num).unwrap();
                let sprite_rect = get_concrete_sprite_rect(player, sprite);
                let dst_rect = sprite_rect;
                bitmap.fill_rect(
                    dst_rect.left,
                    dst_rect.top,
                    dst_rect.right,
                    dst_rect.bottom,
                    resolve_color_ref(
                        &palettes,
                        &sprite.color,
                        &PaletteRef::BuiltIn(get_system_default_palette()),
                        bitmap.original_bit_depth,
                    ),
                    &palettes,
                    sprite.blend as f32 / 100.0,
                );
            }
            CastMemberType::Field(field_member) => {
                let sprite = get_score_sprite(&player.movie, score_source, channel_num).unwrap();

                let font_opt = get_or_load_font(
                    &mut player.font_manager,
                    &player.movie.cast_manager,
                    &field_member.font,
                    Some(field_member.font_size),
                    None,
                );

                if let Some(font) = font_opt {
                    let font_bitmap = player.bitmap_manager.get_bitmap(font.bitmap_ref).unwrap();

                    let params = CopyPixelsParams {
                        blend: sprite.blend as i32,
                        ink: sprite.ink as u32,
                        color: sprite.color.clone(),
                        bg_color: sprite.bg_color.clone(),
                        mask_image: None,
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

                let font_opt = get_or_load_font(
                    &mut player.font_manager,
                    &player.movie.cast_manager,
                    &text_member.font,
                    Some(text_member.font_size),
                    None,
                );

                if let Some(font) = font_opt {
                    let font_bitmap = player.bitmap_manager.get_bitmap(font.bitmap_ref).unwrap();

                    let params = CopyPixelsParams {
                        blend: sprite.blend as i32,
                        ink: sprite.ink as u32,
                        color: sprite.color.clone(),
                        bg_color: sprite.bg_color.clone(),
                        mask_image: None,
                    };

                    bitmap.draw_text(
                        &text_member.text,
                        &font,
                        font_bitmap,
                        sprite.loc_h,
                        sprite.loc_v,
                        params,
                        &palettes,
                        text_member.fixed_line_space,
                        text_member.top_spacing,
                    );
                }
            }
            CastMemberType::FilmLoop(film_loop) => {
                let sprite = get_score_sprite(&player.movie, score_source, channel_num).unwrap();
                let sprite_rect = get_concrete_sprite_rect(player, sprite);
                let dest_x = sprite.loc_h;
                let dest_y = sprite.loc_v;
                render_score_to_bitmap(
                    player,
                    // &mut player.bitmap_manager,
                    &ScoreRef::FilmLoop(member_ref.clone()),
                    bitmap,
                    debug_sprite_num,
                    sprite_rect,
                    // IntRect::from_size(dest_x, dest_y, sprite_rect.width(), sprite_rect.height())
                )
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
}

thread_local! {
    pub static RENDERER_LOCK: RefCell<Option<PlayerCanvasRenderer>> = RefCell::new(None);
}

#[allow(dead_code)]
pub fn with_canvas_renderer_ref<F, R>(f: F) -> R
where
    F: FnOnce(&PlayerCanvasRenderer) -> R,
{
    RENDERER_LOCK.with(|renderer_lock| {
        let renderer = renderer_lock.borrow();
        f(renderer.as_ref().unwrap())
    })
}

pub fn with_canvas_renderer_mut<F, R>(f: F) -> R
where
    F: FnOnce(&mut Option<PlayerCanvasRenderer>) -> R,
{
    RENDERER_LOCK.with_borrow_mut(|renderer_lock| f(renderer_lock))
}

#[wasm_bindgen]
pub fn player_set_preview_member_ref(cast_lib: i32, cast_num: i32) -> Result<(), JsValue> {
    with_canvas_renderer_mut(|renderer| {
        renderer.as_mut().unwrap().preview_member_ref = Some(CastMemberRef {
            cast_lib,
            cast_member: cast_num,
        });
    });
    Ok(())
}

#[wasm_bindgen]
pub fn player_set_debug_selected_channel(channel_num: i16) -> Result<(), JsValue> {
    with_canvas_renderer_mut(|renderer| {
        renderer.as_mut().unwrap().debug_selected_channel_num = Some(channel_num);
    });
    JsApi::dispatch_channel_changed(channel_num);
    Ok(())
}

#[wasm_bindgen]
pub fn player_set_preview_parent(parent_selector: &str) -> Result<(), JsValue> {
    if parent_selector.is_empty() {
        with_canvas_renderer_mut(|renderer| {
            renderer
                .as_mut()
                .unwrap()
                .set_preview_container_element(None);
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

    with_canvas_renderer_mut(|renderer| {
        renderer
            .as_mut()
            .unwrap()
            .set_preview_container_element(Some(parent_element));
    });

    Ok(())
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
    with_canvas_renderer_mut(|renderer_lock| {
        if renderer_lock.is_none() {
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

            let canvas_size = reserve_player_ref(|player| {
                (
                    player.movie.rect.width() as u32,
                    player.movie.rect.height() as u32,
                )
            });

            canvas.set_width(canvas_size.0);
            canvas.set_height(canvas_size.1);

            preview_canvas.set_width(1);
            preview_canvas.set_height(1);

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

            preview_canvas
                .style()
                .set_property("image-rendering", "pixelated")
                .unwrap_or(());
            preview_canvas
                .style()
                .set_property("image-rendering", "-moz-crisp-edges")
                .unwrap_or(());
            preview_canvas
                .style()
                .set_property("image-rendering", "crisp-edges")
                .unwrap_or(());

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

            let renderer = PlayerCanvasRenderer {
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
            };

            *renderer_lock = Some(renderer);
            spawn_local(async {
                run_draw_loop().await;
            });
        }
    });

    with_canvas_renderer_mut(|renderer| {
        renderer
            .as_mut()
            .unwrap()
            .set_container_element(container_element);
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
            with_canvas_renderer_mut(|renderer| {
                let renderer = renderer.as_mut().unwrap();
                renderer.draw_frame(&mut player);
                renderer.draw_preview_frame(&mut player);
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
