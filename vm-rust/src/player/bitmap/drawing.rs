use std::collections::HashMap;

use nohash_hasher::IntMap;
use rgb565::Rgb565;

use crate::{
    director::lingo::datum::Datum,
    player::{
        font::{bitmap_font_copy_char, BitmapFont},
        geometry::IntRect,
        sprite::{ColorRef, is_skew_flip},
        bitmap::bitmap::{get_system_default_palette, PaletteRef},
        bitmap::palette::SYSTEM_WIN_PALETTE, Sprite, Score,
        reserve_player_mut,
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
    ink == 36 || ink == 33 || ink == 41 || ink == 8 || ink == 7
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
        let mask_image = mask_image.map(|x| x.to_mask().unwrap());

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
        let bg_color_resolved = if src.original_bit_depth == 32 && !src.use_alpha && ink != 0 {
            match &params.bg_color {
                ColorRef::Rgb(r, g, b) => (*r, *g, *b),
                ColorRef::PaletteIndex(_) => {
                    // Director behavior: palette indices are ignored for 32-bit bgColor
                    (255, 255, 255)
                }
            }
        } else {
            resolve_color_ref(
                palettes,
                &params.bg_color,
                &src.palette_ref,
                src.original_bit_depth,
            )
        };

        let fg_color_resolved = resolve_color_ref(
            palettes,
            &params.color,
            &src.palette_ref,
            src.original_bit_depth,
        );

        let is_indexed = src.original_bit_depth <= 8;

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
            && (ink == 8 || ink == 0)
            && is_matte_bitmap
            && (src.original_bit_depth <= 8 || src.original_bit_depth == 32);

        let mut matte_mask: Option<Vec<Vec<bool>>> = None;

        // ----------------------------------------------------------
        // 32-bit matte key: use edge color, NOT backColor
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

            // ---- seed flood fill from edges ----
            for x in 0..width {
                let (r1, g1, b1, _) =
                    src.get_pixel_color_with_alpha(palettes, x as u16, 0);

                let is_bg = if let Some(edge) = edge_matte_color {
                    (r1, g1, b1) == edge
                } else {
                    (r1, g1, b1) == bg_color_resolved
                };

                if is_bg {
                    stack.push((x, 0));
                }

                let (r2, g2, b2, _) =
                    src.get_pixel_color_with_alpha(palettes, x as u16, (height - 1) as u16);

                let is_bg = if let Some(edge) = edge_matte_color {
                    (r2, g2, b2) == edge
                } else {
                    (r2, g2, b2) == bg_color_resolved
                };

                if is_bg {
                    stack.push((x, height - 1));
                }
            }

            for y in 0..height {
                let (r1, g1, b1, _) =
                    src.get_pixel_color_with_alpha(palettes, 0, y as u16);

                let is_bg = if let Some(edge) = edge_matte_color {
                    (r1, g1, b1) == edge
                } else {
                    (r1, g1, b1) == bg_color_resolved
                };

                if is_bg {
                    stack.push((0, y));
                }

                let (r2, g2, b2, _) =
                    src.get_pixel_color_with_alpha(palettes, (width - 1) as u16, y as u16);

                let is_bg = if let Some(edge) = edge_matte_color {
                    (r2, g2, b2) == edge
                } else {
                    (r2, g2, b2) == bg_color_resolved
                };

                if is_bg {
                    stack.push((width - 1, y));
                }
            }

            // ---- flood fill ----
            while let Some((x, y)) = stack.pop() {
                if mask[y][x] {
                    continue;
                }

                let (r, g, b, _) =
                    src.get_pixel_color_with_alpha(palettes, x as u16, y as u16);

                let is_bg = if let Some(edge) = edge_matte_color {
                    (r, g, b) == edge
                } else {
                    (r, g, b) == bg_color_resolved
                };

                if !is_bg {
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

                    let (sr, sg, sb) = resolve_color_ref(
                        palettes,
                        &color_ref,
                        &src.palette_ref,
                        src.original_bit_depth,
                    );

                    self.set_pixel(dst_x, dst_y, (sr, sg, sb), palettes);
                    continue;
                }

                // Indexed bitmap (1-8 bit) ink 36 color-key transparency
                if ink == 36 && is_indexed {
                    let ColorRef::PaletteIndex(i) = src.get_pixel_color_ref(sx, sy) else {
                        unreachable!("indexed bitmap returned non-index color");
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
                        let dst_color = self.get_pixel_color(palettes, dst_x as u16, dst_y as u16);

                        let blended = if alpha >= 0.999 {
                            src_color
                        } else {
                            blend_color_alpha(dst_color, src_color, alpha)
                        };

                        self.set_pixel(dst_x, dst_y, blended, palettes);
                        continue;
                    }

                    let transparent_index = if src.original_bit_depth <= 4 {
                        0 // Director rule for ≤4-bit
                    } else {
                        bg_index // 8-bit
                    };

                    // Fast path: check index match first
                    if i == transparent_index {
                        let (r, g, b) = resolve_color_ref(
                            palettes,
                            &ColorRef::PaletteIndex(i),
                            &src.palette_ref,
                            src.original_bit_depth,
                        );

                        if (r, g, b) == bg_color_resolved {
                            continue;
                        }
                    }

                    // Resolve both colors and compare RGB values
                    // (handles case where background color exists at multiple palette indices)
                    let (r, g, b) = resolve_color_ref(
                        palettes,
                        &ColorRef::PaletteIndex(i),
                        &src.palette_ref,
                        src.original_bit_depth,
                    );

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

                    let dst_color = self.get_pixel_color(palettes, dst_x as u16, dst_y as u16);

                    let blended = if src_alpha >= 0.999 && alpha >= 0.999 {
                        src_color
                    } else {
                        blend_color_alpha(dst_color, src_color, src_alpha * alpha)
                    };

                    self.set_pixel(dst_x, dst_y, blended, palettes);
                    continue;
                }

                // 16-bit bitmap ink 36 color-key transparency
                // 16-bit is stored as 32-bit RGB, so compare RGB values directly
                if ink == 36 && src.original_bit_depth == 16 {
                    let (r, g, b, _) = src.get_pixel_color_with_alpha(palettes, sx, sy);

                    // Skip pixel if it matches the sprite's bgColor
                    if (r, g, b) == bg_color_resolved {
                        continue; // transparent - RGB matches background color
                    }

                    let src_color = (r, g, b);
                    let dst_color = self.get_pixel_color(palettes, dst_x as u16, dst_y as u16);

                    let blended = if alpha >= 0.999 {
                        src_color
                    } else {
                        blend_color_alpha(dst_color, src_color, alpha)
                    };

                    self.set_pixel(dst_x, dst_y, blended, palettes);
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
                    let dst_color = self.get_pixel_color(palettes, dst_x as u16, dst_y as u16);

                    let blended = if alpha >= 0.999 {
                        src_color
                    } else {
                        blend_color_alpha(dst_color, src_color, alpha)
                    };

                    self.set_pixel(dst_x, dst_y, blended, palettes);
                    continue;
                }

                // Indexed bitmap (1-8 bit) ink 8
                if ink == 8 && is_indexed {
                    let color_ref = src.get_pixel_color_ref(sx, sy);

                    let (sr, sg, sb) = resolve_color_ref(
                        palettes,
                        &color_ref,
                        &src.palette_ref,
                        src.original_bit_depth,
                    );

                    // Check matte mask - only edge-connected bg pixels are transparent
                    if let Some(mask) = &matte_mask {
                        if mask[sy as usize][sx as usize] {
                            continue; // This pixel is transparent
                        }
                    }

                    // If alpha channel disabled → fully opaque
                    let src_alpha = 1.0;

                    let src_color = (sr, sg, sb);
                    let dst_color = self.get_pixel_color(palettes, dst_x as u16, dst_y as u16);

                    let blended = if src_alpha >= 0.999 && alpha >= 0.999 {
                        src_color
                    } else {
                        blend_color_alpha(dst_color, src_color, src_alpha * alpha)
                    };

                    self.set_pixel(dst_x, dst_y, blended, palettes);
                    continue;
                }

                // Sample source pixel
                let (sr, sg, sb, mut sa) =
                    src.get_pixel_color_with_alpha(palettes, sx, sy);

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

                    if src.trim_white_space && (sr, sg, sb) == (255, 255, 255) {
                        if let Some(mask) = &matte_mask {
                            if mask[sy as usize][sx as usize] {
                                continue; // This pixel is transparent
                            }
                        }
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
                        self.get_pixel_color(palettes, dst_x as u16, dst_y as u16);

                    let blended = if src_alpha >= 0.999 && alpha >= 0.999 {
                        src_color
                    } else {
                        blend_color_alpha(dst_color, src_color, src_alpha * alpha)
                    };

                    self.set_pixel(dst_x, dst_y, blended, palettes);
                    continue;
                }

                // ----------------------------------------------------------
                // Director ink 36 (Blend) alpha semantics
                // ----------------------------------------------------------
                if ink == 36 && sa == 0 && src.original_bit_depth == 32 {
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
                    && ink == 36
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
                            self.get_pixel_color(palettes, dst_x as u16, dst_y as u16);
                        let blended = blend_pixel(
                            dst_color,
                            fg_color_resolved,
                            ink,
                            bg_color_resolved,
                            alpha,
                            sa as f32 / 255.0,
                        );
                        self.set_pixel(dst_x, dst_y, blended, palettes);
                    }

                    // White pixel → FULLY TRANSPARENT → skip
                    continue;
                }

                // Blend and write destination pixel

                // ----------------------------------------------------------
                // 4. NON-TEXT normal rendering
                // ----------------------------------------------------------
                let src_alpha = sa as f32 / 255.0;
                let dst_color = self.get_pixel_color(palettes, dst_x as u16, dst_y as u16);

                let blended = blend_pixel(
                    dst_color,
                    src_color,
                    ink,
                    bg_color_resolved,
                    alpha,
                    src_alpha,
                );

                self.set_pixel(dst_x, dst_y, blended, palettes);
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

    pub fn fill_shape_rect_with_sprite(
        &mut self,
        sprite: &crate::player::sprite::Sprite,
        dst_rect: IntRect,
        palettes: &PaletteMap,
    ) {
        // Create a temporary 1×1 bitmap representing the foreground color
        let mut temp = Bitmap::new(
            1,
            1,
            self.bit_depth,
            self.original_bit_depth,
            0,
            self.palette_ref.clone(),
        );

        // Resolve sprite.color (foreground)
        let fg_rgb = resolve_color_ref(
            palettes,
            &sprite.color,
            &PaletteRef::BuiltIn(get_system_default_palette()),
            self.original_bit_depth,
        );
        temp.set_pixel(0, 0, fg_rgb, palettes);

        // Build Director-style copy_pixels parameters
        let mut params = HashMap::new();
        params.insert("blend".into(), Datum::Int(sprite.blend as i32));
        params.insert("ink".into(), Datum::Int(sprite.ink as i32));
        params.insert("color".into(), Datum::ColorRef(sprite.color.clone()));
        params.insert("bgColor".into(), Datum::ColorRef(sprite.bg_color.clone()));

        // Copy the 1×1 bitmap over the rectangle, using copy_pixels
        self.copy_pixels(
            palettes,
            &temp,
            dst_rect,
            IntRect::from_tuple((0, 0, 1, 1)),
            &params,
            None,
        );
    }
}
