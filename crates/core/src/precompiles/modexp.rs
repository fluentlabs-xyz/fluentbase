use fluentbase_sdk::{ContextReader, ExecutionContext, LowLevelAPI, LowLevelSDK};
use fluentbase_types::{Bytes, ExitCode};
use revm_interpreter::primitives::PrecompileError;

pub fn deploy() {}

pub fn main() {
    let cr = ExecutionContext::DEFAULT;
    let input = ExecutionContext::raw_input();
    let gas_limit = cr.contract_gas_limit();

    let result = revm_precompile::modexp::berlin_run(&input, gas_limit);
    let result = match result {
        Ok((_, result)) => result,
        Err(err) => match err {
            PrecompileError::OutOfGas => {
                LowLevelSDK::sys_halt(ExitCode::OutOfFuel.into_i32());
                Bytes::new()
            }
            _ => {
                LowLevelSDK::sys_halt(ExitCode::PrecompileError.into_i32());
                Bytes::new()
            }
        },
    };
    LowLevelSDK::sys_write(result.as_ref());
}
