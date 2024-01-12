use crate::{
    common::u256_be_to_tuple_le,
    common_sp::{stack_pop_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use core::slice;

#[no_mangle]
fn memory_mstore() {
    let offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let value = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let offset = u256_be_to_tuple_le(offset);

    let mem_chunk =
        unsafe { slice::from_raw_parts_mut(offset.0 as *mut u8, U256_BYTES_COUNT as usize) };
    mem_chunk.copy_from_slice(&value);
}
