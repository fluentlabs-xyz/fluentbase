use crate::{
    account::Account,
    account_types::MAX_CODE_SIZE,
    helpers::{calc_create2_address, read_address_from_input, rwasm_exec, wasm2rwasm},
};
use alloc::{alloc::alloc, vec};
use core::alloc::Layout;
use fluentbase_sdk::{
    evm::{ContractInput, ExecutionContext, IContractInput, U256},
    LowLevelAPI,
    LowLevelSDK,
};
use fluentbase_types::{Bytes, ExitCode, B256};
use revm_interpreter::primitives::{alloy_primitives, Bytecode};

#[no_mangle]
pub fn _wasm_create2(
    value32_offset: *const u8,
    code_offset: *const u8,
    code_length: u32,
    salt32_offset: *const u8,
    gas_limit: u32,
    out_address20_offset: *mut u8,
) -> ExitCode {
    // TODO: "gas calculations"
    // TODO: "call depth stack check >= 1024"
    // check write protection
    if ExecutionContext::contract_is_static() {
        return ExitCode::WriteProtection;
    }
    // read value input and contract address
    let value32_slice = unsafe { &*core::ptr::slice_from_raw_parts(value32_offset, 32) };
    let salt32_slice = unsafe { &*core::ptr::slice_from_raw_parts(salt32_offset, 32) };
    let salt = B256::from_slice(salt32_slice);
    let value = U256::from_be_slice(value32_slice);
    let caller_address =
        read_address_from_input(<ContractInput as IContractInput>::ContractCaller::FIELD_OFFSET);
    // load deployer and contract accounts
    let mut deployer_account = Account::new_from_jzkt(&caller_address);
    let bytecode = unsafe { &*core::ptr::slice_from_raw_parts(code_offset, code_length as usize) };
    let bytecode_bytes = alloy_primitives::Bytes::from_static(bytecode);
    let deployer_bytecode = Bytecode::new_raw(bytecode_bytes);
    let deployed_contract_address = calc_create2_address(
        &caller_address,
        &salt,
        &B256::from_slice(deployer_bytecode.hash_slow().as_slice()),
    );
    let mut contract_account = Account::new_from_jzkt(&deployed_contract_address);
    // if nonce or code is not empty then its collision
    if contract_account.is_not_empty() {
        return ExitCode::CreateCollision;
    }
    deployer_account.inc_nonce();
    contract_account.nonce = 1;

    if !deployer_account.transfer_value(&mut contract_account, &value) {
        return ExitCode::InsufficientBalance;
    }

    let bytecode_rwasm = wasm2rwasm(bytecode, true);
    rwasm_exec(&bytecode_rwasm, &[], gas_limit, false);
    let source_bytecode_out_length = LowLevelSDK::sys_output_size();
    let mut source_bytecode_out = vec![0u8; source_bytecode_out_length as usize];
    LowLevelSDK::sys_read_output(
        source_bytecode_out.as_mut_ptr(),
        0,
        source_bytecode_out_length,
    );
    let bytecode_out = wasm2rwasm(&source_bytecode_out, false);
    deployer_account.write_to_jzkt();
    contract_account.update_source_bytecode(&source_bytecode_out.into());
    contract_account.update_bytecode(&bytecode_out.into());

    unsafe { core::ptr::copy(deployed_contract_address.as_ptr(), out_address20_offset, 20) }

    ExitCode::Ok
}
