/// Builtin to query the current state selector.
use crate::RuntimeContext;
use rwasm::{Store, TrapCode, Value};

/// Writes `ctx.state` into `result[0]`.
pub fn syscall_state_handler(
    caller: &mut impl Store<RuntimeContext>,
    _params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    let state = syscall_state_impl(caller.data());
    result[0] = Value::I32(state as i32);
    Ok(())
}

pub fn syscall_state_impl(ctx: &RuntimeContext) -> u32 {
    ctx.state
}
