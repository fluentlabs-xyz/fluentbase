use crate::helpers::calc_storage_key;
use fluentbase_sdk::{AccountManager, ContextReader, LowLevelAPI, LowLevelSDK};
use fluentbase_types::{Address, ExitCode, U256};

pub fn _evm_sload<AM: AccountManager>(am: &AM, address: Address, slot: U256) -> (U256, bool) {
    am.storage(address, slot, false)
}
