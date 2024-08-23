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
                    let remaining = Self::fn_impl(caller.data_mut(), delta);
                    if remaining == u64::MAX {
                        println!("gas_record_cost(err): delta={} err=OutOfGas", delta);
                        return Err(ExitCode::OutOfGas.into_trap());
                    }
                    println!(
                        "gas_record_cost(ok): remaining={}, delta={}",
                        remaining, delta,
                    );
                    Ok(remaining)
                }
                FuelError::OutOfFuel => {
                    println!("gas_record_cost(err): delta={} err=OutOfGas", delta);
                    Err(ExitCode::OutOfGas.into_trap())
                }
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
