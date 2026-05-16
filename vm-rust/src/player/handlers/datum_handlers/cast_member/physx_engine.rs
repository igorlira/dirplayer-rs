//! PhysX (AGEIA) engine singleton
//!
//! Native AGEIA hosts a process-global singleton that owns every
//! `IPhysicsWorld` (the `member("PhysicsWorld")` instances). In dirplayer-rs
//! each world lives inside a `PhysXPhysicsMember` cast member, so this
//! module is mostly a counter for unique IDs and a flag tracking whether
//! the SDK has been initialised. Future work could add a process-global
//! profile-zone manager / cooking helper as the wrapper layer matures.
//!
//! The Lingo surface never reaches the engine directly — Director scripts
//! always go through `member("…").<method>` — but `getUniqueID()` is used
//! by world+body factories to mint stable IDs.

use std::sync::atomic::{AtomicU32, Ordering};

/// Process-global monotonic counter mirroring `CPhysicsEngineAGEIA::m_ID`.
/// First call returns 1.
static NEXT_ID: AtomicU32 = AtomicU32::new(0);

/// Mirrors `CPhysicsEngineAGEIA::GetUniqueID()` — returns a fresh ID
/// every call, never reuses.
pub fn get_unique_id() -> u32 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed) + 1
}
