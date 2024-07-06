use super::{allocator::DatumAllocatorEvent, script::ScriptInstanceId, ALLOCATOR_TX};

pub struct ScriptInstanceRef(ScriptInstanceId);

impl<T> From<T> for ScriptInstanceRef where T: Into<ScriptInstanceId> {
  #[inline]
  fn from(id: T) -> Self {
    let val = id.into();
    unsafe {
      ALLOCATOR_TX.as_ref().unwrap().try_send(DatumAllocatorEvent::ScriptInstanceRefAdded(val)).unwrap();
    }
    Self(val)
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
    Self::from(self.0)
  }
}

impl Drop for ScriptInstanceRef {
  fn drop(&mut self) {
    unsafe {
      ALLOCATOR_TX.as_ref().unwrap().try_send(DatumAllocatorEvent::ScriptInstanceRefDropped(self.0)).unwrap();
    }
  }
}

impl std::fmt::Display for ScriptInstanceRef {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}", self.0)
  }
}