use crate::RuntimeContext;
use fluentbase_types::IJournaledTrie;
use rwasm::{core::Trap, Caller};

pub struct SysHalt;

impl SysHalt {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        exit_code: i32,
    ) -> Result<(), Trap> {
        Self::fn_impl(caller.data_mut(), exit_code);
        Err(Trap::i32_exit(exit_code))
    }

    pub fn fn_impl<DB: IJournaledTrie>(ctx: &mut RuntimeContext<DB>, exit_code: i32) {
        ctx.exit_code = exit_code;
    }
}
