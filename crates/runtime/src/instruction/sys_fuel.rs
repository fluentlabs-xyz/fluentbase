use crate::RuntimeContext;
use fluentbase_types::{ExitCode, IJournaledTrie};
use rwasm::{core::Trap, errors::FuelError, Caller};

pub struct SysFuel;

impl SysFuel {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        delta: u64,
    ) -> Result<u64, Trap> {
        match caller.consume_fuel(delta) {
            Ok(remaining) => return Ok(remaining),
            Err(err) => match err {
                FuelError::FuelMeteringDisabled => return Ok(u64::MAX),
                FuelError::OutOfFuel => Err(ExitCode::OutOfFuel.into_trap()),
            },
        }
    }

    pub fn fn_impl<DB: IJournaledTrie>(_ctx: &mut RuntimeContext<DB>, _delta: u64) -> u64 {
        // TODO: "we can't charge fuel anyhow in this mode, safer just to skip it for now"
        u64::MAX
    }
}
