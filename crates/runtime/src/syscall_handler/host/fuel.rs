use crate::RuntimeContext;
use rwasm::{Store, TrapCode, Value};

/// Builtin to query remaining engine fuel.
pub struct SyscallFuel;

impl SyscallFuel {
    /// Writes the remaining fuel (or u64::MAX if metering is disabled) into result[0].
    pub fn fn_handler(
        caller: &mut impl Store<RuntimeContext>,
        _params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let fuel_remaining = caller.remaining_fuel().unwrap_or(u64::MAX);
        result[0] = Value::I64(fuel_remaining as i64);
        Ok(())
    }

    pub fn fn_impl(_ctx: &RuntimeContext) -> u64 {
        unimplemented!()
    }
}
