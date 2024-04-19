use crate::helpers::exec_evm_bytecode;
use crate::{account::Account, fluent_host::FluentHost, helpers::DefaultEvmSpec};
use alloc::boxed::Box;
use fluentbase_sdk::evm::ExecutionContext;
use fluentbase_sdk::{EvmCreateMethodInput, LowLevelAPI, LowLevelSDK};
use fluentbase_types::{Address, ExitCode, B256};
use revm_interpreter::{
    analysis::to_analysed,
    opcode::make_instruction_table,
    primitives::{Bytecode, Bytes},
    BytecodeLocked, Contract, Interpreter, SharedMemory, MAX_CODE_SIZE,
};
use revm_primitives::U256;

pub fn _evm_create(input: EvmCreateMethodInput) -> Result<Address, ExitCode> {
    // TODO: "gas calculations"
    // TODO: "load account so it needs to be marked as warm for access list"
    // TODO: "call depth stack check >= 1024"

    // check write protection
    let is_static = ExecutionContext::contract_is_static();
    if is_static {
        return Err(ExitCode::WriteProtection);
    }

    // load deployer and contract accounts
    let caller_address = ExecutionContext::contract_caller();
    let mut caller_account = Account::new_from_jzkt(&caller_address);

    let mut source_code_hash: B256 = B256::ZERO;
    LowLevelSDK::crypto_keccak256(
        input.init_code.as_ptr(),
        input.init_code.len() as u32,
        source_code_hash.as_mut_ptr(),
    );

    // create an account
    let mut callee_account = Account::create_account(
        &mut caller_account,
        input.value,
        input.salt.map(|salt| (salt, source_code_hash)),
    )?;

    let analyzed_bytecode = to_analysed(Bytecode::new_raw(input.init_code.into()));
    let deployer_bytecode_locked = BytecodeLocked::try_from(analyzed_bytecode).unwrap();
    let hash = deployer_bytecode_locked.hash_slow();

    let contract = Contract {
        input: Bytes::new(),
        bytecode: deployer_bytecode_locked,
        hash,
        address: callee_account.address,
        caller: caller_address,
        value: input.value,
    };

    let new_bytecode = exec_evm_bytecode(contract, input.gas_limit, is_static)?;
    if new_bytecode.len() > MAX_CODE_SIZE {
        return Err(ExitCode::ContractSizeLimit);
    }

    // write caller changes to database
    caller_account.write_to_jzkt();

    // write callee changes to database
    callee_account.update_bytecode(
        &new_bytecode,
        None,
        &include_bytes!("../../../contracts/assets/evm_loader_contract.rwasm").into(),
        None,
    );

    Ok(callee_account.address)
}
