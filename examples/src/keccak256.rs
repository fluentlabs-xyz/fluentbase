use fluentbase_sdk::{ExecutionContext, LowLevelSDK, SharedAPI};

pub fn deploy() {}

pub fn main() {
    let input = ExecutionContext::raw_input();
    let mut output = [0u8; 32];
    LowLevelSDK::keccak256(input.as_ptr(), input.len() as u32, output.as_mut_ptr());
    LowLevelSDK::write(&output);
}
