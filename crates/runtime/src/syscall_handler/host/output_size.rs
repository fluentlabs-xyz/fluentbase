/// Builtin to query the size of the current return_data buffer.
use crate::RuntimeContext;
use rwasm::{StoreTr, TrapCode, Value};

/// Writes the length of ctx.execution_result.return_data into result[0].
pub fn syscall_output_size_handler(
    caller: &mut impl StoreTr<RuntimeContext>,
    _params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    let output_size = syscall_output_size_impl(caller.data());
    result[0] = Value::I32(output_size as i32);
    Ok(())
}

pub fn syscall_output_size_impl(ctx: &RuntimeContext) -> u32 {
    ctx.execution_result.return_data.len() as u32
}
