use crate::RuntimeContext;
use rwasm::{Store, TrapCode, TypedCaller, Value};

/// Builtin to append bytes to the runtime output buffer.
pub struct SyscallWrite;

impl SyscallWrite {
    /// Reads a slice from linear memory and appends it to ctx.execution_result.output.
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (offset, length) = (params[0].i32().unwrap(), params[1].i32().unwrap());
        let mut data = vec![0u8; length as usize];
        caller.memory_read(offset as usize, &mut data)?;
        caller.context_mut(|ctx| Self::fn_impl(ctx, &data));
        Ok(())
    }

    pub fn fn_impl(ctx: &mut RuntimeContext, data: &[u8]) {
        ctx.execution_result.output.extend_from_slice(data);
    }
}
