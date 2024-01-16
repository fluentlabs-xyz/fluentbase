use crate::RuntimeContext;
use fluentbase_rwasm::{common::Trap, Caller};

pub struct ZkTrieUpdate;

impl ZkTrieUpdate {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        key32_offset: u32,
        flags: u32,
        vals32_offset: u32,
        vals32_len: u32,
    ) -> Result<(), Trap> {
        Ok(())
    }

    pub fn fn_impl() {}
}
