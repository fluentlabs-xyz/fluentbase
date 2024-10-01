use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{core::Trap, Caller};

pub struct SyscallReadOutput;

impl SyscallReadOutput {
    pub fn fn_handler(
        mut caller: Caller<'_, RuntimeContext>,
        target: u32,
        offset: u32,
        length: u32,
    ) -> Result<(), Trap> {
        let input = Self::fn_impl(caller.data(), offset, length).map_err(|err| err.into_trap())?;
        let _ = caller.write_memory(target, &input)?;
        Ok(())
    }

    pub fn fn_impl(ctx: &RuntimeContext, offset: u32, length: u32) -> Result<Vec<u8>, ExitCode> {
        if offset + length <= ctx.execution_result.return_data.len() as u32 {
            Ok(ctx.execution_result.return_data
                [(offset as usize)..(offset as usize + length as usize)]
                .to_vec())
        } else {
            Err(ExitCode::MemoryOutOfBounds)
        }
    }
}
