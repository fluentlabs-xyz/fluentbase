use crate::RuntimeContext;
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub struct SyscallPreimageSize;

impl SyscallPreimageSize {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let hash32_offset = params[0].i32().unwrap();
        let mut hash = [0u8; 32];
        caller.memory_read(hash32_offset as usize, &mut hash)?;
        let preimage_size = caller.context(|ctx| Self::fn_impl(ctx, &hash))?;
        result[0] = Value::I32(preimage_size as i32);
        Ok(())
    }

    pub fn fn_impl(_ctx: &RuntimeContext, _hash: &[u8]) -> Result<u32, TrapCode> {
        Err(TrapCode::UnreachableCodeReached)
    }
}
