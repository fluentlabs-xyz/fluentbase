use crate::decode_method_input;
use crate::helpers::unwrap_exit_code;
use crate::wasm::{call::_wasm_call, create::_wasm_create};
use fluentbase_codec::{BufferDecoder, Encoder};
use fluentbase_sdk::{
    evm::ExecutionContext, CoreInput, LowLevelAPI, LowLevelSDK, WasmCallMethodInput,
    WasmCreateMethodInput, WASM_CALL_METHOD_ID, WASM_CREATE_METHOD_ID,
};

pub fn deploy() {}

pub fn main() {
    let contract_input = ExecutionContext::contract_input();
    let mut buffer = BufferDecoder::new(contract_input.as_ref());
    let mut core_input = CoreInput::default();
    CoreInput::decode_body(&mut buffer, 0, &mut core_input);

    match core_input.method_id {
        WASM_CREATE_METHOD_ID => {
            let method_input = decode_method_input!(core_input, WasmCreateMethodInput);
            let address = unwrap_exit_code(_wasm_create(method_input));
            LowLevelSDK::sys_write(address.as_slice());
        }
        WASM_CALL_METHOD_ID => {
            let method_input = decode_method_input!(core_input, WasmCallMethodInput);
            let exit_code = _wasm_call(method_input);
            if !exit_code.is_ok() {
                panic!(
                    "wcl: call method failed, exit code: {}",
                    exit_code.into_i32()
                )
            }
        }
        _ => panic!("unknown method id: {}", core_input.method_id),
    }
}

#[cfg(test)]
mod tests {
    use fluentbase_codec::Encoder;
    use fluentbase_sdk::{
        evm::ContractInput, CoreInput, LowLevelSDK, WasmCreateMethodInput, WASM_CREATE_METHOD_ID,
    };
    use fluentbase_types::{Address, Bytes};
    use revm_primitives::U256;

    #[test]
    fn test_greeting_deploy() {
        let wasm_bytecode = include_bytes!("../../../../examples/bin/greeting.wasm");
        let wasm_call_input = WasmCreateMethodInput {
            value: U256::ZERO,
            bytecode: wasm_bytecode.into(),
            gas_limit: 3_000_000,
            salt: None,
        };
        let core_input = CoreInput {
            method_id: WASM_CREATE_METHOD_ID,
            method_data: wasm_call_input.encode_to_vec(0),
        };
        let contract_input = ContractInput {
            contract_address: Address::new([0u8; 20]),
            contract_caller: Address::new([3u8; 20]),
            contract_input: Bytes::from(core_input.encode_to_vec(0)),
            ..Default::default()
        };
        LowLevelSDK::with_test_input(contract_input.encode_to_vec(0));
        super::main();
        assert!(LowLevelSDK::get_test_output().len() > 0);
    }
}
