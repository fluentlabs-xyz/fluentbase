use crate::RuntimeContext;
use rwasm::{core::Trap, Caller};

pub struct SyscallFuel;

impl SyscallFuel {
    pub fn fn_handler(caller: Caller<'_, RuntimeContext>) -> Result<u64, Trap> {
        Ok(Self::fn_impl(caller.data()))
        // Ok(caller.fuel_consumed().unwrap_or(u64::MAX))
    }

    pub fn fn_impl(ctx: &RuntimeContext) -> u64 {
        // println!("fuel remaining={}", ctx.fuel().remaining());
        ctx.fuel().remaining()
    }
}
