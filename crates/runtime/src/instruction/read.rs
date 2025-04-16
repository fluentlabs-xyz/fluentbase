use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm_executor::{Caller, RwasmError};

pub struct SyscallRead;

impl SyscallRead {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let [target_ptr, offset, length] = caller.stack_pop_n();
        let input = Self::fn_impl(caller.context(), offset.as_u32(), length.as_u32())?;
        let _ = caller.memory_write(target_ptr.as_usize(), &input)?;
        Ok(())
    }

    pub fn fn_impl(ctx: &RuntimeContext, offset: u32, length: u32) -> Result<Vec<u8>, RwasmError> {
        if offset + length <= ctx.input.len() as u32 {
            Ok(ctx.input[(offset as usize)..(offset as usize + length as usize)].to_vec())
        } else {
            Err(RwasmError::ExecutionHalted(
                ExitCode::InputOutputOutOfBounds.into_i32(),
            ))
        }
    }
}
