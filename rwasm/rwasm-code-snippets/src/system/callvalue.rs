use crate::common_sp::{u256_push, SP_VAL_MEM_OFFSET_DEFAULT};
use fluentbase_sdk::evm::ExecutionContext;

#[no_mangle]
fn system_callvalue() {
    let v = ExecutionContext::contract_value().to_be_bytes();

    u256_push(SP_VAL_MEM_OFFSET_DEFAULT, v);
}
