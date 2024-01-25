use crate::{types::Address, RuntimeContext};
use fluentbase_rwasm::{common::Trap, Caller};
use fluentbase_types::ExitCode;

pub struct StateDbGetCode;

impl StateDbGetCode {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        key20_offset: u32,
        output_offset: u32,
        output_len: u32,
    ) -> Result<(), Trap> {
        let address = caller.read_memory(key20_offset, 20).to_vec();
        let code = Self::fn_impl(caller.data_mut(), &address, output_len)
            .map_err(|err| err.into_trap())?;
        caller.write_memory(output_offset, &code);
        Ok(())
    }

    pub fn fn_impl<T>(
        context: &mut RuntimeContext<T>,
        key: &[u8],
        output_len: u32,
    ) -> Result<Vec<u8>, ExitCode> {
        let account_db = context.account_db.clone().unwrap();
        let account = account_db
            .borrow_mut()
            .get_account(&Address::from_slice(key))
            .unwrap_or_default();
        let code = account.code.unwrap_or_default();
        if code.len() <= output_len as usize {
            Ok(code.to_vec())
        } else {
            Err(ExitCode::OutputOverflow)
        }
    }
}
