use crate::{
    as_usize_or_fail_ret,
    gas,
    pop_ret,
    push,
    push_b256,
    refund,
    resize_memory,
    result::InstructionResult,
    EVM,
};
use core::{cmp::min, ops::Range};
use fluentbase_sdk::{
    syscall::SyscallResult,
    Address,
    Bytes,
    ExitCode,
    SharedAPI,
    B256,
    FUEL_DENOM_RATE,
    U256,
};

pub(crate) fn instruction_result_from_exit_code(exit_code: ExitCode) -> InstructionResult {
    match exit_code {
        ExitCode::OutOfFuel => InstructionResult::OutOfGas,
        _ => unreachable!(
            "unexpected return err: {:?} ({})",
            exit_code,
            exit_code.into_i32()
        ),
    }
}

pub(crate) fn insert_create_outcome<SDK: SharedAPI>(
    evm: &mut EVM<SDK>,
    result: SyscallResult<Bytes>,
) {
    gas!(evm, result.fuel_consumed / FUEL_DENOM_RATE);
    refund!(evm, result.fuel_refunded / FUEL_DENOM_RATE as i64);
    evm.return_data_buffer = Bytes::new();
    match result.status {
        ExitCode::Ok => {
            assert_eq!(result.data.len(), 20);
            let created_address = Address::from_slice(result.data.as_ref());
            push_b256!(evm, created_address.into_word());
        }
        ExitCode::Panic => {
            evm.return_data_buffer = result.data;
            push_b256!(evm, B256::ZERO);
        }
        ExitCode::Err => {
            push_b256!(evm, B256::ZERO);
        }
        _ => evm.state = instruction_result_from_exit_code(result.status),
    }
}

pub(crate) fn insert_call_outcome<SDK: SharedAPI>(
    evm: &mut EVM<SDK>,
    result: SyscallResult<Bytes>,
    return_memory_offset: Range<usize>,
) {
    let out_offset = return_memory_offset.start;
    let out_len = return_memory_offset.len();
    evm.return_data_buffer = result.data;
    let target_len = min(out_len, evm.return_data_buffer.len());
    match result.status {
        ExitCode::Ok => {
            gas!(evm, result.fuel_consumed / FUEL_DENOM_RATE);
            refund!(evm, result.fuel_refunded / FUEL_DENOM_RATE as i64);
            evm.memory
                .set(out_offset, &evm.return_data_buffer[..target_len]);
            push!(evm, U256::from(1));
        }
        ExitCode::Panic => {
            gas!(evm, result.fuel_consumed / FUEL_DENOM_RATE);
            evm.memory
                .set(out_offset, &evm.return_data_buffer[..target_len]);
            push!(evm, U256::ZERO);
        }
        ExitCode::Err => {
            gas!(evm, result.fuel_consumed / FUEL_DENOM_RATE);
            push!(evm, U256::ZERO);
        }
        _ => evm.state = instruction_result_from_exit_code(result.status),
    }
}

/// Resize memory and return range of memory.
/// If `len` is 0 dont touch memory and return `usize::MAX` as offset and 0 as length.
#[inline]
pub fn resize_memory<SDK: SharedAPI>(
    evm: &mut EVM<SDK>,
    offset: U256,
    len: U256,
) -> Option<Range<usize>> {
    let len = as_usize_or_fail_ret!(evm, len, None);
    let offset = if len != 0 {
        let offset = as_usize_or_fail_ret!(evm, offset, None);
        resize_memory!(evm, offset, len, None);
        offset
    } else {
        usize::MAX //unrealistic value so we are sure it is not used
    };
    Some(offset..offset + len)
}

#[inline]
pub fn get_memory_input_and_out_ranges<SDK: SharedAPI>(
    evm: &mut EVM<SDK>,
) -> Option<(Bytes, Range<usize>)> {
    pop_ret!(evm, in_offset, in_len, out_offset, out_len, None);

    let in_range = resize_memory(evm, in_offset, in_len)?;

    let mut input = Bytes::new();
    if !in_range.is_empty() {
        input = Bytes::copy_from_slice(evm.memory.slice_range(in_range));
    }

    let ret_range = resize_memory(evm, out_offset, out_len)?;
    Some((input, ret_range))
}
