use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{core::Trap, errors::FuelError, Caller};

pub struct SyscallChargeFuel;

impl SyscallChargeFuel {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>, delta: u64) -> Result<u64, Trap> {
        match caller.consume_fuel(delta) {
            Ok(remaining) => Ok(remaining),
            Err(err) => match err {
                FuelError::FuelMeteringDisabled => Ok(u64::MAX),
                FuelError::OutOfFuel => Err(ExitCode::OutOfGas.into_trap()),
            },
        }
    }

    pub fn fn_impl(ctx: &mut RuntimeContext, delta: u64) -> u64 {
        if !ctx.fuel_mut().charge(delta) {
            return u64::MAX;
        }
        ctx.fuel().remaining()
    }
}
