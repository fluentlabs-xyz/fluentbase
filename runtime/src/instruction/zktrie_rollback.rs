use crate::RuntimeContext;
use fluentbase_rwasm::{common::Trap, Caller};

pub struct ZkTrieRollback;

impl ZkTrieRollback {
    pub fn fn_handler<T>(_caller: Caller<'_, RuntimeContext<T>>) -> Result<(), Trap> {
        Ok(())
    }

    pub fn fn_impl() {}
}
