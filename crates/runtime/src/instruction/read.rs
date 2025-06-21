use crate::RuntimeContext;
use core::cell::RefMut;
use fluentbase_types::ExitCode;
use rwasm::{Caller, TrapCode, Value};

pub struct SyscallRead;

impl SyscallRead {
    pub fn fn_handler(
        caller: &mut dyn Caller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (target_ptr, offset, length) = (
            params[0].i32().unwrap() as usize,
            params[1].i32().unwrap() as u32,
            params[2].i32().unwrap() as u32,
        );
        let input = Self::fn_impl(caller.context_mut(), offset, length)?;
        let _ = caller.memory_write(target_ptr, &input)?;
        Ok(())
    }

    pub fn fn_impl(
        mut ctx: RefMut<RuntimeContext>,
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
