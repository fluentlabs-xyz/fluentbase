use crate::{
    common_sp::{stack_peek_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use core::slice;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

#[no_mangle]
pub fn host_balance() {
    let address = stack_peek_u256(SP_BASE_MEM_OFFSET_DEFAULT, 0);

    let out_balance =
        unsafe { slice::from_raw_parts_mut(address.0 as *mut u8, U256_BYTES_COUNT as usize) };
    LowLevelSDK::statedb_get_balance(
        &address.1[U256_BYTES_COUNT as usize - 20..],
        out_balance,
        false,
    );
}
