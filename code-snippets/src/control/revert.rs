use crate::{
    common::u256_be_to_u64tuple_le,
    common_sp::{stack_pop_u256, SP_BASE_MEM_OFFSET_DEFAULT},
};
use core::slice;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

#[no_mangle]
pub fn control_revert() {
    let offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let size = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let offset = u256_be_to_u64tuple_le(offset).0;
    let size = u256_be_to_u64tuple_le(size).0;

    let mem_chunk = unsafe { slice::from_raw_parts(offset as *const u8, size as usize) };
    LowLevelSDK::sys_write(mem_chunk);
    LowLevelSDK::sys_halt(0);
}
