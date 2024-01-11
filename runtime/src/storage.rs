mod noop;
#[cfg(feature = "zktrie")]
mod zktrie;

use crate::RuntimeError;
use std::{collections::HashMap, fmt::Debug};

pub trait PersistentDatabase {
    fn put(&mut self, key: &[u8; 32], value: &[u8]);

    fn get(&self, key: &[u8; 32]) -> Option<&Vec<u8>>;
}

#[derive(Default)]
pub struct InMemoryDatabase(HashMap<[u8; 32], Vec<u8>>);

impl PersistentDatabase for InMemoryDatabase {
    fn put(&mut self, key: &[u8; 32], value: &[u8]) {
        self.0.insert(*key, value.to_vec());
    }

    fn get(&self, key: &[u8; 32]) -> Option<&Vec<u8>> {
        self.0.get(key)
    }
}

pub trait PersistentStorage {
    fn open(&self, key: &[u8; 32]) -> Result<Self, RuntimeError>
    where
        Self: Sized;

    fn compute_root(&self) -> [u8; 32];

    fn get(&self, key: &[u8; 32]) -> Option<[u8; 32]>;

    fn update(&mut self, key: &[u8; 32], value: &[u8; 32]) -> Result<(), RuntimeError>;
}
