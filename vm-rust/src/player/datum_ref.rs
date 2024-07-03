use std::fmt::Display;

use super::{allocator::DatumAllocatorEvent, ALLOCATOR_TX};

pub type DatumId = usize;

pub enum DatumRef {
  Void,
  Ref(DatumId),
}
pub static VOID_DATUM_REF: DatumRef = DatumRef::Void;

impl DatumRef {
  pub fn from_id(id: DatumId) -> DatumRef {
    if id != 0 {
      unsafe {
        ALLOCATOR_TX.as_ref().unwrap().try_send(DatumAllocatorEvent::RefAdded(id)).unwrap();
      }
      DatumRef::Ref(id)
    } else {
      DatumRef::Void
    }
  }

  pub fn unwrap(&self) -> DatumId {
    match self {
      DatumRef::Void => 0,
      DatumRef::Ref(id, ..) => *id
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
      _ => false
    }
  }
}

impl core::fmt::Debug for DatumRef {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      DatumRef::Void => write!(f, "DatumRef(Void)"),
      DatumRef::Ref(id, ..) => write!(f, "DatumRef({})", id)
    }
  }
}

impl Clone for DatumRef {
  fn clone(&self) -> Self {
    match self {
      DatumRef::Void => DatumRef::Void,
      DatumRef::Ref(id) => {
        DatumRef::from_id(*id)
      }
    }
  }
}

impl Drop for DatumRef {
  fn drop(&mut self) {
    if let DatumRef::Ref(id) = self {
      unsafe {
        ALLOCATOR_TX.as_ref().unwrap().try_send(DatumAllocatorEvent::RefDropped(*id)).unwrap();
      }
    }
  }
}

impl Display for DatumRef {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      DatumRef::Void => write!(f, "DatumRef(Void)"),
      DatumRef::Ref(id, ..) => write!(f, "DatumRef({})", id)
    }
  }
}
