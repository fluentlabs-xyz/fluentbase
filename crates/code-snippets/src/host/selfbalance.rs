use crate::{
    common_sp::{sp_compute_mem_offset, sp_inc, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use core::slice;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

#[no_mangle]
pub fn host_selfbalance() {
    let sp = sp_inc(SP_BASE_MEM_OFFSET_DEFAULT, U256_BYTES_COUNT);
    let sp_mem_offset = sp_compute_mem_offset(SP_BASE_MEM_OFFSET_DEFAULT, sp);

    let out_balance =
        unsafe { slice::from_raw_parts_mut(sp_mem_offset as *mut u8, U256_BYTES_COUNT as usize) };
    LowLevelSDK::statedb_get_balance(&[], out_balance, true);
}
