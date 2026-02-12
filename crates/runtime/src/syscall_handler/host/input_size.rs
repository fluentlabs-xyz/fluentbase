/// Builtin to query the size of ctx.input.
use crate::RuntimeContext;
use rwasm::{StoreTr, TrapCode, Value};

/// Writes the input length in bytes into result[0].
pub fn syscall_input_size_handler(
    caller: &mut impl StoreTr<RuntimeContext>,
    _params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    let input_size = syscall_input_size_impl(caller.data());
    result[0] = Value::I32(input_size as i32);
    Ok(())
}

pub fn syscall_input_size_impl(ctx: &RuntimeContext) -> u32 {
    ctx.input.len() as u32
}
