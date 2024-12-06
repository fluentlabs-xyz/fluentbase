use crate::RuntimeContext;
use fluentbase_sdk::{ExitCode, JournalCheckpoint};
use rwasm::{core::Trap, Caller};

pub struct SyscallRollback;

impl SyscallRollback {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>, checkpoint: u64) -> Result<(), Trap> {
        Self::fn_impl(caller.data_mut(), JournalCheckpoint::from_u64(checkpoint))
            .map_err(|err| err.into_trap())?;
        Ok(())
    }

    pub fn fn_impl(ctx: &RuntimeContext, checkpoint: JournalCheckpoint) -> Result<(), ExitCode> {
        ctx.jzkt().rollback(checkpoint);
        Ok(())
    }
}
