use crate::{ExitCode, RuntimeContext};
use fluentbase_rwasm::{common::Trap, Caller};

pub struct SysRead;

impl SysRead {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        target: u32,
        offset: u32,
        length: u32,
    ) -> Result<u32, Trap> {
        let input = caller.data().input().clone();
        if offset > input.len() as u32 {
            return Err(ExitCode::MemoryOutOfBounds.into());
        }
        let input = &input.as_slice()[(offset as usize)..];
        let copy_length = core::cmp::min(length, input.len() as u32);
        caller.write_memory(target, &input[..copy_length as usize]);
        Ok(input.len() as u32)
    }

    pub fn fn_impl() {}
}
