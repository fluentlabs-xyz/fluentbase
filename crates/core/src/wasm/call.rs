use crate::account::Account;
use fluentbase_codec::Encoder;
use fluentbase_sdk::{
    evm::{ContractInput, ExecutionContext, U256},
    LowLevelAPI,
    LowLevelSDK,
};
use fluentbase_types::{Address, Bytes, ExitCode, STATE_MAIN};

#[no_mangle]
pub fn _wasm_call(
    gas_limit: u32,
    callee_address20_offset: *const u8,
    value32_offset: *const u8,
    args_offset: *const u8,
    args_size: u32,
) -> ExitCode {
    // parse input value
    let value =
        U256::from_be_slice(unsafe { &*core::ptr::slice_from_raw_parts(value32_offset, 32) });
    // don't allow to do static calls with non zero value
    let is_static = ExecutionContext::contract_is_static();
    if is_static && value != U256::ZERO {
        return ExitCode::WriteProtection;
    }
    // parse args
    let args = unsafe { &*core::ptr::slice_from_raw_parts(args_offset, args_size as usize) };
    // parse callee address
    let callee_address = Address::from_slice(unsafe {
        &*core::ptr::slice_from_raw_parts(callee_address20_offset, 20)
    });
    let mut callee_account = Account::new_from_jzkt(&callee_address);
    // read caller address from execution context
    let caller_address = ExecutionContext::contract_caller();
    let mut caller_account = Account::new_from_jzkt(&caller_address);
    // if value is not zero then transfer funds from one account to another
    if value != U256::ZERO {
        match Account::transfer(&mut caller_account, &mut callee_account, value) {
            Ok(_) => {}
            Err(exit_code) => return exit_code,
        }
    };

    let contract_input = ContractInput {
        journal_checkpoint: ExecutionContext::journal_checkpoint().into(),
        contract_gas_limit: gas_limit as u64,
        contract_address: callee_address,
        contract_caller: ExecutionContext::contract_caller(),
        contract_input: Bytes::from_static(args),
        tx_caller: ExecutionContext::tx_caller(),
        ..Default::default()
    };
    let contract_input_vec = contract_input.encode_to_vec(0);

    if value != U256::ZERO {
        return ExitCode::UnknownError;
    };
    let bytecode_hash = callee_account.rwasm_bytecode_hash;
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

    ExitCode::Ok
}
