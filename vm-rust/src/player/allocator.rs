use async_std::channel::{Receiver, Sender};
use nohash_hasher::IntMap;

use crate::{console_warn, director::lingo::datum::Datum};

use super::{datum_ref::{DatumId, DatumRef}, reserve_player_mut, reserve_player_ref, script::{ScriptInstance, ScriptInstanceId}, script_ref::ScriptInstanceRef, ScriptError, PLAYER_LOCK, VOID_DATUM_REF};


pub struct DatumRefEntry {
  pub id: DatumId,
  pub ref_count: u32,
  pub datum: Datum,
}

pub struct ScriptInstanceRefEntry {
  pub id: ScriptInstanceId,
  pub ref_count: u32,
  pub script_instance: ScriptInstance,
}

pub trait ResetableAllocator {
  fn reset(&mut self);
}

pub trait DatumAllocatorTrait {
  fn alloc_datum(&mut self, datum: Datum) -> Result<DatumRef, ScriptError>;
  fn get_datum(&self, id: &DatumRef) -> &Datum;
  fn get_datum_mut(&mut self, id: &DatumRef) -> &mut Datum;
  fn on_datum_ref_added(&mut self, id: DatumId);
  fn on_datum_ref_dropped(&mut self, id: DatumId);
}

pub trait ScriptInstanceAllocatorTrait {
  fn alloc_script_instance(&mut self, script_instance: ScriptInstance) -> ScriptInstanceRef;
  fn get_script_instance(&self, instance_ref: &ScriptInstanceRef) -> &ScriptInstance;
  fn get_script_instance_opt(&self, instance_ref: &ScriptInstanceRef) -> Option<&ScriptInstance>;
  fn get_script_instance_mut(&mut self, instance_ref: &ScriptInstanceRef) -> &mut ScriptInstance;
  fn on_script_instance_ref_added(&mut self, id: ScriptInstanceId);
  fn on_script_instance_ref_dropped(&mut self, id: ScriptInstanceId);
}

pub enum DatumAllocatorEvent {
  RefAdded(DatumId),
  RefDropped(DatumId),
  ScriptInstanceRefAdded(ScriptInstanceId),
  ScriptInstanceRefDropped(ScriptInstanceId),
}

pub struct DatumAllocator {
  pub datums: IntMap<DatumId, DatumRefEntry>,
  pub script_instances: IntMap<ScriptInstanceId, ScriptInstanceRefEntry>,
  datum_id_counter: DatumId,
  script_instance_counter: ScriptInstanceId,
  void_datum: Datum,
  pub tx: Sender<DatumAllocatorEvent>,
  pub rx: Receiver<DatumAllocatorEvent>,
}

const MAX_DATUM_ID: DatumId = 0xFFFFFF;
const MAX_SCRIPT_INSTANCE_ID: ScriptInstanceId = 0xFFFFFF;

impl DatumAllocator {
  pub fn default(rx: Receiver<DatumAllocatorEvent>, tx: Sender<DatumAllocatorEvent>) -> Self {
    DatumAllocator {
      datums: IntMap::default(),
      script_instances: IntMap::default(),
      datum_id_counter: 1,
      script_instance_counter: 1,
      void_datum: Datum::Void,
      tx,
      rx,
    }
  }

  fn contains_datum(&self, id: DatumId) -> bool {
    self.datums.contains_key(&id)
  }

  fn get_free_id(&self) -> Option<DatumId> {
    if self.datum_count() >= MAX_DATUM_ID {
      panic!("Datum limit reached");
    }
    if !self.contains_datum(self.datum_id_counter) {
      Some(self.datum_id_counter)
    } else if self.datum_id_counter + 1 < MAX_DATUM_ID && !self.contains_datum(self.datum_id_counter + 1) {
      Some(self.datum_id_counter + 1)
    } else {
      console_warn!("Datum id overflow. Searching for free id...");
      let first_free_id = (1..MAX_DATUM_ID).find(|id| !self.contains_datum(*id));
      first_free_id
    }
  }

  pub fn get_free_script_instance_id(&self) -> ScriptInstanceId {
    if self.script_instance_count() >= MAX_SCRIPT_INSTANCE_ID as usize {
      panic!("Script instance limit reached");
    }
    if !self.script_instances.contains_key(&self.script_instance_counter) {
      self.script_instance_counter
    } else if self.script_instance_counter + 1 < MAX_SCRIPT_INSTANCE_ID && !self.contains_script_instance(self.script_instance_counter + 1) {
      self.script_instance_counter + 1
    } else {
      console_warn!("Script instance id overflow. Searching for free id...");
      let first_free_id = (1..MAX_SCRIPT_INSTANCE_ID).find(|id| !self.contains_script_instance(*id));
      if let Some(id) = first_free_id {
        id
      } else {
        panic!("Failed to find free script instance id");
      }
    }
  }

  fn contains_script_instance(&self, id: ScriptInstanceId) -> bool {
    self.script_instances.contains_key(&id)
  }

  pub fn script_instance_count(&self) -> usize {
    self.script_instances.len()
  }

  pub fn datum_count(&self) -> usize {
    self.datums.len()
  }

  fn dealloc_datum(&mut self, id: DatumId) {
    self.datums.remove(&id);
  }

  fn dealloc_script_instance(&mut self, id: ScriptInstanceId) {
    self.script_instances.remove(&id);
  }
}

impl DatumAllocatorTrait for DatumAllocator {
  fn alloc_datum(&mut self, datum: Datum) -> Result<DatumRef, ScriptError> {
    if datum.is_void() {
      return Ok(VOID_DATUM_REF.clone());
    }
    
    if let Some(id) = self.get_free_id() {
      let entry = DatumRefEntry {
        id,
        ref_count: 0,
        datum,
      };
      self.datum_id_counter += 1;
      self.datums.insert(id, entry);
      Ok(DatumRef::from_id(id))
    } else {
      Err(ScriptError::new("Failed to allocate datum".to_string()))
    }
  }

  fn get_datum(&self, id: &DatumRef) -> &Datum {
    match id {
      DatumRef::Ref(id, ..) => {
        let entry = self.datums.get(id).unwrap();
        &entry.datum
      }
      DatumRef::Void => &Datum::Void,
    }
  }

  fn get_datum_mut(&mut self, id: &DatumRef) -> &mut Datum {
    match id {
      DatumRef::Ref(id, ..) => {
        let entry = self.datums.get_mut(id).unwrap();
        &mut entry.datum
      }
      DatumRef::Void => &mut self.void_datum,
    }
  }

  fn on_datum_ref_added(&mut self, id: DatumId) {
    let entry = self.datums.get_mut(&id).unwrap();
    entry.ref_count += 1;
  }

  fn on_datum_ref_dropped(&mut self, id: DatumId) {
    let entry = self.datums.get_mut(&id).unwrap();
    entry.ref_count -= 1;
    if entry.ref_count <= 0 {
      self.dealloc_datum(id);
    }
  }
}

impl ScriptInstanceAllocatorTrait for DatumAllocator {
  fn alloc_script_instance(&mut self, script_instance: ScriptInstance) -> ScriptInstanceRef {
    let id = self.get_free_script_instance_id();
    self.script_instance_counter += 1;
    self.script_instances.insert(id, ScriptInstanceRefEntry {
      id,
      ref_count: 0,
      script_instance,
    });
    ScriptInstanceRef::from(id)
  }

  fn get_script_instance(&self, instance_ref: &ScriptInstanceRef) -> &ScriptInstance {
    &self.script_instances.get(instance_ref).unwrap().script_instance
  }

  fn get_script_instance_opt(&self, instance_ref: &ScriptInstanceRef) -> Option<&ScriptInstance> {
    self.script_instances.get(instance_ref).map(|entry| &entry.script_instance)
  }

  fn get_script_instance_mut(&mut self, instance_ref: &ScriptInstanceRef) -> &mut ScriptInstance {
    &mut self.script_instances.get_mut(instance_ref).unwrap().script_instance
  }

  fn on_script_instance_ref_added(&mut self, id: ScriptInstanceId) {
    let entry = self.script_instances.get_mut(&id).unwrap();
    entry.ref_count += 1;
  }

  fn on_script_instance_ref_dropped(&mut self, id: ScriptInstanceId) {
    let entry = self.script_instances.get_mut(&id).unwrap();
    entry.ref_count -= 1;
    if entry.ref_count <= 0 {
      self.dealloc_script_instance(id);
    }
  }
}

impl ResetableAllocator for DatumAllocator {
  fn reset(&mut self) {
    self.datums.clear();
    self.datum_id_counter = 1;
    self.script_instances.clear();
    self.script_instance_counter = 1;
  }
}

pub fn player_run_allocator_cycle() {
  let queue = reserve_player_ref(|player| {
    let rx = &player.allocator.rx;
    let mut result = vec![];
    while !rx.is_empty() {
      let item = rx.try_recv().unwrap();
      result.push(item);
    }
    result
  });
  reserve_player_mut(|player| {
    for item in queue {
      match item {
        DatumAllocatorEvent::RefAdded(id) => {
          player.allocator.on_datum_ref_added(id);
        }
        DatumAllocatorEvent::RefDropped(id) => {
          player.allocator.on_datum_ref_dropped(id);
        }
        DatumAllocatorEvent::ScriptInstanceRefAdded(id) => {
          player.allocator.on_script_instance_ref_added(id);
        }
        DatumAllocatorEvent::ScriptInstanceRefDropped(id) => {
          player.allocator.on_script_instance_ref_dropped(id);
        }
      }
    }
  });
}
