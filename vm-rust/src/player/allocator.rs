use std::cell::UnsafeCell;

use fxhash::FxHashMap;
use log::{debug, warn};

use crate::director::lingo::datum::Datum;

use super::{
    datum_ref::{DatumId, DatumRef},
    script::{ScriptInstance, ScriptInstanceId},
    script_ref::ScriptInstanceRef,
    ScriptError,
};

/// Flag set during allocator reset to skip DatumRef::drop logic.
/// During reset, arena entries are cleared one by one; inner DatumRefs
/// may point to already-freed entries, so we must not dereference their
/// ref_count pointers.
pub static mut ALLOCATOR_RESETTING: bool = false;

const ARENA_CHUNK_SIZE: usize = 4096;

const INT_POOL_MIN: i32 = -128;
const INT_POOL_MAX: i32 = 255;
const INT_POOL_SIZE: usize = (INT_POOL_MAX - INT_POOL_MIN + 1) as usize; // 384

pub struct Arena<T> {
    chunks: Vec<Box<[Option<T>]>>,
    free_list: Vec<usize>,
    count: usize,
    next_slot: usize,
}

impl<T> Arena<T> {
    pub fn new() -> Self {
        Arena {
            chunks: Vec::new(),
            free_list: Vec::new(),
            count: 0,
            next_slot: 0,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let num_chunks = (capacity + ARENA_CHUNK_SIZE - 1) / ARENA_CHUNK_SIZE;
        let mut chunks = Vec::with_capacity(num_chunks);
        for _ in 0..num_chunks {
            chunks.push(Self::new_chunk());
        }
        Arena {
            chunks,
            free_list: Vec::with_capacity(capacity),
            count: 0,
            next_slot: 0,
        }
    }

    fn new_chunk() -> Box<[Option<T>]> {
        let mut chunk = Vec::with_capacity(ARENA_CHUNK_SIZE);
        chunk.resize_with(ARENA_CHUNK_SIZE, || None);
        chunk.into_boxed_slice()
    }

    fn ensure_chunk(&mut self, chunk_idx: usize) {
        while self.chunks.len() <= chunk_idx {
            self.chunks.push(Self::new_chunk());
        }
    }

    #[inline]
    pub fn alloc(&mut self, value: T) -> usize {
        self.count += 1;
        if let Some(idx) = self.free_list.pop() {
            self.chunks[idx / ARENA_CHUNK_SIZE][idx % ARENA_CHUNK_SIZE] = Some(value);
            idx + 1
        } else {
            let idx = self.next_slot;
            self.ensure_chunk(idx / ARENA_CHUNK_SIZE);
            self.chunks[idx / ARENA_CHUNK_SIZE][idx % ARENA_CHUNK_SIZE] = Some(value);
            self.next_slot += 1;
            idx + 1
        }
    }

    pub fn insert_at(&mut self, id: usize, value: T) {
        let idx = id - 1;
        self.ensure_chunk(idx / ARENA_CHUNK_SIZE);
        let chunk_idx = idx / ARENA_CHUNK_SIZE;
        let slot_idx = idx % ARENA_CHUNK_SIZE;
        // Use take() to safely drop the old value (if any) before inserting
        let was_empty = self.chunks[chunk_idx][slot_idx].take().is_none();
        self.chunks[chunk_idx][slot_idx] = Some(value);
        if was_empty {
            self.count += 1;
        }
        if idx >= self.next_slot {
            self.next_slot = idx + 1;
        }
    }

    #[inline]
    pub fn remove(&mut self, id: usize) -> Option<T> {
        if id == 0 {
            return None;
        }
        let idx = id - 1;
        let chunk_idx = idx / ARENA_CHUNK_SIZE;
        if chunk_idx < self.chunks.len() {
            let slot_idx = idx % ARENA_CHUNK_SIZE;
            if let Some(value) = self.chunks[chunk_idx][slot_idx].take() {
                self.free_list.push(idx);
                self.count -= 1;
                Some(value)
            } else {
                None
            }
        } else {
            None
        }
    }

    #[inline]
    pub fn get(&self, id: usize) -> Option<&T> {
        if id == 0 {
            return None;
        }
        let idx = id - 1;
        let chunk_idx = idx / ARENA_CHUNK_SIZE;
        if chunk_idx < self.chunks.len() {
            self.chunks[chunk_idx][idx % ARENA_CHUNK_SIZE].as_ref()
        } else {
            None
        }
    }

    #[inline]
    pub fn get_mut(&mut self, id: usize) -> Option<&mut T> {
        if id == 0 {
            return None;
        }
        let idx = id - 1;
        let chunk_idx = idx / ARENA_CHUNK_SIZE;
        if chunk_idx < self.chunks.len() {
            self.chunks[chunk_idx][idx % ARENA_CHUNK_SIZE].as_mut()
        } else {
            None
        }
    }

    #[inline]
    pub fn contains(&self, id: usize) -> bool {
        if id == 0 {
            return false;
        }
        let idx = id - 1;
        let chunk_idx = idx / ARENA_CHUNK_SIZE;
        chunk_idx < self.chunks.len()
            && self.chunks[chunk_idx][idx % ARENA_CHUNK_SIZE].is_some()
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, &T)> {
        let next_slot = self.next_slot;
        (0..next_slot).filter_map(move |idx| {
            let chunk_idx = idx / ARENA_CHUNK_SIZE;
            let slot_idx = idx % ARENA_CHUNK_SIZE;
            if chunk_idx < self.chunks.len() {
                self.chunks[chunk_idx][slot_idx].as_ref().map(|v| (idx + 1, v))
            } else {
                None
            }
        })
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
        self.free_list.clear();
        self.count = 0;
        self.next_slot = 0;
    }

    pub fn clear_individually_reverse(&mut self) {
        for chunk_idx in (0..self.chunks.len()).rev() {
            for slot_idx in (0..ARENA_CHUNK_SIZE).rev() {
                // Use take() so the slot is set to None BEFORE the value is
                // dropped. This ensures re-entrant contains() checks during
                // drop cascades correctly see the slot as empty.
                drop(self.chunks[chunk_idx][slot_idx].take());
            }
        }
        self.free_list.clear();
        self.count = 0;
        self.next_slot = 0;
    }

    pub fn clear_individually(&mut self) {
        for chunk_idx in 0..self.chunks.len() {
            for slot_idx in 0..ARENA_CHUNK_SIZE {
                drop(self.chunks[chunk_idx][slot_idx].take());
            }
        }
        self.free_list.clear();
        self.count = 0;
        self.next_slot = 0;
    }
}

pub struct DatumRefEntry {
    pub id: DatumId,
    pub ref_count: UnsafeCell<u32>,
    pub datum: Datum,
}

pub struct ScriptInstanceRefEntry {
    pub id: ScriptInstanceId,
    pub ref_count: UnsafeCell<u32>,
    pub script_instance: ScriptInstance,
}

pub trait ResetableAllocator {
    fn reset(&mut self);
}

pub trait DatumAllocatorTrait {
    fn alloc_datum(&mut self, datum: Datum) -> Result<DatumRef, ScriptError>;
    fn get_datum(&self, id: &DatumRef) -> &Datum;
    fn get_datum_mut(&mut self, id: &DatumRef) -> &mut Datum;
    fn on_datum_ref_dropped(&mut self, id: DatumId);
}

pub trait ScriptInstanceAllocatorTrait {
    fn alloc_script_instance(&mut self, script_instance: ScriptInstance) -> ScriptInstanceRef;
    fn get_script_instance(&self, instance_ref: &ScriptInstanceRef) -> &ScriptInstance;
    fn get_script_instance_opt(&self, instance_ref: &ScriptInstanceRef) -> Option<&ScriptInstance>;
    fn get_script_instance_mut(&mut self, instance_ref: &ScriptInstanceRef) -> &mut ScriptInstance;
    fn on_script_instance_ref_dropped(&mut self, id: ScriptInstanceId);
}

pub struct DatumAllocator {
    pub datums: Arena<DatumRefEntry>,
    pub script_instances: Arena<ScriptInstanceRefEntry>,
    script_instance_counter: ScriptInstanceId,
    void_datum: Datum,
    pub int_alloc_count: usize,
    pub int_dealloc_count: usize,
    pub snapshot_max_id: usize,
    int_pool_ids: [DatumId; INT_POOL_SIZE],
    symbol_pool: FxHashMap<String, DatumId>,
}

const MAX_SCRIPT_INSTANCE_ID: ScriptInstanceId = 0xFFFFFF;

impl DatumAllocator {
    pub fn default() -> Self {
        let mut alloc = DatumAllocator {
            datums: Arena::with_capacity(4096),
            script_instances: Arena::new(),
            script_instance_counter: 1,
            void_datum: Datum::Void,
            int_alloc_count: 0,
            int_dealloc_count: 0,
            snapshot_max_id: 0,
            int_pool_ids: [0; INT_POOL_SIZE],
            symbol_pool: FxHashMap::default(),
        };
        alloc.init_int_pool();
        alloc
    }

    fn init_int_pool(&mut self) {
        for i in 0..INT_POOL_SIZE {
            let n = (i as i32) + INT_POOL_MIN;
            let entry = DatumRefEntry {
                id: 0,
                ref_count: UnsafeCell::new(u32::MAX),
                datum: Datum::Int(n),
            };
            let id = self.datums.alloc(entry);
            self.datums.get_mut(id).unwrap().id = id;
            self.int_pool_ids[i] = id;
        }
    }

    pub fn contains_datum(&self, id: DatumId) -> bool {
        self.datums.contains(id)
    }

    pub fn get_free_script_instance_id(&self) -> ScriptInstanceId {
        if self.script_instance_count() >= MAX_SCRIPT_INSTANCE_ID as usize {
            panic!("Script instance limit reached");
        }
        if !self.script_instances.contains(self.script_instance_counter as usize) {
            self.script_instance_counter
        } else if self.script_instance_counter + 1 < MAX_SCRIPT_INSTANCE_ID
            && !self
                .script_instances
                .contains((self.script_instance_counter + 1) as usize)
        {
            self.script_instance_counter + 1
        } else {
            warn!("Script instance id overflow. Searching for free id...");
            let first_free_id = (1..MAX_SCRIPT_INSTANCE_ID)
                .find(|id| !self.script_instances.contains(*id as usize));
            if let Some(id) = first_free_id {
                id
            } else {
                panic!("Failed to find free script instance id");
            }
        }
    }

    pub fn script_instance_count(&self) -> usize {
        self.script_instances.len()
    }

    pub fn datum_count(&self) -> usize {
        self.datums.len()
    }

    pub fn datum_type_stats(&self) -> String {
        let mut counts: std::collections::HashMap<String, (usize, usize)> = std::collections::HashMap::new();
        // Int-specific tracking
        let mut int_rc_dist: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
        let mut int_value_dist: std::collections::HashMap<i32, usize> = std::collections::HashMap::new();
        let mut int_samples: Vec<(usize, i32, u32)> = Vec::new(); // (id, value, rc)

        for (id, entry) in self.datums.iter() {
            let type_name = entry.datum.type_str().to_string();
            let rc = unsafe { *entry.ref_count.get() };
            let (count, total_rc) = counts.entry(type_name).or_insert((0, 0));
            *count += 1;
            *total_rc += rc as usize;

            // Track Int datum details
            if let Datum::Int(val) = &entry.datum {
                *int_rc_dist.entry(rc).or_insert(0) += 1;
                *int_value_dist.entry(*val).or_insert(0) += 1;
                if int_samples.len() < 20 {
                    int_samples.push((id, *val, rc));
                }
            }
        }
        let mut sorted: Vec<_> = counts.into_iter().collect();
        sorted.sort_by(|a, b| b.1.0.cmp(&a.1.0));
        let mut result = format!("Live datums: {}\n", self.datums.len());
        for (type_name, (count, total_rc)) in &sorted {
            result.push_str(&format!("  {}: {} (total rc: {})\n", type_name, count, total_rc));
        }
        result.push_str(&format!("Free list: {}\n", self.datums.free_list.len()));

        // Int ref count distribution
        let mut rc_sorted: Vec<_> = int_rc_dist.into_iter().collect();
        rc_sorted.sort_by_key(|&(rc, _)| rc);
        result.push_str("Int rc distribution:\n");
        for (rc, count) in &rc_sorted {
            result.push_str(&format!("  rc={}: {}\n", rc, count));
        }

        // Int value distribution (top 15 most common values)
        let mut val_sorted: Vec<_> = int_value_dist.into_iter().collect();
        val_sorted.sort_by(|a, b| b.1.cmp(&a.1));
        result.push_str("Int value distribution (top 15):\n");
        for (val, count) in val_sorted.iter().take(15) {
            result.push_str(&format!("  val={}: {}\n", val, count));
        }

        // Int alloc/dealloc counters
        result.push_str(&format!("Int allocs: {}, deallocs: {}, delta: {}\n",
            self.int_alloc_count, self.int_dealloc_count,
            self.int_alloc_count as i64 - self.int_dealloc_count as i64));

        // Show new Int datums since snapshot (if set)
        if self.snapshot_max_id > 0 {
            let mut new_ints = 0;
            let mut new_int_samples: Vec<(usize, i32, u32)> = Vec::new();
            for (id, entry) in self.datums.iter() {
                if id > self.snapshot_max_id {
                    if let Datum::Int(val) = &entry.datum {
                        let rc = unsafe { *entry.ref_count.get() };
                        new_ints += 1;
                        if new_int_samples.len() < 20 {
                            new_int_samples.push((id, *val, rc));
                        }
                    }
                }
            }
            result.push_str(&format!("New Int datums since snapshot (id>{}): {}\n",
                self.snapshot_max_id, new_ints));
            result.push_str("New Int samples:\n");
            for (id, val, rc) in &new_int_samples {
                result.push_str(&format!("  #{}: val={}, rc={}\n", id, val, rc));
            }
        }

        result
    }

    pub fn take_datum_snapshot(&mut self) {
        self.snapshot_max_id = self.datums.next_slot;
        self.int_alloc_count = 0;
        self.int_dealloc_count = 0;
    }

    fn dealloc_datum(&mut self, id: DatumId) {
        if let Some(entry) = self.datums.get(id) {
            if unsafe { *entry.ref_count.get() } == u32::MAX {
                return; // Pooled/immortal entry, never free
            }
            if matches!(&entry.datum, Datum::Int(_)) {
                self.int_dealloc_count += 1;
            }
        }
        self.datums.remove(id);
    }

    fn dealloc_script_instance(&mut self, id: ScriptInstanceId) {
        self.script_instances.remove(id as usize);
    }

    pub fn get_datum_ref(&self, id: DatumId) -> Option<DatumRef> {
        if let Some(entry) = self.datums.get(id) {
            Some(DatumRef::from_id(id, entry.ref_count.get()))
        } else {
            None
        }
    }

    pub fn get_script_instance_ref(&self, id: ScriptInstanceId) -> Option<ScriptInstanceRef> {
        if let Some(entry) = self.script_instances.get(id as usize) {
            Some(ScriptInstanceRef::from_id(id, entry.ref_count.get()))
        } else {
            None
        }
    }

    pub fn get_script_instance_entry(
        &self,
        id: ScriptInstanceId,
    ) -> Option<&ScriptInstanceRefEntry> {
        self.script_instances.get(id as usize)
    }

    pub fn get_script_instance_entry_mut(
        &mut self,
        id: ScriptInstanceId,
    ) -> Option<&mut ScriptInstanceRefEntry> {
        self.script_instances.get_mut(id as usize)
    }
}

impl DatumAllocatorTrait for DatumAllocator {
    #[inline]
    fn alloc_datum(&mut self, datum: Datum) -> Result<DatumRef, ScriptError> {
        if datum.is_void() {
            return Ok(DatumRef::Void);
        }

        // Return pooled entry for common int values
        if let Datum::Int(n) = &datum {
            if *n >= INT_POOL_MIN && *n <= INT_POOL_MAX {
                let pool_idx = (*n - INT_POOL_MIN) as usize;
                let id = self.int_pool_ids[pool_idx];
                let entry = self.datums.get(id).unwrap();
                return Ok(DatumRef::Ref(id, entry.ref_count.get()));
            }
        }

        // Intern symbols: same symbol string returns the same pooled entry
        if let Datum::Symbol(s) = &datum {
            if let Some(&id) = self.symbol_pool.get(s) {
                let entry = self.datums.get(id).unwrap();
                return Ok(DatumRef::Ref(id, entry.ref_count.get()));
            }
            // First time seeing this symbol — allocate and register
            let key = s.clone();
            let entry = DatumRefEntry {
                id: 0,
                ref_count: UnsafeCell::new(u32::MAX),
                datum,
            };
            let id = self.datums.alloc(entry);
            self.datums.get_mut(id).unwrap().id = id;
            self.symbol_pool.insert(key, id);
            let entry = self.datums.get(id).unwrap();
            return Ok(DatumRef::Ref(id, entry.ref_count.get()));
        }

        let is_int = matches!(&datum, Datum::Int(_));
        let entry = DatumRefEntry {
            id: 0,
            ref_count: UnsafeCell::new(1), // Start at 1 to avoid the extra increment in from_id
            datum,
        };
        let id = self.datums.alloc(entry);
        let entry = self.datums.get_mut(id).unwrap();
        entry.id = id;
        let ref_count_ptr = entry.ref_count.get();
        if is_int {
            self.int_alloc_count += 1;
        }
        Ok(DatumRef::Ref(id, ref_count_ptr))
    }

    #[inline]
    fn get_datum(&self, id: &DatumRef) -> &Datum {
        match id {
            DatumRef::Ref(id, ..) => {
                let entry = unsafe { self.datums.get(*id).unwrap_unchecked() };
                &entry.datum
            }
            DatumRef::Void => &Datum::Void,
        }
    }

    #[inline]
    fn get_datum_mut(&mut self, id: &DatumRef) -> &mut Datum {
        match id {
            DatumRef::Ref(id, ..) => {
                let entry = unsafe { self.datums.get_mut(*id).unwrap_unchecked() };
                &mut entry.datum
            }
            DatumRef::Void => &mut self.void_datum,
        }
    }

    #[inline]
    fn on_datum_ref_dropped(&mut self, id: DatumId) {
        self.dealloc_datum(id);
    }
}

impl ScriptInstanceAllocatorTrait for DatumAllocator {
    fn alloc_script_instance(&mut self, script_instance: ScriptInstance) -> ScriptInstanceRef {
        let id = script_instance.instance_id;
        self.script_instance_counter += 1;
        self.script_instances.insert_at(
            id as usize,
            ScriptInstanceRefEntry {
                id,
                ref_count: UnsafeCell::new(0),
                script_instance,
            },
        );
        let ref_count_ptr = self
            .script_instances
            .get(id as usize)
            .unwrap()
            .ref_count
            .get();
        ScriptInstanceRef::from_id(id, ref_count_ptr)
    }

    fn get_script_instance(&self, instance_ref: &ScriptInstanceRef) -> &ScriptInstance {
        &self
            .script_instances
            .get(instance_ref.id() as usize)
            .unwrap()
            .script_instance
    }

    fn get_script_instance_opt(
        &self,
        instance_ref: &ScriptInstanceRef,
    ) -> Option<&ScriptInstance> {
        self.script_instances
            .get(instance_ref.id() as usize)
            .map(|entry| &entry.script_instance)
    }

    fn get_script_instance_mut(
        &mut self,
        instance_ref: &ScriptInstanceRef,
    ) -> &mut ScriptInstance {
        &mut self
            .script_instances
            .get_mut(instance_ref.id() as usize)
            .unwrap()
            .script_instance
    }

    fn on_script_instance_ref_dropped(&mut self, id: ScriptInstanceId) {
        self.dealloc_script_instance(id);
    }
}

impl ResetableAllocator for DatumAllocator {
    fn reset(&mut self) {
        // Remove entries individually to ensure proper Drop cleanup.
        // Datum Drop impls may reference other datums, so reverse order
        // helps ensure dependents are dropped before their dependencies.
        unsafe { ALLOCATOR_RESETTING = true; }

        debug!("Removing all datums");
        self.datums.clear_individually_reverse();

        debug!("Removing all script instances");
        self.script_instances.clear_individually();

        self.script_instance_counter = 1;

        unsafe { ALLOCATOR_RESETTING = false; }

        // Re-create pools after clearing
        self.symbol_pool.clear();
        self.init_int_pool();
    }
}
