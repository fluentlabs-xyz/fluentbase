use crate::RuntimeContext;
use rwasm::{core::Trap, Caller};

pub struct SyscallWrite;

impl SyscallWrite {
    pub fn fn_handler(
        mut caller: Caller<'_, RuntimeContext>,
        offset: u32,
        length: u32,
    ) -> Result<(), Trap> {
        let data = caller.read_memory(offset, length)?.to_vec();
        Self::fn_impl(caller.data_mut(), &data);
        Ok(())
    }

    pub fn fn_impl(ctx: &mut RuntimeContext, data: &[u8]) {
        ctx.execution_result.output.extend_from_slice(data);
    }
}
