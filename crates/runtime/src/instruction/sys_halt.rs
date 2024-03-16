use crate::RuntimeContext;
use rwasm::{core::Trap, Caller};

pub struct SysHalt;

impl SysHalt {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        exit_code: i32,
    ) -> Result<(), Trap> {
        Self::fn_impl(caller.data_mut(), exit_code);
        Err(Trap::i32_exit(exit_code))
    }

    pub fn fn_impl<T>(ctx: &mut RuntimeContext<T>, exit_code: i32) {
        ctx.exit_code = exit_code;
    }
}
