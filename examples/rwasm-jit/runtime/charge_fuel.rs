use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{core::Trap, errors::FuelError, Caller};

pub struct SyscallChargeFuel;

impl SyscallChargeFuel {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>, delta: u64) -> Result<u64, Trap> {
        match caller.consume_fuel(delta) {
            Ok(remaining) => {
                println!(
                    "gas_record_cost(ok): remaining={}, delta={}",
                    remaining, delta,
                );
                Ok(remaining)
            }
            Err(err) => match err {
                FuelError::FuelMeteringDisabled => {
                    // TODO(dmitry123): "how safe to return u64::MAX for fuel disabled?"
                    Ok(u64::MAX)
                }
                FuelError::OutOfFuel => {
                    println!("gas_record_cost(err): delta={} err=OutOfGas", delta);
                    Err(ExitCode::OutOfGas.into_trap())
                }
            },
        }
    }

    pub fn fn_impl(_ctx: &mut RuntimeContext, _delta: u64) -> u64 {
        // TODO(dmitry123): "we can't charge fuel in runtime context, what to do?"
        u64::MAX
    }
}
