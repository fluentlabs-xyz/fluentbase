use crate::RuntimeContext;
use rwasm::{Caller, RwasmError};

pub struct SyscallChargeFuelManually;

impl SyscallChargeFuelManually {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        // this method is allowed only in manual fuel mode that is possible with disabled fuel
        if !caller.context().disable_fuel {
            return Err(RwasmError::NotAllowedInFuelMode);
        }
        let fuel_refunded: i64 = caller.stack_pop_as();
        let fuel_consumed: u64 = caller.stack_pop_as();
        caller.vm_mut().try_consume_fuel(fuel_consumed)?;
        caller.vm_mut().refund_fuel(fuel_refunded);
        let remaining_fuel = caller.vm().remaining_fuel().unwrap_or(u64::MAX);
        caller.stack_push(remaining_fuel);
        Ok(())
    }

    pub fn fn_impl(ctx: &mut RuntimeContext, fuel_consumed: u64, fuel_refunded: i64) -> u64 {
        ctx.try_consume_fuel(fuel_consumed).unwrap();
        ctx.refund_fuel(fuel_refunded);
        ctx.remaining_fuel()
    }
}
