use fluentbase_sdk::{
    storage::{
        StorageAddress, StorageBool, StorageDescriptor, StorageMap, StorageU16, StorageU256,
        StorageU32, StorageU64, StorageU8, StorageVec,
    },
    Address, ContextReader, StorageAPI, U256,
};

use crate::consts::*;

pub const STATUS_NOT_FOUND: u8 = 0;
pub const STATUS_ACTIVE: u8 = 1;
pub const STATUS_PENDING: u8 = 2;

pub type AddressMap = StorageMap<Address, StorageAddress>;
pub type U8Map = StorageMap<Address, StorageU8>;
pub type U16Map = StorageMap<Address, StorageU16>;
pub type U32Map = StorageMap<Address, StorageU32>;
pub type U64Map = StorageMap<Address, StorageU64>;
pub type U256Map = StorageMap<Address, StorageU256>;

pub fn initialized<SDK: StorageAPI>(sdk: &SDK) -> Result<bool, fluentbase_sdk::ExitCode> {
    StorageBool::new(INITIALIZED_SLOT, 31).get_checked(sdk)
}

pub fn owner<SDK: StorageAPI>(sdk: &SDK) -> Result<Address, fluentbase_sdk::ExitCode> {
    StorageAddress::new(OWNER_SLOT, 12).get_checked(sdk)
}

pub fn status<SDK: StorageAPI>(
    sdk: &SDK,
    validator: Address,
) -> Result<u8, fluentbase_sdk::ExitCode> {
    U8Map::new(VALIDATOR_STATUS_SLOT)
        .entry(validator)
        .get_checked(sdk)
}

pub fn current_epoch<SDK: fluentbase_sdk::SystemAPI>(
    sdk: &SDK,
) -> Result<u64, fluentbase_sdk::ExitCode> {
    let activation = StorageU64::new(ACTIVATION_BLOCK_SLOT, 24).get_checked(sdk)?;
    let interval = StorageU64::new(EPOCH_INTERVAL_SLOT, 24).get_checked(sdk)?;
    crate::math::epoch_at_block(sdk.context().block_number(), activation, interval)
        .ok_or(fluentbase_sdk::ExitCode::IntegerDivisionByZero)
}

pub fn active_validators() -> StorageVec<StorageAddress> {
    StorageVec::new(ACTIVE_VALIDATORS_SLOT)
}

pub fn remove_active<SDK: StorageAPI>(
    sdk: &mut SDK,
    validator: Address,
) -> Result<(), fluentbase_sdk::ExitCode> {
    let active = active_validators();
    let len = active.len_checked(sdk)?;
    for index in 0..len {
        if active.at(index).get_checked(sdk)? != validator {
            continue;
        }
        if index + 1 != len {
            let last = active.at(len - 1).get_checked(sdk)?;
            active.at(index).set_checked(sdk, last)?;
        }
        active.pop_checked(sdk)?;
        break;
    }
    Ok(())
}

pub fn total_delegated<SDK: StorageAPI>(
    sdk: &SDK,
    validator: Address,
) -> Result<U256, fluentbase_sdk::ExitCode> {
    U256Map::new(VALIDATOR_TOTAL_DELEGATED_SLOT)
        .entry(validator)
        .get_checked(sdk)
}
