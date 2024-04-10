use crate::RuntimeContext;
use fluentbase_types::{ExitCode, IJournaledTrie, JournalCheckpoint};
use rwasm::{core::Trap, Caller};

pub struct JzktCheckpoint;

impl JzktCheckpoint {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
    ) -> Result<u64, Trap> {
        let checkpoint = Self::fn_impl(caller.data_mut()).map_err(|err| err.into_trap())?;
        Ok(checkpoint.to_u64())
    }

    pub fn fn_impl<DB: IJournaledTrie>(
        context: &mut RuntimeContext<DB>,
    ) -> Result<JournalCheckpoint, ExitCode> {
        let jzkt = context.jzkt.as_mut().expect("jzkt is not set");
        let checkpoint = jzkt.checkpoint();
        Ok(checkpoint)
    }
}
