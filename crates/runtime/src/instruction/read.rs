use crate::RuntimeContext;
use fluentbase_rwasm::{Caller, RwasmError};
use fluentbase_types::ExitCode;

pub struct SyscallRead;

impl SyscallRead {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let [target_ptr, offset, length] = caller.stack_pop_n();
        let input = Self::fn_impl(caller.data(), offset.as_u32(), length.as_u32())
            .map_err(|err| RwasmError::ExecutionHalted(err.into_i32()))?;
        let _ = caller.memory_write(target_ptr.as_usize(), &input)?;
        Ok(())
    }

    pub fn fn_impl(ctx: &RuntimeContext, offset: u32, length: u32) -> Result<Vec<u8>, ExitCode> {
        if offset + length <= ctx.input.len() as u32 {
            Ok(ctx.input[(offset as usize)..(offset as usize + length as usize)].to_vec())
        } else {
            Err(ExitCode::InputOutputOutOfBounds)
        }
    }
}
