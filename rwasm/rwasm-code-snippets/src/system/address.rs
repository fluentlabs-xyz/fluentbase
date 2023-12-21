use fluentbase_sdk::evm::ExecutionContext;

#[no_mangle]
fn system_address() -> [u8; 20] {
    let mut res = [0u8; 20];
    let v = ExecutionContext::contract_address();
    res.copy_from_slice(v.as_slice());
    res
}
