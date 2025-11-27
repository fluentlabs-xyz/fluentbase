use alloc::vec::Vec;
use core::mem::take;
use fluentbase_sdk::U256;
use hashbrown::hash_map::Entry;
use hashbrown::HashMap;

pub const GLOBAL_SERVICE_VALUES_CAP: usize = 8;
pub const GLOBAL_SERVICE_QUERY_CAP: usize = 8;
pub const GLOBAL_SERVICE_EVENT_CAP: usize = 8;

#[derive(Debug)]
pub struct GlobalService {
    existing_values: HashMap<U256, U256>,
    new_values: HashMap<U256, U256>,
    events: Vec<(Vec<[u8; 32]>, Vec<u8>)>,
}

impl GlobalService {
    pub fn new() -> Self {
        Self {
            existing_values: HashMap::with_capacity(GLOBAL_SERVICE_VALUES_CAP),
            new_values: HashMap::with_capacity(GLOBAL_SERVICE_VALUES_CAP),
            events: Vec::with_capacity(GLOBAL_SERVICE_EVENT_CAP),
        }
    }

    pub fn set_value(&mut self, key: &U256, value: &U256) -> Option<U256> {
        let entry = self.existing_values.entry(*key);
        match entry {
            Entry::Occupied(v) => {
                if value == v.get() {
                    return None;
                }
            }
            _ => {}
        }
        self.new_values.insert(key.clone(), value.clone())
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

    pub fn set_existing(&mut self, key: &U256, value: &U256) -> Option<U256> {
        self.existing_values.insert(key.clone(), value.clone())
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

    pub fn clear_new_values(&mut self) {
        self.new_values.clear();
    }

    pub fn events(&self) -> &Vec<(Vec<[u8; 32]>, Vec<u8>)> {
        &self.events
    }

    pub fn take_events(&mut self) -> Vec<(Vec<[u8; 32]>, Vec<u8>)> {
        take(&mut self.events)
    }

    pub fn add_event(&mut self, topics: Vec<[u8; 32]>, data: Vec<u8>) {
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
