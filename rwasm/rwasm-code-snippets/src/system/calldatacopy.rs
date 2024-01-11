use crate::{
    common::{u256_be_to_tuple_le, u256_from_be_slice_align_left},
    common_sp::{stack_pop_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use core::slice;
use fluentbase_sdk::evm::ExecutionContext;

#[no_mangle]
fn system_calldatacopy() {
    let size = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let mut size = u256_be_to_tuple_le(size).0 as usize;
    let offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let mut offset = u256_be_to_tuple_le(offset).0 as usize;
    let dest_offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let mut dest_offset = u256_be_to_tuple_le(dest_offset).0 as usize;

    let ci = ExecutionContext::contract_input();

    let mut shift = 0;

    while size > 0 {
        let data_size = if size >= U256_BYTES_COUNT as usize {
            U256_BYTES_COUNT as usize
        } else {
            size
        };

        let offset = offset + shift;
        let dest_offset = dest_offset + shift;

        let dest =
            unsafe { slice::from_raw_parts_mut(dest_offset as *mut u8, U256_BYTES_COUNT as usize) };

        let v = if offset < ci.len() {
            if offset + data_size < ci.len() {
                &ci[offset..offset + data_size]
            } else {
                &ci[offset..ci.len()]
            }
        } else {
            &[]
        };
        let v = &u256_from_be_slice_align_left(v);

        dest.copy_from_slice(v);

        shift += data_size;
        size -= data_size;
    }
}
