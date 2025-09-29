use crate::RuntimeContext;
use rwasm::{Store, TrapCode, Value};

/// Builtin to query the size of ctx.input.
pub struct SyscallInputSize;

impl SyscallInputSize {
    /// Writes the input length in bytes into result[0].
    pub fn fn_handler(
        caller: &mut impl Store<RuntimeContext>,
        _params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let input_size = caller.context(Self::fn_impl);
        result[0] = Value::I32(input_size as i32);
        Ok(())
    }

    pub fn fn_impl(ctx: &RuntimeContext) -> u32 {
        ctx.input.len() as u32
    }
}
