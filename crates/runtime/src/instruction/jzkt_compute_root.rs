use crate::RuntimeContext;
use rwasm::{core::Trap, Caller};

pub struct JzktComputeRoot;

impl JzktComputeRoot {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        output32_offset: u32,
    ) -> Result<(), Trap> {
        let root = Self::fn_impl(caller.data_mut());
        caller.write_memory(output32_offset, &root)?;
        Ok(())
    }

    pub fn fn_impl<T>(context: &mut RuntimeContext<T>) -> [u8; 32] {
        let jzkt = context.jzkt.clone().unwrap();
        let result = jzkt.borrow().compute_root();
        result
    }
}
