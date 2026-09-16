use super::{
    Anchor, Committee, CommitteeError, CommitteeRecord, EpochReads, EpochVerifier, Geometry,
    GeometryRx, Member,
};
use alloy_primitives::B256;
use commonware_utils::TryFromIterator as _;
use fluentbase_bls::{PeerPubkey, Scheme as BlsScheme};
use fluentbase_staking_reader::{reader::ValidatorSetSnapshot, ReadClass, ReadError};
use fluentbase_types::staking_protocol::MAX_COMMITTEE_LOOKAHEAD_EPOCHS;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
use tracing::error;

use crate::SCHEME_RETENTION_EPOCHS;

/// Counts committee reads that can never succeed, labelled by `reason`; the
/// `error!` beside it has the detail.
pub(crate) const READ_PERMANENT: &str = "dpos_committee_read_permanent_total";
/// An epoch refused by the window predicate, by `side` (`above`/`below`). The
/// label is load-bearing: the two sides differ in whether a retry can help.
pub(crate) const OUT_OF_WINDOW: &str = "dpos_committee_out_of_window_total";
/// An epoch inside the window this node cannot see yet; expected while catching
/// up, so a rate that never falls means the chain committed nothing.
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

/// Counts scheme upgrades refused for dropping the beacon oracle.
pub(crate) const ORACLE_DROP_REFUSED: &str = "dpos_epoch_scheme_oracle_drop_refused_total";

/// One epoch's slot in the store's single map: the frozen record and the scheme
/// built from it. One slot rather than two maps, because a scheme must not
/// outlive the record's window nor exist for an epoch never read; `scheme` is
/// `None` until the [`EpochVerifier`] can build one.
struct EpochEntry {
    record: Arc<CommitteeRecord>,
    scheme: Option<Arc<BlsScheme>>,
}

/// One epoch's slot after the contract answered something no committed epoch
/// can answer ([`ReadClass::Impossible`]).
///
/// Memoised so the two staticcalls of a read that cannot succeed are paid once
/// per epoch instead of on every `committee(E)`; `reason` keeps the refusal
/// counted under the cause that produced it.
struct Poison {
    error: ReadError,
    reason: &'static str,
}

/// Everything the store mutates, behind one lock.
///
/// The lock is never held across a staking read: an `eth_call` into reth is a
/// blocking state read, and holding the map across it would serialize every
/// consumer behind the slowest reader.
#[derive(Default)]
struct State {
    records: BTreeMap<u64, EpochEntry>,
    /// Epochs whose permanent failure has already been logged.
    ///
    /// A [`ReadClass::Permanent`] failure is re-derived on every call, so
    /// without this the first reverting contract would log one `error!` per
    /// consumer per tick. Pruned by the same window floor as `records` — which
    /// is what makes "once per epoch" true — and bounded by the window's width.
    reported: BTreeSet<u64>,
    /// Epochs the contract answered impossibly, with the verdict to repeat —
    /// see [`Poison`]. A map beside `records` because a poisoned epoch has
    /// neither a record nor a scheme, and it takes precedence over `records`:
    /// a forked epoch is refused rather than answered from whichever read won.
    poisoned: BTreeMap<u64, Poison>,
}

/// The production [`Committee`] store, generic over the reads so tests can
/// count staticcalls and branch an answer by hash.
pub struct CommitteeStore<R> {
    reads: R,
    anchor: Arc<dyn Anchor>,
    geometry: GeometryRx,
    /// The single producer of a verify-only scheme — see [`EpochVerifier`].
    verifier: EpochVerifier,
    state: Mutex<State>,
    /// Highest readable epoch, published as a wake-up.
    readable: tokio::sync::watch::Sender<u64>,
}

impl<R: EpochReads> CommitteeStore<R> {
    /// `geometry` arrives lazily, because the store is built before the plane
    /// freezes `(activation, interval)`: until the watch holds a pair, every read
    /// answers [`CommitteeError::NotReadable`] with `ready_at: 0` and touches
    /// nothing. `verifier` runs on the record this store just installed, so no
    /// scheme can be built for a committee the module has not read.
    pub fn new(
        reads: R,
        anchor: Arc<dyn Anchor>,
        geometry: GeometryRx,
        verifier: EpochVerifier,
    ) -> Self {
        let seed = match Self::geometry_of(&geometry) {
            Some(g) => g.epoch_of(anchor.height()) + MAX_COMMITTEE_LOOKAHEAD_EPOCHS,
            None => 0,
        };
        Self {
            reads,
            anchor,
            geometry,
            verifier,
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

    fn highest_readable(&self, geometry: &Geometry) -> u64 {
        geometry.epoch_of(self.anchor.height()) + MAX_COMMITTEE_LOOKAHEAD_EPOCHS
    }

    /// Drop what the window no longer admits. Called with the lock held.
    ///
    /// Retention is the window floor, not a count: a count would evict epochs
    /// the module still answers for, so a later, different answer would install
    /// into the vacant slot unnoticed — the very fork the write-once check
    /// exists to catch.
    fn prune(state: &mut State, lo: u64) {
        state.records.retain(|epoch, _| *epoch >= lo);
        state.reported.retain(|epoch| *epoch >= lo);
        state.poisoned.retain(|epoch, _| *epoch >= lo);
    }

    /// `[lo, hi]`, inclusive both ends.
    fn window(geometry: &Geometry, anchor_height: u64) -> (u64, u64) {
        let anchor_epoch = geometry.epoch_of(anchor_height);
        (
            anchor_epoch.saturating_sub(SCHEME_RETENTION_EPOCHS as u64),
            anchor_epoch.saturating_add(MAX_COMMITTEE_LOOKAHEAD_EPOCHS),
        )
    }

    /// A permanent read failure: counted every time, logged once while the epoch
    /// stays in the window, then returned; a transient failure is returned
    /// uncounted, the normal shape of a node still catching up.
    ///
    /// [`ReadClass::Impossible`] poisons the epoch's slot — the chain cannot take
    /// that answer back — while a [`ReadClass::Permanent`] revert is re-derived,
    /// since repairing its cause must not need a restart. Every caller is past
    /// the window predicate, so the window is not re-checked here.
    fn failed(&self, epoch: u64, error: ReadError, reason: &'static str) -> CommitteeError {
        let class = error.class();
        if class != ReadClass::Transient {
            metrics::counter!(READ_PERMANENT, "reason" => reason).increment(1);
            let first = {
                let mut state = self.state.lock().unwrap();
                if class == ReadClass::Impossible {
                    state.poisoned.entry(epoch).or_insert_with(|| Poison {
                        error: error.clone(),
                        reason,
                    });
                }
                state.reported.insert(epoch)
            };
            if first {
                // Both messages below share the prefix "committee read failed PERMANENTLY
                // inside the read window", so an operator greps one string.
                if class == ReadClass::Impossible {
                    error!(
                        epoch,
                        %error,
                        "committee read failed PERMANENTLY inside the read window — the \
                         contract answered something no committed epoch can answer; this epoch \
                         will not be registered, no retry can fix it, and this node STOPS as \
                         soon as it must enter the epoch"
                    );
                } else {
                    error!(
                        epoch,
                        %error,
                        "committee read failed PERMANENTLY inside the read window — the read \
                         cannot succeed as things stand (the call reverted, or this node's own \
                         storage faulted); this epoch will not be registered and no retry can \
                         fix it — repair the cause and restart"
                    );
                }
            }
        }
        CommitteeError::Read(error)
    }

    fn build(
        &self,
        epoch: u64,
        snap: ValidatorSetSnapshot,
        changed: bool,
    ) -> Result<CommitteeRecord, CommitteeError> {
        // The BLS projection is the one derivation that can fail: the reader rejects
        // duplicate peer keys, but nothing on chain enforces uniqueness of the BLS
        // half. Taken through the production constructor, so there is no second
        // derivation here.
        let bls = crate::scheme::epoch_committee_from_snapshot(&snap).map_err(|e| {
            self.failed(
                epoch,
                ReadError::AbiDecode(format!(
                    "epoch {epoch} committee has non-unique consensus keys: {e}"
                )),
                REASON_NON_UNIQUE_KEYS,
            )
        })?;

        // `weights: None` inside the window is impossible (see
        // `WINDOW_FITS_THE_WEIGHT_RING`), so it means a contract fork, not a
        // missing optional; defaulting to zeros would give the leader elector a
        // uniform lottery, i.e. a per-node leader split rather than a failure.
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

        // Weights are positional — one per member, in the member order the leader
        // elector indexes — and the module is generic over the read port, so the
        // length is a checked precondition rather than an assumption.
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

        // A repeated peer key is refused rather than de-duplicated: the peer-set
        // projection must not hand out a roster shorter than the committee.
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
    /// The occupied arm is reachable, not dead code: the lock is not held across
    /// the staticcalls, so two consumers can miss the map together and both
    /// arrive here. Agreeing is the normal case; disagreeing means the chain
    /// answered two committees for one epoch, and the first record stands.
    fn install(
        &self,
        epoch: u64,
        record: CommitteeRecord,
        lo: u64,
    ) -> Result<Arc<CommitteeRecord>, CommitteeError> {
        let record = Arc::new(record);
        let scheme = (self.verifier)(&record).map(Arc::new);
        let mut state = self.state.lock().unwrap();
        if let Some(entry) = state.records.get(&epoch) {
            let existing = entry.record.clone();
            if existing.same_value(&record) {
                return Ok(existing);
            }
            metrics::counter!(READ_PERMANENT, "reason" => REASON_FORK).increment(1);
            let forked = ReadError::AbiDecode(format!(
                "epoch {epoch} committee was already read with a different value"
            ));
            // Poisoned even though a record sits right here: the chain stated two
            // different committees for this epoch, and serving whichever read won
            // the race would be serving a guess.
            state.poisoned.entry(epoch).or_insert_with(|| Poison {
                error: forked.clone(),
                reason: REASON_FORK,
            });
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
            return Err(CommitteeError::Read(forked));
        }
        // Prune before the insert, so what this call returns is what the map
        // holds: `lo` is the floor this read was gated on, and a record an
        // anchor jump has since passed stays until the next advance.
        Self::prune(&mut state, lo);
        state.records.insert(
            epoch,
            EpochEntry {
                record: record.clone(),
                scheme,
            },
        );
        Ok(record)
    }

    /// The scheme slot of an epoch already in the map, filling it if the
    /// verifier could not answer at install time. Called with no lock held.
    fn resolve_scheme(&self, epoch: u64) -> Option<Arc<BlsScheme>> {
        let record = {
            let state = self.state.lock().unwrap();
            let entry = state.records.get(&epoch)?;
            if let Some(scheme) = &entry.scheme {
                return Some(scheme.clone());
            }
            entry.record.clone()
        };
        // The install-time verifier answered `None` because the beacon did not
        // exist yet; retrying here rather than caching the absence keeps the
        // epoch from being pinned to no scheme by the order the plane was built.
        let built = Arc::new((self.verifier)(&record)?);
        let mut state = self.state.lock().unwrap();
        let entry = state.records.get_mut(&epoch)?;
        Some(entry.scheme.get_or_insert(built).clone())
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

    /// The epochs whose slot is poisoned — the memoised-refusal observation.
    pub(super) fn poisoned_epochs(&self) -> Vec<u64> {
        self.state
            .lock()
            .unwrap()
            .poisoned
            .keys()
            .copied()
            .collect()
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

        // Without geometry there is no epoch arithmetic — no window, no commit
        // height — so nothing can be read and no EVM call is justified;
        // `ready_at: 0` says the retry waits on the freeze, not on a height.
        let Some(geometry) = self.geometry() else {
            metrics::counter!(NOT_READABLE).increment(1);
            return Err(CommitteeError::NotReadable { epoch, ready_at: 0 });
        };

        // A predicate on the request, checked before any EVM call or lock: an
        // epoch outside the window is refused whatever the chain says, which is
        // what makes a p2p frame naming an arbitrary epoch free to reject.
        let (lo, hi) = Self::window(&geometry, anchor_height);
        if epoch < lo || epoch > hi {
            let side = if epoch > hi { "above" } else { "below" };
            metrics::counter!(OUT_OF_WINDOW, "side" => side).increment(1);
            return Err(CommitteeError::OutOfWindow { epoch, lo, hi });
        }

        // Answered by arithmetic, so backfill and the first blocks after a start
        // cost no EVM call.
        let ready_at = geometry.commit_height(epoch);
        if anchor_height < ready_at {
            metrics::counter!(NOT_READABLE).increment(1);
            return Err(CommitteeError::NotReadable { epoch, ready_at });
        }

        // A hit is final: the record is write-once and the anchor that produced it
        // does not matter inside the window; the window is checked first so an epoch
        // under the floor is refused rather than served from a leftover. A poisoned
        // slot is a hit too and is consulted first — the counter still ticks, but the
        // `error!` is not repeated, since `failed` logged it once.
        let cached = {
            let state = self.state.lock().unwrap();
            match state.poisoned.get(&epoch) {
                Some(poison) => Some(Err((poison.error.clone(), poison.reason))),
                None => state
                    .records
                    .get(&epoch)
                    .map(|entry| Ok(entry.record.clone())),
            }
        };
        match cached {
            Some(Ok(record)) => return Ok(record),
            Some(Err((error, reason))) => {
                metrics::counter!(READ_PERMANENT, "reason" => reason).increment(1);
                return Err(CommitteeError::Read(error));
            }
            None => {}
        }

        // `Ok(None)` parks: the height is not executed yet. `Err` is a fault at a
        // materialized height, routed by its own class — a torn static-file read
        // is transient, a header-index miss is not.
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

        // The snapshot does not carry the qual bit, so it takes a second call at
        // the same `at` — the two halves of one question cannot describe two blocks.
        let snap = self
            .reads
            .epoch_committee_snapshot(epoch, at)
            .map_err(|e| self.failed(epoch, e, REASON_READ_ERROR))?;
        if snap.validators.is_empty() {
            // An uncommitted epoch is `Ok` with an empty committee, never an
            // error: the contract only skips a commit together with halting the
            // chain, so this is "not yet", never "never".
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

        let record = self.build(epoch, snap, changed)?;
        self.install(epoch, record, lo)
    }

    fn changed(&self, epoch: u64) -> Result<bool, CommitteeError> {
        Ok(self.committee(epoch)?.changed)
    }

    fn is_member(&self, epoch: u64, peer: &PeerPubkey) -> Result<bool, CommitteeError> {
        // `participants` is sorted, so this is a binary search; a scan of
        // `members` would be linear and in contract order.
        Ok(self.committee(epoch)?.participants.position(peer).is_some())
    }

    fn scheme(&self, epoch: u64) -> Option<Arc<BlsScheme>> {
        // Reading the committee is what produces the scheme, so ask for it
        // first; outside the window or below the commit height that costs no EVM
        // call, and a hit is a map lookup.
        self.committee(epoch).ok()?;
        self.resolve_scheme(epoch)
    }

    fn upgrade_scheme(&self, epoch: u64, scheme: BlsScheme) -> bool {
        use commonware_cryptography::certificate::Scheme as _;
        let mut state = self.state.lock().unwrap();
        let Some(entry) = state.records.get_mut(&epoch) else {
            // No record means no committee this node has read, so there is no
            // strength to raise. Unreachable from the one production caller, which
            // upgrades a committee it holds — loud because reaching it would mean a
            // scheme came from somewhere else.
            error!(
                epoch,
                "refused a scheme upgrade for an epoch with no committee record — the scheme \
                 was derived from something this module did not read"
            );
            return false;
        };
        if entry.record.participants != *scheme.participants() {
            error!(
                epoch,
                "refused a scheme upgrade whose committee is not this epoch's — preserving the \
                 entry built from the record"
            );
            return false;
        }
        let Some(existing) = &entry.scheme else {
            entry.scheme = Some(Arc::new(scheme));
            return true;
        };
        if existing.me().is_some() && scheme.me().is_none() {
            error!(
                epoch,
                "refused a signer→verifier scheme downgrade — an engine's own scheme is never \
                 weakened underneath it"
            );
            return false;
        }
        if existing.is_beacon_active() && !scheme.is_beacon_active() {
            drop(state);
            metrics::counter!(ORACLE_DROP_REFUSED).increment(1);
            error!(
                epoch,
                "refused a scheme upgrade that DROPS the beacon oracle — the replacement admits \
                 a cleared seed slot where the entry it would replace refuses one (upgrades are \
                 monotone in verification strength)"
            );
            return false;
        }
        entry.scheme = Some(Arc::new(scheme));
        true
    }

    fn verifier_epochs(&self) -> Vec<u64> {
        use commonware_cryptography::certificate::Scheme as _;
        self.state
            .lock()
            .unwrap()
            .records
            .iter()
            .filter(|(_, e)| e.scheme.as_ref().is_some_and(|s| s.me().is_none()))
            .map(|(epoch, _)| *epoch)
            .collect()
    }

    fn latest_scheme(&self) -> Option<Arc<BlsScheme>> {
        self.state
            .lock()
            .unwrap()
            .records
            .values()
            .rev()
            .find_map(|e| e.scheme.clone())
    }

    fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.readable.subscribe()
    }

    fn geometry(&self) -> Option<Geometry> {
        // Delegates to the one accessor, so the window, `commit_height` and the
        // retention floor cannot disagree about `(activation, interval)`.
        Self::geometry_of(&self.geometry)
    }

    fn anchor_advanced(&self) {
        // No geometry means no window to prune to and no readable epoch to
        // publish; nothing is lost, since every read so far answered
        // `NotReadable` before touching the map.
        let Some(geometry) = self.geometry() else {
            return;
        };
        let anchor_height = self.anchor.height();
        let (lo, _) = Self::window(&geometry, anchor_height);
        {
            let mut state = self.state.lock().unwrap();
            Self::prune(&mut state, lo);
        }

        // Publish on every advance, not only on growth: an epoch below its commit
        // height opens when the anchor passes that height, while one whose anchor
        // height is not executed yet opens when execution catches up at the same
        // height — the value does not move. The published value stays monotone
        // through the `max`, so a consumer never sees the hint go backwards.
        let highest = self.highest_readable(&geometry);
        self.readable.send_if_modified(|current| {
            *current = (*current).max(highest);
            true
        });
    }

    fn anchor_hash(&self) -> Option<B256> {
        self.anchor
            .executed_hash(self.anchor.height())
            .ok()
            .flatten()
    }
}

/// The production [`Anchor`]: the executor's ordering-finalized cursor for the
/// height, reth's own finalized tag as a floor under it, and reth's
/// materialized-state probe for the hash.
///
/// The tag matters because the cursor is only a process quantity: on a fresh
/// consensus datadir beside an already-synced reth the cursor reads 0 while the
/// node has finalized height N, and the window would be `[0, 2]` until the first
/// finalized derive. The `max` is safe and monotone: the tag is
/// `ordering_finalized − K` of a height this node finalized, so it can never name
/// an unfinalized height, and it only moves forward, so the window never shrinks.
/// An absent tag reads as 0.
///
/// The cursor is left untouched: raising it from an EL tag would move the tier the
/// executor's result gate samples.
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
