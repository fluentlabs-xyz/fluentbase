use crate::{
    common::{u256_be_to_tuple_le, u256_from_be_slice},
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use fluentbase_sdk::evm::ExecutionContext;

#[no_mangle]
fn system_calldataload() {
    let i = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let i = u256_be_to_tuple_le(i).0;
    let ci = ExecutionContext::contract_input();
    let v = if i < ci.len() as u64 {
        if i + U256_BYTES_COUNT < ci.len() as u64 {
            &ci[i as usize..(i + U256_BYTES_COUNT) as usize]
        } else {
            &ci[i as usize..ci.len() - i as usize]
        }
    } else {
        &[]
    };

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, u256_from_be_slice(v));
}
