use fluentbase_sdk::{ExecutionContext, LowLevelAPI, LowLevelSDK};

pub fn deploy() {}

pub fn main() {
    let input = ExecutionContext::raw_input();
    let mut output = [0u8; 32];
    LowLevelSDK::crypto_keccak256(input.as_ptr(), input.len() as u32, output.as_mut_ptr());
    LowLevelSDK::sys_write(&output);
}
