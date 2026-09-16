//! Finality-gated epoch-boundary orchestrator.
//!
//! Every collaborator is a constructor parameter, so the module compiles and
//! unit-tests without the consensus, p2p and node layers.
//!
//! Invariants:
//! - only finalized blocks are applied — a reorg is impossible, so none is handled;
//! - `track` is write-once per epoch: a re-delivery of a covered epoch is a no-op;
//! - the committee size is checked here, so an oversized set is a typed error
//!   rather than a panic from commonware;
//! - cold start reads the current finalized committee once;
//! - `last_tracked_epoch` advances only after `boundary_tx.try_send` succeeds, so
//!   a full channel leaves the epoch un-tracked for the next block to retry;
//! - [`TransitionOutcome`] distinguishes a real advance from an intra-epoch no-op.

use alloy_primitives::B256;
use commonware_utils::ordered::Set;
use core::future::Future;
use fluentbase_bls::PeerPubkey;

use crate::{
    error::ReadError,
    reader::{check_peer_set_size, epoch_at_block, is_epoch_boundary, StakingStateRead},
};

/// Freeze the first observed value; later calls return it and warn on drift.
/// `what` names the field and the consensus authority it backs.
fn freeze_or_warn<T: Copy + PartialEq + std::fmt::Debug>(
    slot: &mut Option<T>,
    observed: T,
    what: &str,
) -> T {
    match *slot {
        Some(frozen) => {
            if observed != frozen {
                tracing::warn!(
                    ?frozen,
                    ?observed,
                    "{what} changed on-chain but is treated as fixed after genesis; ignoring"
                );
            }
            frozen
        }
        None => {
            *slot = Some(observed);
            observed
        }
    }
}

/// Outcome of [`EpochTransition::on_finalized`], distinguishing an intra-epoch
/// no-op from an actual epoch advance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionOutcome {
    /// No epoch state advanced: a re-delivery of a tracked epoch, a still-empty
    /// missed-commit epoch, or a retry whose boundary trigger was not delivered.
    Intra,
    /// `last_tracked_epoch` advanced to this value and the boundary trigger
    /// reached the consensus bridge.
    EpochAdvanced(u64),
}

/// Surface a replay advance even when the new delivery was a no-op; a
/// new-delivery advance takes precedence.
fn merge_replay_outcome(
    replay_advance: Option<TransitionOutcome>,
    new: TransitionOutcome,
) -> TransitionOutcome {
    match new {
        TransitionOutcome::EpochAdvanced(_) => new,
        TransitionOutcome::Intra => replay_advance.unwrap_or(new),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TriggerResult {
    /// Boundary trigger delivered, or no bridge configured: the epoch advanced.
    Advanced,
    /// Bridge channel full: retry the send on the next poke.
    Full,
    /// Bridge channel closed: unrecoverable, do not retry.
    Closed,
}

impl TriggerResult {
    fn into_outcome(self, epoch: u64) -> TransitionOutcome {
        match self {
            TriggerResult::Advanced => TransitionOutcome::EpochAdvanced(epoch),
            TriggerResult::Full | TriggerResult::Closed => TransitionOutcome::Intra,
        }
    }
}

/// The two tiers of one epoch's peer set, kept apart to the `Oracle`: commonware
/// treats them differently, and every channel's membership check asks which epoch
/// a peer owes its place to.
///
/// `primary` holds the per-epoch committee records (`E−1`, `E`, `E+1`) rather than
/// their union, so a frame's consumer can ask which record a sender sits in;
/// [`Self::primary`] takes the union commonware wants, [`Self::epochs_of`] answers
/// the membership question.
///
/// `secondary` is the Active validator registry: commonware never dials it, gossips
/// bit-vecs about it or caches its bodies, but does accept and serve its inbound
/// connections — the tier an ejected or upcoming validator belongs in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackedPeers {
    /// `(epoch, committee[epoch])` for each readable record among `E−1`, `E`,
    /// `E+1`, ascending. An unreadable record (below the contract's window at
    /// cold start, or not committed yet) is absent rather than empty: "no
    /// record" and "empty committee" are different answers, and only the second
    /// is a fault.
    ///
    /// At most three entries, with distinct epochs; consumers size fixed
    /// three-slot masks off that bound, and a fourth record or a repeated epoch
    /// is a producer bug rather than something they cope with.
    ///
    /// Public so the p2p mask can read it and tests can build a window by hand.
    pub committees: Vec<(u64, Set<PeerPubkey>)>,
    /// Tier 2: every Active registry entry at the anchor.
    pub secondary: Set<PeerPubkey>,
}

impl TrackedPeers {
    pub fn primary(&self) -> Set<PeerPubkey> {
        Set::from_iter_dedup(
            self.committees
                .iter()
                .flat_map(|(_, members)| members.iter().cloned()),
        )
    }

    /// The epochs whose carried record contains `peer` — the membership mask a
    /// channel's ingress check reads; empty means the peer is not primary here.
    pub fn epochs_of<'a>(&'a self, peer: &'a PeerPubkey) -> impl Iterator<Item = u64> + 'a {
        self.committees
            .iter()
            .filter(move |(_, members)| members.position(peer).is_some())
            .map(|(epoch, _)| *epoch)
    }
}

/// Where the assembled peer set is delivered; p2p-agnostic on purpose, so
/// `staking-reader` does not depend on `commonware-p2p`.
pub trait PeerSetSink {
    fn track(&mut self, epoch: u64, peers: TrackedPeers) -> impl Future<Output = ()> + Send;
}

/// Re-poke cadence for a parked boundary: callers retry `on_finalized` with this
/// backoff until [`EpochTransition::has_pending_boundary`] clears.
pub const PENDING_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(200);

pub struct EpochTransition<R, S> {
    reader: R,
    sink: S,
    /// commonware `max_peer_set_size`, injected by the node; the committee-size
    /// guard input.
    max_peer_set_size: usize,
    /// Write-once guard: the epoch already fed to `track`.
    last_tracked_epoch: Option<u64>,
    /// Boundary trigger for the engine's `OuterEngine::boundary_sender`: every
    /// advance fires `(epoch, snapshot)` once. `try_send` is lossy on purpose, and
    /// a closed receiver means the consumer has already shut down.
    boundary_tx: Option<tokio::sync::mpsc::Sender<(u64, crate::reader::ValidatorSetSnapshot)>>,
    /// `epochBlockInterval` frozen on the first finalized block: consensus's
    /// `FixedEpocher` is frozen at startup, so following a live governance change
    /// here would diverge the two epoch authorities. Drift is logged and ignored.
    frozen_interval: Option<u64>,
    /// `dposActivationBlock` frozen on the first finalized block: the origin for
    /// relative epoch numbering, fixed because consensus's `OriginEpocher` is
    /// frozen at startup.
    frozen_activation: Option<u64>,
    /// Materialized-state-gated EVM hash by height: committee reads resolve at
    /// `number − result_lag`, a height that can have a header without executed
    /// state. `Ok(Some)` = materialized; `Ok(None)` = above reth's
    /// `best_block_number()` (pipeline backfill), which parks the caller; `Err` =
    /// a real read fault at a materialized height, which must not be folded into
    /// the park.
    executed_hash: std::sync::Arc<dyn Fn(u64) -> Result<Option<B256>, ReadError> + Send + Sync>,
    /// Result lag K, injected because this crate must not depend on consensus.
    result_lag: u64,
    /// Read-height floor: the cold-start anchor, which is executed by
    /// construction, so heights at or below it need no gate.
    anchor_height: Option<u64>,
    /// Boundary remembered while the executed tip lagged its read height. Only
    /// boundary heights are stored: an unconditional overwrite would let a
    /// non-boundary clobber a remembered boundary during a sustained lag and lose
    /// the epoch enter forever.
    pending_boundary: Option<u64>,
    /// The boundary height whose empty-committee park has already been warned: the
    /// re-poke loop drops to `debug!` on later re-reads. Overwritten when a
    /// different boundary parks empty, so each distinct boundary warns once.
    warned_empty_boundary: Option<u64>,
}

impl<R, S> EpochTransition<R, S>
where
    R: StakingStateRead,
    S: PeerSetSink,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        reader: R,
        sink: S,
        max_peer_set_size: usize,
        boundary_tx: Option<tokio::sync::mpsc::Sender<(u64, crate::reader::ValidatorSetSnapshot)>>,
        executed_hash: std::sync::Arc<dyn Fn(u64) -> Result<Option<B256>, ReadError> + Send + Sync>,
        result_lag: u64,
    ) -> Self {
        Self {
            reader,
            sink,
            max_peer_set_size,
            last_tracked_epoch: None,
            boundary_tx,
            frozen_interval: None,
            frozen_activation: None,
            executed_hash,
            result_lag,
            anchor_height: None,
            pending_boundary: None,
            warned_empty_boundary: None,
        }
    }

    /// The relative epoch `number` falls in, over the frozen geometry; `None`
    /// until the geometry freezes.
    pub fn epoch_at(&self, number: u64) -> Option<u64> {
        epoch_at_block(number, self.frozen_activation?, self.frozen_interval?)
    }

    fn is_epoch_boundary_frozen(&self, number: u64) -> Option<bool> {
        Some(is_epoch_boundary(
            number,
            self.frozen_activation?,
            self.frozen_interval?,
        ))
    }

    /// Whether a boundary is parked awaiting execution catch-up. Callers must
    /// re-poke (retry with [`PENDING_RETRY_BACKOFF`]) after a delivery that
    /// leaves this set: during catch-up the parked boundary is the last
    /// deliverable block, so no later delivery will trigger the replay.
    pub fn has_pending_boundary(&self) -> bool {
        self.pending_boundary.is_some()
    }

    /// The parked boundary height; `None` when nothing is parked. Feeds the
    /// signer hook's gauge, which alerts on a boundary parked for too long.
    pub fn pending_boundary(&self) -> Option<u64> {
        self.pending_boundary
    }

    /// The frozen `(dposActivationBlock, epochBlockInterval)` once a readable,
    /// DPoS-scheduled anchor has been resolved; `None` until then. The single
    /// in-plane source of the epoch geometry.
    pub fn frozen_geometry(&self) -> Option<(u64, u64)> {
        Some((self.frozen_activation?, self.frozen_interval?))
    }

    /// The executed height committee reads resolve at for an ordering-finalized
    /// `number`: `number − result_lag`, clamped to the cold-start anchor (at or
    /// below the anchor is executed by construction).
    ///
    /// The lagged read point loses nothing: the committee array, the per-member
    /// leader weights and the consensus keys it carries are all frozen at commit.
    fn read_height_for(&self, number: u64) -> u64 {
        let floor = self.anchor_height.unwrap_or(0);
        number.saturating_sub(self.result_lag).max(floor)
    }

    /// Raise the read-height floor — monotone forward, never lowers.
    ///
    /// Published by the executor when a steady-state re-jump lands: the jumped-over
    /// history is gone, so the boundary the landing enters (often the previous
    /// epoch's terminal, up to a full epoch below the tip) would read at
    /// `number − result_lag` — a pruned height — and the caller's retry arm would
    /// re-read it forever instead of entering the epoch.
    ///
    /// Both writers of the floor — [`Self::cold_start`] and this setter — take
    /// the max, so the guarantee does not depend on their call order.
    pub fn raise_anchor_height(&mut self, height: u64) {
        self.anchor_height = Some(self.anchor_height.map_or(height, |a| a.max(height)));
    }

    /// Apply one finalized block `B`, delivered sequentially by commonware
    /// `Reporter` and acked.
    ///
    /// Idempotent per epoch: a re-delivery is a no-op and never a re-`track`, and
    /// the side effects a stalled retry re-runs are individually idempotent.
    pub async fn on_finalized(&mut self, number: u64) -> Result<TransitionOutcome, ReadError> {
        if self.frozen_interval.is_none() {
            return Err(ReadError::Backend(
                "on_finalized before cold_start (epoch geometry not frozen)".into(),
            ));
        }
        // Boundaries are replayed before the new delivery so they are applied in
        // height order.
        let mut replay_advance: Option<TransitionOutcome> = None;
        if let Some(b) = self.pending_boundary {
            if let Some(at) = (self.executed_hash)(self.read_height_for(b))? {
                // `apply_at` owns the slot: an error propagates with it untouched.
                // `b` is the last deliverable block during catch-up, so dropping it
                // would wedge the next epoch forever.
                let replay = self.apply_at(b, at).await?;
                tracing::debug!(boundary = b, ?replay, "replayed pending boundary");
                if matches!(replay, TransitionOutcome::EpochAdvanced(_)) {
                    replay_advance = Some(replay);
                }
            }
        }
        // An un-executed height must park, never be read: a state read there
        // fails and the caller would retry the same dead height forever.
        let Some(at) = (self.executed_hash)(self.read_height_for(number))? else {
            // Transient while the executed tip has not reached `number −
            // result_lag`; only boundaries are worth replaying.
            if self.is_epoch_boundary_frozen(number) == Some(true) {
                // Single slot: a second park would clobber the first and drop that
                // epoch's handoff. Only this delivery path can park — the executor's
                // landing reads at a floor it has already executed, and cold start
                // clears the slot — so a second park means a third producer appeared.
                debug_assert!(
                    self.pending_boundary.is_none_or(|p| p == number),
                    "two boundaries pending at once (parked {:?}, new {number}): the park slot \
                     has more than one producer — only the in-order delivery path may park",
                    self.pending_boundary,
                );
                self.pending_boundary = Some(number);
            }
            return Ok(merge_replay_outcome(
                replay_advance,
                TransitionOutcome::Intra,
            ));
        };
        let outcome = self.apply_at(number, at).await?;
        Ok(merge_replay_outcome(replay_advance, outcome))
    }

    /// `Ok(None)` = DPoS is not a scheduled, deployed chain at `at` yet: the
    /// ChainConfig staticcalls revert (codeless account) or read the `0`
    /// unscheduled sentinel. A cold restart can momentarily anchor on the genesis
    /// fallback before reth surfaces its persisted finalized marker, and freezing
    /// there would mis-read the geometry, so the caller stays unfrozen and retries
    /// at a later height.
    fn resolve_and_freeze(&mut self, at: B256) -> Result<Option<(u64, u64)>, ReadError> {
        let Some(scheduled_activation) = self.reader.scheduled_dpos_activation(at)? else {
            return Ok(None);
        };
        let observed = self.reader.epoch_block_interval(at)?;
        if observed == 0 {
            return Err(ReadError::ZeroEpochInterval);
        }
        let interval = freeze_or_warn(
            &mut self.frozen_interval,
            observed,
            "epochBlockInterval (consensus FixedEpocher is frozen)",
        );
        let activation = freeze_or_warn(
            &mut self.frozen_activation,
            scheduled_activation,
            "dposActivationBlock (consensus OriginEpocher is frozen)",
        );
        Ok(Some((activation, interval)))
    }

    /// `Ok(true)` = this call froze the geometry; `Ok(false)` = it was already
    /// frozen, or DPoS is not scheduled at `at` yet, so the caller retries at a
    /// later height. Idempotent.
    ///
    /// Deliberately skips the bootstrap — no `track`, bridge trigger, read floor or
    /// park — because that branch is write-once and must be driven from an
    /// ordering-scale anchor: off the plane's lagging EL cursor it would pick the
    /// wrong epoch.
    pub fn freeze_geometry(&mut self, at: B256) -> Result<bool, ReadError> {
        if self.frozen_geometry().is_some() {
            return Ok(false);
        }
        Ok(self.resolve_and_freeze(at)?.is_some())
    }

    async fn apply_at(&mut self, number: u64, at: B256) -> Result<TransitionOutcome, ReadError> {
        let Some((activation, interval)) = self.resolve_and_freeze(at)? else {
            return Ok(TransitionOutcome::Intra);
        };
        // The interval is non-zero (checked above), so the shared epoch function
        // cannot answer `None` here.
        let epoch_e =
            epoch_at_block(number, activation, interval).ok_or(ReadError::ZeroEpochInterval)?;

        // Activation-relative, matching `epoch_at_block` and consensus's
        // `OriginEpocher`: the absolute form agrees only when
        // `activation % interval == 0`, which production anchors do not guarantee.
        let is_boundary = is_epoch_boundary(number, activation, interval);

        // Cold-start bootstrap: stand up the current epoch's engine and return, so
        // this never also takes the boundary branch below and double-spawns. On a
        // boundary the network has already advanced, so bootstrap E+1 — entering E
        // would hint a marshal floor the node already holds and deadlock the resume.
        if self.last_tracked_epoch.is_none() {
            // This branch re-runs on every delivery until it advances, so a park left
            // by an earlier delivery must be released here or the re-poke loop would
            // spin on a slot nothing clears.
            self.pending_boundary = None;
            let cold_epoch = if is_boundary { epoch_e + 1 } else { epoch_e };
            let snap = self.reader.epoch_committee_snapshot(cold_epoch, at)?;
            if snap.validators.is_empty() {
                return Ok(TransitionOutcome::Intra);
            }
            return Ok(self
                .track_and_trigger(cold_epoch, snap, at)
                .await?
                .into_outcome(cold_epoch));
        }

        // When the last block of epoch E finalizes, spawn epoch E+1; its committee
        // was committed well before this read, because the node's pre-execution
        // stage commits every epoch up to `current + MAX_COMMITTEE_LOOKAHEAD_EPOCHS`.
        let next = epoch_e + 1;
        if is_boundary && self.last_tracked_epoch < Some(next) {
            // An epoch with no `commitEpochCommittee` is legal (unslashable by
            // design) and reads back empty. Never track an empty set: skipping lets a
            // later finalized block apply it while commonware keeps the prior set.
            let snap = self.reader.epoch_committee_snapshot(next, at)?;
            if snap.validators.is_empty() {
                // An empty read at the spawn height is a state-visibility lag, not a
                // missed commit: the ahead-commit loop froze this committee a whole
                // epoch ago. Keep the boundary parked so the re-poke loop re-reads
                // until the snapshot materializes; dropping it would lose the
                // epoch-E+1 engine spawn permanently.
                if self.warned_empty_boundary == Some(number) {
                    tracing::debug!(
                        epoch = next,
                        boundary = number,
                        "epoch boundary: committee[next] still empty — re-poking parked boundary"
                    );
                } else {
                    tracing::warn!(
                        epoch = next,
                        boundary = number,
                        "epoch boundary: committee[next] empty at the spawn height — parking \
                         for re-poke (transient state-visibility lag; the ahead-commit loop \
                         froze this committee an epoch ago)"
                    );
                    self.warned_empty_boundary = Some(number);
                }
                self.pending_boundary = Some(number);
                return Ok(TransitionOutcome::Intra);
            }
            let result = self.track_and_trigger(next, snap, at).await?;
            // Keep the boundary parked only on `Full`: no later delivery re-detects
            // it during catch-up. A real advance or a closed channel (forwarder gone;
            // retrying only spins during teardown) releases it.
            self.pending_boundary = match result {
                TriggerResult::Full => Some(number),
                TriggerResult::Advanced | TriggerResult::Closed => None,
            };
            return Ok(result.into_outcome(next));
        }
        Ok(TransitionOutcome::Intra)
    }

    /// The peer set for `epoch`: primary is `committee[epoch − 1] ∪
    /// committee[epoch] ∪ committee[epoch + 1]` as separate records, secondary is
    /// the Active registry at the anchor.
    ///
    /// These records come from the staking reader rather than consensus's
    /// `committee` module, because this crate sits below consensus in the
    /// dependency graph and the two cannot share the read.
    ///
    /// Those three committees are the ones whose traffic is legitimate while `epoch`
    /// is tracked: `epoch + 1` agrees its epoch key during `epoch`, and `epoch − 1`
    /// is still finalizing and answering resolver fetches for its own rounds, so
    /// dropping the outgoing one would partition the plane at a full turnover.
    ///
    /// The size guard checks primary only — commonware panics on an oversized primary
    /// set and does not check secondary at all.
    ///
    /// An uncommitted neighbour reads back `Ok` with no validators and is skipped as
    /// a record; a failed read is `Err` and replays the whole boundary. Skipping is
    /// safe — a member of the skipped record is refused at the channel's ingress gate
    /// until the next `track` carries it, and the dealer leg re-sends — but degrading
    /// to a short set on `Err` is not: the epoch advances on it and commonware ignores
    /// a re-`track` of an index it already holds, so one failed read would cost the
    /// epoch its neighbour reachability.
    fn assemble_tracked_peers(
        &self,
        epoch: u64,
        snap: &crate::reader::ValidatorSetSnapshot,
        at: B256,
    ) -> Result<TrackedPeers, ReadError> {
        let secondary = Set::from_iter_dedup(self.reader.active_registry_peers(at)?);
        let mut committees: Vec<(u64, Set<PeerPubkey>)> = Vec::with_capacity(3);
        if let Some(prev) = epoch.checked_sub(1) {
            self.push_neighbour_committee(&mut committees, prev, at, "outgoing")?;
        }
        committees.push((
            epoch,
            Set::from_iter_dedup(snap.validators.iter().map(|v| v.keys.peer_pubkey.clone())),
        ));
        self.push_neighbour_committee(&mut committees, epoch + 1, at, "incoming")?;
        let tracked = TrackedPeers {
            committees,
            secondary,
        };
        check_peer_set_size(epoch, tracked.primary().len(), self.max_peer_set_size)?;
        Ok(tracked)
    }

    fn push_neighbour_committee(
        &self,
        committees: &mut Vec<(u64, Set<PeerPubkey>)>,
        neighbour: u64,
        at: B256,
        which: &'static str,
    ) -> Result<(), ReadError> {
        let record = self.reader.epoch_committee_snapshot(neighbour, at)?;
        if record.validators.is_empty() {
            tracing::debug!(
                epoch = neighbour,
                which,
                "neighbour committee is not committed at this anchor; peer-set tier skipped"
            );
            return Ok(());
        }
        committees.push((
            neighbour,
            Set::from_iter_dedup(record.validators.iter().map(|v| v.keys.peer_pubkey.clone())),
        ));
        Ok(())
    }

    /// Register the peer set for the epoch `number` falls in, and nothing else.
    ///
    /// `Ok(Some(epoch))` = that epoch's set went to the sink; `Ok(None)` = the
    /// geometry is not frozen yet, or `committee[epoch]` reads empty at `at`, so the
    /// caller retries at a later height. The epoch is chosen by the same rule the
    /// bootstrap branch of [`Self::apply_at`] uses (a boundary height already belongs
    /// to `E + 1`), so an early registration never names the committee the chain has
    /// just left.
    ///
    /// It exists because an empty-archive validator parks in the layer's cold-start
    /// jump loop until a tracked plane peer serves it a frontier, and the one
    /// bootstrapper — the only other path to a `track` — runs after that loop. It
    /// therefore touches none of the bootstrap state (`last_tracked_epoch`, the read
    /// floor, the park, the bridge); a later re-registration of the same index is
    /// harmless.
    pub async fn track_peers(&mut self, at: B256, number: u64) -> Result<Option<u64>, ReadError> {
        // No freeze attempt here: the plane's cursor must not be what fixes the
        // geometry.
        let Some(epoch_e) = self.epoch_at(number) else {
            return Ok(None);
        };
        let epoch = if self.is_epoch_boundary_frozen(number) == Some(true) {
            epoch_e + 1
        } else {
            epoch_e
        };
        let snap = self.reader.epoch_committee_snapshot(epoch, at)?;
        if snap.validators.is_empty() {
            // Tracking an empty set would replace the peer set commonware holds,
            // so skip and retry.
            return Ok(None);
        }
        let tracked = self.assemble_tracked_peers(epoch, &snap, at)?;
        self.sink.track(epoch, tracked).await;
        Ok(Some(epoch))
    }

    async fn track_and_trigger(
        &mut self,
        epoch: u64,
        snap: crate::reader::ValidatorSetSnapshot,
        at: B256,
    ) -> Result<TriggerResult, ReadError> {
        let tracked = self.assemble_tracked_peers(epoch, &snap, at)?;
        self.sink.track(epoch, tracked).await;

        if let Some(tx) = self.boundary_tx.as_ref() {
            match tx.try_send((epoch, snap)) {
                Ok(()) => {}
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    tracing::warn!(epoch, "bridge channel full; retry on next finalized block");
                    return Ok(TriggerResult::Full);
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    tracing::error!(epoch, "bridge channel closed — forwarder has shut down");
                    return Ok(TriggerResult::Closed);
                }
            }
        }
        self.last_tracked_epoch = Some(epoch);
        Ok(TriggerResult::Advanced)
    }

    /// Cold start: freeze the epoch geometry, read the current finalized committee
    /// at the explicit anchor hash `head` (executed by construction, so no
    /// `executed_hash` resolution is needed), apply once, and raise the read-height
    /// floor for every later `on_finalized`. Must run before `on_finalized`.
    ///
    /// The only entry point that picks the starting epoch: the beacon plane's
    /// [`Self::freeze_geometry`] leaves `last_tracked_epoch` at `None`, so this call
    /// takes the bootstrap branch of [`Self::apply_at`].
    pub async fn cold_start(
        &mut self,
        head: B256,
        head_number: u64,
    ) -> Result<TransitionOutcome, ReadError> {
        self.raise_anchor_height(head_number);
        self.apply_at(head_number, head).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::{ConsensusKeys, ValidatorSetSnapshot, ValidatorWithKeys};
    use alloy_primitives::Address;
    use commonware_codec::DecodeExt as _;
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer};
    use commonware_math::algebra::Random as _;
    use commonware_runtime::{deterministic, Runner};
    use fluentbase_bls::BlsPubkey;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;
    use std::sync::{Arc, Mutex};

    fn validator(seed: u64) -> ValidatorWithKeys {
        let mut rng = StdRng::seed_from_u64(seed);
        let peer = Ed25519PrivateKey::random(&mut rng).public_key();
        let bls = BlsPubkey::decode(
            fluentbase_bls::keys::ValidatorBlsKeypair::generate(&mut rng)
                .public_bytes()
                .as_slice(),
        )
        .unwrap();
        ValidatorWithKeys {
            address: Address::repeat_byte(seed as u8),
            keys: ConsensusKeys {
                bls_pubkey: bls,
                peer_pubkey: peer,
                activation_epoch: 1,
            },
            tombstoned: false,
        }
    }

    struct MockReader {
        committee: usize,
        interval: u64,
    }
    impl StakingStateRead for MockReader {
        fn epoch_committee_snapshot(
            &self,
            epoch: u64,
            at: B256,
        ) -> Result<ValidatorSetSnapshot, ReadError> {
            Ok(ValidatorSetSnapshot {
                block_hash: at,
                block_number: epoch * 100,
                epoch,
                validators: (0..self.committee as u64)
                    .map(|i| validator(epoch * 1000 + i))
                    .collect(),
                weights: None,
            })
        }
        fn epoch_block_interval(&self, _at: B256) -> Result<u64, ReadError> {
            Ok(self.interval)
        }
        fn dpos_activation_block(&self, _at: B256) -> Result<u64, ReadError> {
            Ok(0) // absolute numbering
        }
        fn active_registry_peers(&self, _at: B256) -> Result<Vec<PeerPubkey>, ReadError> {
            // Empty registry, so the tracked set equals the committee.
            Ok(vec![])
        }
    }

    /// Test constructor: a resolver that always resolves to `h`, so every height
    /// looks executed.
    fn et(
        reader: MockReader,
        sink: RecordingSink,
        max: usize,
        tx: Option<tokio::sync::mpsc::Sender<(u64, crate::reader::ValidatorSetSnapshot)>>,
        h: B256,
    ) -> EpochTransition<MockReader, RecordingSink> {
        EpochTransition::new(
            reader,
            sink,
            max,
            tx,
            std::sync::Arc::new(move |_n| Ok(Some(h))),
            3,
        )
    }

    /// `MockReader` plus a non-empty tier-2 registry, disjoint from the committee.
    struct RegistryReader {
        inner: MockReader,
        registry: Vec<PeerPubkey>,
    }
    impl StakingStateRead for RegistryReader {
        fn epoch_committee_snapshot(
            &self,
            epoch: u64,
            at: B256,
        ) -> Result<ValidatorSetSnapshot, ReadError> {
            self.inner.epoch_committee_snapshot(epoch, at)
        }
        fn epoch_block_interval(&self, at: B256) -> Result<u64, ReadError> {
            self.inner.epoch_block_interval(at)
        }
        fn dpos_activation_block(&self, at: B256) -> Result<u64, ReadError> {
            self.inner.dpos_activation_block(at)
        }
        fn active_registry_peers(&self, _at: B256) -> Result<Vec<PeerPubkey>, ReadError> {
            Ok(self.registry.clone())
        }
    }

    /// `MockReader` with a future `dposActivationBlock`.
    struct FutureActivationReader {
        inner: MockReader,
        activation: u64,
    }
    impl StakingStateRead for FutureActivationReader {
        fn epoch_committee_snapshot(
            &self,
            epoch: u64,
            at: B256,
        ) -> Result<ValidatorSetSnapshot, ReadError> {
            self.inner.epoch_committee_snapshot(epoch, at)
        }
        fn epoch_block_interval(&self, at: B256) -> Result<u64, ReadError> {
            self.inner.epoch_block_interval(at)
        }
        fn dpos_activation_block(&self, _at: B256) -> Result<u64, ReadError> {
            Ok(self.activation)
        }
        fn active_registry_peers(&self, at: B256) -> Result<Vec<PeerPubkey>, ReadError> {
            self.inner.active_registry_peers(at)
        }
    }

    /// A committee read above `ok_through` is unavailable — empty (not committed)
    /// or a hard failure, the two ways `committee[epoch + 1]` can be missing; the
    /// requested epochs are recorded so a test can prove the union asked for it.
    struct IncomingUnavailableReader {
        inner: MockReader,
        ok_through: u64,
        fail: bool,
        requested: Arc<Mutex<Vec<u64>>>,
    }
    impl StakingStateRead for IncomingUnavailableReader {
        fn epoch_committee_snapshot(
            &self,
            epoch: u64,
            at: B256,
        ) -> Result<ValidatorSetSnapshot, ReadError> {
            self.requested.lock().unwrap().push(epoch);
            if epoch > self.ok_through {
                if self.fail {
                    return Err(ReadError::Backend(format!(
                        "committee[{epoch}] read failed"
                    )));
                }
                return Ok(ValidatorSetSnapshot {
                    block_hash: at,
                    block_number: epoch * 100,
                    epoch,
                    validators: vec![],
                    weights: None,
                });
            }
            self.inner.epoch_committee_snapshot(epoch, at)
        }
        fn epoch_block_interval(&self, at: B256) -> Result<u64, ReadError> {
            self.inner.epoch_block_interval(at)
        }
        fn dpos_activation_block(&self, at: B256) -> Result<u64, ReadError> {
            self.inner.dpos_activation_block(at)
        }
        fn active_registry_peers(&self, at: B256) -> Result<Vec<PeerPubkey>, ReadError> {
            self.inner.active_registry_peers(at)
        }
    }

    /// Records the full tracked set, not just its size: the union's point is which
    /// keys reach the plane, and both tiers matter.
    type TrackedSets = Arc<Mutex<Vec<(u64, TrackedPeers)>>>;
    #[derive(Clone, Default)]
    struct KeySink(TrackedSets);
    impl PeerSetSink for KeySink {
        fn track(&mut self, epoch: u64, peers: TrackedPeers) -> impl Future<Output = ()> + Send {
            let log = self.0.clone();
            async move {
                log.lock().unwrap().push((epoch, peers));
            }
        }
    }

    /// Records every `track` call as `(epoch, |primary|)` — the size the
    /// commonware cap is taken against.
    #[derive(Clone, Default)]
    struct RecordingSink(Arc<Mutex<Vec<(u64, usize)>>>);
    impl PeerSetSink for RecordingSink {
        fn track(&mut self, epoch: u64, peers: TrackedPeers) -> impl Future<Output = ()> + Send {
            let log = self.0.clone();
            async move {
                log.lock().unwrap().push((epoch, peers.primary().len()));
            }
        }
    }

    /// Primary is the three committees and nothing else; the registry is tier 2.
    /// A registry-only key must not appear in `primary()`, and the outgoing
    /// committee `C[E−1]` must have joined it.
    #[test]
    fn the_registry_is_tier_two_and_the_outgoing_committee_is_tier_one() {
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = KeySink::default();
            let h = B256::repeat_byte(0x33);
            // Seeds far from every committee's, so the three committees are
            // pairwise disjoint.
            let registry_only: Vec<PeerPubkey> = vec![
                validator(900_001).keys.peer_pubkey,
                validator(900_002).keys.peer_pubkey,
            ];
            let reader = RegistryReader {
                inner: MockReader {
                    committee: 3,
                    interval: 100,
                },
                registry: registry_only.clone(),
            };
            let mut et = EpochTransition::new(
                reader,
                sink.clone(),
                51,
                None,
                std::sync::Arc::new(move |_n| Ok(Some(h))),
                3,
            );
            et.cold_start(h, 200).await.unwrap();

            let log = sink.0.lock().unwrap();
            let [(epoch, tracked)] = log.as_slice() else {
                panic!("expected exactly one track call, got {log:?}");
            };
            assert_eq!(*epoch, 2);
            assert_eq!(
                tracked
                    .committees
                    .iter()
                    .map(|(e, _)| *e)
                    .collect::<Vec<_>>(),
                vec![1, 2, 3],
                "primary carries C[E-1], C[E], C[E+1] as separate records"
            );
            let primary = tracked.primary();
            assert_eq!(primary.len(), 9, "three disjoint committees of 3");
            let reader = MockReader {
                committee: 3,
                interval: 100,
            };
            for member in reader.epoch_committee_snapshot(1, h).unwrap().validators {
                assert!(
                    primary.position(&member.keys.peer_pubkey).is_some(),
                    "outgoing committee[1] member {:?} missing from primary",
                    member.address
                );
            }
            for peer in &registry_only {
                assert!(
                    primary.position(peer).is_none(),
                    "a registry entry in no committee must not be primary"
                );
                assert!(
                    tracked.secondary.position(peer).is_some(),
                    "a registry entry in no committee must be secondary"
                );
                assert_eq!(
                    tracked.epochs_of(peer).count(),
                    0,
                    "a secondary peer's membership mask is empty"
                );
            }
            assert_eq!(
                tracked.secondary.len(),
                2,
                "the registry is the whole of it"
            );
        });
    }

    #[test]
    fn incoming_committee_is_in_the_tracked_peer_set() {
        // The E+1 agreement instance runs during E and `buffered` retains a body
        // only from a tracked sender, so committee[E+1] must already be tracked
        // when E starts.
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = KeySink::default();
            let (boundary_tx, mut boundary_rx) = tokio::sync::mpsc::channel(8);
            let h = B256::repeat_byte(0x5E);
            let mut et = EpochTransition::new(
                MockReader {
                    committee: 3,
                    interval: 100,
                },
                sink.clone(),
                64,
                Some(boundary_tx),
                std::sync::Arc::new(move |_n| Ok(Some(h))),
                3,
            );
            assert_eq!(
                et.cold_start(h, 500).await.unwrap(),
                TransitionOutcome::EpochAdvanced(5)
            );

            let tracked = sink.0.lock().unwrap();
            let [(epoch, peers)] = tracked.as_slice() else {
                panic!("expected exactly one track call, got {tracked:?}");
            };
            assert_eq!(*epoch, 5);
            let reader = MockReader {
                committee: 3,
                interval: 100,
            };
            let primary = peers.primary();
            for member in reader.epoch_committee_snapshot(6, h).unwrap().validators {
                assert!(
                    primary.position(&member.keys.peer_pubkey).is_some(),
                    "committee[6] member {:?} missing from the epoch-5 peer set",
                    member.address
                );
            }
            assert_eq!(
                primary.len(),
                9,
                "committee[4] ∪ committee[5] ∪ committee[6], each of 3"
            );
            assert_eq!(
                peers.committees.iter().map(|(e, _)| *e).collect::<Vec<_>>(),
                vec![4, 5, 6],
                "the three records are carried separately, not flattened"
            );

            // The union is additive: the boundary trigger still carries the
            // current committee.
            let fired = boundary_rx.try_recv().expect("boundary trigger delivered");
            assert_eq!(fired.0, 5);
            assert_eq!(fired.1.validators.len(), 3);
        });
    }

    #[test]
    fn uncommitted_incoming_committee_skips_the_union_and_still_triggers() {
        // An empty read means "not committed yet", never "empty committee": skip
        // the union tier, keep the boundary.
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = RecordingSink::default();
            let (boundary_tx, mut boundary_rx) = tokio::sync::mpsc::channel(8);
            let requested = Arc::new(Mutex::new(vec![]));
            let h = B256::repeat_byte(0x5F);
            let mut et = EpochTransition::new(
                IncomingUnavailableReader {
                    inner: MockReader {
                        committee: 3,
                        interval: 100,
                    },
                    ok_through: 5,
                    fail: false,
                    requested: requested.clone(),
                },
                sink.clone(),
                64,
                Some(boundary_tx),
                std::sync::Arc::new(move |_n| Ok(Some(h))),
                3,
            );
            assert_eq!(
                et.cold_start(h, 500).await.unwrap(),
                TransitionOutcome::EpochAdvanced(5)
            );
            assert!(
                requested.lock().unwrap().contains(&6),
                "the union must have ASKED for committee[6] — else this proves nothing"
            );
            assert_eq!(
                *sink.0.lock().unwrap(),
                vec![(5, 6)],
                "committee[4] ∪ committee[5]; the empty incoming read adds nothing"
            );
            assert_eq!(et.last_tracked_epoch, Some(5));
            assert_eq!(
                boundary_rx
                    .try_recv()
                    .expect("boundary trigger delivered")
                    .0,
                5
            );
        });
    }

    /// A failed neighbour read replays the boundary instead of registering a short
    /// peer set and moving on: the epoch stays un-tracked, so the next finalized
    /// block re-reads it.
    #[test]
    fn a_failed_neighbour_committee_read_replays_the_boundary_instead_of_tracking() {
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = RecordingSink::default();
            let (boundary_tx, mut boundary_rx) = tokio::sync::mpsc::channel(8);
            let requested = Arc::new(Mutex::new(vec![]));
            let h = B256::repeat_byte(0x60);
            let mut et = EpochTransition::new(
                IncomingUnavailableReader {
                    inner: MockReader {
                        committee: 3,
                        interval: 100,
                    },
                    ok_through: 5,
                    fail: true,
                    requested: requested.clone(),
                },
                sink.clone(),
                64,
                Some(boundary_tx),
                std::sync::Arc::new(move |_n| Ok(Some(h))),
                3,
            );
            let err = et
                .cold_start(h, 500)
                .await
                .expect_err("a failed neighbour read must surface, not degrade");
            assert!(
                matches!(err, ReadError::Backend(_)),
                "the read's own error must reach the caller verbatim: {err:?}"
            );
            assert!(
                requested.lock().unwrap().contains(&6),
                "the union must have ASKED for committee[6] — else this proves nothing"
            );
            assert!(
                sink.0.lock().unwrap().is_empty(),
                "no peer set may be registered off a failed read: {:?}",
                sink.0.lock().unwrap()
            );
            assert_eq!(
                et.last_tracked_epoch, None,
                "the epoch stays un-tracked so the next finalized block re-reads it"
            );
            assert!(
                boundary_rx.try_recv().is_err(),
                "the boundary trigger must not fire off a set that was never tracked"
            );

            let mut healed = EpochTransition::new(
                IncomingUnavailableReader {
                    inner: MockReader {
                        committee: 3,
                        interval: 100,
                    },
                    ok_through: u64::MAX,
                    fail: true,
                    requested: requested.clone(),
                },
                sink.clone(),
                64,
                None,
                std::sync::Arc::new(move |_n| Ok(Some(h))),
                3,
            );
            assert_eq!(
                healed.cold_start(h, 500).await.unwrap(),
                TransitionOutcome::EpochAdvanced(5)
            );
            assert_eq!(
                *sink.0.lock().unwrap(),
                vec![(5, 9)],
                "the healed read registers committee[4] ∪ committee[5] ∪ committee[6]"
            );
        });
    }

    #[test]
    fn boundary_apply_persists_and_tracks_once() {
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = RecordingSink::default();
            let h = B256::repeat_byte(0x11);
            let mut et = et(
                MockReader {
                    committee: 5,
                    interval: 100,
                },
                sink.clone(),
                64,
                None,
                h,
            );
            let outcome_first = et.cold_start(h, 500).await.unwrap();
            assert_eq!(outcome_first, TransitionOutcome::EpochAdvanced(5));
            // 550 is mid-epoch, so a re-delivery is not a boundary.
            let outcome_second = et.on_finalized(550).await.unwrap();
            assert_eq!(outcome_second, TransitionOutcome::Intra);
            {
                let log = sink.0.lock().unwrap();
                assert_eq!(
                    *log,
                    vec![(5, 15)],
                    "tracked once for epoch 5: committee[4] ∪ committee[5] ∪ committee[6]"
                );
            }
        });
    }

    #[test]
    fn replayed_boundary_advance_is_surfaced_not_dropped() {
        // A boundary parked while execution lagged, then replayed on the next
        // intra-epoch delivery, must surface its `EpochAdvanced` rather than the new
        // delivery's `Intra`.
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = RecordingSink::default();
            let h = B256::repeat_byte(0x21);
            let resolve = Arc::new(std::sync::atomic::AtomicBool::new(true));
            let resolve_c = resolve.clone();
            let mut et = EpochTransition::new(
                MockReader {
                    committee: 5,
                    interval: 100,
                },
                sink.clone(),
                64,
                None,
                std::sync::Arc::new(move |_n| {
                    Ok(resolve_c
                        .load(std::sync::atomic::Ordering::Acquire)
                        .then_some(h))
                }),
                3,
            );
            assert_eq!(
                et.cold_start(h, 500).await.unwrap(),
                TransitionOutcome::EpochAdvanced(5)
            );
            // The last block of epoch 5 finalizes while no hash resolves.
            resolve.store(false, std::sync::atomic::Ordering::Release);
            assert_eq!(
                et.on_finalized(599).await.unwrap(),
                TransitionOutcome::Intra
            );
            assert!(et.has_pending_boundary(), "boundary parked");
            resolve.store(true, std::sync::atomic::Ordering::Release);
            assert_eq!(
                et.on_finalized(600).await.unwrap(),
                TransitionOutcome::EpochAdvanced(6),
                "the replayed boundary's advance must surface (dropped before bug 11 fix)"
            );
        });
    }

    #[test]
    fn last_block_of_epoch_spawns_next_epoch() {
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = RecordingSink::default();
            let h = B256::repeat_byte(0x22);
            let mut et = et(
                MockReader {
                    committee: 5,
                    interval: 100,
                },
                sink.clone(),
                64,
                None,
                h,
            );
            assert_eq!(
                et.cold_start(h, 550).await.unwrap(),
                TransitionOutcome::EpochAdvanced(5)
            );
            // 599 is the last block of epoch 5, so it spawns epoch 6.
            assert_eq!(
                et.on_finalized(599).await.unwrap(),
                TransitionOutcome::EpochAdvanced(6)
            );
            // 650 is mid-epoch 6 and already tracked, so it is a no-op.
            assert_eq!(
                et.on_finalized(650).await.unwrap(),
                TransitionOutcome::Intra
            );
            let log = sink.0.lock().unwrap();
            assert_eq!(
                *log,
                vec![(5, 15), (6, 15)],
                "bootstrap epoch 5, then spawn epoch 6 at its boundary"
            );
        });
    }

    #[test]
    fn cold_start_on_boundary_enters_next_epoch() {
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = RecordingSink::default();
            let h = B256::repeat_byte(0x55);
            let mut et = et(
                MockReader {
                    committee: 5,
                    interval: 100,
                },
                sink.clone(),
                64,
                None,
                h,
            );
            // A finalized boundary means the network is already in epoch 6;
            // entering 5 would make a catch-up node's hint a marshal no-op.
            assert_eq!(
                et.cold_start(h, 599).await.unwrap(),
                TransitionOutcome::EpochAdvanced(6),
            );
            assert_eq!(
                *sink.0.lock().unwrap(),
                vec![(6, 15)],
                "boundary cold-start tracks epoch 6"
            );
        });
    }

    #[test]
    fn oversize_committee_is_typed_error_not_panic() {
        deterministic::Runner::default().start(|_ctx| async move {
            let h = B256::repeat_byte(0x22);
            let mut et = et(
                MockReader {
                    committee: 10,
                    interval: 100,
                },
                RecordingSink::default(),
                // Below the tracked primary of three disjoint committees of 10.
                4,
                None,
                h,
            );
            assert!(matches!(
                et.cold_start(h, 200).await,
                Err(ReadError::PeerSetTooLarge {
                    epoch: 2,
                    size: 30,
                    max: 4
                })
            ));
        });
    }

    #[test]
    fn zero_interval_is_typed_error_not_panic() {
        deterministic::Runner::default().start(|_ctx| async move {
            let h = B256::repeat_byte(0x01);
            let mut et = et(
                MockReader {
                    committee: 3,
                    interval: 0,
                },
                RecordingSink::default(),
                64,
                None,
                h,
            );
            assert!(matches!(
                et.cold_start(h, 100).await,
                Err(ReadError::ZeroEpochInterval)
            ));
        });
    }

    #[test]
    fn missed_commit_epoch_skipped_not_tracked_empty() {
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = RecordingSink::default();
            let h = B256::repeat_byte(0x44);
            let mut et = et(
                MockReader {
                    committee: 0,
                    interval: 100,
                }, // no commit ⇒ empty
                sink.clone(),
                64,
                None,
                h,
            );
            // An epoch with no committee committed is a no-op, not an advance.
            let outcome = et.cold_start(h, 700).await.unwrap();
            assert_eq!(outcome, TransitionOutcome::Intra);
            assert!(
                sink.0.lock().unwrap().is_empty(),
                "no empty peer set tracked"
            );
            assert_eq!(et.last_tracked_epoch, None, "epoch NOT write-once-locked");
        });
    }

    #[test]
    fn try_send_full_returns_intra_and_does_not_advance() {
        // A full bridge channel leaves `last_tracked_epoch` un-advanced so the next
        // finalized block retries.
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = RecordingSink::default();
            let (boundary_tx, _boundary_rx) = tokio::sync::mpsc::channel(1);
            let dummy = ValidatorSetSnapshot {
                block_hash: B256::ZERO,
                block_number: 0,
                epoch: 999,
                validators: vec![],
                weights: None,
            };
            boundary_tx.try_send((999, dummy)).expect("first slot");
            let h = B256::repeat_byte(0xC6);
            let mut et = et(
                MockReader {
                    committee: 3,
                    interval: 100,
                },
                sink.clone(),
                64,
                Some(boundary_tx),
                h,
            );
            let outcome = et.cold_start(h, 500).await.unwrap();
            assert_eq!(
                outcome,
                TransitionOutcome::Intra,
                "Full bridge channel must surface as Intra outcome"
            );
            assert_eq!(
                et.last_tracked_epoch, None,
                "last_tracked_epoch must NOT advance"
            );
        });
    }

    #[test]
    fn boundary_full_channel_parks_and_recovers() {
        // A boundary whose send hits a full bridge channel stays parked until the
        // channel drains; clearing the park on any non-error would wedge the re-poke
        // loop.
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = RecordingSink::default();
            // Capacity 1, so the epoch-6 boundary send hits a full channel.
            let (boundary_tx, mut boundary_rx) = tokio::sync::mpsc::channel(1);
            let h = B256::repeat_byte(0xC7);
            let mut et = et(
                MockReader {
                    committee: 3,
                    interval: 100,
                },
                sink.clone(),
                64,
                Some(boundary_tx),
                h,
            );
            assert_eq!(
                et.cold_start(h, 500).await.unwrap(),
                TransitionOutcome::EpochAdvanced(5)
            );
            assert_eq!(
                et.on_finalized(599).await.unwrap(),
                TransitionOutcome::Intra,
                "Full bridge channel surfaces as Intra"
            );
            assert_eq!(
                et.last_tracked_epoch,
                Some(5),
                "epoch 6 must NOT advance while the channel is Full"
            );
            assert!(
                et.has_pending_boundary(),
                "boundary 599 must stay PARKED so the re-poke loop retries"
            );
            assert_eq!(boundary_rx.try_recv().expect("epoch 5 queued").0, 5);
            et.on_finalized(599).await.unwrap();
            assert_eq!(
                et.last_tracked_epoch,
                Some(6),
                "epoch 6 advances once the channel has room"
            );
            assert!(
                !et.has_pending_boundary(),
                "park released after the successful advance"
            );
            assert_eq!(boundary_rx.try_recv().expect("epoch 6 queued").0, 6);
        });
    }

    #[test]
    fn cold_start_branch_releases_a_stale_park() {
        // A park left while the bootstrap had not yet run must be released by the
        // cold-start branch, or the re-poke loop spins on a slot nothing clears.
        deterministic::Runner::default().start(|_ctx| async move {
            let h = B256::repeat_byte(0x77);
            let mut et = et(
                MockReader {
                    committee: 3,
                    interval: 100,
                },
                RecordingSink::default(),
                64,
                None,
                h,
            );
            et.pending_boundary = Some(599);
            assert_eq!(et.last_tracked_epoch, None);
            assert_eq!(
                et.cold_start(h, 500).await.unwrap(),
                TransitionOutcome::EpochAdvanced(5)
            );
            assert!(
                !et.has_pending_boundary(),
                "cold-start branch must release the stale park after advancing"
            );
        });
    }

    #[test]
    fn boundary_closed_channel_releases_park() {
        // A closed bridge (forwarder gone) is unrecoverable: unlike `Full`, the
        // boundary must not stay parked, or the re-poke loop spins during teardown.
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = RecordingSink::default();
            let (boundary_tx, mut boundary_rx) = tokio::sync::mpsc::channel(8);
            let h = B256::repeat_byte(0x78);
            let mut et = et(
                MockReader {
                    committee: 3,
                    interval: 100,
                },
                sink,
                64,
                Some(boundary_tx),
                h,
            );
            assert_eq!(
                et.cold_start(h, 500).await.unwrap(),
                TransitionOutcome::EpochAdvanced(5)
            );
            let _ = boundary_rx.try_recv();
            drop(boundary_rx);
            assert_eq!(
                et.on_finalized(599).await.unwrap(),
                TransitionOutcome::Intra
            );
            assert!(
                !et.has_pending_boundary(),
                "Closed channel is unrecoverable — must not park"
            );
            assert_eq!(
                et.last_tracked_epoch,
                Some(5),
                "Closed does not advance the epoch"
            );
        });
    }

    #[test]
    fn boundary_tx_fires_once_per_epoch() {
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = RecordingSink::default();
            let (boundary_tx, mut boundary_rx) = tokio::sync::mpsc::channel(8);
            let h = B256::repeat_byte(0xCD);
            let mut et = et(
                MockReader {
                    committee: 4,
                    interval: 100,
                },
                sink.clone(),
                64,
                Some(boundary_tx),
                h,
            );
            et.cold_start(h, 800).await.unwrap();
            et.on_finalized(850).await.unwrap();
            let first = boundary_rx.try_recv().expect("first boundary fires");
            assert_eq!(first.0, 8);
            assert_eq!(first.1.validators.len(), 4);
            assert!(boundary_rx.try_recv().is_err(), "no duplicate boundary");
        });
    }

    #[test]
    fn cold_start_reads_current_finalized_once() {
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = RecordingSink::default();
            let h = B256::repeat_byte(0x33);
            let mut et = et(
                MockReader {
                    committee: 3,
                    interval: 100,
                },
                sink.clone(),
                64,
                None,
                h,
            );
            et.cold_start(h, 1200).await.unwrap();
            assert_eq!(*sink.0.lock().unwrap(), vec![(12, 9)]);
        });
    }

    #[test]
    fn lagging_execution_defers_boundary_and_replays_it() {
        // A boundary arriving while execution lags is remembered; a later
        // non-boundary unresolved height must not clobber it, and the next
        // delivery replays it once execution catches up.
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = RecordingSink::default();
            let h = B256::repeat_byte(0x66);
            let resolvable = Arc::new(Mutex::new(true));
            let resolvable_for_et = resolvable.clone();
            let mut et = EpochTransition::new(
                MockReader {
                    committee: 5,
                    interval: 100,
                },
                sink.clone(),
                64,
                None,
                std::sync::Arc::new(move |_n| Ok(resolvable_for_et.lock().unwrap().then_some(h))),
                3,
            );
            assert_eq!(
                et.cold_start(h, 550).await.unwrap(),
                TransitionOutcome::EpochAdvanced(5)
            );

            *resolvable.lock().unwrap() = false;
            assert_eq!(
                et.on_finalized(599).await.unwrap(),
                TransitionOutcome::Intra,
                "boundary deferred while execution lags"
            );
            assert_eq!(
                et.on_finalized(600).await.unwrap(),
                TransitionOutcome::Intra,
                "non-boundary lag must not clobber the pending boundary"
            );

            *resolvable.lock().unwrap() = true;
            assert_eq!(
                et.on_finalized(601).await.unwrap(),
                TransitionOutcome::EpochAdvanced(6),
                "601 itself is intra, but the boundary fires via the replay — that \
                 advance is now surfaced, not dropped (bug 11)"
            );
            assert_eq!(
                *sink.0.lock().unwrap(),
                vec![(5, 15), (6, 15)],
                "epoch 6 entered via the pending-boundary replay"
            );
        });
    }

    #[test]
    fn cold_start_pre_activation_bootstraps_epoch_0_never_1() {
        // Cold start on a block before a scheduled activation bootstraps epoch 0: a
        // pre-activation block belongs to no relative epoch, so it is not a
        // boundary, and treating it as one would track a phantom committee[1].
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = RecordingSink::default();
            let (boundary_tx, mut boundary_rx) = tokio::sync::mpsc::channel(8);
            let h = B256::repeat_byte(0x3A);
            let mut et = EpochTransition::new(
                FutureActivationReader {
                    inner: MockReader {
                        committee: 3,
                        interval: 100,
                    },
                    activation: 1000,
                },
                sink.clone(),
                64,
                Some(boundary_tx),
                std::sync::Arc::new(move |_n| Ok(Some(h))),
                3,
            );
            assert_eq!(
                et.cold_start(h, 500).await.unwrap(),
                TransitionOutcome::EpochAdvanced(0),
                "pre-activation cold-start bootstraps epoch 0, never epoch 1"
            );
            assert_eq!(
                *sink.0.lock().unwrap(),
                vec![(0, 6)],
                "epoch 0 tracked (peer set = committee[0] ∪ the incoming committee[1]) \
                 — never a phantom TRACK of epoch 1"
            );
            assert_eq!(
                et.last_tracked_epoch,
                Some(0),
                "epoch 0 tracked; last_tracked_epoch never prematurely Some(1)"
            );
            let fired = boundary_rx
                .try_recv()
                .expect("epoch-0 boundary trigger delivered");
            assert_eq!(fired.0, 0, "the boundary trigger carries epoch 0");
            assert_eq!(fired.1.validators.len(), 3);
        });
    }

    #[test]
    fn cold_start_at_activation_minus_one_bootstraps_epoch_0() {
        // Block `activation - 1` has relative offset 0; it must classify as
        // pre-activation, not as a boundary, and bootstrap epoch 0.
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = RecordingSink::default();
            let h = B256::repeat_byte(0x3B);
            let mut et = EpochTransition::new(
                FutureActivationReader {
                    inner: MockReader {
                        committee: 3,
                        interval: 100,
                    },
                    activation: 1000,
                },
                sink.clone(),
                64,
                None,
                std::sync::Arc::new(move |_n| Ok(Some(h))),
                3,
            );
            assert_eq!(
                et.cold_start(h, 999).await.unwrap(),
                TransitionOutcome::EpochAdvanced(0),
                "block activation-1 is pre-activation ⇒ epoch 0, not epoch 1"
            );
            assert_eq!(et.last_tracked_epoch, Some(0));
            assert_eq!(*sink.0.lock().unwrap(), vec![(0, 6)]);
        });
    }

    // A header can exist without executed state while reth's pipeline backfills,
    // so `executed_hash` must answer `None` there and let the park fire instead of
    // driving a state read at an unexecuted hash.

    /// Encode a height into a B256 so the mocks below can recover the height
    /// behind an opaque `at` hash, the way reth keys state by hash.
    fn hash_at(n: u64) -> B256 {
        let mut b = [0u8; 32];
        b[24..].copy_from_slice(&n.to_be_bytes());
        B256::from(b)
    }
    fn height_from_hash(at: B256) -> u64 {
        u64::from_be_bytes(at.0[24..].try_into().unwrap())
    }

    /// Models reth's materialized head: a state read above `materialized` errors
    /// the way the erased `StateForHashNotFound` does, otherwise it delegates to
    /// `inner`. Reads are recorded so a test can prove a parked boundary never
    /// attempted one at an unexecuted hash.
    struct StateLagReader {
        inner: MockReader,
        materialized: Arc<Mutex<u64>>,
        reads: Arc<Mutex<Vec<u64>>>,
    }
    impl StateLagReader {
        fn gate(&self, at: B256) -> Result<(), ReadError> {
            let h = height_from_hash(at);
            self.reads.lock().unwrap().push(h);
            if h > *self.materialized.lock().unwrap() {
                return Err(ReadError::Backend(format!("no state found for block {at}")));
            }
            Ok(())
        }
    }
    impl StakingStateRead for StateLagReader {
        fn epoch_committee_snapshot(
            &self,
            epoch: u64,
            at: B256,
        ) -> Result<ValidatorSetSnapshot, ReadError> {
            self.gate(at)?;
            self.inner.epoch_committee_snapshot(epoch, at)
        }
        fn epoch_block_interval(&self, at: B256) -> Result<u64, ReadError> {
            self.gate(at)?;
            self.inner.epoch_block_interval(at)
        }
        fn dpos_activation_block(&self, at: B256) -> Result<u64, ReadError> {
            self.gate(at)?;
            self.inner.dpos_activation_block(at)
        }
        fn active_registry_peers(&self, at: B256) -> Result<Vec<PeerPubkey>, ReadError> {
            self.gate(at)?;
            self.inner.active_registry_peers(at)
        }
    }

    fn state_lag_mock() -> MockReader {
        MockReader {
            committee: 5,
            interval: 100,
        }
    }

    /// The state-gated closure contract: `Ok(None)` above `best`, `Ok(Some)` at or
    /// below it.
    fn state_gated_hash(
        best: Arc<Mutex<u64>>,
    ) -> std::sync::Arc<dyn Fn(u64) -> Result<Option<B256>, ReadError> + Send + Sync> {
        std::sync::Arc::new(move |read_h| {
            Ok((read_h <= *best.lock().unwrap()).then(|| hash_at(read_h)))
        })
    }

    /// A resolver that answers `Some` on header presence regardless of executed
    /// state — the behaviour the state gate exists to prevent.
    fn header_based_hash(
    ) -> std::sync::Arc<dyn Fn(u64) -> Result<Option<B256>, ReadError> + Send + Sync> {
        std::sync::Arc::new(|read_h| Ok(Some(hash_at(read_h))))
    }

    #[test]
    fn header_lead_state_lag_errors_and_would_shut_down() {
        // With the header-based resolver, a read height above the materialized head
        // drives `apply_at` at an unexecuted hash and errors instead of parking.
        deterministic::Runner::default().start(|_ctx| async move {
            let materialized = Arc::new(Mutex::new(500u64));
            let reads = Arc::new(Mutex::new(vec![]));
            let mut et = EpochTransition::new(
                StateLagReader {
                    inner: state_lag_mock(),
                    materialized: materialized.clone(),
                    reads,
                },
                RecordingSink::default(),
                64,
                None,
                header_based_hash(),
                3,
            );
            assert_eq!(
                et.cold_start(hash_at(500), 500).await.unwrap(),
                TransitionOutcome::EpochAdvanced(5),
                "anchor is executed by construction (materialized covers it)"
            );
            // Read heights 593..=596 all sit above the materialized head.
            for n in 596..=599 {
                assert!(
                    matches!(et.on_finalized(n).await, Err(ReadError::Backend(_))),
                    "header-lead state-lag at {n} errors — the counted fatal read"
                );
            }
            // Raising the head past the band makes the same call succeed: the error
            // was the un-materialized state, not a geometry slip.
            *materialized.lock().unwrap() = 600;
            assert!(matches!(
                et.on_finalized(596).await,
                Ok(TransitionOutcome::Intra)
            ));
        });
    }

    #[test]
    fn header_lead_state_lag_parks_not_shuts_down() {
        // The state-gated closure reports `Ok(None)` for the un-materialized band,
        // so deliveries park: no error, no committee read attempted, and the
        // boundary replays once the head catches up.
        deterministic::Runner::default().start(|_ctx| async move {
            let best = Arc::new(Mutex::new(500u64));
            let reads = Arc::new(Mutex::new(vec![]));
            let mut et = EpochTransition::new(
                StateLagReader {
                    inner: state_lag_mock(),
                    materialized: best.clone(),
                    reads: reads.clone(),
                },
                RecordingSink::default(),
                64,
                None,
                state_gated_hash(best.clone()),
                3,
            );
            et.cold_start(hash_at(500), 500).await.unwrap();
            reads.lock().unwrap().clear();

            for n in 596..=599 {
                assert_eq!(
                    et.on_finalized(n).await.unwrap(),
                    TransitionOutcome::Intra,
                    "un-materialized delivery parks, never errors"
                );
            }
            assert!(et.has_pending_boundary(), "boundary 599 parked");
            assert_eq!(et.pending_boundary(), Some(599));
            assert!(
                reads.lock().unwrap().is_empty(),
                "the committee read is DEFERRED — never attempted at an un-executed hash"
            );

            *best.lock().unwrap() = 600;
            assert_eq!(
                et.on_finalized(600).await.unwrap(),
                TransitionOutcome::EpochAdvanced(6),
                "the parked boundary replays once state materializes"
            );
            assert!(!et.has_pending_boundary(), "park cleared on heal");
        });
    }

    #[test]
    fn materialized_but_missing_state_is_still_a_real_error() {
        // A read that fails at a height the closure reports as materialized is a
        // real error, not a park: the two cases stay distinguishable.
        deterministic::Runner::default().start(|_ctx| async move {
            let closure_best = Arc::new(Mutex::new(700u64));
            let reader_materialized = Arc::new(Mutex::new(500u64));
            let mut et = EpochTransition::new(
                StateLagReader {
                    inner: state_lag_mock(),
                    materialized: reader_materialized,
                    reads: Arc::new(Mutex::new(vec![])),
                },
                RecordingSink::default(),
                64,
                None,
                state_gated_hash(closure_best),
                3,
            );
            et.cold_start(hash_at(500), 500).await.unwrap();
            assert!(
                matches!(et.on_finalized(599).await, Err(ReadError::Backend(_))),
                "a genuine fault at a claimed-materialized height stays a real error"
            );
            assert!(
                !et.has_pending_boundary(),
                "a real error is NOT silently parked"
            );
        });
    }

    #[test]
    fn parked_boundary_survives_flat_then_jump_backfill() {
        // The park has no internal give-up: it survives a long flat backfill and
        // heals on the single jump that catches the head up.
        deterministic::Runner::default().start(|_ctx| async move {
            let best = Arc::new(Mutex::new(500u64));
            let mut et = EpochTransition::new(
                StateLagReader {
                    inner: state_lag_mock(),
                    materialized: best.clone(),
                    reads: Arc::new(Mutex::new(vec![])),
                },
                RecordingSink::default(),
                64,
                None,
                state_gated_hash(best.clone()),
                3,
            );
            et.cold_start(hash_at(500), 500).await.unwrap();
            // Re-poke far past any fixed retry limit.
            for _ in 0..350 {
                assert_eq!(
                    et.on_finalized(599).await.unwrap(),
                    TransitionOutcome::Intra
                );
                assert_eq!(
                    et.pending_boundary(),
                    Some(599),
                    "the parked boundary is never abandoned during the flat backfill"
                );
            }
            *best.lock().unwrap() = 600; // single pipeline-completion jump
            assert_eq!(
                et.on_finalized(599).await.unwrap(),
                TransitionOutcome::EpochAdvanced(6),
                "heals on the jump"
            );
            assert!(!et.has_pending_boundary());
        });
    }

    #[test]
    fn no_committee_read_or_track_at_unexecuted_hash() {
        // While parked, nothing is read, tracked or advanced — the park only
        // defers; once state materializes the same committee is tracked once.
        deterministic::Runner::default().start(|_ctx| async move {
            let best = Arc::new(Mutex::new(500u64));
            let reads = Arc::new(Mutex::new(vec![]));
            let sink = RecordingSink::default();
            let mut et = EpochTransition::new(
                StateLagReader {
                    inner: state_lag_mock(),
                    materialized: best.clone(),
                    reads: reads.clone(),
                },
                sink.clone(),
                64,
                None,
                state_gated_hash(best.clone()),
                3,
            );
            et.cold_start(hash_at(500), 500).await.unwrap();
            let tracked_after_cold_start = sink.0.lock().unwrap().clone();
            reads.lock().unwrap().clear();

            for n in 596..=599 {
                assert_eq!(et.on_finalized(n).await.unwrap(), TransitionOutcome::Intra);
            }
            assert!(
                reads.lock().unwrap().is_empty(),
                "no state read attempted at an un-executed hash"
            );
            assert_eq!(
                *sink.0.lock().unwrap(),
                tracked_after_cold_start,
                "no committee tracked while parked"
            );

            *best.lock().unwrap() = 600;
            assert_eq!(
                et.on_finalized(600).await.unwrap(),
                TransitionOutcome::EpochAdvanced(6)
            );
            let epoch6_tracks = sink
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|(e, _)| *e == 6)
                .count();
            assert_eq!(
                epoch6_tracks, 1,
                "the deferred committee is tracked exactly once"
            );
        });
    }

    #[test]
    fn honest_nodes_with_equal_materialized_head_park_identically() {
        // The gate is a pure provider read with no wall-clock input, so two honest
        // nodes fed the same head sequence make identical park/advance decisions.
        async fn run(best_script: &[u64]) -> Vec<TransitionOutcome> {
            let best = Arc::new(Mutex::new(500u64));
            let mut et = EpochTransition::new(
                StateLagReader {
                    inner: state_lag_mock(),
                    materialized: best.clone(),
                    reads: Arc::new(Mutex::new(vec![])),
                },
                RecordingSink::default(),
                64,
                None,
                state_gated_hash(best.clone()),
                3,
            );
            et.cold_start(hash_at(500), 500).await.unwrap();
            let mut outcomes = vec![];
            for &b in best_script {
                *best.lock().unwrap() = b;
                outcomes.push(et.on_finalized(599).await.unwrap());
            }
            outcomes
        }
        deterministic::Runner::default().start(|_ctx| async move {
            // Flat below the read height for three deliveries, then a jump; both
            // nodes agree step for step.
            let script = [500u64, 500, 500, 600];
            let a = run(&script).await;
            let b = run(&script).await;
            assert_eq!(a, b, "honest nodes park/advance identically");
            assert_eq!(
                a,
                vec![
                    TransitionOutcome::Intra,
                    TransitionOutcome::Intra,
                    TransitionOutcome::Intra,
                    TransitionOutcome::EpochAdvanced(6),
                ]
            );
        });
    }

    // Re-jump landing clamp: the landing enters the terminal at or below itself,
    // usually the previous epoch's, up to `interval − 1` blocks down — far outside a
    // pruned node's retention window at the production interval. Without the clamp
    // every read at `boundary − K` hits pruned state and is retried forever, so the
    // landing epoch is never entered. The smoke suite runs at interval 64, whose
    // worst read is 66 blocks back — inside retention — so these tests are the only
    // guard for the class.

    /// Blocks a `--full` reth retains state for; not imported because this crate
    /// must not depend on reth.
    const RETENTION_WINDOW: u64 = 10_064;
    /// The production `epochBlockInterval`.
    const PROD_INTERVAL: u64 = 86_400;

    /// Models a pruned node: a read below the retention floor errors the way
    /// `StateAtBlockPruned` reaches this crate — untyped, so it is retried, never
    /// parked. Reads are recorded for the same reason as [`StateLagReader`].
    struct PrunedStateReader {
        inner: MockReader,
        retained_from: Arc<Mutex<u64>>,
        reads: Arc<Mutex<Vec<u64>>>,
    }
    impl PrunedStateReader {
        fn gate(&self, at: B256) -> Result<(), ReadError> {
            let h = height_from_hash(at);
            self.reads.lock().unwrap().push(h);
            if h < *self.retained_from.lock().unwrap() {
                return Err(ReadError::Backend(format!("state at block {at} is pruned")));
            }
            Ok(())
        }
    }
    impl StakingStateRead for PrunedStateReader {
        fn epoch_committee_snapshot(
            &self,
            epoch: u64,
            at: B256,
        ) -> Result<ValidatorSetSnapshot, ReadError> {
            self.gate(at)?;
            self.inner.epoch_committee_snapshot(epoch, at)
        }
        fn epoch_block_interval(&self, at: B256) -> Result<u64, ReadError> {
            self.gate(at)?;
            self.inner.epoch_block_interval(at)
        }
        fn dpos_activation_block(&self, at: B256) -> Result<u64, ReadError> {
            self.gate(at)?;
            self.inner.dpos_activation_block(at)
        }
        fn active_registry_peers(&self, at: B256) -> Result<Vec<PeerPubkey>, ReadError> {
            self.gate(at)?;
            self.inner.active_registry_peers(at)
        }
    }

    /// A pruned node at production geometry, staged as the executor stages a
    /// landing; without the clamp the entry reads ~50k blocks below the retention
    /// floor and every read fails.
    #[test]
    fn landing_entry_at_production_geometry_reads_inside_the_retention_window() {
        deterministic::Runner::default().start(|_ctx| async move {
            let best = Arc::new(Mutex::new(100_000u64));
            let retained_from = Arc::new(Mutex::new(0u64));
            let reads = Arc::new(Mutex::new(vec![]));
            let mut et = EpochTransition::new(
                PrunedStateReader {
                    inner: MockReader {
                        committee: 5,
                        interval: PROD_INTERVAL,
                    },
                    retained_from: retained_from.clone(),
                    reads: reads.clone(),
                },
                RecordingSink::default(),
                64,
                None,
                state_gated_hash(best.clone()),
                3,
            );
            assert_eq!(
                et.cold_start(hash_at(100_000), 100_000).await.unwrap(),
                TransitionOutcome::EpochAdvanced(1)
            );

            // A re-jump lands in epoch 11 while the EL retains only the last
            // RETENTION_WINDOW blocks.
            let landing = 1_000_000u64;
            let floor = landing - 3; // landing − K, the result-final point
            *best.lock().unwrap() = landing;
            *retained_from.lock().unwrap() = landing - RETENTION_WINDOW;
            et.raise_anchor_height(floor);

            // The terminal the executor enters: epoch 10's last block, far below the
            // retention floor.
            let boundary = 950_399u64;
            assert!(
                boundary - 3 < *retained_from.lock().unwrap(),
                "the unclamped read height must be pruned, else this proves nothing"
            );
            reads.lock().unwrap().clear();
            assert_eq!(
                et.on_finalized(boundary).await.unwrap(),
                TransitionOutcome::EpochAdvanced(11),
                "the landing epoch must be entered, not retried forever on pruned state"
            );
            let reads = reads.lock().unwrap();
            assert!(!reads.is_empty(), "the entry must actually have read state");
            assert!(
                reads.iter().all(|h| *h == floor),
                "every read must resolve at the clamped floor, got {reads:?}"
            );
        });
    }

    /// The clamp is monotone forward: a lower publication is ignored, so a stale or
    /// duplicate landing cannot re-open the pruned window an earlier one closed.
    #[test]
    fn raise_anchor_height_never_lowers_the_floor() {
        deterministic::Runner::default().start(|_ctx| async move {
            let h = B256::repeat_byte(0x77);
            let mut et = et(
                MockReader {
                    committee: 3,
                    interval: 100,
                },
                RecordingSink::default(),
                64,
                None,
                h,
            );
            et.cold_start(h, 500).await.unwrap();
            assert_eq!(et.anchor_height, Some(500), "cold start pins the anchor");
            et.raise_anchor_height(999_997);
            assert_eq!(et.anchor_height, Some(999_997));
            et.raise_anchor_height(400);
            assert_eq!(et.anchor_height, Some(999_997), "a lower value is ignored");
            et.raise_anchor_height(1_000_000);
            assert_eq!(et.anchor_height, Some(1_000_000), "a higher value wins");
        });
    }

    /// For a boundary at the tip, `number − K` is above the floor and still wins
    /// the `max`, so the read stays at the result-final height.
    #[test]
    fn delivered_boundary_still_reads_at_number_minus_k() {
        deterministic::Runner::default().start(|_ctx| async move {
            let best = Arc::new(Mutex::new(100_000u64));
            let retained_from = Arc::new(Mutex::new(0u64));
            let reads = Arc::new(Mutex::new(vec![]));
            let mut et = EpochTransition::new(
                PrunedStateReader {
                    inner: MockReader {
                        committee: 5,
                        interval: PROD_INTERVAL,
                    },
                    retained_from: retained_from.clone(),
                    reads: reads.clone(),
                },
                RecordingSink::default(),
                64,
                None,
                state_gated_hash(best.clone()),
                3,
            );
            et.cold_start(hash_at(100_000), 100_000).await.unwrap();
            let landing = 1_000_000u64;
            *retained_from.lock().unwrap() = landing - RETENTION_WINDOW;
            et.raise_anchor_height(landing - 3);

            // The next epoch terminal, delivered in order: the ordinary boundary
            // path.
            let boundary = 1_036_799u64;
            *best.lock().unwrap() = boundary;
            reads.lock().unwrap().clear();
            assert_eq!(
                et.on_finalized(boundary).await.unwrap(),
                TransitionOutcome::EpochAdvanced(12)
            );
            let reads = reads.lock().unwrap();
            assert!(!reads.is_empty());
            assert!(
                reads.iter().all(|h| *h == boundary - 3),
                "a near-tip delivery must still read at number − K, got {reads:?}"
            );
        });
    }

    /// The floor is monotone against both of its writers: a cold start arriving
    /// after a landing's raise must not drop it back into the pruned window.
    #[test]
    fn cold_start_after_a_raise_does_not_lower_the_floor() {
        deterministic::Runner::default().start(|_ctx| async move {
            let h = B256::repeat_byte(0x55);
            let mut et = et(
                MockReader {
                    committee: 3,
                    interval: 100,
                },
                RecordingSink::default(),
                64,
                None,
                h,
            );
            et.cold_start(h, 500).await.unwrap();
            assert_eq!(et.anchor_height, Some(500), "cold start pins the anchor");
            et.raise_anchor_height(999_997);
            et.cold_start(h, 400).await.unwrap();
            assert_eq!(
                et.anchor_height,
                Some(999_997),
                "a later cold start must not lower the floor a landing raised"
            );
            et.cold_start(h, 1_000_000).await.unwrap();
            assert_eq!(
                et.anchor_height,
                Some(1_000_000),
                "a cold start ABOVE the floor still moves it forward"
            );
        });
    }

    /// Boundary detection is pointwise, so a coalescing driver — newest height only —
    /// loses the epoch enter outright: no track, no trigger, and no park to replay,
    /// because the park remembers a boundary that was detected. This is why the
    /// per-block delivery hook owns the transition and a watch poller cannot.
    #[test]
    fn a_coalesced_driver_skips_the_boundary_a_stepping_one_enters() {
        deterministic::Runner::default().start(|_ctx| async move {
            let h = B256::repeat_byte(0x33);
            // Epoch 1 terminates at 199.
            let coalesced_sink = RecordingSink::default();
            let (coalesced_tx, mut coalesced_rx) = tokio::sync::mpsc::channel(64);
            let mut coalesced = et(
                MockReader {
                    committee: 3,
                    interval: 100,
                },
                coalesced_sink.clone(),
                64,
                Some(coalesced_tx),
                h,
            );
            coalesced.cold_start(h, 150).await.unwrap();
            // One coalesced delivery jumps over the terminal at 199.
            assert_eq!(
                coalesced.on_finalized(203).await.unwrap(),
                TransitionOutcome::Intra
            );

            let stepping_sink = RecordingSink::default();
            let (stepping_tx, mut stepping_rx) = tokio::sync::mpsc::channel(64);
            let mut stepping = et(
                MockReader {
                    committee: 3,
                    interval: 100,
                },
                stepping_sink.clone(),
                64,
                Some(stepping_tx),
                h,
            );
            stepping.cold_start(h, 150).await.unwrap();
            for number in 151..=203 {
                stepping.on_finalized(number).await.unwrap();
            }

            let epochs = |sink: &RecordingSink| -> Vec<u64> {
                sink.0.lock().unwrap().iter().map(|(e, _)| *e).collect()
            };
            let drain = |rx: &mut tokio::sync::mpsc::Receiver<(
                u64,
                crate::reader::ValidatorSetSnapshot,
            )>|
             -> Vec<u64> {
                let mut out = vec![];
                while let Ok((epoch, _)) = rx.try_recv() {
                    out.push(epoch);
                }
                out
            };

            assert_eq!(
                epochs(&coalesced_sink),
                vec![1],
                "the coalesced driver tracked only the cold-start epoch — 2 is lost"
            );
            assert_eq!(
                drain(&mut coalesced_rx),
                vec![1],
                "and the bridge saw no trigger for epoch 2, i.e. the engine never enters it"
            );
            assert_eq!(
                coalesced.last_tracked_epoch,
                Some(1),
                "the skipped boundary left the write-once guard where the cold start put it"
            );
            assert_eq!(
                coalesced.pending_boundary(),
                None,
                "and nothing is parked: the park replays a DETECTED boundary, it cannot \
                 find a skipped one"
            );

            assert_eq!(
                epochs(&stepping_sink),
                vec![1, 2],
                "the stepping driver lands on 199 and enters epoch 2"
            );
            assert_eq!(drain(&mut stepping_rx), vec![1, 2]);
            assert_eq!(stepping.last_tracked_epoch, Some(2));
        });
    }

    /// The plane poller needs the geometry and nothing else, so this must leave the
    /// bootstrap state untouched.
    #[test]
    fn freeze_geometry_freezes_the_geometry_and_nothing_else() {
        deterministic::Runner::default().start(|_ctx| async move {
            let h = B256::repeat_byte(0x71);
            let sink = RecordingSink::default();
            let (bridge_tx, mut bridge_rx) = tokio::sync::mpsc::channel(8);
            let mut et = et(
                MockReader {
                    committee: 3,
                    interval: 100,
                },
                sink.clone(),
                64,
                Some(bridge_tx),
                h,
            );
            assert_eq!(et.frozen_geometry(), None);
            assert!(
                et.freeze_geometry(h).unwrap(),
                "the first call reports that IT froze the geometry"
            );
            assert_eq!(et.frozen_geometry(), Some((0, 100)));
            assert_eq!(
                et.last_tracked_epoch, None,
                "the write-once bootstrap gate is untouched — the layer still owns it"
            );
            assert_eq!(
                et.anchor_height, None,
                "the read floor is untouched — the landing and the layer own it"
            );
            assert_eq!(et.pending_boundary(), None, "nothing is parked");
            assert!(sink.0.lock().unwrap().is_empty(), "no peer set was tracked");
            assert!(
                bridge_rx.try_recv().is_err(),
                "no epoch reached the bridge, so the epoch manager learned nothing"
            );

            assert!(
                !et.freeze_geometry(h).unwrap(),
                "a second call is a no-op and says so"
            );
            assert_eq!(et.frozen_geometry(), Some((0, 100)));
            assert_eq!(et.last_tracked_epoch, None);
            assert_eq!(et.anchor_height, None);
        });
    }

    /// Which height the one transition is bootstrapped from decides which epoch
    /// the engine ever enters, so there may be exactly one bootstrapper and it has
    /// to hold an ordering-scale anchor: off the plane's EL-finalized cursor, a
    /// bootstrap inside the K-wide window after a boundary picks `E − 1` while the
    /// ordering anchor picks `E`. The delivery hook then starts above the terminal
    /// that would have entered `E`, so that epoch is lost until the next boundary.
    #[test]
    fn an_el_scale_bootstrap_in_the_k_window_after_a_boundary_loses_the_epoch() {
        deterministic::Runner::default().start(|_ctx| async move {
            let h = B256::repeat_byte(0x2B);
            // Epoch 1 terminates at 199; the ordering anchor 201 sits in epoch 2,
            // while the EL cursor 201 − K is still in epoch 1.
            let el_sink = RecordingSink::default();
            let (el_tx, mut el_rx) = tokio::sync::mpsc::channel(64);
            let mut el_scale = et(
                MockReader {
                    committee: 3,
                    interval: 100,
                },
                el_sink.clone(),
                64,
                Some(el_tx),
                h,
            );
            el_scale.cold_start(h, 198).await.unwrap();

            let ordering_sink = RecordingSink::default();
            let (ordering_tx, mut ordering_rx) = tokio::sync::mpsc::channel(64);
            let mut ordering = et(
                MockReader {
                    committee: 3,
                    interval: 100,
                },
                ordering_sink.clone(),
                64,
                Some(ordering_tx),
                h,
            );
            ordering.cold_start(h, 201).await.unwrap();

            // The hook fires above the ordering anchor, so neither instance sees
            // terminal 199.
            for number in 202..=298 {
                el_scale.on_finalized(number).await.unwrap();
                ordering.on_finalized(number).await.unwrap();
            }

            let epochs = |sink: &RecordingSink| -> Vec<u64> {
                sink.0.lock().unwrap().iter().map(|(e, _)| *e).collect()
            };
            let drain = |rx: &mut tokio::sync::mpsc::Receiver<(
                u64,
                crate::reader::ValidatorSetSnapshot,
            )>|
             -> Vec<u64> {
                let mut out = vec![];
                while let Ok((epoch, _)) = rx.try_recv() {
                    out.push(epoch);
                }
                out
            };

            assert_eq!(
                epochs(&el_sink),
                vec![1],
                "the EL-scale bootstrap entered the PREVIOUS epoch"
            );
            assert_eq!(
                drain(&mut el_rx),
                vec![1],
                "and that is the only epoch the bridge — the one edge into the epoch \
                 manager — ever carried"
            );
            assert_eq!(el_scale.last_tracked_epoch, Some(1));
            assert_eq!(
                epochs(&ordering_sink),
                vec![2],
                "the ordering-scale bootstrap entered the epoch the node is actually in"
            );
            assert_eq!(drain(&mut ordering_rx), vec![2]);
            assert_eq!(ordering.last_tracked_epoch, Some(2));

            // The next boundary shows the loss is permanent, not a delay: the
            // write-once gate takes 3.
            assert_eq!(
                el_scale.on_finalized(299).await.unwrap(),
                TransitionOutcome::EpochAdvanced(3)
            );
            assert_eq!(
                epochs(&el_sink),
                vec![1, 3],
                "epoch 2 is lost forever — the walk resumes at 3"
            );
            assert_eq!(
                ordering.on_finalized(299).await.unwrap(),
                TransitionOutcome::EpochAdvanced(3)
            );
            assert_eq!(
                epochs(&ordering_sink),
                vec![2, 3],
                "the ordering-scale walk is contiguous"
            );
        });
    }

    /// The peer set must be registered before the layer's cold-start jump, and
    /// `track_peers` must not bootstrap — that branch belongs to the layer.
    #[test]
    fn track_peers_registers_the_peer_set_without_bootstrapping() {
        deterministic::Runner::default().start(|_ctx| async move {
            let h = B256::repeat_byte(0x4D);
            let reader = || RegistryReader {
                inner: MockReader {
                    committee: 3,
                    interval: 100,
                },
                registry: vec![
                    validator(900_001).keys.peer_pubkey,
                    validator(900_002).keys.peer_pubkey,
                ],
            };
            let sink = KeySink::default();
            let (bridge_tx, mut bridge_rx) = tokio::sync::mpsc::channel(8);
            let mut et = EpochTransition::new(
                reader(),
                sink.clone(),
                64,
                Some(bridge_tx),
                std::sync::Arc::new(move |_n| Ok(Some(h))),
                3,
            );

            // Before the freeze there is no epoch to name, and the call must not
            // freeze one.
            assert_eq!(
                et.track_peers(h, 250).await.unwrap(),
                None,
                "an unfrozen geometry has no epoch to track, and the call does not \
                 invent one"
            );
            assert_eq!(et.frozen_geometry(), None, "and it did not freeze anything");
            assert!(sink.0.lock().unwrap().is_empty(), "no peer set was tracked");

            assert!(et.freeze_geometry(h).unwrap());
            assert_eq!(
                et.track_peers(h, 250).await.unwrap(),
                Some(2),
                "height 250 over (activation 0, interval 100) is epoch 2"
            );

            // None of the bootstrap state moved, so the layer's cold start still
            // owns the starting epoch.
            assert_eq!(
                et.last_tracked_epoch, None,
                "the write-once bootstrap gate is untouched — the layer still owns it"
            );
            assert_eq!(
                et.anchor_height, None,
                "the read floor is untouched — the landing and the layer own it"
            );
            assert_eq!(et.pending_boundary(), None, "nothing is parked");
            assert!(
                bridge_rx.try_recv().is_err(),
                "no epoch reached the bridge, so the epoch manager learned nothing"
            );

            let outcome = et.cold_start(h, 250).await.unwrap();
            assert_eq!(
                outcome,
                TransitionOutcome::EpochAdvanced(2),
                "the layer's bootstrap runs afterwards exactly as if the plane had \
                 never touched the instance"
            );
            assert_eq!(et.last_tracked_epoch, Some(2));

            // One formula for both registrations, so the union cannot drift.
            let log = sink.0.lock().unwrap();
            assert_eq!(log.len(), 2, "one early track, one bootstrap track");
            assert_eq!(log[0].0, 2);
            assert_eq!(log[1].0, 2);
            assert_eq!(
                log[0].1, log[1].1,
                "the pre-jump track and the bootstrap track register the SAME set"
            );
            assert_eq!(
                log[0].1.primary().len(),
                9,
                "primary = committee[1] union committee[2] union committee[3], each of 3"
            );
            assert_eq!(
                log[0].1.secondary.len(),
                2,
                "the registry is tier 2 now, and is no longer part of primary"
            );
            drop(log);

            // On a boundary height the epoch is E+1, the same choice the bootstrap
            // branch makes, so the early track never registers a committee the
            // network has left.
            let boundary_sink = KeySink::default();
            let mut boundary_et = EpochTransition::new(
                reader(),
                boundary_sink.clone(),
                64,
                None,
                std::sync::Arc::new(move |_n| Ok(Some(h))),
                3,
            );
            assert!(boundary_et.freeze_geometry(h).unwrap());
            assert_eq!(
                boundary_et.track_peers(h, 199).await.unwrap(),
                Some(2),
                "199 terminates epoch 1, and a finalized terminal means the network \
                 is already in 2"
            );
            assert_eq!(boundary_et.last_tracked_epoch, None);
        });
    }
}
