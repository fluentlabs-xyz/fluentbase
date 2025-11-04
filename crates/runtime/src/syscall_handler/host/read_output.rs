///! Builtin to copy a slice of the current return_data into linear memory.
use crate::syscall_handler::syscall_process_exit_code;
use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{Store, TrapCode, Value};

/// Reads [offset, offset+length) from ctx.execution_result.return_data and writes it at target_ptr.
pub fn syscall_read_output_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let (target_ptr, offset, length) = (
        params[0].i32().unwrap() as usize,
        params[1].i32().unwrap() as u32,
        params[2].i32().unwrap() as u32,
    );
    let input = ctx
        .context_mut(|ctx| syscall_read_output_impl(ctx, offset, length))
        .map_err(|exit_code| syscall_process_exit_code(ctx, exit_code))?;
    let _ = ctx.memory_write(target_ptr, &input)?;
    Ok(())
}

pub fn syscall_read_output_impl(
    ctx: &mut RuntimeContext,
    offset: u32,
    length: u32,
) -> Result<Vec<u8>, ExitCode> {
    let offset_length = offset
        .checked_add(length)
        .ok_or(ExitCode::InputOutputOutOfBounds)?;
    if offset_length <= ctx.execution_result.return_data.len() as u32 {
        Ok(
            ctx.execution_result.return_data
                [(offset as usize)..(offset as usize + length as usize)]
                .to_vec(),
        )
    } else {
        Err(ExitCode::InputOutputOutOfBounds)
    }
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
}
