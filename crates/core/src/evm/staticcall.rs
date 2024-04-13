use crate::{account::Account, fluent_host::FluentHost, helpers::DefaultEvmSpec};
use alloc::boxed::Box;
use core::ptr;
use fluentbase_sdk::{
    evm::{ExecutionContext, U256},
    LowLevelAPI, LowLevelSDK,
};
use fluentbase_types::{Address, ExitCode};
use revm_interpreter::{
    analysis::to_analysed, opcode::make_instruction_table, primitives::Bytecode, BytecodeLocked,
    Contract, Interpreter, SharedMemory,
};

pub fn _evm_staticcall(
    gas_limit: u32,
    callee_address20_offset: *const u8,
    args_offset: *const u8,
    args_size: u32,
    ret_offset: *mut u8,
    ret_size: u32,
) -> ExitCode {
    let value = U256::ZERO;
    let callee_address =
        Address::from_slice(unsafe { &*ptr::slice_from_raw_parts(callee_address20_offset, 20) });
    let callee_account = Account::new_from_jzkt(&callee_address);
    let caller_address = ExecutionContext::contract_caller();
    let bytecode = BytecodeLocked::try_from(to_analysed(Bytecode::new_raw(
        callee_account.load_source_bytecode(),
    )))
    .unwrap();
    let contract = Contract {
        input: unsafe { &*ptr::slice_from_raw_parts(args_offset, args_size as usize) }.into(),
        hash: callee_account.source_code_hash,
        bytecode,
        address: callee_address,
        caller: caller_address,
        value,
    };
    let mut interpreter = Interpreter::new(Box::new(contract), gas_limit as u64, true);
    let instruction_table = make_instruction_table::<FluentHost, DefaultEvmSpec>();
    let mut host = FluentHost::default();
    let shared_memory = SharedMemory::new();
    let result = match interpreter
        .run(shared_memory, &instruction_table, &mut host)
        .into_result_return()
    {
        Some(v) => v,
        None => return ExitCode::EVMCallError,
    };
    let exit_code = if result.is_error() {
        ExitCode::EVMCallError
    } else if result.is_revert() {
        ExitCode::EVMCallRevert
    } else {
        ExitCode::Ok
    };
    let output = result.output;
    LowLevelSDK::sys_write(&output);
    if ret_size > 0 {
        let ret_size_actual = core::cmp::min(output.len(), ret_size as usize);
        unsafe { ptr::copy(output.as_ptr(), ret_offset, ret_size_actual) };
    }
    // map execution result into error exit code
    exit_code
}
