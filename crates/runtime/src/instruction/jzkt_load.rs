use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{common::Trap, Caller};

pub struct JzktLoad;

impl JzktLoad {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        slot32_ptr: u32,
        value32_ptr: u32,
    ) -> Result<i32, Trap> {
        let slot = caller.read_memory(slot32_ptr, 32).to_vec();
        let value = Self::fn_impl(caller.data_mut(), slot.as_slice().try_into().unwrap())
            .map_err(|err| err.into_trap())?;
        let result = match value {
            Some((value, is_cold)) => {
                caller.write_memory(value32_ptr, &value);
                is_cold as i32
            }
            None => -1,
        };
        Ok(result)
    }

    pub fn fn_impl<T>(
        context: &mut RuntimeContext<T>,
        slot: &[u8; 32],
    ) -> Result<Option<([u8; 32], bool)>, ExitCode> {
        let jzkt = context.jzkt.clone().unwrap();
        let result = jzkt.borrow_mut().load(&context.address, slot);
        Ok(result)
    }
}
