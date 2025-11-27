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
    values_existing: HashMap<U256, U256>,
    values_new: HashMap<U256, U256>,
    events: Vec<(Vec<[u8; 32]>, Vec<u8>)>,
}

impl GlobalService {
    pub fn new() -> Self {
        Self {
            values_existing: HashMap::with_capacity(GLOBAL_SERVICE_VALUES_CAP),
            values_new: HashMap::with_capacity(GLOBAL_SERVICE_VALUES_CAP),
            events: Vec::with_capacity(GLOBAL_SERVICE_EVENT_CAP),
        }
    }

    pub fn set_existing(&mut self, key: &U256, value: &U256) -> Option<U256> {
        self.values_existing.insert(key.clone(), value.clone())
    }

    pub fn set_value(&mut self, key: &U256, value: &U256) -> Option<U256> {
        let entry = self.values_existing.entry(*key);
        match entry {
            Entry::Occupied(v) => {
                if value == v.get() {
                    return None;
                }
            }
            _ => {}
        }
        self.values_new.insert(key.clone(), value.clone())
    }

    pub fn try_get_value(&self, slot: &U256) -> Option<&U256> {
        if let Some(v) = self.values_new.get(slot) {
            return Some(v);
        }
        if let Some(v) = self.values_existing.get(slot) {
            return Some(v);
        }
        None
    }

    pub fn values_existing(&self) -> &HashMap<U256, U256> {
        &self.values_existing
    }

    pub fn values_existing_clear(&mut self) -> bool {
        let has_some = self.values_existing.len() > 0;
        if has_some {
            self.values_existing.clear();
        }
        has_some
    }

    pub fn values_new(&self) -> &HashMap<U256, U256> {
        &self.values_new
    }

    pub fn values_new_clear(&mut self) -> bool {
        let has_some = self.values_new.len() > 0;
        if has_some {
            self.values_new.clear();
        }
        has_some
    }

    pub fn events(&self) -> &Vec<(Vec<[u8; 32]>, Vec<u8>)> {
        &self.events
    }

    pub fn events_take(&mut self) -> Vec<(Vec<[u8; 32]>, Vec<u8>)> {
        take(&mut self.events)
    }

    pub fn events_add(&mut self, topics: Vec<[u8; 32]>, data: Vec<u8>) {
        self.events.push((topics, data));
    }

    pub fn events_clear(&mut self) -> bool {
        let has_some = self.events.len() > 0;
        if has_some {
            self.events.clear();
        }
        has_some
    }

    pub fn clear(&mut self) {
        self.values_existing.clear();
        self.values_new.clear();
        self.events.clear();
    }
}
