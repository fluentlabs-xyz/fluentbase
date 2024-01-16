use crate::RuntimeContext;
use fluentbase_rwasm::{common::Trap, Caller};

pub struct ZkTrieOpen;

impl ZkTrieOpen {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        root32_offset: u32,
    ) -> Result<(), Trap> {
        let zktrie = caller.data_mut().zktrie.clone().unwrap();

        Ok(())
    }

    pub fn fn_impl(root: &[u8; 32]) {}
}
