use crate::{
    common::{u256_be_to_tuple_le, u256_from_be_slice},
    common_sp::{stack_pop_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use core::slice;
use fluentbase_sdk::evm::ExecutionContext;

#[no_mangle]
fn system_calldatacopy() {
    let size = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let size = u256_be_to_tuple_le(size).0;
    let offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let offset = u256_be_to_tuple_le(offset).0;
    let dest_offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let dest_offset = u256_be_to_tuple_le(dest_offset).0;

    let ci = ExecutionContext::contract_input();
    let ci_ptr = ci.as_ptr();

    let dest =
        unsafe { slice::from_raw_parts_mut(dest_offset as *mut u8, U256_BYTES_COUNT as usize) };

    let v = if offset < ci.len() as u64 {
        if offset + size < ci.len() as u64 {
            unsafe { slice::from_raw_parts(ci_ptr, U256_BYTES_COUNT as usize) }
        } else {
            unsafe { slice::from_raw_parts(ci_ptr, ci.len() - size as usize) }
        }
    } else {
        &[]
    };
    let v = &u256_from_be_slice(v);

    dest.copy_from_slice(v);
}
