use alloc::vec;
use fluentbase_codec::Encoder;
use fluentbase_core::account::Account;
use fluentbase_sdk::{
    evm::{ContractInput, ExecutionContext},
    LowLevelAPI,
    LowLevelSDK,
};
use fluentbase_types::{address, ExitCode, STATE_MAIN};

pub fn deploy() {
    LowLevelSDK::sys_write(include_bytes!("../bin/evm_call_from_wasm.wasm"));
    LowLevelSDK::sys_halt(0);
}

pub fn main() {
    let ctx = ExecutionContext::default();

    // must be evm_loader address
    let contract_address = ExecutionContext::contract_address();
    let contract_input = ExecutionContext::contract_input();

    let evm_loader_contract_address = address!("0000000000000000000000000000000000000002");
    let evm_contract_address = address!("199e3643d07aefc7672aaa66aada5347e67e6076");

    let contract_input = ContractInput {
        journal_checkpoint: ExecutionContext::journal_checkpoint().into(),
        contract_address: evm_contract_address,
        contract_caller: ExecutionContext::contract_caller(),
        contract_input_size: contract_input.len() as u32,
        contract_input,
        tx_caller: ExecutionContext::tx_caller(),
        ..Default::default()
    };
    let contract_input_vec = contract_input.encode_to_vec(0);
    let fuel: u32 = 10_000_000;
    // TODO rewrite using basic funcs
    let account = Account::new_from_jzkt(&evm_contract_address);
    let bytecode = account.load_bytecode();

    let exit_code = LowLevelSDK::sys_exec(
        bytecode.as_ptr(),
        bytecode.len() as u32,
        contract_input_vec.as_ptr(),
        contract_input_vec.len() as u32,
        core::ptr::null_mut(),
        0,
        &fuel as *const u32,
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
