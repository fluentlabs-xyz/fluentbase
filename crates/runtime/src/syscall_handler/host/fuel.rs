/// Builtin to query remaining engine fuel.
use crate::RuntimeContext;
use rwasm::{StoreTr, TrapCode, Value};

/// Writes the remaining fuel (or `u64::MAX` if metering is disabled) into `result[0]`.
pub fn syscall_fuel_handler(
    caller: &mut impl StoreTr<RuntimeContext>,
    _params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    let fuel_remaining = caller.remaining_fuel().unwrap_or(u64::MAX);
    result[0] = Value::I64(fuel_remaining as i64);
    Ok(())
}

pub fn syscall_fuel_impl(_ctx: &RuntimeContext) -> u64 {
    unimplemented!()
}
