use crate::RuntimeContext;
use rwasm::{common::Trap, Caller};

pub struct SysInputSize;

impl SysInputSize {
    pub fn fn_handler<T>(caller: Caller<'_, RuntimeContext<T>>) -> Result<u32, Trap> {
        Ok(Self::fn_impl(caller.data()))
    }

    pub fn fn_impl<T>(ctx: &RuntimeContext<T>) -> u32 {
        ctx.input_size()
    }
}
