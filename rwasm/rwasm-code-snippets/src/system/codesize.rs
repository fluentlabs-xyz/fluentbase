use crate::{
    common::u256_from_slice,
    common_sp::{u256_push, SP_VAL_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use fluentbase_sdk::{evm::ExecutionContext, EvmPlatformSDK, SDK};

#[no_mangle]
pub fn system_codesize() {
    let v = ExecutionContext::contract_code_size().to_be_bytes();

    u256_push(SP_VAL_MEM_OFFSET_DEFAULT, u256_from_slice(&v));
}
