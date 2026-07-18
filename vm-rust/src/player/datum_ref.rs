use std::fmt::Display;

use super::{allocator::{DatumAllocatorTrait, ALLOCATOR_RESETTING}, ACTIVE_PLAYER_ID, NESTED_PLAYERS, PLAYER_OPT};

pub type DatumId = usize;

pub enum DatumRef {
    Void,
    Ref(DatumId, *mut u32),
}

impl DatumRef {
    #[inline]
    pub fn from_id(id: DatumId, ref_count: *mut u32) -> DatumRef {
        if id != 0 {
            let mut_ref = unsafe { &mut *ref_count };
            if *mut_ref != u32::MAX {
                *mut_ref += 1;
            }
            DatumRef::Ref(id, ref_count)
        } else {
            DatumRef::Void
        }
    }

    #[inline]
    pub fn unwrap(&self) -> DatumId {
        match self {
            DatumRef::Void => 0,
            DatumRef::Ref(id, ..) => *id,
        }
    }
}

impl PartialEq for DatumRef {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (DatumRef::Void, DatumRef::Void) => true,
            (DatumRef::Ref(id1, ..), DatumRef::Void) => *id1 == 0,
            (DatumRef::Void, DatumRef::Ref(id2, ..)) => *id2 == 0,
            (DatumRef::Ref(id1, ..), DatumRef::Ref(id2, ..)) => id1 == id2,
            _ => false,
        }
    }
}

impl core::fmt::Debug for DatumRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatumRef::Void => write!(f, "DatumRef(Void)"),
            DatumRef::Ref(id, ..) => write!(f, "DatumRef({})", id),
        }
    }
}

impl Clone for DatumRef {
    fn clone(&self) -> Self {
        match self {
            DatumRef::Void => DatumRef::Void,
            DatumRef::Ref(id, ref_count) => DatumRef::from_id(*id, ref_count.clone()),
        }
    }
}

impl Drop for DatumRef {
    #[inline]
    fn drop(&mut self) {
        if let DatumRef::Ref(id, ref_count) = self {
            unsafe {
                // During allocator reset, arena entries are being cleared one-by-one.
                // Inner DatumRefs may point to already-freed entries, so bail out.
                if ALLOCATOR_RESETTING {
                    return;
                }
                // Normal operation: the ref_count pointer is always valid because
                // a DatumRef can only exist while its datum is alive in the arena.
                let rc = &mut **ref_count;
                if *rc == u32::MAX {
                    return; // Pooled/immortal entry, skip ref counting
                }
                *rc -= 1;
                if *rc == 0 {
                    // Route the dealloc to the ACTIVE player's allocator, not
                    // always the host. A nested `#movie` sub-player allocates and
                    // drops its datums under its own active id; freeing them from
                    // PLAYER_OPT (the host) left every sub datum unreclaimed —
                    // g349's `beginSprite` alone leaked ~8000 datums/frame,
                    // ballooning WASM memory and eventually OOM-crashing the datum
                    // arena. (The ref_count above is decremented through the
                    // DatumRef's own raw pointer into the owner arena, so it stays
                    // correct regardless of which player is active.)
                    let player_opt = if ACTIVE_PLAYER_ID == 0 {
                        PLAYER_OPT.as_mut()
                    } else {
                        NESTED_PLAYERS
                            .get_mut(ACTIVE_PLAYER_ID - 1)
                            .and_then(|o| o.as_mut())
                    };
                    if let Some(player) = player_opt {
                        let bitmap_to_decref = player.allocator.on_datum_ref_dropped(*id);
                        // If the freed entry held an ephemeral Datum::BitmapRef
                        // we now own a decref. Apply it AFTER the allocator hop
                        // so the two field borrows on `player` don't overlap.
                        if let Some(bm_ref) = bitmap_to_decref {
                            player.bitmap_manager.decref_ephemeral(bm_ref);
                        }
                    }
                }
            }
        }
    }
}

impl Display for DatumRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatumRef::Void => write!(f, "DatumRef(Void)"),
            DatumRef::Ref(id, ..) => write!(f, "DatumRef({})", id),
        }
    }
}
