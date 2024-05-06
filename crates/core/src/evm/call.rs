use crate::helpers::{debug_log, exec_evm_bytecode, exit_code_from_evm_error};
use crate::{account::Account, fluent_host::FluentHost, helpers::DefaultEvmSpec, result_value};
use alloc::boxed::Box;
use alloc::format;
use core::ascii::escape_default;
use core::ptr;
use fluentbase_sdk::{
    evm::ExecutionContext, EvmCallMethodInput, EvmCallMethodOutput, LowLevelAPI, LowLevelSDK,
};
use fluentbase_types::{Address, Bytes, ExitCode, U256};
use revm_interpreter::instructions::host::call;
use revm_interpreter::{
    analysis::to_analysed, opcode::make_instruction_table, primitives::Bytecode, return_ok,
    BytecodeLocked, Contract, InstructionResult, Interpreter, InterpreterAction, SharedMemory,
};
use revm_primitives::CreateScheme;

pub fn _evm_call(input: EvmCallMethodInput) -> EvmCallMethodOutput {
    debug_log("_evm_call start");
    // for static calls passing value is not allowed according to standards
    let is_static = ExecutionContext::contract_is_static();
    if is_static && input.value != U256::ZERO {
        debug_log(&format!(
            "_evm_call return: exit_code: {}",
            ExitCode::WriteProtection
        ));
        return EvmCallMethodOutput::from_exit_code(ExitCode::WriteProtection)
            .with_gas(input.gas_limit);
    }

    // create new checkpoint position in the journal
    let checkpoint = Account::checkpoint();

    // read caller and callee
    let mut caller_account = Account::new_from_jzkt(ExecutionContext::contract_caller());
    let mut callee_account = Account::new_from_jzkt(input.callee);

    // transfer funds from caller to callee
    match Account::transfer(&mut caller_account, &mut callee_account, input.value) {
        Ok(_) => {}
        Err(exit_code) => {
            debug_log(&format!("_evm_call return: Err: exit_code: {}", exit_code));
            return EvmCallMethodOutput::from_exit_code(exit_code).with_gas(input.gas_limit);
        }
    }

    // load bytecode and convert it to analysed (yes, too slow)
    let bytecode = BytecodeLocked::try_from(to_analysed(Bytecode::new_raw(
        callee_account.load_source_bytecode(),
    )))
    .unwrap();
    let gas_limit = input.gas_limit;

    // if bytecode is empty then commit result and return empty buffer
    if bytecode.is_empty() {
        Account::commit();
        debug_log(&format!("_evm_call return: exit_code: {}", ExitCode::Ok));
        return EvmCallMethodOutput::from_exit_code(ExitCode::Ok).with_gas(input.gas_limit);
    }

    // initiate contract instance and pass it to interpreter for and EVM transition
    let contract = Contract {
        input: input.input,
        hash: callee_account.source_code_hash,
        bytecode,
        address: ExecutionContext::contract_address(),
        caller: caller_account.address,
        value: input.value,
    };
    let result = exec_evm_bytecode(contract, gas_limit, is_static);

    // debug_log(&format!(
    //     "_evm_call return: {}",
    //     result_value!(result
    //         .as_ref()
    //         .map(|v| { format!("Ok: len {}", v.len()) })
    //         .map_err(|v| { format!("Err: ExitCode: {}", v) }))
    // ));

    caller_account.write_to_jzkt();
    callee_account.write_to_jzkt();

    if matches!(result.result, return_ok!()) {
        Account::commit();
    } else {
        Account::rollback(checkpoint);
    }

    let exit_code = exit_code_from_evm_error(result.result);

    debug_log(&format!("ecl(_evm_call) return exit_code={}", exit_code));
    EvmCallMethodOutput {
        output: result.output,
        exit_code: exit_code.into_i32(),
        gas: result.gas.remaining(),
    }
}
