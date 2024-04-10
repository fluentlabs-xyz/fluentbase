use crate::RuntimeContext;
use fluentbase_types::{ExitCode, IJournaledTrie};
use rwasm::{core::Trap, Caller};

pub struct SysReadOutput;

impl SysReadOutput {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        target: u32,
        offset: u32,
        length: u32,
    ) -> Result<(), Trap> {
        let input = Self::fn_impl(caller.data(), offset, length).map_err(|err| err.into_trap())?;
        let _ = caller.write_memory(target, &input)?;
        Ok(())
    }

    pub fn fn_impl<DB: IJournaledTrie>(
        ctx: &RuntimeContext<DB>,
        offset: u32,
        length: u32,
    ) -> Result<Vec<u8>, ExitCode> {
        if offset + length <= ctx.return_data.len() as u32 {
            Ok(ctx.return_data[(offset as usize)..(offset as usize + length as usize)].to_vec())
        } else {
            Err(ExitCode::MemoryOutOfBounds)
        }
    }
}
