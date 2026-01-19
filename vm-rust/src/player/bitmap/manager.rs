use std::collections::HashMap;

use super::bitmap::Bitmap;

pub type BitmapRef = u32;
pub const INVALID_BITMAP_REF: BitmapRef = 0;

pub struct BitmapManager {
    bitmaps: HashMap<BitmapRef, Bitmap>,
    ref_counter: BitmapRef,
}

impl BitmapManager {
    pub fn new() -> Self {
        Self {
            bitmaps: HashMap::new(),
            ref_counter: 0,
        }
    }

    pub fn add_bitmap(&mut self, bitmap: Bitmap) -> BitmapRef {
        self.ref_counter += 1;

        let bitmap_ref = self.ref_counter;
        self.bitmaps.insert(bitmap_ref, bitmap);
        bitmap_ref
    }

    pub fn replace_bitmap(&mut self, bitmap_ref: BitmapRef, mut bitmap: Bitmap) {
        // Increment version to indicate the bitmap has changed
        // This allows texture caches to know when to re-upload
        if let Some(old_bitmap) = self.bitmaps.get(&bitmap_ref) {
            bitmap.version = old_bitmap.version.wrapping_add(1);
        }
        self.bitmaps.insert(bitmap_ref, bitmap);
    }

    #[allow(dead_code)]
    pub fn get_bitmap(&self, bitmap_ref: BitmapRef) -> Option<&Bitmap> {
        self.bitmaps.get(&bitmap_ref)
    }

    #[allow(dead_code)]
    pub fn get_bitmap_mut(&mut self, bitmap_ref: BitmapRef) -> Option<&mut Bitmap> {
        // Increment version when giving mutable access, as the bitmap may be modified
        // This ensures texture caches know to re-upload the texture
        if let Some(bitmap) = self.bitmaps.get_mut(&bitmap_ref) {
            bitmap.version = bitmap.version.wrapping_add(1);
            Some(bitmap)
        } else {
            None
        }
    }
}
