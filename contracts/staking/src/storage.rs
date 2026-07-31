//! ERC-7201 storage layout.

use crate::consts::{
    CHAIN_CONFIG_STORAGE_SLOT, CONSENSUS_STORAGE_SLOT, INITIALIZER_STORAGE_SLOT,
    STAKING_STORAGE_SLOT,
};
use fluentbase_sdk::{
    derive::Storage,
    storage::{
        StorageAddress, StorageBool, StorageBytes, StorageBytes32, StorageMap, StorageU16,
        StorageU256, StorageU32, StorageU64, StorageU8, StorageUint112, StorageUint96, StorageVec,
    },
    Address, B256,
};

/// ERC-7201 namespaced initialization state.
#[derive(Storage)]
pub struct InitializerStorage {
    initialized: StorageBool,
}

/// ERC-7201 namespaced chain configuration.
#[derive(Storage)]
pub struct ChainConfigStorage {
    staking_token: StorageAddress,
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

/// ERC-7201 namespaced consensus, liveness, and equivocation state.
#[derive(Storage)]
pub struct ConsensusStorage {
    consensus_keys: StorageMap<Address, ConsensusKeysStorage>,
    peer_pubkey_owner: StorageMap<B256, StorageAddress>,
    epoch_committees: StorageMap<u64, StorageVec<StorageAddress>>,
    dkg_qual: StorageMap<u64, StorageBool>,
    last_committed_epoch_p1: StorageU64,
    pruned_up_to_p1: StorageU64,
    jailed_validators: StorageVec<StorageAddress>,
    jailed_scan_cursor: StorageU64,
    tombstoned: StorageMap<Address, StorageBool>,
    equivocation_commitments: StorageMap<Address, EquivocationCommitmentStorage>,
}

/// Single ERC-7201 namespaced storage root for staking.
///
/// Deriving the layout keeps packing and nested map/vector locations
/// deterministic while avoiding a slot constant and map type for every field.
#[derive(Storage)]
pub struct StakingStorage {
    owner: StorageAddress,
    validators: StorageMap<Address, ValidatorStorage>,
    owner_validators: StorageMap<Address, StorageAddress>,
    active_validators: StorageVec<StorageAddress>,
    selection_roster: StorageVec<StorageAddress>,
    selection_membership: StorageMap<Address, SelectionMembershipStorage>,
    validator_snapshots: StorageMap<Address, StorageMap<u64, ValidatorSnapshotStorage>>,
    validator_delegations: StorageMap<Address, StorageMap<Address, ValidatorDelegationStorage>>,
    credited_blend: StorageU256,
    last_rewarded_epoch_p1: StorageU64,
    /// Sorted epochs with materialized validator snapshots.
    ///
    /// Allows historical lookups to use binary search instead of scanning every
    /// intervening epoch.
    validator_snapshot_epochs: StorageMap<Address, StorageVec<StorageU64>>,
}

pub fn initializer_storage() -> InitializerStorage {
    InitializerStorage::new(INITIALIZER_STORAGE_SLOT, 0)
}

pub fn chain_config_storage() -> ChainConfigStorage {
    ChainConfigStorage::new(CHAIN_CONFIG_STORAGE_SLOT, 0)
}

pub fn consensus_storage() -> ConsensusStorage {
    ConsensusStorage::new(CONSENSUS_STORAGE_SLOT, 0)
}

pub fn staking_storage() -> StakingStorage {
    StakingStorage::new(STAKING_STORAGE_SLOT, 0)
}
