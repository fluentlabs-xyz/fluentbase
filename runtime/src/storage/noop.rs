use crate::{complex_types::RuntimeError, storage::TrieDb};

#[derive(Debug, Default, Clone)]
pub struct NoopPersistentStorage;

#[allow(dead_code, unused)]
impl TrieDb for NoopPersistentStorage {
    fn open(&self, key: &[u8; 32]) -> Result<Self, RuntimeError>
    where
        Self: Sized,
    {
        Ok(NoopPersistentStorage::default())
    }

    fn compute_root(&self) -> [u8; 32] {
        [0u8; 32]
    }

    fn get(&self, key: &[u8; 32]) -> Option<Vec<u8>> {
        None
    }

    fn update(
        &mut self,
        _key: &[u8; 32],
        _value_flags: u32,
        _value: &Vec<[u8; 32]>,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn proof(&self, key: &[u8; 32]) -> Option<Vec<Vec<u8>>> {
        Some(vec![])
    }
}
