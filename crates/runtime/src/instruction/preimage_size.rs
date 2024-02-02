use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{common::Trap, Caller};

pub struct PreimageSize;

impl PreimageSize {
    pub fn fn_handler<T>(
        caller: Caller<'_, RuntimeContext<T>>,
        hash32_offset: u32,
    ) -> Result<u32, Trap> {
        let hash = caller.read_memory(hash32_offset, 32).to_vec();
        Self::fn_impl(caller.data(), &hash).map_err(|err| err.into_trap())
    }

    pub fn fn_impl<T>(ctx: &RuntimeContext<T>, hash: &[u8]) -> Result<u32, ExitCode> {
        let preimage_db = ctx
            .preimage_db
            .as_ref()
            .ok_or(ExitCode::PreimageUnavailable)?;
        let preimage_size = preimage_db.borrow().preimage_size(hash);
        Ok(preimage_size)
    }
}
