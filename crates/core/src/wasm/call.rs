use crate::debug_log;
use fluentbase_codec::Encoder;
use fluentbase_sdk::{
    types::{EvmCallMethodOutput, WasmCallMethodInput, WasmCallMethodOutput},
    SovereignAPI,
};
use fluentbase_types::{ExitCode, Fuel, NativeAPI, STATE_MAIN, U256};

pub fn _wasm_call<SDK: SovereignAPI>(
    sdk: &mut SDK,
    input: WasmCallMethodInput,
) -> WasmCallMethodOutput {
    debug_log!(sdk, "_wasm_call start");

    // don't allow to do static calls with non zero value
    if input.is_static && input.value != U256::ZERO {
        debug_log!(
            sdk,
            "_wasm_call return: Err: exit_code: {}",
            ExitCode::WriteProtection
        );
        return WasmCallMethodOutput::from_exit_code(ExitCode::WriteProtection);
    }

    assert_eq!(
        input.bytecode_address, input.address,
        "bytecode address and callee address can't be different in WASM mode"
    );
    assert_eq!(
        input.value, input.apparent_value,
        "value and apparent value can't be different in WASM mode"
    );

    // call depth check
    if input.depth > 1024 {
        return EvmCallMethodOutput::from_exit_code(ExitCode::CallDepthOverflow);
    }

    // create a new checkpoint position in the journal
    let checkpoint = sdk.checkpoint();

    // parse callee address
    let (callee_account, _) = sdk.account(&input.bytecode_address);

    let gas_limit = input.gas_limit;

    let mut fuel = Fuel::from(gas_limit);
    let (output_buffer, exit_code) = sdk.context_call(
        &input.caller,
        &input.bytecode_address,
        &input.value,
        &mut fuel,
        &input.input,
        STATE_MAIN,
    );

    // if exit code success then commit changes, otherwise rollback
    if ExitCode::from(exit_code).is_ok() {
        sdk.commit();
    } else {
        sdk.rollback(checkpoint);
    }

    debug_log!(sdk, "_wasm_call return: OK: exit_code: {}", exit_code);
    WasmCallMethodOutput {
        output: output_buffer.into(),
        exit_code: exit_code.into_i32(),
        gas_remaining: gas_limit,
        gas_refund: 0,
    }
}
