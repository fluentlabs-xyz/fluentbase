use crate::{
    common::u256_from_be_slice,
    common_sp::{stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
};
use fluentbase_sdk::evm::ExecutionContext;

#[no_mangle]
pub fn host_blockhash() {
    let v = ExecutionContext::block_hash();

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, u256_from_be_slice(v.as_slice()));
}
