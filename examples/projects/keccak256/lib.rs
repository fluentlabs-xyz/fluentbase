#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code)]

extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{ContextReader, ExecutionContext, LowLevelAPI, LowLevelSDK};

// Function to deploy the contract
#[cfg(not(feature = "std"))]
#[no_mangle]
#[cfg(target_arch = "wasm32")]
pub extern "C" fn deploy() {}

// Main function
#[cfg(not(feature = "std"))]
#[no_mangle]
#[cfg(target_arch = "wasm32")]
pub extern "C" fn main() {
    let input = ExecutionContext::DEFAULT.contract_input();
    let mut output = [0u8; 32];
    LowLevelSDK::crypto_keccak256(input.as_ptr(), input.len() as u32, output.as_mut_ptr());
    LowLevelSDK::sys_write(&output);
}
