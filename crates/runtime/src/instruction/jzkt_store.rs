use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{common::Trap, Caller};

pub struct JzktStore;

impl JzktStore {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        slot32_ptr: u32,
        value32_ptr: u32,
    ) -> Result<(), Trap> {
        let slot = caller.read_memory(slot32_ptr, 32).to_vec();
        let value = caller.read_memory(value32_ptr, 32).to_vec();
        Self::fn_impl(
            caller.data_mut(),
            slot.as_slice().try_into().unwrap(),
            value.as_slice().try_into().unwrap(),
        )
        .map_err(|err| err.into_trap())?;
        Ok(())
    }

    pub fn fn_impl<T>(
        context: &mut RuntimeContext<T>,
        slot: &[u8; 32],
        value: &[u8; 32],
    ) -> Result<(), ExitCode> {
        let jzkt = context.jzkt.clone().unwrap();
        jzkt.borrow_mut().store(&context.address, slot, value);
        Ok(())
    }
}
