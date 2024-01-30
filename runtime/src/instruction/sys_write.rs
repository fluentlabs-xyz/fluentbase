use crate::RuntimeContext;
use rwasm::{common::Trap, Caller};

pub struct SysWrite;

impl SysWrite {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        offset: u32,
        length: u32,
    ) -> Result<(), Trap> {
        let data = caller.read_memory(offset, length).to_vec();
        Self::fn_impl(caller.data_mut(), &data);
        Ok(())
    }

    pub fn fn_impl<T>(ctx: &mut RuntimeContext<T>, data: &[u8]) {
        ctx.extend_return_data(data);
    }
}
