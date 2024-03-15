use crate::{account::Account, consts::WCL_CONTRACT_ADDRESS};
use alloc::vec;
use fluentbase_codec::Encoder;
use fluentbase_core_api::{
    api::CoreInput,
    bindings::{WasmCallMethodInput, WASM_CALL_METHOD_ID},
};
use fluentbase_sdk::{evm::ExecutionContext, LowLevelAPI, LowLevelSDK};
use fluentbase_types::STATE_MAIN;

pub fn deploy() {
    LowLevelSDK::sys_write(include_bytes!("../../bin/wasm_loader_contract.wasm"));
    LowLevelSDK::sys_halt(0);
}

#[deprecated]
pub fn main() {
    let mut contract_input_data = ExecutionContext::contract_input_full();

    let gas_limit = contract_input_data.contract_gas_limit as u32;
    let method_data = WasmCallMethodInput::new(
        contract_input_data.contract_address.into_array(),
        contract_input_data.contract_value.to_be_bytes(),
        contract_input_data.contract_input.to_vec(),
        gas_limit,
    );
    let core_input = CoreInput::new(WASM_CALL_METHOD_ID, method_data.encode_to_vec(0));
    contract_input_data.contract_input = core_input.encode_to_vec(0).into();
    contract_input_data.contract_address = WCL_CONTRACT_ADDRESS;
    // TODO simplify by loading with more granular methods
    let ecl_account = Account::new_from_jzkt(&WCL_CONTRACT_ADDRESS);
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
    LowLevelSDK::sys_forward_output(0, out_size);
}
