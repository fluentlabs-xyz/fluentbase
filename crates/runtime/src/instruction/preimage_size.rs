use crate::RuntimeContext;
use core::cell::Ref;
use rwasm::{Caller, TrapCode, Value};

pub struct SyscallPreimageSize;

impl SyscallPreimageSize {
    pub fn fn_handler(
        caller: &mut dyn Caller<RuntimeContext>,
        params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let hash32_offset = params[0].i32().unwrap();
        let mut hash = [0u8; 32];
        caller.memory_read(hash32_offset as usize, &mut hash)?;
        let preimage_size = Self::fn_impl(caller.context(), &hash)?;
        result[0] = Value::I32(preimage_size as i32);
        Ok(())
    }

    pub fn fn_impl(_ctx: Ref<RuntimeContext>, _hash: &[u8]) -> Result<u32, TrapCode> {
        Err(TrapCode::UnreachableCodeReached)
    }
}
