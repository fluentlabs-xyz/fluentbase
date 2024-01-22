use crate::{
    common::{u256_be_to_tuple_le, u256_from_be_slice_align_left},
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use fluentbase_sdk::evm::ExecutionContext;

#[no_mangle]
fn system_calldataload() {
    let i = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let i = u256_be_to_tuple_le(i).0 as usize;

    let ci = ExecutionContext::contract_input();
    let v = if i < ci.len() {
        if i + (U256_BYTES_COUNT as usize) < ci.len() {
            &ci[i..i + U256_BYTES_COUNT as usize]
        } else {
            &ci[i..ci.len()]
        }
    } else {
        &[]
    };

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, u256_from_be_slice_align_left(v));
}
