use crate::{
    common_sp::{stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use fluentbase_sdk::evm::ExecutionContext;

#[no_mangle]
pub fn host_env_gasprice() {
    let v: [u8; U256_BYTES_COUNT as usize] = ExecutionContext::tx_gas_price().to_be_bytes();

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, v);
}
