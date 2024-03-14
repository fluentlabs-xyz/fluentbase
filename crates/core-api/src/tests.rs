use crate::bindings::{
    EvmMethodName,
    WasmMethodName,
    EVM_CALL_METHOD_ID,
    EVM_CREATE2_METHOD_ID,
    EVM_CREATE_METHOD_ID,
    WASM_CALL_METHOD_ID,
    WASM_CREATE2_METHOD_ID,
    WASM_CREATE_METHOD_ID,
};

#[test]
fn method_name() {
    assert!(EvmMethodName::try_from(EVM_CREATE_METHOD_ID).is_ok());
    assert!(EvmMethodName::try_from(EVM_CREATE2_METHOD_ID).is_ok());
    assert!(EvmMethodName::try_from(EVM_CALL_METHOD_ID).is_ok());
    assert!(WasmMethodName::try_from(WASM_CREATE_METHOD_ID).is_ok());
    assert!(WasmMethodName::try_from(WASM_CREATE2_METHOD_ID).is_ok());
    assert!(WasmMethodName::try_from(WASM_CALL_METHOD_ID).is_ok());

    assert!(EvmMethodName::try_from(WASM_CREATE_METHOD_ID).is_ok());
    assert!(WasmMethodName::try_from(EVM_CREATE_METHOD_ID).is_ok());
}
