use crate::{syscall_handler::syscall_process_exit_code, RuntimeContext};
use core::{mem::take, ops::Range};
use fluentbase_types::ExitCode;
use rwasm::{StoreTr, TrapCode, Value};

pub fn syscall_read_output_handler(
    ctx: &mut impl StoreTr<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let (target_ptr, offset, length) = (
        params[0].i32().unwrap() as usize,
        params[1].i32().unwrap() as u32,
        params[2].i32().unwrap() as u32,
    );
    let range = syscall_read_output_range(ctx.data(), offset, length)
        .map_err(|exit_code| syscall_process_exit_code(ctx, exit_code))?;
    // Write straight from the return-data buffer into guest memory. The buffer is taken out of
    // the context for the duration of the write because the store borrows the context mutably.
    let return_data = take(&mut ctx.data_mut().execution_result.return_data);
    let result = ctx.memory_write(target_ptr, &return_data[range]);
    ctx.data_mut().execution_result.return_data = return_data;
    result
}

/// Validates `offset..offset + length` against the return data and returns it as a range.
pub fn syscall_read_output_range(
    ctx: &RuntimeContext,
    offset: u32,
    length: u32,
) -> Result<Range<usize>, ExitCode> {
    let offset_length = offset
        .checked_add(length)
        .ok_or(ExitCode::InputOutputOutOfBounds)?;
    if offset_length <= ctx.execution_result.return_data.len() as u32 {
        Ok(offset as usize..offset_length as usize)
    } else {
        Err(ExitCode::InputOutputOutOfBounds)
    }
}

pub fn syscall_read_output_impl(
    ctx: &mut RuntimeContext,
    offset: u32,
    length: u32,
) -> Result<Vec<u8>, ExitCode> {
    let range = syscall_read_output_range(ctx, offset, length)?;
    Ok(ctx.execution_result.return_data[range].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_offset_overflow_causes_memory_out_of_bounds() {
        let mut ctx = RuntimeContext::default();
        let exit_code = syscall_read_output_impl(&mut ctx, u32::MAX, 100).unwrap_err();
        assert_eq!(exit_code, ExitCode::InputOutputOutOfBounds);
    }

    #[test]
    fn range_is_bounded_by_the_return_data() {
        let mut ctx = RuntimeContext::default();
        ctx.execution_result.return_data = vec![1, 2, 3, 4];
        assert_eq!(syscall_read_output_range(&ctx, 1, 3), Ok(1..4));
        assert_eq!(syscall_read_output_range(&ctx, 4, 0), Ok(4..4));
        assert_eq!(
            syscall_read_output_range(&ctx, 2, 3),
            Err(ExitCode::InputOutputOutOfBounds)
        );
    }
}
