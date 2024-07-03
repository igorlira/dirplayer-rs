use async_std::{channel::Sender, channel::Receiver};

use crate::{console_warn, director::lingo::datum::Datum};

use super::{DatumId, DatumRef, ScriptError, PLAYER_LOCK, VOID_DATUM_REF};


pub struct DatumRefEntry {
  pub id: DatumId,
  pub ref_count: u32,
  pub datum: Datum,
}

pub trait DatumAllocatorTrait {
  fn alloc_datum(&mut self, datum: Datum) -> Result<DatumRef, ScriptError>;
  fn get_datum(&self, id: &DatumRef) -> &Datum;
  fn get_datum_mut(&mut self, id: &DatumRef) -> &mut Datum;
  fn on_datum_ref_added(&mut self, id: DatumId);
  fn on_datum_ref_dropped(&mut self, id: DatumId);
  fn reset(&mut self);
}

pub enum DatumAllocatorEvent {
  RefAdded(DatumId),
  RefDropped(DatumId),
}

pub struct DatumAllocator {
  pub datums: Vec<Option<DatumRefEntry>>,
  datum_id_counter: DatumId,
  void_datum: Datum,
  pub tx: Sender<DatumAllocatorEvent>,
  pub datum_count: usize,
}

const MAX_DATUM_ID: DatumId = 0xFFFFFF;

impl DatumAllocator {
  pub fn default(tx: Sender<DatumAllocatorEvent>) -> Self {
    let mut vector = Vec::with_capacity(MAX_DATUM_ID);
    vector.resize_with(MAX_DATUM_ID, || None);
    DatumAllocator {
      datums: vector,
      datum_id_counter: 1,
      datum_count: 0,
      void_datum: Datum::Void,
      tx,
    }
  }

  fn contains_datum(&self, id: DatumId) -> bool {
    self.datums.get(id).is_some_and(|x| x.is_some())
  }

  fn get_free_id(&self) -> Option<DatumId> {
    if !self.contains_datum(self.datum_id_counter) {
      Some(self.datum_id_counter)
    } else if self.datum_id_counter + 1 < MAX_DATUM_ID && !self.contains_datum(self.datum_id_counter + 1) {
      Some(self.datum_id_counter + 1)
    } else {
      console_warn!("Maxium datum id reached");
      let first_free_id = (1..MAX_DATUM_ID).find(|id| !self.contains_datum(*id));
      first_free_id
    }
  }

  fn dealloc_datum(&mut self, id: DatumId) {
    self.datum_count -= 1;
    self.datums[id] = None;
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
      self.datum_count += 1;
      if id >= self.datums.len() {
        self.datums.insert(id, Some(entry))
      } else {
        self.datums[id] = Some(entry);
      }
      Ok(DatumRef::from_id(id))
    } else {
      Err(ScriptError::new("Failed to allocate datum".to_string()))
    }
  }

  fn get_datum(&self, id: &DatumRef) -> &Datum {
    match id {
      DatumRef::Ref(id, ..) => {
        let entry = self.datums.get(*id).unwrap().as_ref().unwrap();
        &entry.datum
      }
      DatumRef::Void => &Datum::Void,
    }
  }

  fn get_datum_mut(&mut self, id: &DatumRef) -> &mut Datum {
    match id {
      DatumRef::Ref(id, ..) => {
        let entry = self.datums.get_mut(*id).unwrap().as_mut().unwrap();
        &mut entry.datum
      }
      DatumRef::Void => &mut self.void_datum,
    }
  }

  fn on_datum_ref_added(&mut self, id: DatumId) {
    let entry = self.datums.get_mut(id).unwrap().as_mut().unwrap();
    entry.ref_count += 1;
  }

  fn on_datum_ref_dropped(&mut self, id: DatumId) {
    let entry = self.datums.get_mut(id).unwrap().as_mut().unwrap();
    entry.ref_count -= 1;
    if entry.ref_count <= 0 {
      self.dealloc_datum(id);
    }
  }

  fn reset(&mut self) {
    self.datums.clear();
    self.datum_id_counter = 0;
  }
}

pub async fn run_allocator_loop(rx: Receiver<DatumAllocatorEvent>) {
  while !rx.is_closed() {
    let item = rx.recv().await.unwrap();
    let mut player_lock = PLAYER_LOCK.lock().await;
    let player = player_lock.as_mut().unwrap();
  
    match item {
      DatumAllocatorEvent::RefAdded(id) => player.allocator.on_datum_ref_added(id),
      DatumAllocatorEvent::RefDropped(id) => player.allocator.on_datum_ref_dropped(id),
    }
  }
}