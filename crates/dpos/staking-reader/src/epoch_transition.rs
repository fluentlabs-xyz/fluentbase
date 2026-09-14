//! Finality-gated epoch-boundary orchestrator (epoch_transition).
//!
//! Injection-style library: every collaborator is a constructor param, so
//! this compiles and unit-tests today without the consensus / p2p / node
//! layers (only their *instances* — the live finalized stream, the real
//! `Oracle`, the node wiring — are deferred).
//!
//! Design invariants:
//! - finality-gated apply;
//! - write-once `track` (no re-track of a covered epoch; no reorg handling
//!   — finalized ⇒ irreversible);
//! - committee-size pre-check (typed error, not a deep commonware panic);
//! - cold-start reads the *current* finalized committee once (no point
//!   taking an outdated state).
//!
//! The durable validator-set cache this module used to write is gone: it existed
//! to answer a committee the contract had pruned, and the contract no longer
//! prunes.
//!
//! Retry / outcome invariants:
//! - `last_tracked_epoch` advances only after `boundary_tx.try_send`
//!   succeeds — a `Full` channel leaves the epoch un-tracked so the next
//!   finalized block retries.
//! - `on_finalized` returns a [`TransitionOutcome`] so the caller's
//!   error counter resets only on `EpochAdvanced(_)`, not on intra-epoch
//!   no-ops.

use alloy_primitives::B256;
use commonware_utils::ordered::Set;
use core::future::Future;
use fluentbase_bls::PeerPubkey;

use crate::{
    error::ReadError,
    reader::{check_peer_set_size, epoch_at_block, is_epoch_boundary, StakingStateRead},
};

/// Freeze a governance-mutable geometry field on its first observation, then
/// treat it as fixed: returns the frozen value on every later call and warns
/// (log-only) if the on-chain value drifts. `what` names the field + the
/// consensus authority it backs (e.g. FixedEpocher / OriginEpocher) for the
/// diagnostic. Shared by the `epochBlockInterval` and `dposActivationBlock`
/// freezes in `apply_at`, which are otherwise identical bar the type.
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

/// Outcome of [`EpochTransition::on_finalized`] — distinguishes an
/// intra-epoch no-op from an actual epoch advance. The dpos.rs
/// boundary-hook closure uses this to decide whether to reset its
/// consecutive-error counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionOutcome {
    /// This block was an intra-epoch re-delivery of an already-tracked
    /// epoch, a still-empty missed-commit epoch, or a retry stalled
    /// on a full bridge channel. No epoch state was advanced.
    Intra,
    /// This block advanced `last_tracked_epoch` to the given value;
    /// the boundary trigger has been delivered to the consensus bridge.
    EpochAdvanced(u64),
}

/// Merge the replayed-boundary outcome with the new-delivery outcome (bug 11):
/// a new-delivery advance wins; otherwise a replay advance is surfaced (instead
/// of being debug-logged and dropped) so the engine hook's consecutive-error
/// counter resets on genuine epoch progress made via the replay path. The
/// single-slot invariant makes a double-advance in one call effectively
/// impossible, and `apply_at` is idempotent per epoch, so this adds no side effect.
fn merge_replay_outcome(
    replay_advance: Option<TransitionOutcome>,
    new: TransitionOutcome,
) -> TransitionOutcome {
    match new {
        TransitionOutcome::EpochAdvanced(_) => new,
        TransitionOutcome::Intra => replay_advance.unwrap_or(new),
    }
}

/// Internal result of `track_and_trigger`, distinguishing the two `Intra`
/// reasons the caller must treat differently for the pending-boundary slot:
/// `Full` is RETRYABLE (keep the boundary parked so the re-poke loop retries),
/// `Closed` is NOT (the forwarder has shut down — releasing the park avoids
/// spinning the re-poke loop against a dead channel during teardown).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TriggerResult {
    /// Boundary trigger delivered (or no bridge configured) — epoch advanced.
    Advanced,
    /// Bridge channel full — retry the send on the next poke.
    Full,
    /// Bridge channel closed (forwarder gone) — unrecoverable, do not retry.
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

/// The two tiers of one epoch's peer set, kept apart all the way to the
/// `Oracle` because commonware treats them differently and because the epoch a
/// peer owes its place to is what every channel's membership check asks for.
///
/// `primary` is not stored as a union: it is the per-epoch committee RECORDS
/// (`E−1`, `E`, `E+1`) the union is taken over, so the consumer of a frame can
/// ask "which of the three does this sender sit in" without a second read of
/// anything. [`Self::primary`] takes the union for commonware, which wants one
/// flat set; [`Self::epochs_of`] answers the membership question.
///
/// `secondary` is the Active validator REGISTRY: commonware never dials it, never
/// gossips bit-vecs about it and never caches its bodies, but does accept its
/// inbound connections and does serve it (`CW:.../tracker/record.rs:171`,
/// `:264`, `:341-348`; `CW:broadcast/src/buffered/engine.rs:319-322`) — which is
/// exactly the tier an ejected/upcoming validator belongs in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackedPeers {
    /// `(epoch, committee[epoch])` for each readable record among `E−1`, `E`,
    /// `E+1`, ascending. A record that is not readable (below the contract's
    /// window at cold start, or not committed yet) is ABSENT rather than empty —
    /// "no record" and "empty committee" are different answers and only the
    /// second one would be a fault.
    ///
    /// INVARIANT, held by every producer and relied on by every consumer: AT MOST
    /// THREE entries, with DISTINCT epochs. [`Self::assemble_tracked_peers`] gets
    /// it by construction (`E−1`, `E`, `E+1`, each pushed at most once); the
    /// ingress mask on the consuming side is a fixed three-slot array sized off it
    /// (`fluentbase_p2p::EpochMask`, which `debug_assert`s the bound rather than
    /// truncating in silence). The field is `pub` because two test call sites build
    /// a window by hand — a fourth record, or a repeated epoch, is a bug in the
    /// builder, not something a consumer is expected to cope with.
    pub committees: Vec<(u64, Set<PeerPubkey>)>,
    /// Tier 2: every Active registry entry at the anchor.
    pub secondary: Set<PeerPubkey>,
}

impl TrackedPeers {
    /// The flat primary set commonware tracks: the union of the carried records.
    pub fn primary(&self) -> Set<PeerPubkey> {
        Set::from_iter_dedup(
            self.committees
                .iter()
                .flat_map(|(_, members)| members.iter().cloned()),
        )
    }

    /// The epochs whose carried record contains `peer` — the membership mask a
    /// channel's ingress check reads. Empty ⇒ the peer is not primary here.
    pub fn epochs_of<'a>(&'a self, peer: &'a PeerPubkey) -> impl Iterator<Item = u64> + 'a {
        self.committees
            .iter()
            .filter(move |(_, members)| members.position(peer).is_some())
            .map(|(epoch, _)| *epoch)
    }
}

/// Where the assembled peer set is delivered. p2p-agnostic on purpose:
/// `staking-reader` does not depend on `commonware-p2p`. The real adapter
/// `impl PeerSetSink for commonware_p2p::Manager<PublicKey = PeerPubkey>`
/// (a one-liner `Manager::track(self, epoch, TrackedPeers::new(..)).await`) is
/// written at the `Oracle`-handle owner (the node wiring), where the
/// `oracle.track` call site lives. Style mirrors commonware's own traits
/// (`-> impl Future + Send`, not `async fn`, to stay clean under `-D warnings`).
pub trait PeerSetSink {
    fn track(&mut self, epoch: u64, peers: TrackedPeers) -> impl Future<Output = ()> + Send;
}

/// Drives finality-gated epoch boundaries: detect → frozen-committee
/// snapshot → size-check → persist (final) → `track` once → prune to the node's
/// own retention window.
///
/// Re-poke cadence for a parked boundary (see
/// [`EpochTransition::has_pending_boundary`]): callers retry `on_finalized`
/// with this backoff until the park clears.
pub const PENDING_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(200);

pub struct EpochTransition<R, S> {
    reader: R,
    sink: S,
    /// commonware `max_peer_set_size` (injected by the node; committee-size guard input).
    max_peer_set_size: usize,
    /// Write-once guard: the epoch already fed to `track`.
    last_tracked_epoch: Option<u64>,
    /// Optional boundary trigger for 04's `OuterEngine::boundary_sender`. When
    /// `Some`, every successful (non-skipped) epoch boundary fires
    /// `(epoch, snapshot)` exactly once. `try_send` is used (lossy) — if 04's
    /// receiver is closed, the trigger is silently dropped (04 has already
    /// shut down).
    boundary_tx: Option<tokio::sync::mpsc::Sender<(u64, crate::reader::ValidatorSetSnapshot)>>,
    /// `epochBlockInterval` frozen on the first finalized block. The consensus
    /// `FixedEpocher` is frozen at startup, so this MUST be treated as fixed
    /// after genesis — honoring a live governance change here would diverge the
    /// two epoch authorities. A later on-chain change is logged and ignored.
    /// (Correct boundary-synced live re-interval is a separate, deferred task.)
    frozen_interval: Option<u64>,
    /// `dposActivationBlock` frozen on the first finalized block — origin for
    /// the relative epoch numbering (consensus `OriginEpocher` is frozen at
    /// startup, so this is treated as fixed identically to the interval).
    frozen_activation: Option<u64>,
    /// Materialized-state-gated EVM hash by height — the deferred-execution
    /// re-key: committee reads resolve at `number − result_lag` (a result-final
    /// height) instead of the ordering-finalized block's own hash, which has no
    /// executed state yet. THREE-valued: `Ok(Some(hash))` = executed state
    /// materialized past the height; `Ok(None)` = height above reth's
    /// materialized head (`best_block_number()`) — a header may exist but the
    /// state is NOT yet materialized (pipeline backfill), so the caller PARKS;
    /// `Err` = a real read fault at a materialized height (header-index
    /// inconsistency / corruption) that MUST surface to the boundary hook's
    /// consecutive-error counter, never be folded into the park. Backed by
    /// `fluentbase_consensus::executed_state_hash` (by NUMBER, gated on
    /// `best_block_number()`, never the header tip `last_block_number`).
    executed_hash: std::sync::Arc<dyn Fn(u64) -> Result<Option<B256>, ReadError> + Send + Sync>,
    /// Result lag K (passed in — this crate must not depend on consensus).
    result_lag: u64,
    /// Cold-start anchor height; floor for the read-height clamp (heights at
    /// or below the anchor are executed by construction).
    anchor_height: Option<u64>,
    /// Boundary remembered while the executed tip lagged its read height.
    /// ONLY boundary heights are stored: a non-boundary apply is Intra by
    /// construction (nothing to replay), and an unconditional overwrite
    /// would clobber a remembered boundary with a non-boundary during a
    /// sustained execution lag — losing the epoch enter forever.
    pending_boundary: Option<u64>,
    /// The boundary height whose empty-committee park has already been WARNED,
    /// so the re-poke loop drops to `debug!` on every subsequent empty re-read
    /// (the first occurrence stays loud; a permanent-empty case does not spam
    /// one warn per finalized block). Overwritten with the new height when a
    /// DIFFERENT boundary parks empty, so each distinct boundary warns once.
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

    /// The relative epoch `number` falls in, over the FROZEN geometry — the same
    /// value [`Self::apply_at`] derives, so a consumer riding the boundary walk
    /// asks the epoch authority instead of re-deriving the formula from a
    /// `frozen_geometry()` pair. `None` until the geometry freezes.
    pub fn epoch_at(&self, number: u64) -> Option<u64> {
        epoch_at_block(number, self.frozen_activation?, self.frozen_interval?)
    }

    /// Activation-relative boundary predicate over the FROZEN geometry —
    /// usable without any state read once `cold_start` froze it.
    fn is_epoch_boundary_frozen(&self, number: u64) -> Option<bool> {
        Some(is_epoch_boundary(
            number,
            self.frozen_activation?,
            self.frozen_interval?,
        ))
    }

    /// Whether a boundary is parked awaiting execution catch-up. The replay
    /// fires on the next `on_finalized` call — callers MUST re-poke (retry
    /// with backoff) when this is set after their delivery was processed:
    /// during epoch catch-up the parked boundary IS the last deliverable
    /// block, so no further delivery will ever arrive to trigger the replay.
    pub fn has_pending_boundary(&self) -> bool {
        self.pending_boundary.is_some()
    }

    /// The parked boundary height, or `None` when nothing is parked. Drives the
    /// signer hook's `parked_boundary_height` gauge (external wedge detection:
    /// `!= 0 for > Xm` Prometheus alert) — the twin of the cert-budget executor
    /// park's `deferred_height`.
    pub fn pending_boundary(&self) -> Option<u64> {
        self.pending_boundary
    }

    /// The frozen `(dposActivationBlock, epochBlockInterval)` once a readable,
    /// DPoS-scheduled anchor has been resolved; `None` until then. This is the
    /// SINGLE in-plane source of the immutable epoch geometry: the beacon-plane
    /// poller drives [`Self::freeze_geometry`] here until it answers `Some`, and the
    /// `DkgActor` reads its activation/interval from the SAME resolution rather than
    /// re-reading the chain itself. `Some(_)` is also the poller's stop condition —
    /// and its PUBLISH condition, whichever caller did the freezing: the layer's
    /// cold start (which freezes through the same path) may get there first.
    pub fn frozen_geometry(&self) -> Option<(u64, u64)> {
        Some((self.frozen_activation?, self.frozen_interval?))
    }

    /// The executed height committee reads resolve at for an
    /// ordering-finalized `number`: `number − result_lag`, clamped to the
    /// cold-start anchor (≤ anchor is executed by construction).
    ///
    /// Hash-invariance now covers the WHOLE snapshot, so the lagged read point
    /// loses nothing: the committee array is frozen storage, consensus keys are
    /// one-shot, and — since 2026-07-31 — the per-member leader WEIGHT is frozen
    /// too, stamped into `leaderStakes[epoch]` at `commitEpochCommittee` from the
    /// selection epoch's stake. Until that landed the stakes leg was a LIVE
    /// at-or-before walk and this comment was false for it: a node reading at a
    /// different height (a cold start reads at its own anchor, not at
    /// `number − K`) could get a different weight vector, hence a different
    /// `total`, hence — since the draw is `rand % total` — a different leader
    /// entirely rather than a shifted band edge.
    fn read_height_for(&self, number: u64) -> u64 {
        let floor = self.anchor_height.unwrap_or(0);
        number.saturating_sub(self.result_lag).max(floor)
    }

    /// Raise the read-height floor — MONOTONE FORWARD, never lowers.
    ///
    /// Published by the executor when a steady-state re-jump lands: the jumped-over
    /// history is gone from this node (the marshal floor teleported past it, and a
    /// pruned reth keeps only a bounded state window), so "where this node's history
    /// begins" has moved and [`Self::read_height_for`] must clamp to the new point.
    /// Without it the boundary the landing enters — up to a full epoch below the tip,
    /// since the terminal at or below the landing is usually the PREVIOUS epoch's —
    /// still reads at `number − result_lag`, a height whose state is pruned: all five
    /// staticcalls in [`Self::apply_at`] fail, and the caller's retry arm re-reads the
    /// same dead height forever instead of entering the epoch.
    ///
    /// Monotone because the value states a fact that only moves forward; accepting a
    /// lower one would re-open the pruned window the raise just closed.
    ///
    /// The monotonicity is a property of the TYPE, not of the wiring. Both writers of
    /// `anchor_height` reach ONE instance: [`Self::cold_start`] (the layer, once, at
    /// the anchor its cold-start discriminator resolved) and this setter (the
    /// executor, on every later landing). Today's call order happens to be fixed —
    /// the layer cold-starts before the executor exists — but that is a fact about
    /// one wiring, and stating the guarantee as "the wiring has a single cold-start
    /// caller" is exactly how the previous version of this doc was left describing a
    /// wiring that had changed under it. So `cold_start` takes the same `max`: it is
    /// not a "the history starts here" assignment but the same forward-only fact
    /// stated from a different source, and an unconditional assignment in either
    /// writer would let it drop the floor back into the pruned window and defeat the
    /// guarantee this setter exists to give.
    pub fn raise_anchor_height(&mut self, height: u64) {
        self.anchor_height = Some(self.anchor_height.map_or(height, |a| a.max(height)));
    }

    /// Apply one **finalized** block `B` (delivered sequentially via
    /// commonware `Reporter Update::Block` + ack).
    ///
    /// Idempotent per epoch (write-once `track`): a re-delivery of the
    /// same epoch is a no-op, never a re-`track` (commonware would silently
    /// drop it anyway). Persist, track and prune are all individually
    /// idempotent (`prunable::Archive::put` skips duplicate indices;
    /// `sink.track` no-ops on a re-track), so a retry path stalled on
    /// a full bridge channel re-executes the upstream side effects safely.
    ///
    /// Returns [`TransitionOutcome`]:
    /// - `Intra` — intra-epoch re-delivery, missed-commit epoch, or a
    ///   retry path where `boundary_tx.try_send` failed; epoch state is
    ///   NOT advanced.
    /// - `EpochAdvanced(epoch)` — the bridge trigger was delivered and
    ///   `last_tracked_epoch` advanced to `epoch`.
    pub async fn on_finalized(&mut self, number: u64) -> Result<TransitionOutcome, ReadError> {
        if self.frozen_interval.is_none() {
            return Err(ReadError::Backend(
                "on_finalized before cold_start (epoch geometry not frozen)".into(),
            ));
        }
        // Replay FIRST: a boundary remembered while the executed tip lagged is
        // applied before the new delivery, keeping boundary handling in height
        // order. Why a single slot suffices is argued at the `debug_assert!` on
        // the park below (it is a property of which caller can park, NOT of the
        // epoch interval).
        // Bug 11: capture the replay outcome so a genuine epoch advance made via
        // the replay path is SURFACED to the caller, not just debug-logged. The
        // engine boundary hook resets its consecutive-error counter only on
        // `EpochAdvanced`, so a dropped replay advance could false-shutdown the
        // consensus thread at MAX_CONSECUTIVE_ON_FINALIZED_ERRORS despite progress.
        let mut replay_advance: Option<TransitionOutcome> = None;
        if let Some(b) = self.pending_boundary {
            // Three-valued probe: `Ok(None)` (b's read height still above the
            // materialized head) leaves the slot parked for the next re-poke;
            // an `Err` (a real read fault) propagates via `?` to the boundary
            // hook's counter arm — the slot untouched (still parked).
            if let Some(at) = (self.executed_hash)(self.read_height_for(b))? {
                // `apply_at` OWNS `pending_boundary`: it releases the slot on a
                // real advance (or an empty missed-commit epoch) and KEEPS it
                // parked when the bridge channel is Full (returns `Intra`
                // without advancing), so the re-poke loop retries the send. A
                // transient `ReadError` propagates via `?` with the slot
                // untouched (still parked) — `b` is the last deliverable block
                // during catch-up, so dropping it would wedge epoch E+1
                // forever. `apply_at` is idempotent per epoch, so re-applying
                // on the next retry is safe.
                let replay = self.apply_at(b, at).await?;
                tracing::debug!(boundary = b, ?replay, "replayed pending boundary");
                if matches!(replay, TransitionOutcome::EpochAdvanced(_)) {
                    replay_advance = Some(replay);
                }
            }
        }
        // Three-valued probe: `Err` (a real read fault at a materialized height)
        // propagates via `?` to the boundary hook's counter arm (fail-fast);
        // `Ok(None)` (height above the materialized head) PARKS; `Ok(Some)`
        // applies. This is the fix's core: a pipeline-backfill state-lag now
        // reports `Ok(None)` (park) instead of the header-based closure's stale
        // `Some(hash)` at an un-executed height (→ apply_at → `no state found`
        // → the 3-error self-shutdown).
        let Some(at) = (self.executed_hash)(self.read_height_for(number))? else {
            // Executed tip hasn't reached number − result_lag yet (transient:
            // bounded by the executor ack window OR, during a deep re-jump, the
            // reth PIPELINE backfill materializing state behind the header
            // frontier). Remember ONLY boundaries.
            if self.is_epoch_boundary_frozen(number) == Some(true) {
                // Single-slot invariant: a second boundary can be parked only by
                // clobbering the first, silently dropping its epoch handoff.
                //
                // What keeps that unreachable is WHICH CALLER can park, not the
                // epoch interval. `on_finalized` now has two producers — the
                // delivered-block adapter and the executor's re-jump landing —
                // and only the first can reach either park site. The landing
                // raises the read floor to `landing − result_lag` before it calls,
                // so its read resolves at exactly that floor — and the floor is
                // materialized by construction: both heights `sync_to` can return
                // are EXECUTED heights, `local_landing` reading `best_block_number`
                // (NOT the header-only `last_block_number`) and the loop exiting
                // only on `Valid{latest_valid_hash == tip}`, reth's own
                // canonical-and-executed verdict (`cold_start_jump.rs:414-419`,
                // `:505-509`). So `landing ≤ best`, hence `landing − result_lag ≤
                // best`: this `Ok(None)` arm ("read height above the materialized
                // head") cannot fire for it, a pruned read would be `Err` (which
                // parks nothing), and the empty-committee park below cannot
                // persist for an epoch whose commit landed long ago. That
                // leaves the delivery path as the only parker, and it delivers in
                // height order — one boundary at a time.
                //
                // `cold_start` is not a third producer either, and that is structural
                // rather than lucky: it is called once, by the layer, while
                // `last_tracked_epoch` is still `None` (the plane freezes the geometry
                // through `freeze_geometry`, which writes none of the bootstrap
                // state), so it takes the bootstrap branch — which CLEARS the slot and
                // has no park site — and never the boundary branch below, which does.
                //
                // The earlier justification here — `interval > MAX_PENDING_ACKS +
                // result_lag` — was an argument about tip-delivery timing that
                // never bound a boundary chosen an epoch below the tip. If this
                // ever fires, a THIRD producer has appeared; fail loud in
                // debug/tests rather than lose an epoch silently in release.
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

    /// Resolve the epoch geometry at `at` and FREEZE it — the whole geometry half of
    /// [`Self::apply_at`], and nothing else.
    ///
    /// `Ok(None)` = DPoS is not a scheduled, deployed chain at `at` yet: until then the
    /// ChainConfig staticcalls revert (codeless account) or read the `0` unscheduled
    /// sentinel. On a cold restart into `--dpos` the anchor can momentarily be the
    /// genesis fallback (reth has not yet surfaced its persisted finalized marker), so
    /// freezing there would FATALLY mis-read the geometry;
    /// `scheduled_dpos_activation` folds both the codeless and the `0` cases to `None`
    /// and the caller stays unfrozen and retries at a later height.
    fn resolve_and_freeze(&mut self, at: B256) -> Result<Option<(u64, u64)>, ReadError> {
        let Some(scheduled_activation) = self.reader.scheduled_dpos_activation(at)? else {
            return Ok(None);
        };
        // `epochBlockInterval` is treated as FIXED after genesis: the consensus
        // `FixedEpocher` is frozen at startup, so acting on a live governance
        // change here would diverge the two epoch authorities (a boundary-synced
        // live re-interval is a separate, deferred task). Freeze on the first
        // readable block; log + ignore any later on-chain change.
        let observed = self.reader.epoch_block_interval(at)?;
        if observed == 0 {
            return Err(ReadError::ZeroEpochInterval);
        }
        let interval = freeze_or_warn(
            &mut self.frozen_interval,
            observed,
            "epochBlockInterval (consensus FixedEpocher is frozen)",
        );
        // Freeze the relative-epoch origin alongside the interval (consensus
        // OriginEpocher is frozen at startup too). Reuse the value already resolved
        // by `scheduled_dpos_activation` — the `0`-fold never reaches here (it
        // returned `None` above), so this is the raw activation height (unscheduled
        // `0` is impossible past the gate).
        let activation = freeze_or_warn(
            &mut self.frozen_activation,
            scheduled_activation,
            "dposActivationBlock (consensus OriginEpocher is frozen)",
        );
        Ok(Some((activation, interval)))
    }

    /// Freeze the epoch geometry and DO NOTHING ELSE — the beacon plane's only
    /// business with this instance.
    ///
    /// `Ok(true)` = this call froze it; `Ok(false)` = it was already frozen (by this
    /// caller on an earlier tick, or by the layer's cold start) or DPoS is not
    /// scheduled at `at` yet, so the caller retries at a later height. Idempotent.
    ///
    /// It deliberately does NOT bootstrap the epoch, `track`, fire the bridge, raise
    /// the read floor or park a boundary: those are the BOOTSTRAP, and the bootstrap
    /// branch of [`Self::apply_at`] is write-once (`last_tracked_epoch.is_none()`), so
    /// a second caller reaching it would decide the starting epoch by winning a race.
    /// The plane's cursor is the EL-finalized height, `result_lag` BELOW the ordering
    /// chain and far below a re-jump landing, so the epoch it would pick is the wrong
    /// one (`an_el_scale_bootstrap_in_the_k_window_after_a_boundary_loses_the_epoch`).
    /// The one bootstrapper is [`Self::cold_start`], called by the layer once its
    /// ordering-scale anchor exists.
    pub fn freeze_geometry(&mut self, at: B256) -> Result<bool, ReadError> {
        if self.frozen_geometry().is_some() {
            return Ok(false);
        }
        Ok(self.resolve_and_freeze(at)?.is_some())
    }

    /// The pre-deferred `on_finalized` body: epoch geometry freeze +
    /// cold-start bootstrap (incl. boundary-resume E+1) + boundary branch,
    /// reading committee state at the RESOLVED executed hash `at`.
    async fn apply_at(&mut self, number: u64, at: B256) -> Result<TransitionOutcome, ReadError> {
        // Deferred bootstrap: on an anchor where DPoS is not a scheduled, deployed
        // chain yet, return a benign no-op and leave the geometry UNFROZEN — see
        // [`Self::resolve_and_freeze`] for why that state exists and how it clears.
        let Some((activation, interval)) = self.resolve_and_freeze(at)? else {
            return Ok(TransitionOutcome::Intra);
        };
        // The interval is non-zero (checked above), so the shared epoch function
        // cannot answer `None` here.
        let epoch_e =
            epoch_at_block(number, activation, interval).ok_or(ReadError::ZeroEpochInterval)?;

        // Boundary detection MUST be activation-relative, matching `epoch_at_block`
        // (reader.rs) and the consensus `OriginEpocher`: the last block of relative
        // epoch E is where `(number - activation) % interval == interval - 1`, i.e.
        // `(number + 1 - activation) % interval == 0`. The absolute form
        // `(number + 1) % interval == 0` only agrees when `activation % interval == 0`
        // (a devnet bootstrap convention, NOT enforced — prod cold-start anchors on
        // an arbitrary recent finalized height), so an absolute check would fire the
        // peer-set handoff at a different block than `OriginEpocher` treats as the
        // boundary — the exact "two epoch authorities diverge" failure the freeze
        // logic above guards against.
        let is_boundary = is_epoch_boundary(number, activation, interval);

        // Cold-start bootstrap: on the very first finalized block, stand up the
        // CURRENT epoch's engine. Its committee is already committed on-chain (the
        // ahead-commit pipeline committed it during the prior epoch), so read the
        // frozen array. `return` so a cold-start call never ALSO falls through to
        // the boundary branch below — otherwise an anchor on the last block of an
        // epoch whose `track_and_trigger` hit a Full channel (last_tracked stays
        // None → `None < Some(next)`) would double-spawn epoch E+1 while E was
        // never tracked.
        //
        // If the resume block IS an epoch boundary (last block of E), a finalized
        // boundary means the network has already advanced to E+1 — bootstrap E+1, not
        // E, so a catch-up node hints `last(E+1)` ABOVE the marshal floor (which sits
        // at this boundary). Entering E would hint `last(E) == floor` → a marshal
        // no-op → permanent boundary-resume deadlock. Mirrors tempo entering the next
        // epoch on a boundary-aligned resume; still a single `track_and_trigger` +
        // `return`, preserving the double-spawn guard.
        if self.last_tracked_epoch.is_none() {
            // Cold start owns its own retry: while `last_tracked_epoch` stays
            // None every delivery re-enters this branch and re-bootstraps, so it
            // never uses the pending-boundary slot. Release any park a prior
            // delivery left set (e.g. a boundary parked while the anchor epoch
            // was an empty missed-commit, replayed here) — otherwise it would
            // wedge the re-poke loop after the bootstrap finally advances.
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

        // Boundary: when the LAST block of epoch E finalizes, spawn epoch E+1. Its
        // committee is committed by now with room to spare: the node's
        // pre-execution stage drains `commitEpochCommittee()` on EVERY block
        // while `nextEpochToCommit() <= current_epoch + MAX_COMMITTEE_LOOKAHEAD_EPOCHS`
        // (`node/src/evm.rs:895-918`, called at `:1227-1231`), and the contract
        // reverts a target above that horizon
        // (`contracts/staking/src/consensus.rs:572-578`) — so the state at any
        // block of epoch `E − 1` already holds `committee[E + 1]`, and the read
        // below happens at `E`'s last block minus `result_lag`. The genesis block
        // for engine E+1 (= this finalized last-block of E) is stored. The
        // engine-E engine keeps producing until E+1 takes over.
        let next = epoch_e + 1;
        if is_boundary && self.last_tracked_epoch < Some(next) {
            // Missed-commit epoch: `Staking.sol` allows an epoch with no
            // `commitEpochCommittee` (unslashable by design; idempotent / monotonic
            // — a skip is safe); `getEpochCommittee` returns empty. Do NOT
            // persist/track an empty peer set — skip so a later finalized block can
            // still apply it if the commit lands, and commonware keeps the prior set.
            let snap = self.reader.epoch_committee_snapshot(next, at)?;
            if snap.validators.is_empty() {
                // committee[next] not yet readable at the deterministic spawn height
                // (`executed_hash(B−K)`). Under the 2-epoch committee warm-up this is a
                // TRANSIENT state-visibility lag, never a genuine missed commit: the
                // ahead-commit loop runs on EVERY block and commits immediately, with no
                // deferral and no qualify-before-commit branch (`node/src/evm.rs:940-946`
                // says so in as many words), so `committee[next]` was frozen a whole
                // epoch before this read and an empty answer here can only be reth's
                // eager-canonicalization state lag. KEEP the
                // boundary PARKED so the re-poke loop RE-READS on subsequent finalized
                // observations until the snapshot materializes; dropping to `None` here
                // would lose the epoch-E+1 engine spawn PERMANENTLY (the wedge amplifier
                // the K-invariant audit found).
                //
                // Warn ONCE per parked boundary (loud on first park), then `debug!` on
                // every re-poke while it stays empty — a genuinely permanent empty
                // (should be unreachable) must not emit one warn per finalized block.
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
            // KEEP the boundary parked ONLY when the send is RETRYABLE (`Full`):
            // this block is the last deliverable one during catch-up, so nothing
            // else re-detects the boundary — the re-poke loop must retry (the
            // same wedge the slot guards against for lagging execution). On a
            // real advance, or a `Closed` channel (forwarder gone — retrying a
            // dead channel only spins the loop during teardown), release it.
            self.pending_boundary = match result {
                TriggerResult::Full => Some(number),
                TriggerResult::Advanced | TriggerResult::Closed => None,
            };
            return Ok(result.into_outcome(next));
        }
        Ok(TransitionOutcome::Intra)
    }

    /// THE peer set for `epoch`, assembled once and in one place:
    /// primary = `committee[epoch − 1] ∪ committee[epoch] ∪ committee[epoch + 1]`,
    /// secondary = the Active validator REGISTRY at the anchor.
    ///
    /// The registry USED to be part of primary, which is what made a body buffer,
    /// a bit-vec and a resolver candidate list scale with the number of ACTIVATED
    /// validators instead of with the committee (R-013, R-037, E4-14). It buys
    /// nothing there: an ejected / upcoming / sequencer peer needs to reach the
    /// plane and be served, and commonware's secondary tier is exactly that — it
    /// connects inbound and is answered, but is never dialed, never bit-vec
    /// gossiped and never cached (`CW:.../tracker/record.rs:171`,
    /// `CW:broadcast/src/buffered/engine.rs:319-322`).
    ///
    /// The three committees are the three whose traffic is legitimate while
    /// `epoch` is the tracked one: `epoch + 1` because its epoch-key agreement
    /// instance runs DURING `epoch` and its `buffered` body engine retains a
    /// proposal body only from a sender in `latest.primary`; `epoch − 1` because
    /// the outgoing committee is still finalizing, still answering resolver
    /// fetches for its own rounds and still re-publishing evidence for them, and
    /// the committee can turn over completely at a boundary (zero overlap is
    /// legitimate) — dropping it from primary at the instant of the boundary is
    /// the same silent partition, one epoch earlier.
    ///
    /// A FUNCTION rather than two copies of the formula because it has two callers
    /// on two different clocks — [`Self::track_and_trigger`] at a boundary and
    /// [`Self::track_peers`] before the layer exists — and a peer set that differed
    /// between them would partition the plane in exactly the window where nothing
    /// is watching. The size guard rides along for the same reason: it is part of
    /// what "the tracked set" means, not of either caller. It checks PRIMARY only —
    /// commonware panics on an oversized primary set and does not check secondary
    /// at all, because the cap exists to bound the gossip bit-vec, which only
    /// covers primary (`CW:.../tracker/actor.rs:157-164`).
    ///
    /// The records come from the reader, not from the `committee/` module: this
    /// crate is BELOW consensus in the dependency graph (`crates/dpos/consensus`
    /// depends on `fluentbase-staking-reader`, not the reverse), so the module's
    /// type is not nameable here. Both read the same write-once committed slot, so
    /// the sets agree; making the module the single source is a Cargo-level move,
    /// not a code-level one.
    ///
    /// The two neighbour outcomes are NOT the same failure and are not treated
    /// alike.
    ///
    /// * `Ok` with no validators = "that epoch is not committed at this anchor"
    ///   (`reader.rs:639-641`), which is a legal chain state, not a fault: the
    ///   ahead-commit loop drains up to `current_epoch + MAX_COMMITTEE_LOOKAHEAD_EPOCHS`
    ///   (= 2, `crates/types/src/staking_protocol.rs:73`, `crates/node/src/evm.rs:902`)
    ///   on every block, so in steady state `C[E+1]` is committed a whole epoch
    ///   before this reads it and only the genesis-era epochs (`E ≤ 2`, before a
    ///   block of `E−1` has executed) can legitimately answer empty. The record is
    ///   then ABSENT from `committees` and the tier is skipped. Nothing downstream
    ///   loses by it: a member of the skipped record that sits in no other record
    ///   is refused at the channel's pre-decode gate (`GatedReceiver`, the one
    ///   sender classification on the beacon channel) until this node's next
    ///   `track` carries the record, and the dealer leg re-sends every pre-seal
    ///   tick. The seat the sender holds in the ceremony is the consumer's check
    ///   (`beacon::actor`, `no_seat`) over the committed record itself, through
    ///   `committee_for` — a reading that CAN disagree with this window for the
    ///   skipped epoch (the record may be readable by the time the consumer asks),
    ///   in the one direction that is safe: the gate is the stricter, and what it
    ///   refuses is re-sent.
    /// * `Err` = the read itself failed (backend, decode, an on-chain invariant
    ///   violation). That is NOT a legal state, and it is `?`. Both callers of this
    ///   function turn the error into a retry that re-reads the SAME boundary:
    ///   [`Self::track_and_trigger`] never reaches its `sink.track`, so
    ///   `last_tracked_epoch` does not advance and the re-poke loop calls
    ///   `on_finalized` again every `PENDING_RETRY_BACKOFF` forever
    ///   (`consensus/src/dpos.rs:2241`, `:2259-2277`); [`Self::track_peers`] leaves
    ///   the node's `peers_tracked` latch unset and retries on the next finalized
    ///   change (`node/src/dpos.rs:1630`, `:1645-1651`). Both re-entries are
    ///   idempotent, so the retry costs nothing.
    ///
    /// Degrading instead — which is what this did before — was NOT free once the
    /// registry left `primary`: `last_tracked_epoch` advanced on the degraded set,
    /// commonware ignores a second `track` for an index it already holds
    /// (`.claude/COMMONWARE_INTERNALS.md:363`), and no second source covers the
    /// missing record any more. One failed read would have cost the whole epoch its
    /// `C[E±1]` reachability with nothing above `warn` to say so.
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
        // typed, not panic
        check_peer_set_size(epoch, tracked.primary().len(), self.max_peer_set_size)?;
        Ok(tracked)
    }

    /// Read one neighbour committee into the primary records.
    ///
    /// An uncommitted epoch reads back empty and is SKIPPED (no record, rather
    /// than an empty one); a failed read is returned and replays the whole
    /// boundary. See [`Self::assemble_tracked_peers`] for why the two are not the
    /// same failure.
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

    /// Register the peer set for the epoch `number` falls in, and DO NOTHING ELSE —
    /// the beacon plane's second and last piece of business with this instance.
    ///
    /// `Ok(Some(epoch))` = that epoch's set went to the sink; `Ok(None)` = the
    /// geometry is not frozen yet, or `committee[epoch]` reads empty at `at`, so the
    /// caller retries at a later height. The epoch is chosen by the SAME rule the
    /// bootstrap branch of [`Self::apply_at`] uses (on a boundary height the network
    /// is already in `E + 1`), so an early registration never names the committee the
    /// chain has just left.
    ///
    /// WHY it exists, and why it is not the bootstrap: an empty-archive validator
    /// parks in `DposLayer::launch`'s cold-start jump loop
    /// (`consensus/src/dpos.rs:1845-1893`) until a PLANE peer serves it a frontier,
    /// and the frontier resolver only talks to peers the Oracle is tracking
    /// (`node/src/dpos.rs:1723-1730`). [`Self::cold_start`] — the one bootstrapper,
    /// and the only other path to a `track` — runs AFTER that loop
    /// (`consensus/src/dpos.rs:2108-2113`), so without this door the node would have
    /// no peers at the moment it needs them and would never leave the loop. It
    /// therefore touches NONE of the bootstrap state (`last_tracked_epoch`,
    /// `anchor_height`, `pending_boundary`, the bridge): the bootstrap branch is
    /// write-once and picking the starting epoch off the plane's EL-scale cursor is
    /// the defect `an_el_scale_bootstrap_in_the_k_window_after_a_boundary_loses_the_epoch`
    /// pins. The later bootstrap re-registering the same index is harmless — commonware
    /// ignores a `track` for an index already registered, and requires the index to
    /// grow (`.claude/COMMONWARE_INTERNALS.md:363`).
    ///
    /// TEMPORARY BRIDGE. The SET is now the 4.3 one —
    /// `TrackedPeers { primary: C[E−1] ∪ C[E] ∪ C[E+1], secondary: registry }`,
    /// assembled by [`Self::assemble_tracked_peers`] — but the two call sites are
    /// still two: this one and the `track` inside [`Self::track_and_trigger`].
    /// Design step 4.3 (`E4-CORE-DESIGN.md:534-548`) moves peer-set registration out
    /// of the epoch machine entirely; when that lands, both are replaced by that one
    /// call site and [`Self::assemble_tracked_peers`] goes with them.
    pub async fn track_peers(&mut self, at: B256, number: u64) -> Result<Option<u64>, ReadError> {
        // `None` until the geometry freezes — the plane's cursor must not be what
        // fixes it either, so there is no freeze attempt here.
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
            // A missed commit or a state-visibility lag: tracking an empty set would
            // REPLACE the peer set commonware holds, so skip and retry.
            return Ok(None);
        }
        let tracked = self.assemble_tracked_peers(epoch, &snap, at)?;
        self.sink.track(epoch, tracked).await;
        Ok(Some(epoch))
    }

    /// Persist + size-check + prune the frozen committee, feed the peer set to the
    /// sink, and fire the boundary trigger — advancing `last_tracked_epoch` only on
    /// a successful `try_send`. Extracted so both the cold-start bootstrap and the
    /// boundary branch share identical (idempotent) side effects.
    ///
    /// The set itself is [`Self::assemble_tracked_peers`]'s, shared verbatim with the
    /// plane's pre-engine [`Self::track_peers`].
    async fn track_and_trigger(
        &mut self,
        epoch: u64,
        snap: crate::reader::ValidatorSetSnapshot,
        at: B256,
    ) -> Result<TriggerResult, ReadError> {
        let tracked = self.assemble_tracked_peers(epoch, &snap, at)?;
        self.sink.track(epoch, tracked).await; // one-shot

        // Gate `last_tracked_epoch` advance on `try_send` success. A
        // `Full` channel means the consensus bridge is backed up; leave the
        // epoch un-tracked and signal RETRY so the next finalized block re-enters
        // here (persist/track/prune are idempotent — see contract above), retries
        // the send, and only advances `last_tracked_epoch` once consensus
        // actually saw the boundary trigger. A `Closed` channel means the
        // forwarder shut down (it fires the shutdown_token path itself, see
        // crates/node/src/dpos.rs bridge forwarder) — signal CLOSED so the caller
        // releases the park instead of spinning the re-poke loop against a dead
        // channel during teardown.
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

    /// Cold start: freeze the epoch geometry and read the **current
    /// finalized** committee at the EXPLICIT anchor hash `head` (the anchor
    /// is executed by construction — the one height where no `executed_hash`
    /// resolution is needed), apply once. Also raises the read-height floor for
    /// every later `on_finalized`. MUST run before `on_finalized`.
    ///
    /// THE bootstrapper: this is the only caller that may pick the starting epoch, and
    /// the layer is the only caller of it — on an ORDERING-scale, post-jump anchor.
    /// The beacon plane freezes the geometry through [`Self::freeze_geometry`] instead,
    /// which touches none of the bootstrap state, so `last_tracked_epoch` is still
    /// `None` when this runs and the bootstrap branch of [`Self::apply_at`] is the one
    /// it takes.
    ///
    /// The floor is RAISED, not assigned — see [`Self::raise_anchor_height`] for why
    /// monotonicity has to be a property of the type rather than of that call order.
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

    /// Canned reader: fixed committee size + interval.
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
            Ok(0) // mock tests use absolute numbering
        }
        fn active_registry_peers(&self, _at: B256) -> Result<Vec<PeerPubkey>, ReadError> {
            // Mock registry == nothing beyond the committee: the union fed to
            // the sink then equals the committee, keeping the existing
            // boundary-tracking assertions meaningful unchanged.
            Ok(vec![])
        }
    }

    /// Test ctor: a resolver that always resolves to `h` (mock chain where
    /// every height is executed), result_lag = 3.
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

    /// MockReader + a non-empty tier-2 registry: `active_registry_peers`
    /// returns peers DISJOINT from the committee, so the tracked union must
    /// be strictly larger than the committee.
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

    /// `MockReader` with a FUTURE `dposActivationBlock`. The `StakingStateRead`
    /// trait's `scheduled_dpos_activation` default folds over `dpos_activation_block`
    /// (`Ok(Some(dpos_activation_block(at)?))`), so overriding that one method is the
    /// whole parameterization — the mock's activation defaults to 0 (absolute
    /// numbering) via `MockReader` unchanged, and this single-method wrapper (mirror
    /// of `PrefixReader`/`RegistryReader`) expresses a scheduled future activation
    /// without touching any existing MockReader-literal test.
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

    /// A committee read above `ok_through` is UNAVAILABLE — either empty (the
    /// epoch is not committed yet) or a hard read failure. Those are the two ways
    /// `committee[epoch + 1]` can be missing when the peer-set union asks for it,
    /// and neither may cost the boundary trigger. Every requested epoch is
    /// recorded so a test can prove the union read was actually attempted.
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

    /// Records the full tracked SET, not just its size — the peer-set union's
    /// whole point is WHICH keys reach the agreement plane, and a size match can
    /// be satisfied by any three keys. Both tiers, because which tier a key lands
    /// in is the whole of 4.3.
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

    /// Primary is the THREE COMMITTEES and nothing else; the registry is tier 2.
    ///
    /// This is the whole of 4.3 A.1 in one assertion. Before it, an Active
    /// registry entry that sits in no committee was a primary peer — which is
    /// what made the `buffered` body cache, the discovery bit-vec and the
    /// resolver candidate list scale with the registry instead of with the
    /// committee (R-013, R-037, E4-14). It must now be secondary-only, and the
    /// outgoing committee `C[E−1]` must have JOINED primary.
    ///
    /// Falsifier: the registry-only key reappearing in `primary()`, or `C[E−1]`
    /// missing from it.
    #[test]
    fn the_registry_is_tier_two_and_the_outgoing_committee_is_tier_one() {
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = KeySink::default();
            let h = B256::repeat_byte(0x33);
            // 2 registry-only peers (seeds far from every committee's) + the three
            // committees of 3 — MockReader seeds per epoch, so C[1], C[2] and C[3]
            // are pairwise disjoint.
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
        // The epoch-key agreement instance for E+1 runs DURING E, and `buffered`
        // retains a body only from a sender inside the tracked set — so every
        // committee[E+1] member must already be there when E starts, or the plane
        // silently never converges.
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

            // The union is additive only — the boundary trigger still carries the
            // CURRENT committee, unchanged.
            let fired = boundary_rx.try_recv().expect("boundary trigger delivered");
            assert_eq!(fired.0, 5);
            assert_eq!(fired.1.validators.len(), 3);
        });
    }

    #[test]
    fn uncommitted_incoming_committee_skips_the_union_and_still_triggers() {
        // `committee[E+1]` not yet committed reads back EMPTY, which means "not
        // committed yet", never "the committee is empty" — skip the union, keep the
        // boundary.
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

    /// A FAILED neighbour read replays the boundary; it does not register a short
    /// peer set and move on.
    ///
    /// This is the difference between "not committed yet" (legal: skip the tier,
    /// `uncommitted_incoming_committee_skips_the_union_and_still_triggers` above)
    /// and "the read broke". Degrading on the second one used to be free, because
    /// the Active registry was ALSO primary and covered an incoming member
    /// incidentally. Since 4.3 the registry is tier 2 and `C[E±1]` has exactly one
    /// source, while `last_tracked_epoch` advances on the degraded set and
    /// commonware ignores a re-`track` of an index it already holds — so a single
    /// failed read would have cost the whole epoch its neighbour reachability, with
    /// nothing above a `warn` to say so.
    ///
    /// Falsifier: `track` being called at all, `last_tracked_epoch` advancing, or
    /// the boundary trigger firing — each of them is the old degrade-and-continue.
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

            // ...and the retry is what makes that safe: the same call against a
            // reader whose read now works registers the full three records and
            // fires the boundary, with no state left over from the failure.
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
            // block 500, interval 100 ⇒ epoch 5: cold_start bootstraps
            // the current epoch ⇒ EpochAdvanced(5)
            let outcome_first = et.cold_start(h, 500).await.unwrap();
            assert_eq!(outcome_first, TransitionOutcome::EpochAdvanced(5));
            // re-delivery on a MID-epoch block (550 is not the last block of epoch
            // 5, so it is not a boundary) ⇒ Intra. (599 would be the last block of
            // epoch 5 and now legitimately spawns epoch 6 — see the boundary test.)
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
        // Bug 11: a boundary parked while execution lagged, then replayed on the
        // next (intra-epoch) delivery, must SURFACE its `EpochAdvanced` rather
        // than be dropped in favour of the new delivery's `Intra` — else the
        // engine's consecutive-error counter never resets and false-shuts-down.
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = RecordingSink::default();
            let h = B256::repeat_byte(0x21);
            // Gate hash resolution so a boundary can be parked (unresolvable) then
            // replayed (resolvable) — the exact lag→catch-up sequence bug 11 needs.
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
            // Bootstrap epoch 5 (block 500, interval 100).
            assert_eq!(
                et.cold_start(h, 500).await.unwrap(),
                TransitionOutcome::EpochAdvanced(5)
            );
            // The last block of epoch 5 (599) finalizes while execution lags (no
            // resolvable hash) → it is PARKED, returns Intra.
            resolve.store(false, std::sync::atomic::Ordering::Release);
            assert_eq!(
                et.on_finalized(599).await.unwrap(),
                TransitionOutcome::Intra
            );
            assert!(et.has_pending_boundary(), "boundary parked");
            // Execution catches up; the next (intra-epoch) block 600 delivers.
            // Replaying the parked boundary advances to epoch 6 — THAT advance
            // must be the returned outcome even though block 600 itself is Intra.
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
            // cold-start mid-epoch-5 ⇒ bootstrap epoch 5
            assert_eq!(
                et.cold_start(h, 550).await.unwrap(),
                TransitionOutcome::EpochAdvanced(5)
            );
            // last block of epoch 5 ((599+1)%100==0) ⇒ spawn epoch 6 one ahead
            assert_eq!(
                et.on_finalized(599).await.unwrap(),
                TransitionOutcome::EpochAdvanced(6)
            );
            // mid-epoch-6 re-delivery ⇒ Intra (already tracked 6)
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
            // Cold-start EXACTLY on the epoch-5 boundary (599 = last block of epoch 5,
            // (599+1)%100==0). A finalized boundary means the network is in epoch 6 →
            // bootstrap epoch 6, NOT epoch 5: entering 5 would deadlock a catch-up node
            // (its hint last(5) == the marshal floor → a no-op).
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
                // Below the tracked primary: committee[1] ∪ committee[2] ∪
                // committee[3], each of 10 and pairwise disjoint.
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
            // epoch 7, empty ⇒ Intra (empty-committee is a no-op, not an advance)
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
        // When the bridge channel is full, on_finalized must leave
        // last_tracked_epoch un-advanced so the next finalized block retries.
        // Outcome must be `Intra` so the dpos.rs hook does NOT reset its
        // consecutive-error counter.
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = RecordingSink::default();
            // Capacity-1 channel; pre-fill it so try_send returns Full on
            // the next attempt without needing a real consumer.
            let (boundary_tx, _boundary_rx) = tokio::sync::mpsc::channel(1);
            // Pre-fill: take a fake (epoch, snap) slot.
            let dummy = ValidatorSetSnapshot {
                block_hash: B256::ZERO,
                block_number: 0,
                epoch: 999,
                validators: vec![],
                weights: None,
            };
            boundary_tx.try_send((999, dummy)).expect("first slot");
            // Now channel is full.
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
            let outcome = et.cold_start(h, 500).await.unwrap(); // epoch 5
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
        // A steady-state boundary whose `track_and_trigger` hits a Full bridge
        // channel must KEEP the boundary parked (so the re-poke loop retries the
        // send) and advance only once the channel drains — the wedge the
        // Err-only clear missed (a Full returns Ok(Intra), not Err).
        deterministic::Runner::default().start(|_ctx| async move {
            let sink = RecordingSink::default();
            // Capacity-1 channel: cold_start fills it with epoch 5, so the
            // epoch-6 boundary send then hits Full.
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
            // cold_start at 500 (mid-epoch 5) tracks epoch 5 → fills the 1 slot.
            assert_eq!(
                et.cold_start(h, 500).await.unwrap(),
                TransitionOutcome::EpochAdvanced(5)
            );
            // Boundary 599 (last block of epoch 5) → track epoch 6 → channel Full.
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
            // Drain the channel (consume the epoch-5 trigger), then re-poke.
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
        // A boundary parked while `last_tracked_epoch` was still None (anchor on
        // a missed-commit epoch) is replayed through the cold-start branch — which
        // must RELEASE the park once the bootstrap advances, else the re-poke loop
        // spins on a slot nothing will ever clear.
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
            // Pre-seed a park with last_tracked still None (the wedge precondition).
            et.pending_boundary = Some(599);
            assert_eq!(et.last_tracked_epoch, None);
            // cold_start at 500 (mid-epoch 5) bootstraps epoch 5 via the cold-start branch.
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
        // A `Closed` bridge (forwarder gone) is unrecoverable — unlike `Full`, the
        // boundary must NOT stay parked, or the re-poke loop spins against a dead
        // channel during teardown.
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
            // Drain epoch 5, then CLOSE the channel (drop the receiver).
            let _ = boundary_rx.try_recv();
            drop(boundary_rx);
            // Boundary 599 → epoch-6 send hits Closed → released, NOT parked.
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
        // Boundary at 599 arrives while the executed tip lags its read height →
        // remembered; a subsequent NON-boundary unresolved height must NOT
        // clobber it; once execution catches up, the next delivery replays the
        // boundary and epoch 6 enters.
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
        // Bug 3: cold-start on a block BEFORE a scheduled future activation must
        // bootstrap epoch 0 (a pre-activation block belongs to no relative epoch, so
        // it is not a boundary). The pre-fix `saturating_sub` underflow classified
        // every pre-activation block as a boundary → `cold_epoch = epoch_e + 1 = 1`,
        // tracking a phantom committee[1] on the sequencer→DPoS migration path.
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
            // block 500 < activation 1000 (pre-activation).
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
        // Bug 3 edge: block `activation - 1` (i.e. `number + 1 == activation`) has a
        // relative offset of 0 — the exact value the pre-fix underflow mapped to a
        // boundary. It must classify as pre-activation (not a boundary) and bootstrap
        // epoch 0.
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

    // ---- header-present / state-absent (pipeline-backfill) state-lag ----
    //
    // These pin the `dpos-onfinalized-state-lag-recover-stall` fix: during a
    // deep re-jump reth PIPELINE-backfills — HEADERS land far ahead of executed
    // STATE — so the OLD header-based `executed_hash` (`block_hash().ok().
    // flatten()`) resolved `Some(hash)` at an UN-EXECUTED height, bypassing the
    // Intra park and driving a committee state read at that hash → reth
    // `StateForHashNotFound` → `ReadError::Backend("no state found …")` → the
    // signer hook's 3-consecutive-error self-shutdown. The fix state-gates the
    // closure (`fluentbase_consensus::executed_state_hash`, `best_block_number()`
    // gate) so a not-yet-materialized height reports `Ok(None)` and the EXISTING
    // park fires instead.

    /// Encode a height into a B256 so the state-lag mock can recover the height
    /// behind an opaque `at` hash (reth keys state by hash; the mock keys its
    /// materialized-head gate by the height that hash stands for).
    fn hash_at(n: u64) -> B256 {
        let mut b = [0u8; 32];
        b[24..].copy_from_slice(&n.to_be_bytes());
        B256::from(b)
    }
    fn height_from_hash(at: B256) -> u64 {
        u64::from_be_bytes(at.0[24..].try_into().unwrap())
    }

    /// Models reth's MATERIALIZED head: a state read at a hash whose height is
    /// ABOVE `materialized` errors with reth's `no state found` (the exact
    /// `reader.rs:415` `.to_string()`-erased `StateForHashNotFound`), else
    /// delegates to `inner`. The default `StakingStateRead` mocks fold nothing,
    /// so the existing mocks CANNOT reproduce the state-absent error — this mock
    /// is what makes the fatal read reproducible at the unit level. Every state
    /// read is recorded, so a test can prove the park DEFERRED the read (never
    /// attempted it) at an un-executed hash — the fork-safety property.
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

    /// State-gated closure — the `fluentbase_consensus::executed_state_hash`
    /// contract modelled directly: `Ok(None)` above `best`, `Ok(Some(hash_at))`
    /// at/below it.
    fn state_gated_hash(
        best: Arc<Mutex<u64>>,
    ) -> std::sync::Arc<dyn Fn(u64) -> Result<Option<B256>, ReadError> + Send + Sync> {
        std::sync::Arc::new(move |read_h| {
            Ok((read_h <= *best.lock().unwrap()).then(|| hash_at(read_h)))
        })
    }

    /// The OLD header-based closure: resolves `Some` on header presence
    /// regardless of executed state — the bug's over-eager probe.
    fn header_based_hash(
    ) -> std::sync::Arc<dyn Fn(u64) -> Result<Option<B256>, ReadError> + Send + Sync> {
        std::sync::Arc::new(|read_h| Ok(Some(hash_at(read_h))))
    }

    #[test]
    fn header_lead_state_lag_errors_and_would_shut_down() {
        // RED characterization of the fatal path (the reason we state-gate): with
        // the OLD header-based closure, a band whose read height sits above the
        // materialized head drives apply_at at an un-executed hash and each
        // delivery returns `Err(ReadError::Backend)` — the exact error the signer
        // hook counts (`dpos.rs`: 3 consecutive ⇒ `MAX_CONSECUTIVE_ON_FINALIZED_
        // ERRORS` ⇒ `shutdown.cancel()`). The hook lives inside a reth-heavy fn
        // (not unit-isolable), so this pins the counted `Err`; the shutdown
        // mapping is cited, not re-simulated.
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
            // Band 596..=599: read heights 593..=596 all > materialized 500.
            for n in 596..=599 {
                assert!(
                    matches!(et.on_finalized(n).await, Err(ReadError::Backend(_))),
                    "header-lead state-lag at {n} errors — the counted fatal read"
                );
            }
            // Gate proof: raise the materialized head past the band and the SAME
            // call reads cleanly — the error was the un-materialized state, not a
            // geometry/mock slip.
            *materialized.lock().unwrap() = 600;
            assert!(matches!(
                et.on_finalized(596).await,
                Ok(TransitionOutcome::Intra)
            ));
        });
    }

    #[test]
    fn header_lead_state_lag_parks_not_shuts_down() {
        // GREEN inversion: the state-gated closure reports `Ok(None)` for the
        // un-materialized band, so the EXISTING Intra park fires — every delivery
        // is `Ok(Intra)` (the counter never ticks), the boundary is parked, and
        // the committee read is DEFERRED (the reader is never even called for the
        // band). Once the materialized head catches up, the next delivery replays
        // the boundary → `EpochAdvanced` (heals).
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
        // The fail-safe (2e): "not yet materialized (height > best → PARK)" and
        // "should be materialized but the read fails (height <= best, genuine
        // corruption/pruned) → REAL error → counter" are cleanly distinguished.
        // Here the CLOSURE reports materialized (best high ⇒ Ok(Some)) but the
        // reader errors the state read anyway — the error MUST surface, not park.
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
            // read_height 596 <= closure best 700 (Ok(Some)) but > reader
            // materialized 500 (state read fails) → the fault surfaces.
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
        // F1: the park has NO internal give-up. Model reth's PIPELINE backfill —
        // `best_block_number()` FLAT below the read height for far more than the
        // deleted `PENDING_RETRY_LIMIT` (300), then a SINGLE jump past it (reth's
        // one `on_backfill_sync_finished` advance). The re-poked boundary is
        // NEVER abandoned across the flat window and heals on the jump. (The hook
        // re-poke loop is not unit-isolable — cf. the header-lead test — so this
        // pins the ET park the loop re-pokes; give-up removal is verified in the
        // diff + end-to-end.)
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
            // Re-poke the delivered boundary far past the old fixed limit.
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
        // Fork-safety: while parked, NO committee/state read is attempted, NO
        // epoch is tracked, and NO `EpochAdvanced` is returned at an un-executed
        // hash — the park derives/tracks NOTHING; it only DEFERS. Then, once state
        // materializes, the SAME committee is tracked exactly once (deferred, not
        // skipped).
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
        // Determinism: the gate is a pure provider read with NO wall-clock, so two
        // honest nodes fed the SAME `best_block_number` sequence and the same
        // deliveries make IDENTICAL park/advance decisions — no non-deterministic
        // input feeds the consensus-relevant outcome.
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
            // Flat below the read height, then a jump — the two nodes must agree
            // step-for-step (park, park, park, advance).
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

    // ---- the read-height clamp at a re-jump landing ----
    //
    // A landing enters the terminal at or below itself, which — unless the landing
    // IS a terminal — is the PREVIOUS epoch's, up to `interval − 1` blocks down.
    // At the production interval (86_400) that is far outside a pruned node's
    // retention window (`--full` keeps 10_064 blocks), so every read `apply_at`
    // makes at `boundary − K` hits pruned state. That surfaces as an untyped
    // `ReadError::Backend`, and the boundary hook's error arm retries the same dead
    // height forever — the landing epoch is never entered. Smoke runs at interval
    // 64, where the read is at most 66 blocks back and always retained, so these
    // tests are the only guard for the class.

    /// Number of blocks a `--full` reth retains state for. Not imported (this crate
    /// must not depend on reth); the value only has to be realistic for the geometry.
    const RETENTION_WINDOW: u64 = 10_064;
    /// The production `epochBlockInterval` (`l2.json` mainnet/testnet).
    const PROD_INTERVAL: u64 = 86_400;

    /// Models a PRUNED node: a state read at a hash whose height is BELOW the
    /// retention floor errors the way reth's `StateAtBlockPruned` reaches this crate
    /// — an untyped `ReadError::Backend`, which is NOT in the transient taxonomy and
    /// so is retried, never parked. Twin of [`StateLagReader`], which gates the other
    /// end of the window (heights ABOVE the materialized head).
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

    /// A pruned node at production geometry, staged the way the executor stages a
    /// landing: `best` and the retention floor jump to the landing, the executor
    /// publishes `landing − K` as the read floor, then it drives the entry. Without
    /// the clamp the entry reads at `boundary − K`, ~50k blocks below the retention
    /// floor, and every read fails.
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
            // Cold start in epoch 1 while nothing is pruned yet.
            assert_eq!(
                et.cold_start(hash_at(100_000), 100_000).await.unwrap(),
                TransitionOutcome::EpochAdvanced(1)
            );

            // The node stalls for ~10 epochs; a re-jump lands it at 1_000_000, which
            // sits in epoch 11 (11 × 86_400 = 950_400). The EL now holds state only
            // for the last RETENTION_WINDOW blocks.
            let landing = 1_000_000u64;
            let floor = landing - 3; // landing − K, the result-final point
            *best.lock().unwrap() = landing;
            *retained_from.lock().unwrap() = landing - RETENTION_WINDOW;
            et.raise_anchor_height(floor);

            // The entry the executor drives: `terminal_at_or_below(landing)` is the
            // last block of epoch 10, ~49_600 blocks below the landing and far below
            // the retention floor.
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

    /// The clamp changes NOTHING for the delivery path: for a boundary at the tip,
    /// `number − K` is above the floor and still wins the `max`, so the committee
    /// read stays at the result-final height it has always used.
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

            // The chain runs on to the next epoch terminal (last block of epoch 11)
            // and the marshal delivers it in order — the ordinary boundary path.
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

    /// The floor is monotone against BOTH of its writers, not just
    /// [`EpochTransition::raise_anchor_height`]. The ONE transition a validator runs is
    /// cold-started by the beacon-plane poller (off the EL-finalized cursor, which can
    /// sit far below a re-jump landing) and raised by the executor's landing, so a cold
    /// start arriving after a raise must not drop the floor back into the window the
    /// landing closed — the property the old doc could only claim by pointing at a
    /// wiring that had exactly one cold-start caller.
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

    /// Boundary detection is POINTWISE — `is_epoch_boundary(number)` is true only for
    /// the terminal height of an epoch — so a driver that COALESCES (takes the newest
    /// height and drops the ones in between) loses the epoch enter outright: no track,
    /// no bridge trigger, and not even a parked boundary to replay, because the park
    /// remembers an already-detected boundary and never finds a skipped one. A driver
    /// that steps every finalized height enters it. This is what decides which driver
    /// may own the single transition: the per-block delivery hook, never a watch poller.
    #[test]
    fn a_coalesced_driver_skips_the_boundary_a_stepping_one_enters() {
        deterministic::Runner::default().start(|_ctx| async move {
            let h = B256::repeat_byte(0x33);
            // interval 100, activation 0 ⇒ epoch 1 terminates at 199.
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
            // One coalesced delivery from mid-epoch-1 to mid-epoch-2, over 199.
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

    /// The plane poller needs the GEOMETRY and nothing else, so the entry point it
    /// calls must freeze exactly that: no epoch bootstrap, no `track`, no bridge
    /// trigger, no read floor, no park. Those belong to the ONE bootstrapper (the
    /// layer's cold start, on an ordering-scale anchor); a poller that reached them
    /// would be the second one, and the bootstrap branch is write-once.
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

    /// WHICH HEIGHT the one transition is bootstrapped from decides which epoch the
    /// engine ever enters, so there may be exactly ONE bootstrapper and it has to be
    /// the layer's — the only caller holding an ORDERING-scale anchor.
    ///
    /// The bootstrap branch is write-once (`last_tracked_epoch.is_none()`) and the
    /// bridge is the only edge by which the epoch manager learns a new epoch. The
    /// beacon plane's cursor is the EL-finalized height, `result_lag` BELOW the
    /// ordering chain, so a bootstrap taken there inside the K-wide window after a
    /// boundary picks `E − 1` while the layer's anchor picks `E`. The delivery hook
    /// then starts at `anchor + 1`, ABOVE the terminal that would have entered `E` —
    /// so the EL-scale bootstrap does not merely delay `E`, it loses it until the
    /// NEXT boundary.
    #[test]
    fn an_el_scale_bootstrap_in_the_k_window_after_a_boundary_loses_the_epoch() {
        deterministic::Runner::default().start(|_ctx| async move {
            let h = B256::repeat_byte(0x2B);
            // interval 100, activation 0 ⇒ epoch 1 terminates at 199; the ordering
            // anchor 201 sits in epoch 2, and the EL cursor is 201 − K(3) = 198,
            // still in epoch 1.
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

            // The delivery hook fires from the block ABOVE the ordering anchor, so
            // the terminal 199 is never delivered to either instance.
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

            // The next boundary proves the loss is PERMANENT, not a delay: the
            // write-once gate `last_tracked_epoch < Some(next)` happily takes 3.
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

    /// The peer set has to be registered BEFORE the layer's cold-start jump, not
    /// after it: an empty-archive validator parks in
    /// `DposLayer::launch`'s jump loop (`consensus/src/dpos.rs:1845-1893`) until a
    /// PLANE peer serves it a frontier, and the frontier resolver only talks to
    /// peers the Oracle is tracking (`node/src/dpos.rs:1723-1730`). The layer's
    /// `cold_start` — the one bootstrapper — runs AFTER that loop
    /// (`consensus/src/dpos.rs:2108-2113`), so the only `track` reachable from it
    /// comes too late. `track_peers` is the beacon plane's door to the peer set,
    /// and it must open it WITHOUT bootstrapping: the bootstrap branch is
    /// write-once and belongs to the layer.
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

            // Before the freeze there is no epoch to name, so the call is a no-op
            // the caller can retry — NOT a freeze of its own (the plane's cursor
            // must never be what fixes the geometry's read height either).
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

            // The whole point of a separate door: none of the bootstrap state moves,
            // so the layer's cold start still takes the write-once branch and still
            // picks the starting epoch off its own ordering anchor.
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

            // ONE formula, not two: the set the plane registered early and the set
            // the bootstrap registers are the same object, so a later change to the
            // union cannot drift the two apart. (The repeat is harmless at the
            // Oracle: a `track` of an index already registered is ignored,
            // `.claude/COMMONWARE_INTERNALS.md:363`.)
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

            // On a boundary height the epoch is E+1 — the same choice the bootstrap
            // branch makes, so the early track never registers the committee the
            // network has already left.
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
