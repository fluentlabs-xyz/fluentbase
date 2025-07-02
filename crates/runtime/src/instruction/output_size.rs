use crate::RuntimeContext;
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub struct SyscallOutputSize;

impl SyscallOutputSize {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        _params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let output_size = caller.context(Self::fn_impl);
        result[0] = Value::I32(output_size as i32);
        Ok(())
    }

    pub fn fn_impl(ctx: &RuntimeContext) -> u32 {
        ctx.execution_result.return_data.len() as u32
    }
}
