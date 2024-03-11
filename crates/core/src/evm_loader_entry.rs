use crate::evm::call::_evm_call;
use alloc::vec;
use core::ptr::null_mut;
use fluentbase_sdk::{evm::ExecutionContext, LowLevelAPI, LowLevelSDK};
use revm_interpreter::primitives::hex;

pub fn deploy() {
    LowLevelSDK::sys_write(include_bytes!("../bin/evm_loader.wasm"));
    LowLevelSDK::sys_halt(0);
}

pub fn main() {
    let mut contract_input = ExecutionContext::contract_input();
    let callee_address20 = ExecutionContext::contract_address();
    let contract_value = ExecutionContext::contract_value();
    // TODO forward to ECL instead of calling _evm_call directly
    // TODO 4test
    if contract_input.starts_with(&hex!("45773e4e")) {
        panic!()
    }
    let exit_code = _evm_call(
        0,
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
    let size = LowLevelSDK::sys_output_size();
    let mut output = vec![0u8; size as usize];
    LowLevelSDK::sys_read_output(output.as_mut_ptr(), 0, output.len() as u32);
    LowLevelSDK::sys_write(&output);
}
