use crate::evm::call::_evm_call;
use alloc::vec;
use core::ptr::null_mut;
use fluentbase_sdk::{evm::ExecutionContext, LowLevelAPI, LowLevelSDK};

pub fn deploy() {
    LowLevelSDK::sys_write(include_bytes!("../bin/evm_loader_contract.wasm"));
    LowLevelSDK::sys_halt(0);
}

pub fn main() {
    let contract_gas_limit = ExecutionContext::contract_gas_limit() as u32;
    let mut contract_input = ExecutionContext::contract_input();
    let callee_address20 = ExecutionContext::contract_address();
    let contract_value = ExecutionContext::contract_value();
    // TODO forward to ECL instead of calling _evm_call directly
    let exit_code = _evm_call(
        contract_gas_limit,
        callee_address20.as_ptr(),
        contract_value.to_be_bytes::<32>().as_ptr(),
        contract_input.as_ptr(),
        contract_input.len() as u32,
        null_mut(),
        0,
    );
    if exit_code.is_not_ok() {
        panic!(
            "_evm_call method failed, exit code: {}",
            exit_code.into_i32()
        )
    }
    // TODO LowLevelSDK::sys_forward_output to get rid of redundant copy
    let out_size = LowLevelSDK::sys_output_size();
    let mut output = vec![0u8; out_size as usize];
    LowLevelSDK::sys_read_output(output.as_mut_ptr(), 0, output.len() as u32);
    LowLevelSDK::sys_write(&output);
}
