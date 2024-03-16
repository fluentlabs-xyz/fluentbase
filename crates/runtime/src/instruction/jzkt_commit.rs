use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{core::Trap, Caller};

pub struct JzktCommit;

impl JzktCommit {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        root32_offset: u32,
    ) -> Result<(), Trap> {
        let output = Self::fn_impl(caller.data_mut()).map_err(|err| err.into_trap())?;
        caller.write_memory(root32_offset, &output);
        Ok(())
    }

    pub fn fn_impl<T>(ctx: &mut RuntimeContext<T>) -> Result<[u8; 32], ExitCode> {
        let jzkt = ctx.jzkt.clone().unwrap();
        let (root, _logs) = jzkt.borrow_mut().commit()?;
        Ok(root)
    }
}
