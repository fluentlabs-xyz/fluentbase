use crate::{
    common::u256_be_to_tuple_le,
    common_sp::{u256_pop, u256_push, SP_VAL_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use core::slice;
use fluentbase_sdk::{CryptoPlatformSDK, SDK};

#[no_mangle]
fn system_keccak() {
    let size = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    let offset = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);

    let offset = u256_be_to_tuple_le(offset);
    let size = u256_be_to_tuple_le(size);
    let data = unsafe { slice::from_raw_parts(offset.0 as *const u8, size.0 as usize) };

    let mut res = [0u8; U256_BYTES_COUNT as usize];
    SDK::crypto_keccak256(data, &mut res);
    u256_push(SP_VAL_MEM_OFFSET_DEFAULT, res);
}
