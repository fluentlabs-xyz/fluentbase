use fluentbase_sdk::{evm::ExecutionContext, LowLevelAPI, LowLevelSDK};
use fluentbase_types::ExitCode;
use revm_interpreter::primitives::PrecompileError;

pub fn deploy() {}

pub fn main() {
    let input = ExecutionContext::contract_input();
    let gas_limit = ExecutionContext::contract_gas_limit();

    let result = revm_precompile::modexp::berlin_run(&input, gas_limit);
    let result = match result {
        Ok((_, result)) => result,
        Err(err) => match err {
            PrecompileError::OutOfGas => {
                LowLevelSDK::sys_halt(ExitCode::OutOfFuel.into_i32());
            }
            _ => {
                LowLevelSDK::sys_halt(ExitCode::PrecompileError.into_i32());
            }
        },
    };
    LowLevelSDK::sys_write(result.as_ref());
}
