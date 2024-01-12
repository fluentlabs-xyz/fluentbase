use crate::{
    common::u256_from_be_slice,
    common_sp::{stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
};
use core::slice;

#[cfg(target_family = "wasm")]
#[no_mangle]
fn memory_msize() {
    let mem_size = core::arch::wasm32::memory_size(0);
    let mem_size = mem_size.to_be_bytes();

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, u256_from_be_slice(&mem_size));
}
