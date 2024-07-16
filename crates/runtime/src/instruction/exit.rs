use crate::RuntimeContext;
use fluentbase_types::{ExitCode, IJournaledTrie};
use rwasm::{core::Trap, Caller};

pub struct SyscallExit;

impl SyscallExit {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        exit_code: i32,
    ) -> Result<(), Trap> {
        let exit_code = Self::fn_impl(caller.data_mut(), exit_code).unwrap_err();
        Err(exit_code.into_trap())
    }

    pub fn fn_impl<DB: IJournaledTrie>(
        ctx: &mut RuntimeContext<DB>,
        exit_code: i32,
    ) -> Result<(), ExitCode> {
        ctx.execution_result.exit_code = exit_code;
        Err(ExitCode::from(exit_code))
    }
}
