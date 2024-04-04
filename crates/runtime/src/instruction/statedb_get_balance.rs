use crate::{types::Address, RuntimeContext};
use fluentbase_types::ExitCode;
use rwasm::{common::Trap, Caller};

pub struct StateDbGetBalance;

impl StateDbGetBalance {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        address20_offset: u32,
        out_balance32_offset: u32,
        is_self: u32,
    ) -> Result<(), Trap> {
        let balance = if is_self == 0 {
            let address = caller.read_memory(address20_offset, 20).to_vec();
            Self::fn_impl(caller.data_mut(), Some(&address)).map_err(|err| err.into_trap())?
        } else {
            Self::fn_impl(caller.data_mut(), None).map_err(|err| err.into_trap())?
        };
        caller.write_memory(out_balance32_offset, &balance[0..32]);
        Ok(())
    }

    pub fn fn_impl<T>(
        context: &mut RuntimeContext<T>,
        address20: Option<&[u8]>,
    ) -> Result<Vec<u8>, ExitCode> {
        let account_db = context.account_db.clone().unwrap();
        let address = address20
            .map(|val| Address::from_slice(val))
            .unwrap_or_else(|| context.caller);
        let account = account_db
            .borrow_mut()
            .get_account(&address)
            .unwrap_or_default();
        Ok(account.balance.to_be_bytes_vec())
    }
}
