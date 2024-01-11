use crate::{storage::PersistentStorage, RuntimeError};

#[derive(Debug, Default, Clone)]
pub struct NoopPersistentStorage;

#[allow(dead_code, unused)]
impl PersistentStorage for NoopPersistentStorage {
    fn open(&self, key: &[u8; 32]) -> Result<Self, RuntimeError>
    where
        Self: Sized,
    {
        todo!()
    }

    fn compute_root(&self) -> [u8; 32] {
        [0u8; 32]
    }

    fn get(&self, key: &[u8; 32]) -> Option<[u8; 32]> {
        None
    }

    fn update(&mut self, key: &[u8; 32], value: &[u8; 32]) -> Result<(), RuntimeError> {
        Ok(())
    }
}
