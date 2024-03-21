use fluentbase_sdk::{evm::ExecutionContext, LowLevelAPI, LowLevelSDK};

pub fn deploy() {}

pub fn main() {
    let input = ExecutionContext::contract_input();
    let gas_limit = ExecutionContext::contract_gas_limit();

    let pr = revm_precompile::modexp::berlin_run(&input, gas_limit);
    let (_, res) = pr.unwrap();
    LowLevelSDK::sys_write(res.as_ref());
}
