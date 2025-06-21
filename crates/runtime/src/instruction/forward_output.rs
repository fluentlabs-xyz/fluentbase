use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{Caller, TrapCode, Value};
use std::cell::RefMut;

pub struct SyscallForwardOutput;

impl SyscallForwardOutput {
    pub fn fn_handler(
        caller: &mut dyn Caller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (offset, length) = (
            params[0].i32().unwrap() as u32,
            params[1].i32().unwrap() as u32,
        );
        Self::fn_impl(caller.context_mut(), offset, length).map_err(|err| {
            caller.context_mut().execution_result.exit_code = err.into_i32();
            TrapCode::ExecutionHalted
        })?;
        Ok(())
    }

    pub fn fn_impl(mut ctx: RefMut<RuntimeContext>, offset: u32, len: u32) -> Result<(), ExitCode> {
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
