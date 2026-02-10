/// Builtin that charges fuel against the VM and the runtime accounting.
use crate::RuntimeContext;
use rwasm::{Store, TrapCode, Value};

/// Consumes fuel from the engine and mirrors the consumption in the runtime context.
pub fn syscall_charge_fuel_handler(
    caller: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let fuel_consumed = params[0].i64().unwrap() as u64;
    // Charge the engine fuel counter (instructions + builtins).
    caller.try_consume_fuel(fuel_consumed)?;
    // Mirror the charge in the runtime context (builtins-managed accounting).
    syscall_charge_fuel_impl(caller.data_mut(), fuel_consumed)?;
    Ok(())
}

/// Adds the consumed fuel to the runtime context and checks against the context fuel limit.
pub fn syscall_charge_fuel_impl(
    ctx: &mut RuntimeContext,
    fuel_consumed: u64,
) -> Result<(), TrapCode> {
    let new_fuel_consumed = ctx
        .execution_result
        .fuel_consumed
        .checked_add(fuel_consumed)
        .unwrap_or(u64::MAX);
    if new_fuel_consumed > ctx.fuel_limit {
        return Err(TrapCode::OutOfFuel);
    }
    ctx.execution_result.fuel_consumed = new_fuel_consumed;
    Ok(())
}
