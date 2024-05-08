use crate::helpers::{exec_evm_bytecode, exit_code_from_evm_error};
use crate::{debug_log, fluent_host::FluentHost, helpers::DefaultEvmSpec, result_value};
use alloc::boxed::Box;
use alloc::format;
use core::ascii::escape_default;
use core::ptr;
use fluentbase_sdk::{
    Account, AccountManager, ContextReader, EvmCallMethodInput, EvmCallMethodOutput, LowLevelAPI,
    LowLevelSDK,
};
use fluentbase_types::{Address, Bytes, ExitCode, U256};
use revm_interpreter::{
    analysis::to_analysed, opcode::make_instruction_table, primitives::Bytecode, return_ok,
    BytecodeLocked, Contract, InstructionResult, Interpreter, InterpreterAction, SharedMemory,
};
use revm_primitives::CreateScheme;

pub fn _evm_call<CR: ContextReader, AM: AccountManager>(
    cr: &CR,
    am: &AM,
    input: EvmCallMethodInput,
) -> EvmCallMethodOutput {
    debug_log!("_evm_call start");
    // for static calls passing value is not allowed according to standards
    let is_static = cr.contract_is_static();
    if is_static && input.value != U256::ZERO {
        return EvmCallMethodOutput::from_exit_code(ExitCode::WriteProtection)
            .with_gas(input.gas_limit);
    }

    // create new checkpoint position in the journal
    let checkpoint = am.checkpoint();

    // read caller and callee
    let (mut caller_account, _) = am.account(cr.contract_caller());
    let (mut callee_account, _) = am.account(cr.contract_address());

    // transfer funds from caller to callee
    match Account::transfer(&mut caller_account, &mut callee_account, input.value) {
        Ok(_) => {}
        Err(exit_code) => {
            debug_log!(
                "_evm_call return: Err: exit_code: {} caller.balance {} input.value {}",
                exit_code,
                caller_account.balance,
                input.value
            );
            return EvmCallMethodOutput::from_exit_code(exit_code).with_gas(input.gas_limit);
        }
    }

    // take right bytecode depending on context params
    let (source_hash, source_bytecode) = if input.callee != callee_account.address {
        let (code_account, _) = am.account(input.callee);
        (
            code_account.source_code_hash,
            am.preimage(&code_account.source_code_hash),
        )
    } else {
        (
            callee_account.source_code_hash,
            am.preimage(&callee_account.source_code_hash),
        )
    };
    // load bytecode and convert it to analysed (we can safely unwrap here)
    let bytecode =
        BytecodeLocked::try_from(to_analysed(Bytecode::new_raw(source_bytecode))).unwrap();

    // if bytecode is empty then commit result and return empty buffer
    if bytecode.is_empty() {
        // write account changes
        am.write_account(&caller_account);
        am.write_account(&callee_account);
        // commit journal
        am.commit();
        debug_log!("_evm_call return: exit_code: {}", ExitCode::Ok);
        return EvmCallMethodOutput::from_exit_code(ExitCode::Ok).with_gas(input.gas_limit);
    }

    // initiate contract instance and pass it to interpreter for and EVM transition
    let contract = Contract {
        input: input.input,
        hash: source_hash,
        bytecode,
        // we don't take contract callee, because callee refers to address with bytecode
        address: cr.contract_address(),
        caller: caller_account.address,
        value: input.value,
    };
    let result = exec_evm_bytecode(cr, am, contract, input.gas_limit, is_static);

    am.write_account(&caller_account);
    am.write_account(&callee_account);

    if matches!(result.result, return_ok!()) {
        am.commit();
    } else {
        am.rollback(checkpoint);
    }

    let exit_code = exit_code_from_evm_error(result.result);

    debug_log!("ecl(_evm_call) return exit_code={}", exit_code);
    EvmCallMethodOutput {
        output: result.output,
        exit_code: exit_code.into_i32(),
        gas: result.gas.remaining(),
    }
}
