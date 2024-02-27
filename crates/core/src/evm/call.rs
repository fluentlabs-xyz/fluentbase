use crate::{account::Account, evm::read_address_from_input, fluent_host::FluentHost};
use alloc::boxed::Box;
use core::ptr;
use fluentbase_sdk::{
    evm::{ContractInput, ExecutionContext, IContractInput, U256},
    LowLevelAPI,
    LowLevelSDK,
};
use fluentbase_types::ExitCode;
use revm_interpreter::{
    analysis::to_analysed,
    opcode::make_instruction_table,
    primitives::{Address, Bytecode, Bytes, ShanghaiSpec, B256},
    BytecodeLocked,
    Contract,
    Interpreter,
    SharedMemory,
};

#[no_mangle]
pub fn _evm_call(
    gas_limit: u32,
    address20_offset: *const u8,
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

    let address = Address::from_slice(unsafe { &*ptr::slice_from_raw_parts(address20_offset, 20) });

    let tx_caller_address =
        read_address_from_input(<ContractInput as IContractInput>::TxCaller::FIELD_OFFSET);
    let account =
        Account::new_from_jzkt(&fluentbase_types::Address::from_slice(address.as_slice()));

    let source_bytecode = account.load_source_bytecode();
    let input = Bytes::copy_from_slice(unsafe {
        &*ptr::slice_from_raw_parts(args_offset, args_size as usize)
    });
    let contract = Contract {
        input,
        hash: B256::from_slice(account.source_code_hash.as_slice()),
        // TODO simplify, we have already analyzed code
        bytecode: BytecodeLocked::try_from(to_analysed(Bytecode::new_raw(Bytes::copy_from_slice(
            source_bytecode.as_ref(),
        ))))
        .unwrap(),
        address: Address::new(account.address.into_array()),
        caller: Address::new(tx_caller_address.into_array()),
        value: U256::ZERO,
    };
    let mut interpreter = Interpreter::new(Box::new(contract), gas_limit as u64, is_static);
    let instruction_table = make_instruction_table::<FluentHost, ShanghaiSpec>();
    let mut host = FluentHost::default();
    let shared_memory = SharedMemory::new();
    let evm_run_result = interpreter.run(shared_memory, &instruction_table, &mut host);
    let interpreter_result = if let Some(v) = evm_run_result.into_result_return() {
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
        // let mut return_data = unsafe { ptr::slice_from_raw_parts_mut(ret_offset, ret_size_actual)
        // };
        unsafe { ptr::copy(output.as_ptr(), ret_offset, ret_size_actual) };
        // return_data.copy_from_slice(output.as_slice());
    }

    ExitCode::Ok
}
