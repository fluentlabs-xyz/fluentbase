use fluentbase_sdk::evm::ExecutionContext;

#[no_mangle]
fn system_caller() -> [u8; 20] {
    ExecutionContext::contract_caller().into_array()
}
