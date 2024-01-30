use crate::RuntimeContext;
use rwasm::{common::Trap, Caller};

pub struct SysHalt;

impl SysHalt {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        exit_code: i32,
    ) -> Result<(), Trap> {
        caller.data_mut().exit_code = exit_code;
        Err(Trap::i32_exit(exit_code))
    }

    pub fn fn_impl() {}
}
