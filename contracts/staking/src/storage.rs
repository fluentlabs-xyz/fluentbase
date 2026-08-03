//! ERC-7201 storage layout.

use crate::consts::{
    CHAIN_CONFIG_STORAGE_SLOT, CONSENSUS_STORAGE_SLOT, INITIALIZER_STORAGE_SLOT,
    PRODUCTION_LIVENESS_STORAGE_SLOT, STAKING_STORAGE_SLOT,
};
use fluentbase_sdk::{
    derive::Storage,
    storage::{
        StorageAddress, StorageArray, StorageBool, StorageBytes32, StorageMap, StorageU16,
        StorageU256, StorageU32, StorageU64, StorageU8, StorageUint112, StorageUint96, StorageVec,
    },
    Address, B256,
};

/// ERC-7201 namespaced initialization state.
#[derive(Storage)]
pub struct InitializerStorage {
    initialized: StorageBool,
    initializing: StorageBool,
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
    slash_reporter_reward_bps: StorageU32,
    slash_fund_address: StorageAddress,
    blend_stipend_per_epoch: StorageU256,
    bls_verifier: StorageAddress,
    evidence_decoder: StorageAddress,
    min_undelegate_blocks: StorageU256,
    /// Unread. The jail tier that consumed it is gone and the production-liveness
    /// tier that replaced it runs inside this contract, so the principal is the
    /// contract itself. Still required non-zero at initialization, and the
    /// initializer's selector is pinned, so removing it is a deliberate ABI break.
    liveness_slashing: StorageAddress,
    blend_reserve: StorageAddress,
    /// Committee size cap history, ascending by `from_epoch`.
    ///
    /// Appended, never inserted: declaration order is the storage layout.
    cap_checkpoints: StorageVec<CapCheckpointStorage>,
    min_verdict_due_blocks: StorageU32,
    exclusion_backoff_cap: StorageU32,
    /// Kill switch for the production-liveness tier, seeded `true` at init.
    ///
    /// Raw, never sentinel-on-zero: a fresh slot reads `false`, which is the
    /// opposite of the intended default, so the seed is the only thing keeping
    /// the tier off on a new chain.
    production_liveness_disabled: StorageBool,
}

/// Fixed-size validator metadata.
///
/// Epoch-varying stake and commission live exclusively in
/// `ValidatorSnapshotStorage`, avoiding duplicate sources of truth.
#[derive(Storage)]
pub struct ValidatorStorage {
    /// Immutable administrative, fee, and slashable self-stake identity.
    owner: StorageAddress,
    status: StorageU8,
    changed_at: StorageU64,
    claimed_at: StorageU64,
    /// First initialized snapshot epoch plus one (`0` means no snapshot).
    ///
    /// Appended to preserve the existing storage layout while bounding
    /// historical snapshot lookups.
    first_snapshot_epoch_p1: StorageU64,
    /// Latest exclusive unlock epoch among queued self-principal operations.
    ///
    /// New owner undelegations extend it; a complete claim or equivocation
    /// seizure clears it. Claim accounting uses each operation's own deadline.
    self_stake_unlock_epoch: StorageU64,
}

/// Per-epoch validator accounting snapshot.
#[derive(Storage)]
pub struct ValidatorSnapshotStorage {
    /// Stake in `BALANCE_COMPACT_PRECISION` units.
    total_delegated: StorageUint112,
    commission_rate: StorageU16,
    /// Per-epoch BLEND reward in token base units; never copied forward.
    total_blend_rewards: StorageUint96,
}

/// Committee size cap in force from `from_epoch` onward.
///
/// The epoch-frozen selection view stands on three epoch-addressed legs:
/// visibility, stake, and this cap. Reading the cap live was the missing leg —
/// a governance change would retroactively rewrite the committee of an epoch
/// that had already been committed.
#[derive(Storage)]
pub struct CapCheckpointStorage {
    from_epoch: StorageU64,
    value: StorageU32,
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
    /// Exclusive epoch when validator-owner principal stops being slashable.
    /// Zero for ordinary delegators.
    self_stake_unlock_epoch: StorageU64,
}

/// Delegation history for one validator/delegator pair.
#[derive(Storage)]
#[allow(dead_code)]
pub struct ValidatorDelegationStorage {
    delegate_queue: StorageVec<DelegationOpStorage>,
    delegate_gap: StorageU64,
    undelegate_queue: StorageVec<UndelegationOpStorage>,
    undelegate_gap: StorageU64,
    /// Unclaimed queued principal in full-precision token units.
    ///
    /// Keeping the aggregate beside the operation history lets equivocation
    /// seizure remain constant-time even when the queue is fragmented.
    pending_undelegated: StorageU256,
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
    /// Compressed 96-byte BLS12-381 G2 key, stored without dynamic-bytes metadata.
    bls_pubkey: StorageArray<StorageBytes32, 3>,
    peer_pubkey: StorageBytes32,
    activation_epoch: StorageU64,
}

/// One beneficiary's active equivocation-report commitment.
#[derive(Storage)]
pub struct EquivocationCommitmentStorage {
    commitment: StorageBytes32,
    committed_at: StorageU64,
}

/// ERC-7201 namespaced consensus, committee, and equivocation state.
#[derive(Storage)]
pub struct ConsensusStorage {
    consensus_keys: StorageMap<Address, ConsensusKeysStorage>,
    peer_pubkey_owner: StorageMap<B256, StorageAddress>,
    epoch_committees: StorageMap<u64, StorageVec<StorageAddress>>,
    dkg_qual: StorageMap<u64, StorageBool>,
    last_committed_epoch_p1: StorageU64,
    pruned_up_to_p1: StorageU64,
    tombstoned: StorageMap<Address, StorageBool>,
    equivocation_commitments: StorageMap<Address, EquivocationCommitmentStorage>,
    /// Validator owning a canonical compressed BLS key, indexed by its keccak256 hash.
    ///
    /// Consensus identities are immutable in v1, so ownership is never released.
    bls_pubkey_owner: StorageMap<B256, StorageAddress>,
    /// Exclusive equivocation-evidence deadline snapshotted for each committee.
    committee_liability_end_epochs: StorageMap<u64, StorageU64>,
    /// Leader weights stamped at commit time, positional with `epoch_committees`.
    ///
    /// Stored in `BALANCE_COMPACT_PRECISION` units. Computing the weight live at
    /// read time makes it depend on the block height each node happens to read
    /// at, and the leader is drawn from those weights — so an unfrozen weight is
    /// a per-node leader split, not an accounting rounding error.
    leader_stakes: StorageMap<u64, StorageVec<StorageUint112>>,
}

/// Single ERC-7201 namespaced storage root for staking.
///
/// Deriving the layout keeps packing and nested map/vector locations
/// deterministic while avoiding a slot constant and map type for every field.
#[derive(Storage)]
pub struct StakingStorage {
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

/// One validator's block-production record.
///
/// Field order is split by write frequency: the two counters the per-block
/// credit touches share one slot, so recording a block costs a single store no
/// matter how the epoch-close fields below them grow.
#[derive(Storage)]
pub struct ProductionValidatorStorage {
    total_produced: StorageU64,
    /// Epoch of the most recent credited block plus one (`0` means never).
    last_produced_epoch_p1: StorageU64,
    /// Epoch of the most recent failing verdict plus one (`0` means never).
    last_failed_epoch_p1: StorageU64,
    /// Epoch at whose close the exclusion is released (`0` means not excluded).
    readmit_at_epoch: StorageU64,
    /// Exclusion episodes, not verdicts. Never decays.
    kick_count: StorageU32,
}

/// ERC-7201 namespaced block-production accounting.
#[derive(Storage)]
pub struct ProductionLivenessStorage {
    /// Highest recorded block: both the idempotency belt and the epoch cursor.
    ///
    /// There is deliberately no second epoch scalar. The epoch is a pure
    /// function of the height, and two cursors obliged to agree can disagree.
    last_processed_block: StorageU64,
    /// Blocks credited per (epoch, committee index).
    ///
    /// Keyed by index rather than by address because index `i` names a
    /// different validator in every epoch.
    produced: StorageMap<u64, StorageMap<u32, StorageU32>>,
    blocks_in_epoch: StorageMap<u64, StorageU32>,
    /// Live exclusions; the length is the concurrent count.
    pending_exclusions: StorageVec<StorageAddress>,
    validators: StorageMap<Address, ProductionValidatorStorage>,
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

pub fn production_liveness_storage() -> ProductionLivenessStorage {
    ProductionLivenessStorage::new(PRODUCTION_LIVENESS_STORAGE_SLOT, 0)
}
