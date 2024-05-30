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
    // Create a default execution context
    let ctx = ExecutionContext::default();
    // Get the contract input
    let input = ctx.contract_input().clone();
    let mut output = [0u8; 32];
    LowLevelSDK::crypto_poseidon(input.as_ptr(), input.len() as u32, output.as_mut_ptr());
    let ctx = ExecutionContext::default();
    ctx.fast_return_and_exit(output, 0);
}
