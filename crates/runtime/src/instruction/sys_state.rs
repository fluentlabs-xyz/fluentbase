use crate::RuntimeContext;
use fluentbase_types::IJournaledTrie;
use rwasm::{core::Trap, Caller};

pub struct SysState;

impl SysState {
    pub fn fn_handler<DB: IJournaledTrie>(
        caller: Caller<'_, RuntimeContext<DB>>,
    ) -> Result<u32, Trap> {
        Ok(Self::fn_impl(caller.data()))
    }

    pub fn fn_impl<DB: IJournaledTrie>(ctx: &RuntimeContext<DB>) -> u32 {
        ctx.state
    }
}
