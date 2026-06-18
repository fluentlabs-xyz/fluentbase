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
/// Retry ceiling (~60s at the backoff above) before giving up loudly.
pub const PENDING_RETRY_LIMIT: u32 = 300;

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
    /// Canonical EVM hash by height — the deferred-execution re-key: committee
    /// reads resolve at `number − result_lag` (a result-final height) instead
    /// of the ordering-finalized block's own hash, which has no executed state
    /// yet. Provider-backed (by NUMBER, never best_number).
    executed_hash: std::sync::Arc<dyn Fn(u64) -> Option<B256> + Send + Sync>,
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
        executed_hash: std::sync::Arc<dyn Fn(u64) -> Option<B256> + Send + Sync>,
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

    /// The executed height committee reads resolve at for an
    /// ordering-finalized `number`: `number − result_lag`, clamped to the
    /// cold-start anchor (≤ anchor is executed by construction). Frozen
    /// committee arrays + one-shot consensus keys make the snapshot content
    /// hash-invariant across any executed in-epoch hash, so the lagged read
    /// point loses nothing.
    fn read_height_for(&self, number: u64) -> u64 {
        let floor = self.anchor_height.unwrap_or(0);
        number.saturating_sub(self.result_lag).max(floor)
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
        // order. A single slot suffices: boundaries are interval-apart
        // (devnet 32, governance-settable) ≫ result_lag + the executor ack
        // window, so two can never be pending at once.
        if let Some(b) = self.pending_boundary {
            if let Some(at) = (self.executed_hash)(self.read_height_for(b)) {
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
            }
        }
        let Some(at) = (self.executed_hash)(self.read_height_for(number)) else {
            // Executed tip hasn't reached number − result_lag yet (transient:
            // bounded by the executor ack window). Remember ONLY boundaries.
            if self.is_epoch_boundary_frozen(number) == Some(true) {
                // Single-slot invariant: a second boundary can be parked only by
                // clobbering the first, silently dropping its epoch handoff. This
                // is unreachable while `interval > MAX_PENDING_ACKS + result_lag`
                // (marshal ack backpressure resolves execution within one
                // interval — see executor.rs), which holds for the shipped
                // interval (32 > 16 + 3). A genesis interval that violates it
                // would make this fire — fail loud in debug/tests rather than
                // lose an epoch silently in release.
                debug_assert!(
                    self.pending_boundary.is_none_or(|p| p == number),
                    "two boundaries pending at once (parked {:?}, new {number}): execution \
                     lagged a full epoch interval — interval > MAX_PENDING_ACKS + result_lag \
                     invariant violated (check the frozen epochBlockInterval)",
                    self.pending_boundary,
                );
                self.pending_boundary = Some(number);
            }
            return Ok(TransitionOutcome::Intra);
        };
        self.apply_at(number, at).await
    }

    /// The pre-deferred `on_finalized` body: epoch geometry freeze +
    /// cold-start bootstrap (incl. boundary-resume E+1) + boundary branch,
    /// reading committee state at the RESOLVED executed hash `at`.
    async fn apply_at(&mut self, number: u64, at: B256) -> Result<TransitionOutcome, ReadError> {
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
        // the interval freeze (consensus OriginEpocher is frozen at startup).
        let observed = self.reader.dpos_activation_block(at)?;
        let activation = freeze_or_warn(
            &mut self.frozen_activation,
            observed,
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
                // Missed-commit epoch: nothing to hand off — release any park so
                // the re-poke loop stops (a later commit lands via a fresh path).
                self.pending_boundary = None;
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
            std::sync::Arc::new(move |_n| Some(h)),
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
                std::sync::Arc::new(move |_n| Some(h)),
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
                std::sync::Arc::new(move |_n| resolvable_for_et.lock().unwrap().then_some(h)),
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
                TransitionOutcome::Intra,
                "601 itself is intra; the boundary fires via the replay"
            );
            assert_eq!(
                *sink.0.lock().unwrap(),
                vec![(5, 5), (6, 5)],
                "epoch 6 entered via the pending-boundary replay"
            );
        });
    }
}
