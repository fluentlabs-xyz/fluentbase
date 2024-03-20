use crate::{journal::JournalCheckpoint, RuntimeContext};
use rwasm::{core::Trap, Caller};

pub struct JzktRollback;

impl JzktRollback {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        checkpoint: u64,
    ) -> Result<(), Trap> {
        Self::fn_impl(caller.data_mut(), JournalCheckpoint::from_u64(checkpoint));
        Ok(())
    }

    pub fn fn_impl<T>(ctx: &mut RuntimeContext<T>, checkpoint: JournalCheckpoint) {
        let jzkt = ctx.jzkt.clone().unwrap();
        jzkt.borrow_mut().rollback(checkpoint);
    }
}
