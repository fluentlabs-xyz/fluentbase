use crate::RuntimeContext;
use rwasm::{core::Trap, Caller};

pub struct SyscallComputeRoot;

impl SyscallComputeRoot {
    pub fn fn_handler(
        mut caller: Caller<'_, RuntimeContext>,
        output32_offset: u32,
    ) -> Result<(), Trap> {
        let root = Self::fn_impl(caller.data_mut());
        caller.write_memory(output32_offset, &root)?;
        Ok(())
    }

    pub fn fn_impl(ctx: &mut RuntimeContext) -> [u8; 32] {
        let result = ctx.jzkt().borrow().compute_root();
        result
    }
}
