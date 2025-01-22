use crate::RuntimeContext;
use fluentbase_rwasm::{Caller, RwasmError};

pub struct SyscallWrite;

impl SyscallWrite {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let (offset, length) = caller.stack_pop2_as::<u32>();
        let data = caller.read_memory(offset, length)?.to_vec();
        Self::fn_impl(caller.data_mut(), &data);
        Ok(())
    }

    pub fn fn_impl(ctx: &mut RuntimeContext, data: &[u8]) {
        ctx.execution_result.output.extend_from_slice(data);
    }
}
