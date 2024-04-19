use alloc::boxed::Box;
use core::ptr;

use fluentbase_core_api::bindings::EvmMethodName::EvmCall;
use fluentbase_core_api::bindings::{
    EvmCallMethodInput, EvmCreate2MethodInput, EvmCreateMethodInput,
};
use revm_interpreter::{
    analysis::to_analysed, opcode::make_instruction_table, primitives::Bytecode, BytecodeLocked,
    Contract, InstructionResult, Interpreter, InterpreterAction, SharedMemory,
};
use revm_primitives::CreateScheme;

use fluentbase_sdk::{
    evm::{ExecutionContext, U256},
    LowLevelAPI, LowLevelSDK,
};
use fluentbase_types::{Address, Bytes, ExitCode};

use crate::evm::create::_evm_create;
use crate::evm::create2::_evm_create2;
use crate::helpers::exec_evm_bytecode;
use crate::{account::Account, fluent_host::FluentHost, helpers::DefaultEvmSpec};

pub fn _evm_call(input: EvmCallMethodInput) -> Result<Bytes, ExitCode> {
    // TODO(dmitry123): "implement nested call depth checks"

    let value = U256::from_be_bytes(input.value32);
    // for static calls passing value is not allowed according to standards
    let is_static = ExecutionContext::contract_is_static();
    if is_static && value != U256::ZERO {
        return Err(ExitCode::WriteProtection);
    }
    // read caller address from execution context
    let caller_address = ExecutionContext::contract_caller();
    // read callee address based on the pass parameter
    let callee_address = Address::from(input.callee_address20);
    let callee_account = Account::new_from_jzkt(&callee_address);
    // load bytecode and convert it to analysed (yes, too slow)
    let bytecode = BytecodeLocked::try_from(to_analysed(Bytecode::new_raw(
        callee_account.load_source_bytecode(),
    )))
    .unwrap();
    let gas_limit = input.gas_limit as u64;
    // initiate contract instance and pass it to interpreter for and EVM transition
    let contract = Contract {
        input: input.args.into(),
        hash: callee_account.source_code_hash,
        bytecode,
        address: callee_address,
        caller: caller_address,
        value,
    };
    exec_evm_bytecode(contract, gas_limit, is_static)
}
