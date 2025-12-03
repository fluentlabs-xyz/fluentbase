use alloc::vec::Vec;
use core::mem::take;
use fluentbase_sdk::{Bytes, B256, U256};
use hashbrown::hash_map::Entry;
use hashbrown::HashMap;
use spin::{Mutex, MutexGuard};

pub const GLOBAL_SERVICE_VALUES_CAP: usize = 4;
pub const GLOBAL_SERVICE_EVENT_CAP: usize = 4;

#[derive(Debug)]
pub struct GlobalService {
    existing_values: HashMap<U256, U256>,
    new_values: HashMap<U256, U256>,
    events: Vec<(Vec<B256>, Bytes)>,
}

impl GlobalService {
    pub fn new() -> Self {
        Self {
            existing_values: HashMap::with_capacity(GLOBAL_SERVICE_VALUES_CAP),
            new_values: HashMap::with_capacity(GLOBAL_SERVICE_VALUES_CAP),
            events: Vec::with_capacity(GLOBAL_SERVICE_EVENT_CAP),
        }
    }

    pub fn set_value(&mut self, key: &U256, value: &U256) {
        let entry = self.existing_values.entry(*key);
        match entry {
            Entry::Occupied(v) => {
                if value == v.get() {
                    return;
                }
            }
            _ => {}
        }
        let _ = self.new_values.insert(key.clone(), value.clone());
    }

    pub fn try_get_value(&self, slot: &U256) -> Option<&U256> {
        if let Some(v) = self.new_values.get(slot) {
            return Some(v);
        }
        if let Some(v) = self.existing_values.get(slot) {
            return Some(v);
        }
        None
    }

    pub fn set_existing_value(&mut self, key: &U256, value: &U256) -> Option<U256> {
        self.existing_values.insert(key.clone(), value.clone())
    }

    pub fn set_existing_values(&mut self, map: HashMap<U256, U256>) {
        self.existing_values = map;
    }

    pub fn existing_values(&self) -> &HashMap<U256, U256> {
        &self.existing_values
    }

    pub fn clear_existing_values(&mut self) {
        self.existing_values.clear();
    }

    pub fn new_values(&self) -> &HashMap<U256, U256> {
        &self.new_values
    }

    pub fn take_new_values(&mut self) -> HashMap<U256, U256> {
        take(&mut self.new_values)
    }

    pub fn clear_new_values(&mut self) {
        self.new_values.clear();
    }

    pub fn events(&self) -> &Vec<(Vec<B256>, Bytes)> {
        &self.events
    }

    pub fn take_events(&mut self) -> Vec<(Vec<B256>, Bytes)> {
        take(&mut self.events)
    }

    pub fn add_event(&mut self, topics: Vec<B256>, data: Bytes) {
        self.events.push((topics, data));
    }

    pub fn clear_events(&mut self) {
        self.events.clear();
    }

    pub fn clear(&mut self) {
        self.existing_values.clear();
        self.new_values.clear();
        self.events.clear();
    }
}

pub static GLOBAL_SERVICE: spin::Once<Mutex<GlobalService>> = spin::Once::new();

pub fn global_service<'a>() -> MutexGuard<'a, GlobalService> {
    GLOBAL_SERVICE
        .call_once(|| {
            let service = GlobalService::new();
            Mutex::new(service)
        })
        .lock()
}
