#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

#[cfg(target_arch = "wasm32")]
#[no_mangle]
extern "C" fn main() {
    use fluentbase_sdk::{alloc_ptr, LowLevelSDK, SharedAPI};
    let input_size = LowLevelSDK::input_size() as usize;
    let buffer = alloc_ptr(input_size);
    LowLevelSDK::read(buffer, input_size as u32, 0);
    LowLevelSDK::write(buffer, input_size as u32);
}
