use crate::{
    account::Account,
    account_types::MAX_CODE_SIZE,
    fluent_host::FluentHost,
    helpers::{calc_create2_address, read_address_from_input, DefaultEvmSpec},
};
use alloc::{alloc::alloc, boxed::Box};
use core::{alloc::Layout, ptr};
use fluentbase_sdk::{
    evm::{ContractInput, ExecutionContext, IContractInput, U256},
    LowLevelAPI,
    LowLevelSDK,
};
use fluentbase_types::{ExitCode, B256};
use revm_interpreter::{
    analysis::to_analysed,
    opcode::make_instruction_table,
    primitives::{Address, Bytecode, Bytes},
    BytecodeLocked,
    Contract,
    Interpreter,
    SharedMemory,
};

#[no_mangle]
pub fn _evm_create2(
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
    let value = U256::from_be_slice(value32_slice);
    let tx_caller_address =
        read_address_from_input(<ContractInput as IContractInput>::ContractCaller::FIELD_OFFSET);
    // load deployer and contract accounts
    let mut deployer_account = Account::new_from_jzkt(&tx_caller_address);
    let salt = B256::from_slice(salt32_slice);

    let deployer_bytecode_slice =
        unsafe { &*ptr::slice_from_raw_parts(code_offset, code_length as usize) };
    let deployer_bytecode_bytes = Bytes::from_static(deployer_bytecode_slice);
    let deployer_bytecode = to_analysed(Bytecode::new_raw(deployer_bytecode_bytes));
    let deployer_bytecode_locked = BytecodeLocked::try_from(deployer_bytecode).unwrap();
    let deployer_bytecode_hash = deployer_bytecode_locked.hash_slow();

    let deployed_contract_address = calc_create2_address(
        &tx_caller_address,
        &salt,
        &B256::from_slice(deployer_bytecode_hash.as_slice()),
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

    let contract = Contract {
        hash: deployer_bytecode_hash,
        bytecode: deployer_bytecode_locked,
        address: Address::new(deployed_contract_address.into_array()),
        caller: Address::new(tx_caller_address.into_array()),
        ..Default::default()
    };
    let mut interpreter = Interpreter::new(Box::new(contract), gas_limit as u64, false);
    let instruction_table = make_instruction_table::<FluentHost, DefaultEvmSpec>();
    let mut host = FluentHost::default();
    let shared_memory = SharedMemory::new();
    let interpreter_result = interpreter.run(shared_memory, &instruction_table, &mut host);
    let interpreter_result = if let Some(v) = interpreter_result.into_result_return() {
        v
    } else {
        return ExitCode::EVMCreateError;
    };
    if interpreter_result.is_error()
        || interpreter_result.is_revert()
        || !interpreter_result.is_ok()
    {
        return ExitCode::EVMCreateError;
    }
    assert!(interpreter_result.is_ok());
    let deployed_bytecode =
        fluentbase_types::Bytes::copy_from_slice(interpreter_result.output.iter().as_slice());

    deployer_account.write_to_jzkt();
    contract_account.update_source_bytecode(&deployed_bytecode);
    contract_account.update_bytecode(&include_bytes!("../../bin/evm_loader_contract.rwasm").into());

    // TODO convert deployed bytecode into rwasm code using evm translator and save result into

    ExitCode::Ok
}
