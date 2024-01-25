use crate::{types::Bytes, RuntimeContext};
use fluentbase_rwasm::{common::Trap, Caller};
use fluentbase_types::{Account, Address, B256};

pub struct StateDbUpdateCode;

impl StateDbUpdateCode {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        key20_offset: u32,
        code_offset: u32,
        code_len: u32,
    ) -> Result<(), Trap> {
        let key = caller.read_memory(key20_offset, 20).to_vec();
        let code = caller.read_memory(code_offset, code_len).to_vec();
        Self::fn_impl(caller.data_mut(), &key, &code);
        Ok(())
    }

    pub fn fn_impl<T>(context: &mut RuntimeContext<T>, key: &[u8], code: &[u8]) {
        let account_db = context.account_db.clone().unwrap();
        let mut account = account_db
            .borrow_mut()
            .get_account(&Address::from_slice(key))
            .unwrap_or_default();
        account.code = Some(Bytes::copy_from_slice(code));
        if code.len() > 0 {
            account.code_hash = B256::from_slice(keccak_hash::keccak(code).as_bytes());
        } else {
            account.code_hash = Account::empty_code_hash();
        }
    }
}
