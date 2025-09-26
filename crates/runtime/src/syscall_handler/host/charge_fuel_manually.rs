use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{Store, TrapCode, Value};

/// Builtin to manually charge and refund fuel when VM metering is disabled.
pub struct SyscallChargeFuelManually;

impl SyscallChargeFuelManually {
    /// Validates that fuel metering is disabled, applies manual charge/refund, and returns remaining fuel.
    pub fn fn_handler(
        caller: &mut impl Store<RuntimeContext>,
        params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        // Only allowed when engine metering is disabled (manual fuel mode).
        caller.context_mut(|ctx| {
            if ctx.is_fuel_disabled() {
                return Ok(());
            }
            ctx.execution_result.exit_code = ExitCode::MalformedBuiltinParams.into_i32();
            Err(TrapCode::ExecutionHalted)
        })?;
        let (fuel_consumed, fuel_refunded) =
            (params[0].i64().unwrap() as u64, params[1].i64().unwrap());
        caller.try_consume_fuel(fuel_consumed)?;
        caller.context_mut(|ctx| Self::fn_impl(ctx, fuel_consumed, fuel_refunded))?;
        let remaining_fuel = caller.remaining_fuel().unwrap_or(u64::MAX);
        result[0] = Value::I64(remaining_fuel as i64);
        Ok(())
    }

    /// Updates context fuel accounting with manual consumption and refund values.
    pub fn fn_impl(
        ctx: &mut RuntimeContext,
        fuel_consumed: u64,
        fuel_refunded: i64,
    ) -> Result<u64, TrapCode> {
        let new_fuel_consumed = ctx
            .execution_result
            .fuel_consumed
            .checked_add(fuel_consumed)
            .unwrap_or(u64::MAX);
        if new_fuel_consumed > ctx.fuel_limit {
            return Err(TrapCode::OutOfFuel);
        }
        ctx.execution_result.fuel_consumed = new_fuel_consumed;
        ctx.execution_result.fuel_refunded += fuel_refunded;
        Ok(0)
    }
}
