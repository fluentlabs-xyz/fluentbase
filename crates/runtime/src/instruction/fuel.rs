use crate::RuntimeContext;
use rwasm::{core::Trap, Caller};

pub struct SyscallFuel;

impl SyscallFuel {
    pub fn fn_handler(caller: Caller<'_, RuntimeContext>) -> Result<u64, Trap> {
        Ok(caller.fuel_consumed().unwrap_or(u64::MAX))
    }

    pub fn fn_impl(ctx: &RuntimeContext) -> u64 {
        ctx.fuel().remaining()
    }
}
