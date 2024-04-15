use crate::{
    account::Account,
    helpers::{rwasm_exec_hash, wasm2rwasm},
};
use fluentbase_core_api::bindings::WasmCreate2MethodInput;
use fluentbase_sdk::{
    evm::{ExecutionContext, U256},
    LowLevelAPI, LowLevelSDK,
};
use fluentbase_types::{Address, ExitCode, B256};

pub fn _wasm_create2(input: WasmCreate2MethodInput) -> Result<Address, ExitCode> {
    let value = U256::from_be_bytes(input.value32);
    let salt = B256::from(input.salt32);

    // TODO: "gas calculations"
    // TODO: "call depth stack check >= 1024"

    // check write protection
    if ExecutionContext::contract_is_static() {
        return Err(ExitCode::WriteProtection);
    }
    // read value input and contract address
    let caller_address = ExecutionContext::contract_caller();
    // load deployer and contract accounts
    let mut deployer_account = Account::new_from_jzkt(&caller_address);

    // calc keccak code hash (we need it for create2)
    let mut init_code_hash = B256::ZERO;
    LowLevelSDK::crypto_keccak256(
        input.code.as_ptr(),
        input.code.len() as u32,
        init_code_hash.as_mut_ptr(),
    );

    // create an account
    let mut contract_account =
        Account::create_account(&mut deployer_account, value, Some((salt, init_code_hash)))?;

    // translate WASM to rWASM
    let rwasm_bytecode = wasm2rwasm(&input.code).unwrap();

    // write deployer to the trie
    deployer_account.write_to_jzkt();

    // write contract to the trie
    contract_account.update_bytecode(&input.code.into(), None, &rwasm_bytecode.into(), None);
    rwasm_exec_hash(
        contract_account.rwasm_code_hash.as_slice(),
        &[],
        input.gas_limit,
        true,
    );

    Ok(contract_account.address)
}
