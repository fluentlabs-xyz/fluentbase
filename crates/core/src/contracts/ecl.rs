use crate::{
    debug_log,
    evm::{call::_evm_call, create::_evm_create},
    helpers::InputHelper,
};
use fluentbase_codec::Encoder;
use fluentbase_sdk::{
    EvmCallMethodInput,
    EvmCreateMethodInput,
    ExecutionContext,
    JzktAccountManager,
    LowLevelSDK,
    SharedAPI,
    EVM_CALL_METHOD_ID,
    EVM_CREATE_METHOD_ID,
};
use fluentbase_types::ExitCode;

pub fn deploy() {}

pub fn main() {
    let cr = ExecutionContext::default();
    let am = JzktAccountManager::default();
    debug_log!("ecl(main): started method");
    let input_helper = InputHelper::new();
    let method_id = input_helper.decode_method_id();
    match method_id {
        EVM_CREATE_METHOD_ID => {
            let method_input = input_helper.decode_method_input::<EvmCreateMethodInput>();
            let method_output = _evm_create(&cr, &am, method_input);
            LowLevelSDK::write(&method_output.encode_to_vec(0));
        }
        EVM_CALL_METHOD_ID => {
            let method_input = input_helper.decode_method_input::<EvmCallMethodInput>();
            let method_output = _evm_call(&cr, &am, method_input);
            LowLevelSDK::write(&method_output.encode_to_vec(0));
            debug_log!("ecl(main): return exit_code={}", method_output.exit_code);
        }
        _ => panic!("unknown method id: {}", method_id),
    }

    debug_log!("ecl(main): return exit_code=0");
    LowLevelSDK::exit(ExitCode::Ok.into_i32());
}
