use crate::RuntimeContext;
use fluentbase_rwasm::{common::Trap, Caller};

pub struct ZkTrieRoot;

impl ZkTrieRoot {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        output32_offset: u32,
    ) -> Result<(), Trap> {
        Ok(())
    }

    pub fn fn_impl() {}
}
