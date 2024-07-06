use super::{allocator::DatumAllocatorEvent, script::ScriptInstanceId, ALLOCATOR_TX};

#[derive(Eq, PartialEq, Debug)]
pub struct ScriptInstanceRef {
  pub id: ScriptInstanceId,
}

impl ScriptInstanceRef {
  pub fn from_id(id: ScriptInstanceId) -> ScriptInstanceRef {
    unsafe {
      ALLOCATOR_TX.as_ref().unwrap().try_send(DatumAllocatorEvent::ScriptInstanceRefAdded(id)).unwrap();
    }
    ScriptInstanceRef { id }
  }
}

impl Clone for ScriptInstanceRef {
  fn clone(&self) -> Self {
    Self::from_id(self.id)
  }
}

impl Drop for ScriptInstanceRef {
  fn drop(&mut self) {
    unsafe {
      ALLOCATOR_TX.as_ref().unwrap().try_send(DatumAllocatorEvent::ScriptInstanceRefDropped(self.id)).unwrap();
    }
  }
}
