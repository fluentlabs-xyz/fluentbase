use crate::bindings::{
    EvmMethodName,
    EVM_CALL_METHOD_ID,
    EVM_CREATE2_METHOD_ID,
    EVM_CREATE_METHOD_ID,
};

#[test]
fn method_name() {
    let mn = EvmMethodName::try_from(EVM_CREATE_METHOD_ID);
    assert!(mn.is_ok());
    let mn = EvmMethodName::try_from(EVM_CREATE2_METHOD_ID);
    assert!(mn.is_ok());
    let mn = EvmMethodName::try_from(EVM_CALL_METHOD_ID);
    assert!(mn.is_ok());
    // let mn = WasmMethodName::try_from(WASM_CREATE_METHOD_ID);
    // assert!(mn.is_ok());
    // let mn = WasmMethodName::try_from(WASM_CREATE2_METHOD_ID);
    // assert!(mn.is_ok());
    // let mn = WasmMethodName::try_from(WASM_CALL_METHOD_ID);
    // assert!(mn.is_ok());
    //
    // let mn = EvmMethodName::try_from(WASM_CREATE_METHOD_ID);
    // assert!(!mn.is_ok());
    // let mn = WasmMethodName::try_from(EVM_CREATE_METHOD_ID);
    // assert!(!mn.is_ok());
}
