use crate::RuntimeContext;
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub struct SyscallState;

impl SyscallState {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        _params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let state = caller.context(Self::fn_impl);
        result[0] = Value::I32(state as i32);
        Ok(())
    }

    pub fn fn_impl(ctx: &RuntimeContext) -> u32 {
        ctx.state
    }
}
