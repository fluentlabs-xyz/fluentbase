use crate::RuntimeContext;
use fluentbase_types::{ExitCode, IJournaledTrie};
use rwasm::{core::Trap, Caller};

pub struct JzktCommit;

impl JzktCommit {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        root32_offset: u32,
    ) -> Result<(), Trap> {
        let output = Self::fn_impl(caller.data_mut()).map_err(|err| err.into_trap())?;
        let _ = caller.write_memory(root32_offset, &output)?;
        Ok(())
    }

    pub fn fn_impl<DB: IJournaledTrie>(ctx: &mut RuntimeContext<DB>) -> Result<[u8; 32], ExitCode> {
        let (root, _logs) = ctx.jzkt().commit()?;
        Ok(root)
    }
}
