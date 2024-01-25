// mod noop;
mod zktrie;

use fluentbase_types::ExitCode;

pub trait TrieDb {
    fn open(&mut self, root32: &[u8]);

    fn compute_root(&self) -> [u8; 32];

    fn get(&self, key: &[u8]) -> Option<Vec<[u8; 32]>>;

    fn update(
        &mut self,
        key: &[u8],
        value_flags: u32,
        value: &Vec<[u8; 32]>,
    ) -> Result<(), ExitCode>;

    fn proof(&self, key: &[u8; 32]) -> Option<Vec<Vec<u8>>>;
}
