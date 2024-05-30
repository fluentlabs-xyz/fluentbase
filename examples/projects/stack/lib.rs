#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code)]

extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

extern "C" {
    fn __get_stack_pointer() -> u32;
    fn __set_stack_pointer(sp: u32);
}

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
    unsafe {
        LowLevelSDK::sys_halt(__get_stack_pointer() as i32);
    }
}
