use core::{cmp::min, ops::Range};
use fluentbase_sdk::{Address, Bytes, SyscallResult, SyscallStatus, B256, FUEL_DENOM_RATE, U256};
use revm_interpreter::{
    gas,
    instructions::contract::resize_memory,
    pop_ret,
    primitives::SpecId,
    push,
    push_b256,
    refund,
    InstructionResult,
    Interpreter,
};

pub(crate) const BASE_SPEC: SpecId = SpecId::CANCUN;

pub(crate) fn insert_create_outcome(interpreter: &mut Interpreter, result: SyscallResult<Bytes>) {
    gas!(interpreter, result.fuel_consumed / FUEL_DENOM_RATE);
    refund!(interpreter, result.fuel_refunded / FUEL_DENOM_RATE as i64);
    match result.status {
        SyscallStatus::Ok => {
            assert_eq!(result.data.len(), 20);
            let created_address = Address::from_slice(result.data.as_ref());
            push_b256!(interpreter, created_address.into_word());
        }
        SyscallStatus::Revert => {
            interpreter.return_data_buffer = result.data;
            push_b256!(interpreter, B256::ZERO);
        }
        SyscallStatus::Err => {
            push_b256!(interpreter, B256::ZERO);
        }
    }
}

pub(crate) fn insert_call_outcome(
    interpreter: &mut Interpreter,
    result: SyscallResult<Bytes>,
    return_memory_offset: Range<usize>,
) {
    let out_offset = return_memory_offset.start;
    let out_len = return_memory_offset.len();
    interpreter.return_data_buffer = result.data;
    let target_len = min(out_len, interpreter.return_data_buffer.len());
    match result.status {
        SyscallStatus::Ok => {
            gas!(interpreter, result.fuel_consumed / FUEL_DENOM_RATE);
            refund!(interpreter, result.fuel_refunded / FUEL_DENOM_RATE as i64);
            interpreter
                .shared_memory
                .set(out_offset, &interpreter.return_data_buffer[..target_len]);
            push!(
                interpreter,
                if interpreter.is_eof {
                    U256::ZERO
                } else {
                    U256::from(1)
                }
            );
        }
        SyscallStatus::Revert => {
            gas!(interpreter, result.fuel_consumed / FUEL_DENOM_RATE);
            interpreter
                .shared_memory
                .set(out_offset, &interpreter.return_data_buffer[..target_len]);
            push!(
                interpreter,
                if interpreter.is_eof {
                    U256::from(1)
                } else {
                    U256::ZERO
                }
            );
        }
        SyscallStatus::Err => {
            gas!(interpreter, result.fuel_consumed / FUEL_DENOM_RATE);
            push!(
                interpreter,
                if interpreter.is_eof {
                    U256::from(2)
                } else {
                    U256::ZERO
                }
            );
        }
    }
    interpreter.instruction_result = InstructionResult::Continue;
}

#[inline]
pub fn get_memory_input_and_out_ranges(
    interpreter: &mut Interpreter,
) -> Option<(Bytes, Range<usize>)> {
    pop_ret!(interpreter, in_offset, in_len, out_offset, out_len, None);

    let in_range = resize_memory(interpreter, in_offset, in_len)?;

    let mut input = Bytes::new();
    if !in_range.is_empty() {
        input = Bytes::copy_from_slice(interpreter.shared_memory.slice_range(in_range));
    }

    let ret_range = resize_memory(interpreter, out_offset, out_len)?;
    Some((input, ret_range))
}

pub(crate) unsafe fn read_i16(ptr: *const u8) -> i16 {
    i16::from_be_bytes(core::slice::from_raw_parts(ptr, 2).try_into().unwrap())
}

pub(crate) unsafe fn read_u16(ptr: *const u8) -> u16 {
    u16::from_be_bytes(core::slice::from_raw_parts(ptr, 2).try_into().unwrap())
}
