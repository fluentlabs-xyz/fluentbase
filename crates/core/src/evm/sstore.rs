use crate::helpers::calc_storage_key;
use fluentbase_sdk::{
    AccountManager, ContextReader, LowLevelAPI, LowLevelSDK, JZKT_STORAGE_COMPRESSION_FLAGS,
};
use fluentbase_types::{Address, ExitCode, U256};

pub fn _evm_sstore<AM: AccountManager>(am: &AM, address: Address, slot: U256, value: U256) -> bool {
    am.write_storage(address, slot, value)
}
