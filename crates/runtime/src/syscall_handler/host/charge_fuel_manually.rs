/// Builtin to manually charge and refund fuel when VM metering is disabled.
///
/// TODO(dmitry123): Remove this method for the finalized builtin scheme.
use crate::RuntimeContext;
use rwasm::{StoreTr, TrapCode, Value};

/// Validates that fuel metering is disabled, applies manual charge/refund, and returns remaining fuel.
pub fn syscall_charge_fuel_manually_handler(
    caller: &mut impl StoreTr<RuntimeContext>,
    params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    let (fuel_consumed, fuel_refunded) =
        (params[0].i64().unwrap() as u64, params[1].i64().unwrap());
    caller.try_consume_fuel(fuel_consumed)?;
    syscall_charge_fuel_manually_impl(caller.data_mut(), fuel_consumed, fuel_refunded)?;
    let remaining_fuel = caller.remaining_fuel().unwrap_or(u64::MAX);
    result[0] = Value::I64(remaining_fuel as i64);
    Ok(())
}

/// Updates context fuel accounting with manual consumption and refund values.
pub fn syscall_charge_fuel_manually_impl(
    ctx: &mut RuntimeContext,
    fuel_consumed: u64,
    fuel_refunded: i64,
) -> Result<u64, TrapCode> {
    let new_fuel_consumed = ctx
        .execution_result
        .fuel_consumed
        .saturating_add(fuel_consumed);
    if new_fuel_consumed > ctx.fuel_limit {
        return Err(TrapCode::OutOfFuel);
    }
    ctx.execution_result.fuel_consumed = new_fuel_consumed;
    ctx.execution_result.fuel_refunded += fuel_refunded;
    Ok(0)
}
