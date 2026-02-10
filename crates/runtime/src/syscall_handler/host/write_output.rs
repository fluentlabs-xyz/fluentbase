/// Builtin to append bytes to the runtime output buffer.
use crate::RuntimeContext;
use rwasm::{Store, TrapCode, Value};

/// Reads a slice from linear memory and appends it to ctx.execution_result.output.
pub fn syscall_write_output_handler(
    caller: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let (offset, length) = (params[0].i32().unwrap(), params[1].i32().unwrap());
    let mut data = vec![0u8; length as usize];
    caller.memory_read(offset as usize, &mut data)?;
    syscall_write_output_impl(caller.data_mut(), &data);
    Ok(())
}

pub fn syscall_write_output_impl(ctx: &mut RuntimeContext, data: &[u8]) {
    ctx.execution_result.output.extend_from_slice(data);
}
