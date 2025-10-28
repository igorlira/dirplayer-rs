use std::{
    cell::{Cell, UnsafeCell},
    fmt::Display,
    rc::Rc,
};

use super::{allocator::DatumAllocatorTrait, PLAYER_OPT};

pub type DatumId = usize;

pub enum DatumRef {
    Void,
    Ref(DatumId, *mut u32),
}

impl DatumRef {
    pub fn from_id(id: DatumId, ref_count: *mut u32) -> DatumRef {
        if id != 0 {
            let mut_ref = unsafe { &mut *ref_count };
            *mut_ref += 1;
            DatumRef::Ref(id, ref_count)
        } else {
            DatumRef::Void
        }
    }

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
    fn drop(&mut self) {
        if let DatumRef::Ref(id, ref_count) = self {
            unsafe {
                let ref_count = &mut **ref_count;
                *ref_count -= 1;
                if *ref_count <= 0 {
                    let player = PLAYER_OPT.as_mut().unwrap();
                    player.allocator.on_datum_ref_dropped(*id);
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
