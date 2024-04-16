use crate::helpers::unwrap_exit_code;
use crate::wasm::{call::_wasm_call, create::_wasm_create, create2::_wasm_create2};
use fluentbase_codec::{BufferDecoder, Encoder};
use fluentbase_core_api::{
    api::CoreInput,
    bindings::{
        WasmCallMethodInput, WasmCreate2MethodInput, WasmCreateMethodInput, WasmMethodName,
    },
};
use fluentbase_sdk::{evm::ExecutionContext, LowLevelAPI, LowLevelSDK};

macro_rules! decode_input {
    ($core_input: ident, $method_input: ident) => {{
        let mut buffer = BufferDecoder::new(&mut $core_input.method_data);
        let mut method_input = $method_input::default();
        $method_input::decode_body(&mut buffer, 0, &mut method_input);
        method_input
    }};
}

pub fn deploy() {}

pub fn main() {
    let contract_input = ExecutionContext::contract_input();
    let mut buffer = BufferDecoder::new(contract_input.as_ref());
    let mut core_input = CoreInput::default();
    CoreInput::decode_body(&mut buffer, 0, &mut core_input);

    let method_name = WasmMethodName::try_from(core_input.method_id);
    if let Ok(method_name) = method_name {
        match method_name {
            WasmMethodName::WasmCreate => {
                let method_input = decode_input!(core_input, WasmCreateMethodInput);
                let address = unwrap_exit_code(_wasm_create(method_input));
                LowLevelSDK::sys_write(address.as_slice());
            }
            WasmMethodName::WasmCreate2 => {
                let method_input = decode_input!(core_input, WasmCreate2MethodInput);
                let address = unwrap_exit_code(_wasm_create2(method_input));
                LowLevelSDK::sys_write(address.as_slice());
            }
            WasmMethodName::WasmCall => {
                let method_input = decode_input!(core_input, WasmCallMethodInput);
                let exit_code = _wasm_call(method_input);
                if !exit_code.is_ok() {
                    panic!("call method failed, exit code: {}", exit_code.into_i32())
                }
            }
        }
    } else {
        panic!("unknown method id: {}", core_input.method_id);
    }
}

#[cfg(test)]
mod tests {
    use fluentbase_codec::Encoder;
    use fluentbase_core_api::{
        api::CoreInput,
        bindings::{WasmCreateMethodInput, WASM_CREATE_METHOD_ID},
    };
    use fluentbase_sdk::{evm::ContractInput, LowLevelSDK};
    use fluentbase_types::{Address, Bytes};

    #[test]
    fn test_greeting_deploy() {
        let wasm_bytecode = include_bytes!("../../../../examples/bin/greeting.wasm");
        let wasm_call_input = WasmCreateMethodInput {
            value32: [0u8; 32],
            code: wasm_bytecode.to_vec(),
            gas_limit: 3_000_000,
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
