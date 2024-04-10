use crate::RuntimeContext;
use fluentbase_types::IJournaledTrie;
use rwasm::{core::Trap, Caller};

pub struct SysWrite;

impl SysWrite {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        offset: u32,
        length: u32,
    ) -> Result<(), Trap> {
        let data = caller.read_memory(offset, length)?.to_vec();
        Self::fn_impl(caller.data_mut(), &data);
        Ok(())
    }

    pub fn fn_impl<DB: IJournaledTrie>(ctx: &mut RuntimeContext<DB>, data: &[u8]) {
        ctx.output.extend_from_slice(data);
    }
}
