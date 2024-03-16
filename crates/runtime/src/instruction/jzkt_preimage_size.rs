use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{core::Trap, Caller};

pub struct JzktPreimageSize;

impl JzktPreimageSize {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        hash32_offset: u32,
    ) -> Result<u32, Trap> {
        let hash = caller.read_memory(hash32_offset, 32).to_vec();
        Self::fn_impl(caller.data_mut(), &hash).map_err(|err| err.into_trap())
    }

    pub fn fn_impl<T>(ctx: &mut RuntimeContext<T>, hash: &[u8]) -> Result<u32, ExitCode> {
        let jzkt = ctx.jzkt.clone().unwrap();
        let preimage_size = jzkt.borrow_mut().preimage_size(hash.try_into().unwrap());
        Ok(preimage_size)
    }
}
