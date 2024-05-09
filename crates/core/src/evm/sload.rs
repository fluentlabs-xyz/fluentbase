use fluentbase_sdk::AccountManager;
use fluentbase_types::{Address, U256};

pub fn _evm_sload<AM: AccountManager>(am: &AM, address: Address, slot: U256) -> (U256, bool) {
    am.storage(address, slot, false)
}
