use std::sync::Arc;

use bitvec::vec::BitVec;

use crate::player::sprite::ColorRef;

use super::{bitmap::Bitmap, palette_map::PaletteMap};

#[derive(Clone)]
pub struct BitmapMask {
    pub width: u16,
    pub height: u16,
    pub data: BitVec,
}

impl BitmapMask {
    pub fn new(width: u16, height: u16, default: bool) -> Self {
        BitmapMask {
            width,
            height,
            data: BitVec::repeat(default, (width as usize) * (height as usize)),
        }
    }

    pub fn get_bit(&self, x: u16, y: u16) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        *self
            .data
            .get((y as usize * self.width as usize) + (x as usize))
            .unwrap()
    }

    pub fn set_bit(&mut self, x: u16, y: u16, value: bool) {
        if x >= self.width || y >= self.height {
            return;
        }
        self.data
            .set((y as usize * self.width as usize) + (x as usize), value);
    }

    pub fn flood_matte(&mut self, points: Vec<(u16, u16)>, from: bool, to: bool) -> BitmapMask {
        let mut stack = points;
        let mut not_visited = BitmapMask::new(self.width, self.height, true);
        while let Some(point) = stack.pop() {
            let (x, y) = point;
            if !not_visited.get_bit(x, y) {
                continue;
            }
            if x < self.width && y < self.height && self.get_bit(x, y) == from {
                self.set_bit(x, y, to);
                not_visited.set_bit(x, y, false);
                if x + 1 < self.width {
                    stack.push((x + 1, y));
                }
                if x > 0 {
                    stack.push((x - 1, y));
                }
                if y + 1 < self.height {
                    stack.push((x, y + 1));
                }
                if y > 0 {
                    stack.push((x, y - 1));
                }
            }
        }
        not_visited
    }
}

impl Bitmap {
    pub fn get_mask(&self, palettes: &PaletteMap, bg_color: &ColorRef) -> BitmapMask {
        let mut mask = BitmapMask::new(self.width, self.height, false);
        for y in 0..self.height {
            for x in 0..self.width {
                let pixel = self.get_pixel_color_ref(x, y);
                mask.set_bit(x, y, pixel != *bg_color);
            }
        }
        mask
    }

    pub fn create_matte_text(&mut self, palettes: &PaletteMap) {
        let bg_color = &self.get_bg_color_ref();

        // Create matte: true for content (opaque), false for background (transparent)
        // This automatically handles both exterior background AND interior holes
        let mut matte = BitmapMask::new(self.width, self.height, false);
        for y in 0..self.height {
            for x in 0..self.width {
                let pixel = self.get_pixel_color_ref(x, y);
                // Opaque if pixel is NOT background color
                matte.set_bit(x, y, pixel != *bg_color);
            }
        }

        self.matte = Some(Arc::new(matte));
    }

    pub fn create_matte(&mut self, palettes: &PaletteMap) {
        let bg_color = &self.get_bg_color_ref();
        let mut mask = self.get_mask(palettes, bg_color);
        let mut outside_pixels = vec![];
        for y in 0..self.height {
            let left_pixel = self.get_pixel_color_ref(0, y);
            let right_pixel = self.get_pixel_color_ref(self.width - 1, y);

            if left_pixel == *bg_color {
                outside_pixels.push((0, y));
            }
            if right_pixel == *bg_color {
                outside_pixels.push((self.width - 1, y));
            }
        }
        for x in 0..self.width {
            let top_pixel = self.get_pixel_color_ref(x, 0);
            let bottom_pixel = self.get_pixel_color_ref(x, self.height - 1);

            if top_pixel == *bg_color {
                outside_pixels.push((x, 0));
            }
            if bottom_pixel == *bg_color {
                outside_pixels.push((x, self.height - 1));
            }
        }
        let matte = mask.flood_matte(outside_pixels, false, true);
        self.matte = Some(Arc::new(matte));
    }
}
