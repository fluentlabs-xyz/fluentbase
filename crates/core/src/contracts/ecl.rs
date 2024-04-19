use crate::{
    decode_method_input,
    evm::{call::_evm_call, create::_evm_create},
    helpers::unwrap_exit_code,
};
use core::ptr::null_mut;
use fluentbase_codec::{BufferDecoder, Encoder};
use fluentbase_sdk::{
    evm::ExecutionContext, CoreInput, EvmCallMethodInput, EvmCreateMethodInput, LowLevelAPI,
    LowLevelSDK, EVM_CALL_METHOD_ID, EVM_CREATE_METHOD_ID,
};

pub fn deploy() {}

pub fn main() {
    let mut input = ExecutionContext::contract_input();
    let mut buffer = BufferDecoder::new(&mut input);
    let mut core_input = CoreInput::default();
    CoreInput::decode_body(&mut buffer, 0, &mut core_input);

    match core_input.method_id {
        EVM_CREATE_METHOD_ID => {
            let method_input = decode_method_input!(core_input, EvmCreateMethodInput);
            let address = unwrap_exit_code(_evm_create(method_input));
            LowLevelSDK::sys_write(address.as_slice())
        }
        EVM_CALL_METHOD_ID => {
            let method_input = decode_method_input!(core_input, EvmCallMethodInput);
            let output = unwrap_exit_code(_evm_call(method_input));
            LowLevelSDK::sys_write(&output)
        }
        _ => panic!("unknown method id: {}", core_input.method_id),
    }
}
