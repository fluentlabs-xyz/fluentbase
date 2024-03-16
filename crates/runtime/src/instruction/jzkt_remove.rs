use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{core::Trap, Caller};

pub struct JzktRemove;

impl JzktRemove {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        key32_offset: u32,
    ) -> Result<(), Trap> {
        let key = caller.read_memory(key32_offset, 32).to_vec();
        Self::fn_impl(caller.data_mut(), &key).map_err(|err| err.into_trap())?;
        Ok(())
    }

    pub fn fn_impl<T>(context: &mut RuntimeContext<T>, key: &[u8]) -> Result<(), ExitCode> {
        let jzkt = context.jzkt.clone().unwrap();
        jzkt.borrow_mut().remove(key.try_into().unwrap());
        Ok(())
    }
}
