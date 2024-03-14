use crate::{
    account::Account,
    account_types::MAX_CODE_SIZE,
    helpers::{calc_create2_address, read_address_from_input},
};
use alloc::alloc::alloc;
use core::{alloc::Layout, ptr};
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
    out_address20_offset: *mut u8,
    gas_limit: u32,
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
    let caller_address =
        read_address_from_input(<ContractInput as IContractInput>::ContractCaller::FIELD_OFFSET);
    // load deployer and contract accounts
    let mut deployer_account = Account::new_from_jzkt(&caller_address);
    let bytecode_slice = unsafe { &*ptr::slice_from_raw_parts(code_offset, code_length as usize) };
    let bytecode_bytes = alloy_primitives::Bytes::from_static(bytecode_slice);
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
    // transfer value to the just created account
    if !deployer_account.transfer_value(&mut contract_account, &value) {
        return ExitCode::InsufficientBalance;
    }

    deployer_account.write_to_jzkt();
    contract_account.update_source_bytecode(&Bytes::from_static(bytecode_slice));
    contract_account
        .update_bytecode(&include_bytes!("../../bin/wasm_loader_contract.rwasm").into());

    // read output bytecode
    let bytecode_length = LowLevelSDK::sys_output_size();
    if bytecode_length > MAX_CODE_SIZE {
        return ExitCode::ContractSizeLimit;
    }
    let bytecode = unsafe {
        alloc(Layout::from_size_align_unchecked(
            bytecode_length as usize,
            8,
        ))
    };
    LowLevelSDK::sys_read_output(bytecode, 0, bytecode_length);

    unsafe { ptr::copy(deployed_contract_address.as_ptr(), out_address20_offset, 20) }

    ExitCode::Ok
}
