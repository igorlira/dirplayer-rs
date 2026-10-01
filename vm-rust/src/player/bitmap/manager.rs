use std::cell::{Cell, OnceCell};
use std::collections::HashMap;

use super::bitmap::Bitmap;

/// A stored bitmap, or the means to decode it on first use.
///
/// A movie's cast is registered all at once when it loads, and decoding every
/// JPEG member then (141 of them in one measured title movie, ~120 ms of the
/// load) paid for pictures the first frames never show. A lazy slot keeps
/// the compressed source and decodes when the bitmap is first read.
struct Slot {
    bitmap: OnceCell<Bitmap>,
    decode: Cell<Option<Box<dyn FnOnce() -> Bitmap>>>,
}

impl Slot {
    fn ready(bitmap: Bitmap) -> Self {
        let cell = OnceCell::new();
        let _ = cell.set(bitmap);
        Self { bitmap: cell, decode: Cell::new(None) }
    }

    fn get(&self) -> &Bitmap {
        self.bitmap.get_or_init(|| match self.decode.take() {
            Some(decode) => decode(),
            None => Bitmap::new(1, 1, 8, 8, 0, super::bitmap::PaletteRef::BuiltIn(super::bitmap::BuiltInPalette::GrayScale)),
        })
    }

    fn get_mut(&mut self) -> &mut Bitmap {
        self.get();
        self.bitmap.get_mut().expect("slot decoded above")
    }
}

pub type BitmapRef = u32;
pub const INVALID_BITMAP_REF: BitmapRef = 0;

pub struct BitmapManager {
    bitmaps: HashMap<BitmapRef, Slot>,
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

    /// Drop every stored bitmap when switching movies. Cast-member-owned
    /// (anchored) bitmaps are never removed by the ephemeral refcount path, so
    /// without this they orphan here forever: loading a new movie replaces the
    /// cast list but leaks the previous movie's bitmaps (Infestation's ~363
    /// bitmaps, ~10 MB+ decoded, on every load — memory that never comes back).
    /// `ref_counter` is NOT reset so freshly-issued refs can't collide with any
    /// `Datum::BitmapRef` that a persisted global still holds.
    pub fn clear_movie_bitmaps(&mut self) {
        self.bitmaps.clear();
        self.ephemeral_refs.clear();
    }

    /// Drop every bitmap outside `keep`, for a movie switch. Anchored bitmaps
    /// are never freed by the refcount path, so the outgoing cast's pictures
    /// stayed here for the rest of the session: measured over twelve task
    /// movies with a hub movie in between, 8,700 slots and 850 MB of pixels
    /// were held that no cast member could reach any more, and the engine's
    /// memory grew by that much. An ephemeral bitmap still wrapped by a datum
    /// is kept whether or not the caller listed it. Returns (slots, bytes)
    /// freed. `ref_counter` keeps counting, so a freed ref is never reissued.
    pub fn retain_only(&mut self, keep: &std::collections::HashSet<BitmapRef>) -> (usize, usize) {
        let ephemeral_refs = &self.ephemeral_refs;
        let mut freed = 0;
        let mut bytes = 0;
        self.bitmaps.retain(|r, slot| {
            let live = keep.contains(r) || ephemeral_refs.get(r).map_or(false, |&n| n > 0);
            if !live {
                freed += 1;
                if let Some(b) = slot.bitmap.get() {
                    bytes += b.data.capacity();
                }
            }
            live
        });
        let bitmaps = &self.bitmaps;
        self.ephemeral_refs.retain(|r, _| bitmaps.contains_key(r));
        (freed, bytes)
    }

    /// Register an anchored bitmap (owned by a cast member or other long-lived
    /// holder). Will not be auto-freed when DatumRefs drop.
    pub fn add_bitmap(&mut self, bitmap: Bitmap) -> BitmapRef {
        self.ref_counter += 1;

        let bitmap_ref = self.ref_counter;
        self.bitmaps.insert(bitmap_ref, Slot::ready(bitmap));
        bitmap_ref
    }

    /// Register an anchored bitmap that is decoded the first time it is read.
    /// `decode` must return a bitmap even when the source is bad (a
    /// placeholder), as an eager load would have stored.
    pub fn add_lazy_bitmap(&mut self, decode: Box<dyn FnOnce() -> Bitmap>) -> BitmapRef {
        self.ref_counter += 1;
        let bitmap_ref = self.ref_counter;
        self.bitmaps.insert(bitmap_ref, Slot { bitmap: OnceCell::new(), decode: Cell::new(Some(decode)) });
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
        self.bitmaps.insert(bitmap_ref, Slot::ready(bitmap));
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
        if let Some(old) = self.bitmaps.get(&bitmap_ref) {
            // A still-undecoded bitmap has version 0 either way.
            if let Some(old_bitmap) = old.bitmap.get() {
                bitmap.version = old_bitmap.version.wrapping_add(1);
            }
        }
        self.bitmaps.insert(bitmap_ref, Slot::ready(bitmap));
    }

    #[allow(dead_code)]
    pub fn get_bitmap(&self, bitmap_ref: BitmapRef) -> Option<&Bitmap> {
        self.bitmaps.get(&bitmap_ref).map(Slot::get)
    }

    #[allow(dead_code)]
    pub fn get_bitmap_mut(&mut self, bitmap_ref: BitmapRef) -> Option<&mut Bitmap> {
        // Increment version when giving mutable access, as the bitmap may be modified
        // This ensures texture caches know to re-upload the texture
        if let Some(slot) = self.bitmaps.get_mut(&bitmap_ref) {
            let bitmap = slot.get_mut();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::bitmap::bitmap::{BuiltInPalette, PaletteRef};

    fn bmp() -> Bitmap {
        Bitmap::new(2, 2, 32, 32, 8, PaletteRef::BuiltIn(BuiltInPalette::GrayScale))
    }

    #[test]
    fn a_movie_switch_keeps_only_what_is_still_reachable() {
        let mut m = BitmapManager::new();
        let cast_owned = m.add_bitmap(bmp());
        let lazy = m.add_lazy_bitmap(Box::new(bmp));
        let kept = m.add_bitmap(bmp());
        let held = m.add_ephemeral_bitmap(bmp());
        m.incref_ephemeral(held);
        let unwrapped = m.add_ephemeral_bitmap(bmp());

        let keep: std::collections::HashSet<BitmapRef> = [kept].into_iter().collect();
        let (freed, _) = m.retain_only(&keep);

        assert_eq!(freed, 3);
        assert!(m.get_bitmap(cast_owned).is_none());
        assert!(m.get_bitmap(lazy).is_none());
        assert!(m.get_bitmap(unwrapped).is_none());
        assert!(m.get_bitmap(kept).is_some());
        assert!(m.get_bitmap(held).is_some());
        // The next bitmap gets a new ref, not a freed one.
        assert!(m.add_bitmap(bmp()) > unwrapped);
    }
}
