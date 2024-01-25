use crate::RuntimeContext;
use fluentbase_rwasm::{common::Trap, Caller};

pub struct ZkTrieSetCode;

impl ZkTrieSetCode {
    pub fn fn_handler<T>(
        _caller: Caller<'_, RuntimeContext<T>>,
        key20_offset: u32,
        code_offset: u32,
        code_len: u32,
    ) -> Result<(), Trap> {
        Ok(())
    }

    pub fn fn_impl() {}
}
