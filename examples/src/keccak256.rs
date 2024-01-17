use crate::deploy_internal;
use fluentbase_sdk::{evm::ExecutionContext, LowLevelAPI, LowLevelSDK};

pub fn deploy() {
    deploy_internal(include_bytes!("../bin/keccak256.wasm"))
}

pub fn main() {
    let input = ExecutionContext::contract_input();
    let mut output = [0u8; 32];
    LowLevelSDK::crypto_keccak256(&input, &mut output);
    let mut ctx = ExecutionContext::default();
    ctx.fast_return_and_exit(output, 0);
}
