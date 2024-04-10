use crate::RuntimeContext;
use fluentbase_types::{ExitCode, IJournaledTrie};
use rwasm::{core::Trap, Caller};

pub struct JzktPreimageSize;

impl JzktPreimageSize {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        hash32_offset: u32,
    ) -> Result<u32, Trap> {
        let hash = caller.read_memory(hash32_offset, 32)?.to_vec();
        Self::fn_impl(caller.data_mut(), &hash).map_err(|err| err.into_trap())
    }

    pub fn fn_impl<DB: IJournaledTrie>(
        ctx: &mut RuntimeContext<DB>,
        hash: &[u8],
    ) -> Result<u32, ExitCode> {
        let jzkt = ctx.jzkt.as_mut().unwrap();
        let preimage_size = jzkt.preimage_size(hash.try_into().unwrap());
        Ok(preimage_size)
    }
}
