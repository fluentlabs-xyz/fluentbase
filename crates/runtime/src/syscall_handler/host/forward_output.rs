///! Builtin to append a slice of return_data to the output buffer.
use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{Store, TrapCode, Value};

/// Copies [offset, offset+length) from return_data into output; halts on out-of-bounds.
pub fn syscall_forward_output_handler(
    caller: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let (offset, length) = (
        params[0].i32().unwrap() as u32,
        params[1].i32().unwrap() as u32,
    );
    caller.context_mut(|ctx| {
        syscall_forward_output_impl(ctx, offset, length).map_err(|err| {
            ctx.execution_result.exit_code = err.into_i32();
            TrapCode::ExecutionHalted
        })
    })
}

pub fn syscall_forward_output_impl(
    ctx: &mut RuntimeContext,
    offset: u32,
    length: u32,
) -> Result<(), ExitCode> {
    let offset_length = offset
        .checked_add(length)
        .ok_or(ExitCode::InputOutputOutOfBounds)?;
    if offset_length <= ctx.execution_result.return_data.len() as u32 {
        let ret_data = &ctx.execution_result.return_data
            [(offset as usize)..(offset as usize + length as usize)]
            .to_vec();
        ctx.execution_result.output.extend_from_slice(ret_data);
        Ok(())
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
        let exit_code = syscall_forward_output_impl(&mut ctx, u32::MAX, 100).unwrap_err();
        assert_eq!(exit_code, ExitCode::InputOutputOutOfBounds);
    }
}
