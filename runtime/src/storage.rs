// mod noop;
mod zktrie;

use fluentbase_types::ExitCode;
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

pub trait StateDb {
    fn open(&mut self, root32: &[u8]);

    fn compute_root(&self) -> [u8; 32];

    fn get(&self, key: &[u8]) -> Option<Vec<[u8; 32]>>;

    fn get_code(&self, key: &[u8]) -> Option<Vec<u8>>;
    fn set_code(&mut self, key: &[u8], code: &[u8]);

    fn update(
        &mut self,
        key: &[u8],
        value_flags: u32,
        value: &Vec<[u8; 32]>,
    ) -> Result<(), ExitCode>;

    fn proof(&self, key: &[u8; 32]) -> Option<Vec<Vec<u8>>>;
}
