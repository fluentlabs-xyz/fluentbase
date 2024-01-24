use crate::{
    common::{u256_be_to_tuple_le, u256_from_be_slice_align_left},
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use fluentbase_sdk::evm::ExecutionContext;

#[no_mangle]
fn system_calldataload() {
    let idx = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let idx = u256_be_to_tuple_le(idx).0 as usize;
    let mut idx_tail = idx + (U256_BYTES_COUNT as usize);

    let ci = ExecutionContext::contract_input();
    if idx_tail > ci.len() {
        idx_tail = ci.len();
    }
    let v = if idx_tail > idx {
        &ci[idx..idx_tail]
    } else {
        &[]
    };

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, u256_from_be_slice_align_left(v));
}
