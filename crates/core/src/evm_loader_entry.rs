use crate::{account::Account, consts::ECL_CONTRACT_ADDRESS, evm::call::_evm_call};
use alloc::vec;
use core::ptr::null_mut;
use fluentbase_codec::Encoder;
use fluentbase_core_api::{
    api::CoreInput,
    bindings::{EvmCallMethodInput, EVM_CALL_METHOD_ID},
};
use fluentbase_sdk::{
    evm::{ContractInput, ExecutionContext},
    LowLevelAPI,
    LowLevelSDK,
};
use fluentbase_types::{address, Address, STATE_MAIN};

pub fn deploy() {
    LowLevelSDK::sys_write(include_bytes!("../bin/evm_loader_contract.wasm"));
    LowLevelSDK::sys_halt(0);
}

pub fn main() {
    let gas_limit = ExecutionContext::contract_gas_limit() as u32;
    let mut contract_input = ExecutionContext::contract_input();
    let callee_address = ExecutionContext::contract_address();
    let contract_value = ExecutionContext::contract_value();

    let method_data = EvmCallMethodInput::new(
        callee_address.into_array(),
        contract_value.to_be_bytes(),
        contract_input.to_vec(),
        gas_limit,
    );
    let core_input = CoreInput::new(EVM_CALL_METHOD_ID, method_data.encode_to_vec(0));
    let mut contract_input_data = ExecutionContext::contract_input_full();
    contract_input_data.contract_input = core_input.encode_to_vec(0).into();
    contract_input_data.contract_address = ECL_CONTRACT_ADDRESS;
    let ecl_account = Account::new_from_jzkt(&ECL_CONTRACT_ADDRESS);
    let bytecode = ecl_account.load_bytecode();
    let contract_input_data_vec = contract_input_data.encode_to_vec(0);

    let exit_code = LowLevelSDK::sys_exec(
        bytecode.as_ptr(),
        bytecode.len() as u32,
        contract_input_data_vec.as_ptr(),
        contract_input_data_vec.len() as u32,
        core::ptr::null_mut(),
        0,
        &gas_limit as *const u32,
        STATE_MAIN,
    );
    if exit_code != 0 {
        panic!("ecl: call failed, exit code: {}", exit_code)
    }
    // TODO LowLevelSDK::sys_forward_output to get rid of redundant copy
    let out_size = LowLevelSDK::sys_output_size();
    let mut output = vec![0u8; out_size as usize];
    LowLevelSDK::sys_read_output(output.as_mut_ptr(), 0, output.len() as u32);
    LowLevelSDK::sys_write(&output);
}
