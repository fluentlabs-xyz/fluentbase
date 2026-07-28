use fluentbase_sdk::{
    derive::Storage,
    storage::{
        StorageAddress, StorageBool, StorageMap, StorageU16, StorageU256, StorageU32, StorageU64,
        StorageU8, StorageVec,
    },
    Address, ContextReader, ExitCode, SharedAPI,
};

use crate::{consts::STAKING_STORAGE_SLOT, math};

pub const STATUS_NOT_FOUND: u8 = 0;
pub const STATUS_ACTIVE: u8 = 1;
pub const STATUS_PENDING: u8 = 2;

/// Configuration formerly read from the external Solidity `ChainConfig`.
///
/// The rWasm staking contract owns this state so consensus-critical reads do
/// not require a second contract call.
#[derive(Storage)]
pub struct ChainConfigStorage {
    staking_token: StorageAddress,
    governance: StorageAddress,
    active_validators_length: StorageU64,
    epoch_block_interval: StorageU64,
    dpos_activation_block: StorageU64,
    min_validator_stake_amount: StorageU256,
    min_staking_amount: StorageU256,
}

/// Fixed-size validator metadata and its latest accounting snapshot.
///
/// Queue/snapshot history will be added as separate derived storage types in
/// the delegation slice.
#[derive(Storage)]
pub struct ValidatorStorage {
    owner: StorageAddress,
    status: StorageU8,
    changed_at: StorageU64,
    jailed_before: StorageU64,
    claimed_at: StorageU64,
    commission_rate: StorageU16,
    slashes_count: StorageU32,
    total_delegated: StorageU256,
}

/// Single ERC-7201 namespaced storage root for staking.
///
/// Deriving the layout keeps packing and nested map/vector locations
/// deterministic while avoiding a slot constant and map type for every field.
#[derive(Storage)]
pub struct StakingStorage {
    initialized: StorageBool,
    owner: StorageAddress,
    config: ChainConfigStorage,
    validators: StorageMap<Address, ValidatorStorage>,
    owner_validators: StorageMap<Address, StorageAddress>,
    active_validators: StorageVec<StorageAddress>,
}

pub fn staking_storage() -> StakingStorage {
    StakingStorage::new(STAKING_STORAGE_SLOT, 0)
}

pub fn current_epoch<SDK: SharedAPI>(sdk: &SDK) -> Result<u64, ExitCode> {
    let storage = staking_storage();
    let config = storage.config_accessor();
    let activation = config.dpos_activation_block_accessor().get_checked(sdk)?;
    let interval = config.epoch_block_interval_accessor().get_checked(sdk)?;
    math::epoch_at_block(sdk.context().block_number(), activation, interval)
        .ok_or(ExitCode::IntegerDivisionByZero)
}

pub fn remove_active<SDK: SharedAPI>(sdk: &mut SDK, validator: Address) -> Result<(), ExitCode> {
    let active = staking_storage().active_validators_accessor();
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
