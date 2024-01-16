use crate::RuntimeContext;
use fluentbase_rwasm::{common::Trap, Caller};

pub struct SysInputSize;

impl SysInputSize {
    pub fn fn_handler<T>(caller: Caller<'_, RuntimeContext<T>>) -> Result<u32, Trap> {
        Ok(caller.data().input_size())
    }

    pub fn fn_impl() {}
}
