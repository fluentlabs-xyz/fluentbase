use crate::{
    common_sp::{stack_peek_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use core::slice;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

#[no_mangle]
pub fn host_extcodehash() {
    let address = stack_peek_u256(SP_BASE_MEM_OFFSET_DEFAULT, 0);

    let address20 = &address.1[U256_BYTES_COUNT as usize - 20..];

    let out_hash_data = unsafe { slice::from_raw_parts_mut(address.0 as *mut u8, 32) };

    LowLevelSDK::statedb_get_code_hash(address20, out_hash_data);
}
