use fluentbase_sdk::SovereignAPI;
use fluentbase_types::{Address, U256};

pub fn _evm_sload<AM: SovereignAPI>(am: &AM, address: Address, slot: U256) -> (U256, bool) {
    am.storage(&address, &slot, false)
}
