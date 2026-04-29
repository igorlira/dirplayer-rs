use std::collections::HashMap;

use super::bitmap::Bitmap;

pub type BitmapRef = u32;
pub const INVALID_BITMAP_REF: BitmapRef = 0;

pub struct BitmapManager {
    bitmaps: HashMap<BitmapRef, Bitmap>,
    ref_counter: BitmapRef,
    /// Side table for ephemeral bitmaps — those produced by Lingo getters
    /// like `(the stage).image`, `image(w, h, d)`, `bitmap.duplicate()`,
    /// member `.image` accessors, etc. The value is the number of
    /// `Datum::BitmapRef` arena entries currently pointing at the bitmap;
    /// when it drops to zero the bitmap is freed.
    ///
    /// Cast-member-owned bitmaps are NOT in this map and are never freed by
    /// the refcount path — they live as long as the cast member does.
    ephemeral_refs: HashMap<BitmapRef, u32>,
}

impl BitmapManager {
    pub fn new() -> Self {
        Self {
            bitmaps: HashMap::new(),
            ref_counter: 0,
            ephemeral_refs: HashMap::new(),
        }
    }

    /// Register an anchored bitmap (owned by a cast member or other long-lived
    /// holder). Will not be auto-freed when DatumRefs drop.
    pub fn add_bitmap(&mut self, bitmap: Bitmap) -> BitmapRef {
        self.ref_counter += 1;

        let bitmap_ref = self.ref_counter;
        self.bitmaps.insert(bitmap_ref, bitmap);
        bitmap_ref
    }

    /// Register an ephemeral bitmap. Once the last `Datum::BitmapRef(N)`
    /// arena entry is dropped, the bitmap is freed. Use for `(the stage)
    /// .image`, `image(w, h, d)`, `bitmap.duplicate()`, member `.image`
    /// snapshots — anywhere a Lingo expression produces a bitmap with no
    /// other persistent owner.
    pub fn add_ephemeral_bitmap(&mut self, bitmap: Bitmap) -> BitmapRef {
        self.ref_counter += 1;

        let bitmap_ref = self.ref_counter;
        self.bitmaps.insert(bitmap_ref, bitmap);
        // Start at 0 — the caller's `alloc_datum(Datum::BitmapRef(...))` will
        // bump it via `incref_ephemeral`. If for some reason the bitmap is
        // never wrapped in a DatumRef the entry leaks, but that's rare and
        // strictly better than the previous always-leak behaviour.
        self.ephemeral_refs.insert(bitmap_ref, 0);
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

    /// Bump the ephemeral refcount for `bitmap_ref`. No-op for anchored
    /// bitmaps (those not in `ephemeral_refs`). Called by the allocator
    /// when a new arena entry wrapping `Datum::BitmapRef(N)` is created.
    pub fn incref_ephemeral(&mut self, bitmap_ref: BitmapRef) {
        if let Some(count) = self.ephemeral_refs.get_mut(&bitmap_ref) {
            *count = count.saturating_add(1);
        }
    }

    /// Decrement the ephemeral refcount. If it reaches zero the bitmap and
    /// its tracking entry are removed. No-op for anchored bitmaps.
    pub fn decref_ephemeral(&mut self, bitmap_ref: BitmapRef) {
        let should_free = if let Some(count) = self.ephemeral_refs.get_mut(&bitmap_ref) {
            *count = count.saturating_sub(1);
            *count == 0
        } else {
            false
        };
        if should_free {
            self.ephemeral_refs.remove(&bitmap_ref);
            self.bitmaps.remove(&bitmap_ref);
        }
    }
}
