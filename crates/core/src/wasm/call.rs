use crate::debug_log;
use fluentbase_codec::Encoder;
use fluentbase_sdk::{
    types::{EvmCallMethodOutput, WasmCallMethodInput, WasmCallMethodOutput},
    ContextReader,
    ContractInput,
    SovereignAPI,
};
use fluentbase_types::{ExitCode, Fuel, STATE_MAIN, U256};

pub fn _wasm_call<CTX: ContextReader, SDK: SovereignAPI>(
    ctx: &CTX,
    sdk: &SDK,
    input: WasmCallMethodInput,
) -> WasmCallMethodOutput {
    debug_log!(sdk, "_wasm_call start");

    // don't allow to do static calls with non zero value
    let is_static = ctx.contract_is_static();
    if is_static && input.value != U256::ZERO {
        debug_log!(
            sdk,
            "_wasm_call return: Err: exit_code: {}",
            ExitCode::WriteProtection
        );
        return WasmCallMethodOutput::from_exit_code(ExitCode::WriteProtection);
    }

    // call depth check
    if input.depth > 1024 {
        return EvmCallMethodOutput::from_exit_code(ExitCode::CallDepthOverflow);
    }

    // create a new checkpoint position in the journal
    let checkpoint = sdk.checkpoint();

    // parse callee address
    let (callee_account, _) = sdk.account(&input.callee);

    let mut gas_limit = input.gas_limit;

    let mut context = ContractInput::clone_from_ctx(ctx);
    context.contract_gas_limit = gas_limit as u64;
    context.contract_address = input.callee;
    let contract_context = context.encode_to_vec(0);

    let mut fuel = Fuel::from(gas_limit);
    let (output_buffer, exit_code) = sdk.context_call(
        &input.callee,
        &input.input,
        &contract_context,
        &mut fuel,
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
