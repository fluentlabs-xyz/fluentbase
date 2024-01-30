use crate::RuntimeContext;
use rwasm::{common::Trap, Caller};

pub struct SysState;

impl SysState {
    pub fn fn_handler<T>(caller: Caller<'_, RuntimeContext<T>>) -> Result<u32, Trap> {
        Ok(caller.data().state)
    }

    pub fn fn_impl() {}
}
