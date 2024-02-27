use crate::{
    account::Account,
    account_types::MAX_CODE_SIZE,
    evm::{calc_create_address, read_address_from_input},
};
use alloc::{alloc::alloc, boxed::Box};
use core::{alloc::Layout, ptr};
use fluentbase_sdk::{
    evm::{ContractInput, ExecutionContext, IContractInput, U256},
    LowLevelAPI,
    LowLevelSDK,
};
use fluentbase_types::ExitCode;
use revm_interpreter::{
    analysis::to_analysed,
    opcode::make_instruction_table,
    primitives::{Address, Bytecode, Bytes, ShanghaiSpec},
    BytecodeLocked,
    Contract,
    DummyHost,
    Interpreter,
    SharedMemory,
};

#[no_mangle]
pub fn _evm_create(
    value32_offset: *const u8,
    code_offset: *const u8,
    code_length: u32,
    output20_offset: *mut u8,
    gas_limit: u32,
) -> ExitCode {
    // TODO: "gas calculations"
    // TODO: "call depth stack check >= 1024"
    // check write protection
    if ExecutionContext::contract_is_static() {
        return ExitCode::WriteProtection;
    }
    // read value input and contract address
    let value = U256::from_be_slice(unsafe { &*ptr::slice_from_raw_parts(value32_offset, 32) });
    let tx_caller_address =
        read_address_from_input(<ContractInput as IContractInput>::TxCaller::FIELD_OFFSET);
    // let contract_address =
    //     read_input_address(<ContractInput as IContractInput>::ContractAddress::FIELD_OFFSET);
    // load deployer and contract accounts
    let mut deployer_account = Account::new_from_jzkt(&tx_caller_address);
    let deployed_contract_address = calc_create_address(&tx_caller_address, deployer_account.nonce);
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
    let deployer_evm_bytecode_slice =
        unsafe { &*ptr::slice_from_raw_parts(code_offset, code_length as usize) };
    let deployer_evm_bytecode_bytes = Bytes::from_static(deployer_evm_bytecode_slice);
    let deployer_evm_bytecode = to_analysed(Bytecode::new_raw(deployer_evm_bytecode_bytes));
    let deployer_evm_bytecode_locked = BytecodeLocked::try_from(deployer_evm_bytecode).unwrap();

    let contract = Contract {
        input: Bytes::new(),
        hash: deployer_evm_bytecode_locked.hash_slow(),
        bytecode: deployer_evm_bytecode_locked,
        address: Address::new(deployed_contract_address.into_array()),
        caller: Address::new(tx_caller_address.into_array()),
        value: U256::ZERO,
    };
    let mut evm_interpreter = Interpreter::new(Box::new(contract), gas_limit as u64, false);
    let instruction_table = make_instruction_table::<DummyHost, ShanghaiSpec>();
    let mut host = DummyHost::default();
    let shared_memory = SharedMemory::new();
    let evm_run_result = evm_interpreter.run(shared_memory, &instruction_table, &mut host);
    let interpreter_result = if let Some(v) = evm_run_result.into_result_return() {
        v
    } else {
        return ExitCode::CreateError;
    };
    if interpreter_result.is_error()
        || interpreter_result.is_revert()
        || !interpreter_result.is_ok()
    {
        return ExitCode::CreateError;
    }
    assert!(interpreter_result.is_ok());
    let deployed_evm_bytecode =
        fluentbase_types::Bytes::copy_from_slice(interpreter_result.output.iter().as_slice());

    // save $deployed_evm_bytecode into account
    let mut deployed_account = Account::new(&deployed_contract_address);
    deployed_account.update_source_bytecode(&deployed_evm_bytecode);

    // TODO convert $deployed_evm_bytecode into rwasm code ($deployed_rwasm_bytecode)
    // TODO save $deployed_rwasm_bytecode into account (with its poseidon hash)

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
    // calc keccak256 and poseidon hashes for account
    // LowLevelSDK::crypto_keccak256(
    //     code_offset,
    //     code_length,
    //     contract.keccak_code_hash.as_mut_ptr(),
    // );
    // LowLevelSDK::crypto_poseidon(code_offset, code_length, contract.code_hash.as_mut_ptr());
    // commit account changes
    // contract.commit(&created_address);
    // copy result address to output and return ok
    unsafe { ptr::copy(deployed_contract_address.as_ptr(), output20_offset, 20) }

    ExitCode::Ok
}
