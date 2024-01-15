use crate::RuntimeContext;
use fluentbase_rwasm::{common::Trap, Caller};

pub struct SysWrite;

impl SysWrite {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        offset: u32,
        length: u32,
    ) -> Result<(), Trap> {
        let data = caller.read_memory(offset, length).to_vec();
        caller.data_mut().extend_return_data(data.as_slice());
        Ok(())
    }

    pub fn fn_impl() {}
}
