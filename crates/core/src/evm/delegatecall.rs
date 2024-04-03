use crate::{account::Account, fluent_host::FluentHost, helpers::DefaultEvmSpec};
use alloc::boxed::Box;
use core::ptr;
use fluentbase_sdk::{evm::ExecutionContext, LowLevelAPI, LowLevelSDK};
use fluentbase_types::{Address, ExitCode};
use revm_interpreter::{
    analysis::to_analysed,
    opcode::make_instruction_table,
    primitives::Bytecode,
    BytecodeLocked,
    Contract,
    Interpreter,
    SharedMemory,
};

#[no_mangle]
pub fn _evm_delegatecall(
    gas_limit: u32,
    callee20_offset: *const u8,
    args_offset: *const u8,
    args_size: u32,
    ret_offset: *mut u8,
    ret_size: u32,
) -> ExitCode {
    // for static calls passing value is not allowed according to standards
    let is_static = ExecutionContext::contract_is_static();
    if is_static {
        return ExitCode::WriteProtection;
    }
    // read callee address based on the pass parameter
    let callee_address =
        Address::from_slice(unsafe { &*ptr::slice_from_raw_parts(callee20_offset, 20) });
    let callee_account = Account::new_from_jzkt(&callee_address);
    // load bytecode and convert it to analysed (yes, too slow)
    let bytecode = BytecodeLocked::try_from(to_analysed(Bytecode::new_raw(
        callee_account.load_source_bytecode(),
    )))
    .unwrap();
    // initiate contract instance and pass it to interpreter for and EVM transition
    let contract = Contract {
        input: unsafe { &*ptr::slice_from_raw_parts(args_offset, args_size as usize) }.into(),
        hash: callee_account.source_bytecode_hash,
        bytecode,
        address: callee_address,
        caller: ExecutionContext::contract_caller(),
        value: ExecutionContext::contract_value(),
    };
    let mut interpreter = Interpreter::new(Box::new(contract), gas_limit as u64, is_static);
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
    // write execution output
    let output = result.output;
    LowLevelSDK::sys_write(&output);
    if ret_size > 0 {
        let ret_size_actual = core::cmp::min(output.len(), ret_size as usize);
        unsafe { ptr::copy(output.as_ptr(), ret_offset, ret_size_actual) };
    }
    // map execution result into error exit code
    exit_code
}
