use crate::{
    common::u256_from_be_slice,
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use fluentbase_sdk::{EvmPlatformSDK, SDK};

#[no_mangle]
pub fn host_sload() {
    let k = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let mut v = [0; U256_BYTES_COUNT as usize];
    SDK::evm_sload(&k, &mut v);

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, u256_from_be_slice(&v));
}
