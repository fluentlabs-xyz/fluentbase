use crate::{account::Account, consts::ECL_CONTRACT_ADDRESS};
use fluentbase_codec::Encoder;
use fluentbase_sdk::{
    evm::ExecutionContext, CoreInput, EvmCallMethodInput, LowLevelAPI, LowLevelSDK,
    EVM_CALL_METHOD_ID,
};
use fluentbase_types::STATE_MAIN;
use revm_primitives::hex;

pub fn deploy() {}

pub fn main() {
    let mut contract_input_data = ExecutionContext::contract_input_full();
    let contract_input_data_prev_vec = ExecutionContext::raw_input();

    let gas_limit = contract_input_data.contract_gas_limit as u32;
    let method_data = EvmCallMethodInput {
        callee: contract_input_data.contract_address,
        value: contract_input_data.contract_value,
        input: contract_input_data.contract_input,
        gas_limit: gas_limit as u64,
    };
    let core_input = CoreInput::new(EVM_CALL_METHOD_ID, method_data.encode_to_vec(0));
    contract_input_data.contract_input = core_input.encode_to_vec(0).into();
    contract_input_data.contract_address = ECL_CONTRACT_ADDRESS;
    let ecl_account = Account::new_from_jzkt(&ECL_CONTRACT_ADDRESS);
    let contract_input_data_vec = contract_input_data.encode_to_vec(0);
    let rwasm_bytecode_hash = ecl_account.rwasm_code_hash;
    let exit_code = LowLevelSDK::sys_exec_hash(
        rwasm_bytecode_hash.as_ptr(),
        contract_input_data_vec.as_ptr(),
        contract_input_data_vec.len() as u32,
        core::ptr::null_mut(),
        0,
        &gas_limit as *const u32,
        STATE_MAIN,
    );
    if exit_code != 0 {
        panic!(
            "evm_loader: call failed, exit code: {}. contract_input_data_prev_vec '{}' contract_input_data_vec '{}' rwasm_bytecode_hash '{}'",
            exit_code,
            hex::encode(contract_input_data_prev_vec),
            hex::encode(contract_input_data_vec),
            hex::encode(rwasm_bytecode_hash.0),
        )
    }
    let out_size = LowLevelSDK::sys_output_size();
    LowLevelSDK::sys_forward_output(0, out_size);
}
