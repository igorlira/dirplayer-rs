use std::sync::Arc;

use bitvec::vec::BitVec;

use crate::player::sprite::ColorRef;

use super::{bitmap::Bitmap, palette_map::PaletteMap};

#[derive(Clone, PartialEq, Eq)]
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

    /// Director `imageObject.createMask()` — a mask object that duplicates MASK
    /// sprite ink (11.5 Scripting Dictionary p.307). This is a DIFFERENT
    /// operation from `createMatte()` (p.308), which duplicates matte ink and
    /// is built from the image's alpha layer; the dictionary lists them as
    /// separate methods that cross-reference each other.
    ///
    /// Mask ink is 1-bit: dark mask pixels are opaque (source shows through),
    /// light ones transparent. Director thresholds a deeper mask image down to
    /// 1 bit, so a 50% luminance cut is used here — exact for the pure
    /// black/white images masks are normally authored as, and a reasonable
    /// stand-in for Director's dithering on anything deeper.
    ///
    /// `createMask` used to be aliased onto `createMatte`, which is an
    /// edge-connected flood fill: it can only ever reach background that
    /// touches the border, so an INTERIOR hole is unreachable and stayed
    /// opaque. Habbo v31's catalogue Spaces preview masks the window glass out
    /// of `catalog_spaces_window` exactly that way, so its 5076 magenta
    /// placeholder pixels were copied onto the preview instead of being left
    /// open for the landscape behind.
    pub fn create_mask(&self, palettes: &PaletteMap) -> BitmapMask {
        let mut mask = BitmapMask::new(self.width, self.height, false);
        for y in 0..self.height {
            for x in 0..self.width {
                let (r, g, b) = self.get_pixel_color(palettes, x, y);
                // Rec.601 luma, matching the grayscale conversion used elsewhere.
                let luma = (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000;
                mask.set_bit(x, y, luma < 128);
            }
        }
        mask
    }

    pub fn create_matte(&mut self, palettes: &PaletteMap) {
        let matte = self.compute_matte(palettes);
        self.matte = Some(Arc::new(matte));
    }

    /// The matte ink's shape: every pixel except the background colour that
    /// is connected to the image's edge. Interior background pixels stay
    /// part of the shape, as they do in Director.
    pub fn compute_matte(&self, palettes: &PaletteMap) -> BitmapMask {
        // A 32-bit image with its alpha channel in use carries its own
        // shape: the matte is every pixel that is not fully transparent.
        // The white-from-the-edge fill below would find no white border on
        // such an image and cover its whole rectangle.
        if self.bit_depth == 32 && self.use_alpha {
            let mut matte = BitmapMask::new(self.width, self.height, false);
            for y in 0..self.height {
                for x in 0..self.width {
                    let i = (y as usize * self.width as usize + x as usize) * 4 + 3;
                    matte.set_bit(x, y, self.data.get(i).copied().unwrap_or(0) > 0);
                }
            }
            return matte;
        }
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
        mask.flood_matte(outside_pixels, false, true)
    }
}

#[cfg(test)]
mod tests {
    use crate::player::bitmap::bitmap::{Bitmap, BuiltInPalette, PaletteRef};
    use crate::player::bitmap::palette_map::PaletteMap;

    #[test]
    fn matte_keeps_interior_white_and_drops_the_white_around() {
        // 5x5 white image with a black square ring at 1..=3: the white
        // outside the ring touches the edge, the white centre does not.
        let mut bmp = Bitmap::new(5, 5, 32, 32, 0, PaletteRef::BuiltIn(BuiltInPalette::SystemWin));
        for y in 0..5usize {
            for x in 0..5usize {
                let ring = (1..=3).contains(&x) && (1..=3).contains(&y) && !(x == 2 && y == 2);
                let v = if ring { 0 } else { 255 };
                let i = (y * 5 + x) * 4;
                bmp.data[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let matte = bmp.compute_matte(&PaletteMap::new());
        assert!(!matte.get_bit(0, 0), "white touching the edge is outside the shape");
        assert!(!matte.get_bit(4, 2));
        assert!(matte.get_bit(1, 1), "the ring is the shape");
        assert!(matte.get_bit(2, 2), "white enclosed by the ring stays part of the shape");
    }

    #[test]
    fn matte_of_an_alpha_image_is_its_visible_pixels() {
        // 3x1, alpha 0 / 255 / 0: only the middle pixel is the shape, even
        // though none of them is white.
        let mut bmp = Bitmap::new(3, 1, 32, 32, 8, PaletteRef::BuiltIn(BuiltInPalette::SystemWin));
        bmp.use_alpha = true;
        bmp.data[0..12].copy_from_slice(&[10, 20, 30, 0, 10, 20, 30, 255, 10, 20, 30, 0]);
        let matte = bmp.compute_matte(&PaletteMap::new());
        assert!(!matte.get_bit(0, 0));
        assert!(matte.get_bit(1, 0));
        assert!(!matte.get_bit(2, 0));
    }
}
