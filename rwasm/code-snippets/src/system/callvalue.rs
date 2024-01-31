use crate::common_sp::{stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT};
use fluentbase_sdk::evm::ExecutionContext;

#[no_mangle]
fn system_callvalue() {
    let v = ExecutionContext::contract_value().to_be_bytes();

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, v);
}
