use crate::RuntimeContext;
use rwasm::{core::Trap, Caller};

pub struct SyscallFuel;

impl SyscallFuel {
    pub fn fn_handler(caller: Caller<'_, RuntimeContext>) -> Result<u64, Trap> {
        Ok(caller.fuel_remaining().unwrap_or(u64::MAX))
    }

    pub fn fn_impl(_ctx: &RuntimeContext) -> u64 {
        // TODO(dmitry123): "we can't get fuel in runtime context, what to do?"
        u64::MAX
    }
}
