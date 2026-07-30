//! ERC-7201 storage layout, epoch calculation, and validator-set helpers.

use fluentbase_sdk::{
    derive::Storage,
    storage::{
        StorageAddress, StorageBool, StorageBytes, StorageBytes32, StorageMap, StorageU16,
        StorageU256, StorageU32, StorageU64, StorageU8, StorageUint112, StorageUint96, StorageVec,
    },
    Address, ContextReader, ExitCode, SharedAPI, B256,
};

use crate::{consts::STAKING_STORAGE_SLOT, math};

pub const STATUS_NOT_FOUND: u8 = 0;
pub const STATUS_ACTIVE: u8 = 1;
pub const STATUS_PENDING: u8 = 2;
pub const STATUS_JAIL: u8 = 3;

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
    undelegate_period: StorageU64,
    dpos_activation_block: StorageU64,
    min_validator_stake_amount: StorageU256,
    min_staking_amount: StorageU256,
    felony_threshold: StorageU32,
    validator_jail_epoch_length: StorageU32,
    slash_reporter_reward_bps: StorageU32,
    slash_fund_address: StorageAddress,
    participation_floor_bps: StorageU32,
    participation_jail_disabled: StorageBool,
    blend_stipend_per_epoch: StorageU256,
    bls_verifier: StorageAddress,
    evidence_decoder: StorageAddress,
    min_undelegate_blocks: StorageU256,
    liveness_slashing: StorageAddress,
    blend_reserve: StorageAddress,
    configured: StorageBool,
}

/// Fixed-size validator metadata.
///
/// Epoch-varying stake, commission, and slash counters live exclusively in
/// `ValidatorSnapshotStorage`, avoiding duplicate sources of truth.
#[derive(Storage)]
pub struct ValidatorStorage {
    /// Immutable administrative, fee, and slashable self-stake identity.
    owner: StorageAddress,
    status: StorageU8,
    changed_at: StorageU64,
    jailed_before: StorageU64,
    claimed_at: StorageU64,
    /// First initialized snapshot epoch plus one (`0` means no snapshot).
    ///
    /// Appended to preserve the existing storage layout while bounding
    /// historical snapshot lookups.
    first_snapshot_epoch_p1: StorageU64,
}

/// Per-epoch validator accounting snapshot.
#[derive(Storage)]
pub struct ValidatorSnapshotStorage {
    /// Stake in `BALANCE_COMPACT_PRECISION` units.
    total_delegated: StorageUint112,
    slashes_count: StorageU32,
    commission_rate: StorageU16,
    /// Per-epoch BLEND reward in token base units; never copied forward.
    total_blend_rewards: StorageUint96,
}

/// Effective delegation balance beginning at `epoch`.
#[derive(Storage)]
pub struct DelegationOpStorage {
    /// Stake in `BALANCE_COMPACT_PRECISION` units.
    amount: StorageUint112,
    epoch: StorageU64,
}

/// Principal queued for release after the undelegation period.
#[derive(Storage)]
pub struct UndelegationOpStorage {
    /// Stake in `BALANCE_COMPACT_PRECISION` units.
    amount: StorageUint112,
    epoch: StorageU64,
}

/// Delegation history for one validator/delegator pair.
#[derive(Storage)]
#[allow(dead_code)]
pub struct ValidatorDelegationStorage {
    delegate_queue: StorageVec<DelegationOpStorage>,
    delegate_gap: StorageU64,
    undelegate_queue: StorageVec<UndelegationOpStorage>,
    undelegate_gap: StorageU64,
}

/// Epoch-stamped selection visibility. Status changes become visible from the
/// following epoch so an in-flight committee derivation cannot drift.
#[derive(Storage)]
pub struct SelectionMembershipStorage {
    visible: StorageBool,
    prev_visible: StorageBool,
    effective_from: StorageU64,
    rostered: StorageBool,
}

/// One validator's immutable v1 consensus identity.
#[derive(Storage)]
pub struct ConsensusKeysStorage {
    bls_pubkey: StorageBytes,
    peer_pubkey: StorageBytes32,
    activation_epoch: StorageU64,
}

/// One beneficiary's active equivocation-report commitment.
#[derive(Storage)]
pub struct EquivocationCommitmentStorage {
    commitment: StorageBytes32,
    committed_at: StorageU64,
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
    jailed_validators: StorageVec<StorageAddress>,
    selection_roster: StorageVec<StorageAddress>,
    selection_membership: StorageMap<Address, SelectionMembershipStorage>,
    validator_snapshots: StorageMap<Address, StorageMap<u64, ValidatorSnapshotStorage>>,
    validator_delegations: StorageMap<Address, StorageMap<Address, ValidatorDelegationStorage>>,
    consensus_keys: StorageMap<Address, ConsensusKeysStorage>,
    peer_pubkey_owner: StorageMap<B256, StorageAddress>,
    epoch_committees: StorageMap<u64, StorageVec<StorageAddress>>,
    dkg_qual: StorageMap<u64, StorageBool>,
    last_committed_epoch_p1: StorageU64,
    pruned_up_to_p1: StorageU64,
    tombstoned: StorageMap<Address, StorageBool>,
    credited_blend: StorageU256,
    last_rewarded_epoch_p1: StorageU64,
    /// Resumable cursor for bounded jailed-validator sweeps.
    ///
    /// Appended to preserve the existing ERC-7201 layout.
    jailed_scan_cursor: StorageU64,
    /// One active, beneficiary-owned commitment per reward account.
    ///
    /// Appended to preserve the existing ERC-7201 layout.
    equivocation_commitments: StorageMap<Address, EquivocationCommitmentStorage>,
}

pub fn staking_storage() -> StakingStorage {
    StakingStorage::new(STAKING_STORAGE_SLOT, 0)
}

pub fn current_epoch<SDK: SharedAPI>(sdk: &SDK) -> Result<u64, ExitCode> {
    current_epoch_at_block(sdk, sdk.context().block_number())
}

pub fn current_epoch_at_block<SDK: SharedAPI>(
    sdk: &SDK,
    block_number: u64,
) -> Result<u64, ExitCode> {
    let storage = staking_storage();
    let config = storage.config_accessor();
    let activation = config.dpos_activation_block_accessor().get_checked(sdk)?;
    let interval = config.epoch_block_interval_accessor().get_checked(sdk)?;
    math::epoch_at_block(block_number, activation, interval).ok_or(ExitCode::IntegerDivisionByZero)
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
