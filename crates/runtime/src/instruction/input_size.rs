use crate::RuntimeContext;
use core::cell::Ref;
use rwasm::{Caller, TrapCode, Value};

pub struct SyscallInputSize;

impl SyscallInputSize {
    pub fn fn_handler(
        caller: &mut dyn Caller<RuntimeContext>,
        _params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let input_size = Self::fn_impl(caller.context());
        result[0] = Value::I32(input_size as i32);
        Ok(())
    }

    pub fn fn_impl(ctx: Ref<RuntimeContext>) -> u32 {
        ctx.input_size()
    }
}
