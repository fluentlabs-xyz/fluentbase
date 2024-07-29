use crate::RuntimeContext;
use fluentbase_types::{ExitCode, JournalCheckpoint};
use rwasm::{core::Trap, Caller};

pub struct SyscallCheckpoint;

impl SyscallCheckpoint {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<u64, Trap> {
        let checkpoint = Self::fn_impl(caller.data_mut()).map_err(|err| err.into_trap())?;
        Ok(checkpoint.to_u64())
    }

    pub fn fn_impl(context: &RuntimeContext) -> Result<JournalCheckpoint, ExitCode> {
        let checkpoint = context.jzkt().borrow().checkpoint();
        Ok(checkpoint)
    }
}
