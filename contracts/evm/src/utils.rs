use core::{cmp::min, ops::Range};
use fluentbase_sdk::{Bytes, SyscallResult, SyscallStatus, FUEL_DENOM_RATE, U256};
use revm_interpreter::{gas, primitives::SpecId, push, InstructionResult, Interpreter};

pub(crate) const BASE_SPEC: SpecId = SpecId::CANCUN;

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
            gas!(interpreter, result.fuel_used as u64 / FUEL_DENOM_RATE);
            // TODO(dmitry123): "add support of refunds"
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
            gas!(interpreter, result.fuel_used as u64 / FUEL_DENOM_RATE);
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
        SyscallStatus::Error => {
            gas!(interpreter, result.fuel_used as u64 / FUEL_DENOM_RATE);
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

pub(crate) unsafe fn read_i16(ptr: *const u8) -> i16 {
    i16::from_be_bytes(core::slice::from_raw_parts(ptr, 2).try_into().unwrap())
}

pub(crate) unsafe fn read_u16(ptr: *const u8) -> u16 {
    u16::from_be_bytes(core::slice::from_raw_parts(ptr, 2).try_into().unwrap())
}
