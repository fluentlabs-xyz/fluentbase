use crate::{
    common::{u256_be_to_u64tuple_le, u256_from_be_slice},
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use core::slice;

#[no_mangle]
fn memory_mload() {
    let offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let offset = u256_be_to_u64tuple_le(offset);

    let mem_chunk =
        unsafe { slice::from_raw_parts_mut(offset.0 as *mut u8, U256_BYTES_COUNT as usize) };
    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, u256_from_be_slice(&mem_chunk));
}
