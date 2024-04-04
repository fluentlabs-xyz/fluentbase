use crate::{types::U256, RuntimeContext};
use fluentbase_types::ExitCode;
use rwasm::{common::Trap, Caller};

pub struct StateDbGetStorage;

impl StateDbGetStorage {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        key32_offset: u32,
        val32_offset: u32,
    ) -> Result<(), Trap> {
        let key = caller.read_memory(key32_offset, 32).to_vec();
        let result = Self::fn_impl(caller.data_mut(), &key).map_err(|err| err.into_trap())?;
        caller.write_memory(val32_offset, &result);
        Ok(())
    }

    pub fn fn_impl<T>(context: &mut RuntimeContext<T>, key: &[u8]) -> Result<Vec<u8>, ExitCode> {
        let account_db = context.account_db.clone().unwrap();
        let result = account_db
            .borrow_mut()
            .get_storage(&context.address, &U256::from_be_slice(key))
            .unwrap_or_default()
            .to_be_bytes_vec();
        Ok(result)
    }
}
