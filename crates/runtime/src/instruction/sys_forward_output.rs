use crate::RuntimeContext;
use fluentbase_types::{ExitCode, IJournaledTrie};
use rwasm::{core::Trap, Caller};

pub struct SysForwardOutput;

impl SysForwardOutput {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        offset: u32,
        len: u32,
    ) -> Result<(), Trap> {
        Self::fn_impl(&mut caller.data_mut(), offset, len).map_err(|err| err.into_trap())?;
        Ok(())
    }

    pub fn fn_impl<DB: IJournaledTrie>(
        ctx: &mut RuntimeContext<DB>,
        offset: u32,
        len: u32,
    ) -> Result<(), ExitCode> {
        if offset + len <= ctx.execution_result.return_data.len() as u32 {
            let ret_data = &ctx.execution_result.return_data
                [(offset as usize)..(offset as usize + len as usize)];
            ctx.execution_result.output.extend_from_slice(ret_data);
            Ok(())
        } else {
            Err(ExitCode::MemoryOutOfBounds)
        }
    }
}
