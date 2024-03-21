use crate::{
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
    ts::ts_get,
};
use fluentbase_sdk::evm::U256;

#[no_mangle]
pub fn host_tload() {
    let index = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let index = U256::from_be_bytes(index);

    let value = ts_get(index);

    if value.is_some() {
        let value = value.unwrap().to_be_bytes();
        stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, value);
    } else {
        stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, [0u8; U256_BYTES_COUNT as usize]);
    }
}
