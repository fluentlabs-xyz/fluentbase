use crate::{syscall_handler::syscall_process_exit_code, RuntimeContext};
use core::{mem::take, ops::Range};
use fluentbase_types::ExitCode;
use rwasm::{StoreTr, TrapCode, Value};

pub fn syscall_read_input_handler(
    ctx: &mut impl StoreTr<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let (target_ptr, offset, length) = (
        params[0].i32().unwrap() as usize,
        params[1].i32().unwrap() as u32,
        params[2].i32().unwrap() as u32,
    );
    let range = syscall_read_input_range(ctx.data(), offset, length)
        .map_err(|exit_code| syscall_process_exit_code(ctx, exit_code))?;
    // Write straight from the input buffer into guest memory. The buffer is taken out of the
    // context for the duration of the write because the store borrows the context mutably.
    let input = take(&mut ctx.data_mut().input);
    let result = ctx.memory_write(target_ptr, &input[range]);
    ctx.data_mut().input = input;
    result
}

/// Validates `offset..offset + length` against the frame input and returns it as a range.
pub fn syscall_read_input_range(
    ctx: &RuntimeContext,
    offset: u32,
    length: u32,
) -> Result<Range<usize>, ExitCode> {
    let offset_length = offset
        .checked_add(length)
        .ok_or(ExitCode::InputOutputOutOfBounds)?;
    if offset_length <= ctx.input.len() as u32 {
        Ok(offset as usize..offset_length as usize)
    } else {
        Err(ExitCode::InputOutputOutOfBounds)
    }
}

pub fn syscall_read_input_impl(
    ctx: &mut RuntimeContext,
    offset: u32,
    length: u32,
) -> Result<Vec<u8>, ExitCode> {
    let range = syscall_read_input_range(ctx, offset, length)?;
    Ok(ctx.input[range].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_offset_overflow_causes_memory_out_of_bounds() {
        let mut ctx = RuntimeContext::default();
        let exit_code = syscall_read_input_impl(&mut ctx, u32::MAX, 100).unwrap_err();
        assert_eq!(exit_code, ExitCode::InputOutputOutOfBounds);
    }

    #[test]
    fn range_is_bounded_by_the_input() {
        let ctx = RuntimeContext::default().with_input(vec![1, 2, 3, 4]);
        assert_eq!(syscall_read_input_range(&ctx, 1, 3), Ok(1..4));
        assert_eq!(syscall_read_input_range(&ctx, 4, 0), Ok(4..4));
        assert_eq!(
            syscall_read_input_range(&ctx, 2, 3),
            Err(ExitCode::InputOutputOutOfBounds)
        );
    }
}
