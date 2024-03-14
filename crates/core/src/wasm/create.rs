use crate::{
    account::Account,
    helpers::{calc_create_address, read_address_from_input},
};
use alloc::vec;
use fluentbase_sdk::{
    evm::{ContractInput, ExecutionContext, IContractInput, U256},
    LowLevelAPI,
    LowLevelSDK,
};
use fluentbase_types::{Bytes, ExitCode, STATE_MAIN};
use rwasm_codegen::{Compiler, CompilerConfig, FuncOrExport, ImportLinker, ImportLinkerDefaults};

#[no_mangle]
pub fn _wasm_create(
    value32_offset: *const u8,
    code_offset: *const u8,
    code_length: u32,
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
    deployer_account.inc_nonce();
    contract_account.nonce = 1;

    if !deployer_account.transfer_value(&mut contract_account, &value) {
        return ExitCode::InsufficientBalance;
    }
    let deployer_wasm_bytecode_slice =
        unsafe { &*core::ptr::slice_from_raw_parts(code_offset, code_length as usize) };

    let mut import_linker = ImportLinker::default();
    ImportLinkerDefaults::new_v1alpha()
        .with_base_index(2)
        .register_import_funcs(&mut import_linker);
    let mut compiler = Compiler::new_with_linker(
        deployer_wasm_bytecode_slice,
        CompilerConfig::default(),
        Some(&import_linker),
    )
    .unwrap();
    compiler.translate(FuncOrExport::Export("deploy")).unwrap();
    let rwasm_bytecode = compiler.finalize().unwrap();
    let exit_code = LowLevelSDK::sys_exec(
        rwasm_bytecode.as_ptr(),
        rwasm_bytecode.len() as u32,
        core::ptr::null_mut(),
        0,
        core::ptr::null_mut(),
        0,
        &gas_limit as *const u32,
        STATE_MAIN,
    );
    if exit_code != 0 {
        panic!("failed to execute rwasm bytecode, exit code: {}", exit_code);
    }
    // read output bytecode
    let bytecode_length = LowLevelSDK::sys_output_size();
    let mut bytecode_out = vec![0u8; bytecode_length as usize];
    LowLevelSDK::sys_read(&mut bytecode_out, 0);

    let deployed_bytecode_bytes: Bytes = bytecode_out.into();
    deployer_account.write_to_jzkt();
    contract_account.update_source_bytecode(&deployed_bytecode_bytes);
    contract_account
        .update_bytecode(&include_bytes!("../../bin/wasm_loader_contract.rwasm").into());

    unsafe { core::ptr::copy(deployed_contract_address.as_ptr(), out_address20_offset, 20) }

    ExitCode::Ok
}
