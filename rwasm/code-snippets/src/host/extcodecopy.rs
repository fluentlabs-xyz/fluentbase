use crate::{
    common::u256_be_to_u64tuple_le,
    common_sp::{stack_pop_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use core::slice;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

#[no_mangle]
pub fn host_extcodecopy() {
    let address = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let dest_offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let size = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let address20 = &address[U256_BYTES_COUNT as usize - 20..];

    let dest_offset = u256_be_to_u64tuple_le(dest_offset);
    let offset = u256_be_to_u64tuple_le(offset);
    let size = u256_be_to_u64tuple_le(size);

    // TODO validate inputs

    let dest_offset = dest_offset.0 as usize;
    let offset = offset.0 as usize;
    let size = size.0 as usize;

    let mut dest_data = unsafe { slice::from_raw_parts_mut(dest_offset as *mut u8, size) };

    let mut size_fact = LowLevelSDK::statedb_get_code_size(address20) as usize;
    if offset > 0 && size_fact > offset {
        size_fact -= offset;
    }
    LowLevelSDK::statedb_get_code(address20, dest_data, offset as u32);

    if size_fact < size {
        dest_data[size_fact..size].fill(0);
    }
}
