use crate::{consts::ECL_CONTRACT_ADDRESS, debug_log};
use fluentbase_codec::Encoder;
use fluentbase_sdk::{
    AccountManager,
    CoreInput,
    EvmCallMethodInput,
    ExecutionContext,
    JzktAccountManager,
    LowLevelAPI,
    LowLevelSDK,
    EVM_CALL_METHOD_ID,
};
use fluentbase_types::STATE_MAIN;

pub fn deploy() {}

pub fn main() {
    debug_log!("evm loader: started");
    let mut contract_input_data = ExecutionContext::contract_input_full();
    let am = JzktAccountManager::default();
    let mut gas_limit = contract_input_data.contract_gas_limit as u32;
    let method_data = EvmCallMethodInput {
        callee: contract_input_data.contract_address,
        value: contract_input_data.contract_value,
        input: contract_input_data.contract_input,
        gas_limit: gas_limit as u64,
        depth: 0,
    };
    let core_input = CoreInput::new(EVM_CALL_METHOD_ID, method_data);
    contract_input_data.contract_input = core_input.encode_to_vec(0).into();
    contract_input_data.contract_address = ECL_CONTRACT_ADDRESS;
    let (ecl_account, _) = am.account(ECL_CONTRACT_ADDRESS);
    let contract_input_data_vec = contract_input_data.encode_to_vec(0);
    let rwasm_bytecode_hash = ecl_account.rwasm_code_hash;
    let exit_code = LowLevelSDK::sys_exec_hash(
        rwasm_bytecode_hash.as_ptr(),
        contract_input_data_vec.as_ptr(),
        contract_input_data_vec.len() as u32,
        core::ptr::null_mut(),
        0,
        &mut gas_limit as *mut u32,
        STATE_MAIN,
    );
    // forward output
    let out_size = LowLevelSDK::sys_output_size();
    LowLevelSDK::sys_forward_output(0, out_size);
    // forward exit code as well
    debug_log!("evm loader: return: sys_halt: exit_code: {}", exit_code);
    LowLevelSDK::sys_halt(exit_code);
}

// pub fn main() {
//     let contract_input_data = ExecutionContext::contract_input_full();
//     match _evm_call(
//         EvmCallMethodInput {
//             callee: contract_input_data.contract_address,
//             value: contract_input_data.contract_value,
//             input: contract_input_data.contract_input,
//             gas_limit: contract_input_data.contract_gas_limit,
//         },
//         None,
//     ) {
//         Ok(result) => LowLevelSDK::sys_write(&result),
//         Err(exit_code) => {
//             panic!(
//                 "evm_loader: call failed, exit code: {} ({})",
//                 exit_code.into_i32(),
//                 exit_code
//             )
//         }
//     }
// }
