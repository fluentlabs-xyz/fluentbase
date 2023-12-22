use crate::{
    common::u256_be_to_tuple_le,
    common_sp::{stack_pop_u256, SP_BASE_MEM_OFFSET_DEFAULT},
};

#[no_mangle]
fn memory_mstore8() {
    let value = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let offset = u256_be_to_tuple_le(offset);

    let mem_chunk = offset.0 as *mut u8;
    unsafe { *mem_chunk = value[value.len() - 1] };
}
