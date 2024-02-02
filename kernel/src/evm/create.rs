use crate::evm::{calc_create_address, read_input_address, Account, MAX_CODE_SIZE};
use alloc::alloc::alloc;
use core::{alloc::Layout, ptr};
use fluentbase_sdk::{
    evm::{ContractInput, ExecutionContext, IContractInput, U256},
    LowLevelAPI,
    LowLevelSDK,
};
use fluentbase_types::ExitCode;

#[no_mangle]
pub fn _evm_create(
    value32_offset: *const u8,
    code_offset: *const u8,
    code_length: u32,
    output20_offset: *mut u8,
    gas_limit: u32,
) -> i32 {
    // check write protection
    if ExecutionContext::contract_is_static() {
        return ExitCode::WriteProtection.into_i32();
    }
    // read value input and contract address
    let value = U256::from_be_slice(unsafe { &*ptr::slice_from_raw_parts(value32_offset, 32) });
    let contract_address =
        read_input_address(<ContractInput as IContractInput>::ContractAddress::FIELD_OFFSET);
    // load deployer and contract accounts
    let mut deployer = Account::read_account(&contract_address);
    let created_address = calc_create_address(&contract_address, deployer.nonce);
    let mut contract = Account::read_account(&created_address);
    // if nonce or code is not empty then its collision
    if contract.is_not_empty() {
        return ExitCode::CreateCollision.into_i32();
    }
    contract.nonce = 1;
    // transfer value to the just created account
    if !deployer.transfer_value(&mut contract, &value) {
        return ExitCode::InsufficientBalance.into_i32();
    }
    // execute deployer bytecode
    LowLevelSDK::sys_exec(
        code_offset,
        code_length,
        ptr::null(),
        0,
        ptr::null_mut(),
        0,
        gas_limit,
    );
    // read output bytecode
    let bytecode_length = LowLevelSDK::sys_output_size();
    if bytecode_length > MAX_CODE_SIZE {
        return ExitCode::ContractSizeLimit.into_i32();
    }
    let bytecode = unsafe {
        alloc(Layout::from_size_align_unchecked(
            bytecode_length as usize,
            1,
        ))
    };
    LowLevelSDK::sys_read_output(bytecode, 0, bytecode_length);
    // calc keccak256 and poseidon hashes for account
    LowLevelSDK::crypto_keccak256(
        code_offset,
        code_length,
        contract.keccak_code_hash.as_mut_ptr(),
    );
    LowLevelSDK::crypto_poseidon(code_offset, code_length, contract.code_hash.as_mut_ptr());
    // commit account changes
    contract.commit(&created_address);
    // copy result address to output and return ok
    unsafe { ptr::copy(created_address.as_ptr(), output20_offset, 20) }
    ExitCode::Ok.into_i32()
}
