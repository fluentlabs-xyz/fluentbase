use crate::{account::Account, fluent_host::FluentHost, helpers::DefaultEvmSpec};
use alloc::boxed::Box;
use core::ptr;
use fluentbase_sdk::{
    evm::{ExecutionContext, U256},
    LowLevelAPI,
    LowLevelSDK,
};
use fluentbase_types::ExitCode;
use revm_interpreter::{
    analysis::to_analysed,
    opcode::make_instruction_table,
    primitives::{Address, Bytecode, Bytes, B256},
    BytecodeLocked,
    Contract,
    Interpreter,
    SharedMemory,
};

#[no_mangle]
pub fn _evm_call(
    gas_limit: u32,
    callee_address20_offset: *const u8,
    value32_offset: *const u8,
    args_offset: *const u8,
    args_size: u32,
    ret_offset: *mut u8,
    ret_size: u32,
) -> ExitCode {
    let value = U256::from_be_slice(unsafe { &*ptr::slice_from_raw_parts(value32_offset, 32) });
    let is_static = ExecutionContext::contract_is_static();
    if is_static && value != U256::ZERO {
        return ExitCode::WriteProtection;
    }

    let callee_address =
        Address::from_slice(unsafe { &*ptr::slice_from_raw_parts(callee_address20_offset, 20) });

    let caller_address = ExecutionContext::contract_caller();
    let callee_account = Account::new_from_jzkt(&fluentbase_types::Address::from_slice(
        callee_address.as_slice(),
    ));

    let source_code_hash = callee_account.source_bytecode_hash.as_slice();
    let source_bytecode = callee_account.load_source_bytecode();
    if value != U256::ZERO {
        return ExitCode::UnknownError;
    };
    let args = unsafe { &*ptr::slice_from_raw_parts(args_offset, args_size as usize) };
    let contract = Contract {
        input: args.into(),
        hash: B256::from_slice(source_code_hash),
        // TODO simplify
        bytecode: BytecodeLocked::try_from(to_analysed(Bytecode::new_raw(Bytes::copy_from_slice(
            source_bytecode.as_ref(),
        ))))
        .unwrap(),
        address: Address::new(callee_account.address.into_array()),
        caller: Address::new(caller_address.into_array()),
        value,
    };
    let mut interpreter = Interpreter::new(Box::new(contract), gas_limit as u64, is_static);
    let instruction_table = make_instruction_table::<FluentHost, DefaultEvmSpec>();
    let mut host = FluentHost::default();
    let shared_memory = SharedMemory::new();
    let interpreter_result = interpreter.run(shared_memory, &instruction_table, &mut host);
    let interpreter_result = if let Some(v) = interpreter_result.into_result_return() {
        v
    } else {
        return ExitCode::EVMCallError;
    };
    if interpreter_result.is_error()
        || interpreter_result.is_revert()
        || !interpreter_result.is_ok()
    {
        return ExitCode::EVMCallError;
    }
    let output = interpreter_result.output;
    LowLevelSDK::sys_write(&output);
    if ret_size > 0 {
        let ret_size_actual = core::cmp::min(output.len(), ret_size as usize);
        unsafe { ptr::copy(output.as_ptr(), ret_offset, ret_size_actual) };
    }

    ExitCode::Ok
}
