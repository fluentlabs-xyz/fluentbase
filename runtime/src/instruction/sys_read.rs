use crate::RuntimeContext;
use fluentbase_rwasm::{common::Trap, Caller};

pub struct SysRead;

impl SysRead {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        target: u32,
        offset: u32,
        length: u32,
    ) -> Result<(), Trap> {
        let input = caller
            .data()
            .read_input(offset, length)
            .map_err(|err| err.into_trap())?
            .to_vec();
        caller.write_memory(target, &input);
        Ok(())
    }

    pub fn fn_impl() {}
}
