//! The committee of an epoch as ONE frozen value, read at ONE anchor.
//!
//! Before this module every consumer that needed `committee[E]` resolved its
//! own block hash and issued its own `eth_call` — six different cursor policies
//! across the node, the follower and the stand. They agreed by luck: the
//! contract freezes membership, keys and weights at the epoch commit, so
//! *usually* any in-window hash answers the same thing. "Usually" is not an
//! invariant, and two authoritative copies of one epoch's committee is the
//! defect class this module exists to close.
//!
//! What it replaces them with:
//!
//! * **One value.** [`CommitteeRecord`] is the whole answer — members, weights,
//!   the `dkgQual` bit, and the participant/BLS projections derived once from
//!   the members. It is WRITE-ONCE: the first successful read of an epoch is
//!   the answer for as long as that epoch is readable at all, and a second read
//!   that disagrees is a contract fork, reported and refused rather than
//!   merged. Retention is the window below and nothing narrower, so there is no
//!   band of epochs the module still answers but no longer recognises.
//! * **One anchor.** Every read is taken at `executed_state_hash(anchor)` where
//!   `anchor` is this node's own ordering-finalized tip ([`Anchor`]). That
//!   height moves, and moving it is safe because of the window below.
//! * **One window.** An epoch is readable only inside
//!   `[epoch(anchor) − SCHEME_RETENTION_EPOCHS, epoch(anchor) +
//!   MAX_COMMITTEE_LOOKAHEAD_EPOCHS]`. Below it the contract's weight ring has
//!   wrapped and the weights are gone; above it the epoch is not committed yet.
//!   Inside it the record's VALUE does not depend on which in-window anchor a
//!   node happens to hold, which is what lets two nodes at different heights
//!   produce byte-identical records.
//! * **One error type.** [`CommitteeError`] separates "not yet" (retry on the
//!   next tip) from "not here" (a window predicate, refused without touching
//!   the EVM) from "the read failed", and only the last carries a retry
//!   decision — [`fluentbase_staking_reader::ReadError::is_transient`].
//!
//! ## Why the weights are NOT an `Option`
//!
//! The contract keeps frozen leader weights in a ring of
//! [`WEIGHT_RING_EPOCHS`] frames and answers an empty `stakes` leg once a
//! frame has been reused, which the reader decodes as
//! [`ValidatorSetSnapshot::weights`]` == None`. The newest epoch committed at
//! the anchor is `epoch(anchor) + MAX_COMMITTEE_LOOKAHEAD_EPOCHS`, and frame
//! `E mod WEIGHT_RING_EPOCHS` is not overwritten before `E +
//! WEIGHT_RING_EPOCHS`, so every epoch strictly above `epoch(anchor) +
//! MAX_COMMITTEE_LOOKAHEAD_EPOCHS − WEIGHT_RING_EPOCHS` still has its weights.
//! The window's own floor is `epoch(anchor) − SCHEME_RETENTION_EPOCHS`, which
//! is higher — `14 > 8` — so inside the window the weights are always there
//! and `weights: None` means the contract answered something impossible. That
//! is [`WINDOW_FITS_THE_WEIGHT_RING`], a compile-time assertion rather than a
//! comment, because the three constants live in three different crates.
//!
//! ## What is deliberately NOT here
//!
//! `tombstoned`. It is the ONE leg of the snapshot the contract does not
//! freeze — it is read live at the snapshot's own block so a mid-epoch
//! equivocation verdict reaches the committee it names — so putting it in a
//! frozen record would either freeze a verdict that had not landed yet or make
//! two honest nodes disagree about a "frozen" value. It stays a separate
//! liveness layer; nothing in this module reads it, and
//! [`Committee::is_member`] deliberately does not (a tombstoned validator is
//! still a member of the committee it was elected to — what it loses is
//! leadership and its connection, not its seat).
//!
//! `Member::activation_epoch`. No consumer in the core reads it.

use alloy_primitives::{Address, B256};
use fluentbase_bls::{scheme::EpochCommittee, BlsPubkey, PeerPubkey};
use fluentbase_staking_reader::{reader::ValidatorSetSnapshot, ReadError};
use fluentbase_types::staking_protocol::{
    epoch_at_block, MAX_COMMITTEE_LOOKAHEAD_EPOCHS, WEIGHT_RING_EPOCHS,
};
use std::{num::NonZeroU64, sync::Arc};

mod facade;
mod store;
#[cfg(test)]
mod tests;

pub use facade::CommitteeReadsFacade;
pub use store::{CommitteeStore, RethAnchor};

use crate::SCHEME_RETENTION_EPOCHS;

/// The inequality that makes [`CommitteeRecord::weights`] a `Vec` and not an
/// `Option<Vec>`: every epoch this module will read still has its weights in
/// the contract's ring.
///
/// `16 − 2 > 8`. Shrinking the contract's ring, widening the commit horizon or
/// lengthening the node's scheme retention past this point does not produce a
/// run-time `None` for the leader elector to invent a uniform lottery from — it
/// produces a build failure here.
pub const WINDOW_FITS_THE_WEIGHT_RING: () = assert!(
    WEIGHT_RING_EPOCHS - MAX_COMMITTEE_LOOKAHEAD_EPOCHS > SCHEME_RETENTION_EPOCHS as u64,
    "the committee read window reaches epochs whose frozen weights the contract's ring has \
     already overwritten — CommitteeRecord::weights can no longer be non-optional"
);

/// One committee member, as the contract froze it.
///
/// `tombstoned` is NOT here (see the module docs) and neither is
/// `activation_epoch`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub address: Address,
    pub peer: PeerPubkey,
    pub bls: BlsPubkey,
}

/// One epoch's committee, frozen. The whole answer, read once.
///
/// `members` is in CONTRACT ORDER, verbatim — the reader never sorts, and that
/// order is the consensus index space the slasher resolves an accused position
/// in. `participants` and `bls` are the two projections consumers actually
/// consume, derived once here rather than rebuilt per call (commonware re-sorts
/// into its own `BiMap` order, which is NOT contract order).
#[derive(Clone, Debug)]
pub struct CommitteeRecord {
    pub epoch: u64,
    pub members: Vec<Member>,
    /// Frozen leader weights, one per member, same order. Non-optional by
    /// [`WINDOW_FITS_THE_WEIGHT_RING`].
    pub weights: Vec<u128>,
    /// `getDkgQual(epoch)` — whether this epoch's committee differs from the
    /// previous one, hence whether its DKG re-mints the beacon key. Read as a
    /// SECOND staticcall at the SAME hash as the snapshot, because
    /// `getEpochCommitteeWithStakes` does not carry the bit.
    pub changed: bool,
    /// `(block_number, block_hash)` of the anchor this record was read at.
    /// DIAGNOSTIC ONLY — two nodes with different anchors hold the same record
    /// with different `snapshot`s, which is exactly why
    /// [`Self::same_value`] ignores it.
    pub snapshot: (u64, B256),
    /// The member peer keys as the ceremony roster / peer-set projection.
    pub participants: commonware_utils::ordered::Set<PeerPubkey>,
    /// The same committee with its BLS half, in the participant BiMap a
    /// certificate is verified under.
    pub bls: EpochCommittee,
}

impl CommitteeRecord {
    /// Whether two records describe the same committee — everything the
    /// contract froze, and nothing about WHERE it was read.
    ///
    /// This is the write-once predicate and the cross-node equality predicate
    /// at once, and it has to skip [`Self::snapshot`] for both: a record read
    /// at height 100 and the same record read at height 140 are the same
    /// answer. `participants` and `bls` are functions of `members`, so
    /// comparing `members` covers them.
    pub fn same_value(&self, other: &Self) -> bool {
        self.epoch == other.epoch
            && self.members == other.members
            && self.weights == other.weights
            && self.changed == other.changed
    }
}

/// Every way a committee read can fail to produce a record.
///
/// Three variants, and the split is by what the CALLER must do, not by where
/// the failure happened.
#[derive(Debug, thiserror::Error)]
pub enum CommitteeError {
    /// The epoch is inside the window but this node cannot see it yet: the
    /// anchor is below the height that first commits it, the anchor height is
    /// not executed yet, or the contract answered an empty committee.
    ///
    /// Never a fault. The caller retries on the next tip. `ready_at` is the
    /// LOWEST ANCHOR HEIGHT AT WHICH A RETRY CAN DO SOMETHING THIS ONE DID NOT
    /// — which is the one sentence covering all three arms: `commit_height(E)`
    /// when the epoch is not committed in any state this anchor covers, the
    /// CURRENT anchor height when the height is right but its state is not
    /// executed yet (the retry waits on execution, not on the cursor), and
    /// `anchor + 1` when the contract answered an empty committee at this very
    /// height (only a later state can hold the commit). It is a LOWER BOUND in
    /// the first arm too, since the ahead-commit drain only runs once the chain
    /// has a staking address. The wake-up to park on is
    /// [`Committee::subscribe`]; `ready_at` is a diagnostic hint, never a
    /// timer.
    #[error("committee[{epoch}] is not readable at this anchor yet (ready at height {ready_at})")]
    NotReadable { epoch: u64, ready_at: u64 },

    /// The epoch is outside `[lo, hi]`. Refused WITHOUT an EVM call — this is a
    /// predicate on the request, lifted into the type so that no caller can
    /// forget to apply it (a p2p frame naming an arbitrary epoch, a jump target
    /// far ahead).
    #[error("committee[{epoch}] is outside the readable window [{lo}, {hi}]")]
    OutOfWindow { epoch: u64, lo: u64, hi: u64 },

    /// The read itself failed. [`Self::is_transient`] decides whether a retry
    /// can help.
    #[error("committee read failed: {0}")]
    Read(#[from] ReadError),
}

impl CommitteeError {
    /// Whether retrying the same request can legitimately succeed later.
    ///
    /// [`Self::NotReadable`] is transient by construction: the anchor only
    /// moves up.
    ///
    /// [`Self::OutOfWindow`] is DIRECTIONAL, and that is the whole reason the
    /// variant carries `hi`. An epoch above the window is one the anchor has
    /// not reached yet — a node catching up asks for `now + 2` from a cursor
    /// that lags execution, and the epoch becomes readable in a block or two
    /// without anyone doing anything, so a consumer routing on this predicate
    /// (§5.4 prescribes exactly that for the slasher) must retry. An epoch
    /// below the window is one the contract's weight ring has already
    /// overwritten: no anchor will ever bring it back, and retrying is the
    /// eternal spin of a stuck slashing charge that §5.4 requires to end in
    /// `Permanent`. One predicate cannot be right for both unless it looks at
    /// the side.
    ///
    /// A failed read defers to [`ReadError::is_transient`].
    pub fn is_transient(&self) -> bool {
        match self {
            Self::NotReadable { .. } => true,
            Self::OutOfWindow { epoch, hi, .. } => epoch > hi,
            Self::Read(e) => e.is_transient(),
        }
    }
}

/// The committee of an epoch, frozen — the whole read surface of this module.
///
/// Synchronous, because the reads underneath are blocking reth state reads and
/// pretending otherwise would only move the blocking somewhere less visible;
/// this is what the four closures and three `CommitteeReads` implementations it
/// replaces already did.
pub trait Committee: Send + Sync {
    /// `committee[epoch]`, read once and then held for as long as the epoch is
    /// inside the read window — which is exactly as long as this trait will
    /// answer for it at all.
    fn committee(&self, epoch: u64) -> Result<Arc<CommitteeRecord>, CommitteeError>;

    /// `dkgQual[epoch]` — whether this epoch re-mints the beacon key. Same
    /// record, same read; never a second staticcall.
    fn changed(&self, epoch: u64) -> Result<bool, CommitteeError>;

    /// Whether `peer` sits in `committee[epoch]`.
    ///
    /// Does NOT consider `tombstoned`: a tombstoned validator keeps its seat in
    /// the frozen committee it was elected to (the certificate bitmap and the
    /// slasher's positional index both depend on that), and what a tombstone
    /// takes away — leadership, proposals, the connection — is enforced by the
    /// liveness layer that owns the flag.
    fn is_member(&self, epoch: u64, peer: &PeerPubkey) -> Result<bool, CommitteeError>;

    /// A wake-up carrying the highest epoch this node can now read:
    /// `epoch(anchor) + MAX_COMMITTEE_LOOKAHEAD_EPOCHS`.
    ///
    /// The VALUE is a hint; the EVENT is the contract. A consumer parked on a
    /// [`CommitteeError::NotReadable`] wakes here and re-asks, instead of
    /// polling on a timer.
    ///
    /// TWO events produce it, because there are two ways an epoch becomes
    /// readable: the ANCHOR moved (the executor's three call sites) and the
    /// GEOMETRY was frozen (the beacon plane's poller, the one and only
    /// publisher of `(activation, interval)`). Before the freeze every epoch is
    /// [`CommitteeError::NotReadable`] with `ready_at: 0` — no height can fix
    /// it — so the freeze is exactly as much of a wake-up as an anchor advance,
    /// and its publisher owes the same [`Committee::anchor_advanced`] call.
    fn subscribe(&self) -> tokio::sync::watch::Receiver<u64>;

    /// Tell the implementation its anchor moved: re-read the anchor, publish
    /// the wake-up, drop what the window no longer admits.
    ///
    /// On the TRAIT and not on the concrete store, because the waker and the
    /// owner are different crates: the store is built in `node/`, while the one
    /// thing that knows the ordering-finalized cursor moved is the executor,
    /// which will hold an `Arc<dyn Committee>`. Its three call sites are the
    /// three that raise the cursor — `executor.rs` init, jump landing and the
    /// finalized derive — and each must call STRICTLY AFTER raising it; calling
    /// before merely loses one wake-up, which the next one makes good.
    ///
    /// A FOURTH caller, for the other event of [`Self::subscribe`]: whoever
    /// freezes the geometry calls it right after publishing the frozen pair. It
    /// takes no height there either — the freeze is what made the anchor's
    /// height mean something, so re-reading the anchor is exactly the right
    /// thing to do.
    ///
    /// Takes NO height: an implementation reads the height back from its own
    /// anchor, so it can never publish a height the anchor does not hold. Any
    /// other caller invoking it is harmless for the same reason — it re-reads
    /// the anchor and finds nothing new.
    fn anchor_advanced(&self);

    /// The hash every read of this implementation is taken at:
    /// `executed_state_hash(anchor)`, or `None` while that height is not
    /// executed.
    ///
    /// Here rather than on a second trait the caller would have to pair up by
    /// hand: [`CommitteeReadsFacade`] has to answer `read_at()` with the hash
    /// the RECORD was read at, and taking it from any other object is the
    /// two-authoritative-cursors defect this module exists to close, restated
    /// one level up.
    fn anchor_hash(&self) -> Option<B256>;
}

/// Where the module's single read anchor comes from.
///
/// Split into its own trait for one reason: the two halves have different
/// owners. The HEIGHT is the executor's ordering-finalized cursor and the HASH
/// is reth's materialized-state probe, and a test needs to move the first
/// without standing up the second.
pub trait Anchor: Send + Sync {
    /// This node's ordering-finalized tip. Monotone.
    fn height(&self) -> u64;

    /// [`crate::executed_state_hash`] at `height`: `Ok(None)` above the
    /// materialized head (the state is not there yet — park), `Err` for a fault
    /// at a materialized height (never a park).
    ///
    /// The `Err` is not one class: a header-index miss is permanent, while a
    /// torn static-file read of the same storage is transient and carries that
    /// verdict in its [`ReadError`]. The store routes on the verdict rather than
    /// on the fact of the `Err`, because a disk hiccup on a cache miss must not
    /// refuse an epoch for good.
    fn executed_hash(&self, height: u64) -> Result<Option<B256>, ReadError>;
}

/// How the frozen geometry reaches the store: the `(activation, interval)`
/// watch the beacon plane publishes the instant its `EpochTransition` freezes
/// one (`crates/node/src/dpos.rs`, `geometry_tx`), `None` until then.
///
/// A watch of a frozen pair rather than a live chain read: the value is frozen
/// ONCE per process by its single source, and re-reading the chain per call
/// would let a mid-flight governance change silently re-slice every epoch
/// boundary this module has already answered on. What the watch adds over a
/// plain value is only the "not frozen yet" state, which the store answers as
/// [`CommitteeError::NotReadable`] without touching the EVM.
pub type GeometryRx = tokio::sync::watch::Receiver<Option<(u64, u64)>>;

/// The frozen `(activation_block, epoch_block_interval)` pair, and the epoch
/// arithmetic over it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    activation: u64,
    interval: NonZeroU64,
}

impl Geometry {
    /// `None` for a zero interval — epoch division is undefined there
    /// ([`ReadError::ZeroEpochInterval`]), and refusing to CONSTRUCT is what
    /// keeps every method below total.
    pub fn new(activation: u64, interval: u64) -> Option<Self> {
        Some(Self {
            activation,
            interval: NonZeroU64::new(interval)?,
        })
    }

    /// The epoch a block height falls in. Pre-activation heights clamp to 0.
    pub fn epoch_of(&self, height: u64) -> u64 {
        // The `None` arm of `epoch_at_block` is the zero-interval one, which
        // `Geometry::new` already refused; the shared formula is called rather
        // than re-derived so a change to the epoch numbering lands in one place.
        epoch_at_block(height, self.activation, self.interval.get()).unwrap_or(0)
    }

    /// First block of `epoch`.
    pub fn start(&self, epoch: u64) -> u64 {
        self.activation
            .saturating_add(epoch.saturating_mul(self.interval.get()))
    }

    /// Last block of `epoch`.
    pub fn last(&self, epoch: u64) -> u64 {
        self.start(epoch.saturating_add(1)).saturating_sub(1)
    }

    /// The lowest anchor height whose state can hold `committee[epoch]`.
    ///
    /// `start(epoch − MAX_COMMITTEE_LOOKAHEAD_EPOCHS)`: the node's
    /// pre-execution stage drains `commitEpochCommittee()` on EVERY block while
    /// `nextEpochToCommit() <= epoch(h) + MAX_COMMITTEE_LOOKAHEAD_EPOCHS`
    /// (`crates/node/src/evm.rs:895-902`, call site `:1231`) and the contract
    /// reverts a target above that horizon, so the first block of epoch `E−2`
    /// is the first state that holds `committee[E]`.
    ///
    /// The first three epochs are NOT `start(0)`, and the reason is that
    /// genesis does not go through that stage at all. The bootstrap issues
    /// exactly ONE `commitEpochCommittee`, for epoch 0
    /// (`devnet/local-dpos-smoke/genesis-bootstrap/src/bootstrap.rs:399-408`),
    /// so epoch 0 is committed AT genesis and epochs 1 and 2 are first
    /// committed by the first EXECUTED block — height 1, not 0
    /// (`.dpos-study/history/E4-PRECONDITIONS.md` §7).
    ///
    /// A LOWER BOUND, not an equality: the drain is skipped while the chain has
    /// no staking address configured, so a chain that schedules DPoS later
    /// commits epochs 1 and 2 later than height 1. The empty-committee arm of
    /// [`Committee::committee`] answers [`CommitteeError::NotReadable`] there
    /// too, so the bound is used to SKIP an EVM call that cannot succeed, never
    /// to conclude that one must.
    pub fn commit_height(&self, epoch: u64) -> u64 {
        match epoch {
            0 => 0,
            e if e <= MAX_COMMITTEE_LOOKAHEAD_EPOCHS => 1,
            e => self.start(e - MAX_COMMITTEE_LOOKAHEAD_EPOCHS),
        }
    }
}

/// The two staticcalls this module makes, and nothing else.
///
/// Not [`fluentbase_staking_reader::StakingStateRead`]: that trait carries the
/// geometry reads and the registry read this module has no business making, and
/// — the blocking reason — it does NOT carry `getDkgQual`, which lives as an
/// inherent method on the concrete reader. A port shaped like the two reads the
/// module actually issues is also what makes "exactly one snapshot call and one
/// qual call per epoch" a countable property in a test.
pub trait EpochReads: Send + Sync {
    /// `getEpochCommitteeWithStakes(epoch)` at `at`.
    fn epoch_committee_snapshot(
        &self,
        epoch: u64,
        at: B256,
    ) -> Result<ValidatorSetSnapshot, ReadError>;

    /// `getDkgQual(epoch)` at `at` — the SAME `at`.
    fn dkg_qual(&self, epoch: u64, at: B256) -> Result<bool, ReadError>;
}

impl<P, E> EpochReads for fluentbase_staking_reader::RethStakingStateReader<P, E>
where
    P: reth_storage_api::StateProviderFactory
        + reth_storage_api::HeaderProvider<Header = reth_primitives_traits::HeaderTy<E::Primitives>>
        + Send
        + Sync,
    E: reth_evm::ConfigureEvm + Send + Sync,
{
    fn epoch_committee_snapshot(
        &self,
        epoch: u64,
        at: B256,
    ) -> Result<ValidatorSetSnapshot, ReadError> {
        fluentbase_staking_reader::StakingStateRead::epoch_committee_snapshot(self, epoch, at)
    }

    fn dkg_qual(&self, epoch: u64, at: B256) -> Result<bool, ReadError> {
        fluentbase_staking_reader::RethStakingStateReader::dkg_qual(self, epoch, at)
    }
}
