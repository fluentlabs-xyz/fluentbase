//! [`CommitteeStore`] — the production [`Committee`]: one map, one anchor, one
//! producer.

use super::{
    Anchor, Committee, CommitteeError, CommitteeRecord, EpochReads, Geometry, GeometryRx, Member,
};
use alloy_primitives::B256;
use commonware_utils::TryFromIterator as _;
use fluentbase_bls::PeerPubkey;
use fluentbase_staking_reader::{reader::ValidatorSetSnapshot, ReadError};
use fluentbase_types::staking_protocol::MAX_COMMITTEE_LOOKAHEAD_EPOCHS;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
use tracing::error;

use crate::SCHEME_RETENTION_EPOCHS;

/// A committee read that will never succeed, by cause (`reason` label). One
/// counter rather than one per cause, because the interesting alert is "this
/// node refused an epoch permanently" and the cause is what the operator reads
/// next; the `error!` beside it carries the detail.
pub(crate) const READ_PERMANENT: &str = "dpos_committee_read_permanent_total";
/// An epoch refused by the window predicate, by `side` (`above`/`below`). The
/// two sides mean opposite things — see [`CommitteeError::is_transient`] — so
/// one counter without the label would be unreadable.
pub(crate) const OUT_OF_WINDOW: &str = "dpos_committee_out_of_window_total";
/// An epoch inside the window this node cannot see yet. Expected while a node
/// catches up; a rate that does not fall is the empty-committee case §5.4 names
/// (the chain committed nothing, i.e. it halted).
pub(crate) const NOT_READABLE: &str = "dpos_committee_not_readable_total";

/// `weights: None` — the contract's ring wrapped past an in-window epoch.
pub(crate) const REASON_WEIGHTS_NONE: &str = "weights_none";
/// A weight vector whose length is not the member count.
pub(crate) const REASON_WEIGHTS_LEN: &str = "weights_len";
/// Two members sharing a peer or BLS key.
pub(crate) const REASON_NON_UNIQUE_KEYS: &str = "non_unique_keys";
/// One epoch read twice with two different values.
pub(crate) const REASON_FORK: &str = "fork";
/// The staticcall itself failed permanently (a revert, a decode).
pub(crate) const REASON_READ_ERROR: &str = "read_error";
/// The anchor's own state probe faulted at a materialized height.
pub(crate) const REASON_ANCHOR_FAULT: &str = "anchor_fault";

/// One epoch's slot in the store's single map.
///
/// A struct with one field rather than a bare `Arc<CommitteeRecord>` on
/// purpose: the record and the epoch's `BlsScheme` are one lifetime with one
/// retention, and the next step makes `EpochSchemeProvider` a VIEW on this map
/// by adding a `scheme` field here. Adding a field is then a local change; a
/// second map beside this one would be the two-authorities defect again.
struct EpochEntry {
    record: Arc<CommitteeRecord>,
}

/// Everything the store mutates, behind one lock.
///
/// The lock is NEVER held across a staking read: an `eth_call` into reth is a
/// blocking state read, and holding the map across it would serialize every
/// consumer behind the slowest reader.
#[derive(Default)]
struct State {
    records: BTreeMap<u64, EpochEntry>,
    /// Epochs whose PERMANENT failure has already been logged.
    ///
    /// A permanent failure is re-derived on every call (nothing is cached for
    /// it, by design — see `weights: None` below), so without this the first
    /// contract fork would produce one `error!` per consumer per tick. It is
    /// pruned by the same window floor as `records` ([`CommitteeStore::prune`]),
    /// which is what makes "once per epoch" true: an epoch is only ever
    /// forgotten together with the window that could still ask about it, and an
    /// epoch below the floor is refused by the window before any read is
    /// attempted. Bounded by the window's width for the same reason.
    reported: BTreeSet<u64>,
}

/// The committee module's production store.
///
/// Generic over the reads rather than over a concrete reader so the tests can
/// count staticcalls and branch an answer by hash — the two properties the
/// module is actually asserting are properties of the CALLER, not of the
/// contract.
pub struct CommitteeStore<R> {
    reads: R,
    anchor: Arc<dyn Anchor>,
    geometry: GeometryRx,
    state: Mutex<State>,
    /// Highest readable epoch, published as a wake-up. See
    /// [`CommitteeStore::anchor_advanced`].
    readable: tokio::sync::watch::Sender<u64>,
}

impl<R: EpochReads> CommitteeStore<R> {
    /// `geometry` carries the FROZEN `(activation, interval)` pair once its
    /// single in-process source has frozen it — in production the beacon
    /// plane's `EpochTransition::frozen_geometry()`, republished on a watch.
    ///
    /// LAZY rather than by value, and that is a wiring fact rather than a
    /// preference: the store is built where the anchor is (before the plane's
    /// poller has seen a finalized block), while the geometry is frozen by the
    /// first readable, DPoS-scheduled block that poller reaches. Taking it by
    /// value would have forced the store to be built later than the cursor it
    /// anchors on — i.e. a second cursor — or the node to block its startup on
    /// a chain read. Until the first `Some`, every read answers
    /// [`CommitteeError::NotReadable`] with `ready_at: 0` and touches nothing.
    pub fn new(reads: R, anchor: Arc<dyn Anchor>, geometry: GeometryRx) -> Self {
        let seed = match Self::geometry_of(&geometry) {
            Some(g) => g.epoch_of(anchor.height()) + MAX_COMMITTEE_LOOKAHEAD_EPOCHS,
            None => 0,
        };
        Self {
            reads,
            anchor,
            geometry,
            state: Mutex::new(State::default()),
            readable: tokio::sync::watch::Sender::new(seed),
        }
    }

    fn geometry_of(rx: &GeometryRx) -> Option<Geometry> {
        let (activation, interval) = (*rx.borrow())?;
        Geometry::new(activation, interval)
    }

    /// The frozen geometry, or `None` while the plane has not frozen one yet.
    fn geometry(&self) -> Option<Geometry> {
        Self::geometry_of(&self.geometry)
    }

    /// `epoch(anchor) + MAX_COMMITTEE_LOOKAHEAD_EPOCHS` — the top of the window.
    fn highest_readable(&self, geometry: &Geometry) -> u64 {
        geometry.epoch_of(self.anchor.height()) + MAX_COMMITTEE_LOOKAHEAD_EPOCHS
    }

    /// Drop what the window no longer admits. Called with the lock held.
    ///
    /// Retention is the window FLOOR, not a count. The map is therefore bounded
    /// by the WIDTH of the window — `SCHEME_RETENTION_EPOCHS +
    /// MAX_COMMITTEE_LOOKAHEAD_EPOCHS + 1` entries — and, more importantly,
    /// every epoch the module can still answer keeps its first value. A count
    /// of `SCHEME_RETENTION_EPOCHS` would have evicted the bottom of its own
    /// window, and the next, DIFFERENT answer for such an epoch would have
    /// found a vacant slot and been installed silently: the contract fork
    /// write-once exists to catch, missing exactly where §5.4 puts the
    /// slasher's oldest evidence.
    fn prune(state: &mut State, lo: u64) {
        state.records.retain(|epoch, _| *epoch >= lo);
        state.reported.retain(|epoch| *epoch >= lo);
    }

    /// `[lo, hi]`, inclusive both ends.
    fn window(geometry: &Geometry, anchor_height: u64) -> (u64, u64) {
        let anchor_epoch = geometry.epoch_of(anchor_height);
        (
            anchor_epoch.saturating_sub(SCHEME_RETENTION_EPOCHS as u64),
            anchor_epoch.saturating_add(MAX_COMMITTEE_LOOKAHEAD_EPOCHS),
        )
    }

    /// A permanent read failure: counted every time, logged ONCE for as long as
    /// the epoch stays inside the window, then returned. (Leaving the window is
    /// the only way an epoch is forgotten, and an epoch below the floor is
    /// refused before any read — so "once" is once.) A transient failure is
    /// counted nowhere and returned silently: it is the normal shape of a node
    /// that is still catching up.
    fn failed(&self, epoch: u64, error: ReadError, reason: &'static str) -> CommitteeError {
        if !error.is_transient() {
            metrics::counter!(READ_PERMANENT, "reason" => reason).increment(1);
            let first = self.state.lock().unwrap().reported.insert(epoch);
            if first {
                error!(
                    epoch,
                    %error,
                    "committee read failed PERMANENTLY inside the read window — the contract \
                     answered something no committed epoch can answer; this epoch will not be \
                     registered and no retry can fix it"
                );
            }
        }
        CommitteeError::Read(error)
    }

    /// Build the record from the two answers, or classify why it cannot be one.
    fn build(
        &self,
        epoch: u64,
        snap: ValidatorSetSnapshot,
        changed: bool,
    ) -> Result<CommitteeRecord, CommitteeError> {
        // The BLS projection is the one derivation that can fail: the reader
        // rejects duplicate PEER keys (`check_committee_ordering`) but nothing
        // on chain enforces cross-validator uniqueness of the BLS half, so the
        // `BiMap` can still refuse. It belongs to the §5.4 "the contract
        // answered something impossible" class, which is permanent.
        //
        // It is taken through the PRODUCTION constructor and before the
        // snapshot is taken apart, so this module cannot grow a second
        // derivation of a committee from a snapshot.
        let bls = crate::scheme::epoch_committee_from_snapshot(&snap).map_err(|e| {
            self.failed(
                epoch,
                ReadError::AbiDecode(format!(
                    "epoch {epoch} committee has non-unique consensus keys: {e}"
                )),
                REASON_NON_UNIQUE_KEYS,
            )
        })?;

        // `weights: None` means the contract's ring has wrapped past this
        // epoch. Inside the window that is impossible — see
        // `WINDOW_FITS_THE_WEIGHT_RING` — so it is a contract fork, not a
        // missing optional. Absent is NOT a vector of zeros: the leader elector
        // reads these, and a uniform lottery nobody asked for is a per-node
        // leader split rather than a visible failure.
        let Some(weights) = snap.weights else {
            return Err(self.failed(
                epoch,
                ReadError::AbiDecode(format!(
                    "epoch {epoch} has no frozen weights inside the read window (the contract's \
                     {}-epoch ring cannot have wrapped past an epoch this module reads)",
                    fluentbase_types::staking_protocol::WEIGHT_RING_EPOCHS
                )),
                REASON_WEIGHTS_NONE,
            ));
        };

        let members: Vec<Member> = snap
            .validators
            .iter()
            .map(|v| Member {
                address: v.address,
                peer: v.keys.peer_pubkey.clone(),
                bls: v.keys.bls_pubkey,
            })
            .collect();

        // One weight per member, same order — `CommitteeRecord::weights` says
        // so and the leader elector indexes them positionally. The production
        // reader forces the length, but the module is generic over
        // `EpochReads` precisely so another implementation can be substituted,
        // and an unwritten precondition of a port is not a guarantee.
        if weights.len() != members.len() {
            return Err(self.failed(
                epoch,
                ReadError::AbiDecode(format!(
                    "epoch {epoch} answered {} frozen weights for {} members",
                    weights.len(),
                    members.len()
                )),
                REASON_WEIGHTS_LEN,
            ));
        }

        // ONE duplicate policy for one impossible input: `from_pairs` above
        // already refused a repeated key, so the participant projection refuses
        // it too rather than silently de-duplicating and handing the peer-set a
        // roster shorter than the committee.
        let participants =
            commonware_utils::ordered::Set::try_from_iter(members.iter().map(|m| m.peer.clone()))
                .map_err(|e| {
                self.failed(
                    epoch,
                    ReadError::AbiDecode(format!(
                        "epoch {epoch} committee has a repeated peer key: {e}"
                    )),
                    REASON_NON_UNIQUE_KEYS,
                )
            })?;

        Ok(CommitteeRecord {
            epoch,
            members,
            weights,
            changed,
            snapshot: (snap.block_number, snap.block_hash),
            participants,
            bls,
        })
    }

    /// Install the record, or reconcile with one that got there first.
    ///
    /// The occupied arm is NOT dead code and NOT a `debug_assert`: two
    /// consumers asking for the same uncached epoch both miss the map, both
    /// issue their own pair of staticcalls (the lock is not held across them —
    /// see [`State`]) and both arrive here. Agreeing is the normal case.
    /// DISAGREEING means one anchor's state and another's answered two
    /// different committees for one epoch, which no honest chain can do; the
    /// FIRST record stands and the second is refused, because silently taking
    /// the newer one is exactly how a node would end up with two authoritative
    /// versions of one epoch.
    fn install(
        &self,
        epoch: u64,
        record: CommitteeRecord,
        lo: u64,
    ) -> Result<Arc<CommitteeRecord>, CommitteeError> {
        let record = Arc::new(record);
        let mut state = self.state.lock().unwrap();
        if let Some(entry) = state.records.get(&epoch) {
            let existing = entry.record.clone();
            if existing.same_value(&record) {
                return Ok(existing);
            }
            metrics::counter!(READ_PERMANENT, "reason" => REASON_FORK).increment(1);
            let first = state.reported.insert(epoch);
            drop(state);
            if first {
                error!(
                    epoch,
                    kept = ?existing.snapshot,
                    refused = ?record.snapshot,
                    "committee epoch read TWICE with two different values — the frozen \
                     committee is not frozen, i.e. the contract forked; keeping the first \
                     record and refusing this epoch"
                );
            }
            return Err(CommitteeError::Read(ReadError::AbiDecode(format!(
                "epoch {epoch} committee was already read with a different value"
            ))));
        }
        // Prune BEFORE the insert, so what this call returns is what the map
        // holds: the record just read was inside the window when it was read —
        // `lo` is the floor THAT read was gated on, not a fresher one — and an
        // anchor that jumped past it mid-read drops it on the next advance
        // rather than on the way out of here.
        Self::prune(&mut state, lo);
        state.records.insert(
            epoch,
            EpochEntry {
                record: record.clone(),
            },
        );
        Ok(record)
    }
}

#[cfg(test)]
impl<R> CommitteeStore<R> {
    /// The reads port, so a test can count staticcalls.
    pub(super) fn reads(&self) -> &R {
        &self.reads
    }

    /// The epochs currently held, oldest first — the retention observation.
    pub(super) fn cached_epochs(&self) -> Vec<u64> {
        self.state.lock().unwrap().records.keys().copied().collect()
    }

    /// The epochs whose permanent failure has been logged — the "once per
    /// epoch" observation, since the log line itself is not capturable here.
    pub(super) fn reported_epochs(&self) -> Vec<u64> {
        self.state
            .lock()
            .unwrap()
            .reported
            .iter()
            .copied()
            .collect()
    }
}

impl<R: EpochReads> Committee for CommitteeStore<R> {
    fn committee(&self, epoch: u64) -> Result<Arc<CommitteeRecord>, CommitteeError> {
        let anchor_height = self.anchor.height();

        // 0. GEOMETRY. Without `(activation, interval)` there is no epoch
        //    arithmetic at all — no window, no commit height — so there is
        //    nothing this node could read and no EVM call it could justify.
        //    `ready_at: 0` says so honestly: the retry is gated on the plane
        //    freezing the geometry, not on any height.
        let Some(geometry) = self.geometry() else {
            metrics::counter!(NOT_READABLE).increment(1);
            return Err(CommitteeError::NotReadable { epoch, ready_at: 0 });
        };

        // 1. WINDOW — a predicate on the request. No EVM, no lock, no anchor
        //    hash: an epoch outside the window is refused whatever the chain
        //    says, which is what makes a p2p frame naming an arbitrary epoch
        //    free to reject.
        let (lo, hi) = Self::window(&geometry, anchor_height);
        if epoch < lo || epoch > hi {
            let side = if epoch > hi { "above" } else { "below" };
            metrics::counter!(OUT_OF_WINDOW, "side" => side).increment(1);
            return Err(CommitteeError::OutOfWindow { epoch, lo, hi });
        }

        // 2. READABLE — is the epoch committed in a state this anchor covers?
        //    Also no EVM: on backfill, and in the first blocks after a start,
        //    the answer is arithmetic.
        let ready_at = geometry.commit_height(epoch);
        if anchor_height < ready_at {
            metrics::counter!(NOT_READABLE).increment(1);
            return Err(CommitteeError::NotReadable { epoch, ready_at });
        }

        // 3. CACHE. A hit is the final answer because the record is write-once
        //    and the anchor that produced it is irrelevant inside the window.
        //    The window is checked FIRST on purpose: an epoch that has dropped
        //    under the floor is refused, not answered from a leftover entry —
        //    and there is no such entry anyway, since retention IS the floor.
        if let Some(entry) = self.state.lock().unwrap().records.get(&epoch) {
            return Ok(entry.record.clone());
        }

        // 4. ANCHOR HASH. `Ok(None)` is "the height is not executed yet" — a
        //    park, not a fault. `Err` is a fault at a materialized height, and
        //    `failed` routes it by its own class: a header-index miss is
        //    permanent (`Backend`), a torn static-file read of the same storage
        //    is transient and costs this epoch one retry, not the epoch itself.
        let at = match self.anchor.executed_hash(anchor_height) {
            Ok(Some(hash)) => hash,
            Ok(None) => {
                metrics::counter!(NOT_READABLE).increment(1);
                return Err(CommitteeError::NotReadable {
                    epoch,
                    ready_at: anchor_height,
                });
            }
            Err(e) => return Err(self.failed(epoch, e, REASON_ANCHOR_FAULT)),
        };

        // 5. THE TWO STATICCALLS, at ONE hash. The snapshot does not carry the
        //    qual bit, so the bit is a second call — but at the same `at`, so
        //    the two halves of one question cannot describe two blocks.
        let snap = self
            .reads
            .epoch_committee_snapshot(epoch, at)
            .map_err(|e| self.failed(epoch, e, REASON_READ_ERROR))?;
        if snap.validators.is_empty() {
            // An uncommitted epoch is `Ok` with an empty committee, never an
            // error — the contract only ever skips a commit together with
            // halting the chain, so this is "not yet", never "never".
            metrics::counter!(NOT_READABLE).increment(1);
            return Err(CommitteeError::NotReadable {
                epoch,
                ready_at: anchor_height.saturating_add(1),
            });
        }
        let changed = self
            .reads
            .dkg_qual(epoch, at)
            .map_err(|e| self.failed(epoch, e, REASON_READ_ERROR))?;

        // 6. BUILD + INSTALL.
        let record = self.build(epoch, snap, changed)?;
        self.install(epoch, record, lo)
    }

    fn changed(&self, epoch: u64) -> Result<bool, CommitteeError> {
        Ok(self.committee(epoch)?.changed)
    }

    fn is_member(&self, epoch: u64, peer: &PeerPubkey) -> Result<bool, CommitteeError> {
        // Over `participants`, which the record derived once and keeps sorted,
        // rather than a scan of `members` in contract order: the record already
        // carries the structure that answers this in a binary search.
        Ok(self.committee(epoch)?.participants.position(peer).is_some())
    }

    fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.readable.subscribe()
    }

    fn anchor_advanced(&self) {
        // No geometry yet ⇒ no window to prune to and no readable epoch to
        // publish. Nothing is lost: the map is empty (every read so far
        // answered `NotReadable` before touching it) and the next advance after
        // the freeze publishes the real value.
        let Some(geometry) = self.geometry() else {
            return;
        };
        let anchor_height = self.anchor.height();
        let (lo, _) = Self::window(&geometry, anchor_height);
        {
            let mut state = self.state.lock().unwrap();
            Self::prune(&mut state, lo);
        }

        let highest = self.highest_readable(&geometry);
        self.readable.send_if_modified(|current| {
            if highest > *current {
                *current = highest;
                true
            } else {
                false
            }
        });
    }

    fn anchor_hash(&self) -> Option<B256> {
        self.anchor
            .executed_hash(self.anchor.height())
            .ok()
            .flatten()
    }
}

/// The production [`Anchor`]: the executor's ordering-finalized cursor and
/// reth's own finalized tag for the height, reth's materialized-state probe for
/// the hash.
///
/// SOURCE (a) of the two the design offered, and the reason is that it is not a
/// source at all — it is the cursor the executor already keeps. Every site that
/// raises `Executor::ordering_finalized` raises this same
/// [`FinalizedCursor`](crate::FinalizedCursor) with the same value in the same
/// arm (`executor.rs:1085`/`:1149`, `:2441`/`:2451`, `:3291`/`:3566`), so
/// introducing a second atomic here would have created a number that can
/// disagree with the one the rest of the executor acts on. See
/// [`FinalizedCursor::height`](crate::FinalizedCursor::height) for the one
/// window where the two differ and why lagging is the safe direction.
///
/// ## Why the reth tag is taken as a FLOOR
///
/// The cursor is a PROCESS quantity: it is born at zero and seeded inside
/// `OuterBuilder::build`, from the marshal's durable acked cursor
/// (`outer.rs` → `executor.rs` `Actor::init`). Reth's `finalized` tag is the
/// same node's DURABLE one: this node's executor set it, from certified data,
/// at `result_final_height(ordering_finalized, floor) = ordering_finalized − K`
/// (`executor.rs` `update_finalized`, the result tier), and reth carries it
/// across a restart. So on a fresh consensus datadir beside an already-synced
/// reth — a follower after a cold-start jump, a node whose consensus store was
/// wiped, any process between its plane being built and `build` running — the
/// cursor says 0 while the node demonstrably finalized height N. Anchoring on
/// the cursor alone makes the window `[0, 2]` there and every mid-chain epoch
/// unreadable until the first finalized derive.
///
/// `max` of the two is therefore the height, and it is safe in both directions:
/// the tag is `ordering_finalized − K` of a height THIS node finalized, so it
/// can never name a height the node has not finalized (the property
/// [`FinalizedCursor::height`](crate::FinalizedCursor::height) is chosen for),
/// and it can never exceed the cursor once the cursor is seeded. It is also
/// monotone — reth's tag only moves forward — so the window never shrinks.
/// `None` (no tag yet: a genuinely fresh EL) reads as 0 and changes nothing.
///
/// The cursor itself is NOT touched: it is the executor's Tier-F cursor, and
/// raising it from an EL tag would let a committee read move the tier the
/// result gate samples.
#[derive(Clone, Debug)]
pub struct RethAnchor<P> {
    cursor: crate::FinalizedCursor,
    provider: P,
}

impl<P> RethAnchor<P> {
    pub fn new(cursor: crate::FinalizedCursor, provider: P) -> Self {
        Self { cursor, provider }
    }
}

impl<P> Anchor for RethAnchor<P>
where
    P: reth_storage_api::BlockHashReader + reth_storage_api::BlockIdReader + Send + Sync,
{
    fn height(&self) -> u64 {
        let tag = self
            .provider
            .finalized_block_number()
            .ok()
            .flatten()
            .unwrap_or(0);
        self.cursor.height().max(tag)
    }

    fn executed_hash(&self, height: u64) -> Result<Option<B256>, ReadError> {
        crate::executed_state_hash(&self.provider, height)
    }
}
