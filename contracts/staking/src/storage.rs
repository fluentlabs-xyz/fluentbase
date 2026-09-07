//! ERC-7201 storage layout.

use crate::consts::{
    BLS_PUBKEY_WORDS, CHAIN_CONFIG_STORAGE_SLOT, CONSENSUS_STORAGE_SLOT, INITIALIZER_STORAGE_SLOT,
    PRODUCTION_LIVENESS_STORAGE_SLOT, STAKING_STORAGE_SLOT, WEIGHT_RING_SLOTS,
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
    slash_fund_address: StorageAddress,
    blend_stipend_per_epoch: StorageU256,
    bls_verifier: StorageAddress,
    min_undelegate_blocks: StorageU256,
    /// Address the epoch stipend is drawn from, not a contract implementing a
    /// reserve interface: a claim pulls with `transferFrom`, so any holder that
    /// has approved this contract works — a wallet, a multisig, a treasury.
    ///
    /// Revoking that approval no longer merely pauses payment. The epoch close
    /// reads `min(balanceOf, allowance)` before it prices an epoch and forfeits
    /// the epoch outright when that is short, so every epoch closing inside the
    /// revoked window is worth zero for good. Only already-accrued epochs wait
    /// for the approval to come back.
    blend_reserve: StorageAddress,
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
    undelegate_queue: StorageVec<UndelegationOpStorage>,
    undelegate_gap: StorageU64,
    /// Unclaimed queued principal in full-precision token units.
    ///
    /// Keeping the aggregate beside the operation history lets equivocation
    /// seizure remain constant-time even when the queue is fragmented.
    pending_undelegated: StorageU256,
    /// Exclusive epoch through which rewards have been paid.
    ///
    /// Separate from `DelegationOpStorage::epoch`, which is the epoch a balance
    /// takes effect from and must stay immutable: historical stake lookups
    /// binary-search that field, so advancing it as a payment cursor rewrites
    /// past-epoch committee views.
    claimed_through_epoch: StorageU64,
}

/// Epoch-stamped selection visibility. Status changes become visible from the
/// following epoch so an in-flight committee derivation cannot drift.
///
/// The record carries three transitions, not one: a single `prev_visible` is
/// only correct until the second stamp overwrites it, after which epochs below
/// the first stamp start answering with the value that took effect between the
/// two. Committed committees read two selection epochs back, so a governance
/// pair in consecutive epochs was enough to rewrite an epoch already in force.
///
/// All three pairs share one slot, so the extra depth costs no extra store.
#[derive(Storage)]
pub struct SelectionMembershipStorage {
    visible: StorageBool,
    prev_visible: StorageBool,
    effective_from: StorageU64,
    /// Epoch from which `prev_visible` took effect.
    ///
    /// Appended, never inserted: declaration order is the storage layout.
    prev_from: StorageU64,
    /// Visibility that was in force before `prev_visible`.
    prev2_visible: StorageBool,
    /// Epoch from which `prev2_visible` took effect. Epochs below it answer
    /// `false`: the history is exactly three transitions deep.
    prev2_from: StorageU64,
}

/// One validator's immutable v1 consensus identity.
#[derive(Storage)]
pub struct ConsensusKeysStorage {
    /// Compressed 96-byte BLS12-381 G2 key, stored without dynamic-bytes metadata.
    bls_pubkey: StorageArray<StorageBytes32, { BLS_PUBKEY_WORDS }>,
    peer_pubkey: StorageBytes32,
    activation_epoch: StorageU64,
}

/// Which membership record an epoch seats, and how many of its entries.
///
/// **The length is duplicated for the hot path, and for nothing else.**
/// `record_production` reads it on every block and would otherwise pay a second
/// lookup into the record; `committee_length_at` touches this field alone.
///
/// It is NOT here because it can disagree with the record. An earlier version of
/// this comment claimed "a later epoch may seat fewer members than the record
/// holds", and that state is **unreachable**: `committee_changed` returns `false`
/// only when the incumbent length matches AND every position matches, so every
/// epoch sharing a record was committed with exactly that record's length. The
/// equality holds by induction over the single writer. Readers still bound by
/// THIS field — it is free and it does not depend on the record — but a test can
/// never fail on the difference, and a justification that names an impossible
/// state teaches the next reader something false.
///
/// `length == 0` keeps its existing meaning — not yet committed. `record == 0`
/// does **not** mean absence: genesis mints record 0, so zero is a real pointer.
/// The record key is the minting epoch narrowed to `u32`; epochs at or beyond
/// 2^32 would resolve to a different record, which is unreachable at any block
/// rate this chain will see.
#[derive(Storage)]
pub struct EpochIndexStorage {
    record: StorageU32,
    length: StorageU32,
}

/// Two members' frozen leader weights and the epoch they belong to, in one slot.
///
/// `14 + 14 + 4 = 32`, so the derive gives `SLOTS == 1` and the stamp costs
/// nothing. Sharing the slot is what makes a torn state unrepresentable: no
/// write ordering can leave the stamp disagreeing with the weights it vouches
/// for, because they are the same store.
///
/// The stamp is the epoch truncated to `u32`. Two epochs that alias are 2^32
/// apart — at an 86,400-block epoch, longer than the chain will exist — and the
/// ring only ever has to tell `E` apart from `E − 16`, `E − 32`, …
#[derive(Storage)]
pub struct WeightPairStorage {
    a: StorageUint112,
    b: StorageUint112,
    stamp: StorageU32,
}

/// ERC-7201 namespaced consensus, committee, and equivocation state.
#[derive(Storage)]
pub struct ConsensusStorage {
    consensus_keys: StorageMap<Address, ConsensusKeysStorage>,
    peer_pubkey_owner: StorageMap<B256, StorageAddress>,
    /// Ordered member addresses, keyed by the epoch that minted them and
    /// appended only when the committee actually changes.
    ///
    /// Never pruned: signature verification, slashing identity and deep catch-up
    /// all resolve at arbitrary depth, and this record is the only thing that
    /// makes that possible. Peer-key ascending, enforced at commit — and the
    /// order stays meaningful for all time only because the sort key is
    /// immutable, which is half of the alignment proof and the half that is easy
    /// to lose.
    committee_records: StorageMap<u64, StorageVec<StorageAddress>>,
    epoch_index: StorageMap<u64, EpochIndexStorage>,
    /// Frozen leader weights for the last `WEIGHT_RING_EPOCHS` epochs, at
    /// `(E mod N) * PAIRS_MAX + i / 2`.
    weight_ring: StorageArray<WeightPairStorage, WEIGHT_RING_SLOTS>,
    dkg_qual: StorageMap<u64, StorageBool>,
    last_committed_epoch_p1: StorageU64,
    tombstoned: StorageMap<Address, StorageBool>,
    /// Validator owning a canonical compressed BLS key, indexed by its keccak256 hash.
    ///
    /// Consensus identities are immutable in v1, so ownership is never released.
    bls_pubkey_owner: StorageMap<B256, StorageAddress>,
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
    selection_membership: StorageMap<Address, SelectionMembershipStorage>,
    validator_snapshots: StorageMap<Address, StorageMap<u64, ValidatorSnapshotStorage>>,
    validator_delegations: StorageMap<Address, StorageMap<Address, ValidatorDelegationStorage>>,
    /// Sorted epochs with materialized validator snapshots.
    ///
    /// Allows historical lookups to use binary search instead of scanning every
    /// intervening epoch.
    validator_snapshot_epochs: StorageMap<Address, StorageVec<StorageU64>>,
}

/// One validator's block-production record.
///
/// Every field here is an epoch-close write. The per-block credit touches none
/// of them — it keys its counter by committee index instead — so the record is
/// read and written once per epoch at most, and fits a single slot.
#[derive(Storage)]
pub struct ProductionValidatorStorage {
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
    /// different validator in every epoch, and safely so: an epoch's committee
    /// is committed once before the epoch starts and never mutated inside it, so
    /// the index cannot come to mean someone else while the counter is live.
    ///
    /// Never pruned, deliberately. Nothing reads an epoch's entries after its
    /// close — in a devnet build the `producedAt` getter can, in a production
    /// build nothing can — and clearing them would cost one store per committee
    /// member on the close path, the most expensive path in the tier, to reclaim
    /// about fifty slots a day at the production epoch interval.
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
