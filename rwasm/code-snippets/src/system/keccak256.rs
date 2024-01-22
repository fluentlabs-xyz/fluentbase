use crate::{
    common::u256_be_to_tuple_le,
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use core::slice;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

#[no_mangle]
fn system_keccak256() {
    let offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let size = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let offset = u256_be_to_tuple_le(offset);
    let size = u256_be_to_tuple_le(size);
    let data = unsafe { slice::from_raw_parts(offset.0 as *const u8, size.0 as usize) };

    let mut res = [0u8; U256_BYTES_COUNT as usize];
    LowLevelSDK::crypto_keccak256(data, &mut res);
    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, res);
}
