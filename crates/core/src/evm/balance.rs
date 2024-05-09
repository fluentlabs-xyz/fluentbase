use fluentbase_sdk::AccountManager;
use fluentbase_types::{Address, U256};

pub fn _evm_balance<AM: AccountManager>(am: &AM, address: Address) -> U256 {
    let (account, _) = am.account(address);
    account.balance
}
