use core::ptr;
use fluentbase_sdk::{
    AccountManager, ContextReader, LowLevelAPI, LowLevelSDK, JZKT_ACCOUNT_BALANCE_FIELD,
};
use fluentbase_types::{Address, Bytes32, U256};

pub fn _evm_balance<AM: AccountManager>(am: &AM, address: Address) -> U256 {
    let (account, _) = am.account(address);
    account.balance
}
