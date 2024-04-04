use crate::{
    common::u256_from_be_slice,
    common_sp::{stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
};
use fluentbase_sdk::evm::ExecutionContext;

#[no_mangle]
pub fn host_coinbase() {
    let v = ExecutionContext::block_coinbase().into_array();

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, u256_from_be_slice(&v));
}
