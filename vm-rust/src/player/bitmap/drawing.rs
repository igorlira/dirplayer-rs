use std::{collections::HashMap, rc::Rc};

use rgb565::Rgb565;

use crate::{director::lingo::datum::Datum, player::{font::{bitmap_font_copy_char, BitmapFont}, geometry::IntRect, handlers::datum_handlers::cast_member::text::TextMemberHandlers, reserve_player_mut, sprite::ColorRef}};

use super::{bitmap::{resolve_color_ref, Bitmap, ProceduralBitmapType}, mask::BitmapMask, palette_map::PaletteMap};

pub struct CopyPixelsParams {
    pub blend: i32,
    pub ink: u32,
    pub color: ColorRef,
    pub bg_color: ColorRef,
    pub mask_image: Option<Rc<BitmapMask>>,
}

impl CopyPixelsParams {
    pub const fn default(bitmap: &Bitmap) -> CopyPixelsParams {
        CopyPixelsParams {
            blend: 100,
            ink: 0,
            color: bitmap.get_fg_color_ref(),
            bg_color: bitmap.get_bg_color_ref(),
            mask_image: None,
        }
    }
}

fn blend_alpha(dst: u8, src: u8, alpha: f32) -> u8 {
    (src as f32 * alpha + dst as f32 * (1.0 - alpha)) as u8
}

fn blend_color_alpha(dst: (u8, u8, u8), src: (u8, u8, u8), alpha: f32) -> (u8, u8, u8) {
    if alpha == 0.0 {
        return dst;
    } else if alpha == 1.0 {
        return src;
    }
    let r = blend_alpha(dst.0, src.0, alpha);
    let g = blend_alpha(dst.1, src.1, alpha);
    let b = blend_alpha(dst.2, src.2, alpha);
    (r, g, b)
}

pub fn should_matte_sprite(ink: u32) -> bool {
    ink == 36 || ink == 33 || ink == 41 || ink == 8 || ink == 7
}

fn blend_pixel(
    dst: (u8, u8, u8), 
    src: (u8, u8, u8), 
    ink: u32,
    bg_color: (u8, u8, u8),
    alpha: f32,
) -> (u8, u8, u8) {
    match ink {
        0 => {
            // Copy
            blend_color_alpha(dst, src, alpha)
        }
        7 => {
            // Not Ghost
            // TODO
            blend_color_alpha(dst, src, alpha)
        }
        8 => {
            // Matte
            // TODO
            blend_color_alpha(dst, src, alpha)
        }
        9 => {
            // Mask
            // TODO
            blend_color_alpha(dst, src, alpha)
        }
        33 => {
            // Add pin
            if src == bg_color {
                dst
            } else {
                let r = dst.0 as i32 + src.0 as i32;
                let g = dst.1 as i32 + src.1 as i32;
                let b = dst.2 as i32 + src.2 as i32;
                let r = r.min(255).max(0) as u8;
                let g = g.min(255).max(0) as u8;
                let b = b.min(255).max(0) as u8;
                (r, g, b)
            }
        }
        36 => {
            // Background transparent
            if src == bg_color {
                dst
            } else {
                blend_color_alpha(dst, src, alpha)
            }
        }
        41 => {
            // Darken
            // TODO
            // bg_color
            let r = (src.0 as f32 / 255.0) * (bg_color.0 as f32 / 255.0) * 255.0;
            let g = (src.1 as f32 / 255.0) * (bg_color.1 as f32 / 255.0) * 255.0;
            let b = (src.2 as f32 / 255.0) * (bg_color.2 as f32 / 255.0) * 255.0;
            let color = (r as u8, g as u8, b as u8);
            blend_color_alpha(dst, color, alpha)
        }
        _ => blend_color_alpha(dst, src, alpha),
    }
}

impl Bitmap {
    pub fn set_pixel(&self, x: i32, y: i32, color: (u8, u8, u8), palettes: &PaletteMap) {
        if x < 0 || y < 0 || x >= self.width() as i32 || y >= self.height() as i32 {
            return;
        }
        let mut data = self.data.borrow_mut();
        self.matte.replace(None); // TODO draw on matte instead
        let (r, g, b) = color;
        let x = x as usize;
        let y = y as usize;
        if x < self.width() as usize && y < self.height() as usize {
            let bytes_per_pixel = self.bit_depth as usize / 8;
            let index = (y * self.width() as usize + x) * bytes_per_pixel;
            match self.bit_depth {
                1 => {
                    let bit_index = y * self.width() as usize + x;
                    let byte_index = bit_index / 8;
                    let bit_offset = bit_index % 8;
                    let value = data[byte_index];
                    let mask = 1 << (7 - bit_offset);
                    let value = if r > 127 || g > 127 || b > 127 {
                        value | mask
                    } else {
                        value & !mask
                    };
                    data[byte_index] = value;
                }
                4 => {
                    let bit_index = (y * self.width() as usize + x) * 4;
                    let index = bit_index / 8;
                    let value = data[index];
                    
                    let own_palette = &self.palette_ref;
                    let mut result_index = 0;
                    let mut result_distance = 0;
                    for index in 0..=255 {
                        let palette_color = resolve_color_ref(palettes, &ColorRef::PaletteIndex(index as u8), &own_palette);
                        let distance = (r as i32 - palette_color.0 as i32).abs() + (g as i32 - palette_color.1 as i32).abs() + (b as i32 - palette_color.2 as i32).abs();
                        if index == 0 || distance < result_distance {
                            result_index = index;
                            result_distance = distance;
                        }
                    }
                    
                    let left = value >> 4;
                    let right = value & 0x0F;

                    let left = if x % 2 == 0 {
                        result_index
                    } else {
                        left
                    };
                    let right = if x % 2 == 1 {
                        result_index
                    } else {
                        right
                    };

                    let value = (left << 4) | right;
                    data[index] = value;
                }
                8 => {
                    let own_palette = &self.palette_ref;
                    let mut result_index = 0;
                    let mut result_distance = 0;
                    for index in 0..=255 {
                        let palette_color = resolve_color_ref(palettes, &ColorRef::PaletteIndex(index as u8), &own_palette);
                        let distance = (r as i32 - palette_color.0 as i32).abs() + (g as i32 - palette_color.1 as i32).abs() + (b as i32 - palette_color.2 as i32).abs();
                        if index == 0 || distance < result_distance {
                            result_index = index;
                            result_distance = distance;
                        }
                    }
                    data[index] = result_index;
                }
                16 => {
                    let r = r as f32 * 31.0 / 255.0;
                    let g = g as f32 * 63.0 / 255.0;
                    let b = b as f32 * 31.0 / 255.0;
                    let value = Rgb565::pack_565((r as u8, g as u8, b as u8));
                    let bytes = value.to_le_bytes();
                    data[index] = bytes[0];
                    data[index + 1] = bytes[1];
                }
                32 => {
                    data[index] = r;
                    data[index + 1] = g;
                    data[index + 2] = b;
                    data[index + 3] = 0xFF;
                }
                _ => {
                    // TODO: Should this be logged?
                    // panic!("Unsupported bit depth fot set_pixel: {}", self.bit_depth)
                }
            }
        }
    }

    pub fn get_pixel_color_ref(&self, x: u16, y: u16) -> ColorRef {
        self.render_if_needed();

        let x = x as usize;
        let y = y as usize;
        if x >= self.width() as usize || y >= self.height() as usize {
            return self.get_bg_color_ref();
        }
        let data = self.data.borrow();
        
        match self.bit_depth {
            4 => {
                let bit_index = (y * self.width() as usize + x) * 4;
                let index = bit_index / 8;
                let value = data[index];
                if x % 2 == 0 {
                    let left = value >> 4;
                    let left = (left as f32 / 15.0 * 255.0) as u8;
                    let left = left;
                    ColorRef::PaletteIndex(left)
                } else {
                    let right = value & 0x0F;
                    let right = (right as f32 / 15.0 * 255.0) as u8;
                    let right = right;
                    ColorRef::PaletteIndex(right)
                }
            }
            8 => {
                let index = y * self.width() as usize + x;
                ColorRef::PaletteIndex(data[index])
            }
            16 => {
                let index = (y * self.width() as usize + x) * 2;
                let value = u16::from_le_bytes([data[index], data[index + 1]]);
                let (red, green, blue) = Rgb565::unpack_565(value);
                let red = (red as f32 / 31.0 * 255.0) as u8;
                let green = (green as f32 / 63.0 * 255.0) as u8;
                let blue = (blue as f32 / 31.0 * 255.0) as u8;
                ColorRef::Rgb(red, green, blue)
            }
            32 => {
                let bytes_per_pixel = 4;
                let index = (y * self.width() as usize + x) * bytes_per_pixel as usize;
                ColorRef::Rgb(data[index], data[index + 1], data[index + 2])
            }
            _ => {
                self.get_bg_color_ref()
                // panic!("Unsupported bit depth: {}", self.bit_depth)
            }
        }
    }

    pub fn get_pixel_color(&self, palettes: &PaletteMap, x: u16, y: u16) -> (u8, u8, u8) {
        let color_ref = self.get_pixel_color_ref(x, y);
        resolve_color_ref(palettes, &color_ref, &self.palette_ref)
    }

    pub const fn has_palette(&self) -> bool {
        self.bit_depth != 16 && self.bit_depth != 32
    }

    pub const fn get_bg_color_ref(&self) -> ColorRef {
        if self.has_palette() {
            ColorRef::PaletteIndex(0)
        } else {
            ColorRef::Rgb(255, 255, 255)
        }
    }

    pub const fn get_fg_color_ref(&self) -> ColorRef {
        if self.has_palette() {
            ColorRef::PaletteIndex(255)
        } else {
            ColorRef::Rgb(0, 0, 0)
        }
    }

    pub fn stroke_sized_rect(&self, left: i32, top: i32, width: i32, height: i32, color: (u8, u8, u8), palettes: &PaletteMap, alpha: f32) {
        let left = left.max(0) as i32;
        let top = top.max(0) as i32;
        let right = (left + width) as i32;
        let bottom = (top + height) as i32;
        self.stroke_rect(left, top, right, bottom, color, palettes, alpha);
    }

    pub fn stroke_rect(&self, x1: i32, y1: i32, x2: i32, y2: i32, color: (u8, u8, u8), palettes: &PaletteMap, alpha: f32) {
        let left = x1;
        let top = y1;
        let right = x2 - 1;
        let bottom = y2 - 1;

        for x in x1..x2 {
            let top_color = self.get_pixel_color(palettes, x as u16, top as u16);
            let bottom_color = self.get_pixel_color(palettes, x as u16, bottom as u16);
            let blended_top = blend_color_alpha(top_color, color, alpha);
            let blended_bottom = blend_color_alpha(bottom_color, color, alpha);
            self.set_pixel(x as i32, top as i32, blended_top, palettes);
            self.set_pixel(x as i32, bottom as i32, blended_bottom, palettes);
        }
        for y in y1..y2 {
            let left_color = self.get_pixel_color(palettes, left as u16, y as u16);
            let right_color = self.get_pixel_color(palettes, right as u16, y as u16);
            let blended_left = blend_color_alpha(left_color, color, alpha);
            let blended_right = blend_color_alpha(right_color, color, alpha);
            self.set_pixel(left as i32, y as i32, blended_left, palettes);
            self.set_pixel(right as i32, y as i32, blended_right, palettes);
        }
    }

    pub fn clear_rect(&self, x1: i32, y1: i32, x2: i32, y2: i32, color: (u8, u8, u8), palettes: &PaletteMap) {
        for y in y1..y2 {
            for x in x1..x2 {
                self.set_pixel(x, y, color, palettes);
            }
        }
    }

    pub fn fill_relative_rect(&self, left: i32, top: i32, right: i32, bottom: i32, color: (u8, u8, u8), palettes: &PaletteMap, alpha: f32) {
        let left = left.max(0);
        let top = top.max(0);
        let right = right.min(self.width() as i32 - 1);
        let bottom = bottom.min(self.height() as i32 - 1);
        
        let x1 = left;
        let y1 = top;
        let x2 = self.width() as i32 - right;
        let y2 = self.height() as i32 - bottom;

        self.fill_rect(x1, y1, x2, y2, color, palettes, alpha);
    }

    pub fn fill_rect(&self, x1: i32, y1: i32, x2: i32, y2: i32, color: (u8, u8, u8), palettes: &PaletteMap, alpha: f32) {
        if alpha == 0.0 {
            return;
        }
        for y in y1..y2 {
            for x in x1..x2 {
                let blended_color = if alpha == 1.0 {
                    color
                } else {
                    let dst_color = self.get_pixel_color(palettes, x as u16, y as u16);
                    blend_color_alpha(dst_color, color, alpha)
                };
                self.set_pixel(x as i32, y as i32, blended_color, palettes);
            }
        }
    }

    pub fn copy_pixels(
        &self, 
        palettes: &PaletteMap,
        src: &Bitmap, 
        dst_rect: IntRect,
        src_rect: IntRect,
        param_list: &HashMap<String, Datum>,
    ) {
        let blend = param_list.get("blend")
            .map(|x| x.int_value().unwrap())
            .unwrap_or(100);
        let ink = param_list.get("ink");
        let ink = if let Some(ink) = ink {
            ink.int_value().unwrap() as u32
        } else {
            0
        };
        let bg_color = param_list.get("bgColor");
        let bg_color = if let Some(bg_color) = bg_color {
            bg_color.to_color_ref().unwrap().to_owned()
        } else {
            ColorRef::PaletteIndex(0)
        };
        let color = param_list.get("color");
        let color = if let Some(color) = color {
            color.to_color_ref().unwrap().to_owned()
        } else {
            ColorRef::PaletteIndex(255)
        };

        let mask_image = param_list.get("maskImage");
        let mask_image = mask_image.map(|x| x.to_mask().unwrap().clone());
        
        let params = CopyPixelsParams {
            blend,
            ink,
            bg_color,
            mask_image,
            color,
        };
        self.copy_pixels_with_params(palettes, src, dst_rect, src_rect, &params);
    }

    pub fn is_dirty(&self) -> bool {
        match &self.procedural_type {
            Some(ProceduralBitmapType::Text(_, is_dirty)) => {
                *is_dirty.borrow()
            }
            _ => false
        }
    }

    pub fn set_dirty(&self, dirty: bool) {
        match &self.procedural_type {
            Some(ProceduralBitmapType::Text(_, is_dirty)) => {
                *is_dirty.borrow_mut() = dirty;
            }
            _ => {}
        }
    }

    fn render_if_needed(&self) {
        match &self.procedural_type {
            Some(ProceduralBitmapType::Text(text_data, is_dirty)) => {
                if !*is_dirty.borrow() {
                    return;
                }
                *is_dirty.borrow_mut() = false;
                let text_data = text_data.borrow();
                reserve_player_mut(|player| {
                    let palettes = &player.movie.cast_manager.palettes();
                    TextMemberHandlers::render_to_bitmap(
                        palettes, 
                        &player.font_manager,
                        &player.bitmap_manager,
                        &text_data, 
                        self
                    ).unwrap();
                });
            }
            _ => {}
        }
    }

    pub fn copy_pixels_with_params(
        &self, 
        palettes: &PaletteMap,
        src: &Bitmap, 
        dst_rect: IntRect,
        src_rect: IntRect,
        params: &CopyPixelsParams,
    ) {
        let ink = params.ink;
        let alpha = params.blend as f32 / 100.0;
        let mask_image = params.mask_image.as_ref();
        let bg_color = &params.bg_color;
        let bg_color = resolve_color_ref(palettes, &bg_color, &self.palette_ref);

        let mut src_y = if dst_rect.height() < 0 { src_rect.bottom } else { src_rect.top } as f32;
        let step_x = src_rect.width() as f32 / dst_rect.width() as f32;
        let step_y = src_rect.height() as f32 / dst_rect.height() as f32;

        let (min_dst_x, max_dst_x) = {
            if dst_rect.width() < 0 {
                (dst_rect.right - 1, dst_rect.left)
            } else {
                (dst_rect.left, dst_rect.right)
            }
        };
        let (min_dst_y, max_dst_y) = {
            if dst_rect.height() < 0 {
                (dst_rect.bottom - 1, dst_rect.top)
            } else {
                (dst_rect.top, dst_rect.bottom)
            }
        };

        for dst_y in min_dst_y..max_dst_y {
            let mut src_x = if dst_rect.width() < 0 { src_rect.right } else { src_rect.left } as f32;
            for dst_x in min_dst_x..max_dst_x {
                if let Some(mask_image) = mask_image {
                    if !mask_image.get_bit(src_x as u16, src_y as u16) {
                        src_x += step_x;
                        continue;
                    }
                }
                let src_color = src.get_pixel_color(palettes, src_x.floor() as u16, src_y.floor() as u16);
                let dst_color = self.get_pixel_color(palettes, dst_x as u16, dst_y as u16);
                let blended_color = blend_pixel(dst_color, src_color, ink, bg_color, alpha);

                self.set_pixel(dst_x, dst_y, blended_color, palettes);
                src_x += step_x;
            }
            src_y += step_y;
        }
        // Uncomment below to debug copyPixel calls
        // self.stroke_rect(min_dst_x, min_dst_y, max_dst_x, max_dst_y, (0, 255, 0), palettes, 1.0);
    }

    pub fn draw_text(
        &self,
        text: &str,
        font: &BitmapFont,
        font_bitmap: &Bitmap,
        loc_h: i32,
        loc_v: i32,
        ink: u32,
        bg_color: ColorRef,
        palettes: &PaletteMap,
        line_spacing: u16,
        top_spacing: i16,
    ) {
        let mut x = loc_h;
        let mut y = loc_v;
        let line_height = font.char_height;

        let mut params = CopyPixelsParams::default(&self);
        params.ink = ink;
        params.bg_color = bg_color;

        for char_num in text.chars() {
            if char_num == '\r' || char_num == '\n' {
                x = loc_h;
                y += line_height as i32 + line_spacing as i32 + 1;
                continue;
            }
            bitmap_font_copy_char(font, font_bitmap, char_num as u8, self, x, y, &palettes, &params);
            x += font.char_width as i32 + 1;
        }
    }

    pub fn trim_whitespace(&self, palettes: &PaletteMap) {
        let mut left = 0 as i32;
        let mut top = 0 as i32;
        let mut right = self.width() as i32;
        let mut bottom = self.height() as i32;
        let bg_color = self.get_bg_color_ref();

        for x in 0..self.width() as i32 {
            let mut is_empty = true;
            for y in 0..self.height() as i32 {
                let color = self.get_pixel_color_ref(x as u16, y as u16);
                if color != bg_color {
                    is_empty = false;
                    break;
                }
            }
            if !is_empty {
                left = x;
                break;
            }
        }

        for x in (0..self.width() as i32).rev() {
            let mut is_empty = true;
            for y in 0..self.height() as i32 {
                let color = self.get_pixel_color_ref(x as u16, y as u16);
                if color != bg_color {
                    is_empty = false;
                    break;
                }
            }
            if !is_empty {
                right = x + 1;
                break;
            }
        }

        for y in 0..self.height() as i32 {
            let mut is_empty = true;
            for x in 0..self.width() as i32 {
                let color = self.get_pixel_color_ref(x as u16, y as u16);
                if color != bg_color {
                    is_empty = false;
                    break;
                }
            }
            if !is_empty {
                top = y;
                break;
            }
        }

        for y in (0..self.height() as i32).rev() {
            let mut is_empty = true;
            for x in 0..self.width() as i32 {
                let color = self.get_pixel_color_ref(x as u16, y as u16);
                if color != bg_color {
                    is_empty = false;
                    break;
                }
            }
            if !is_empty {
                bottom = y + 1;
                break;
            }
        }

        let width = right - left;
        let height = bottom - top;

        let trimmed = Bitmap::new(width as u16, height as u16, self.bit_depth, self.palette_ref.clone(), None);
        let params = CopyPixelsParams::default(&self);
        trimmed.copy_pixels_with_params(palettes, &self, IntRect::from(0, 0, width, height), IntRect::from(left, top, right, bottom), &params);
        
        let data = trimmed.data.take();
        self.set_size_with_data(width as u16, height as u16, data);
    }

    pub fn to_mask(&self) -> BitmapMask {
        let mut mask = BitmapMask::new(self.width(), self.height(), false);
        let bg_color = self.get_bg_color_ref();
        for y in 0..self.height() {
            for x in 0..self.width() {
                let pixel = self.get_pixel_color_ref(x, y);
                if pixel != bg_color {
                    mask.set_bit(x, y, true);
                }
            }
        }
        mask
    }
}
