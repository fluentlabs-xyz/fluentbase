use alloc::format;
use alloc::string::ToString;

use revm_interpreter::{analysis::to_analysed, primitives::Bytecode, BytecodeLocked, Contract};

use fluentbase_sdk::{
    evm::{ExecutionContext, U256},
    EvmCallMethodInput, LowLevelAPI,
};
use fluentbase_types::{Bytes, ExitCode};

use crate::account::Account;
use crate::helpers::{debug_log, exec_evm_bytecode};
use crate::result_value;

pub fn _evm_call(input: EvmCallMethodInput) -> Result<Bytes, ExitCode> {
    debug_log("_evm_call start");
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
    let gas_limit = input.gas_limit;
    // initiate contract instance and pass it to interpreter for and EVM transition
    let contract = Contract {
        input: input.input,
        hash: callee_account.source_code_hash,
        bytecode,
        address: input.callee,
        caller: caller_address,
        value: input.value,
    };
    let res = exec_evm_bytecode(contract, gas_limit, is_static);
    debug_log(&format!(
        "_evm_call return: {}",
        result_value!(res
            .as_ref()
            .map(|v| { format!("Ok: len {}", v.len()) })
            .map_err(|v| { format!("Err: ExitCode: {}", v) }))
    ));
    res
}
