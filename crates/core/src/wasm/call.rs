use crate::debug_log;
use fluentbase_codec::Encoder;
use fluentbase_sdk::{
    types::{EvmCallMethodOutput, WasmCallMethodInput, WasmCallMethodOutput},
    AccountManager,
    ContextReader,
    ContractInput,
};
use fluentbase_types::{ExitCode, STATE_MAIN, U256};

pub fn _wasm_call<CR: ContextReader, AM: AccountManager>(
    cr: &CR,
    am: &AM,
    input: WasmCallMethodInput,
) -> WasmCallMethodOutput {
    debug_log!("_wasm_call start");

    // don't allow to do static calls with non zero value
    let is_static = cr.contract_is_static();
    if is_static && input.value != U256::ZERO {
        debug_log!(
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
    let checkpoint = am.checkpoint();

    // parse callee address
    let (callee_account, _) = am.account(input.callee);

    let mut gas_limit = input.gas_limit as u32;

    let mut context = ContractInput::clone_from_cr(cr);
    context.contract_gas_limit = gas_limit as u64;
    context.contract_address = input.callee;
    let contract_context = context.encode_to_vec(0);

    let bytecode_hash = callee_account.rwasm_code_hash;
    let (output_buffer, exit_code) = am.exec_hash(
        bytecode_hash.as_ptr(),
        &contract_context,
        &input.input,
        &mut gas_limit as *mut u32,
        STATE_MAIN,
    );

    // if exit code is successful, then commit changes, otherwise rollback
    if ExitCode::from(exit_code).is_ok() {
        am.commit();
    } else {
        am.rollback(checkpoint);
    }

    debug_log!("_wasm_call return: OK: exit_code: {}", exit_code);
    WasmCallMethodOutput {
        output: output_buffer.into(),
        exit_code,
        gas_remaining: gas_limit as u64,
        gas_refund: 0,
    }
}
