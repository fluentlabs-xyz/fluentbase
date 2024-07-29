use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{core::Trap, Caller};

pub struct SyscallPreimageSize;

impl SyscallPreimageSize {
    pub fn fn_handler(
        mut caller: Caller<'_, RuntimeContext>,
        hash32_offset: u32,
    ) -> Result<u32, Trap> {
        let hash = caller.read_memory(hash32_offset, 32)?.to_vec();
        Self::fn_impl(caller.data_mut(), &hash).map_err(|err| err.into_trap())
    }

    pub fn fn_impl(ctx: &RuntimeContext, hash: &[u8]) -> Result<u32, ExitCode> {
        let preimage_size = ctx.jzkt().borrow().preimage_size(hash.try_into().unwrap());
        Ok(preimage_size)
    }
}
