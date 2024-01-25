use crate::{types::Address, RuntimeContext};
use fluentbase_rwasm::{common::Trap, Caller};

pub struct StateDbGetCodeSize;

impl StateDbGetCodeSize {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        key20_offset: u32,
    ) -> Result<u32, Trap> {
        let address = caller.read_memory(key20_offset, 20).to_vec();
        Ok(Self::fn_impl(caller.data_mut(), &address))
    }

    pub fn fn_impl<T>(context: &mut RuntimeContext<T>, key: &[u8]) -> u32 {
        let account_db = context.account_db.clone().unwrap();
        let account = account_db
            .borrow_mut()
            .get_account(&Address::from_slice(key))
            .unwrap_or_default();
        let code = account.code.unwrap_or_default();
        code.len() as u32
    }
}
