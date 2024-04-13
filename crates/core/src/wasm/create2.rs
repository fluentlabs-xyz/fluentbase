use crate::{
    account::Account,
    helpers::{calc_create2_address, rwasm_exec_hash, wasm2rwasm},
};
use core::ptr;
use fluentbase_sdk::{
    evm::{ExecutionContext, U256},
    LowLevelAPI, LowLevelSDK,
};
use fluentbase_types::{ExitCode, B256};

#[no_mangle]
pub fn _wasm_create2(
    value32_offset: *const u8,
    code_offset: *const u8,
    code_length: u32,
    salt32_offset: *const u8,
    gas_limit: u32,
    address20_offset: *mut u8,
) -> ExitCode {
    // TODO: "gas calculations"
    // TODO: "call depth stack check >= 1024"
    // check write protection
    if ExecutionContext::contract_is_static() {
        return ExitCode::WriteProtection;
    }
    // read value input and contract address
    let value32_slice = unsafe { &*ptr::slice_from_raw_parts(value32_offset, 32) };
    let salt32_slice = unsafe { &*ptr::slice_from_raw_parts(salt32_offset, 32) };
    let salt = B256::from_slice(salt32_slice);
    let value = U256::from_be_slice(value32_slice);
    let caller_address = ExecutionContext::contract_caller();
    // load deployer and contract accounts
    let mut deployer_account = Account::new_from_jzkt(&caller_address);

    let init_code = unsafe { &*ptr::slice_from_raw_parts(code_offset, code_length as usize) };
    let mut init_code_hash = B256::ZERO;
    LowLevelSDK::crypto_keccak256(
        init_code.as_ptr(),
        init_code.len() as u32,
        init_code_hash.as_mut_ptr(),
    );

    let deployed_contract_address = calc_create2_address(&caller_address, &salt, &init_code_hash);
    let mut contract_account = Account::new_from_jzkt(&deployed_contract_address);
    // if nonce or code is not empty then its collision
    if contract_account.is_not_empty() {
        return ExitCode::CreateCollision;
    }
    deployer_account.inc_nonce().expect("nonce inc failed");
    contract_account.nonce = 1;

    if !deployer_account.transfer_value(&mut contract_account, &value) {
        return ExitCode::InsufficientBalance;
    }

    // translate WASM to rWASM
    let bytecode_rwasm = wasm2rwasm(init_code).unwrap();

    // write deployer to the trie
    deployer_account.write_to_jzkt();

    // write contract to the trie
    contract_account.update_bytecode(&init_code.into(), None, &bytecode_rwasm.into(), None);
    rwasm_exec_hash(
        contract_account.rwasm_code_hash.as_slice(),
        &[],
        gas_limit,
        true,
    );

    // copy output address
    unsafe { core::ptr::copy(deployed_contract_address.as_ptr(), address20_offset, 20) }

    ExitCode::Ok
}
