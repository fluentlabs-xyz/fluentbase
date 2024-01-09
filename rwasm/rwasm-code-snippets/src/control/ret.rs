use crate::{
    common::{u256_be_to_tuple_le, u256_from_be_slice},
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use core::slice;
use fluentbase_sdk::{SysPlatformSDK, SDK};

#[no_mangle]
pub fn control_return() {
    let size = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let size = u256_be_to_tuple_le(size).0;
    let offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let offset = u256_be_to_tuple_le(offset).0;

    let mem_chunk = unsafe { slice::from_raw_parts(offset as *const u8, size as usize) };
    SDK::sys_write(mem_chunk);
}
