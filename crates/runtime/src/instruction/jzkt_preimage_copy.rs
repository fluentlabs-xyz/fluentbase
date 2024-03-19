use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{core::Trap, Caller};

pub struct JzktPreimageCopy;

impl JzktPreimageCopy {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        hash32_ptr: u32,
        preimage_ptr: u32,
    ) -> Result<(), Trap> {
        let hash = caller.read_memory(hash32_ptr, 32)?.to_vec();
        let preimage = Self::fn_impl(caller.data_mut(), &hash).map_err(|err| err.into_trap())?;
        caller.write_memory(preimage_ptr, &preimage)?;
        Ok(())
    }

    pub fn fn_impl<T>(ctx: &mut RuntimeContext<T>, hash: &[u8]) -> Result<Vec<u8>, ExitCode> {
        let jzkt = ctx.jzkt.clone().unwrap();
        let preimage = jzkt.borrow_mut().preimage(hash.try_into().unwrap());
        Ok(preimage)
    }
}
