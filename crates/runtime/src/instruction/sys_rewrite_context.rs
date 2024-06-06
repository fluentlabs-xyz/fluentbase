use crate::RuntimeContext;
use fluentbase_types::{ExitCode, IJournaledTrie};
use rwasm::{core::Trap, Caller};

pub struct SysRewriteContext;

impl SysRewriteContext {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        context_ptr: u32,
        context_len: u32,
    ) -> Result<(), Trap> {
        let new_context = caller.read_memory(context_ptr, context_len)?.to_vec();
        Self::fn_impl(caller.data_mut(), new_context).map_err(|err| err.into_trap())?;
        Ok(())
    }

    pub fn fn_impl<DB: IJournaledTrie>(
        ctx: &mut RuntimeContext<DB>,
        context: Vec<u8>,
    ) -> Result<(), ExitCode> {
        ctx.context = context;
        Ok(())
    }
}
