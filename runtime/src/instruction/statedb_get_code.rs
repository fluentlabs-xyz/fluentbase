use crate::{
    types::{Address, Bytes},
    RuntimeContext,
};
use fluentbase_rwasm::{common::Trap, Caller};
use fluentbase_types::ExitCode;

pub struct StateDbGetCode;

impl StateDbGetCode {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        address20_offset: u32,
        out_offset: u32,
        code_offset: u32,
        out_len: u32,
    ) -> Result<(), Trap> {
        if out_len <= 0 {
            return Ok(());
        }
        let address = caller.read_memory(address20_offset, 20).to_vec();
        let code = Self::fn_impl(caller.data_mut(), &address, code_offset, out_len)
            .map_err(|err| err.into_trap())?;
        let output_len_fact = core::cmp::min(code.len(), out_len as usize);
        caller.write_memory(out_offset, &code[0..output_len_fact]);
        Ok(())
    }

    pub fn fn_impl<T>(
        context: &mut RuntimeContext<T>,
        key: &[u8],
        code_offset: u32,
        out_len: u32,
    ) -> Result<Vec<u8>, ExitCode> {
        let account_db = context.account_db.clone().unwrap();
        let account = account_db
            .borrow_mut()
            .get_account(&Address::from_slice(key))
            .unwrap_or_default();
        let code = account.code.unwrap_or_default();
        let code = if code_offset < code.len() as u32 {
            Bytes::copy_from_slice(&code[code_offset as usize..])
        } else {
            Bytes::new()
        };

        let output_len_fact = core::cmp::min(out_len as usize, code.len());
        Ok(code[0..output_len_fact].to_vec())
    }
}
