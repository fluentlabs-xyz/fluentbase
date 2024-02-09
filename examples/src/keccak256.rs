use crate::deploy_internal;
use core::{alloc::Layout, ptr};
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

pub fn deploy() {
    deploy_internal(include_bytes!("../bin/keccak256.wasm"))
}

pub fn main() {
    let input_size = LowLevelSDK::sys_input_size();
    let buffer = unsafe {
        let ptr = alloc::alloc::alloc(Layout::from_size_align_unchecked(
            input_size as usize,
            8usize,
        ));
        &mut *ptr::slice_from_raw_parts_mut(ptr, input_size as usize)
    };
    LowLevelSDK::sys_read(buffer, 0);
    let mut output = [0u8; 32];
    LowLevelSDK::crypto_keccak256(buffer.as_ptr(), input_size, output.as_mut_ptr());
    LowLevelSDK::sys_write(&output);
}
