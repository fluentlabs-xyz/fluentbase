/// Builtin to copy a slice of the current return_data into linear memory.
use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{Store, TrapCode, Value};

/// Reads [offset, offset+length) from ctx.execution_result.return_data and writes it at target_ptr.
pub fn syscall_read_output_handler(
    caller: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let (target_ptr, offset, length) = (
        params[0].i32().unwrap() as usize,
        params[1].i32().unwrap() as u32,
        params[2].i32().unwrap() as u32,
    );
    let input = caller.context_mut(|ctx| syscall_read_output_impl(ctx, offset, length))?;
    let _ = caller.memory_write(target_ptr, &input)?;
    Ok(())
}

pub fn syscall_read_output_impl(
    ctx: &mut RuntimeContext,
    offset: u32,
    length: u32,
) -> Result<Vec<u8>, TrapCode> {
    if offset + length <= ctx.execution_result.return_data.len() as u32 {
        Ok(
            ctx.execution_result.return_data
                [(offset as usize)..(offset as usize + length as usize)]
                .to_vec(),
        )
    } else {
        ctx.execution_result.exit_code = ExitCode::InputOutputOutOfBounds.into_i32();
        Err(TrapCode::ExecutionHalted)
    }
}
