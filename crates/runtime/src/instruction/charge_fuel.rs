use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{core::Trap, errors::FuelError, Caller};

pub struct SyscallChargeFuel;

impl SyscallChargeFuel {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>, delta: u64) -> Result<u64, Trap> {
        match caller.consume_fuel(delta) {
            Ok(remaining) => return Ok(remaining),
            Err(err) => match err {
                FuelError::FuelMeteringDisabled => return Ok(u64::MAX),
                FuelError::OutOfFuel => Err(ExitCode::OutOfGas.into_trap()),
            },
        }
    }

    pub fn fn_impl(_ctx: &mut RuntimeContext, _delta: u64) -> u64 {
        // TODO: "we can't charge fuel anyhow in this mode, safer just to skip it for now"
        u64::MAX
    }
}
