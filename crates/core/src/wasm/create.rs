use crate::{
    account::Account,
    helpers::{calc_create_address, rwasm_exec_hash, wasm2rwasm},
};
use fluentbase_sdk::evm::ExecutionContext;
use fluentbase_types::{Address, ExitCode, U256};
use revm_primitives::RWASM_MAX_CODE_SIZE;

pub fn _wasm_create(
    value32_offset: *const u8,
    code_offset: *const u8,
    code_length: u32,
    gas_limit: u32,
) -> Result<Address, ExitCode> {
    // TODO: "gas calculations"
    // TODO: "call depth stack check >= 1024"
    // check write protection
    if ExecutionContext::contract_is_static() {
        return Err(ExitCode::WriteProtection);
    }
    // code length can't exceed max limit
    if code_length > RWASM_MAX_CODE_SIZE as u32 {
        return Err(ExitCode::ContractSizeLimit);
    }

    // read value input and contract address
    let value32_slice = unsafe { &*core::ptr::slice_from_raw_parts(value32_offset, 32) };
    let value = U256::from_be_slice(value32_slice);
    let caller_address = ExecutionContext::contract_caller();
    // load deployer and contract accounts
    let mut deployer_account = Account::new_from_jzkt(&caller_address);
    let deployed_contract_address = calc_create_address(&caller_address, deployer_account.nonce);
    let mut contract_account = Account::new_from_jzkt(&deployed_contract_address);

    // if nonce or code is not empty then its collision
    if contract_account.is_not_empty() {
        return Err(ExitCode::CreateCollision);
    }
    deployer_account.inc_nonce().expect("nonce inc failed");
    contract_account.nonce = 1;

    if !deployer_account.transfer_value(&mut contract_account, &value) {
        return Err(ExitCode::InsufficientBalance);
    }

    // translate WASM to rWASM
    let wasm_bytecode =
        unsafe { &*core::ptr::slice_from_raw_parts(code_offset, code_length as usize) };
    let rwasm_bytecode = wasm2rwasm(wasm_bytecode).unwrap();

    // write deployer to the trie
    deployer_account.write_to_jzkt();

    // write contract to the trie
    contract_account.update_bytecode(&wasm_bytecode.into(), None, &rwasm_bytecode.into(), None);
    let exit_code = rwasm_exec_hash(
        &contract_account.rwasm_code_hash.as_slice(),
        &[],
        gas_limit,
        true,
    );
    // if call is not success set deployed address to zero
    if exit_code != ExitCode::Ok.into_i32() {
        return Err(ExitCode::TransactError);
    }

    Ok(deployed_contract_address)
}
