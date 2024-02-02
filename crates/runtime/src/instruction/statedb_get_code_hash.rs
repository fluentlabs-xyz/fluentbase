use crate::{types::Address, RuntimeContext};
use keccak_hash::H256;
use rwasm::{common::Trap, Caller};

pub struct StateDbGetCodeHash;

impl StateDbGetCodeHash {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        key20_offset: u32,
        out_hash32_offset: u32,
    ) -> Result<(), Trap> {
        let address = caller.read_memory(key20_offset, 20).to_vec();
        let hash = Self::fn_impl(caller.data_mut(), &address);
        caller.write_memory(out_hash32_offset, &hash[0..hash.len()]);
        Ok(())
    }

    pub fn fn_impl<T>(context: &mut RuntimeContext<T>, key: &[u8]) -> Vec<u8> {
        let account_db = context.account_db.clone().unwrap();
        let account = account_db
            .borrow_mut()
            .get_account(&Address::from_slice(key));
        let hash: H256;
        if let Some(account) = account {
            let code = account.code;
            hash = if let Some(code) = code {
                keccak_hash::keccak(&code)
            } else {
                keccak_hash::KECCAK_EMPTY
            };
        } else {
            hash = H256::zero()
        }
        hash.as_bytes().to_vec()
    }
}
