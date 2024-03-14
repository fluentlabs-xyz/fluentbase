use crate::wasm::{call::_wasm_call, create::_wasm_create, create2::_wasm_create2};
use fluentbase_codec::{BufferDecoder, Encoder};
use fluentbase_core_api::{
    api::CoreInput,
    bindings::{
        WasmCallMethodInput,
        WasmCreate2MethodInput,
        WasmCreateMethodInput,
        WasmMethodName,
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

pub fn deploy() {
    LowLevelSDK::sys_write(include_bytes!("../../bin/wcl_contract.wasm"));
    LowLevelSDK::sys_halt(0);
}

pub fn main() {
    let mut input = ExecutionContext::contract_input();
    let mut buffer = BufferDecoder::new(&mut input);
    let mut core_input = CoreInput::default();
    CoreInput::decode_body(&mut buffer, 0, &mut core_input);

    let method_name = WasmMethodName::try_from(core_input.method_id);
    if let Ok(method_name) = method_name {
        match method_name {
            WasmMethodName::WasmCreate => {
                let method_input = decode_input!(core_input, WasmCreateMethodInput);
                let mut output20 = [0u8; 20];
                let exit_code = _wasm_create(
                    method_input.value32.as_ptr(),
                    method_input.code.as_ptr(),
                    method_input.code.len() as u32,
                    method_input.gas_limit,
                    output20.as_mut_ptr(),
                );
                if !exit_code.is_ok() {
                    panic!("create method failed, exit code: {}", exit_code.into_i32())
                }
                LowLevelSDK::sys_write(&output20);
            }
            WasmMethodName::WasmCreate2 => {
                let method_input = decode_input!(core_input, WasmCreate2MethodInput);
                let mut out_address = [0u8; 20];
                let exit_code = _wasm_create2(
                    method_input.value32.as_ptr(),
                    method_input.code.as_ptr(),
                    method_input.code.len() as u32,
                    method_input.salt32.as_ptr(),
                    method_input.gas_limit,
                    out_address.as_mut_ptr(),
                );
                if !exit_code.is_ok() {
                    panic!("create2 method failed, exit code: {}", exit_code.into_i32())
                }
                LowLevelSDK::sys_write(&out_address);
            }
            WasmMethodName::WasmCall => {
                let method_input = decode_input!(core_input, WasmCallMethodInput);
                let exit_code = _wasm_call(
                    method_input.gas_limit,
                    method_input.callee_address20.as_ptr(),
                    method_input.value32.as_ptr(),
                    core::ptr::null_mut(),
                    0,
                );
                if !exit_code.is_ok() {
                    panic!("call method failed, exit code: {}", exit_code.into_i32())
                }
            }
        }
    } else {
        panic!("unknown method id: {}", core_input.method_id);
    }
}
