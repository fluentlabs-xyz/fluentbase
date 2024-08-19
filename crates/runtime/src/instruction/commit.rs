use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{core::Trap, Caller};

pub struct SyscallCommit;

impl SyscallCommit {
    pub fn fn_handler(
        mut caller: Caller<'_, RuntimeContext>,
        root32_offset: u32,
    ) -> Result<(), Trap> {
        let output = Self::fn_impl(caller.data_mut()).map_err(|err| err.into_trap())?;
        let _ = caller.write_memory(root32_offset, &output)?;
        Ok(())
    }

    pub fn fn_impl(ctx: &RuntimeContext) -> Result<[u8; 32], ExitCode> {
        let (root, _logs) = ctx.jzkt().commit()?;
        Ok(root)
    }
}
