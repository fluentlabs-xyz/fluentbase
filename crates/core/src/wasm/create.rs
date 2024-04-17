use alloc::vec;

use revm_primitives::RWASM_MAX_CODE_SIZE;

use fluentbase_core_api::bindings::WasmCreateMethodInput;
use fluentbase_sdk::evm::ExecutionContext;
use fluentbase_sdk::LowLevelAPI;
use fluentbase_sdk::LowLevelSDK;
use fluentbase_types::{Address, ExitCode, U256};

use crate::{account::Account, helpers::rwasm_exec_hash};

pub fn _wasm_create(input: WasmCreateMethodInput) -> Result<Address, ExitCode> {
    let value = U256::from_be_bytes(input.value32);

    // TODO: "gas calculations"
    // TODO: "call depth stack check >= 1024"

    // check write protection
    if ExecutionContext::contract_is_static() {
        return Err(ExitCode::WriteProtection);
    }

    // code length can't exceed max constructor limit
    if input.code.len() > RWASM_MAX_CODE_SIZE {
        return Err(ExitCode::ContractSizeLimit);
    }

    // read value input and contract address
    let caller_address = ExecutionContext::contract_caller();
    // load deployer and contract accounts
    let mut deployer_account = Account::new_from_jzkt(&caller_address);

    // create an account
    let mut contract_account = Account::create_account(&mut deployer_account, value, None)?;

    // translate WASM to rWASM
    let exit_code = LowLevelSDK::wasm_to_rwasm(
        input.code.as_ptr(),
        input.code.len() as u32,
        core::ptr::null_mut(),
        0,
    );
    if exit_code != ExitCode::Ok.into_i32() {
        panic!("wasm create failed, exit code: {}", exit_code);
    }
    let rwasm_bytecode_len = LowLevelSDK::sys_output_size();
    let mut rwasm_bytecode = vec![0u8; rwasm_bytecode_len as usize];
    LowLevelSDK::sys_read_output(rwasm_bytecode.as_mut_ptr(), 0, rwasm_bytecode_len);

    // write deployer to the trie
    deployer_account.write_to_jzkt();

    // write contract to the trie
    contract_account.update_bytecode(&input.code.into(), None, &rwasm_bytecode.into(), None);
    let exit_code = rwasm_exec_hash(
        &contract_account.rwasm_code_hash.as_slice(),
        &[],
        input.gas_limit,
        true,
    );
    // if call is not success set deployed address to zero
    if exit_code != ExitCode::Ok.into_i32() {
        return Err(ExitCode::TransactError);
    }

    Ok(contract_account.address)
}
