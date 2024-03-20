use fluentbase_sdk::{evm::ExecutionContext, LowLevelAPI, LowLevelSDK};

pub fn deploy() {}

pub fn main() {
    let input = ExecutionContext::contract_input();
    let gas_limit = ExecutionContext::contract_gas_limit();

    let pr = revm_precompile::blake2::run(&input, gas_limit);
    let res = pr.unwrap();
    // TODO use codec
    LowLevelSDK::sys_write(res.0.to_le_bytes().as_slice());
    LowLevelSDK::sys_write(res.1.as_ref());
}
