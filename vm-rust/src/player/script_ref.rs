use super::{allocator::{ScriptInstanceAllocatorTrait, ALLOCATOR_RESETTING}, script::ScriptInstanceId, ACTIVE_PLAYER_ID, NESTED_PLAYERS, PLAYER_OPT};

#[derive(Debug)]
pub struct ScriptInstanceRef(ScriptInstanceId, *mut u32);

impl ScriptInstanceRef {
    #[inline]
    pub fn from_id(id: ScriptInstanceId, ref_count: *mut u32) -> Self {
        let val = id.into();
        unsafe {
            let mut_ref = &mut *ref_count;
            *mut_ref += 1;
        }
        Self(val, ref_count)
    }

    #[inline]
    pub fn id(&self) -> ScriptInstanceId {
        self.0
    }
}

impl std::ops::Deref for ScriptInstanceRef {
    type Target = ScriptInstanceId;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Clone for ScriptInstanceRef {
    fn clone(&self) -> Self {
        Self::from_id(self.0, self.1)
    }
}

impl Drop for ScriptInstanceRef {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            if ALLOCATOR_RESETTING {
                return;
            }
            let rc = &mut *self.1;
            *rc -= 1;
            if *rc == 0 {
                // Route to the ACTIVE player's allocator (a nested sub-player owns
                // its own script instances) — see the DatumRef::drop note.
                let player_opt = if ACTIVE_PLAYER_ID == 0 {
                    PLAYER_OPT.as_mut()
                } else {
                    NESTED_PLAYERS
                        .get_mut(ACTIVE_PLAYER_ID - 1)
                        .and_then(|o| o.as_mut())
                };
                if let Some(player) = player_opt {
                    player.allocator.on_script_instance_ref_dropped(self.0);
                }
            }
        }
    }
}

impl std::fmt::Display for ScriptInstanceRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
