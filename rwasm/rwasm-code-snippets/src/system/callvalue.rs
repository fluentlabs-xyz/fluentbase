use fluentbase_sdk::evm::ExecutionContext;

#[no_mangle]
fn system_callvalue() -> [u8; 32] {
    let mut res = [0u8; 32];
    let v = ExecutionContext::contract_value();
    res.copy_from_slice(&v.to_be_bytes::<32>());
    res
}
