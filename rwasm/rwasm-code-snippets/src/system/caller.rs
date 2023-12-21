use crate::{
    common_sp::{u256_push, SP_VAL_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use fluentbase_sdk::evm::ExecutionContext;

#[no_mangle]
fn system_caller() {
    let v = ExecutionContext::contract_caller().into_array();

    let mut r = [0u8; U256_BYTES_COUNT as usize];
    r[U256_BYTES_COUNT as usize - v.len()..].copy_from_slice(&v);

    u256_push(SP_VAL_MEM_OFFSET_DEFAULT, r);
}
