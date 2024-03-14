use crate::account::Account;
use alloc::vec;
use fluentbase_codec::Encoder;
use fluentbase_sdk::{
    evm::{ContractInput, ExecutionContext, U256},
    LowLevelAPI,
    LowLevelSDK,
};
use fluentbase_types::{ExitCode, STATE_MAIN};
use revm_interpreter::primitives::Address;

#[no_mangle]
pub fn _wasm_call(
    gas_limit: u32,
    callee_address20_offset: *const u8,
    value32_offset: *const u8,
    args_offset: *const u8,
    args_size: u32,
) -> ExitCode {
    let value =
        U256::from_be_slice(unsafe { &*core::ptr::slice_from_raw_parts(value32_offset, 32) });
    let is_static = ExecutionContext::contract_is_static();
    if is_static && value != U256::ZERO {
        return ExitCode::WriteProtection;
    }

    let callee_address = Address::from_slice(unsafe {
        &*core::ptr::slice_from_raw_parts(callee_address20_offset, 20)
    });

    let caller_address = ExecutionContext::contract_caller();
    let callee_account = Account::new_from_jzkt(&fluentbase_types::Address::from_slice(
        callee_address.as_slice(),
    ));

    let contract_input = ExecutionContext::contract_input();
    let contract_address = ExecutionContext::contract_address();
    let contract_input = ContractInput {
        journal_checkpoint: ExecutionContext::journal_checkpoint().into(),
        contract_gas_limit: gas_limit as u64,
        contract_address,
        contract_caller: ExecutionContext::contract_caller(),
        contract_input_size: contract_input.len() as u32,
        contract_input,
        tx_caller: ExecutionContext::tx_caller(),
        ..Default::default()
    };
    let contract_input_vec = contract_input.encode_to_vec(0);

    let bytecode = callee_account.load_bytecode();
    if value != U256::ZERO {
        return ExitCode::UnknownError;
    };
    let exit_code = LowLevelSDK::sys_exec(
        bytecode.as_ptr(),
        bytecode.len() as u32,
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
    let mut output = vec![0u8; out_size as usize];
    // TODO optimize, copy from source to dest without buffer
    LowLevelSDK::sys_read_output(output.as_mut_ptr(), 0, output.len() as u32);
    LowLevelSDK::sys_write(&output);

    ExitCode::Ok
}
