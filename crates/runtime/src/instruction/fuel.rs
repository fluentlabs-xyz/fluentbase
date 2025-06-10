use crate::RuntimeContext;
use rwasm::{Caller, TrapCode};

pub struct SyscallFuel;

impl SyscallFuel {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), TrapCode> {
        let fuel_remaining = caller.store().remaining_fuel().unwrap_or(u64::MAX);
        caller.stack_push(fuel_remaining);
        Ok(())
    }

    pub fn fn_impl(ctx: &RuntimeContext) -> u64 {
        ctx.remaining_fuel()
    }
}
