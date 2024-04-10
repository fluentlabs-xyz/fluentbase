use crate::RuntimeContext;
use fluentbase_types::{ExitCode, IJournaledTrie};
use rwasm::{core::Trap, Caller};

pub struct JzktPreimageCopy;

impl JzktPreimageCopy {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        hash32_ptr: u32,
        preimage_ptr: u32,
    ) -> Result<(), Trap> {
        let hash = caller.read_memory(hash32_ptr, 32)?.to_vec();
        let preimage = Self::fn_impl(caller.data_mut(), &hash).map_err(|err| err.into_trap())?;
        caller.write_memory(preimage_ptr, &preimage)?;
        Ok(())
    }

    pub fn fn_impl<DB: IJournaledTrie>(
        ctx: &mut RuntimeContext<DB>,
        hash: &[u8],
    ) -> Result<Vec<u8>, ExitCode> {
        let jzkt = ctx.jzkt.as_mut().unwrap();
        let preimage = jzkt.preimage(hash.try_into().unwrap());
        Ok(preimage)
    }
}
