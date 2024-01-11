use crate::{
    common::{u256_be_to_tuple_le, u256_from_be_slice},
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
};
use fluentbase_sdk::evm::ExecutionContext;

#[no_mangle]
pub fn host_env_blobhash() {
    let idx = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let idx = u256_be_to_tuple_le(idx);
    let hashes = ExecutionContext::tx_blob_hashes();
    if idx.1 > 0 || idx.2 > 0 || idx.3 > 0 || idx.0 >= hashes.len() as u64 {
        stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, u256_from_be_slice(&[]));
    } else {
        let hash = hashes[idx.0 as usize].0;
        stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, hash);
    }
}
