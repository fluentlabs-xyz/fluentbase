use crate::{types::Address, RuntimeContext};
use fluentbase_rwasm::{common::Trap, Caller};
use fluentbase_types::ExitCode;

pub struct StateDbGetBalance;

impl StateDbGetBalance {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        address20_offset: u32,
        out_balance32_offset: u32,
        is_self: u32,
    ) -> Result<(), Trap> {
        let balance = {
            let is_self = is_self != 0;
            if is_self {
                let address = caller.read_memory(address20_offset, 20).to_vec();
                Self::fn_impl(caller.data_mut(), &address, is_self)
                    .map_err(|err| err.into_trap())?
            } else {
                Self::fn_impl(caller.data_mut(), &[], is_self).map_err(|err| err.into_trap())?
            }
        };
        caller.write_memory(out_balance32_offset, &balance[0..32]);
        Ok(())
    }

    pub fn fn_impl<T>(
        context: &mut RuntimeContext<T>,
        address20: &[u8],
        is_self: bool,
    ) -> Result<Vec<u8>, ExitCode> {
        let account_db = context.account_db.clone().unwrap();
        let account = if is_self {
            account_db
                .borrow_mut()
                .get_account(&context.caller)
                .unwrap_or_default()
        } else {
            account_db
                .borrow_mut()
                .get_account(&Address::from_slice(address20))
                .unwrap_or_default()
        };
        Ok(account.balance.to_be_bytes_vec())
    }
}
