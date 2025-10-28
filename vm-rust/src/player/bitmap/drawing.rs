use std::collections::HashMap;

use nohash_hasher::IntMap;
use rgb565::Rgb565;

use crate::{
    director::lingo::datum::Datum,
    player::{
        font::{bitmap_font_copy_char, BitmapFont},
        geometry::IntRect,
        sprite::ColorRef,
    },
};

use super::{
    bitmap::{resolve_color_ref, Bitmap},
    mask::BitmapMask,
    palette_map::PaletteMap,
};

pub struct CopyPixelsParams<'a> {
    pub blend: i32,
    pub ink: u32,
    pub color: ColorRef,
    pub bg_color: ColorRef,
    pub mask_image: Option<&'a BitmapMask>,
}

impl CopyPixelsParams<'_> {
    pub const fn default(bitmap: &Bitmap) -> CopyPixelsParams<'static> {
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
    blend_alpha: f32, // This is params.blend / 100.0
    src_alpha: f32,   // Alpha from the source pixel (0.0 to 1.0)
) -> (u8, u8, u8) {
    // Calculate the effective alpha: combination of native source alpha and blend parameter
    let effective_alpha = src_alpha * blend_alpha;

    match ink {
        // 0 = Copy (Director semantics: copy source over destination)
        // If fully opaque/effective_alpha==1 => hard copy, otherwise alpha-blend.
        0 => {
            if (effective_alpha - 1.0).abs() < 1e-6 {
                src
            } else {
                blend_color_alpha(dst, src, effective_alpha)
            }
        }
        // ... (other ink modes use effective_alpha too, just like 'Copy')
        // 7 = Not Ghost
        // Approximation: similar to copy but skip bg_color (like ink 36),
        // many implementations treat this as a matte-related/alpha-preserving ink.
        // We'll behave like "if src == bg_color -> dst, else blend normally".
        7 => {
            if src == bg_color {
                dst
            } else {
                blend_color_alpha(dst, src, effective_alpha)
            }
        }
        // 8 = Matte
        // Use source alpha (or mask) as matte. If fully opaque, copy; otherwise blend.
        8 => {
            if src_alpha <= 0.001 {
                dst
            } else if (effective_alpha - 1.0).abs() < 1e-6 {
                src
            } else {
                blend_color_alpha(dst, src, effective_alpha)
            }
        }
        // 9 = Mask
        // Mask typically means use mask to determine copy. Here we assume mask was applied earlier,
        // so treat it as a hard copy when present (i.e. if we've reached here, copy).
        9 => {
            if src_alpha <= 0.001 {
                dst
            } else {
                // If blend < 1, respect it.
                if effective_alpha >= 1.0 {
                    src
                } else {
                    blend_color_alpha(dst, src, effective_alpha)
                }
            }
        }
        // 33 = Add Pin (additive but skip background color)
        // Add source color to destination (optionally modulated by effective alpha).
        33 => {
            if src == bg_color {
                dst
            } else {
                let r = (dst.0 as f32 + (src.0 as f32 * effective_alpha)).min(255.0);
                let g = (dst.1 as f32 + (src.1 as f32 * effective_alpha)).min(255.0);
                let b = (dst.2 as f32 + (src.2 as f32 * effective_alpha)).min(255.0);
                (r as u8, g as u8, b as u8)
            }
        }
        // 36 = Background Transparent
        // If the source equals the bg_color, skip; otherwise blend normally.
        36 => {
            if src == bg_color {
                dst
            } else {
                blend_color_alpha(dst, src, effective_alpha)
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
            blend_color_alpha(dst, color, effective_alpha)
        }
        _ => blend_color_alpha(dst, src, effective_alpha),
    }
}

impl Bitmap {
    pub fn get_pixel_color_with_alpha(
        &self,
        palettes: &PaletteMap,
        x: u16,
        y: u16,
    ) -> (u8, u8, u8, u8) {
        let color_ref = self.get_pixel_color_ref(x, y);
        let (r, g, b) = resolve_color_ref(
            palettes,
            &color_ref,
            &self.palette_ref,
            self.original_bit_depth,
        );

        if self.bit_depth == 32 {
            let x_usize = x as usize;
            let y_usize = y as usize;
            if x_usize < self.width as usize && y_usize < self.height as usize {
                let index = (y_usize * self.width as usize + x_usize) * 4;
                // The alpha component is the 4th byte for 32-bit data (R, G, B, A)
                let a = self.data[index + 3];
                return (r, g, b, a);
            }
        }
        // Default to fully opaque
        (r, g, b, 0xFF)
    }

    pub fn set_pixel(&mut self, x: i32, y: i32, color: (u8, u8, u8), palettes: &PaletteMap) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        self.matte = None; // TODO draw on matte instead
        let (r, g, b) = color;
        let x = x as usize;
        let y = y as usize;
        if x < self.width as usize && y < self.height as usize {
            let bytes_per_pixel = self.bit_depth as usize / 8;
            let index = (y * self.width as usize + x) * bytes_per_pixel;
            match self.bit_depth {
                1 => {
                    let bit_index = y * self.width as usize + x;
                    let byte_index = bit_index / 8;
                    let bit_offset = bit_index % 8;
                    let value = self.data[byte_index];
                    let mask = 1 << (7 - bit_offset);
                    let value = if r > 127 || g > 127 || b > 127 {
                        value | mask
                    } else {
                        value & !mask
                    };
                    self.data[byte_index] = value;
                }
                4 => {
                    let own_palette = &self.palette_ref;
                    let mut result_index: u8 = 0;
                    let mut result_distance = i32::MAX;

                    for palette_idx in 0..16u8 {
                        let palette_color = resolve_color_ref(
                            palettes,
                            &ColorRef::PaletteIndex(palette_idx),
                            &own_palette,
                            self.original_bit_depth,
                        );
                        let distance = (r as i32 - palette_color.0 as i32).abs()
                            + (g as i32 - palette_color.1 as i32).abs()
                            + (b as i32 - palette_color.2 as i32).abs();
                        if distance < result_distance {
                            result_index = palette_idx;
                            result_distance = distance;
                        }
                    }
                    self.data[index] = result_index;
                }
                8 => {
                    let own_palette = &self.palette_ref;
                    let mut result_index = 0;
                    let mut result_distance = i32::MAX;

                    for idx in 0..=255 {
                        let palette_color = resolve_color_ref(
                            palettes,
                            &ColorRef::PaletteIndex(idx as u8),
                            &own_palette,
                            self.original_bit_depth,
                        );
                        let distance = (r as i32 - palette_color.0 as i32).abs()
                            + (g as i32 - palette_color.1 as i32).abs()
                            + (b as i32 - palette_color.2 as i32).abs();
                        if distance < result_distance {
                            result_index = idx;
                            result_distance = distance;
                        }
                    }
                    self.data[index] = result_index;
                }
                16 => {
                    let r = r as f32 * 31.0 / 255.0;
                    let g = g as f32 * 63.0 / 255.0;
                    let b = b as f32 * 31.0 / 255.0;
                    let value = Rgb565::pack_565((r as u8, g as u8, b as u8));
                    let bytes = value.to_le_bytes();
                    self.data[index] = bytes[0];
                    self.data[index + 1] = bytes[1];
                }
                32 => {
                    self.data[index] = r;
                    self.data[index + 1] = g;
                    self.data[index + 2] = b;
                    self.data[index + 3] = 0xFF;
                }
                _ => {
                    // TODO: Should this be logged?
                    println!("Unsupported bit depth for set_pixel: {}", self.bit_depth);
                }
            }
        }
    }

    pub fn get_pixel_color_ref(&self, x: u16, y: u16) -> ColorRef {
        let x = x as usize;
        let y = y as usize;
        if x >= self.width as usize || y >= self.height as usize {
            return self.get_bg_color_ref();
        }

        match self.bit_depth {
            4 => {
                let bit_index = (y * self.width as usize + x) * 4;
                let byte_index = bit_index / 8;
                let value = self.data[byte_index];

                let nibble = if x % 2 == 0 {
                    value >> 4 // High nibble (0-15)
                } else {
                    value & 0x0F // Low nibble (0-15)
                };

                // 4-bit uses 16-color palette, values are 0-15
                ColorRef::PaletteIndex(nibble)
            }
            8 => {
                let index = y * self.width as usize + x;
                ColorRef::PaletteIndex(self.data[index])
            }
            16 => {
                let index = (y * self.width as usize + x) * 2;
                let value = u16::from_le_bytes([self.data[index], self.data[index + 1]]);
                let (red, green, blue) = Rgb565::unpack_565(value);
                let red = (red as f32 / 31.0 * 255.0) as u8;
                let green = (green as f32 / 63.0 * 255.0) as u8;
                let blue = (blue as f32 / 31.0 * 255.0) as u8;
                ColorRef::Rgb(red, green, blue)
            }
            32 => {
                let bytes_per_pixel = 4;
                let index = (y * self.width as usize + x) * bytes_per_pixel as usize;
                ColorRef::Rgb(self.data[index], self.data[index + 1], self.data[index + 2])
            }
            _ => {
                self.get_bg_color_ref()
                // panic!("Unsupported bit depth: {}", self.bit_depth)
            }
        }
    }

    pub fn get_pixel_color(&self, palettes: &PaletteMap, x: u16, y: u16) -> (u8, u8, u8) {
        let color_ref = self.get_pixel_color_ref(x, y);
        resolve_color_ref(
            palettes,
            &color_ref,
            &self.palette_ref,
            self.original_bit_depth,
        )
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

    pub fn _flipped_hv(&self, palettes: &PaletteMap) -> Bitmap {
        let mut flipped = self.clone();
        for y in 0..self.height as usize {
            for x in 0..self.width as usize {
                let src_x = x;
                let src_y = y;
                let dst_x = self.width as usize - x - 1;
                let dst_y = self.height as usize - y - 1;
                let src_color = self.get_pixel_color(palettes, src_x as u16, src_y as u16);
                flipped.set_pixel(dst_x as i32, dst_y as i32, src_color, palettes);
            }
        }
        flipped
    }

    pub fn _flipped_h(&self, palettes: &PaletteMap) -> Bitmap {
        let mut flipped = self.clone();
        for y in 0..self.height as usize {
            for x in 0..self.width as usize {
                let src_x = x;
                let src_y = y;
                let dst_x = self.width as usize - x - 1;
                let dst_y = y;
                let src_color = self.get_pixel_color(palettes, src_x as u16, src_y as u16);
                flipped.set_pixel(dst_x as i32, dst_y as i32, src_color, palettes);
            }
        }
        flipped
    }

    pub fn _flipped_v(&self, palettes: &PaletteMap) -> Bitmap {
        let mut flipped = self.clone();
        for y in 0..self.height as usize {
            for x in 0..self.width as usize {
                let src_x = x;
                let src_y = y;
                let dst_x = x;
                let dst_y = self.height as usize - y - 1;
                let src_color = self.get_pixel_color(palettes, src_x as u16, src_y as u16);
                flipped.set_pixel(dst_x as i32, dst_y as i32, src_color, palettes);
            }
        }
        flipped
    }

    pub fn stroke_sized_rect(
        &mut self,
        left: i32,
        top: i32,
        width: i32,
        height: i32,
        color: (u8, u8, u8),
        palettes: &PaletteMap,
        alpha: f32,
    ) {
        let left = left.max(0) as i32;
        let top = top.max(0) as i32;
        let right = (left + width) as i32;
        let bottom = (top + height) as i32;
        self.stroke_rect(left, top, right, bottom, color, palettes, alpha);
    }

    pub fn stroke_rect(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        color: (u8, u8, u8),
        palettes: &PaletteMap,
        alpha: f32,
    ) {
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

    pub fn clear_rect(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        color: (u8, u8, u8),
        palettes: &PaletteMap,
    ) {
        for y in y1..y2 {
            for x in x1..x2 {
                self.set_pixel(x, y, color, palettes);
            }
        }
    }

    pub fn fill_relative_rect(
        &mut self,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        color: (u8, u8, u8),
        palettes: &PaletteMap,
        alpha: f32,
    ) {
        let left = left.max(0);
        let top = top.max(0);
        let right = right.min(self.width as i32 - 1);
        let bottom = bottom.min(self.height as i32 - 1);

        let x1 = left;
        let y1 = top;
        let x2 = self.width as i32 - right;
        let y2 = self.height as i32 - bottom;

        self.fill_rect(x1, y1, x2, y2, color, palettes, alpha);
    }

    pub fn fill_rect(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        color: (u8, u8, u8),
        palettes: &PaletteMap,
        alpha: f32,
    ) {
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
        &mut self,
        palettes: &PaletteMap,
        src: &Bitmap,
        dst_rect: IntRect,
        src_rect: IntRect,
        param_list: &HashMap<String, Datum>,
    ) {
        let blend = param_list
            .get("blend")
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
        let mask_image = mask_image.map(|x| x.to_mask().unwrap());

        let params = CopyPixelsParams {
            blend,
            ink,
            bg_color,
            mask_image,
            color,
        };
        self.copy_pixels_with_params(palettes, src, dst_rect, src_rect, &params);
    }

    /// Copy pixels from src to self, respecting scaling, flipping, masks, and blending
    pub fn copy_pixels_with_params(
        &mut self,
        palettes: &PaletteMap,
        src: &Bitmap,
        dst_rect: IntRect,
        src_rect: IntRect,
        params: &CopyPixelsParams,
    ) {
        let ink = params.ink;
        let alpha = params.blend as f32 / 100.0;
        let mask_image = params.mask_image;
        let bg_color = resolve_color_ref(
            palettes,
            &params.bg_color,
            &src.palette_ref,
            src.original_bit_depth,
        );

        let is_flipped_h = dst_rect.width() < 0;
        let is_flipped_v = dst_rect.height() < 0;
        let step_x = (src_rect.width() as f32 / dst_rect.width() as f32).abs();
        let step_y = (src_rect.height() as f32 / dst_rect.height() as f32).abs();

        let (min_dst_x, max_dst_x) = if is_flipped_h {
            (dst_rect.right, dst_rect.left)
        } else {
            (dst_rect.left, dst_rect.right)
        };
        let (min_dst_y, max_dst_y) = if is_flipped_v {
            (dst_rect.bottom, dst_rect.top)
        } else {
            (dst_rect.top, dst_rect.bottom)
        };

        let mut src_y = if is_flipped_v {
            src_rect.bottom as f32 - step_y / 2.0
        } else {
            src_rect.top as f32 + step_y / 2.0
        };

        for dst_y in min_dst_y..max_dst_y {
            let mut src_x = if is_flipped_h {
                src_rect.right as f32 - step_x / 2.0
            } else {
                src_rect.left as f32 + step_x / 2.0
            };

            for dst_x in min_dst_x..max_dst_x {
                // Skip out-of-bounds
                if dst_x < 0
                    || dst_y < 0
                    || dst_x >= self.width as i32
                    || dst_y >= self.height as i32
                {
                    src_x += if is_flipped_h { -step_x } else { step_x };
                    continue;
                }

                let src_x_int = src_x.floor() as u16;
                let src_y_int = src_y.floor() as u16;

                if src_x_int >= src.width || src_y_int >= src.height {
                    src_x += if is_flipped_h { -step_x } else { step_x };
                    continue;
                }

                if let Some(mask_image) = mask_image {
                    if !mask_image.get_bit(src_x_int, src_y_int) {
                        src_x += if is_flipped_h { -step_x } else { step_x };
                        continue;
                    }
                }

                let (src_r, src_g, src_b, src_a) =
                    src.get_pixel_color_with_alpha(palettes, src_x_int, src_y_int);
                let src_color = (src_r, src_g, src_b);
                let src_alpha = src_a as f32 / 255.0;

                // Skip background
                if ink == 36 && src_color == bg_color {
                    src_x += if is_flipped_h { -step_x } else { step_x };
                    continue;
                }

                let dst_color = self.get_pixel_color(palettes, dst_x as u16, dst_y as u16);

                let blended_color =
                    blend_pixel(dst_color, src_color, ink, bg_color, alpha, src_alpha);

                self.set_pixel(dst_x, dst_y, blended_color, palettes);
                src_x += if is_flipped_h { -step_x } else { step_x };
            }
            src_y += if is_flipped_v { -step_y } else { step_y };
        }
        // Uncomment below to debug copyPixel calls
        // self.stroke_rect(min_dst_x, min_dst_y, max_dst_x, max_dst_y, (0, 255, 0), palettes, 1.0);
    }

    pub fn _draw_bitmap(
        &mut self,
        palettes: &PaletteMap,
        bitmap: &Bitmap,
        loc_h: i32,
        loc_v: i32,
        width: i32,
        height: i32,
        ink: u32,
        bg_color: (u8, u8, u8),
        alpha: f32,
    ) {
        let mut params = HashMap::new();
        params.insert("blend".to_owned(), Datum::Int((alpha * 100.0) as i32));
        params.insert("ink".to_owned(), Datum::Int(ink as i32));
        params.insert(
            "bgColor".to_owned(),
            Datum::ColorRef(ColorRef::Rgb(bg_color.0, bg_color.1, bg_color.2)),
        );

        let src_rect = IntRect::from_tuple((0, 0, bitmap.width as i32, bitmap.height as i32));
        let dst_rect =
            IntRect::from_tuple((loc_h, loc_v, loc_h + width as i32, loc_v + height as i32));
        self.copy_pixels(palettes, bitmap, dst_rect, src_rect, &params);
    }

    pub fn draw_text(
        &mut self,
        text: &str,
        font: &BitmapFont,
        font_bitmap: &Bitmap,
        loc_h: i32,
        loc_v: i32,
        params: CopyPixelsParams,
        palettes: &PaletteMap,
        line_spacing: u16,
        top_spacing: i16,
    ) {
        let mut x = loc_h;
        let mut y = loc_v + top_spacing as i32;
        let line_height = font.char_height;

        for char_num in text.chars() {
            if char_num == '\r' || char_num == '\n' {
                x = loc_h;
                y += line_height as i32 + line_spacing as i32;
                continue;
            }

            bitmap_font_copy_char(
                font,
                font_bitmap,
                char_num as u8,
                self,
                x,
                y,
                &palettes,
                &params,
            );

            // Use the font's actual char_width, not char_width + 1
            // PFR fonts already have proper spacing built in
            x += font.char_width as i32;
        }
    }

    pub fn trim_whitespace(&mut self, palettes: &PaletteMap) {
        let mut left = 0 as i32;
        let mut top = 0 as i32;
        let mut right = self.width as i32;
        let mut bottom = self.height as i32;
        let bg_color = self.get_bg_color_ref();

        for x in 0..self.width as i32 {
            let mut is_empty = true;
            for y in 0..self.height as i32 {
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

        for x in (0..self.width as i32).rev() {
            let mut is_empty = true;
            for y in 0..self.height as i32 {
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

        for y in 0..self.height as i32 {
            let mut is_empty = true;
            for x in 0..self.width as i32 {
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

        for y in (0..self.height as i32).rev() {
            let mut is_empty = true;
            for x in 0..self.width as i32 {
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

        let mut trimmed = Bitmap::new(
            width as u16,
            height as u16,
            self.bit_depth,
            self.original_bit_depth,
            0,
            self.palette_ref.clone(),
        );
        let params = CopyPixelsParams::default(&self);
        trimmed.copy_pixels_with_params(
            palettes,
            &self,
            IntRect::from(0, 0, width, height),
            IntRect::from(left, top, right, bottom),
            &params,
        );

        self.width = width as u16;
        self.height = height as u16;
        self.data = trimmed.data;
    }

    pub fn to_mask(&self) -> BitmapMask {
        let mut mask = BitmapMask::new(self.width, self.height, false);
        let bg_color = self.get_bg_color_ref();
        for y in 0..self.height {
            for x in 0..self.width {
                let pixel = self.get_pixel_color_ref(x, y);
                if pixel != bg_color {
                    mask.set_bit(x, y, true);
                }
            }
        }
        mask
    }

    /// Flood fills starting from a point, replacing the original color with the target color.
    /// Emulates Director's `image.floodFill(point, color)` behavior.
    pub fn flood_fill(
        &mut self,
        start_point: (i32, i32),
        target_color: (u8, u8, u8),
        palettes: &PaletteMap,
    ) {
        let (start_x, start_y) = start_point;

        // --- Bounds check (Director silently ignores invalid coords)
        if start_x < 0
            || start_y < 0
            || start_x >= self.width as i32
            || start_y >= self.height as i32
        {
            return;
        }

        // --- Capture the original color at the starting pixel
        let original_color = self.get_pixel_color(palettes, start_x as u16, start_y as u16);

        // --- If the starting color is already the target color, nothing to fill
        if Self::color_equal(original_color, target_color) {
            return;
        }

        use std::collections::HashSet;

        let mut stack = Vec::with_capacity(256);
        let mut visited = HashSet::with_capacity(256);

        stack.push((start_x, start_y));
        visited.insert((start_x as u16, start_y as u16));

        while let Some((x, y)) = stack.pop() {
            // --- Bounds check
            if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
                continue;
            }

            // --- Check current pixel color
            let current_color = self.get_pixel_color(palettes, x as u16, y as u16);

            // --- Only fill if the color matches the original color
            if !Self::color_equal(current_color, original_color) {
                continue;
            }

            // --- Set pixel to target color
            self.set_pixel(x, y, target_color, palettes);

            // --- Push 4-connected neighbors
            for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                if nx >= 0 && ny >= 0 && nx < self.width as i32 && ny < self.height as i32 {
                    let pos = (nx as u16, ny as u16);
                    if visited.insert(pos) {
                        stack.push((nx, ny));
                    }
                }
            }
        }
    }

    fn color_equal(a: (u8, u8, u8), b: (u8, u8, u8)) -> bool {
        // Director treats colors as equal if their RGB values match exactly.
        // Ensure get_pixel_color() already resolved to RGB.
        a.0 == b.0 && a.1 == b.1 && a.2 == b.2
    }
}
