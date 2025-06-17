use crate::RuntimeContext;
use rwasm::{Caller, TrapCode};

pub struct SyscallChargeFuel;

impl SyscallChargeFuel {
    pub fn fn_handler(mut caller: Caller<RuntimeContext>) -> Result<(), TrapCode> {
        let fuel_consumed: u64 = caller.stack_pop_u64();
        caller.store_mut().try_consume_fuel(fuel_consumed)?;
        Ok(())
    }

    pub fn fn_impl(ctx: &mut RuntimeContext, fuel_consumed: u64) {
        ctx.try_consume_fuel(fuel_consumed).unwrap();
    }
}
