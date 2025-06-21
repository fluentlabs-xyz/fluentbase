use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{Caller, TrapCode, Value};
use std::cell::RefMut;

// TODO(dmitry123): "replace with intrinsics once it's implemented in rwasm"
// TODO(dmitry123): "how to support disable fuel condition then?"
pub struct SyscallChargeFuelManually;

impl SyscallChargeFuelManually {
    pub fn fn_handler(
        caller: &mut dyn Caller<RuntimeContext>,
        params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        // this method is allowed only in manual fuel mode that is possible with disabled fuel
        if !caller.context().disable_fuel {
            caller.context_mut().execution_result.exit_code =
                ExitCode::MalformedBuiltinParams.into();
            return Err(TrapCode::ExecutionHalted);
        }
        let (fuel_refunded, fuel_consumed) =
            (params[0].i64().unwrap(), params[1].i64().unwrap() as u64);
        caller.try_consume_fuel(fuel_consumed)?;
        Self::fn_impl(caller.context_mut(), fuel_consumed, fuel_refunded);
        let remaining_fuel = caller.remaining_fuel().unwrap_or(u64::MAX);
        result[0] = Value::I64(remaining_fuel as i64);
        Ok(())
    }

    pub fn fn_impl(mut ctx: RefMut<RuntimeContext>, fuel_consumed: u64, fuel_refunded: i64) -> u64 {
        // TODO(dmitry123): "how to sync fuel between caller and context? do we need to sync it?"
        ctx.try_consume_fuel(fuel_consumed).unwrap();
        ctx.refund_fuel(fuel_refunded);
        ctx.remaining_fuel()
    }
}
