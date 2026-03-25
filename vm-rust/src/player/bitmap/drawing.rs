use std::collections::HashMap;

use rgb565::Rgb565;

use crate::{
    director::{
        enums::{ShapeInfo, ShapeType},
        lingo::datum::Datum,
    },
    player::{
        font::{bitmap_font_copy_char, BitmapFont},
        geometry::IntRect,
        sprite::{ColorRef, is_skew_flip},
        bitmap::bitmap::{get_system_default_palette, PaletteRef}, Sprite, Score,
        reserve_player_mut,
    },
};

use super::{
    bitmap::{resolve_color_ref, resolve_palette_table, Bitmap},
    mask::BitmapMask,
    palette_map::PaletteMap,
};

pub struct CopyPixelsParams<'a> {
    pub blend: i32,
    pub ink: u32,
    pub color: ColorRef,
    pub bg_color: ColorRef,
    pub mask_image: Option<&'a BitmapMask>,
    pub is_text_rendering: bool,
    pub rotation: f64,
    pub skew: f64,
    pub sprite: Option<&'a Sprite>,
    pub original_dst_rect: Option<IntRect>,
}

impl CopyPixelsParams<'_> {
    pub const fn default(bitmap: &Bitmap) -> CopyPixelsParams<'static> {
        CopyPixelsParams {
            blend: 100,
            ink: 0,
            color: bitmap.get_fg_color_ref(),
            bg_color: bitmap.get_bg_color_ref(),
            mask_image: None,
            is_text_rendering: false,
            rotation: 0.0,
            skew: 0.0,
            sprite: None,
            original_dst_rect: None,
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
    ink == 2 || ink == 36 || ink == 33 || ink == 37 || ink == 39 || ink == 41 || ink == 8 || ink == 7
}

fn director_blend_ink0(
    dst: (u8, u8, u8),
    src: (u8, u8, u8),
    src_alpha: f32,
    blend: f32,
) -> (u8, u8, u8) {
    // Premultiply source by its own alpha
    let sr = src.0 as f32 * src_alpha;
    let sg = src.1 as f32 * src_alpha;
    let sb = src.2 as f32 * src_alpha;

    let dr = dst.0 as f32;
    let dg = dst.1 as f32;
    let db = dst.2 as f32;

    let inv = 1.0 - blend;

    (
        (dr * inv + sr * blend).round().clamp(0.0, 255.0) as u8,
        (dg * inv + sg * blend).round().clamp(0.0, 255.0) as u8,
        (db * inv + sb * blend).round().clamp(0.0, 255.0) as u8,
    )
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
            if blend_alpha >= 0.999 {
                // Normal copy, still respecting source alpha
                if src_alpha >= 0.999 {
                    src
                } else {
                    blend_color_alpha(dst, src, src_alpha)
                }
            } else {
                director_blend_ink0(dst, src, src_alpha, blend_alpha)
            }
        }
        // ... (other ink modes use effective_alpha too, just like 'Copy')
        // 7 = Not Ghost
        // Approximation: similar to copy but skip bg_color (like ink 36),
        // many implementations treat this as a matte-related/alpha-preserving ink.
        // We'll behave like "if src == bg_color -> dst, else blend normally".
        7 => {
            blend_color_alpha(dst, src, effective_alpha)
        }
        // 8 = Matte
        // Transparency is decided BEFORE blending.
        // At this point, the pixel is opaque (or partially via src_alpha).
        8 => {
            if src_alpha <= 0.001 {
                dst
            } else if effective_alpha >= 0.999 {
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
                if effective_alpha >= 0.999 {
                    src
                } else {
                    blend_color_alpha(dst, src, effective_alpha)
                }
            }
        }
        // 33 = Add Pin (Director-style additive, pinned to 255)
        // bg_color pixels are transparent
        // Standard additive: add source RGB to destination
        33 => {
            if src == bg_color {
                dst
            } else {
                // Standard additive: add source to destination
                let r = ((dst.0 as u32 + src.0 as u32).min(255)) as u8;
                let g = ((dst.1 as u32 + src.1 as u32).min(255)) as u8;
                let b = ((dst.2 as u32 + src.2 as u32).min(255)) as u8;

                // Apply blend factor
                if blend_alpha >= 0.999 {
                    (r, g, b)
                } else {
                    blend_color_alpha(dst, (r, g, b), blend_alpha)
                }
            }
        }
        // 35 = Sub Pin (Director-style subtractive, pinned to 0)
        // bg_color pixels are transparent
        // Subtractive: subtract source RGB from destination
        35 => {
            if src == bg_color {
                dst
            } else {
                // Subtractive: subtract source from destination
                let r = (dst.0 as i32 - src.0 as i32).max(0) as u8;
                let g = (dst.1 as i32 - src.1 as i32).max(0) as u8;
                let b = (dst.2 as i32 - src.2 as i32).max(0) as u8;

                // Apply blend factor
                if blend_alpha >= 0.999 {
                    (r, g, b)
                } else {
                    blend_color_alpha(dst, (r, g, b), blend_alpha)
                }
            }
        }
        // 36 = Background Transparent
        // If the source equals the bg_color, skip; otherwise blend normally.
        36 => {
            blend_color_alpha(dst, src, effective_alpha)
        }
        // 37 = Light (Lighten)
        // Pick the higher of src and dst for each channel.
        // bg_color pixels are transparent (skipped).
        37 => {
            if src == bg_color {
                dst
            } else {
                let r = src.0.max(dst.0);
                let g = src.1.max(dst.1);
                let b = src.2.max(dst.2);
                if blend_alpha >= 0.999 {
                    (r, g, b)
                } else {
                    blend_color_alpha(dst, (r, g, b), blend_alpha)
                }
            }
        }
        // 39 = Dark (Darken)
        // Pick the lower of src and dst for each channel.
        // bg_color pixels are transparent (skipped).
        39 => {
            if src == bg_color {
                dst
            } else {
                let r = src.0.min(dst.0);
                let g = src.1.min(dst.1);
                let b = src.2.min(dst.2);
                if blend_alpha >= 0.999 {
                    (r, g, b)
                } else {
                    blend_color_alpha(dst, (r, g, b), blend_alpha)
                }
            }
        }
        // 40 = Lighten
        40 => {
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

    /// Like `get_pixel_color_with_alpha`, but uses a pre-resolved palette table.
    #[inline]
    pub fn get_pixel_color_with_alpha_fast(
        &self,
        palette_cache: &[(u8, u8, u8)],
        x: u16,
        y: u16,
    ) -> (u8, u8, u8, u8) {
        let color_ref = self.get_pixel_color_ref(x, y);
        let (r, g, b) = match &color_ref {
            ColorRef::PaletteIndex(i) => palette_cache[*i as usize],
            ColorRef::Rgb(r, g, b) => (*r, *g, *b),
        };

        if self.bit_depth == 32 {
            let x_usize = x as usize;
            let y_usize = y as usize;
            if x_usize < self.width as usize && y_usize < self.height as usize {
                let index = (y_usize * self.width as usize + x_usize) * 4;
                let a = self.data[index + 3];
                return (r, g, b, a);
            }
        }
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

    /// Like `set_pixel`, but uses a pre-resolved palette table for indexed formats.
    /// For 4-bit/8-bit bitmaps, this avoids calling `resolve_color_ref` 16/256 times per pixel.
    pub fn set_pixel_fast(&mut self, x: i32, y: i32, color: (u8, u8, u8), palette_cache: &[(u8, u8, u8)]) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        self.matte = None;
        let (r, g, b) = color;
        let x = x as usize;
        let y = y as usize;
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
                let mut result_index: u8 = 0;
                let mut result_distance = i32::MAX;
                for (palette_idx, &(pr, pg, pb)) in palette_cache.iter().enumerate().take(16) {
                    let distance = (r as i32 - pr as i32).abs()
                        + (g as i32 - pg as i32).abs()
                        + (b as i32 - pb as i32).abs();
                    if distance < result_distance {
                        result_index = palette_idx as u8;
                        result_distance = distance;
                    }
                }
                let index = (y * self.width as usize + x) / 2;
                if x % 2 == 0 {
                    self.data[index] = (self.data[index] & 0x0F) | (result_index << 4);
                } else {
                    self.data[index] = (self.data[index] & 0xF0) | (result_index & 0x0F);
                }
            }
            8 => {
                let mut result_index: u8 = 0;
                let mut result_distance = i32::MAX;
                for (idx, &(pr, pg, pb)) in palette_cache.iter().enumerate() {
                    let distance = (r as i32 - pr as i32).abs()
                        + (g as i32 - pg as i32).abs()
                        + (b as i32 - pb as i32).abs();
                    if distance < result_distance {
                        result_index = idx as u8;
                        result_distance = distance;
                    }
                }
                let index = y * self.width as usize + x;
                self.data[index] = result_index;
            }
            16 => {
                let r = r as f32 * 31.0 / 255.0;
                let g = g as f32 * 63.0 / 255.0;
                let b = b as f32 * 31.0 / 255.0;
                let value = Rgb565::pack_565((r as u8, g as u8, b as u8));
                let bytes = value.to_le_bytes();
                let index = (y * self.width as usize + x) * 2;
                self.data[index] = bytes[0];
                self.data[index + 1] = bytes[1];
            }
            32 => {
                let index = (y * self.width as usize + x) * 4;
                self.data[index] = r;
                self.data[index + 1] = g;
                self.data[index + 2] = b;
                self.data[index + 3] = 0xFF;
            }
            _ => {}
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

    /// Like `get_pixel_color`, but uses a pre-resolved palette table for indexed formats.
    #[inline]
    pub fn get_pixel_color_fast(&self, palette_cache: &[(u8, u8, u8)], x: u16, y: u16) -> (u8, u8, u8) {
        let color_ref = self.get_pixel_color_ref(x, y);
        match color_ref {
            ColorRef::PaletteIndex(i) => palette_cache[i as usize],
            ColorRef::Rgb(r, g, b) => (r, g, b),
        }
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

    /// Clears a rectangular region with fully transparent pixels (alpha = 0).
    /// Used for filmloop rendering where we need transparency instead of a solid background.
    pub fn clear_rect_transparent(&mut self, x1: i32, y1: i32, x2: i32, y2: i32) {
        // Only works for 32-bit bitmaps
        if self.bit_depth != 32 {
            return;
        }
        for y in y1..y2 {
            for x in x1..x2 {
                if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
                    continue;
                }
                let index = (y as usize * self.width as usize + x as usize) * 4;
                // Set RGBA to (0, 0, 0, 0) - fully transparent black
                self.data[index] = 0;
                self.data[index + 1] = 0;
                self.data[index + 2] = 0;
                self.data[index + 3] = 0; // Alpha = 0 (transparent)
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

    /// Draw a filled ellipse inscribed in the given bounding box.
    /// Uses midpoint ellipse algorithm with horizontal scanline filling.
    pub fn fill_ellipse(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        color: (u8, u8, u8),
        palettes: &PaletteMap,
        alpha: f32,
    ) {
        if alpha == 0.0 { return; }
        let w = (x2 - x1).max(1);
        let h = (y2 - y1).max(1);
        // Center coordinates (doubled to avoid fractions)
        let cx2 = x1 + x2; // 2 * center_x
        let cy2 = y1 + y2; // 2 * center_y
        let a = w; // 2 * semi-axis a
        let b = h; // 2 * semi-axis b
        let a2 = (a as i64) * (a as i64);
        let b2 = (b as i64) * (b as i64);

        // Fill scanlines for each y from top to bottom
        for py in y1..y2 {
            // Map py to doubled coordinates relative to center
            let dy2 = 2 * py - cy2 + 1; // doubled distance from center
            let dy2_sq = (dy2 as i64) * (dy2 as i64);
            // Ellipse equation: (dx/(w/2))^2 + (dy/(h/2))^2 <= 1
            // In doubled coords: (dx2/a)^2 + (dy2/b)^2 <= 1
            // => dx2^2 <= a^2 * (1 - dy2^2/b^2) = a^2 * (b^2 - dy2^2) / b^2
            if b2 == 0 { continue; }
            let dx2_sq_max = a2 * (b2 - dy2_sq) / b2;
            if dx2_sq_max < 0 { continue; }
            let dx2_max = (dx2_sq_max as f64).sqrt() as i32;
            let px_left = (cx2 - dx2_max) / 2;
            let px_right = (cx2 + dx2_max + 1) / 2; // +1 for ceiling
            let left = px_left.max(x1);
            let right = px_right.min(x2);
            for px in left..right {
                if alpha == 1.0 {
                    self.set_pixel(px, py, color, palettes);
                } else {
                    let dst = self.get_pixel_color(palettes, px as u16, py as u16);
                    self.set_pixel(px, py, blend_color_alpha(dst, color, alpha), palettes);
                }
            }
        }
    }

    /// Draw an ellipse outline inscribed in the given bounding box.
    pub fn stroke_ellipse(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        color: (u8, u8, u8),
        palettes: &PaletteMap,
        alpha: f32,
        thickness: i32,
    ) {
        if alpha == 0.0 || thickness <= 0 { return; }
        // Draw by filling outer ellipse minus inner ellipse
        // For thickness=1, just plot boundary pixels
        let w = (x2 - x1).max(1);
        let h = (y2 - y1).max(1);
        let cx2 = x1 + x2;
        let cy2 = y1 + y2;
        let a_outer = w;
        let b_outer = h;
        let a_inner = (w - 2 * thickness).max(0);
        let b_inner = (h - 2 * thickness).max(0);
        let a_outer2 = (a_outer as i64) * (a_outer as i64);
        let b_outer2 = (b_outer as i64) * (b_outer as i64);
        let a_inner2 = (a_inner as i64) * (a_inner as i64);
        let b_inner2 = (b_inner as i64) * (b_inner as i64);

        for py in y1..y2 {
            let dy2 = 2 * py - cy2 + 1;
            let dy2_sq = (dy2 as i64) * (dy2 as i64);

            // Outer ellipse x range
            if b_outer2 == 0 { continue; }
            let dx2_sq_outer = a_outer2 * (b_outer2 - dy2_sq) / b_outer2;
            if dx2_sq_outer < 0 { continue; }
            let dx2_outer = (dx2_sq_outer as f64).sqrt() as i32;
            let outer_left = (cx2 - dx2_outer) / 2;
            let outer_right = (cx2 + dx2_outer + 1) / 2;

            // Inner ellipse x range
            let (inner_left, inner_right) = if b_inner2 > 0 && a_inner2 > 0 {
                let dx2_sq_inner = a_inner2 * (b_inner2 - dy2_sq) / b_inner2;
                if dx2_sq_inner > 0 {
                    let dx2_inner = (dx2_sq_inner as f64).sqrt() as i32;
                    let il = (cx2 - dx2_inner) / 2 + thickness;
                    let ir = (cx2 + dx2_inner + 1) / 2 - thickness;
                    if ir > il { (il, ir) } else { (outer_right, outer_left) } // no inner gap
                } else {
                    (outer_right, outer_left) // no inner gap at this y
                }
            } else {
                (outer_right, outer_left) // fully filled at this y (thin ellipse)
            };

            // Draw left stroke band and right stroke band
            for px in outer_left.max(x1)..inner_left.min(outer_right).min(x2) {
                if alpha == 1.0 {
                    self.set_pixel(px, py, color, palettes);
                } else {
                    let dst = self.get_pixel_color(palettes, px as u16, py as u16);
                    self.set_pixel(px, py, blend_color_alpha(dst, color, alpha), palettes);
                }
            }
            for px in inner_right.max(outer_left).max(x1)..outer_right.min(x2) {
                if alpha == 1.0 {
                    self.set_pixel(px, py, color, palettes);
                } else {
                    let dst = self.get_pixel_color(palettes, px as u16, py as u16);
                    self.set_pixel(px, py, blend_color_alpha(dst, color, alpha), palettes);
                }
            }
        }
    }

    /// Draw a filled rounded rectangle with given corner radius.
    pub fn fill_round_rect(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        radius: i32,
        color: (u8, u8, u8),
        palettes: &PaletteMap,
        alpha: f32,
    ) {
        if alpha == 0.0 { return; }
        let w = x2 - x1;
        let h = y2 - y1;
        let r = radius.min(w / 2).min(h / 2).max(0);

        for py in y1..y2 {
            let dy_top = py - y1;
            let dy_bottom = (y2 - 1) - py;
            let dy = dy_top.min(dy_bottom);

            let (left, right) = if dy < r {
                // In corner region — compute circular inset
                let ry = r - dy;
                let rx = r - ((r * r - ry * ry) as f64).sqrt() as i32;
                (x1 + rx, x2 - rx)
            } else {
                (x1, x2)
            };

            for px in left..right {
                if alpha == 1.0 {
                    self.set_pixel(px, py, color, palettes);
                } else {
                    let dst = self.get_pixel_color(palettes, px as u16, py as u16);
                    self.set_pixel(px, py, blend_color_alpha(dst, color, alpha), palettes);
                }
            }
        }
    }

    /// Draw a rounded rectangle outline with given corner radius and line thickness.
    pub fn stroke_round_rect(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        radius: i32,
        color: (u8, u8, u8),
        palettes: &PaletteMap,
        alpha: f32,
        thickness: i32,
    ) {
        if alpha == 0.0 || thickness <= 0 { return; }
        let w = x2 - x1;
        let h = y2 - y1;
        let r_outer = radius.min(w / 2).min(h / 2).max(0);
        let r_inner = (r_outer - thickness).max(0);

        for py in y1..y2 {
            let dy_top = py - y1;
            let dy_bottom = (y2 - 1) - py;
            let dy = dy_top.min(dy_bottom);

            // Outer edge
            let (outer_left, outer_right) = if dy < r_outer {
                let ry = r_outer - dy;
                let rx = r_outer - ((r_outer * r_outer - ry * ry) as f64).sqrt() as i32;
                (x1 + rx, x2 - rx)
            } else {
                (x1, x2)
            };

            // Inner edge
            let (inner_left, inner_right) = if dy < thickness {
                // Top or bottom edge — fill entire outer span
                (outer_right, outer_left) // signals "no gap"
            } else if dy < r_outer {
                let inner_dy = dy - thickness;
                if inner_dy < r_inner {
                    let ry = r_inner - inner_dy;
                    let rx = r_inner - ((r_inner * r_inner - ry * ry).max(0) as f64).sqrt() as i32;
                    (x1 + thickness + rx, x2 - thickness - rx)
                } else {
                    (x1 + thickness, x2 - thickness)
                }
            } else {
                (x1 + thickness, x2 - thickness)
            };

            // If inner_left >= inner_right, fill the whole outer row
            if inner_left >= inner_right {
                for px in outer_left..outer_right {
                    if alpha == 1.0 {
                        self.set_pixel(px, py, color, palettes);
                    } else {
                        let dst = self.get_pixel_color(palettes, px as u16, py as u16);
                        self.set_pixel(px, py, blend_color_alpha(dst, color, alpha), palettes);
                    }
                }
            } else {
                // Left stroke band
                for px in outer_left..inner_left.min(outer_right) {
                    if alpha == 1.0 {
                        self.set_pixel(px, py, color, palettes);
                    } else {
                        let dst = self.get_pixel_color(palettes, px as u16, py as u16);
                        self.set_pixel(px, py, blend_color_alpha(dst, color, alpha), palettes);
                    }
                }
                // Right stroke band
                for px in inner_right.max(outer_left)..outer_right {
                    if alpha == 1.0 {
                        self.set_pixel(px, py, color, palettes);
                    } else {
                        let dst = self.get_pixel_color(palettes, px as u16, py as u16);
                        self.set_pixel(px, py, blend_color_alpha(dst, color, alpha), palettes);
                    }
                }
            }
        }
    }

    /// Draw a line with given thickness using Bresenham's algorithm.
    pub fn draw_line_thick(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        color: (u8, u8, u8),
        palettes: &PaletteMap,
        alpha: f32,
        thickness: i32,
    ) {
        if alpha == 0.0 || thickness <= 0 { return; }
        let half = thickness / 2;

        let dx = (x2 - x1).abs();
        let dy = -(y2 - y1).abs();
        let sx: i32 = if x1 < x2 { 1 } else { -1 };
        let sy: i32 = if y1 < y2 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut cx = x1;
        let mut cy = y1;

        loop {
            // Draw a filled circle/square at each point for thickness
            if thickness <= 2 {
                // For thin lines, draw a small square
                for oy in -half..=(thickness - 1 - half) {
                    for ox in -half..=(thickness - 1 - half) {
                        let px = cx + ox;
                        let py = cy + oy;
                        if alpha == 1.0 {
                            self.set_pixel(px, py, color, palettes);
                        } else {
                            let dst = self.get_pixel_color(palettes, px as u16, py as u16);
                            self.set_pixel(px, py, blend_color_alpha(dst, color, alpha), palettes);
                        }
                    }
                }
            } else {
                // For thicker lines, draw perpendicular to line direction
                for oy in -half..=(thickness - 1 - half) {
                    for ox in -half..=(thickness - 1 - half) {
                        let px = cx + ox;
                        let py = cy + oy;
                        if alpha == 1.0 {
                            self.set_pixel(px, py, color, palettes);
                        } else {
                            let dst = self.get_pixel_color(palettes, px as u16, py as u16);
                            self.set_pixel(px, py, blend_color_alpha(dst, color, alpha), palettes);
                        }
                    }
                }
            }

            if cx == x2 && cy == y2 { break; }
            let e2 = 2 * err;
            if e2 >= dy {
                if cx == x2 { break; }
                err += dy;
                cx += sx;
            }
            if e2 <= dx {
                if cy == y2 { break; }
                err += dx;
                cy += sy;
            }
        }
    }

    /// Flatten a cubic bezier curve into line segments using adaptive subdivision.
    /// Returns a list of (x, y) points along the curve.
    /// P0 = start, C1 = first control point, C2 = second control point, P3 = end.
    pub fn flatten_cubic_bezier(
        p0: (f32, f32),
        c1: (f32, f32),
        c2: (f32, f32),
        p3: (f32, f32),
        tolerance: f32,
    ) -> Vec<(f32, f32)> {
        let mut result = vec![p0];
        Self::flatten_bezier_recursive(p0, c1, c2, p3, tolerance, &mut result, 0);
        result
    }

    fn flatten_bezier_recursive(
        p0: (f32, f32),
        c1: (f32, f32),
        c2: (f32, f32),
        p3: (f32, f32),
        tolerance: f32,
        result: &mut Vec<(f32, f32)>,
        depth: u32,
    ) {
        // Check flatness: distance from control points to line P0-P3
        let dx = p3.0 - p0.0;
        let dy = p3.1 - p0.1;
        let len_sq = dx * dx + dy * dy;

        if depth > 10 || len_sq < 0.001 {
            // Max recursion or degenerate segment
            result.push(p3);
            return;
        }

        // Distance from C1 and C2 to line P0-P3
        let d1 = ((c1.0 - p0.0) * dy - (c1.1 - p0.1) * dx).abs();
        let d2 = ((c2.0 - p0.0) * dy - (c2.1 - p0.1) * dx).abs();
        let max_dist = (d1 + d2) / len_sq.sqrt();

        if max_dist <= tolerance {
            result.push(p3);
            return;
        }

        // De Casteljau subdivision at t=0.5
        let m01 = ((p0.0 + c1.0) * 0.5, (p0.1 + c1.1) * 0.5);
        let m12 = ((c1.0 + c2.0) * 0.5, (c1.1 + c2.1) * 0.5);
        let m23 = ((c2.0 + p3.0) * 0.5, (c2.1 + p3.1) * 0.5);
        let m012 = ((m01.0 + m12.0) * 0.5, (m01.1 + m12.1) * 0.5);
        let m123 = ((m12.0 + m23.0) * 0.5, (m12.1 + m23.1) * 0.5);
        let mid = ((m012.0 + m123.0) * 0.5, (m012.1 + m123.1) * 0.5);

        Self::flatten_bezier_recursive(p0, m01, m012, mid, tolerance, result, depth + 1);
        Self::flatten_bezier_recursive(mid, m123, m23, p3, tolerance, result, depth + 1);
    }

    /// Blend a color with alpha onto a 32-bit RGBA pixel in the bitmap data.
    /// Uses premultiplied alpha "over" compositing.
    fn blend_pixel_aa(&mut self, px: i32, py: i32, color: (u8, u8, u8), coverage: f32) {
        if px < 0 || py < 0 || px >= self.width as i32 || py >= self.height as i32 {
            return;
        }
        let idx = (py as usize * self.width as usize + px as usize) * 4;
        if idx + 3 >= self.data.len() { return; }

        let src_a = (coverage * 255.0 + 0.5) as u8;
        if src_a == 0 { return; }

        let dst_r = self.data[idx] as u16;
        let dst_g = self.data[idx + 1] as u16;
        let dst_b = self.data[idx + 2] as u16;
        let dst_a = self.data[idx + 3] as u16;

        let sa = src_a as u16;
        let inv_sa = 255 - sa;

        self.data[idx]     = ((color.0 as u16 * sa + dst_r * inv_sa) / 255) as u8;
        self.data[idx + 1] = ((color.1 as u16 * sa + dst_g * inv_sa) / 255) as u8;
        self.data[idx + 2] = ((color.2 as u16 * sa + dst_b * inv_sa) / 255) as u8;
        self.data[idx + 3] = (dst_a + (sa * (255 - dst_a)) / 255).min(255) as u8;
    }

    /// Draw an anti-aliased thick line segment using signed distance from the line.
    fn draw_line_aa(
        &mut self,
        x1: f32, y1: f32,
        x2: f32, y2: f32,
        half_width: f32,
        color: (u8, u8, u8),
        _palettes: &PaletteMap,
        alpha: f32,
    ) {
        let dx = x2 - x1;
        let dy = y2 - y1;
        let seg_len = (dx * dx + dy * dy).sqrt();
        if seg_len < 0.001 { return; }

        // Bounding box of the thick line
        let expand = half_width + 1.0;
        let px_min = (x1.min(x2) - expand).floor().max(0.0) as i32;
        let py_min = (y1.min(y2) - expand).floor().max(0.0) as i32;
        let px_max = (x1.max(x2) + expand).ceil().min(self.width as f32 - 1.0) as i32;
        let py_max = (y1.max(y2) + expand).ceil().min(self.height as f32 - 1.0) as i32;

        let inv_len_sq = 1.0 / (seg_len * seg_len);
        for py in py_min..=py_max {
            for px in px_min..=px_max {
                let fx = px as f32 + 0.5;
                let fy = py as f32 + 0.5;

                // Project onto line segment
                let t = (((fx - x1) * dx + (fy - y1) * dy) * inv_len_sq).clamp(0.0, 1.0);

                // Distance to closest point on segment
                let cx = x1 + t * dx;
                let cy = y1 + t * dy;
                let dist = ((fx - cx) * (fx - cx) + (fy - cy) * (fy - cy)).sqrt();

                // Coverage with smooth anti-aliased edge
                let coverage = (half_width + 0.5 - dist).clamp(0.0, 1.0) * alpha;
                if coverage > 0.001 {
                    self.blend_pixel_aa(px, py, color, coverage);
                }
            }
        }
    }

    /// Draw an anti-aliased filled circle for round line joins/caps.
    fn draw_circle_aa(
        &mut self,
        cx: f32, cy: f32,
        radius: f32,
        color: (u8, u8, u8),
        _palettes: &PaletteMap,
        alpha: f32,
    ) {
        let expand = radius + 1.0;
        let px_min = (cx - expand).floor().max(0.0) as i32;
        let py_min = (cy - expand).floor().max(0.0) as i32;
        let px_max = (cx + expand).ceil().min(self.width as f32 - 1.0) as i32;
        let py_max = (cy + expand).ceil().min(self.height as f32 - 1.0) as i32;

        for py in py_min..=py_max {
            for px in px_min..=px_max {
                let fx = px as f32 + 0.5;
                let fy = py as f32 + 0.5;
                let dist = ((fx - cx) * (fx - cx) + (fy - cy) * (fy - cy)).sqrt();
                let coverage = (radius + 0.5 - dist).clamp(0.0, 1.0) * alpha;
                if coverage > 0.001 {
                    self.blend_pixel_aa(px, py, color, coverage);
                }
            }
        }
    }

    /// Draw a vectorShape (bezier curves with stroke/fill) onto this bitmap.
    /// Coordinates are mapped from local vertex space to the destination rect.
    pub fn draw_vector_shape(
        &mut self,
        vector_data: &crate::player::cast_member::VectorShapeMember,
        dst_rect: IntRect,
        palettes: &PaletteMap,
        alpha: f32,
    ) {
        if vector_data.vertices.len() < 2 || alpha == 0.0 {
            return;
        }

        let verts = &vector_data.vertices;

        // Compute bounding box from vertices + stroke padding.
        // The stroke extends half its width beyond vertex positions, so we pad
        // the bbox to ensure the full stroke fits within the destination bitmap.
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        for v in verts.iter() {
            min_x = min_x.min(v.x);
            min_y = min_y.min(v.y);
            max_x = max_x.max(v.x);
            max_y = max_y.max(v.y);
        }
        let pad = vector_data.stroke_width / 1.0;
        min_x -= pad;
        min_y -= pad;
        max_x += pad;
        max_y += pad;

        let src_w = max_x - min_x;
        let src_h = max_y - min_y;
        if src_w <= 0.0 || src_h <= 0.0 {
            return;
        }

        let dst_w = (dst_rect.right - dst_rect.left) as f32;
        let dst_h = (dst_rect.bottom - dst_rect.top) as f32;
        let scale_x = dst_w / src_w;
        let scale_y = dst_h / src_h;

        // Map a point from local vertex space to destination pixel coordinates
        let map_point = |x: f32, y: f32| -> (f32, f32) {
            let px = (x - min_x) * scale_x + dst_rect.left as f32;
            let py = (y - min_y) * scale_y + dst_rect.top as f32;
            (px, py)
        };

        // Flatten all bezier segments into polyline points
        let mut all_points: Vec<(f32, f32)> = Vec::new();
        let num_segments = if vector_data.closed {
            verts.len()
        } else {
            verts.len() - 1
        };

        for i in 0..num_segments {
            let j = (i + 1) % verts.len();
            let v0 = &verts[i];
            let v1 = &verts[j];

            // P0 = vertex[i]
            let p0 = map_point(v0.x, v0.y);
            // C1 = vertex[i] + handle1[i] (outgoing control point)
            let c1 = map_point(v0.x + v0.handle1_x, v0.y + v0.handle1_y);
            // C2 = vertex[j] + handle2[j] (incoming control point)
            let c2 = map_point(v1.x + v1.handle2_x, v1.y + v1.handle2_y);
            // P3 = vertex[j]
            let p3 = map_point(v1.x, v1.y);

            let segment_points = Self::flatten_cubic_bezier(p0, c1, c2, p3, 0.5);
            if i == 0 {
                all_points.extend_from_slice(&segment_points);
            } else {
                // Skip first point (duplicate of previous segment's last point)
                all_points.extend_from_slice(&segment_points[1..]);
            }
        }

        // Fill if fillMode != 0 (solid fill)
        if vector_data.fill_mode != 0 && all_points.len() >= 3 {
            self.scanline_fill_polygon(&all_points, vector_data.fill_color, palettes, alpha);
        }

        // Anti-aliased stroke
        if vector_data.stroke_width > 0.0 && all_points.len() >= 2 {
            let half_w = vector_data.stroke_width / 2.0;
            let color = vector_data.stroke_color;
            for i in 0..all_points.len() - 1 {
                let (x1, y1) = all_points[i];
                let (x2, y2) = all_points[i + 1];
                self.draw_line_aa(x1, y1, x2, y2, half_w, color, palettes, alpha);
            }
            // Draw round caps at each polyline point for smooth joins
            for &(px, py) in &all_points {
                self.draw_circle_aa(px, py, half_w, color, palettes, alpha);
            }
        }
    }

    /// Draw a vector shape with full ink support, following the same pattern as
    /// draw_shape_with_sprite: ink 0 draws directly, other inks use an intermediate
    /// bitmap + copy_pixels for proper ink compositing.
    pub fn draw_vector_shape_with_sprite(
        &mut self,
        sprite: &crate::player::sprite::Sprite,
        vector_data: &crate::player::cast_member::VectorShapeMember,
        dst_rect: IntRect,
        palettes: &PaletteMap,
    ) {
        let alpha = (sprite.blend as f32 / 100.0).clamp(0.0, 1.0);

        if sprite.ink == 0 {
            // Copy ink: draw directly onto destination bitmap
            self.draw_vector_shape(vector_data, dst_rect, palettes, alpha);
        } else {
            // Non-copy ink: use temp bitmap + copy_pixels for proper ink handling
            let w = dst_rect.width().max(1);
            let h = dst_rect.height().max(1);
            let mut temp = Bitmap::new(
                w as u16, h as u16,
                32, 32, 0,
                super::bitmap::PaletteRef::BuiltIn(get_system_default_palette()),
            );
            // Start fully transparent
            temp.data.fill(0);
            temp.use_alpha = true;

            // Draw vector shape into temp at local (0,0) coordinates
            let local_rect = IntRect::from_tuple((0, 0, w, h));
            temp.draw_vector_shape(vector_data, local_rect, palettes, 1.0);

            let mut params = HashMap::new();
            params.insert("blend".into(), Datum::Int(sprite.blend as i32));
            params.insert("ink".into(), Datum::Int(sprite.ink as i32));
            params.insert("color".into(), Datum::ColorRef(sprite.color.clone()));
            params.insert("bgColor".into(), Datum::ColorRef(sprite.bg_color.clone()));

            self.copy_pixels(
                palettes,
                &temp,
                dst_rect,
                IntRect::from_tuple((0, 0, w, h)),
                &params,
                None,
            );
        }
    }

    /// Scanline fill a polygon defined by a list of points.
    fn scanline_fill_polygon(
        &mut self,
        points: &[(f32, f32)],
        color: (u8, u8, u8),
        palettes: &PaletteMap,
        alpha: f32,
    ) {
        if points.len() < 3 || alpha == 0.0 {
            return;
        }

        // Find vertical extent
        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;
        for p in points.iter() {
            min_y = min_y.min(p.1);
            max_y = max_y.max(p.1);
        }
        let y_start = min_y.floor() as i32;
        let y_end = max_y.ceil() as i32;

        // For each scanline, find intersection x-coordinates with polygon edges
        for y in y_start..=y_end {
            let yf = y as f32 + 0.5; // sample at scanline center
            let mut intersections: Vec<f32> = Vec::new();

            let n = points.len();
            for i in 0..n {
                let j = (i + 1) % n;
                let (x0, y0) = points[i];
                let (x1, y1) = points[j];

                // Check if scanline crosses this edge
                if (y0 <= yf && y1 > yf) || (y1 <= yf && y0 > yf) {
                    let t = (yf - y0) / (y1 - y0);
                    intersections.push(x0 + t * (x1 - x0));
                }
            }

            intersections.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

            // Fill between pairs of intersections
            let mut i = 0;
            while i + 1 < intersections.len() {
                let x_start = intersections[i].ceil() as i32;
                let x_end = intersections[i + 1].floor() as i32;
                for x in x_start..=x_end {
                    if alpha == 1.0 {
                        self.set_pixel(x, y, color, palettes);
                    } else {
                        let dst = self.get_pixel_color(palettes, x as u16, y as u16);
                        self.set_pixel(x, y, blend_color_alpha(dst, color, alpha), palettes);
                    }
                }
                i += 2;
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
        score: Option<&Score>,
    ) {
        let mut blend = param_list
            .get("blend")
            .map(|x| x.int_value().unwrap())
            .unwrap_or(100);
        let ink = param_list.get("ink");
        let mut ink = if let Some(ink) = ink {
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
        let mask_image = mask_image.and_then(|x| x.to_mask_or_none());

        // Extract rotation parameter (defaults to 0.0 if not provided)
        let rotation = param_list
            .get("rotation")
            .and_then(|x| x.float_value().ok())
            .unwrap_or(0.0);

        // Extract skew parameter (defaults to 0.0 if not provided)
        let skew = param_list
            .get("skew")
            .and_then(|x| x.float_value().ok())
            .unwrap_or(0.0);

        // Check if is_text_rendering parameter exists and is true
        // This is typically NOT set from Lingo scripts, only internally
        let is_text_rendering = param_list
            .get("is_text_rendering")
            .and_then(|x| x.to_bool().ok())
            .unwrap_or(false);

        // Get sprite number, then resolve to actual sprite
        let sprite = score.and_then(|score| {
            param_list
                .get("sprite")
                .and_then(|x| x.to_sprite_ref().ok())
                .and_then(|sprite_num| score.get_sprite(sprite_num))
        });

        let original_dst_rect: Option<IntRect> = param_list
            .get("original_dst_rect")
            .and_then(|datum| {
                if let Datum::Rect(rect_refs) = datum {
                    reserve_player_mut(|player| {
                        let left = player.get_datum(&rect_refs[0]).int_value().ok()?;
                        let top = player.get_datum(&rect_refs[1]).int_value().ok()?;
                        let right = player.get_datum(&rect_refs[2]).int_value().ok()?;
                        let bottom = player.get_datum(&rect_refs[3]).int_value().ok()?;

                        Some(IntRect::from(left, top, right, bottom))
                    })
                } else {
                    None
                }
            });

        // Text glyphs ALWAYS use Copy ink
        if is_text_rendering {
            ink = 0;
        }

        let params = CopyPixelsParams {
            blend,
            ink,
            bg_color,
            mask_image,
            color,
            is_text_rendering,
            rotation,
            skew,
            sprite,
            original_dst_rect,
        };
        self.copy_pixels_with_params(palettes, src, dst_rect, src_rect, &params);
    }

    fn calculate_rotated_bounding_box(
        rect: &IntRect,
        rotation_degrees: f64,
        pivot_x: i32,
        pivot_y: i32,
    ) -> IntRect {
        let theta = rotation_degrees * std::f64::consts::PI / 180.0;
        let cos_theta = theta.cos();
        let sin_theta = theta.sin();
        
        // Registration point in sprite-local coordinates
        let pivot_x = pivot_x as f64;
        let pivot_y = pivot_y as f64;
            
        // Define the 4 corners of the original rectangle
        let corners = [
            (rect.left as f64, rect.top as f64),
            (rect.right as f64, rect.top as f64),
            (rect.right as f64, rect.bottom as f64),
            (rect.left as f64, rect.bottom as f64),
        ];
        
        // Rotate each corner around the pivot point
        let mut rotated_corners = Vec::new();
        for (x, y) in corners.iter() {
            let dx = x - pivot_x as f64;
            let dy = y - pivot_y as f64;
            
            let rotated_x = pivot_x as f64 + (dx * cos_theta - dy * sin_theta);
            let rotated_y = pivot_y as f64 + (dx * sin_theta + dy * cos_theta);
            
            rotated_corners.push((rotated_x, rotated_y));
        }
        
        // Find the bounding box of rotated corners
        let min_x = rotated_corners.iter().map(|(x, _)| *x).fold(f64::INFINITY, f64::min) as i32;
        let max_x = rotated_corners.iter().map(|(x, _)| *x).fold(f64::NEG_INFINITY, f64::max) as i32;
        let min_y = rotated_corners.iter().map(|(_, y)| *y).fold(f64::INFINITY, f64::min) as i32;
        let max_y = rotated_corners.iter().map(|(_, y)| *y).fold(f64::NEG_INFINITY, f64::max) as i32;
        
        IntRect::from(min_x, min_y, max_x, max_y)
    }

    fn apply_forecolor_tint(
        src: (u8, u8, u8),
        fore: (u8, u8, u8),
    ) -> (u8, u8, u8) {
        (
            ((src.0 as u16 * fore.0 as u16) / 255) as u8,
            ((src.1 as u16 * fore.1 as u16) / 255) as u8,
            ((src.2 as u16 * fore.2 as u16) / 255) as u8,
        )
    }

    fn allows_colorize(depth: u8, ink: u32, is_text: bool) -> bool {
        if is_text {
            return true; // text has its own rules
        }

        match (depth, ink) {
            (32, 0) => true,            // grayscale remap
            (32, 8) | (32, 9) => true,  // foreColor only
            (d, 0) if d <= 8 => true,
            (d, 8) | (d, 9) if d <= 8 => true,
            _ => false, // ink 36, 7, 33, 40, etc
        }
    }

    fn uses_back_color(depth: u8, ink: u32) -> bool {
        ink == 0 && (depth == 32 || depth <= 8)
    }

    /// Copy pixels from src to self, respecting scaling, flipping, masks, blending, and rotation
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
        // Sprite foreColor/backColor palette indices are resolved against the bitmap's palette,
        // so they work together correctly (e.g., index 248/255 in a custom 256-color palette).
        let bg_color_resolved = resolve_color_ref(
            palettes,
            &params.bg_color,
            &src.palette_ref,
            src.original_bit_depth,
        );

        let fg_color_resolved = resolve_color_ref(
            palettes,
            &params.color,
            &src.palette_ref,
            src.original_bit_depth,
        );

        let is_indexed = src.original_bit_depth <= 8;

        // Pre-resolve palettes into lookup tables to avoid per-pixel resolve_color_ref calls.
        // Source table: used for reading indexed source pixels without calling resolve_color_ref.
        let src_palette_cache: Option<Vec<(u8, u8, u8)>> = if is_indexed {
            Some(resolve_palette_table(palettes, &src.palette_ref, src.original_bit_depth))
        } else {
            None
        };
        // Destination table: used by set_pixel_fast to avoid O(256) nearest-color search per pixel.
        let dst_palette_cache: Vec<(u8, u8, u8)> = if self.bit_depth <= 8 {
            resolve_palette_table(palettes, &self.palette_ref, self.original_bit_depth)
        } else {
            Vec::new()
        };

        let bg_index = match &params.bg_color {
            ColorRef::PaletteIndex(i) => *i,
            _ => 0, // Director default
        }; 

        let is_matte_bitmap =
            src.trim_white_space
            || params.is_text_rendering;

        let use_grayscale_as_alpha = match src.palette_ref {
            PaletteRef::BuiltIn(palette) => {
                palette.symbol_string().eq_ignore_ascii_case("grayscale")
            }
            _ => false, // Any other palette → not grayscale
        };

        // ----------- Setup destination bounds and flip flags -------------
        let min_dst_x = dst_rect.left.min(dst_rect.right);
        let max_dst_x = dst_rect.left.max(dst_rect.right);
        let min_dst_y = dst_rect.top.min(dst_rect.bottom);
        let max_dst_y = dst_rect.top.max(dst_rect.bottom);
        let flip_x = dst_rect.right < dst_rect.left;
        let flip_y = dst_rect.bottom < dst_rect.top;

        // ----------- Scaling factors -------------
        let dst_w = (max_dst_x - min_dst_x) as f64;
        let dst_h = (max_dst_y - min_dst_y) as f64;
        let src_w = src_rect.width() as f64;
        let src_h = src_rect.height() as f64;

        // For rotation: use original rect dimensions for scaling, not expanded bounding box
        let (scale_w, scale_h) = if let Some(orig_rect) = &params.original_dst_rect {
            (
                (orig_rect.right - orig_rect.left) as f64,
                (orig_rect.bottom - orig_rect.top) as f64,
            )
        } else {
            (dst_w, dst_h)
        };

        // Use full ratio (src_size / dst_size)
        let scale_x = src_w / scale_w;
        let scale_y = src_h / scale_h;

        let min_dst_x_f = min_dst_x as f64;
        let min_dst_y_f = min_dst_y as f64;
        let src_left_f = src_rect.left as f64;
        let src_top_f = src_rect.top as f64;

        let mut sprite_num = 0;

        // ----------------------------------------------------------
        // Calculate sprite rotation pivot (registration point)
        // ----------------------------------------------------------
        let (center_x, center_y) = if let Some(sprite) = params.sprite {
            sprite_num = sprite.number;
            // Rotate around the sprite's registration point (locH, locV)
            (sprite.loc_h as f64, sprite.loc_v as f64)
        } else {
            // Fallback: rotate around center of destination rect
            (
                (dst_rect.left + dst_rect.right) as f64 / 2.0,
                (dst_rect.top + dst_rect.bottom) as f64 / 2.0,
            )
        };

        // Precompute rotation values if sprite rotation is needed
        let has_sprite_rotation = params.rotation.abs() > 0.1;
        let (cos_theta, sin_theta) = if has_sprite_rotation {
            let theta = -(params.rotation) * std::f64::consts::PI / 180.0;
            (theta.cos(), theta.sin())
        } else {
            (1.0, 0.0)
        };

        // Check for skew-based flip (skew=±180° combined with rotation produces a mirror)
        let has_skew_flip = is_skew_flip(params.skew);

        // ----------------------------------------------------------
        // Director-style draw bounds (allow rotated overflow)
        // ----------------------------------------------------------
        let (draw_min_x, draw_max_x, draw_min_y, draw_max_y) =
            if has_sprite_rotation {
                if let (Some(orig_rect), Some(sprite)) =
                    (&params.original_dst_rect, params.sprite)
                {
                    let expanded = Self::calculate_rotated_bounding_box(
                        orig_rect,
                        params.rotation,
                        sprite.loc_h,
                        sprite.loc_v,
                    );

                    (
                        expanded.left.min(expanded.right),
                        expanded.left.max(expanded.right),
                        expanded.top.min(expanded.bottom),
                        expanded.top.max(expanded.bottom),
                    )
                } else {
                    (min_dst_x, max_dst_x, min_dst_y, max_dst_y)
                }
            } else {
                (min_dst_x, max_dst_x, min_dst_y, max_dst_y)
            };

        let needs_matte_mask =
            !params.is_text_rendering
            && is_matte_bitmap
            && ((ink == 0 && src.original_bit_depth <= 8)
                || (ink == 8 && (src.original_bit_depth <= 8 || (src.original_bit_depth == 32 && !src.use_alpha))));

        let mut matte_mask: Option<Vec<Vec<bool>>> = None;

        // ----------------------------------------------------------
        // 32-bit matte key color:
        // - Ink 0: use bgColor (sprite's background color, typically white)
        // - Other inks (8, etc.): use edge color (pixel 0,0)
        // ----------------------------------------------------------
        let edge_matte_color: Option<(u8, u8, u8)> =
            if src.original_bit_depth == 32 && !src.use_alpha {
                let (r, g, b, _) =
                    src.get_pixel_color_with_alpha(palettes, 0, 0);
                Some((r, g, b))
            } else {
                None
            };

        if needs_matte_mask {
            let width = src.width as usize;
            let height = src.height as usize;

            let mut mask = vec![vec![false; width]; height];
            let mut stack = Vec::<(usize, usize)>::new();

            // Fast pixel color getter using cached palette table
            let get_src_rgb = |x: u16, y: u16| -> (u8, u8, u8) {
                let color_ref = src.get_pixel_color_ref(x, y);
                if let (ColorRef::PaletteIndex(i), Some(cache)) = (&color_ref, &src_palette_cache) {
                    cache[*i as usize]
                } else {
                    resolve_color_ref(palettes, &color_ref, &src.palette_ref, src.original_bit_depth)
                }
            };

            let matte_bg = edge_matte_color.unwrap_or(bg_color_resolved);

            // ---- seed flood fill from edges ----
            for x in 0..width {
                if get_src_rgb(x as u16, 0) == matte_bg {
                    stack.push((x, 0));
                }
                if get_src_rgb(x as u16, (height - 1) as u16) == matte_bg {
                    stack.push((x, height - 1));
                }
            }

            for y in 0..height {
                if get_src_rgb(0, y as u16) == matte_bg {
                    stack.push((0, y));
                }
                if get_src_rgb((width - 1) as u16, y as u16) == matte_bg {
                    stack.push((width - 1, y));
                }
            }

            // ---- flood fill ----
            while let Some((x, y)) = stack.pop() {
                if mask[y][x] {
                    continue;
                }

                if get_src_rgb(x as u16, y as u16) != matte_bg {
                    continue;
                }

                mask[y][x] = true;

                if x > 0 { stack.push((x - 1, y)); }
                if x + 1 < width { stack.push((x + 1, y)); }
                if y > 0 { stack.push((x, y - 1)); }
                if y + 1 < height { stack.push((x, y + 1)); }
            }

            matte_mask = Some(mask);
        }

        // ---------------- Pixel loop ----------------
        for dst_y in draw_min_y..draw_max_y {
            for dst_x in draw_min_x..draw_max_x {
                if dst_x < 0
                    || dst_y < 0
                    || dst_x >= self.width as i32
                    || dst_y >= self.height as i32
                {
                    continue;
                }

                // ----------------------------------------------------------
                // SPRITE ROTATION & SKEW: Apply transforms to destination coordinates
                // ----------------------------------------------------------
                let (rotated_x, rotated_y) = if has_sprite_rotation || has_skew_flip {
                    // Translate to center
                    let dx = dst_x as f64 - center_x as f64;
                    let mut dy = dst_y as f64 - center_y as f64;

                    // Apply skew flip (negate Y before rotation)
                    // When combined with rotation=180, this produces a vertical flip (mirror)
                    // rotation=180 alone: (dx, dy) -> (-dx, -dy) = upside down
                    // rotation=180 + skew=180: (dx, -dy) -> (-dx, dy) = vertical flip
                    if has_skew_flip {
                        dy = -dy;
                    }

                    // Apply inverse rotation matrix
                    let rx = dx * cos_theta - dy * sin_theta;
                    let ry = dx * sin_theta + dy * cos_theta;

                    // Translate back
                    (rx + center_x as f64, ry + center_y as f64)
                } else {
                    (dst_x as f64, dst_y as f64)
                };

                // Calculate indices relative to destination rect
                let dst_x_idx = rotated_x - min_dst_x_f;
                let dst_y_idx = rotated_y - min_dst_y_f;

                // Check if rotated pixel is within destination bounds
                if dst_x_idx < 0.0 || dst_x_idx >= dst_w || dst_y_idx < 0.0 || dst_y_idx >= dst_h {
                    continue;
                }

                // Map destination pixel to source coordinate with scaling
                let src_f_x = src_left_f + (dst_x_idx + 0.5) * scale_x;
                let src_f_y = src_top_f + (dst_y_idx + 0.5) * scale_y;

                // Handle horizontal flip
                let src_mapped_x = if flip_x {
                    let rel = src_f_x - src_left_f;
                    src_left_f + src_w - rel
                } else {
                    src_f_x
                };

                // Handle vertical flip
                let src_mapped_y = if flip_y {
                    let rel = src_f_y - src_top_f;
                    src_top_f + src_h - rel
                } else {
                    src_f_y
                };

                // Convert to integer sample coordinates with flooring and clamping
                let sx = src_mapped_x.floor() as i32;
                let sy = src_mapped_y.floor() as i32;

                let src_max_x = src_rect.right - 1;
                let src_max_y = src_rect.bottom - 1;

                if src_rect.left > src_max_x || src_rect.top > src_max_y {
                    continue;
                }

                let sx = sx.clamp(src_rect.left, src_max_x) as u16;
                let sy = sy.clamp(src_rect.top, src_max_y) as u16;

                // check if its in the boundaries
                if sx >= src.width || sy >= src.height {
                    continue;
                }

                // Indexed bitmap (1-8 bit) ink 0
                if ink == 0 && is_indexed {
                    // Check matte mask for trimWhiteSpace transparency
                    // mask: true = opaque (draw), false = transparent (skip)
                    if let Some(mask) = mask_image {
                        if !mask.get_bit(sx, sy) {
                            continue; // Edge-connected background pixel - skip
                        }
                    }

                    let color_ref = src.get_pixel_color_ref(sx, sy);
                    let (sr, sg, sb) = if let (ColorRef::PaletteIndex(i), Some(cache)) = (&color_ref, &src_palette_cache) {
                        cache[*i as usize]
                    } else {
                        resolve_color_ref(palettes, &color_ref, &src.palette_ref, src.original_bit_depth)
                    };

                    self.set_pixel_fast(dst_x, dst_y, (sr, sg, sb), &dst_palette_cache);
                    continue;
                }

                // Indexed bitmap (1-8 bit) ink 36 color-key transparency
                if (ink == 2 || ink == 36) && is_indexed {
                    let color_ref = src.get_pixel_color_ref(sx, sy);
                    let ColorRef::PaletteIndex(i) = color_ref else {
                        let (sr, sg, sb) = match &color_ref {
                            ColorRef::Rgb(r, g, b) => (*r, *g, *b),
                            _ => continue,
                        };
                        if (sr, sg, sb) == bg_color_resolved {
                            continue;
                        }
                        let dst_color = if !dst_palette_cache.is_empty() { self.get_pixel_color_fast(&dst_palette_cache, dst_x as u16, dst_y as u16) } else { self.get_pixel_color(palettes, dst_x as u16, dst_y as u16) };
                        let blended = if alpha >= 0.999 {
                            (sr, sg, sb)
                        } else {
                            blend_color_alpha(dst_color, (sr, sg, sb), alpha)
                        };
                        self.set_pixel_fast(dst_x, dst_y, blended, &dst_palette_cache);
                        continue;
                    };

                    // For 1-bit bitmaps: use strict index-based transparency only
                    // Index 0 (bit=0) = background → transparent
                    // Index 255 (bit=1) = foreground → render with foreColor
                    // This is important when foreColor and bgColor resolve to the same RGB
                    // (e.g., both white) - we still want foreground pixels to render.
                    if src.original_bit_depth == 1 {
                        if i == 0 {
                            continue; // Background bit → transparent
                        }
                        // Foreground bit → render with foreColor
                        let src_color = fg_color_resolved;
                        let dst_color = if !dst_palette_cache.is_empty() { self.get_pixel_color_fast(&dst_palette_cache, dst_x as u16, dst_y as u16) } else { self.get_pixel_color(palettes, dst_x as u16, dst_y as u16) };

                        let blended = if alpha >= 0.999 {
                            src_color
                        } else {
                            blend_color_alpha(dst_color, src_color, alpha)
                        };

                        self.set_pixel_fast(dst_x, dst_y, blended, &dst_palette_cache);
                        continue;
                    }

                    let transparent_index = if src.original_bit_depth <= 4 {
                        0 // Director rule for ≤4-bit
                    } else {
                        bg_index // 8-bit
                    };

                    // Resolve color via cached palette table
                    let (r, g, b) = if let Some(cache) = &src_palette_cache {
                        cache[i as usize]
                    } else {
                        resolve_color_ref(palettes, &ColorRef::PaletteIndex(i), &src.palette_ref, src.original_bit_depth)
                    };

                    // Fast path: check index match first
                    if i == transparent_index && (r, g, b) == bg_color_resolved {
                        continue;
                    }

                    if (r, g, b) == bg_color_resolved {
                        continue; // transparent - RGB matches background color
                    }

                    // If alpha channel disabled → fully opaque
                    let src_alpha = 1.0;

                    // For monochrome-style bitmaps (black content on white bg) with ink 36,
                    // the foreground pixels should be tinted with the sprite's foreColor.
                    // This allows white foreColor to make black numbers appear white.
                    // Check if pixel is "foreground" (index 255 or black color)
                    let src_color = if i == 255 || (r, g, b) == (0, 0, 0) {
                        fg_color_resolved // Tint foreground with sprite's foreColor
                    } else {
                        (r, g, b) // Keep original color for other pixels
                    };

                    let dst_color = if !dst_palette_cache.is_empty() { self.get_pixel_color_fast(&dst_palette_cache, dst_x as u16, dst_y as u16) } else { self.get_pixel_color(palettes, dst_x as u16, dst_y as u16) };

                    let blended = if src_alpha >= 0.999 && alpha >= 0.999 {
                        src_color
                    } else {
                        blend_color_alpha(dst_color, src_color, src_alpha * alpha)
                    };

                    self.set_pixel_fast(dst_x, dst_y, blended, &dst_palette_cache);
                    continue;
                }

                // 16-bit bitmap ink 36 color-key transparency
                // 16-bit is stored as 32-bit RGB, so compare RGB values directly
                if (ink == 2 || ink == 36) && src.original_bit_depth == 16 {
                    let (r, g, b, _) = src.get_pixel_color_with_alpha(palettes, sx, sy);

                    // Skip pixel if it matches the sprite's bgColor
                    if (r, g, b) == bg_color_resolved {
                        continue; // transparent - RGB matches background color
                    }

                    let src_color = (r, g, b);
                    let dst_color = if !dst_palette_cache.is_empty() { self.get_pixel_color_fast(&dst_palette_cache, dst_x as u16, dst_y as u16) } else { self.get_pixel_color(palettes, dst_x as u16, dst_y as u16) };

                    let blended = if alpha >= 0.999 {
                        src_color
                    } else {
                        blend_color_alpha(dst_color, src_color, alpha)
                    };

                    self.set_pixel_fast(dst_x, dst_y, blended, &dst_palette_cache);
                    continue;
                }

                // 32-bit bitmap ink 36 color-key transparency
                // PFR font bitmaps are decoded to 32-bit RGBA; background is white, glyphs are black.
                if (ink == 2 || ink == 36) && src.original_bit_depth == 32 {
                    let (r, g, b, a) = src.get_pixel_color_with_alpha(palettes, sx, sy);

                    // Skip fully transparent pixels (use_alpha bitmaps like text member images)
                    if src.use_alpha && a == 0 {
                        continue;
                    }

                    // Skip pixel if it matches the sprite's bgColor (transparent background)
                    if (r, g, b) == bg_color_resolved {
                        continue;
                    }

                    // Colorize foreground (black/dark) pixels with the sprite's foreColor
                    let src_color = if (r, g, b) == (0, 0, 0) {
                        fg_color_resolved
                    } else {
                        (r, g, b)
                    };

                    // For text rendering to intermediate bitmap: write colorized RGB with
                    // per-pixel alpha directly. set_pixel always writes alpha=255 which
                    // destroys anti-aliasing information needed by WebGL2 compositing.
                    if params.is_text_rendering && src.use_alpha && a < 255 && self.bit_depth == 32 {
                        if dst_x >= 0 && dst_y >= 0 && dst_x < self.width as i32 && dst_y < self.height as i32 {
                            let idx = (dst_y as usize * self.width as usize + dst_x as usize) * 4;
                            if idx + 3 < self.data.len() {
                                let (sr, sg, sb) = src_color;
                                let dest_a = self.data[idx + 3];
                                if dest_a == 0 {
                                    self.data[idx] = sr;
                                    self.data[idx + 1] = sg;
                                    self.data[idx + 2] = sb;
                                    self.data[idx + 3] = a;
                                } else {
                                    // Composite src over dest using "over" operator
                                    let sa = a as f32 / 255.0;
                                    let da = dest_a as f32 / 255.0;
                                    let out_a = sa + da * (1.0 - sa);
                                    if out_a > 0.001 {
                                        let out_r = (sr as f32 * sa + self.data[idx] as f32 * da * (1.0 - sa)) / out_a;
                                        let out_g = (sg as f32 * sa + self.data[idx + 1] as f32 * da * (1.0 - sa)) / out_a;
                                        let out_b = (sb as f32 * sa + self.data[idx + 2] as f32 * da * (1.0 - sa)) / out_a;
                                        self.data[idx] = out_r.round().min(255.0) as u8;
                                        self.data[idx + 1] = out_g.round().min(255.0) as u8;
                                        self.data[idx + 2] = out_b.round().min(255.0) as u8;
                                        self.data[idx + 3] = (out_a * 255.0).round().min(255.0) as u8;
                                    }
                                }
                            }
                        }
                        continue;
                    }

                    let dst_color = if !dst_palette_cache.is_empty() { self.get_pixel_color_fast(&dst_palette_cache, dst_x as u16, dst_y as u16) } else { self.get_pixel_color(palettes, dst_x as u16, dst_y as u16) };

                    let blended = if alpha >= 0.999 {
                        src_color
                    } else {
                        blend_color_alpha(dst_color, src_color, alpha)
                    };

                    self.set_pixel_fast(dst_x, dst_y, blended, &dst_palette_cache);
                    continue;
                }

                // Skip mask check for ink 33 - it handles transparency via bgColor check
                if ink != 33 {
                    if let Some(mask) = mask_image {
                        if !mask.get_bit(sx, sy) {
                            continue;
                        }
                    }
                }

                // 16-bit bitmap ink 0 (copy with matte for trimWhiteSpace)
                // Uses matte mask created from edge-connected white pixels
                if ink == 0 && src.original_bit_depth == 16 {
                    // Matte mask check already done above via mask_image
                    let (r, g, b, _) = src.get_pixel_color_with_alpha(palettes, sx, sy);

                    let src_color = (r, g, b);
                    let dst_color = if !dst_palette_cache.is_empty() { self.get_pixel_color_fast(&dst_palette_cache, dst_x as u16, dst_y as u16) } else { self.get_pixel_color(palettes, dst_x as u16, dst_y as u16) };

                    let blended = if alpha >= 0.999 {
                        src_color
                    } else {
                        blend_color_alpha(dst_color, src_color, alpha)
                    };

                    self.set_pixel_fast(dst_x, dst_y, blended, &dst_palette_cache);
                    continue;
                }

                // Indexed bitmap (1-8 bit) ink 8
                if ink == 8 && is_indexed {
                    let color_ref = src.get_pixel_color_ref(sx, sy);
                    let (sr, sg, sb) = if let (ColorRef::PaletteIndex(i), Some(cache)) = (&color_ref, &src_palette_cache) {
                        cache[*i as usize]
                    } else {
                        resolve_color_ref(palettes, &color_ref, &src.palette_ref, src.original_bit_depth)
                    };

                    // Check matte mask - only edge-connected bg pixels are transparent
                    if let Some(mask) = &matte_mask {
                        if mask[sy as usize][sx as usize] {
                            continue; // This pixel is transparent
                        }
                    }

                    // If alpha channel disabled → fully opaque
                    let src_alpha = 1.0;

                    let src_color = (sr, sg, sb);
                    let dst_color = if !dst_palette_cache.is_empty() { self.get_pixel_color_fast(&dst_palette_cache, dst_x as u16, dst_y as u16) } else { self.get_pixel_color(palettes, dst_x as u16, dst_y as u16) };

                    let blended = if src_alpha >= 0.999 && alpha >= 0.999 {
                        src_color
                    } else {
                        blend_color_alpha(dst_color, src_color, src_alpha * alpha)
                    };

                    self.set_pixel_fast(dst_x, dst_y, blended, &dst_palette_cache);
                    continue;
                }

                // Sample source pixel
                let (sr, sg, sb, mut sa) = if let Some(cache) = &src_palette_cache {
                    src.get_pixel_color_with_alpha_fast(cache, sx, sy)
                } else {
                    src.get_pixel_color_with_alpha(palettes, sx, sy)
                };

                // Skip fully transparent pixels from RGBA bitmaps (e.g., filmloop compositing)
                // This ensures transparent areas don't overwrite destination with black
                if src.original_bit_depth == 32 && src.use_alpha && sa == 0 {
                    continue;
                }

                let mut src_color = (sr, sg, sb);

                // ----------------------------------------------------------
                // DIRECTOR COLORIZE (foreColor / backColor tweening)
                // ----------------------------------------------------------
                if let Some(sprite) = params.sprite {
                    if Self::allows_colorize(src.original_bit_depth, ink, params.is_text_rendering) {
                        let has_fg = sprite.has_fore_color;
                        let has_bg = sprite.has_back_color;

                        if has_fg || has_bg {
                            match src.original_bit_depth {
                                // ---------- 32-BIT ----------
                                32 => {
                                    // Treat source as grayscale intensity
                                    let gray = ((sr as u16 + sg as u16 + sb as u16) / 3) as u8;

                                    if has_fg && has_bg && Self::uses_back_color(32, ink) {
                                        let t = gray as f32 / 255.0;
                                        src_color = (
                                            ((1.0 - t) * fg_color_resolved.0 as f32
                                                + t * bg_color_resolved.0 as f32) as u8,
                                            ((1.0 - t) * fg_color_resolved.1 as f32
                                                + t * bg_color_resolved.1 as f32) as u8,
                                            ((1.0 - t) * fg_color_resolved.2 as f32
                                                + t * bg_color_resolved.2 as f32) as u8,
                                        );
                                    } else if has_fg && gray <= 1 {
                                        src_color = fg_color_resolved;
                                    }
                                }

                                // ---------- INDEXED (≤8-bit) ----------
                                _ => {
                                    // Palette index based semantics
                                    let color_ref = src.get_pixel_color_ref(sx, sy);

                                    if let ColorRef::PaletteIndex(i) = color_ref {
                                        let max = (1 << src.original_bit_depth) - 1;
                                        let t = i as f32 / max as f32;

                                        if has_fg && has_bg && Self::uses_back_color(src.original_bit_depth, ink) {
                                            src_color = (
                                                ((1.0 - t) * fg_color_resolved.0 as f32
                                                    + t * bg_color_resolved.0 as f32) as u8,
                                                ((1.0 - t) * fg_color_resolved.1 as f32
                                                    + t * bg_color_resolved.1 as f32) as u8,
                                                ((1.0 - t) * fg_color_resolved.2 as f32
                                                    + t * bg_color_resolved.2 as f32) as u8,
                                            );
                                        } else if has_fg && i == 0 {
                                            src_color = fg_color_resolved;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if src.original_bit_depth == 32 && ink == 0 && !params.is_text_rendering {
                    if !src.use_alpha {
                        sa = 255;
                    } else if sa == 0 {
                        continue;
                    }
                }

                if src.original_bit_depth == 32 && ink == 8 && !params.is_text_rendering {
                    if !src.use_alpha {
                        if let Some(mask) = &matte_mask {
                            if mask[sy as usize][sx as usize] {
                                continue;
                            }
                        }
                        sa = 255;
                    }

                    let src_alpha = if src.use_alpha {
                        sa as f32 / 255.0
                    } else {
                        1.0
                    };

                    let mut src_color = (sr, sg, sb);

                    if let Some(sprite) = params.sprite {
                        if sprite.has_fore_color && fg_color_resolved != (0, 0, 0) {
                            src_color = Self::apply_forecolor_tint(src_color, fg_color_resolved);
                        }
                    }

                    let dst_color =
                        if !dst_palette_cache.is_empty() { self.get_pixel_color_fast(&dst_palette_cache, dst_x as u16, dst_y as u16) } else { self.get_pixel_color(palettes, dst_x as u16, dst_y as u16) };

                    let blended = if src_alpha >= 0.999 && alpha >= 0.999 {
                        src_color
                    } else {
                        blend_color_alpha(dst_color, src_color, src_alpha * alpha)
                    };

                    self.set_pixel_fast(dst_x, dst_y, blended, &dst_palette_cache);
                    continue;
                }

                // ----------------------------------------------------------
                // Director ink 36 (Blend) alpha semantics
                // ----------------------------------------------------------
                if (ink == 2 || ink == 36) && sa == 0 && src.original_bit_depth == 32 {
                    if (sr, sg, sb) == bg_color_resolved {
                        continue;
                    }

                    sa = 255;
                }

                // ----------------------------------------------------------
                // 1. Skip background transparent ink
                // ----------------------------------------------------------
                if !params.is_text_rendering
                    && sa == 255
                    && (ink == 2 || ink == 36)
                    && (sr, sg, sb) == bg_color_resolved
                {
                    continue; // This pixel is background → transparent
                }

                // ----------------------------------------------------------
                // 2. Matte / Mask grayscale white = transparent
                // ----------------------------------------------------------
                if !params.is_text_rendering
                    && (ink == 8 || ink == 9)
                    && use_grayscale_as_alpha
                    && src.original_bit_depth <= 8
                    && (sr, sg, sb) == (255, 255, 255)
                {
                    continue;
                }

                // ----------------------------------------------------------
                // 3. TEXT RENDERING MODE
                // ----------------------------------------------------------
                if params.is_text_rendering {
                    // Black pixel → foreground color
                    if (sr, sg, sb) == (0, 0, 0) {
                        let dst_color =
                            if !dst_palette_cache.is_empty() { self.get_pixel_color_fast(&dst_palette_cache, dst_x as u16, dst_y as u16) } else { self.get_pixel_color(palettes, dst_x as u16, dst_y as u16) };
                        let blended = blend_pixel(
                            dst_color,
                            fg_color_resolved,
                            ink,
                            bg_color_resolved,
                            alpha,
                            sa as f32 / 255.0,
                        );
                        self.set_pixel_fast(dst_x, dst_y, blended, &dst_palette_cache);
                    }

                    // White pixel → FULLY TRANSPARENT → skip
                    continue;
                }

                // Blend and write destination pixel

                // ----------------------------------------------------------
                // 4. NON-TEXT normal rendering
                // ----------------------------------------------------------
                let src_alpha = sa as f32 / 255.0;
                let dst_color = if !dst_palette_cache.is_empty() { self.get_pixel_color_fast(&dst_palette_cache, dst_x as u16, dst_y as u16) } else { self.get_pixel_color(palettes, dst_x as u16, dst_y as u16) };

                let blended = blend_pixel(
                    dst_color,
                    src_color,
                    ink,
                    bg_color_resolved,
                    alpha,
                    src_alpha,
                );

                self.set_pixel_fast(dst_x, dst_y, blended, &dst_palette_cache);
            }
        }
        // Uncomment below to debug copyPixel calls
        // self.stroke_rect(draw_min_x, draw_min_y, draw_max_x, draw_max_y, (0, 255, 0), palettes, 1.0);
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
        self.copy_pixels(palettes, bitmap, dst_rect, src_rect, &params, None);
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

            // Use per-character advance width when available (PFR/proportional fonts)
            x += font.get_char_advance(char_num as u8) as i32;
        }
    }

    /// Draw text with word wrapping. Returns the number of lines drawn.
    pub fn draw_text_wrapped(
        &mut self,
        text: &str,
        font: &BitmapFont,
        font_bitmap: &Bitmap,
        loc_h: i32,
        loc_v: i32,
        max_width: i32,
        alignment: &str,
        params: CopyPixelsParams,
        palettes: &PaletteMap,
        line_spacing: u16,
        top_spacing: i16,
    ) -> i32 {
        let line_height = font.char_height as i32 + line_spacing as i32;

        // Break text into wrapped lines
        let lines = Self::wrap_text_lines(text, font, max_width);

        let mut y = loc_v + top_spacing as i32;
        for line in &lines {
            // Calculate x based on alignment
            let line_w: i32 = line.chars()
                .map(|ch| font.get_char_advance(ch as u8) as i32)
                .sum();
            let x = match alignment {
                "center" => loc_h + ((max_width - line_w) / 2).max(0),
                "right" => loc_h + (max_width - line_w).max(0),
                _ => loc_h,
            };

            // Draw each character
            let mut cx = x;
            for ch in line.chars() {
                bitmap_font_copy_char(
                    font, font_bitmap, ch as u8,
                    self, cx, y, palettes, &params,
                );
                cx += font.get_char_advance(ch as u8) as i32;
            }
            y += line_height;
        }
        lines.len() as i32
    }

    /// Break text into lines that fit within max_width, wrapping at word boundaries.
    pub fn wrap_text_lines(text: &str, font: &BitmapFont, max_width: i32) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();

        for paragraph in text.split(|c| c == '\r' || c == '\n') {
            if max_width <= 0 {
                lines.push(paragraph.to_string());
                continue;
            }

            let words: Vec<&str> = paragraph.split(' ').collect();
            let mut current_line = String::new();
            let mut current_width: i32 = 0;
            let space_width = font.get_char_advance(b' ') as i32;

            for (i, word) in words.iter().enumerate() {
                let word_width: i32 = word.chars()
                    .map(|ch| font.get_char_advance(ch as u8) as i32)
                    .sum();

                let needed = if current_line.is_empty() {
                    word_width
                } else {
                    space_width + word_width
                };

                if !current_line.is_empty() && current_width + needed > max_width {
                    // Wrap: push current line and start new one
                    lines.push(current_line);
                    current_line = word.to_string();
                    current_width = word_width;
                } else {
                    if !current_line.is_empty() {
                        current_line.push(' ');
                        current_width += space_width;
                    }
                    current_line.push_str(word);
                    current_width += word_width;
                }
            }
            lines.push(current_line);
        }

        lines
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

    /// Draw a vector shape with proper type dispatch and ink/blend support.
    /// Renders the shape to a temporary bitmap, then copy_pixels onto destination
    /// to properly handle Director ink modes.
    /// Draw a vector shape directly onto this bitmap with ink/blend support.
    pub fn draw_shape_with_sprite(
        &mut self,
        sprite: &crate::player::sprite::Sprite,
        shape_info: &ShapeInfo,
        dst_rect: IntRect,
        palettes: &PaletteMap,
        palette_ref: &PaletteRef,
    ) {
        let x1 = dst_rect.left;
        let y1 = dst_rect.top;
        let x2 = dst_rect.right;
        let y2 = dst_rect.bottom;

        // Resolve foreground color
        let fg_rgb = resolve_color_ref(
            palettes,
            &sprite.color,
            palette_ref,
            self.original_bit_depth,
        );

        let filled = shape_info.fill_type != 0;
        // Per ScummVM: for outlined shapes, line_thickness of 1 means invisible (subtract 1)
        let thickness = if filled {
            // Filled shapes: outline is optional, drawn at actual thickness
            shape_info.line_thickness as i32
        } else {
            // Outlined shapes: thickness 1 = invisible per ScummVM convention
            (shape_info.line_thickness as i32) - 1
        };

        // Convert blend percentage to alpha (0.0-1.0)
        let alpha = (sprite.blend as f32 / 100.0).clamp(0.0, 1.0);

        // For ink=copy (0), draw directly onto destination bitmap with alpha blending.
        // For other inks, use temp bitmap + copy_pixels for proper ink handling.
        let use_direct = sprite.ink == 0;

        if use_direct {
            // Direct drawing onto destination bitmap
            match shape_info.shape_type {
                ShapeType::Rect => {
                    if filled {
                        self.fill_rect(x1, y1, x2, y2, fg_rgb, palettes, alpha);
                    }
                    if thickness > 0 {
                        for t in 0..thickness {
                            self.stroke_rect(x1 + t, y1 + t, x2 - t, y2 - t, fg_rgb, palettes, alpha);
                        }
                    }
                }
                ShapeType::OvalRect => {
                    let radius = 12;
                    if filled {
                        self.fill_round_rect(x1, y1, x2, y2, radius, fg_rgb, palettes, alpha);
                    }
                    if thickness > 0 {
                        self.stroke_round_rect(x1, y1, x2, y2, radius, fg_rgb, palettes, alpha, thickness);
                    }
                }
                ShapeType::Oval => {
                    if filled {
                        self.fill_ellipse(x1, y1, x2, y2, fg_rgb, palettes, alpha);
                    }
                    if thickness > 0 {
                        self.stroke_ellipse(x1, y1, x2, y2, fg_rgb, palettes, alpha, thickness);
                    }
                }
                ShapeType::Line => {
                    let t = (shape_info.line_thickness as i32).max(1);
                    if shape_info.line_direction == 6 {
                        self.draw_line_thick(x1, y2 - 1, x2 - 1, y1, fg_rgb, palettes, alpha, t);
                    } else {
                        self.draw_line_thick(x1, y1, x2 - 1, y2 - 1, fg_rgb, palettes, alpha, t);
                    }
                }
                ShapeType::Unknown => {
                    self.fill_rect(x1, y1, x2, y2, fg_rgb, palettes, alpha);
                }
            }
        } else {
            // Non-copy ink: use temp bitmap + copy_pixels for proper ink handling
            let w = (x2 - x1).max(1);
            let h = (y2 - y1).max(1);
            let mut temp = Bitmap::new(
                w as u16, h as u16,
                self.bit_depth, self.original_bit_depth, 0,
                self.palette_ref.clone(),
            );

            match shape_info.shape_type {
                ShapeType::Rect | ShapeType::Unknown => {
                    if filled {
                        temp.fill_rect(0, 0, w, h, fg_rgb, palettes, 1.0);
                    }
                    if thickness > 0 {
                        for t in 0..thickness {
                            temp.stroke_rect(t, t, w - t, h - t, fg_rgb, palettes, 1.0);
                        }
                    }
                }
                ShapeType::OvalRect => {
                    let radius = 12;
                    if filled {
                        temp.fill_round_rect(0, 0, w, h, radius, fg_rgb, palettes, 1.0);
                    }
                    if thickness > 0 {
                        temp.stroke_round_rect(0, 0, w, h, radius, fg_rgb, palettes, 1.0, thickness);
                    }
                }
                ShapeType::Oval => {
                    if filled {
                        temp.fill_ellipse(0, 0, w, h, fg_rgb, palettes, 1.0);
                    }
                    if thickness > 0 {
                        temp.stroke_ellipse(0, 0, w, h, fg_rgb, palettes, 1.0, thickness);
                    }
                }
                ShapeType::Line => {
                    let t = (shape_info.line_thickness as i32).max(1);
                    if shape_info.line_direction == 6 {
                        temp.draw_line_thick(0, h - 1, w - 1, 0, fg_rgb, palettes, 1.0, t);
                    } else {
                        temp.draw_line_thick(0, 0, w - 1, h - 1, fg_rgb, palettes, 1.0, t);
                    }
                }
            }

            let mut params = HashMap::new();
            params.insert("blend".into(), Datum::Int(sprite.blend as i32));
            params.insert("ink".into(), Datum::Int(sprite.ink as i32));
            params.insert("color".into(), Datum::ColorRef(sprite.color.clone()));
            params.insert("bgColor".into(), Datum::ColorRef(sprite.bg_color.clone()));

            self.copy_pixels(
                palettes,
                &temp,
                dst_rect,
                IntRect::from_tuple((0, 0, w, h)),
                &params,
                None,
            );
        }
    }

    /// Legacy wrapper for backward compatibility — calls draw_shape_with_sprite with default rect shape.
    pub fn fill_shape_rect_with_sprite(
        &mut self,
        sprite: &crate::player::sprite::Sprite,
        dst_rect: IntRect,
        palettes: &PaletteMap,
        palette_ref: &PaletteRef,
    ) {
        let w = (dst_rect.right - dst_rect.left).max(1);
        let h = (dst_rect.bottom - dst_rect.top).max(1);
        // Default to filled rect shape
        let default_shape = ShapeInfo {
            shape_type: ShapeType::Rect,
            rect_top: 0,
            rect_left: 0,
            rect_bottom: h as i16,
            rect_right: w as i16,
            pattern: 0,
            fore_color: 0,
            back_color: 0,
            fill_type: 1,
            line_thickness: 0,
            line_direction: 0,
        };
        self.draw_shape_with_sprite(sprite, &default_shape, dst_rect, palettes, palette_ref);
    }
}
