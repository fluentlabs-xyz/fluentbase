use crate::RuntimeContext;
use fluentbase_sdk::ExitCode;
use rwasm::{Store, TrapCode, Value};

/// Builtin to copy a slice of the input buffer into linear memory.
pub struct SyscallRead;

impl SyscallRead {
    /// Reads [offset, offset+length) from ctx.input and writes it at target_ptr.
    pub fn fn_handler(
        caller: &mut impl Store<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (target_ptr, offset, length) = (
            params[0].i32().unwrap() as usize,
            params[1].i32().unwrap() as u32,
            params[2].i32().unwrap() as u32,
        );
        let input = caller.context_mut(|ctx| Self::fn_impl(ctx, offset, length))?;
        let _ = caller.memory_write(target_ptr, &input)?;
        Ok(())
    }

    pub fn fn_impl(
        ctx: &mut RuntimeContext,
        offset: u32,
        length: u32,
    ) -> Result<Vec<u8>, TrapCode> {
        if offset + length <= ctx.input.len() as u32 {
            Ok(ctx.input[(offset as usize)..(offset as usize + length as usize)].to_vec())
        } else {
            ctx.execution_result.exit_code = ExitCode::InputOutputOutOfBounds.into_i32();
            Err(TrapCode::ExecutionHalted)
        }
    }
}
