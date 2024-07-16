#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

#[cfg(target_arch = "wasm32")]
#[no_mangle]
extern "C" fn main() {
    use fluentbase_sdk::{alloc_slice, rwasm::RwasmContext, SharedAPI};
    let sdk = RwasmContext::default();
    let input_size = sdk.input_size();
    let buffer = alloc_slice(input_size as usize);
    sdk.read(buffer, 0);
    sdk.write(buffer);
}
