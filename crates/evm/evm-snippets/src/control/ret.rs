use crate::{
    common::u256_be_to_u64tuple_le,
    common_sp::{stack_pop_u256, SP_BASE_MEM_OFFSET_DEFAULT},
};
use fluentbase_sdk::{LowLevelSDK, SharedAPI};

#[no_mangle]
pub fn control_return() {
    let offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let size = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let offset = u256_be_to_u64tuple_le(offset).0;
    let size = u256_be_to_u64tuple_le(size).0;

    LowLevelSDK::write(offset as *const u8, size as u32);
}
