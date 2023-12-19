use crate::{
    common::u256_be_to_tuple_le,
    common_sp::{u256_pop, SP_VAL_MEM_OFFSET_DEFAULT},
};
use core::slice;

#[no_mangle]
fn memory_mstore8() {
    let value = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    let offset = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);

    let offset = u256_be_to_tuple_le(offset);

    let mem_chunk = offset.0 as *mut u8;
    unsafe { *mem_chunk = value[value.len() - 1] };
}
