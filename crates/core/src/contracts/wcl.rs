use crate::decode_method_input;
use crate::helpers::{debug_log, unwrap_exit_code, InputHelper};
use crate::wasm::{call::_wasm_call, create::_wasm_create};
use alloc::{format, vec};
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_codec::{BufferDecoder, Encoder};
use fluentbase_sdk::{
    evm::ExecutionContext, CoreInput, EvmCreateMethodInput, ICoreInput, LowLevelAPI, LowLevelSDK,
    WasmCallMethodInput, WasmCreateMethodInput, WASM_CALL_METHOD_ID, WASM_CREATE_METHOD_ID,
};
use fluentbase_types::Bytes;

pub fn deploy() {}

pub fn main() {
    let input_helper = InputHelper::new();
    let method_id = input_helper.decode_method_id();
    match method_id {
        WASM_CREATE_METHOD_ID => {
            let method_input = input_helper.decode_method_input::<WasmCreateMethodInput>();
            let address = unwrap_exit_code(_wasm_create(method_input));
            LowLevelSDK::sys_write(address.as_slice());
        }
        WASM_CALL_METHOD_ID => {
            let method_input = input_helper.decode_method_input::<WasmCallMethodInput>();
            let method_output = _wasm_call(method_input);
            if !method_output.output.is_empty() {
                LowLevelSDK::sys_write(method_output.output.as_ref());
            }
            debug_log(&format!(
                "wcl: WASM_CALL_METHOD_ID: sys_halt: exit_code: {}",
                method_output.exit_code
            ));
            LowLevelSDK::sys_halt(method_output.exit_code);
        }
        _ => panic!("unknown method id: {}", method_id),
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
        let core_input = CoreInput {
            method_id: WASM_CREATE_METHOD_ID,
            method_data: WasmCreateMethodInput {
                value: U256::ZERO,
                bytecode: wasm_bytecode.into(),
                gas_limit: 3_000_000,
                salt: None,
            },
        };
        let contract_input = ContractInput {
            contract_input: core_input.encode_to_vec(0).into(),
            ..Default::default()
        }
        .encode_to_vec(0);
        LowLevelSDK::with_test_input(contract_input);
        super::main();
        assert!(LowLevelSDK::get_test_output().len() > 0);
    }
}
