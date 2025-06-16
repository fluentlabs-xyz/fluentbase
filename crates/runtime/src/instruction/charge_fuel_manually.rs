use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{Caller, TrapCode};

// TODO(dmitry123): "replace with intrinsics once it's implemented in rwasm"
pub struct SyscallChargeFuelManually;

impl SyscallChargeFuelManually {
    pub fn fn_handler(mut caller: Caller<RuntimeContext>) -> Result<(), TrapCode> {
        // this method is allowed only in manual fuel mode that is possible with disabled fuel
        if !caller.context().disable_fuel {
            caller.context_mut().execution_result.exit_code =
                ExitCode::MalformedBuiltinParams.into();
            return Err(TrapCode::ExecutionHalted);
        }
        let fuel_refunded: i64 = caller.stack_pop_as();
        let fuel_consumed: u64 = caller.stack_pop_as();
        caller.store_mut().try_consume_fuel(fuel_consumed)?;
        caller.store_mut().refund_fuel(fuel_refunded);
        let remaining_fuel = caller.store().remaining_fuel().unwrap_or(u64::MAX);
        caller.stack_push(remaining_fuel);
        Ok(())
    }

    pub fn fn_impl(ctx: &mut RuntimeContext, fuel_consumed: u64, fuel_refunded: i64) -> u64 {
        ctx.try_consume_fuel(fuel_consumed).unwrap();
        ctx.refund_fuel(fuel_refunded);
        ctx.remaining_fuel()
    }
}
