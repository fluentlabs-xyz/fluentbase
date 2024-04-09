use crate::{journal::JournalCheckpoint, RuntimeContext};
use fluentbase_types::ExitCode;
use rwasm::{core::Trap, Caller};

pub struct JzktCheckpoint;

impl JzktCheckpoint {
    pub fn fn_handler<T>(mut caller: Caller<'_, RuntimeContext<T>>) -> Result<u64, Trap> {
        let checkpoint = Self::fn_impl(caller.data_mut()).map_err(|err| err.into_trap())?;
        Ok(checkpoint.to_u64())
    }

    pub fn fn_impl<T>(context: &mut RuntimeContext<T>) -> Result<JournalCheckpoint, ExitCode> {
        let jzkt = context.jzkt.clone().expect("jzkt is not set");
        let checkpoint = jzkt.borrow_mut().checkpoint();
        Ok(checkpoint)
    }
}
