use alloc::vec::Vec;
use core::mem::take;
use fluentbase_sdk::{B256, U256};
use hashbrown::hash_map::Entry;
use hashbrown::{HashMap, HashSet};

pub const GLOBAL_SERVICE_VALUES_CAP: usize = 32;
pub const GLOBAL_SERVICE_QUERY_CAP: usize = 8;
pub const GLOBAL_SERVICE_EVENT_CAP: usize = 32;

#[derive(Debug)]
pub struct GlobalService {
    default_storage_on_read: bool,
    values_existing: HashMap<U256, U256>,
    values_new: HashMap<U256, U256>,
    keys_to_query: HashSet<U256>,
    events: Vec<(Vec<[u8; 32]>, Vec<u8>)>,
}

impl GlobalService {
    pub fn new(default_storage_on_read: bool) -> Self {
        Self {
            default_storage_on_read,
            values_existing: HashMap::with_capacity(GLOBAL_SERVICE_VALUES_CAP),
            values_new: HashMap::with_capacity(GLOBAL_SERVICE_VALUES_CAP),
            keys_to_query: HashSet::with_capacity(GLOBAL_SERVICE_QUERY_CAP),
            events: Vec::with_capacity(GLOBAL_SERVICE_EVENT_CAP),
        }
    }

    pub fn try_set(&mut self, key: &U256, value: &U256) -> Option<U256> {
        let entry = self.values_existing.entry(*key);
        match entry {
            Entry::Occupied(v) => {
                if value == v.get() {
                    return None;
                }
            }
            Entry::Vacant(_) => {}
        }
        if self.values_new.len() >= GLOBAL_SERVICE_VALUES_CAP {
            panic!("new values full");
        }
        self.values_new.insert(key.clone(), value.clone())
    }

    pub fn set_existing(&mut self, key: &U256, value: &U256) -> Option<U256> {
        if self.values_existing.len() >= GLOBAL_SERVICE_VALUES_CAP {
            panic!("existing values full");
        }
        self.values_existing.insert(key.clone(), value.clone())
    }

    pub fn try_get(&mut self, slot: &U256) -> Option<&U256> {
        if self.default_storage_on_read {
            return Some(&U256::ZERO);
        }
        if let Some(v) = self.values_new.get(slot) {
            return Some(v);
        }
        if let Some(v) = self.values_existing.get(slot) {
            return Some(v);
        }
        if self.keys_to_query.len() >= GLOBAL_SERVICE_QUERY_CAP {
            panic!("query stack full");
        }
        self.keys_to_query.insert(*slot);
        None
    }

    pub fn keys_to_query(&self) -> &HashSet<U256> {
        &self.keys_to_query
    }

    pub fn keys_to_query_pop(&mut self) -> Option<U256> {
        let val = self.keys_to_query.iter().next();
        if let Some(v) = val.cloned() {
            self.keys_to_query.remove(&v);
            return Some(v.clone());
        }
        None
    }

    pub fn keys_to_query_clear(&mut self) -> bool {
        let has_some = self.keys_to_query.len() > 0;
        if has_some {
            self.keys_to_query.clear();
        }
        has_some
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

    pub fn values_new_pop(&mut self) -> Option<(U256, U256)> {
        let val = self
            .values_new
            .iter()
            .next()
            .map(|v| (v.0.clone(), v.1.clone()));
        if let Some(v) = val {
            self.values_new.remove(&v.0);
            return Some((v.0, v.1));
        }
        None
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
        self.keys_to_query.clear();
        self.events.clear();
    }
}
