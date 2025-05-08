use crate::RuntimeContext;
use rwasm_executor::{Caller, RwasmError};

pub struct SyscallChargeFuel;

impl SyscallChargeFuel {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let fuel_consumed: u64 = caller.stack_pop_as();
        println!("ChargeFuel: fuel_consumed={}", fuel_consumed);
        caller.vm_mut().try_consume_fuel(fuel_consumed)?;
        Ok(())
    }

    pub fn fn_impl(ctx: &mut RuntimeContext, fuel_consumed: u64) {
        ctx.try_consume_fuel(fuel_consumed).unwrap();
    }
}
