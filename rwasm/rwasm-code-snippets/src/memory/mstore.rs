use crate::{
    common::u256_be_to_tuple_le,
    common_sp::{u256_pop, SP_VAL_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use core::slice;

#[no_mangle]
fn memory_mstore() {
    let value = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    let offset = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);

    let offset = u256_be_to_tuple_le(offset);

    let mem_chunk =
        unsafe { slice::from_raw_parts_mut(offset.0 as *mut u8, U256_BYTES_COUNT as usize) };
    mem_chunk.copy_from_slice(&value);
}
