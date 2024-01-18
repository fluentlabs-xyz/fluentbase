mod noop;
mod zktrie;

use crate::complex_types::RuntimeError;
use std::collections::HashMap;

pub trait KeyValueDb {
    fn put(&mut self, key: &[u8], value: &Vec<u8>);

    fn get(&self, key: &[u8]) -> Option<Vec<u8>>;
}

#[derive(Default)]
pub struct InMemoryDb(HashMap<Vec<u8>, Vec<u8>>);

impl KeyValueDb for InMemoryDb {
    fn put(&mut self, key: &[u8], value: &Vec<u8>) {
        self.0.insert(key.to_vec(), value.clone());
    }

    fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.0.get(&key.to_vec()).cloned()
    }
}

pub trait TrieDb {
    fn open(&self, key: &[u8; 32]) -> Result<Self, RuntimeError>
    where
        Self: Sized;

    fn compute_root(&self) -> [u8; 32];

    fn get(&self, key: &[u8; 32]) -> Option<Vec<u8>>;

    fn update(
        &mut self,
        key: &[u8; 32],
        value_flags: u32,
        value: &Vec<[u8; 32]>,
    ) -> Result<(), RuntimeError>;

    fn proof(&self, key: &[u8; 32]) -> Option<Vec<Vec<u8>>>;
}
