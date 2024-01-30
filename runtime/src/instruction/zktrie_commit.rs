use crate::RuntimeContext;
use rwasm::{common::Trap, Caller};

pub struct ZkTrieCommit;

impl ZkTrieCommit {
    pub fn fn_handler<T>(_caller: Caller<'_, RuntimeContext<T>>) -> Result<(), Trap> {
        Ok(())
    }

    pub fn fn_impl() {}
}
