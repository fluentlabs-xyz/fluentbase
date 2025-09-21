use crate::RuntimeContext;
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub struct SyscallChargeFuel;

impl SyscallChargeFuel {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let fuel_consumed = params[0].i64().unwrap() as u64;
        // Here we have two fuel counters, one for instructions & builtins
        caller.try_consume_fuel(fuel_consumed)?;
        // And the second counter for builtins only (like this function call),
        // it's required to support gas model where it's managed by runtime
        caller.context_mut(|ctx| Self::fn_impl(ctx, fuel_consumed))?;
        Ok(())
    }

    pub fn fn_impl(ctx: &mut RuntimeContext, fuel_consumed: u64) -> Result<(), TrapCode> {
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
}
