use crate::{
    common_sp::{stack_pop_u256, stack_push_u256, u256_zero, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

#[no_mangle]
pub fn host_extcodesize() {
    let address = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let address20 = &address[U256_BYTES_COUNT as usize - 20..];

    let size = LowLevelSDK::statedb_get_code_size(address20);

    let mut code_size = u256_zero();

    code_size[U256_BYTES_COUNT as usize - core::mem::size_of::<u32>()..]
        .copy_from_slice(&size.to_be_bytes());

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, code_size);
}
