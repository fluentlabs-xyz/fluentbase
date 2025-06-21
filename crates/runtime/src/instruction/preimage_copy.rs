use crate::RuntimeContext;
use core::cell::Ref;
use fluentbase_types::Bytes;
use rwasm::{Caller, TrapCode, Value};

pub struct SyscallPreimageCopy;

impl SyscallPreimageCopy {
    pub fn fn_handler(
        caller: &mut dyn Caller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (hash32_ptr, preimage_ptr) = (params[0].i32().unwrap(), params[1].i32().unwrap());
        let mut hash = [0u8; 32];
        caller.memory_read(hash32_ptr as usize, &mut hash)?;
        let preimage = Self::fn_impl(caller.context(), &hash)?;
        caller.memory_write(preimage_ptr as usize, preimage.as_ref())?;
        Ok(())
    }

    pub fn fn_impl(_ctx: Ref<RuntimeContext>, _hash: &[u8]) -> Result<Bytes, TrapCode> {
        Err(TrapCode::UnreachableCodeReached)
    }
}
