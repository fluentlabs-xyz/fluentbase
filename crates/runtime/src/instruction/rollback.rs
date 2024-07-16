use crate::RuntimeContext;
use fluentbase_types::{ExitCode, IJournaledTrie, JournalCheckpoint};
use rwasm::{core::Trap, Caller};

pub struct SyscallRollback;

impl SyscallRollback {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        checkpoint: u64,
    ) -> Result<(), Trap> {
        Self::fn_impl(caller.data_mut(), JournalCheckpoint::from_u64(checkpoint))
            .map_err(|err| err.into_trap())?;
        Ok(())
    }

    pub fn fn_impl<DB: IJournaledTrie>(
        ctx: &RuntimeContext<DB>,
        checkpoint: JournalCheckpoint,
    ) -> Result<(), ExitCode> {
        ctx.jzkt().rollback(checkpoint);
        Ok(())
    }
}
