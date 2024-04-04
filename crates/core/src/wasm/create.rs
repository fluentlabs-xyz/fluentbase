use crate::{
    account::Account,
    helpers::{calc_create_address, read_address_from_input, rwasm_exec_hash, wasm2rwasm},
};
use fluentbase_sdk::evm::{ContractInput, ExecutionContext, IContractInput, U256};
use fluentbase_types::ExitCode;
use revm_primitives::RWASM_MAX_CODE_SIZE;

#[no_mangle]
pub fn _wasm_create(
    value32_offset: *const u8,
    code_offset: *const u8,
    code_length: u32,
    gas_limit: u32,
    address20_offset: *mut u8,
) -> ExitCode {
    // TODO: "gas calculations"
    // TODO: "call depth stack check >= 1024"
    // check write protection
    if ExecutionContext::contract_is_static() {
        return ExitCode::WriteProtection;
    }
    // code length can't exceed max limit
    if code_length > RWASM_MAX_CODE_SIZE as u32 {
        return ExitCode::ContractSizeLimit;
    }

    // read value input and contract address
    let value32_slice = unsafe { &*core::ptr::slice_from_raw_parts(value32_offset, 32) };
    let value = U256::from_be_slice(value32_slice);
    let caller_address =
        read_address_from_input(<ContractInput as IContractInput>::ContractCaller::FIELD_OFFSET);
    // load deployer and contract accounts
    let mut deployer_account = Account::new_from_jzkt(&caller_address);
    let deployed_contract_address = calc_create_address(&caller_address, deployer_account.nonce);
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
    let bytecode_wasm =
        unsafe { &*core::ptr::slice_from_raw_parts(code_offset, code_length as usize) };
    let bytecode_rwasm = wasm2rwasm(bytecode_wasm).unwrap();

    // write deployer to the trie
    deployer_account.write_to_jzkt();

    // write contract to the trie
    contract_account.update_source_bytecode(&bytecode_wasm.into());
    contract_account.update_rwasm_bytecode(&bytecode_rwasm.into());
    rwasm_exec_hash(
        &contract_account.rwasm_bytecode_hash.as_slice(),
        &[],
        gas_limit,
        true,
    );

    // copy output address
    unsafe { core::ptr::copy(deployed_contract_address.as_ptr(), address20_offset, 20) }

    ExitCode::Ok
}
