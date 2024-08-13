use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{core::Trap, errors::FuelError, Caller};

pub struct SyscallFuel;

impl SyscallFuel {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<u64, Trap> {
        Ok(caller.fuel_consumed().unwrap_or(u64::MAX))
    }

    pub fn fn_impl(_ctx: &mut RuntimeContext, _delta: u64) -> u64 {
        // TODO: "we can't charge fuel anyhow in this mode, safer just to skip it for now"
        u64::MAX
    }
}
