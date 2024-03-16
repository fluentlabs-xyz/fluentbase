use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{core::Trap, Caller};

pub struct SysForwardOutput;

impl SysForwardOutput {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        offset: u32,
        len: u32,
    ) -> Result<(), Trap> {
        Self::fn_impl(&mut caller.data_mut(), offset, len).map_err(|err| err.into_trap())?;
        Ok(())
    }

    pub fn fn_impl<T>(ctx: &mut RuntimeContext<T>, offset: u32, len: u32) -> Result<(), ExitCode> {
        if offset + len <= ctx.return_data.len() as u32 {
            let ret_data = &ctx.return_data[(offset as usize)..(offset as usize + len as usize)];
            ctx.output.extend_from_slice(ret_data);
            Ok(())
        } else {
            Err(ExitCode::MemoryOutOfBounds)
        }
    }
}
