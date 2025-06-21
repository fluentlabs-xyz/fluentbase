use crate::RuntimeContext;
use core::cell::Ref;
use rwasm::{Caller, TrapCode, Value};

pub struct SyscallOutputSize;

impl SyscallOutputSize {
    pub fn fn_handler(
        caller: &mut dyn Caller<RuntimeContext>,
        _params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let output_size = Self::fn_impl(caller.context());
        result[0] = Value::I32(output_size as i32);
        Ok(())
    }

    pub fn fn_impl(ctx: Ref<RuntimeContext>) -> u32 {
        ctx.execution_result.return_data.len() as u32
    }
}
