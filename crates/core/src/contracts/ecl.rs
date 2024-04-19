use core::ptr::null_mut;

use fluentbase_codec::{BufferDecoder, Encoder};
use fluentbase_core_api::{
    api::CoreInput,
    bindings::{EvmCallMethodInput, EvmCreate2MethodInput, EvmCreateMethodInput, EvmMethodName},
};
use fluentbase_sdk::{evm::ExecutionContext, LowLevelAPI, LowLevelSDK};

use crate::decode_method_input;
use crate::evm::{call::_evm_call, create::_evm_create, create2::_evm_create2};
use crate::helpers::unwrap_exit_code;

pub fn deploy() {}

pub fn main() {
    let mut input = ExecutionContext::contract_input();
    let mut buffer = BufferDecoder::new(&mut input);
    let mut core_input = CoreInput::default();
    CoreInput::decode_body(&mut buffer, 0, &mut core_input);

    let method_name = EvmMethodName::try_from(core_input.method_id).expect("unknown method id");

    match method_name {
        EvmMethodName::EvmCreate => {
            let method_input = decode_method_input!(core_input, EvmCreateMethodInput);
            let address = unwrap_exit_code(_evm_create(method_input));
            LowLevelSDK::sys_write(address.as_slice())
        }
        EvmMethodName::EvmCreate2 => {
            let method_input = decode_method_input!(core_input, EvmCreate2MethodInput);
            let address = unwrap_exit_code(_evm_create2(method_input));
            LowLevelSDK::sys_write(address.as_slice())
        }
        EvmMethodName::EvmCall => {
            let method_input = decode_method_input!(core_input, EvmCallMethodInput);
            let output = unwrap_exit_code(_evm_call(method_input));
            LowLevelSDK::sys_write(&output)
        }
    }
}
