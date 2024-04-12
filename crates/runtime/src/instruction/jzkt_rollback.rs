use crate::RuntimeContext;
use fluentbase_types::{IJournaledTrie, JournalCheckpoint};
use rwasm::{core::Trap, Caller};

pub struct JzktRollback;

impl JzktRollback {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        checkpoint: u64,
    ) -> Result<(), Trap> {
        Self::fn_impl(caller.data_mut(), JournalCheckpoint::from_u64(checkpoint));
        Ok(())
    }

    pub fn fn_impl<DB: IJournaledTrie>(
        ctx: &mut RuntimeContext<DB>,
        checkpoint: JournalCheckpoint,
    ) {
        ctx.jzkt().rollback(checkpoint);
    }
}
