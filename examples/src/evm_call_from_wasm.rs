use alloc::vec;
use fluentbase_codec::Encoder;
use fluentbase_core::Account;
use fluentbase_sdk::{
    evm::{ContractInput, ExecutionContext},
    LowLevelAPI, LowLevelSDK,
};
use fluentbase_types::{ExitCode, STATE_MAIN};

pub fn deploy() {}

pub fn main() {
    let ctx = ExecutionContext::default();
    let contract_input = ExecutionContext::contract_input();
    let evm_contract_address = ExecutionContext::contract_address();
    let gas_limit: u32 = ExecutionContext::contract_gas_limit() as u32;
    let contract_input = ContractInput {
        journal_checkpoint: ExecutionContext::journal_checkpoint().into(),
        contract_gas_limit: gas_limit as u64,
        contract_address: evm_contract_address,
        contract_caller: ExecutionContext::contract_caller(),
        contract_input,
        tx_caller: ExecutionContext::tx_caller(),
        ..Default::default()
    };
    let contract_input_vec = contract_input.encode_to_vec(0);
    let account = Account::new_from_jzkt(&evm_contract_address);
    let rwasm_bytecode_hash = account.rwasm_code_hash;

    let exit_code = LowLevelSDK::sys_exec_hash(
        rwasm_bytecode_hash.as_ptr(),
        contract_input_vec.as_ptr(),
        contract_input_vec.len() as u32,
        core::ptr::null_mut(),
        0,
        &gas_limit as *const u32,
        STATE_MAIN,
    );
    if exit_code != ExitCode::Ok.into_i32() {
        panic!("failed to exec loader: {}", exit_code);
    }
    let out_size = LowLevelSDK::sys_output_size();
    let mut out_buf = vec![0u8; out_size as usize];
    LowLevelSDK::sys_read_output(out_buf.as_mut_ptr(), 0, out_buf.len() as u32);
    ctx.fast_return_and_exit(out_buf, ExitCode::Ok.into_i32());
}
