use crate::RuntimeContext;
use fluentbase_rwasm::{Caller, RwasmError};

pub struct SyscallChargeFuel;

impl SyscallChargeFuel {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let delta: u64 = caller.stack_pop_as();
        caller.store_mut().try_consume_fuel(delta)?;
        let remaining_fuel = caller.store().remaining_fuel().unwrap_or(u64::MAX);
        caller.stack_push(remaining_fuel);
        Ok(())
    }

    pub fn fn_impl(_ctx: &mut RuntimeContext, _delta: u64) -> u64 {
        // TODO(dmitry123): "we can't charge fuel in the runtime context, what to do?"
        u64::MAX
    }
}
