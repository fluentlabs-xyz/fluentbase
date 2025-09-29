use crate::RuntimeContext;
use fluentbase_types::Bytes;
use rwasm::{Store, TrapCode, Value};

pub struct SyscallPreimageCopy;

impl SyscallPreimageCopy {
    pub fn fn_handler(
        caller: &mut impl Store<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (hash32_ptr, preimage_ptr) = (params[0].i32().unwrap(), params[1].i32().unwrap());
        let mut hash = [0u8; 32];
        caller.memory_read(hash32_ptr as usize, &mut hash)?;
        let preimage = caller.context(|ctx| Self::fn_impl(ctx, &hash))?;
        caller.memory_write(preimage_ptr as usize, preimage.as_ref())?;
        Ok(())
    }

    pub fn fn_impl(_ctx: &RuntimeContext, _hash: &[u8]) -> Result<Bytes, TrapCode> {
        Err(TrapCode::UnreachableCodeReached)
    }
}
