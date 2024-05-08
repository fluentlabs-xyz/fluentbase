use crate::debug_log;
use alloc::{format, vec};
use fluentbase_codec::Encoder;
use fluentbase_sdk::{
    AccountManager, ContextReader, ContractInput, LowLevelAPI, LowLevelSDK, WasmCallMethodInput,
    WasmCallMethodOutput,
};
use fluentbase_types::{Address, Bytes, ExitCode, STATE_MAIN, U256};

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
    // parse callee address
    let (callee_account, _) = am.account(input.callee);

    let mut gas_limit = input.gas_limit as u32;

    let contract_input = ContractInput {
        journal_checkpoint: cr.journal_checkpoint().into(),
        contract_gas_limit: gas_limit as u64,
        contract_address: input.callee,
        contract_caller: cr.contract_caller(),
        contract_input: input.input,
        tx_caller: cr.tx_caller(),
        ..Default::default()
    };
    let contract_input_vec = contract_input.encode_to_vec(0);

    let bytecode_hash = callee_account.rwasm_code_hash;
    let (output_buffer, exit_code) = am.exec_hash(
        bytecode_hash.as_ptr(),
        &contract_input_vec,
        &mut gas_limit as *mut u32,
        STATE_MAIN,
    );

    debug_log!("_wasm_call return: OK: exit_code: {}", exit_code);
    WasmCallMethodOutput {
        output: output_buffer.into(),
        exit_code,
        gas: gas_limit as u64,
    }
}
