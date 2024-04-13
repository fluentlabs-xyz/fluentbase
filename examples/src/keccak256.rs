use fluentbase_sdk::evm::ExecutionContext;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

pub fn deploy() {}

pub fn main() {
    let input = ExecutionContext::contract_input();
    let mut output = [0u8; 32];
    LowLevelSDK::crypto_keccak256(input.as_ptr(), input.len() as u32, output.as_mut_ptr());
    LowLevelSDK::sys_write(&output);
}
