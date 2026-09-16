//! The committee of an epoch as a single frozen value, read at a single anchor.
//!
//! [`CommitteeRecord`] is the whole answer — members, weights, the `dkgQual` bit
//! and the projections derived from the members. It is write-once: the first
//! successful read of an epoch is the answer for as long as that epoch is
//! readable at all, and a second read that disagrees is a contract fork,
//! reported and refused rather than merged.
//!
//! Every read is taken at `executed_state_hash(anchor)`, where `anchor` is this
//! node's own ordering-finalized tip ([`Anchor`]). The height moves, and moving
//! it is safe because a record's value does not depend on which in-window anchor
//! read it: an epoch is readable only inside `[epoch(anchor) -
//! SCHEME_RETENTION_EPOCHS, epoch(anchor) + MAX_COMMITTEE_LOOKAHEAD_EPOCHS]`.
//! Below that window the contract's weight ring has wrapped and the weights are
//! gone; above it the epoch is not committed yet. Retention is that same floor,
//! so an epoch the module still answers keeps the value it was first read with.
//!
//! [`CommitteeError`] separates "not yet" from "not here" from "the read
//! failed", and only the last carries a retry decision. The epoch's
//! [`BlsScheme`] lives in the same map slot as the record, built by the one
//! [`EpochVerifier`] the store was constructed with; [`Committee::upgrade_scheme`]
//! can only raise an entry's strength, never create one for an epoch whose
//! committee this node has not read.
//!
//! `tombstoned` and `Member::activation_epoch` are deliberately not part of the
//! record. The contract does not freeze the tombstone flag and it is read live,
//! so a mid-epoch equivocation verdict reaches the committee it names; no
//! consumer in the core reads `activation_epoch`.

use alloy_primitives::{Address, B256};
use fluentbase_bls::{scheme::EpochCommittee, BlsPubkey, PeerPubkey, Scheme as BlsScheme};
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
/// `Option<Vec>`: every epoch this module reads still has its weights in the
/// contract's ring (`16 - 2 > 8` — the window floor is above the oldest epoch
/// whose frame could have been reused). Shrinking the ring, widening the commit
/// horizon or lengthening retention past this point fails the build here rather
/// than producing a run-time `None` the leader elector would turn into a uniform
/// lottery.
pub const WINDOW_FITS_THE_WEIGHT_RING: () = assert!(
    WEIGHT_RING_EPOCHS - MAX_COMMITTEE_LOOKAHEAD_EPOCHS > SCHEME_RETENTION_EPOCHS as u64,
    "the committee read window reaches epochs whose frozen weights the contract's ring has \
     already overwritten — CommitteeRecord::weights can no longer be non-optional"
);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub address: Address,
    pub peer: PeerPubkey,
    pub bls: BlsPubkey,
}

/// One epoch's committee, frozen.
///
/// `members` is in contract order, verbatim — the reader never sorts, and that
/// order is the consensus index space the slasher resolves an accused position
/// in. `participants` and `bls` are derived once here rather than rebuilt per
/// call; `bls` carries a commonware `BiMap`, whose order is not contract order.
#[derive(Clone, Debug)]
pub struct CommitteeRecord {
    pub epoch: u64,
    pub members: Vec<Member>,
    /// Frozen leader weights, one per member, in the same order as `members`.
    /// Non-optional by [`WINDOW_FITS_THE_WEIGHT_RING`].
    pub weights: Vec<u128>,
    /// `getDkgQual(epoch)`: whether this epoch's committee differs from the
    /// previous one, so its DKG re-mints the beacon key. A second staticcall at
    /// the snapshot's own hash — the snapshot read does not carry the bit.
    pub changed: bool,
    /// `(block_number, block_hash)` of the anchor this record was read at.
    /// Diagnostic only: two nodes hold the same record at different anchors, so
    /// [`Self::same_value`] ignores it.
    pub snapshot: (u64, B256),
    pub participants: commonware_utils::ordered::Set<PeerPubkey>,
    pub bls: EpochCommittee,
}

impl CommitteeRecord {
    /// The record projected into the [`ValidatorSetSnapshot`] shape that the
    /// surfaces outside this module consume.
    ///
    /// A projection, not a second authority: every leg comes from this record,
    /// so two nodes holding the same record project the same snapshot. The two
    /// legs a record cannot carry are filled with the value that means "not
    /// stated here" — `tombstoned: false` and `activation_epoch: 0` — and no
    /// consumer of this projection reads either. `weights` is always `Some`, so
    /// an elector built from this projection cannot fall back to a uniform
    /// lottery.
    pub fn snapshot_view(&self) -> ValidatorSetSnapshot {
        use fluentbase_staking_reader::reader::{ConsensusKeys, ValidatorWithKeys};
        ValidatorSetSnapshot {
            block_hash: self.snapshot.1,
            block_number: self.snapshot.0,
            epoch: self.epoch,
            validators: self
                .members
                .iter()
                .map(|m| ValidatorWithKeys {
                    address: m.address,
                    keys: ConsensusKeys {
                        peer_pubkey: m.peer.clone(),
                        bls_pubkey: m.bls,
                        activation_epoch: 0,
                    },
                    tombstoned: false,
                })
                .collect(),
            weights: Some(self.weights.clone()),
        }
    }

    /// Whether two records describe the same committee.
    ///
    /// Both the write-once predicate and the cross-node equality predicate: it
    /// compares everything the contract froze and nothing about where it was
    /// read, so it skips [`Self::snapshot`]. `participants` and `bls` are
    /// functions of `members`.
    pub fn same_value(&self, other: &Self) -> bool {
        self.epoch == other.epoch
            && self.members == other.members
            && self.weights == other.weights
            && self.changed == other.changed
    }
}

/// Every way a committee read can fail to produce a record.
///
/// The three variants split by what the caller must do, not by where the failure
/// happened.
#[derive(Debug, thiserror::Error)]
pub enum CommitteeError {
    /// The epoch is inside the window but this node cannot see it yet: the anchor
    /// is below the height that first commits it, the anchor height is not
    /// executed yet, or the contract answered an empty committee.
    ///
    /// Never a fault; the caller retries on the next tip. `ready_at` is the
    /// lowest anchor height at which a retry can do something this one did not —
    /// the commit height when the epoch is not committed in any state this anchor
    /// covers, the current anchor height when only its state is missing, and
    /// `anchor + 1` for an empty committee. It is a lower bound and a diagnostic
    /// hint, never a timer; the wake-up to park on is [`Committee::subscribe`].
    #[error("committee[{epoch}] is not readable at this anchor yet (ready at height {ready_at})")]
    NotReadable { epoch: u64, ready_at: u64 },

    /// The epoch is outside `[lo, hi]`. Refused without an EVM call — a predicate
    /// on the request, lifted into the type so no caller can forget it (a p2p
    /// frame naming an arbitrary epoch, a jump target far ahead).
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
    /// [`Self::NotReadable`] is transient by construction: the anchor only moves
    /// up. [`Self::OutOfWindow`] is directional, which is why the variant carries
    /// `hi`: an epoch above the window is one the anchor has not reached yet and
    /// becomes readable on its own, while one below it has left the contract's
    /// weight ring for good, where retrying can only spin. A failed read defers
    /// to [`ReadError::is_transient`].
    pub fn is_transient(&self) -> bool {
        match self {
            Self::NotReadable { .. } => true,
            Self::OutOfWindow { epoch, hi, .. } => epoch > hi,
            Self::Read(e) => e.is_transient(),
        }
    }

    /// Whether the contract answered something no committed epoch can answer —
    /// [`fluentbase_staking_reader::ReadClass::Impossible`], and only through
    /// [`Self::Read`]: only a read that came back can carry a statement the chain
    /// made about itself, while the other two variants are this node's own view.
    pub fn is_contract_impossible(&self) -> bool {
        match self {
            Self::NotReadable { .. } | Self::OutOfWindow { .. } => false,
            Self::Read(e) => e.is_contract_impossible(),
        }
    }
}

/// The committee of an epoch, frozen — the whole read surface of this module.
///
/// Synchronous, because the reads underneath are blocking reth state reads and
/// pretending otherwise would only move the blocking somewhere less visible.
pub trait Committee: Send + Sync {
    /// `committee[epoch]`, read once and then held for as long as the epoch is
    /// inside the read window — which is as long as this trait answers for it.
    fn committee(&self, epoch: u64) -> Result<Arc<CommitteeRecord>, CommitteeError>;

    /// `dkgQual[epoch]` — whether this epoch re-mints the beacon key. Same
    /// record, same read; never a second staticcall.
    fn changed(&self, epoch: u64) -> Result<bool, CommitteeError>;

    /// Whether `peer` sits in `committee[epoch]`.
    ///
    /// Does not consider `tombstoned`: a tombstoned validator keeps its seat in
    /// the frozen committee it was elected to — the certificate bitmap and the
    /// slasher's positional index both depend on that — and what it loses is
    /// enforced by the liveness layer that owns the flag.
    fn is_member(&self, epoch: u64, peer: &PeerPubkey) -> Result<bool, CommitteeError>;

    /// The epoch's certificate scheme, from the same map slot as its record.
    ///
    /// Reading the committee is what produces it, so this asks for the record
    /// first: a hit is a map lookup, a miss inside the window costs the record's
    /// two staticcalls, and an epoch outside the window or not committed yet
    /// answers `None` without touching the EVM. `None` also covers "the verifier
    /// could not build one yet" (see [`EpochVerifier`]); the caller's contract is
    /// the same either way — defer and ask again.
    fn scheme(&self, epoch: u64) -> Option<std::sync::Arc<BlsScheme>>;

    /// Replace the epoch's scheme with a stronger one — the signer half an engine
    /// spawn needs, and the only write to a scheme slot that is not
    /// [`EpochVerifier`].
    ///
    /// Monotone in verification strength: it refuses a scheme for another
    /// committee, a verifier replacing a signer, and an oracle-less scheme
    /// replacing a beacon-active one (which would return the epoch to vote-only
    /// certificate admission). It returns whether the scheme is now installed, so
    /// a caller that must not spawn an engine over a refusal can see it.
    fn upgrade_scheme(&self, epoch: u64, scheme: BlsScheme) -> bool;

    /// Epochs holding a verify-only scheme — the repair sweep's candidate list.
    ///
    /// This map is the authority on what is registered: it is bounded by the read
    /// window and covers every path that made a record. Signer entries are
    /// excluded — a signer's engine resolves its own epoch key through the
    /// promote gates, which have a value-gate this sweep does not.
    fn verifier_epochs(&self) -> Vec<u64>;

    /// The scheme of the highest epoch this node holds one for. Its
    /// `participants()` are the peers to target for a finalization re-fetch on
    /// catch-up — they are connected and hold the durable finalizations.
    fn latest_scheme(&self) -> Option<std::sync::Arc<BlsScheme>>;

    /// A wake-up carrying the highest epoch this node can now read:
    /// `epoch(anchor) + MAX_COMMITTEE_LOOKAHEAD_EPOCHS`.
    ///
    /// The value is a hint; the event is the contract. A consumer parked on a
    /// [`CommitteeError::NotReadable`] wakes here and re-asks instead of polling
    /// on a timer, and it fires on every anchor advance, not only on the ones
    /// that raise the value: an epoch can be unreadable because its commit height
    /// is above the anchor (the value moves when that changes) or because the
    /// anchor's own height is not executed yet (it does not).
    ///
    /// Two events produce it, because there are two ways an epoch becomes
    /// readable: the anchor moved, or the geometry was frozen. Before the freeze
    /// every epoch is [`CommitteeError::NotReadable`] with `ready_at: 0` — no
    /// height can fix it — so the freeze owes the same
    /// [`Committee::anchor_advanced`] call as an anchor advance.
    fn subscribe(&self) -> tokio::sync::watch::Receiver<u64>;

    /// Tell the implementation its anchor moved: re-read the anchor, publish the
    /// wake-up, drop what the window no longer admits.
    ///
    /// On the trait rather than the concrete store because the one that knows the
    /// ordering-finalized cursor moved is the executor, which holds an
    /// `Arc<dyn Committee>`, while the store is built by the node. Call it
    /// strictly after raising the cursor; calling before only loses one wake-up,
    /// which the next one makes good.
    ///
    /// Takes no height: an implementation reads the height back from its own
    /// anchor, so it can never publish a height the anchor does not hold, and any
    /// other caller re-reads the anchor and finds nothing new.
    fn anchor_advanced(&self);

    /// The one epoch geometry of this process, or `None` while the single
    /// in-process source has not frozen `(activation, interval)` yet.
    ///
    /// The read window, `commit_height` and the record retention are all derived
    /// from this pair, so a consumer that needs `epoch_of(height)` must reach it
    /// here rather than keep a second copy.
    fn geometry(&self) -> Option<Geometry>;

    /// The epoch a block height falls in, over [`Self::geometry`]. `None`
    /// before the freeze — no height means anything yet, and answering `0`
    /// there would let a frontier answer bind itself to epoch 0.
    fn epoch_of(&self, height: u64) -> Option<u64> {
        Some(self.geometry()?.epoch_of(height))
    }

    /// The hash every read of this implementation is taken at:
    /// `executed_state_hash(anchor)`, or `None` while that height is not
    /// executed.
    ///
    /// [`CommitteeReadsFacade`] must answer `read_at()` with the hash the record
    /// was read at, so it asks here instead of pairing itself with a cursor the
    /// caller hands in.
    fn anchor_hash(&self) -> Option<B256>;
}

/// Where the module's single read anchor comes from.
///
/// Split into its own trait for one reason: the two halves have different
/// owners. The height is the executor's ordering-finalized cursor and the hash
/// is reth's materialized-state probe, and a test needs to move the first
/// without standing up the second.
pub trait Anchor: Send + Sync {
    /// This node's ordering-finalized tip. Monotone.
    fn height(&self) -> u64;

    /// [`crate::executed_state_hash`] at `height`: `Ok(None)` above the
    /// materialized head (the state is not there yet — park), `Err` for a fault
    /// at a materialized height (never a park).
    ///
    /// The `Err` is not one class: a header-index miss is permanent, while a torn
    /// static-file read of the same storage is transient and carries that verdict
    /// in its [`ReadError`]. The store routes on the verdict rather than on the
    /// fact of the `Err`, because a disk hiccup on a cache miss must not refuse
    /// an epoch for good.
    fn executed_hash(&self, height: u64) -> Result<Option<B256>, ReadError>;
}

/// How the frozen geometry reaches the store: the `(activation, interval)` watch
/// the beacon plane publishes once its `EpochTransition` freezes one, `None`
/// until then.
///
/// A watch of a frozen pair rather than a live chain read: the value is frozen
/// once per process by its single source, and re-reading the chain per call would
/// let a mid-flight governance change re-slice every epoch boundary this module
/// has already answered on. The watch adds the "not frozen yet" state, which the
/// store answers as [`CommitteeError::NotReadable`] without touching the EVM.
pub type GeometryRx = tokio::sync::watch::Receiver<Option<(u64, u64)>>;

/// The one producer of an epoch's verify-only [`BlsScheme`]: the record the
/// module just installed, plus the chain namespace and the epoch's beacon oracle
/// ([`crate::beacon::Beacon::oracle_for`]).
///
/// A closure rather than a method on the store because the beacon is built after
/// the store on both node classes (the beacon consumes
/// [`CommitteeReadsFacade`], which is a view on the store), so the store cannot
/// hold an `Arc<dyn Beacon>` at construction. It answers `None` for exactly that
/// window, and the store retries rather than caching the absence, so a record
/// installed before the beacon existed is not pinned to a vote-only verifier for
/// the life of the process. `None` is otherwise unreachable: the record's BLS
/// projection was already built and validated by the store.
pub type EpochVerifier = Arc<dyn Fn(&CommitteeRecord) -> Option<BlsScheme> + Send + Sync>;

/// The beacon handle an [`EpochVerifier`] needs, filled once the beacon exists.
///
/// Weak, and that is a correctness property rather than hygiene: the beacon holds
/// this module through [`CommitteeReadsFacade`] and this slot would hold the
/// beacon, so a strong handle closes a reference cycle that keeps the store, the
/// beacon and every journal sender it owns alive past the shutdown that drops
/// them. A failed upgrade answers `None`, which is right once the beacon is gone.
pub type BeaconSlot = Arc<std::sync::OnceLock<std::sync::Weak<dyn crate::beacon::Beacon>>>;

/// Fill the [`BeaconSlot`], once, and report loudly if it is ever filled twice.
///
/// The second `set` is not a lost write: the slot still holds the first beacon's
/// handle, which after an in-process rebuild is a dead `Weak`. Every later epoch
/// would then get no scheme at all rather than a vote-only one, and the marshal
/// would quietly stop verifying certificates of every new epoch.
///
/// Unreachable while the beacon is built once per process; kept as a sentry for
/// an in-process rebuild, where this failure would be silent.
pub fn fill_beacon_slot(slot: &BeaconSlot, beacon: &Arc<dyn crate::beacon::Beacon>) {
    if slot.set(Arc::downgrade(beacon)).is_err() {
        tracing::error!(
            "beacon rebuilt in-process; committee module keeps the FIRST beacon — every later \
             epoch would get no scheme"
        );
    }
}

pub fn epoch_verifier(chain_id: u64, beacon: BeaconSlot) -> EpochVerifier {
    let namespace = fluentbase_bls::fluent_namespace(chain_id);
    Arc::new(move |record: &CommitteeRecord| {
        // `None` while the beacon is younger than this store — the store asks
        // again rather than remembering the absence — and `None` again once it is
        // gone, at shutdown.
        let beacon = beacon.get()?.upgrade()?;
        Some(fluentbase_bls::scheme::build_verifier(
            &namespace,
            record.bls.bimap.clone(),
            record.epoch,
            beacon.oracle_for(record.epoch),
        ))
    })
}

/// The frozen `(activation_block, epoch_block_interval)` pair, and the epoch
/// arithmetic over it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    activation: u64,
    interval: NonZeroU64,
}

impl Geometry {
    /// `None` for a zero interval — epoch division is undefined there
    /// ([`ReadError::ZeroEpochInterval`]), and refusing to construct is what
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

    /// The lowest anchor height whose state can hold `committee[epoch]`:
    /// `start(epoch - MAX_COMMITTEE_LOOKAHEAD_EPOCHS)`, because the pre-execution
    /// stage drains `commitEpochCommittee()` on every block while
    /// `nextEpochToCommit() <= epoch(h) + MAX_COMMITTEE_LOOKAHEAD_EPOCHS` and the
    /// contract reverts a target above that horizon.
    ///
    /// Epochs 0, 1 and 2 are the exception: genesis does not go through that
    /// stage and commits epoch 0 only, so epochs 1 and 2 are first committed by
    /// the first executed block — height 1, not 0.
    ///
    /// A lower bound, not an equality: the drain is skipped while the chain has no
    /// staking address, so a chain that schedules DPoS later commits epochs 1 and
    /// 2 later than height 1. The empty-committee arm of [`Committee::committee`]
    /// answers [`CommitteeError::NotReadable`] there too, so the bound skips an
    /// EVM call that cannot succeed; it never concludes that one must.
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
/// the blocking reason is that it does not carry `getDkgQual`, which lives as an
/// inherent method on the concrete reader. A port shaped like the two reads the
/// module actually issues also makes "exactly one snapshot call and one qual call
/// per epoch" a countable property in a test.
pub trait EpochReads: Send + Sync {
    /// `getEpochCommitteeWithStakes(epoch)` at `at`.
    fn epoch_committee_snapshot(
        &self,
        epoch: u64,
        at: B256,
    ) -> Result<ValidatorSetSnapshot, ReadError>;

    /// `getDkgQual(epoch)` at `at` — the same `at` as the snapshot.
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

/// Test doubles shared by the consumers of this module.
#[cfg(test)]
pub(crate) mod testing {
    use super::{BlsScheme, Committee, CommitteeError, CommitteeRecord, Geometry, PeerPubkey};
    use alloy_primitives::B256;
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };

    /// A [`Committee`] that is only the scheme half of the map — a closure
    /// answering "what is `committee[E]`'s scheme", memoised the way
    /// [`super::CommitteeStore`] memoises a record.
    ///
    /// The memoisation is the point: a consumer sees one production-shaped fact —
    /// an epoch is read at most once and its answer never changes afterwards — so
    /// a test counting reads counts what the real store would make it count.
    /// `None` is not memoised, for the same reason the store does not cache a
    /// failure: it means "not readable at this anchor yet", and the next call must
    /// be free to succeed. `committee()` answers [`CommitteeError::NotReadable`]
    /// by default; [`Self::with_geometry`] and [`Self::with_window`] give it the
    /// record half as well, for consumers that distinguish "cannot read
    /// `committee[E]`" from "the record is here but no scheme could be built from
    /// it".
    pub(crate) struct SchemeCommittee {
        answer: Box<dyn Fn(u64) -> Option<BlsScheme> + Send + Sync>,
        records: Box<dyn Fn(u64) -> Option<CommitteeRecord> + Send + Sync>,
        entries: Mutex<BTreeMap<u64, Arc<BlsScheme>>>,
        /// The frozen geometry this double answers [`Committee::geometry`] with.
        /// `None` by default, the unfrozen state where every `epoch_of` is `None`.
        geometry: Option<Geometry>,
        /// When `Some((lo, hi))`, an epoch outside it is refused with
        /// [`CommitteeError::OutOfWindow`] before the record closure is asked.
        /// `None` means no window.
        window: Option<(u64, u64)>,
    }

    impl SchemeCommittee {
        pub(crate) fn new(
            answer: impl Fn(u64) -> Option<BlsScheme> + Send + Sync + 'static,
        ) -> Arc<Self> {
            Self::with_geometry(answer, |_| None, None)
        }

        /// The same double with a read window, so a consumer that must react to
        /// `OutOfWindow` differently from `NotReadable` can be shown both.
        pub(crate) fn with_window(
            answer: impl Fn(u64) -> Option<BlsScheme> + Send + Sync + 'static,
            records: impl Fn(u64) -> Option<CommitteeRecord> + Send + Sync + 'static,
            geometry: Option<Geometry>,
            window: (u64, u64),
        ) -> Arc<Self> {
            let mut this = Self::build(answer, records, geometry);
            Arc::get_mut(&mut this).expect("sole owner").window = Some(window);
            this
        }

        /// The same double with the geometry half, for the consumers that bind a
        /// height to an epoch through the module ([`Committee::epoch_of`]) rather
        /// than through a second copy of `(activation, interval)`.
        pub(crate) fn with_geometry(
            answer: impl Fn(u64) -> Option<BlsScheme> + Send + Sync + 'static,
            records: impl Fn(u64) -> Option<CommitteeRecord> + Send + Sync + 'static,
            geometry: Option<Geometry>,
        ) -> Arc<Self> {
            Self::build(answer, records, geometry)
        }

        fn build(
            answer: impl Fn(u64) -> Option<BlsScheme> + Send + Sync + 'static,
            records: impl Fn(u64) -> Option<CommitteeRecord> + Send + Sync + 'static,
            geometry: Option<Geometry>,
        ) -> Arc<Self> {
            Arc::new(Self {
                answer: Box::new(answer),
                records: Box::new(records),
                entries: Mutex::new(BTreeMap::new()),
                geometry,
                window: None,
            })
        }
    }

    impl Committee for SchemeCommittee {
        fn committee(&self, epoch: u64) -> Result<Arc<CommitteeRecord>, CommitteeError> {
            if let Some((lo, hi)) = self.window {
                if epoch < lo || epoch > hi {
                    return Err(CommitteeError::OutOfWindow { epoch, lo, hi });
                }
            }
            (self.records)(epoch)
                .map(Arc::new)
                .ok_or(CommitteeError::NotReadable { epoch, ready_at: 0 })
        }

        fn changed(&self, epoch: u64) -> Result<bool, CommitteeError> {
            Err(CommitteeError::NotReadable { epoch, ready_at: 0 })
        }

        fn is_member(&self, epoch: u64, _peer: &PeerPubkey) -> Result<bool, CommitteeError> {
            Err(CommitteeError::NotReadable { epoch, ready_at: 0 })
        }

        fn scheme(&self, epoch: u64) -> Option<Arc<BlsScheme>> {
            if let Some(scheme) = self.entries.lock().unwrap().get(&epoch) {
                return Some(scheme.clone());
            }
            let built = Arc::new((self.answer)(epoch)?);
            Some(
                self.entries
                    .lock()
                    .unwrap()
                    .entry(epoch)
                    .or_insert(built)
                    .clone(),
            )
        }

        fn upgrade_scheme(&self, epoch: u64, scheme: BlsScheme) -> bool {
            self.entries.lock().unwrap().insert(epoch, Arc::new(scheme));
            true
        }

        fn verifier_epochs(&self) -> Vec<u64> {
            use commonware_cryptography::certificate::Scheme as _;
            self.entries
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, s)| s.me().is_none())
                .map(|(epoch, _)| *epoch)
                .collect()
        }

        fn latest_scheme(&self) -> Option<Arc<BlsScheme>> {
            self.entries.lock().unwrap().values().next_back().cloned()
        }

        fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
            tokio::sync::watch::Sender::new(0).subscribe()
        }

        fn geometry(&self) -> Option<Geometry> {
            self.geometry
        }

        fn anchor_advanced(&self) {}

        fn anchor_hash(&self) -> Option<B256> {
            None
        }
    }
}
