use crate::RuntimeContext;
use fluentbase_rwasm::{common::Trap, Caller};
use fluentbase_types::ExitCode;

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
        let zktrie = context.trie_db.clone().unwrap();
        let result = zktrie
            .borrow_mut()
            .get(key)
            .ok_or_else(|| ExitCode::PersistentStorageError)?;
        if !result.is_empty() {
            Ok(result[0].to_vec())
        } else {
            Err(ExitCode::PersistentStorageError)
        }
    }
}
