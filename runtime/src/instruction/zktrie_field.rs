use crate::RuntimeContext;
use fluentbase_rwasm::{common::Trap, Caller};

pub struct ZkTrieField;

impl ZkTrieField {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        key32_offset: u32,
        output_offset: u32,
    ) -> Result<(), Trap> {
        Ok(())
    }

    pub fn fn_impl() {}
}
