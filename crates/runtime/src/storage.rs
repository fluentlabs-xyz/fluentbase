use crate::types::Bytes;
use fluentbase_types::ExitCode;

pub trait TrieStorage {
    fn open(&mut self, root32: &[u8]) -> bool;

    fn compute_root(&self) -> [u8; 32];

    fn get(&self, key: &[u8]) -> Option<Vec<[u8; 32]>>;

    fn update(
        &mut self,
        key: &[u8],
        value_flags: u32,
        value: &Vec<[u8; 32]>,
    ) -> Result<(), ExitCode>;

    fn remove(&mut self, key: &[u8]) -> Result<(), ExitCode>;

    fn proof(&self, key: &[u8; 32]) -> Option<Vec<Vec<u8>>>;

    fn get_preimage(&mut self, key: &[u8]) -> Option<Bytes>;

    fn preimage_size(&mut self, key: &[u8]) -> u32 {
        self.get_preimage(key)
            .map(|v| v.len() as u32)
            .unwrap_or_default()
    }

    fn update_preimage(&mut self, key: &[u8], value: Bytes);
}
