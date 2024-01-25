use crate::{types::U256, RuntimeContext};
use fluentbase_rwasm::{common::Trap, Caller};
use fluentbase_types::ExitCode;

pub struct StateDbUpdateStorage;

impl StateDbUpdateStorage {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        key32_offset: u32,
        val32_offset: u32,
    ) -> Result<(), Trap> {
        let key = caller.read_memory(key32_offset, 32).to_vec();
        let value = caller.read_memory(val32_offset, 32).to_vec();
        Self::fn_impl(caller.data_mut(), &key, &value).map_err(|err| err.into_trap())?;
        Ok(())
    }

    pub fn fn_impl<T>(
        context: &mut RuntimeContext<T>,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), ExitCode> {
        let account_db = context.account_db.clone().unwrap();
        account_db.borrow_mut().update_storage(
            &context.address,
            &U256::from_be_slice(key),
            &U256::from_be_slice(value),
        );
        Ok(())
    }
}
