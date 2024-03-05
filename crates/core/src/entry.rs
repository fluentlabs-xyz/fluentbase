use crate::{api::CREATE_METHOD_ID, evm::create::_evm_create, CoreInput, CreateMethodInput};
use fluentbase_codec::{BufferDecoder, Encoder};
use fluentbase_sdk::{evm::ExecutionContext, LowLevelAPI, LowLevelSDK};

#[no_mangle]
pub fn main() {
    let mut input = ExecutionContext::contract_input();
    let mut buffer = BufferDecoder::new(&mut input);
    let mut core_input = CoreInput::default();
    CoreInput::decode_body(&mut buffer, 0, &mut core_input);
    match core_input.method_id {
        CREATE_METHOD_ID => {
            let mut buffer = BufferDecoder::new(&mut core_input.method_data);
            let mut data = CreateMethodInput::default();
            CreateMethodInput::decode_body(&mut buffer, 0, &mut data);
            let mut output20 = [0u8; 20];
            let exit_code = _evm_create(
                data.value32.as_ptr(),
                data.code.as_ptr(),
                data.code.len() as u32,
                output20.as_mut_ptr(),
                data.gas_limit,
            );
            if !exit_code.is_ok() {
                panic!("create method failed, exit code: {}", exit_code.into_i32())
            }
            LowLevelSDK::sys_write(&output20);
        }
        _ => {
            panic!("unsupported method id: {}", core_input.method_id);
        }
    }
    // TODO route to EVM func
}
