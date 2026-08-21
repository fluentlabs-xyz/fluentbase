/// Builtin to append bytes to the runtime output buffer.
use crate::RuntimeContext;
use rwasm::{StoreTr, TrapCode, Value};

/// Reads a slice from linear memory and appends it to ctx.execution_result.output.
pub fn syscall_write_output_handler(
    caller: &mut impl StoreTr<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    // Allocation safety invariant: the rWASM translator injects the `_write` linear fuel charge,
    // including `length`, before this host handler can run. Each append is charged separately, so
    // repeated writes are cumulatively bounded by the frame's remaining fuel. Keep the import's
    // fuel procedure in sync with this allocation if the syscall is changed.
    let (offset, length) = (params[0].i32().unwrap(), params[1].i32().unwrap());
    let data = caller.memory_read_into_vec(offset as usize, length as usize)?;
    syscall_write_output_impl(caller.data_mut(), &data);
    Ok(())
}

pub fn syscall_write_output_impl(ctx: &mut RuntimeContext, data: &[u8]) {
    ctx.execution_result.output.extend_from_slice(data);
}
