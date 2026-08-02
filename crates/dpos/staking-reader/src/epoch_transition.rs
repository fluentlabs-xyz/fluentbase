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
//! - the frozen-committee snapshot is what is persisted;
//! - committee-size pre-check (typed error, not a deep commonware panic);
//! - cold-start reads the *current* finalized committee once (no point
//!   taking an outdated state);
//! - retention mirrors the contract's own `_pruneStaleCommittees`
//!   (`undelegatePeriod + EPOCH_COMMITTEE_RETENTION_MARGIN`).
//!
//! Retry / outcome invariants:
//! - `last_tracked_epoch` advances only after `boundary_tx.try_send`
//!   succeeds — a `Full` channel leaves the epoch un-tracked so the next
//!   finalized block retries.
//! - `on_finalized` returns a [`TransitionOutcome`] so the caller's
//!   error counter resets only on `EpochAdvanced(_)`, not on intra-epoch
//!   no-ops.

use alloy_primitives::B256;
use commonware_runtime::{BufferPooler, Metrics, Storage};
use commonware_utils::ordered::Set;
use core::future::Future;
use fluentbase_bls::PeerPubkey;

use crate::{
    cache::ValidatorSetCache,
    error::ReadError,
    reader::{
        check_peer_set_size, epoch_of_block, is_epoch_boundary, StakingStateRead,
        EPOCH_COMMITTEE_RETENTION_MARGIN,
    },
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

/// Where the assembled peer set is delivered. p2p-agnostic on purpose:
/// `staking-reader` does not depend on `commonware-p2p`. The real adapter
/// `impl PeerSetSink for commonware_p2p::Manager<PublicKey = PeerPubkey>`
/// (a one-liner `Manager::track(self, epoch, set).await`) is written at the
/// `Oracle`-handle owner (the node wiring), where the `oracle.track` call
/// site lives. Style mirrors commonware's own traits (`-> impl Future + Send`,
/// not `async fn`, to stay clean under `-D warnings`).
pub trait PeerSetSink {
    fn track(&mut self, epoch: u64, peers: Set<PeerPubkey>) -> impl Future<Output = ()> + Send;
}

/// Drives finality-gated epoch boundaries: detect → frozen-committee
/// snapshot → size-check → persist (final) → `track` once → prune to the
/// contract's retention window.
///
/// Cache is held behind `Arc<tokio::sync::Mutex<_>>` so the slasher
/// can take a read lock from a separate task to fall back to historical
/// committees when the on-chain prune cursor has advanced past evidence
/// epoch (`get_by_epoch`). Only ET writes; the slasher only reads.
/// Re-poke cadence for a parked boundary (see
/// [`EpochTransition::has_pending_boundary`]): callers retry `on_finalized`
/// with this backoff until the park clears.
pub const PENDING_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(200);

pub struct EpochTransition<R, S, E: Storage + Metrics + BufferPooler> {
    reader: R,
    cache: std::sync::Arc<tokio::sync::Mutex<ValidatorSetCache<E>>>,
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
    frozen_interval: Option<u32>,
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

impl<R, S, E> EpochTransition<R, S, E>
where
    R: StakingStateRead,
    S: PeerSetSink,
    E: Storage + Metrics + BufferPooler,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        reader: R,
        cache: std::sync::Arc<tokio::sync::Mutex<ValidatorSetCache<E>>>,
        sink: S,
        max_peer_set_size: usize,
        boundary_tx: Option<tokio::sync::mpsc::Sender<(u64, crate::reader::ValidatorSetSnapshot)>>,
        executed_hash: std::sync::Arc<
            dyn Fn(u64) -> Result<Option<B256>, ReadError> + Send + Sync,
        >,
        result_lag: u64,
    ) -> Self {
        Self {
            reader,
            cache,
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

    /// Activation-relative boundary predicate over the FROZEN geometry —
    /// usable without any state read once `cold_start` froze it.
    fn is_epoch_boundary_frozen(&self, number: u64) -> Option<bool> {
        Some(is_epoch_boundary(
            number,
            self.frozen_interval?,
            self.frozen_activation?,
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
    /// DPoS-scheduled anchor has been applied; `None` until then. This is the
    /// SINGLE in-plane source of the immutable epoch geometry: the beacon-plane
    /// poller drives `cold_start`/`on_finalized` here (which freezes the geometry,
    /// codeless-tolerant — see [`Self::apply_at`]), and the `DkgActor` reads its
    /// activation/interval from the SAME resolution rather than re-reading the
    /// chain itself. `Some(_)` also doubles as the bootstrap signal the poller
    /// uses to switch from `cold_start` to the steady `on_finalized` boundary walk
    /// (which REQUIRES the freeze), distinct from a plain `Intra` no-op (which can
    /// also mean "already tracked").
    pub fn frozen_geometry(&self) -> Option<(u64, u64)> {
        Some((self.frozen_activation?, self.frozen_interval? as u64))
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
    /// The monotonicity is a property of HOW THIS SEAM IS WIRED, not of the type:
    /// [`Self::cold_start`] is the other writer and it assigns `anchor_height`
    /// unconditionally, so it can also lower it. What holds is that the instance the
    /// seam is wired to — the consensus-plane one — is cold-started exactly once, at
    /// construction, before the executor that publishes the raise exists
    /// (`consensus/dpos.rs:2162`). Wiring it instead to a repeatedly cold-started
    /// instance — the beacon-plane transition is cold-started on EVERY 500 ms poller
    /// tick until the geometry freezes (`node/src/dpos.rs:1298-1315`) — would let a
    /// later `cold_start` drop the floor back into the pruned window and defeat the
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
            return Ok(merge_replay_outcome(replay_advance, TransitionOutcome::Intra));
        };
        let outcome = self.apply_at(number, at).await?;
        Ok(merge_replay_outcome(replay_advance, outcome))
    }

    /// The pre-deferred `on_finalized` body: epoch geometry freeze +
    /// cold-start bootstrap (incl. boundary-resume E+1) + boundary branch,
    /// reading committee state at the RESOLVED executed hash `at`.
    async fn apply_at(&mut self, number: u64, at: B256) -> Result<TransitionOutcome, ReadError> {
        // Deferred bootstrap: until DPoS is actually a scheduled, deployed chain at
        // `at`, the ChainConfig staticcalls below revert (codeless account) or read
        // the `0` unscheduled sentinel. On a cold-restart into `--dpos` the anchor
        // can momentarily be the genesis fallback (reth has not yet surfaced its
        // persisted finalized marker), so freezing here would FATALLY mis-read the
        // geometry. `scheduled_dpos_activation` folds both the codeless and the `0`
        // cases to `None`; on `None` we return a benign no-op and leave the geometry
        // UNFROZEN — the beacon-plane poller re-`cold_start`s each tick off the live
        // finalized cursor (an existing event, NOT a new timer) and freezes the
        // instant a readable, DPoS-scheduled finalized block exists, at which point
        // `frozen_geometry()` becomes `Some(_)` (consumed by the poller's branch and
        // the DkgActor). The resolved activation is reused for the freeze below so
        // this adds no extra read.
        let Some(scheduled_activation) = self.reader.scheduled_dpos_activation(at)? else {
            return Ok(TransitionOutcome::Intra);
        };
        // `epochBlockInterval` is treated as FIXED after genesis: the consensus
        // `FixedEpocher` is frozen at startup, so acting on a live governance
        // change here would diverge the two epoch authorities (a boundary-synced
        // live re-interval is a separate, deferred task). Freeze on the first
        // finalized block; log + ignore any later on-chain change.
        let observed = self.reader.epoch_block_interval(at)?;
        if observed == 0 {
            return Err(ReadError::ZeroEpochInterval);
        }
        let interval = freeze_or_warn(
            &mut self.frozen_interval,
            observed,
            "epochBlockInterval (consensus FixedEpocher is frozen)",
        );
        // Freeze the relative-epoch origin on the first finalized block, mirroring
        // the interval freeze (consensus OriginEpocher is frozen at startup). Reuse
        // the value already resolved by `scheduled_dpos_activation` — the `0`-fold
        // never reaches here (it returned `None` above), so this is the raw
        // activation height (unscheduled `0` is impossible past the gate).
        let activation = freeze_or_warn(
            &mut self.frozen_activation,
            scheduled_activation,
            "dposActivationBlock (consensus OriginEpocher is frozen)",
        );
        let epoch_e = epoch_of_block(number, interval, activation);

        // Boundary detection MUST be activation-relative, matching `epoch_of_block`
        // (reader.rs) and the consensus `OriginEpocher`: the last block of relative
        // epoch E is where `(number - activation) % interval == interval - 1`, i.e.
        // `(number + 1 - activation) % interval == 0`. The absolute form
        // `(number + 1) % interval == 0` only agrees when `activation % interval == 0`
        // (a devnet bootstrap convention, NOT enforced — prod cold-start anchors on
        // an arbitrary recent finalized height), so an absolute check would fire the
        // peer-set handoff at a different block than `OriginEpocher` treats as the
        // boundary — the exact "two epoch authorities diverge" failure the freeze
        // logic above guards against.
        let is_boundary = is_epoch_boundary(number, interval, activation);

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
        // committee was committed one epoch ahead (at the first block of epoch E,
        // §4.4), so the frozen `getEpochCommittee(E+1)` is on-chain by now and the
        // genesis block for engine E+1 (= this finalized last-block of E) is
        // stored. The engine-E engine keeps producing until E+1 takes over.
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
                // (`executed_hash(B−1−K)`). Under the v41 QUALIFY-BEFORE-COMMIT schedule
                // this is a TRANSIENT state-visibility lag, never a genuine missed
                // commit: EVERY epoch now gets a committee — candidate-if-qualified else
                // the incumbent carry — committed at `H_qual = B−8 ≤ B−1−K`, so an empty
                // read here can only be reth's eager-canonicalization state lag. KEEP the
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
                         for re-poke (transient state-visibility lag; v41 schedule guarantees \
                         a committed committee by H_qual = B−8)"
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

    /// Persist + size-check + prune the frozen committee, feed the peer set to the
    /// sink, and fire the boundary trigger — advancing `last_tracked_epoch` only on
    /// a successful `try_send`. Extracted so both the cold-start bootstrap and the
    /// boundary branch share identical (idempotent) side effects.
    ///
    /// The tracked peer set is the Active validator REGISTRY ∪ the frozen
    /// committee (tier-2: every activated validator — ejected, upcoming, the
    /// sequencer — keeps consensus-plane connectivity; the committee union
    /// covers the mid-epoch-jailed member that already left the registry but
    /// is still in the frozen committee). The cache/schemes/bridge continue
    /// to consume the COMMITTEE snapshot only.
    async fn track_and_trigger(
        &mut self,
        epoch: u64,
        snap: crate::reader::ValidatorSetSnapshot,
        at: B256,
    ) -> Result<TriggerResult, ReadError> {
        let mut tracked = self.reader.active_registry_peers(at)?;
        tracked.extend(snap.validators.iter().map(|v| v.keys.peer_pubkey.clone()));
        check_peer_set_size(epoch, tracked.len(), self.max_peer_set_size)?; // typed, not panic
        let retention =
            self.reader.undelegate_period(at)? as u64 + EPOCH_COMMITTEE_RETENTION_MARGIN;
        {
            let mut cache = self.cache.lock().await;
            cache.persist_final(snap.clone()).await?; // finality-gated — idempotent
            cache.prune(epoch.saturating_sub(retention)).await?; // mirror on-chain prune
        }
        self.sink.track(epoch, Set::from_iter_dedup(tracked)).await; // one-shot

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

    /// Bulk catch-up committee reader: register a verify-only scheme for every
    /// epoch in the inclusive span `[from, to]` by reading each committee from
    /// the CURRENT finalized state — used by the consensus catch-up path
    /// ([`crate::epoch_transition`]'s `soft_enter_span` callback) to pre-register
    /// a whole gap in one step instead of one boundary per finalized round-trip.
    ///
    /// Committees are read at the SAME result-final state the in-order boundary
    /// path uses: `read_height_for(anchor_number)` (= `anchor_number − result_lag`,
    /// clamped to the anchor floor), resolved to an executed hash via
    /// `executed_hash`. For each `epoch in from..=to`, `epoch_committee_snapshot`
    /// is read at that hash; a non-empty committee is handed to `register` and
    /// `registered` advances; the FIRST empty / unreadable / missed-commit epoch
    /// BREAKS the loop (the on-chain commit cursor is a contiguous prefix, so a
    /// gap means nothing above is committed yet). Returns the highest epoch
    /// registered, or `from − 1` if the read state is unresolvable / none qualify.
    ///
    /// SIDE-EFFECT-FREE: it advances NO `EpochTransition` state — neither
    /// `last_tracked_epoch` nor `pending_boundary` — and does not persist/track/
    /// prune. It is orthogonal to the in-order `on_finalized` boundary walk: this
    /// is a read-only fan-out that hands schemes to the marshal's verifier, while
    /// the boundary walk remains the sole authority that advances epoch state.
    pub async fn soft_enter_span(
        &self,
        from: u64,
        to: u64,
        anchor_number: u64,
        register: &(dyn Fn(u64, crate::reader::ValidatorSetSnapshot) + Send + Sync),
    ) -> u64 {
        let none = from.saturating_sub(1);
        // Speculative side-effect-free fan-out: an unreadable read state is its
        // existing non-fatal stop, so BOTH `Ok(None)` (not yet materialized) and
        // `Err` (a read fault) collapse to "register nothing" here — a fault on
        // this path is never fatal (the in-order boundary walk is the sole
        // authority and re-surfaces any real error).
        let Ok(Some(read_at)) = (self.executed_hash)(self.read_height_for(anchor_number)) else {
            return none;
        };
        let mut registered = none;
        for epoch in from..=to {
            match self.reader.epoch_committee_snapshot(epoch, read_at) {
                Ok(snap) if !snap.validators.is_empty() => {
                    register(epoch, snap);
                    registered = epoch;
                }
                // Empty (missed-commit), unreadable, or read error: the on-chain
                // commit cursor is a contiguous prefix, so stop at the first gap.
                _ => break,
            }
        }
        registered
    }

    /// Cold start: freeze the epoch geometry and read the **current
    /// finalized** committee at the EXPLICIT anchor hash `head` (the anchor
    /// is executed by construction — the one height where no `executed_hash`
    /// resolution is needed), apply once. Also pins the read-height floor for
    /// every later `on_finalized`. MUST run before `on_finalized`.
    pub async fn cold_start(
        &mut self,
        head: B256,
        head_number: u64,
    ) -> Result<TransitionOutcome, ReadError> {
        self.anchor_height = Some(head_number);
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
            stake: 1,
        }
    }

    /// Canned reader: fixed committee size + undelegate period + interval.
    struct MockReader {
        committee: usize,
        undelegate: u32,
        interval: u32,
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
            })
        }
        fn undelegate_period(&self, _at: B256) -> Result<u32, ReadError> {
            Ok(self.undelegate)
        }
        fn epoch_block_interval(&self, _at: B256) -> Result<u32, ReadError> {
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
        cache: std::sync::Arc<tokio::sync::Mutex<ValidatorSetCache<deterministic::Context>>>,
        sink: RecordingSink,
        max: usize,
        tx: Option<tokio::sync::mpsc::Sender<(u64, crate::reader::ValidatorSetSnapshot)>>,
        h: B256,
    ) -> EpochTransition<MockReader, RecordingSink, deterministic::Context> {
        EpochTransition::new(
            reader,
            cache,
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
        fn undelegate_period(&self, at: B256) -> Result<u32, ReadError> {
            self.inner.undelegate_period(at)
        }
        fn epoch_block_interval(&self, at: B256) -> Result<u32, ReadError> {
            self.inner.epoch_block_interval(at)
        }
        fn dpos_activation_block(&self, at: B256) -> Result<u64, ReadError> {
            self.inner.dpos_activation_block(at)
        }
        fn active_registry_peers(&self, _at: B256) -> Result<Vec<PeerPubkey>, ReadError> {
            Ok(self.registry.clone())
        }
    }

    /// `MockReader` capped to a contiguous committed prefix: a committee is
    /// non-empty for `epoch <= committed_to`, empty above — mirroring the
    /// on-chain `commitEpochCommittee` cursor that only ever fills a prefix.
    struct PrefixReader {
        inner: MockReader,
        committed_to: u64,
    }
    impl StakingStateRead for PrefixReader {
        fn epoch_committee_snapshot(
            &self,
            epoch: u64,
            at: B256,
        ) -> Result<ValidatorSetSnapshot, ReadError> {
            if epoch > self.committed_to {
                return Ok(ValidatorSetSnapshot {
                    block_hash: at,
                    block_number: epoch * 100,
                    epoch,
                    validators: vec![],
                });
            }
            self.inner.epoch_committee_snapshot(epoch, at)
        }
        fn undelegate_period(&self, at: B256) -> Result<u32, ReadError> {
            self.inner.undelegate_period(at)
        }
        fn epoch_block_interval(&self, at: B256) -> Result<u32, ReadError> {
            self.inner.epoch_block_interval(at)
        }
        fn dpos_activation_block(&self, at: B256) -> Result<u64, ReadError> {
            self.inner.dpos_activation_block(at)
        }
        fn active_registry_peers(&self, at: B256) -> Result<Vec<PeerPubkey>, ReadError> {
            self.inner.active_registry_peers(at)
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
        fn undelegate_period(&self, at: B256) -> Result<u32, ReadError> {
            self.inner.undelegate_period(at)
        }
        fn epoch_block_interval(&self, at: B256) -> Result<u32, ReadError> {
            self.inner.epoch_block_interval(at)
        }
        fn dpos_activation_block(&self, _at: B256) -> Result<u64, ReadError> {
            Ok(self.activation)
        }
        fn active_registry_peers(&self, at: B256) -> Result<Vec<PeerPubkey>, ReadError> {
            self.inner.active_registry_peers(at)
        }
    }

    /// Records every `track` call.
    #[derive(Clone, Default)]
    struct RecordingSink(Arc<Mutex<Vec<(u64, usize)>>>);
    impl PeerSetSink for RecordingSink {
        fn track(&mut self, epoch: u64, peers: Set<PeerPubkey>) -> impl Future<Output = ()> + Send {
            let log = self.0.clone();
            async move {
                log.lock().unwrap().push((epoch, peers.len()));
            }
        }
    }

    #[test]
    fn tracked_set_is_registry_union_committee() {
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let sink = RecordingSink::default();
            let h = B256::repeat_byte(0x33);
            // 2 registry-only peers (seeds far from the committee's) + committee of 3.
            let reader = RegistryReader {
                inner: MockReader {
                    committee: 3,
                    undelegate: 7,
                    interval: 100,
                },
                registry: vec![
                    validator(900_001).keys.peer_pubkey,
                    validator(900_002).keys.peer_pubkey,
                ],
            };
            let mut et = EpochTransition::new(
                reader,
                cache,
                sink.clone(),
                51,
                None,
                std::sync::Arc::new(move |_n| Ok(Some(h))),
                3,
            );
            et.cold_start(h, 200).await.unwrap();
            let log = sink.0.lock().unwrap();
            assert_eq!(log.as_slice(), &[(2, 5)]);
        });
    }

    #[test]
    fn boundary_apply_persists_and_tracks_once() {
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let sink = RecordingSink::default();
            let h = B256::repeat_byte(0x11);
            let mut et = et(
                MockReader {
                    committee: 5,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
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
                assert_eq!(*log, vec![(5, 5)], "tracked once, 5 peers, epoch 5");
            }
            assert!(
                et.cache.lock().await.contains(h).await.unwrap(),
                "snapshot persisted"
            );
        });
    }

    #[test]
    fn replayed_boundary_advance_is_surfaced_not_dropped() {
        // Bug 11: a boundary parked while execution lagged, then replayed on the
        // next (intra-epoch) delivery, must SURFACE its `EpochAdvanced` rather
        // than be dropped in favour of the new delivery's `Intra` — else the
        // engine's consecutive-error counter never resets and false-shuts-down.
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let sink = RecordingSink::default();
            let h = B256::repeat_byte(0x21);
            // Gate hash resolution so a boundary can be parked (unresolvable) then
            // replayed (resolvable) — the exact lag→catch-up sequence bug 11 needs.
            let resolve = Arc::new(std::sync::atomic::AtomicBool::new(true));
            let resolve_c = resolve.clone();
            let mut et = EpochTransition::new(
                MockReader {
                    committee: 5,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
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
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let sink = RecordingSink::default();
            let h = B256::repeat_byte(0x22);
            let mut et = et(
                MockReader {
                    committee: 5,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
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
                vec![(5, 5), (6, 5)],
                "bootstrap epoch 5, then spawn epoch 6 at its boundary"
            );
        });
    }

    #[test]
    fn cold_start_on_boundary_enters_next_epoch() {
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let sink = RecordingSink::default();
            let h = B256::repeat_byte(0x55);
            let mut et = et(
                MockReader {
                    committee: 5,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
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
                vec![(6, 5)],
                "boundary cold-start tracks epoch 6"
            );
        });
    }

    #[test]
    fn oversize_committee_is_typed_error_not_panic() {
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let h = B256::repeat_byte(0x22);
            let mut et = et(
                MockReader {
                    committee: 10,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
                RecordingSink::default(),
                4, // max_peer_set_size < tracked union (registry ∅ + committee 10)
                None,
                h,
            );
            assert!(matches!(
                et.cold_start(h, 200).await,
                Err(ReadError::PeerSetTooLarge {
                    epoch: 2,
                    size: 10,
                    max: 4
                })
            ));
        });
    }

    #[test]
    fn zero_interval_is_typed_error_not_panic() {
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let h = B256::repeat_byte(0x01);
            let mut et = et(
                MockReader {
                    committee: 3,
                    undelegate: 7,
                    interval: 0,
                },
                cache,
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
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let sink = RecordingSink::default();
            let h = B256::repeat_byte(0x44);
            let mut et = et(
                MockReader {
                    committee: 0,
                    undelegate: 7,
                    interval: 100,
                }, // no commit ⇒ empty
                cache,
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
            assert!(
                !et.cache.lock().await.contains(h).await.unwrap(),
                "empty snapshot not persisted"
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
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
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
            };
            boundary_tx.try_send((999, dummy)).expect("first slot");
            // Now channel is full.
            let h = B256::repeat_byte(0xC6);
            let mut et = et(
                MockReader {
                    committee: 3,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
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
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let sink = RecordingSink::default();
            // Capacity-1 channel: cold_start fills it with epoch 5, so the
            // epoch-6 boundary send then hits Full.
            let (boundary_tx, mut boundary_rx) = tokio::sync::mpsc::channel(1);
            let h = B256::repeat_byte(0xC7);
            let mut et = et(
                MockReader {
                    committee: 3,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
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
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let h = B256::repeat_byte(0x77);
            let mut et = et(
                MockReader {
                    committee: 3,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
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
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let sink = RecordingSink::default();
            let (boundary_tx, mut boundary_rx) = tokio::sync::mpsc::channel(8);
            let h = B256::repeat_byte(0x78);
            let mut et = et(
                MockReader {
                    committee: 3,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
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
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let sink = RecordingSink::default();
            let (boundary_tx, mut boundary_rx) = tokio::sync::mpsc::channel(8);
            let h = B256::repeat_byte(0xCD);
            let mut et = et(
                MockReader {
                    committee: 4,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
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
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let sink = RecordingSink::default();
            let h = B256::repeat_byte(0x33);
            let mut et = et(
                MockReader {
                    committee: 3,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
                sink.clone(),
                64,
                None,
                h,
            );
            et.cold_start(h, 1200).await.unwrap();
            assert_eq!(*sink.0.lock().unwrap(), vec![(12, 3)]);
        });
    }

    #[test]
    fn lagging_execution_defers_boundary_and_replays_it() {
        // Boundary at 599 arrives while the executed tip lags its read height →
        // remembered; a subsequent NON-boundary unresolved height must NOT
        // clobber it; once execution catches up, the next delivery replays the
        // boundary and epoch 6 enters.
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let sink = RecordingSink::default();
            let h = B256::repeat_byte(0x66);
            let resolvable = Arc::new(Mutex::new(true));
            let resolvable_for_et = resolvable.clone();
            let mut et = EpochTransition::new(
                MockReader {
                    committee: 5,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
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
                vec![(5, 5), (6, 5)],
                "epoch 6 entered via the pending-boundary replay"
            );
        });
    }

    #[test]
    fn soft_enter_span_registers_contiguous_committed_prefix() {
        // The on-chain commit cursor fills only a prefix: committee non-empty for
        // epoch ≤ committed_to, empty above. soft_enter_span must register exactly
        // [from ..= committed_to], return committed_to, hand back snapshots equal
        // to the boundary-path read, and touch NO EpochTransition state.
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let h = B256::repeat_byte(0x9A);
            let committed_to = 5u64;
            let et = EpochTransition::new(
                PrefixReader {
                    inner: MockReader {
                        committee: 4,
                        undelegate: 7,
                        interval: 100,
                    },
                    committed_to,
                },
                cache,
                RecordingSink::default(),
                64,
                None,
                std::sync::Arc::new(move |_n| Ok(Some(h))),
                3,
            );

            let recorded: Arc<Mutex<Vec<(u64, ValidatorSetSnapshot)>>> = Arc::new(Mutex::new(vec![]));
            let rec = recorded.clone();
            let register = move |epoch: u64, snap: ValidatorSetSnapshot| {
                rec.lock().unwrap().push((epoch, snap));
            };
            let registered = et.soft_enter_span(2, 8, 500, &register).await;

            assert_eq!(registered, committed_to, "truncates at the committed prefix");
            let recorded = recorded.lock().unwrap();
            assert_eq!(
                recorded.iter().map(|(e, _)| *e).collect::<Vec<_>>(),
                vec![2, 3, 4, 5],
                "registers exactly the contiguous committed prefix [2..=5]"
            );
            // Each handed-back snapshot equals the boundary-path read at the
            // result-final state (read_height_for(500) resolves to `h`).
            for (epoch, snap) in recorded.iter() {
                let expected = et
                    .reader
                    .epoch_committee_snapshot(*epoch, h)
                    .unwrap();
                assert_eq!(snap.epoch, expected.epoch);
                assert_eq!(snap.validators.len(), expected.validators.len());
                assert_eq!(snap.block_hash, expected.block_hash);
            }
            // Side-effect-free: no ET state advanced.
            assert_eq!(et.last_tracked_epoch, None, "soft_enter_span advances no epoch state");
            assert!(!et.has_pending_boundary(), "soft_enter_span parks no boundary");
        });
    }

    #[test]
    fn soft_enter_span_unresolvable_read_state_registers_nothing() {
        // When the executed tip hasn't reached the result-final read height, the
        // read state is unresolvable → register nothing, return from − 1.
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let et = EpochTransition::new(
                MockReader {
                    committee: 4,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
                RecordingSink::default(),
                64,
                None,
                std::sync::Arc::new(|_n| Ok(None)), // nothing executed yet
                3,
            );
            let calls = Arc::new(Mutex::new(0u32));
            let c = calls.clone();
            let register = move |_e: u64, _s: ValidatorSetSnapshot| {
                *c.lock().unwrap() += 1;
            };
            let registered = et.soft_enter_span(3, 8, 500, &register).await;
            assert_eq!(registered, 2, "from − 1 when the read state is unresolvable");
            assert_eq!(*calls.lock().unwrap(), 0, "nothing registered");
        });
    }

    #[test]
    fn cold_start_pre_activation_bootstraps_epoch_0_never_1() {
        // Bug 3: cold-start on a block BEFORE a scheduled future activation must
        // bootstrap epoch 0 (a pre-activation block belongs to no relative epoch, so
        // it is not a boundary). The pre-fix `saturating_sub` underflow classified
        // every pre-activation block as a boundary → `cold_epoch = epoch_e + 1 = 1`,
        // tracking a phantom committee[1] on the sequencer→DPoS migration path.
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let sink = RecordingSink::default();
            let (boundary_tx, mut boundary_rx) = tokio::sync::mpsc::channel(8);
            let h = B256::repeat_byte(0x3A);
            let mut et = EpochTransition::new(
                FutureActivationReader {
                    inner: MockReader {
                        committee: 3,
                        undelegate: 7,
                        interval: 100,
                    },
                    activation: 1000,
                },
                cache,
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
                vec![(0, 3)],
                "committee[0] tracked — never a phantom committee[1]"
            );
            assert_eq!(
                et.last_tracked_epoch,
                Some(0),
                "epoch 0 tracked; last_tracked_epoch never prematurely Some(1)"
            );
            let fired = boundary_rx.try_recv().expect("epoch-0 boundary trigger delivered");
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
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let sink = RecordingSink::default();
            let h = B256::repeat_byte(0x3B);
            let mut et = EpochTransition::new(
                FutureActivationReader {
                    inner: MockReader {
                        committee: 3,
                        undelegate: 7,
                        interval: 100,
                    },
                    activation: 1000,
                },
                cache,
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
            assert_eq!(*sink.0.lock().unwrap(), vec![(0, 3)]);
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
        fn undelegate_period(&self, at: B256) -> Result<u32, ReadError> {
            self.gate(at)?;
            self.inner.undelegate_period(at)
        }
        fn epoch_block_interval(&self, at: B256) -> Result<u32, ReadError> {
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
            undelegate: 7,
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
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let materialized = Arc::new(Mutex::new(500u64));
            let reads = Arc::new(Mutex::new(vec![]));
            let mut et = EpochTransition::new(
                StateLagReader {
                    inner: state_lag_mock(),
                    materialized: materialized.clone(),
                    reads,
                },
                cache,
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
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let best = Arc::new(Mutex::new(500u64));
            let reads = Arc::new(Mutex::new(vec![]));
            let mut et = EpochTransition::new(
                StateLagReader {
                    inner: state_lag_mock(),
                    materialized: best.clone(),
                    reads: reads.clone(),
                },
                cache,
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
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let closure_best = Arc::new(Mutex::new(700u64));
            let reader_materialized = Arc::new(Mutex::new(500u64));
            let mut et = EpochTransition::new(
                StateLagReader {
                    inner: state_lag_mock(),
                    materialized: reader_materialized,
                    reads: Arc::new(Mutex::new(vec![])),
                },
                cache,
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
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let best = Arc::new(Mutex::new(500u64));
            let mut et = EpochTransition::new(
                StateLagReader {
                    inner: state_lag_mock(),
                    materialized: best.clone(),
                    reads: Arc::new(Mutex::new(vec![])),
                },
                cache,
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
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let best = Arc::new(Mutex::new(500u64));
            let reads = Arc::new(Mutex::new(vec![]));
            let sink = RecordingSink::default();
            let mut et = EpochTransition::new(
                StateLagReader {
                    inner: state_lag_mock(),
                    materialized: best.clone(),
                    reads: reads.clone(),
                },
                cache,
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
                assert_eq!(
                    et.on_finalized(n).await.unwrap(),
                    TransitionOutcome::Intra
                );
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
            assert_eq!(epoch6_tracks, 1, "the deferred committee is tracked exactly once");
        });
    }

    #[test]
    fn honest_nodes_with_equal_materialized_head_park_identically() {
        // Determinism: the gate is a pure provider read with NO wall-clock, so two
        // honest nodes fed the SAME `best_block_number` sequence and the same
        // deliveries make IDENTICAL park/advance decisions — no non-deterministic
        // input feeds the consensus-relevant outcome.
        async fn run(
            ctx: deterministic::Context,
            best_script: &[u64],
        ) -> Vec<TransitionOutcome> {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let best = Arc::new(Mutex::new(500u64));
            let mut et = EpochTransition::new(
                StateLagReader {
                    inner: state_lag_mock(),
                    materialized: best.clone(),
                    reads: Arc::new(Mutex::new(vec![])),
                },
                cache,
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
        deterministic::Runner::default().start(|ctx| async move {
            // Flat below the read height, then a jump — the two nodes must agree
            // step-for-step (park, park, park, advance).
            let script = [500u64, 500, 500, 600];
            let a = run(ctx.with_label("node_a"), &script).await;
            let b = run(ctx.with_label("node_b"), &script).await;
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
    const PROD_INTERVAL: u32 = 86_400;

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
        fn undelegate_period(&self, at: B256) -> Result<u32, ReadError> {
            self.gate(at)?;
            self.inner.undelegate_period(at)
        }
        fn epoch_block_interval(&self, at: B256) -> Result<u32, ReadError> {
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
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let best = Arc::new(Mutex::new(100_000u64));
            let retained_from = Arc::new(Mutex::new(0u64));
            let reads = Arc::new(Mutex::new(vec![]));
            let mut et = EpochTransition::new(
                PrunedStateReader {
                    inner: MockReader {
                        committee: 5,
                        undelegate: 7,
                        interval: PROD_INTERVAL,
                    },
                    retained_from: retained_from.clone(),
                    reads: reads.clone(),
                },
                cache,
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
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let h = B256::repeat_byte(0x77);
            let mut et = et(
                MockReader {
                    committee: 3,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
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
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let best = Arc::new(Mutex::new(100_000u64));
            let retained_from = Arc::new(Mutex::new(0u64));
            let reads = Arc::new(Mutex::new(vec![]));
            let mut et = EpochTransition::new(
                PrunedStateReader {
                    inner: MockReader {
                        committee: 5,
                        undelegate: 7,
                        interval: PROD_INTERVAL,
                    },
                    retained_from: retained_from.clone(),
                    reads: reads.clone(),
                },
                cache,
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
}
