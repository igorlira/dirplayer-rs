use log::warn;

use super::{allocator::ScriptInstanceAllocatorTrait, script::ScriptInstanceId, PLAYER_OPT};

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
    fn drop(&mut self) {
        unsafe {
            // Check if we can safely dereference the ref_count pointer
            // During allocator reset, the Rc may have been freed
            if let Some(player) = PLAYER_OPT.as_mut() {
                // Only proceed if the script instance still exists in the allocator
                if player.allocator.script_instances.contains_key(&self.0) {
                    let mut_ref = &mut *self.1;
                    *mut_ref -= 1;
                    if *mut_ref <= 0 {
                        player.allocator.on_script_instance_ref_dropped(self.0);
                    }
                } else {
                    warn!("Attempted to drop ScriptInstanceRef for non-existing ScriptInstanceId: {}", self.0);
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
