use crate::helpers::exec_evm_bytecode;
use crate::{account::Account, fluent_host::FluentHost, helpers::DefaultEvmSpec};
use alloc::boxed::Box;
use core::ptr;
use fluentbase_sdk::{
    evm::{ExecutionContext, U256},
    EvmCallMethodInput, LowLevelAPI, LowLevelSDK,
};
use fluentbase_types::{Address, Bytes, ExitCode};
use revm_interpreter::{
    analysis::to_analysed, opcode::make_instruction_table, primitives::Bytecode, BytecodeLocked,
    Contract, InstructionResult, Interpreter, InterpreterAction, SharedMemory,
};
use revm_primitives::CreateScheme;

pub fn _evm_call(input: EvmCallMethodInput) -> Result<Bytes, ExitCode> {
    // TODO(dmitry123): "implement nested call depth checks"

    // for static calls passing value is not allowed according to standards
    let is_static = ExecutionContext::contract_is_static();
    if is_static && input.value != U256::ZERO {
        return Err(ExitCode::WriteProtection);
    }
    // read caller address from execution context
    let caller_address = ExecutionContext::contract_caller();
    // read callee address based on the pass parameter
    let callee_account = Account::new_from_jzkt(&input.callee);
    // load bytecode and convert it to analysed (yes, too slow)
    let bytecode = BytecodeLocked::try_from(to_analysed(Bytecode::new_raw(
        callee_account.load_source_bytecode(),
    )))
    .unwrap();
    let gas_limit = input.gas_limit as u64;
    // initiate contract instance and pass it to interpreter for and EVM transition
    let contract = Contract {
        input: input.input,
        hash: callee_account.source_code_hash,
        bytecode,
        address: input.callee,
        caller: caller_address,
        value: input.value,
    };
    exec_evm_bytecode(contract, gas_limit, is_static)
}
