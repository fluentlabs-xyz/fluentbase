use crate::helpers::{debug_log, InputHelper};
use crate::{
    decode_method_input,
    evm::{call::_evm_call, create::_evm_create},
    helpers::unwrap_exit_code,
};
use alloc::format;
use byteorder::{ByteOrder, LittleEndian};
use core::ptr::null_mut;
use fluentbase_codec::{BufferDecoder, Encoder};
use fluentbase_sdk::{
    ContractInput, CoreInput, EvmCallMethodInput, EvmCreateMethodInput, ExecutionContext,
    IContractInput, ICoreInput, LowLevelAPI, LowLevelSDK, EVM_CALL_METHOD_ID, EVM_CREATE_METHOD_ID,
};
use fluentbase_types::{Bytes, ExitCode};
use revm_interpreter::SharedMemory;

pub fn deploy() {}

pub fn main() {
    let cr = ExecutionContext::default();
    debug_log("ecl(main): started method");
    let input_helper = InputHelper::new(cr);
    let method_id = input_helper.decode_method_id();
    match method_id {
        EVM_CREATE_METHOD_ID => {
            let method_input = input_helper.decode_method_input::<EvmCreateMethodInput>();
            let method_output = _evm_create(&cr, method_input);
            LowLevelSDK::sys_write(&method_output.encode_to_vec(0));
        }
        EVM_CALL_METHOD_ID => {
            let method_input = input_helper.decode_method_input::<EvmCallMethodInput>();
            let method_output = _evm_call(&cr, method_input);
            LowLevelSDK::sys_write(&method_output.encode_to_vec(0));
            debug_log(&format!(
                "ecl(main): return exit_code={}",
                method_output.exit_code
            ));
        }
        _ => panic!("unknown method id: {}", method_id),
    }

    debug_log("ecl(main): return exit_code=0");
    LowLevelSDK::sys_halt(ExitCode::Ok.into_i32());
}
