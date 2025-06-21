use crate::RuntimeContext;
use core::cell::Ref;
use rwasm::{Caller, TrapCode, Value};

pub struct SyscallFuel;

impl SyscallFuel {
    pub fn fn_handler(
        caller: &mut dyn Caller<RuntimeContext>,
        _params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let fuel_remaining = caller.remaining_fuel().unwrap_or(u64::MAX);
        result[0] = Value::I64(fuel_remaining as i64);
        Ok(())
    }

    pub fn fn_impl(ctx: Ref<RuntimeContext>) -> u64 {
        ctx.remaining_fuel()
    }
}
