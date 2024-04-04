use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{common::Trap, Caller};

pub struct JzktOpen;

impl JzktOpen {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        root32_offset: u32,
    ) -> Result<(), Trap> {
        let root32 = caller.read_memory(root32_offset, 32).to_vec();
        Self::fn_impl(caller.data_mut(), &root32).map_err(|err| err.into_trap())?;
        Ok(())
    }

    pub fn fn_impl<T>(_context: &mut RuntimeContext<T>, _root32: &[u8]) -> Result<(), ExitCode> {
        todo!("not implemented yet")
        // let jzkt = context.jzkt.clone().unwrap();
        // jzkt.borrow_mut().open(root32);
        // Ok(())
    }
}
