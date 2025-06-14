use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{Caller, RwasmError};

pub struct SyscallForwardOutput;

impl SyscallForwardOutput {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let (offset, length) = caller.stack_pop2();
        Self::fn_impl(&mut caller.context_mut(), offset.as_u32(), length.as_u32())
            .map_err(|err| RwasmError::ExecutionHalted(err.into_i32()))?;
        Ok(())
    }

    pub fn fn_impl(ctx: &mut RuntimeContext, offset: u32, len: u32) -> Result<(), ExitCode> {
        if offset + len <= ctx.execution_result.return_data.len() as u32 {
            let ret_data = &ctx.execution_result.return_data
                [(offset as usize)..(offset as usize + len as usize)];
            ctx.execution_result.output.extend_from_slice(ret_data);
            Ok(())
        } else {
            Err(ExitCode::MemoryOutOfBounds)
        }
    }
}
