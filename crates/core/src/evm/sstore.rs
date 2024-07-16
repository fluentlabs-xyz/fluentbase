use fluentbase_sdk::SovereignAPI;
use fluentbase_types::{Address, U256};

pub fn _evm_sstore<AM: SovereignAPI>(am: &AM, address: Address, slot: U256, value: U256) -> bool {
    am.write_storage(&address, &slot, &value)
}
