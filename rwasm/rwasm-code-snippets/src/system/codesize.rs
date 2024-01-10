use crate::{
    common::u256_from_be_slice,
    common_sp::{stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use fluentbase_sdk::{evm::ExecutionContext, EvmPlatformSDK, SDK};

#[no_mangle]
pub fn system_codesize() {
    let v = ExecutionContext::contract_code_size().to_be_bytes();

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, u256_from_be_slice(&v));
}
