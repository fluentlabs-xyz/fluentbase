use fluentbase_codec::Encoder;
use fluentbase_sdk::{
    evm::{ContractInput, ExecutionContext, U256},
    LowLevelAPI, LowLevelSDK, WasmCallMethodInput,
};
use fluentbase_types::{ExitCode, STATE_MAIN};

use crate::account::Account;
use crate::helpers::debug_log;

pub fn _wasm_call(input: WasmCallMethodInput) -> ExitCode {
    debug_log("_wasm_call start");
    // don't allow to do static calls with non zero value
    let is_static = ExecutionContext::contract_is_static();
    if is_static && input.value != U256::ZERO {
        return ExitCode::WriteProtection;
    }
    // parse callee address
    let callee_account = Account::new_from_jzkt(&input.callee);

    let gas_limit = input.gas_limit as u32;

    let contract_input = ContractInput {
        journal_checkpoint: ExecutionContext::journal_checkpoint().into(),
        contract_gas_limit: gas_limit as u64,
        contract_address: input.callee,
        contract_caller: ExecutionContext::contract_caller(),
        contract_input: input.input,
        tx_caller: ExecutionContext::tx_caller(),
        ..Default::default()
    };
    let contract_input_vec = contract_input.encode_to_vec(0);

    let bytecode_hash = callee_account.rwasm_code_hash;
    let exit_code = LowLevelSDK::sys_exec_hash(
        bytecode_hash.as_ptr(),
        contract_input_vec.as_ptr(),
        contract_input_vec.len() as u32,
        core::ptr::null_mut(),
        0,
        &gas_limit as *const u32,
        STATE_MAIN,
    );
    if exit_code != ExitCode::Ok.into_i32() {
        panic!("wasm call failed, exit code: {}", exit_code);
    }
    let out_size = LowLevelSDK::sys_output_size();

    LowLevelSDK::sys_forward_output(0, out_size);

    debug_log("_wasm_call return: OK");
    ExitCode::Ok
}
