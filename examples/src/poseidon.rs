use fluentbase_sdk::{evm::ExecutionContext, LowLevelAPI, LowLevelSDK};

pub fn deploy() {}

pub fn main() {
    let input = ExecutionContext::contract_input();
    let mut output = [0u8; 32];
    LowLevelSDK::crypto_poseidon(input.as_ptr(), input.len() as u32, output.as_mut_ptr());
    let ctx = ExecutionContext::default();
    ctx.fast_return_and_exit(output, 0);
}
