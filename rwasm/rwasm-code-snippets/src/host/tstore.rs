use crate::common_sp::{stack_pop_u256, SP_BASE_MEM_OFFSET_DEFAULT};
use fluentbase_sdk::evm::U256;

#[no_mangle]
pub fn host_tstore() {
    let index = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let value = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let index = U256::from_be_bytes(index);
    let value = U256::from_be_bytes(value);

    // let contract_address = ExecutionContext::contract_address();
    // ts_set(index, value);
}
