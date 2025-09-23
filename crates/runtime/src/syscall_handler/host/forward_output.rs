use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{Store, TrapCode, TypedCaller, Value};

/// Builtin to append a slice of return_data to the output buffer.
pub struct SyscallForwardOutput;

impl SyscallForwardOutput {
    /// Copies [offset, offset+length) from return_data into output; halts on out-of-bounds.
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (offset, length) = (
            params[0].i32().unwrap() as u32,
            params[1].i32().unwrap() as u32,
        );
        caller.context_mut(|ctx| {
            Self::fn_impl(ctx, offset, length).map_err(|err| {
                ctx.execution_result.exit_code = err.into_i32();
                TrapCode::ExecutionHalted
            })
        })
    }

    pub fn fn_impl(ctx: &mut RuntimeContext, offset: u32, len: u32) -> Result<(), ExitCode> {
        if offset + len <= ctx.execution_result.return_data.len() as u32 {
            let ret_data = &ctx.execution_result.return_data
                [(offset as usize)..(offset as usize + len as usize)]
                .to_vec();
            ctx.execution_result.output.extend_from_slice(ret_data);
            Ok(())
        } else {
            Err(ExitCode::MemoryOutOfBounds)
        }
    }
}
