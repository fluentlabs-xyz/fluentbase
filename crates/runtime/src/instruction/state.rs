use crate::RuntimeContext;
use core::cell::Ref;
use rwasm::{Caller, TrapCode, Value};

pub struct SyscallState;

impl SyscallState {
    pub fn fn_handler(
        caller: &mut dyn Caller<RuntimeContext>,
        _params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let state = Self::fn_impl(caller.context());
        result[0] = Value::I32(state as i32);
        Ok(())
    }

    pub fn fn_impl(ctx: Ref<RuntimeContext>) -> u32 {
        ctx.state
    }
}
