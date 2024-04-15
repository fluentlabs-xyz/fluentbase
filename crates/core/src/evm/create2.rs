use crate::{account::Account, fluent_host::FluentHost, helpers::DefaultEvmSpec};
use alloc::boxed::Box;
use fluentbase_core_api::bindings::EvmCreate2MethodInput;
use fluentbase_sdk::{
    evm::{ExecutionContext, U256},
    LowLevelAPI, LowLevelSDK,
};
use fluentbase_types::{Address, Bytes, ExitCode, B256};
use revm_interpreter::{
    analysis::to_analysed, opcode::make_instruction_table, primitives::Bytecode, BytecodeLocked,
    Contract, Interpreter, SharedMemory, MAX_CODE_SIZE,
};

pub fn _evm_create2(input: EvmCreate2MethodInput) -> Result<Address, ExitCode> {
    let value = U256::from_be_bytes(input.value32);
    let salt = B256::from(input.salt32);

    // TODO: "gas calculations"
    // TODO: "load account so it needs to be marked as warm for access list"
    // TODO: "call depth stack check >= 1024"

    // check write protection
    let is_static = ExecutionContext::contract_is_static();
    if is_static {
        return Err(ExitCode::WriteProtection);
    }

    let mut init_code_hash = B256::ZERO;
    LowLevelSDK::crypto_keccak256(
        input.code.as_ptr(),
        input.code.len() as u32,
        init_code_hash.as_mut_ptr(),
    );

    // load deployer and contract accounts
    let caller_address = ExecutionContext::contract_caller();
    let mut caller_account = Account::new_from_jzkt(&caller_address);

    // create an account
    let mut callee_account =
        Account::create_account(&mut caller_account, value, Some((salt, init_code_hash)))?;

    let gas_limit = input.gas_limit;
    let analyzed_bytecode = to_analysed(Bytecode::new_raw(input.code.into()));
    let deployer_bytecode_locked = BytecodeLocked::try_from(analyzed_bytecode).unwrap();

    let contract = Contract {
        input: Bytes::new(),
        bytecode: deployer_bytecode_locked,
        hash: init_code_hash,
        address: callee_account.address,
        caller: caller_address,
        value,
    };
    let mut interpreter = Interpreter::new(Box::new(contract), gas_limit as u64, false);
    let instruction_table = make_instruction_table::<FluentHost, DefaultEvmSpec>();
    let mut host = FluentHost::default();
    let shared_memory = SharedMemory::new();
    let result = if let Some(v) = interpreter
        .run(shared_memory, &instruction_table, &mut host)
        .into_result_return()
    {
        v
    } else {
        return Err(ExitCode::EVMCreateError);
    };

    if result.is_error() {
        return Err(ExitCode::EVMCreateError);
    } else if result.is_revert() {
        return Err(ExitCode::EVMCreateRevert);
    }

    if result.output.len() > MAX_CODE_SIZE {
        return Err(ExitCode::ContractSizeLimit);
    }

    caller_account.write_to_jzkt();

    callee_account.update_bytecode(
        &result.output,
        None,
        &include_bytes!("../../../contracts/assets/evm_loader_contract.rwasm").into(),
        None,
    );

    Ok(callee_account.address)
}
