use crate::RuntimeContext;
use rwasm::{core::Trap, Caller};

pub struct SysState;

impl SysState {
    pub fn fn_handler<T>(caller: Caller<'_, RuntimeContext<T>>) -> Result<u32, Trap> {
        Ok(Self::fn_impl(caller.data()))
    }

    pub fn fn_impl<T>(ctx: &RuntimeContext<T>) -> u32 {
        ctx.state
    }
}
