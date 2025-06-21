use crate::RuntimeContext;
use core::cell::RefMut;
use rwasm::{Caller, TrapCode, Value};

pub struct SyscallWrite;

impl SyscallWrite {
    pub fn fn_handler(
        caller: &mut dyn Caller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (offset, length) = (params[0].i32().unwrap(), params[1].i32().unwrap());
        let mut data = vec![0u8; length as usize];
        caller.memory_read(offset as usize, &mut data)?;
        Self::fn_impl(caller.context_mut(), &data);
        Ok(())
    }

    pub fn fn_impl(mut ctx: RefMut<RuntimeContext>, data: &[u8]) {
        ctx.execution_result.output.extend_from_slice(data);
    }
}
