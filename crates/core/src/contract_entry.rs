use crate::evm::create::_evm_create;
use fluentbase_codec::{BufferDecoder, Encoder};
use fluentbase_core_api::{
    api::CoreInput,
    bindings::{EVMMethodName, EvmCreateMethodInput},
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

#[no_mangle]
pub fn main() {
    let mut input = ExecutionContext::contract_input();
    let mut buffer = BufferDecoder::new(&mut input);
    let mut core_input = CoreInput::default();
    CoreInput::decode_body(&mut buffer, 0, &mut core_input);

    let method_name = EVMMethodName::try_from(core_input.method_id);
    if let Ok(method_name) = method_name {
        match method_name {
            EVMMethodName::EvmCreate => {
                let method_input = decode_input!(core_input, EvmCreateMethodInput);
                let mut output20 = [0u8; 20];
                let exit_code = _evm_create(
                    method_input.value32.as_ptr(),
                    method_input.code.as_ptr(),
                    method_input.code.len() as u32,
                    output20.as_mut_ptr(),
                    method_input.gas_limit,
                );
                if !exit_code.is_ok() {
                    panic!("create method failed, exit code: {}", exit_code.into_i32())
                }
                LowLevelSDK::sys_write(&output20);
            }
            EVMMethodName::EvmCreate2 => {}
            EVMMethodName::EvmCall => {}
        }
    } else {
        panic!("unknown method id: {}", core_input.method_id);
    }
}
