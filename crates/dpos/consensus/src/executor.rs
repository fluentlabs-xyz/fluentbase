//! Executor: drives the reth EL from ordering-finalized [`OrderBlock`]s —
//! derive, execute (`new_payload`), then a two-tier forkchoice update.
//!
//! Three-tier forkchoice: `head` follows the locally derived speculative tip;
//! `safe` rides the ordering-finalized tip, content-immutable the moment it is
//! finalized; `finalized` follows result finality `ordering_finalized − K`,
//! clamped to the cold-start anchor — the height whose derived hash the
//! committee attested by agreeing the OrderBlock K heights above it. The
//! invariant `finalized ⊆ safe ⊆ head` holds at every update.
//!
//! Derive pipeline: the beacon seed for height `h` is σ of `h`'s own agreed
//! round, `Round(epoch(h), h.proposal_view)`, read from the local seed store,
//! so no block carries another block's randomness. Resolution is predicate
//! first: `mandatory_at(epoch(h))` decides before the store is consulted, so a
//! σ filed at a round the agreed epoch map calls beacon-inactive is ignored and
//! counted, never obeyed — ignoring derives what the rest of the network
//! derives, where halting would turn one bad journal record into an outage. A
//! miss on a beacon-active round holds the block in [`Actor::awaiting_seed`],
//! whose only exit is the seed-record notify — never a timer and never the
//! `order.digest()` fallback, which re-rolls `prev_randao` and forks. The derive
//! path never parks, re-pokes, re-fetches by height, or reads a certificate.
//!
//! Ack flow: the marshal's `Exact` ack fires only after derive and import, so
//! marshal backpressure (`MAX_PENDING_ACKS`) is execution backpressure.
//!
//! The executor never acknowledges a block it has not derived, and never drops
//! an `Exact` while the marshal is alive: a dropped `Exact` cancels, and the
//! marshal treats a canceled ack as fatal, killing the component that serves
//! blocks and certs to peers. An ack is therefore (a) acknowledged after derive
//! and import, (b) held in [`Actor::awaiting_seed`] until σ arrives, (c) parked
//! with a deferred block (an absent `h+K` body is the only park), or (d)
//! retained unresolved by [`Actor::park_halted`] when a `SafetyHalt`
//! engages — the halt posture stops progress while the marshal keeps serving
//! peers. `reseed_forward` is the only path that disposes a parked or held ack
//! differently, acknowledging it because the floor moves past the parked height
//! (pruned, not skipped). At shutdown the held ack is dropped deliberately: the
//! executor and the marshal die together at the runtime drop, so the marshal's
//! fatal ack arm cannot observe the cancellation, and the withheld ack is the
//! restart self-heal — `last_processed_height` advances only on `Ok`, so the
//! restarted marshal re-dispatches the held height to derive on the next run.
//! Acknowledging it at shutdown would durably skip it forever.

use crate::digest::Digest;
use crate::{
    application::{BeaconEngineLike, DerivedBlock as _, DerivedBlockBuilder, ExecutedChain},
    fault::{DeferReason, Fault, FaultClass},
    order_block::OrderBlock,
    sync_metrics::{SyncMetrics, SyncReason},
};
use alloy_primitives::B256;
use alloy_rpc_types_engine::{ForkchoiceState, ForkchoiceUpdated};
use commonware_consensus::{
    marshal::Update,
    simplex::types::{Activity, Finalization},
    types::{Height, Round},
    Reporter,
};
use commonware_runtime::{spawn_cell, Clock, ContextCell, Handle, Metrics as _, Spawner};
use commonware_utils::{
    acknowledgement::Exact, channel::oneshot, futures::OptionFuture, vec::NonEmptyVec,
    Acknowledgement as _,
};
use eyre::WrapErr as _;
use fluentbase_bls::PeerPubkey;
use fluentbase_bls::Scheme as BlsScheme;
use futures::{
    future::{ready, BoxFuture, Ready},
    stream::FuturesOrdered,
    FutureExt as _, StreamExt as _,
};
use prometheus_client::metrics::gauge::Gauge;
use std::{
    collections::BTreeMap,
    ops::RangeInclusive,
    pin::Pin,
    time::{Duration, SystemTime},
};
use tokio::{select, sync::broadcast, sync::mpsc};
use tracing::{debug, error, error_span, info, info_span, instrument, warn, Level, Span};

/// Pacing seam for execution-layer calls (`fork_choice_updated`, `import_derived`).
/// `commonware_runtime::Pacer::pace` is a no-op under the tokio runtime; the
/// deterministic `Pacer` blocks the OS thread and sits behind the runtime's
/// `external` feature, which makes that runtime sleep real time and pins every test
/// to the wall clock. The seam keeps the `fcu_pace` call sites and knob in place.
trait PaceElCall: std::future::Future + Sized {
    /// Runs the call immediately; `expected_latency` is documentation only.
    fn pace_el_call(self, _expected_latency: Duration) -> Self {
        self
    }
}

impl<F: std::future::Future> PaceElCall for F {}

/// An executor command paired with its tracing span, preserving the causal
/// `parent` for `#[instrument]`.
pub struct Message {
    pub cause: Span,
    pub command: Command,
}

pub enum Command {
    /// Derive and import a finalized ordering artifact, or refresh the catch-up
    /// target.
    Finalize(Box<Update<OrderBlock>>),
    /// Speculatively derive and import a just-notarized block ahead of finalization,
    /// to hide execution latency under the finalization rounds. Best-effort:
    /// `try_derive` stays the sole authority and reconciles a mismatch (reuse or
    /// re-derive + reorg). Boxed to keep the enum small.
    SpecNotarized(Box<Notarized>),
}

/// Payload of [`Command::SpecNotarized`]: the ordering digest and the seed
/// recovered from the notarization certificate (the round rides in
/// `seed.target_round`); the body is fetched from the marshal by digest at
/// execution time.
pub struct Notarized {
    pub digest: crate::digest::Digest,
    pub seed: Option<crate::beacon::Seed>,
}

/// Value stored per speculatively-executed height in `Actor::spec_executed`: the
/// notarized ordering digest, the round of the seed the speculation used (`None`
/// on a seed-independent height), and the EVM hash of the parent it executed
/// against. `try_derive` keeps the speculation only when all three match the
/// finalized fork: same ordering block, same seed round, and parent-linked to the
/// block canonical at `height − 1` now.
///
/// A digest match with a different round is an anomaly, not routine churn — both
/// sides are `Round(Ep, block.proposal_view)` — and re-derives from σ of its own
/// round. The parent hash catches a head rollback at `height − 1` that re-derived
/// the parent to a different hash: such a block was executed against an orphaned
/// parent (wrong pre-state) and must re-derive.
#[derive(Clone)]
struct SpecExecuted {
    digest: crate::digest::Digest,
    seed_round: Option<commonware_consensus::types::Round>,
    parent_hash: B256,
}

/// Why a `try_eager_finalized_derive` attempt was made — it only selects the
/// metric label, so the record-vs-delivery race stays observable.
///
/// `Delivery` is the attempt made when the block is delivered or held (the normal
/// record-lag closer): a miss is the transient race (`outcome="miss"`), a hit
/// `outcome="hit"`. `Notified` is the event-driven re-attempt fired when a block
/// is still held and a seed was just recorded; a hit is `outcome="recovered"` —
/// the race healed without a further finalized delivery, which matters because
/// finality only advances via new blocks. A miss is a silent no-op.
#[derive(Clone, Copy)]
enum EagerTrigger {
    Delivery,
    Notified,
}

/// What the executor's wake-up arm does with one delivery of
/// [`Beacon::subscribe`](crate::beacon::Beacon::subscribe).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SeedWake {
    /// Re-run the eager finalized derive of the held tip.
    Derive,
    /// A wake-up for another consumer: no derive, stay armed.
    Ignore,
    /// The beacon's sender is gone. A closed `broadcast` receiver returns `Closed`
    /// immediately and forever, so an armed arm would spin while a tip is held; the
    /// arm disarms itself instead.
    Disarm,
}

/// The wake-up arm's whole classification, out of line so a test can pin it: the
/// spin it closes is only observable through a live `select!`.
fn classify_seed_wake(
    event: &Result<crate::beacon::BeaconEvent, broadcast::error::RecvError>,
) -> SeedWake {
    match event {
        // `Lagged` is a wake-up like any other: the re-check it triggers is
        // idempotent, and the events carry no payload to lose.
        Ok(crate::beacon::BeaconEvent::SeedRecorded)
        | Err(broadcast::error::RecvError::Lagged(_)) => SeedWake::Derive,
        Ok(crate::beacon::BeaconEvent::KeyAvailable)
        | Ok(crate::beacon::BeaconEvent::ParticipationChanged) => SeedWake::Ignore,
        Err(broadcast::error::RecvError::Closed) => SeedWake::Disarm,
    }
}

/// A notarized speculative block parked (not dropped) because it arrived ahead of
/// `spec_head` (a gap) or before its parent had executed. Holds exactly the data
/// [`Command::SpecNotarized`] carries so [`Actor::try_drain_parked`] can re-drive
/// [`Actor::spec_execute`] once `spec_head` catches up.
///
/// Without the park, a notarization beyond `spec_head + 1` would be dropped
/// forever after the executor falls behind, and speculation would stay dead until
/// finalization caught the tip up. Overwrite-by-height is deliberate: a later-view
/// sibling replaces the earlier guess, and a wrong guess is reconciled at
/// finalization.
#[derive(Clone)]
struct ParkedSpec {
    digest: crate::digest::Digest,
    seed: Option<crate::beacon::Seed>,
}

#[derive(Clone)]
pub struct Mailbox {
    tx: mpsc::UnboundedSender<Message>,
}

impl Mailbox {
    fn new(tx: mpsc::UnboundedSender<Message>) -> Self {
        Self { tx }
    }

    /// Builds a drain-only mailbox for tests that need one without spawning an
    /// executor.
    #[cfg(test)]
    pub(crate) fn new_for_test(tx: mpsc::UnboundedSender<Message>) -> Self {
        Self { tx }
    }

    /// Sync send; `UnboundedSender::send` never blocks.
    // SendError<Message> carries the rejected message verbatim so the
    // caller can retry; boxing solely to silence the lint would add an
    // alloc on the hot path.
    #[allow(clippy::result_large_err)]
    pub fn send(&self, msg: Message) -> Result<(), mpsc::error::SendError<Message>> {
        self.tx.send(msg)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LastCanonicalized {
    forkchoice: ForkchoiceState,
    head_height: Height,
    /// Ordering-final tier (the BFT cert tip): the engine-API `safe` tag. Distinct
    /// from `finalized_height`, which lags by K (the committee-attested result);
    /// invariant `finalized ⊆ safe ⊆ head`.
    safe_height: Height,
    finalized_height: Height,
}

impl LastCanonicalized {
    /// Result-final tier (committee-attested execution, `ordering − K`). Sets only
    /// `finalized`; the `head >=` clause pushes `head` so a finalized delivery with
    /// no speculative lead still advances it.
    fn update_finalized(mut self, height: Height, hash: B256) -> Self {
        if height > self.finalized_height {
            self.finalized_height = height;
            self.forkchoice.finalized_block_hash = hash;
        }
        if height >= self.head_height {
            self.head_height = height;
            self.forkchoice.head_block_hash = hash;
        }
        self
    }

    /// Ordering-final tier (the BFT cert tip) → the engine-API `safe` tag.
    ///
    /// The guard is `height >= self.safe_height` (not strict `>`): monotone in
    /// height but lets the hash follow a same-height re-finalization. A strict `>`
    /// would leave `safe` on an orphaned sibling after `head` reorgs away from it,
    /// giving `safe ⊄ head` and reth `-38002`. Do not tighten to `>`.
    ///
    /// A `height < safe_height` delivery is a legitimate no-op: a deep-catch-up
    /// follower seeds `safe_height` at the cold-start anchor and then derives the K
    /// blocks below it, and those deliveries must not roll `safe` back below where
    /// the node trust-anchored.
    ///
    /// Touches only `safe_*`; `head` is owned by `update_finalized` and
    /// `update_head`.
    fn update_safe(mut self, height: Height, hash: B256) -> Self {
        if height >= self.safe_height {
            self.safe_height = height;
            self.forkchoice.safe_block_hash = hash;
        }
        self
    }

    fn update_head(mut self, height: Height, hash: B256) -> Self {
        // A lower-height head on the finalized fork (a reorg of an unfinalized
        // tail) is allowed to roll the head back.
        if height > self.finalized_height || hash == self.forkchoice.finalized_block_hash {
            self.head_height = height;
            self.forkchoice.head_block_hash = hash;
        }
        self
    }
}

// Minimal trait so the executor does not depend on the concrete marshal mailbox.

pub trait BlockFetcher: Clone + Send + Sync + 'static {
    fn fetch_block_by_height(
        &self,
        height: Height,
    ) -> impl std::future::Future<Output = Option<OrderBlock>> + Send;

    /// Best-effort local lookup of a block by its ordering digest. A `None` means
    /// "not local yet", so the speculative path skips and the finalized path derives
    /// it.
    fn fetch_block_by_digest(
        &self,
        digest: crate::digest::Digest,
    ) -> impl std::future::Future<Output = Option<OrderBlock>> + Send;

    /// Local read of the `(finalization, block)` pair archived at `height`, or
    /// `None` on an archive miss. The archive is written only after
    /// `verify_delivered`, so a hit is already committee-authenticated and usable
    /// as a jump target.
    ///
    /// Separate from [`Self::fetch_block_by_height`]: one returns a body for
    /// derive, the other an attested pair for the jump.
    fn pair_at(
        &self,
        height: Height,
    ) -> impl std::future::Future<Output = Option<(Finalization<BlsScheme, Digest>, OrderBlock)>> + Send;

    /// Ask peers for the finalization at `height`. Fire-and-forget; the marshal skips
    /// it when already local.
    fn hint_finalization(
        &self,
        height: Height,
        targets: NonEmptyVec<PeerPubkey>,
    ) -> impl std::future::Future<Output = ()> + Send;

    /// Advance the running marshal's in-order dispatch floor to `height`, resuming
    /// contiguous dispatch from `floor + 1`. Raises-only.
    fn set_floor(&self, height: Height) -> impl std::future::Future<Output = ()> + Send;

    /// Store an already-authenticated finalization and block through the marshal's
    /// inlet ingress (`verified`, then `report(Finalization)`).
    ///
    /// Exists so a caller can seed an entry before raising the floor past it, and that
    /// order is load-bearing: this and [`Self::set_floor`] share one mailbox drained a
    /// message per loop turn, and the below-floor write gate is evaluated at
    /// message-processing time, so a store enqueued first stays readable while one
    /// enqueued after `set_floor` is dropped.
    fn store_verified_finalization(
        &self,
        round: Round,
        block: OrderBlock,
        finalization: Finalization<BlsScheme, Digest>,
    ) -> impl std::future::Future<Output = ()> + Send;
}

/// Concrete marshal mailbox impl; the orphan rule allows it because `BlockFetcher`
/// is local and the mailbox foreign.
impl BlockFetcher
    for commonware_consensus::marshal::core::Mailbox<
        fluentbase_bls::Scheme,
        commonware_consensus::marshal::standard::Standard<OrderBlock>,
    >
{
    async fn fetch_block_by_height(&self, height: Height) -> Option<OrderBlock> {
        self.get_block(height).await
    }

    async fn fetch_block_by_digest(&self, digest: crate::digest::Digest) -> Option<OrderBlock> {
        self.get_block(&digest).await
    }

    async fn pair_at(
        &self,
        height: Height,
    ) -> Option<(Finalization<BlsScheme, Digest>, OrderBlock)> {
        // Delegate rather than duplicate: `FrontierMarshal` already answers this for
        // the same type, and the two-await read has a trap (a block finalizing between
        // the awaits pairs `fin@h` with `block@h+1`) that must stay fixed in one copy.
        crate::plane_upstream::FrontierMarshal::pair_at(self, height).await
    }

    async fn hint_finalization(&self, height: Height, targets: NonEmptyVec<PeerPubkey>) {
        self.hint_finalized(height, targets).await;
    }

    async fn set_floor(&self, height: Height) {
        commonware_consensus::marshal::core::Mailbox::set_floor(self, height).await;
    }

    async fn store_verified_finalization(
        &self,
        round: Round,
        block: OrderBlock,
        finalization: Finalization<BlsScheme, Digest>,
    ) {
        let mut mailbox = self.clone();
        mailbox.verified(round, block).await;
        Reporter::report(&mut mailbox, Activity::Finalization(finalization)).await;
    }
}

/// Idle cadence of the frozen-tip frontier probe (`ReJump::probe`): one `get_latest`
/// resolver fetch against one peer, well under the frontier-channel quota, skipped
/// while the tip advances via consensus or an inlet.
const FRONTIER_PROBE_INTERVAL: Duration = Duration::from_secs(1);

/// Fast catch-up cadence used while probes are discovering new frontier heights, so
/// the node trails the chain by about one RTT instead of a whole idle tick. 5/s per
/// node, well under the per-peer quota.
const FRONTIER_PROBE_INTERVAL_FAST: Duration = Duration::from_millis(200);

/// Probe ticks that stay fast after the last productive probe (one that hinted a new
/// frontier): 15 ticks ≈ 3 s at the fast cadence, enough to bridge the ~1 blk/s
/// arrival gaps so a continuously-following node does not decay to the idle tick
/// between blocks.
const FRONTIER_PROBE_FAST_BURST: u8 = 15;

/// Backoff between transient engine-API transport retries at the finalize FCU.
/// Short, because an RPC/channel blip clears in ms; a durable stall is reported
/// through `dpos_sync_degraded{reason=engine_retry}` rather than a crash.
const ENGINE_TRANSPORT_RETRY_BACKOFF: Duration = Duration::from_millis(200);

/// How many times the finalized re-apply loop may re-walk on a still-invisible
/// parent before it dies loudly. At `ENGINE_TRANSPORT_RETRY_BACKOFF` per iteration
/// this is a ~10 s budget, and the bound is the point: an unbounded retry here would
/// spin silently.
const REAPPLY_PARENT_VISIBILITY_RETRIES: u32 = 50;

/// How many times the finalized-tier postcondition re-reads the EL before an absent
/// block at a finalized height becomes a corruption verdict. ~10 s at
/// `ENGINE_TRANSPORT_RETRY_BACKOFF` per re-read, because a height devp2p just landed
/// can be invisible for a moment and killing a node that would have healed is the
/// worse error.
const FINALIZED_TIER_VISIBILITY_RETRIES: u32 = 50;

/// Returns the current committee's peers to target for a finalization re-fetch,
/// or `None` if no committee is known yet. Re-invoked per retry so it tracks the
/// catch-up walk's advancing epoch.
pub type PeersForFinalization =
    std::sync::Arc<dyn Fn() -> Option<NonEmptyVec<PeerPubkey>> + Send + Sync>;

/// Steady-state self-healing re-jump callback, invoked from the `Update::Tip` arm
/// when the marshal tip runs more than `ReJump::threshold` finalized blocks ahead of
/// the highest derived ordering height: beyond that window the upstream resolver
/// serves nothing, the marshal floor freezes and the executor wedges. It runs the
/// same forward-only `cold_start_jump::jump_to_target` as the cold-start path,
/// fast-forwarding reth with one FCU and devp2p backfill.
///
/// The jump's generics are erased behind this boxed `Fn` so the executor actor gains
/// no new generic params. The executor spawns the future as a read-only waiter and
/// reacts to its terminal `JumpOutcome` on a `oneshot` `select!` arm; the jump's only
/// reth touch is the read-side `sync_to` FCU, which ancestor-skips when backward, so
/// it cannot corrupt the executor's own forward FCUs.
///
/// `from` is the trigger's `ordering_finalized`; the target is the
/// `(finalization, block)` pair read from this node's own marshal archive, whose only
/// writer is `store_finalization` after `verify_delivered`, so it is already
/// committee-authenticated.
pub type ReJumpFn = std::sync::Arc<
    dyn Fn(
            u64,
            crate::cert_follow::UpstreamFinalized,
        ) -> BoxFuture<'static, crate::cold_start_jump::JumpOutcome>
        + Send
        + Sync,
>;

/// `dpos_frontier_step_unserved_total` counts one probe tick where the tip was frozen,
/// the ladder step `Finalized{last(T+1)}` was requested, and nobody in
/// `committee[T+1]` served it inside the fetch bound. The node is not stuck: it keeps
/// walking contiguously from the floor and the next tick asks again.
const FRONTIER_STEP_UNSERVED: &str = "dpos_frontier_step_unserved_total";

/// `dpos_frontier_step_skipped_total{reason}` counts one probe tick where the ladder
/// step was not put at all: either this node cannot name `committee[T+1]`
/// (`no_tracked_epoch`, `no_geometry`, `out_of_window`, `not_readable`,
/// `read_failed`, `no_participants`) or the marshal would discard it
/// (`at_or_below_the_floor`). The probe then asks `Latest` alone.
pub(crate) const FRONTIER_STEP_SKIPPED: &str = "dpos_frontier_step_skipped_total";

/// What one frontier probe tick produced: the `Latest` answer and the ladder step to
/// name.
///
/// Only the `Latest` fetch is a network call here; the ladder step is named, not
/// fetched, because the marshal resolver is the writer whose deliveries end in
/// `store_finalization`, and fetching here would make this file a second writer of the
/// same finalization.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProbeOutcome {
    /// The height of a `Latest` answer that passed `deliver`, or `None` when the probe
    /// was not served or the answer was refused as unauthenticated.
    ///
    /// Its one consumer is `hint_finalization(frontier)` when the height stands above
    /// the marshal tip — a hint the marshal answers with its own fetch and
    /// `verify_delivered` before storing anything, so an unauthenticated height buys
    /// one by-height fetch and nothing else.
    pub frontier: Option<Height>,
    /// `(last(T+1), committee[T+1])` — the ladder step and its addressees. `None` when
    /// this node cannot name them yet: no epoch tracked, no frozen geometry, or
    /// `committee[T+1]` unreadable at this anchor. The probe then asks `Latest` alone.
    pub step: Option<(Height, NonEmptyVec<PeerPubkey>)>,
}

/// Erased frontier probe (see `ReJump::probe`): asks the upstream for `Latest` and
/// names the ladder step for the tracked epoch `T`. The argument is `T`; `None` before
/// any epoch is tracked, the one state with no step to take.
///
/// The returned `Latest` height is trusted height-only: the answer passed
/// `FrontierHandler`'s `deliver` checks, so an inflated tip is refused at the channel
/// rather than at the end of a wasted backfill.
pub type FrontierProbeFn =
    std::sync::Arc<dyn Fn(Option<u64>) -> BoxFuture<'static, ProbeOutcome> + Send + Sync>;

/// Erased "which epoch did this node last hand to `track`" — `T`, read from the
/// process's `EpochTransition::last_tracked_epoch`, which advances only once the
/// boundary trigger has been delivered.
///
/// `None` until the first epoch is tracked. Synchronous and cheap: it reads a cell the
/// transition's bridge forwarder writes, never the transition's async mutex, so a probe
/// tick cannot block on the epoch machine it is asking about.
pub type TrackedEpochFn = std::sync::Arc<dyn Fn() -> Option<u64> + Send + Sync>;

/// Erased "this node's history now begins here" publisher: raises `EpochTransition`'s
/// read-height floor to the `u64` argument. Awaited rather than spawned because the
/// floor must be in place before the entry's first committee read and the state machine
/// sits behind an async mutex, so a spawn would race the entry it unblocks.
pub type BoundaryReadFloorFn = std::sync::Arc<dyn Fn(u64) -> BoxFuture<'static, ()> + Send + Sync>;

/// The committee module's "my anchor moved" wake-up, erased to the one verb the executor
/// owes it.
///
/// The module reads every epoch committee at `executed_state_hash(anchor)`, where
/// `anchor` is this node's ordering-finalized cursor — the cursor this actor raises — so
/// the executor tells it after each `ExecutedChain::advance_finalized` call. Not calling
/// leaves consumers parked on `CommitteeError::NotReadable` until some other read
/// happens to succeed.
///
/// A one-verb handle rather than the committee itself: the executor must not grow a
/// committee read of its own beside the two tiers it reconciles.
pub type AnchorAdvancedFn = std::sync::Arc<dyn Fn() + Send + Sync>;

/// The steady-state re-jump callback bundled with the signal its trigger reads.
#[derive(Clone)]
pub struct ReJump {
    /// The forward-only jump onto a target read from this node's own marshal archive;
    /// see the callback notes above.
    pub call: ReJumpFn,
    /// Forward-only re-jump need-gate, mirrored into the spawned jump's own gate
    /// (`jump_to_target`). The defer deadlock is epoch-relative, so the recovery gate
    /// is too: `min(JUMP_THRESHOLD, epoch_block_interval)`, which keeps the 1024-block
    /// serving-window size on production epochs and lets a compressed test epoch heal
    /// within an epoch instead of after a fixed 1024-block gap.
    pub threshold: u64,
    /// The inlet's upstream-rotation escape, the same `CertUpstream::rotate_callback()`
    /// the data-fault inlet uses. Fired only on a `Stalled` outcome, and only after
    /// `MAX_UPSTREAM_FAULTS` consecutive ones so an honest transient stall does not
    /// instantly rotate. `InvalidTarget` does not rotate: its target is this node's own
    /// attested archive pair, so the contradiction is local corruption. `None` in tests
    /// and no-rotate configs.
    pub rotate: Option<crate::cert_inlet::RotateUpstream>,
    /// Upstream frontier-discovery probe, fired from the probe tick whenever
    /// `last_tip_height` did not advance since the previous tick. It is the live-follow
    /// driver for a validator with no cert-inlet: a rotated-out validator takes part in
    /// no consensus, so without the probe its marshal tip freezes at the demotion
    /// boundary and it wedges silently.
    ///
    /// Everything the probe produces lands on the marshal: the ladder step
    /// `Finalized{last(T+1)}` addressed at `committee[T+1]`, and
    /// `hint_finalization(frontier)` when the answered `Latest` stands above the tip.
    /// The marshal fetches, verifies and stores, and the normal `Update::Tip` pipeline
    /// walks the gap. An advancing tip skips the network probe entirely. `None` in unit
    /// tests.
    pub probe: Option<FrontierProbeFn>,
    /// `T` for the ladder step the probe takes — see [`TrackedEpochFn`]. `None` in unit
    /// tests and wherever no `EpochTransition` is wired, leaving the probe to ask
    /// `Latest` alone.
    pub tracked_epoch: Option<TrackedEpochFn>,
}

/// A finalized block parked by guard #2: the node is ≥ K behind (`last_tip >=
/// h + K`) but the committee-attested body at `h + K` is not backfilled yet, so the
/// convergence check cannot run. The seed is already resolved, so a re-poke re-derives
/// with no lookups, and the `pending_finalizations` drain is paused while this is
/// `Some`, preserving strict derive order under the marshal's `MAX_PENDING_ACKS`
/// backpressure.
///
/// Re-poked by the marshal's `Update::Tip`/`Update::Block` stream and the FCU
/// heartbeat; there is no wall-clock give-up.
struct Deferred {
    cause: Span,
    order: OrderBlock,
    ack: Exact,
    /// The resolved σ for this block's own round, retained so `repoke_deferred` is a
    /// plain "is `h + K`'s body here yet" retry.
    seed: Option<crate::beacon::Seed>,
}

/// A finalized block held for σ of its own round — the executor's only hold, not a
/// park: no gauge, no hint, no re-poke, no deadline. Its sole exit is σ arriving on
/// the seed-record notify.
struct HeldForSeed {
    cause: Span,
    order: OrderBlock,
    ack: Exact,
    /// When the block first entered the hold, preserved across every miss re-hold so
    /// the stall detector measures the age of the hold rather than of the last failed
    /// lookup; a notify storm resetting it would silence the detector when it matters
    /// most.
    since: SystemTime,
    /// This hold has already been reported: the warn and counter fire once per hold,
    /// not once per heartbeat, since a stall outliving its threshold is one event and
    /// repeating it would bury the log it exists to make readable.
    reported: bool,
}

/// How long a block may sit in the awaiting-seed hold before the detector reports it.
/// Not a deadline — nothing derives, skips or aborts when it elapses; it only decides
/// when a stall becomes visible.
///
/// 60 s is 60 ordering blocks at the 1 blk/s target, far longer than any honest hold: σ
/// is filed from the same finalization certificate that makes the marshal dispatch the
/// body, so the record-vs-delivery race is one certificate wide, and the slowest
/// legitimate case (a follower whose σ stays `Pending` until `PK_epoch` lands) is bounded
/// by one artifact fetch. It is also far below the re-jump horizon, so a stall is named
/// as a seed hold before a deep-gap jump can paper over it.
const SEED_HOLD_STALL_THRESHOLD: Duration = Duration::from_secs(60);

/// What `seed_at_own_round` found for a height's own agreed round.
enum OwnRoundSeed {
    /// The beacon is active in this height's epoch and σ is in the store.
    Present(crate::beacon::Seed),
    /// The beacon is not mandatory in this height's epoch (or the epocher cannot name
    /// it): the agreed derivation is `None` and no σ can change that.
    Inactive,
    /// Beacon-active, σ not recorded yet — the block must wait, never fall back.
    Missing,
}

/// Result of attempting to derive a finalized block.
enum DeriveOutcome {
    /// Derived, imported, FCU'd and acked.
    Done,
    /// Guard #2 could not run: the node is ≥ K behind but the committee-attested body
    /// at `height + K` is not backfilled yet. The park payload (block, ack and the
    /// already-resolved σ) is handed back to be parked and re-poked event-driven; boxed
    /// to keep the hot `Done` arm small.
    NeedAttestation(Box<Deferred>),
    /// The gap-walk's canonicalization FCU could not land: reth answers syncing while a
    /// backfill holds the engine exclusively, which the cold-start jump starts. The
    /// parent stays invisible, so the derive cannot proceed and must not be fatal: park
    /// and re-poke. Same payload as [`Self::NeedAttestation`], a distinct variant because
    /// the fresh-park side effects differ — nothing is missing from the archive here, so
    /// no `h + K` hint.
    NeedParentVisible(Box<Deferred>),
    /// A gap-walk prefix element sits on a beacon-active round whose σ is not in the
    /// store yet. The payload carries the delivered height's σ, never the prefix
    /// element's: the prefix lookup re-runs inside the walk on every re-poke, so the
    /// missing value is re-resolved rather than carried forward.
    NeedPrefixSeed(Box<Deferred>),
}

/// What the run loop must do after `dispatch_fault` disposed of a fault. A `ForkSafety`
/// fault never produces one: the router parks forever instead of returning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Disposition {
    /// Transient / deferred: keep looping.
    Continue,
    /// Idiosyncratic local corruption: `break` — the supervisor aborts-all.
    Shutdown,
}

/// `true` for the typed parent-visibility failure, wherever it sits in the
/// `wrap_err` chain the walk builds around it.
fn is_parent_not_visible(error: &eyre::Report) -> bool {
    error
        .chain()
        .any(|e| e.is::<crate::application::ParentHeaderMissing>())
}

/// `true` for the typed prefix-σ failure, wherever it sits in the `wrap_err`
/// chain the walk builds around it.
fn is_prefix_seed_missing(error: &eyre::Report) -> bool {
    error
        .chain()
        .any(|e| e.is::<crate::application::PrefixSeedMissing>())
}

pub struct Config<BE, D, XC, MarshalMailbox> {
    pub beacon_engine: BE,
    pub deriver: D,
    pub executed: XC,
    pub marshal: MarshalMailbox,
    pub fcu_heartbeat_interval: Duration,
    pub last_consensus_finalized_height: Height,
    pub last_execution_finalized_height: u64,
    pub initial_finalized: (Height, B256),
    pub initial_head: (Height, B256),
    /// The marshal floor this node boots with, matching the value `outer.rs` sends in
    /// its buffered `SetFloor`. Seeds the stale-dispatch guard so it is live from tick
    /// zero rather than from the first `reseed_forward`.
    pub initial_marshal_floor: u64,
    /// Authenticated by-height seam for boundary seeding at `reseed_forward`.
    pub boundary_fetch: Option<crate::cert_follow::BoundaryFetchFn>,
    /// Epoch-entry seam (see [`crate::outer::OuterBuilder::boundary_enter`]), invoked
    /// once per successful re-jump landing.
    pub boundary_enter: std::sync::Arc<dyn Fn(u64) + Send + Sync>,
    /// Read-floor seam (see [`crate::outer::OuterBuilder::boundary_read_floor`]), awaited
    /// immediately before [`Self::boundary_enter`] on a re-jump landing.
    pub boundary_read_floor: BoundaryReadFloorFn,
    /// Chain-wide sequencer→DPoS activation block — the origin of the `result_target`
    /// pre-activation window (`height < activation + K` requires a zero result). A chain
    /// constant, not this node's cold-start anchor: a deep-catch-up follower
    /// trust-anchors above activation yet still derives the K below-anchor blocks, which
    /// are post-activation and carry real results. Keying the cross-check on the anchor
    /// would reject the chain.
    pub dpos_activation_block: u64,
    pub fcu_pace: Duration,
    pub peers_for_finalization: PeersForFinalization,
    /// The randomness handle. The executor reads three operations off it:
    /// [`crate::beacon::Beacon::mandatory_at`], the network-agreed "is the beacon
    /// active in this epoch" gate on every seed lookup;
    /// [`crate::beacon::Beacon::seed`], to re-canonicalise the speculative seed round
    /// to the block's own `proposal_view` and to resolve the finalized derive's round;
    /// and [`crate::beacon::Beacon::subscribe`], to wake when a seed lands. A provider
    /// with no seeds degrades to skipping speculation and holding the tip until σ
    /// arrives, never to speculating with a known-wrong seed.
    pub randomness: std::sync::Arc<dyn crate::beacon::Beacon>,
    /// Cross-epoch block→epoch map. Used to form `h`'s own seed round
    /// `Round(epocher.containing(h).epoch(), h.proposal_view)` — a pure function of
    /// agreed data, so every honest node resolves the identical σ and a wrong epoch can
    /// only miss, never yield a wrong seed.
    pub epocher: crate::epocher::OriginEpocher,
    /// The committee module's anchor wake-up (see [`AnchorAdvancedFn`]). Called at every
    /// site that raises the finalized-execution cursor.
    pub anchor_advanced: AnchorAdvancedFn,
    /// The executor's own counters (`seed_active` / `digest_fallback`), one increment
    /// per derived block.
    pub metrics: ExecutorMetrics,
    /// Self-heal observability handle. The executor raises
    /// `dpos_sync_degraded{reason=engine_retry}` while retrying a transient engine-API
    /// transport error at the finalize FCU, and clears `{reason=crash_recover}` when the
    /// startup backfill drain finishes — the `RecoverOutcome::DeferToElSync` cold start
    /// anchors at reth's tip and defers closing its EL gap to that drain.
    pub sync_metrics: SyncMetrics,
    /// Fork-safety latch. The executor engages it on result divergence, an EL
    /// `Ok(Invalid)` verdict, and an L1-fork re-jump, halting instead of extending a
    /// rejected branch. Engaging stops the executor driving reth and demotes the node
    /// to verify-only permanently, while marshal and `consensus`-RPC stay alive (the
    /// supervisor parks rather than aborting all).
    pub safety_halt: crate::sync_metrics::SafetyHalt,
    /// Fired on every ordering-finalized advance so `epoch_manager` can re-poke a
    /// per-epoch engine spawn parked on the `Inline::genesis(E)` precondition: the E-1
    /// boundary block landing in marshal storage is an executor finalized-advance.
    /// Event-driven, no clock poll.
    pub spawn_unblocked: std::sync::Arc<tokio::sync::Notify>,
    /// Steady-state self-healing re-jump (see [`ReJump`]). `None` for a plain validator,
    /// which catches up on the consensus-plane treadmill, and in tests that do not
    /// exercise it.
    pub re_jump: Option<ReJump>,
}

/// The two counters the executor owns: one per derived block, saying whether
/// `prev_randao` was the verified threshold seed or the digest fallback.
///
/// Registered on both node classes, because the executor runs on both and cannot tell
/// which it is on; making them class-specific would register them twice on every
/// follower, which `prometheus_client::Registry` accepts silently.
#[derive(Clone, Debug, Default)]
pub struct ExecutorMetrics {
    /// A block's `prev_randao` was the verified threshold seed (`assurance=true`).
    pub seed_active: prometheus_client::metrics::counter::Counter,
    /// A beacon-active block fell back to `order.digest()` (seed absent or failed
    /// σ-verify vs `PK_E`). A beacon-active boundary is nullified before it finalizes,
    /// so this counts the local pre-nullify observation on a node that derived ahead of
    /// it; it is 0 post-anchor on a healthy chain.
    pub digest_fallback: prometheus_client::metrics::counter::Counter,
}

impl ExecutorMetrics {
    /// Register both counters. Call once per process, against the same context the other
    /// metric structs use: commonware prefixes each family with the context's label path,
    /// so a labelled child context would rename them in the scrape.
    pub fn register(&self, ctx: &impl commonware_runtime::Metrics) {
        ctx.register(
            "beacon_seed_active_total",
            "Blocks whose prev_randao was the verified threshold seed.",
            self.seed_active.clone(),
        );
        ctx.register(
            "beacon_digest_fallback_total",
            "Beacon-active blocks that fell back to order.digest() (seed absent/unverified). \
             0 post-anchor on a healthy chain.",
            self.digest_fallback.clone(),
        );
    }
}

pub struct Actor<E, BE, D, XC, MarshalMailbox> {
    context: ContextCell<E>,
    beacon_engine: BE,
    deriver: D,
    executed: XC,
    marshal: MarshalMailbox,
    mailbox: mpsc::UnboundedReceiver<Message>,
    metrics: ExecutorMetrics,
    /// Self-heal stuck-detector (see [`Config::sync_metrics`]).
    sync_metrics: SyncMetrics,
    /// Fork-safety latch (see [`Config::safety_halt`]).
    safety_halt: crate::sync_metrics::SafetyHalt,
    spawn_unblocked: std::sync::Arc<tokio::sync::Notify>,
    /// Steady-state self-healing re-jump callback (see [`ReJump`]).
    re_jump: Option<ReJump>,
    /// Consecutive re-jump `Stalled` outcomes since the last reset; at
    /// `MAX_UPSTREAM_FAULTS` the executor rotates and resets. Reset also on
    /// `Landed`/`Lagging`, so a streak accrued on one upstream never carries into the
    /// next (cross-upstream carryover would oscillate between them). A second,
    /// independent streak from the inlet's data-fault counter feeds the same `rotate()`
    /// sink.
    rejump_fault_streak: u32,
    /// Completion channel of the in-flight spawned re-jump waiter; its terminal
    /// `JumpOutcome` is consumed in a dedicated `select!` arm. The waiter is spawned as
    /// a read-only `sync_to` wait, so the executor's loop stays responsive during the
    /// multi-minute backfill.
    jump_done: OptionFuture<oneshot::Receiver<crate::cold_start_jump::JumpOutcome>>,
    /// Handle of the spawned re-jump waiter, aborted on shutdown so the spawned
    /// `sync_to` wait does not outlive the executor task.
    jump_handle: Option<Handle<()>>,
    /// Highest marshal-frontier height observed via `Update::Tip`. The FCU heartbeat
    /// re-pokes `maybe_re_jump` with this height so a stalled re-jump is re-evaluated
    /// even when the upstream frontier has plateaued and sends no further `Update::Tip`;
    /// without it, a plateaued frontier is a silent permanent wedge.
    last_tip_height: Height,

    /// The running marshal floor (landing − K), mirrored from the last
    /// `reseed_forward`'s `set_floor`; 0 on non-jump paths. Guards the `Update::Block`
    /// arm against a stale-backlog escape: `set_floor` is fire-and-forget, so a slot
    /// freed by a disposal can dispatch an old-range block before the marshal processes
    /// `SetFloor`, and deriving it against `db_tip = landing` is the deep-overlay walk
    /// the jump exists to avoid. Such `≤ floor` deliveries are acked without deriving.
    ///
    /// Keyed strictly on the marshal floor, never on `anchor`/`safe_height`: the
    /// legitimate below-safe deliveries in the `anchor − K + 1 ..= anchor` deep-catch-up
    /// window must still derive.
    marshal_floor: u64,
    /// Authenticated by-height seam for seeding an epoch-boundary block the floor is
    /// about to bury (`reseed_forward`); `None` without an upstream. Used by
    /// `seed_boundary_below_floor` to locate the epoch terminal a floor raise would
    /// bury; the `epocher` field supplies the geometry, so seeding cannot disagree with
    /// the gate it satisfies.
    boundary_fetch: Option<crate::cert_follow::BoundaryFetchFn>,
    /// Epoch-entry seam — see [`crate::outer::OuterBuilder::boundary_enter`].
    boundary_enter: std::sync::Arc<dyn Fn(u64) + Send + Sync>,
    /// Read-floor seam — see [`crate::outer::OuterBuilder::boundary_read_floor`].
    boundary_read_floor: BoundaryReadFloorFn,

    last_canonicalized: LastCanonicalized,
    /// Highest ordering-finalized height processed; drives the result-final cursor
    /// (`− K`, clamped to the anchor). Restart-seeded from the marshal's durable acked
    /// cursor, not the reth head, which is a speculative tip that may carry a nullified
    /// sibling above the ack.
    ordering_finalized: u64,
    /// Anchor floor for the finalized cursor: the cold-start finalized point
    /// is result-final by construction (committee-external trust root).
    anchor_finalized: (Height, B256),
    /// Chain-wide activation block for the `result_target` pre-activation window (see
    /// [`Config::dpos_activation_block`]). Coincides with `anchor_finalized` only on the
    /// fresh-migration signer path.
    dpos_activation_block: u64,

    fcu_heartbeat_interval: Duration,
    fcu_heartbeat_timer: Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
    fcu_pace: Duration,

    /// Tick driving [`ReJump::probe`]. Armed unconditionally; the arm no-ops when no
    /// probe is wired or the tip advanced. Cadence is the fast interval while
    /// `probe_fast_left > 0`, else the idle one.
    frontier_probe_timer: Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
    /// `last_tip_height` snapshot at the previous probe tick: if the tip advanced since,
    /// the marshal is live and the network probe is skipped.
    probe_prev_tip: Height,
    /// A ladder step was put on the previous probe tick and the tip has not moved
    /// since. It only decides whether `dpos_frontier_step_unserved_total` ticks: a step
    /// is unserved exactly when it was asked for, a whole tick passed, and the tip stayed
    /// frozen. Not a state machine — the bit is a metric's memory.
    probe_step_pending: bool,
    /// Fast-cadence hysteresis: reset to [`FRONTIER_PROBE_FAST_BURST`] on every
    /// productive probe (one that hinted a new frontier) and decremented per tick
    /// otherwise. While non-zero the probe fires every tick even if the tip just
    /// advanced, since that advance was the probe's own delivery, so a demoted follower
    /// trails by about one RTT.
    probe_fast_left: u8,

    finalized_heights_to_backfill: RangeInclusive<u64>,
    pending_backfill: OptionFuture<BoxFuture<'static, (u64, Option<OrderBlock>)>>,
    pending_finalizations: FuturesOrdered<Ready<(Span, OrderBlock, Exact)>>,

    /// Gauge for `pending_finalizations.len()`. Sustained values above 4 mean the EL is
    /// falling behind consensus (`MAX_PENDING_ACKS = 16` is the marshal-side ceiling).
    pending_finalizations_gauge: Gauge<i64>,

    /// Height of the block currently parked by guard #2 awaiting the committee-attested
    /// `h + K` body (0 = none). Set on a fresh park, reset on derive / re-jump disposal;
    /// 0 in steady state, since the pipeline holds the tip rather than parking it. A
    /// constant non-zero value is the durable-park alert signal; "how long" lives in the
    /// alert, not in an in-process counter.
    deferred_height: Gauge<i64>,

    /// Heartbeat FCUs are suppressed until consensus advances from the
    /// cold-start snapshot, so a stale initial head is never re-sent over a
    /// canonical chain that moved without us.
    has_advanced_since_init: bool,

    /// Highest height the executor has imported, speculatively or finalized. Speculation
    /// fires only for `spec_head + 1`; tracked here rather than via `executed_tip()` to
    /// avoid reth's `best_number` lag race.
    spec_head: u64,
    /// Heights speculatively executed at notarization but not yet finalized (see
    /// [`SpecExecuted`]). On finalized delivery, matching digest and seed round means the
    /// speculation was correct, so the derive is skipped and the head lead kept; a digest
    /// mismatch (nullified sibling finalized) or a seed-round mismatch forces a re-derive
    /// and head reorg.
    spec_executed: BTreeMap<u64, SpecExecuted>,
    /// Notarized speculative blocks that arrived ahead of `spec_head` or before their
    /// parent executed, keyed by height and parked rather than dropped. Re-driven by
    /// [`Self::try_drain_parked`] on the next `spec_head` advance, so speculation resumes
    /// after a transient fall-behind. Bounded to about K by the drain's leading
    /// `split_off`; entries ≤ `spec_head` are already executed or finalized.
    parked_spec: BTreeMap<u64, ParkedSpec>,

    peers_for_finalization: PeersForFinalization,
    /// A finalized block parked by guard #2 (see [`Deferred`]), held with its `Exact`
    /// ack and resolved σ; the `pending_finalizations` drain is paused while this is
    /// `Some`, preserving strict order. Re-poked off the marshal's delivery stream and
    /// the FCU heartbeat — the heartbeat is the completeness backstop, since a body
    /// landing at `height <= tip` fires no `Update::Tip`. No give-up timer.
    deferred: Option<Deferred>,

    /// The delivered, not-yet-derived finalized block whose σ has not landed yet.
    /// Beacon-active rounds only: an inactive round derives `None` immediately and
    /// never reaches this slot. It is held, not parked (no gauge, no hint, no re-poke, no
    /// deadline), and derived by the seed-record notify arm the moment σ lands — the only
    /// exit. Both block feeders, the `pending_finalizations` drain and the
    /// startup-backfill walk, are gated on this slot being empty, which keeps a second
    /// delivery from overwriting a live `Exact`.
    ///
    /// On a re-jump the held ack is acked `Ok` — the floor moves, so the height is
    /// pruned, not skipped; on shutdown it is dropped deliberately, never acked, and that
    /// withheld ack is the restart self-heal (see the module doc). On a `SafetyHalt` it
    /// joins `park_halted`'s retained set.
    awaiting_seed: Option<HeldForSeed>,

    /// See [`Config::randomness`]. Read by `spec_execute`'s round re-canonicalisation and
    /// by [`Self::seed_at_own_round`], the sole seed source of the finalized derive.
    randomness: std::sync::Arc<dyn crate::beacon::Beacon>,

    /// See [`Config::epocher`]. Read only by [`Self::seed_at_own_round`] to form `h`'s own
    /// agreed seed round.
    epocher: crate::epocher::OriginEpocher,

    /// See [`Config::anchor_advanced`]. Called immediately after each site that raises
    /// the finalized-execution cursor.
    anchor_advanced: AnchorAdvancedFn,

    /// The `Exact` ack of the block currently inside `try_derive`, moved here at entry
    /// and taken back at every non-`Err` exit. On an `Err` exit it stays here rather than
    /// being dropped in `try_derive`'s frame, so a `SafetyHalt` reaches `park_halted`
    /// with the ack alive and retainable (the module doc's ack invariant). `None` whenever
    /// `try_derive` is not on the stack.
    inflight_ack: Option<Exact>,
}

impl<E, BE, D, XC, MarshalMailbox> Actor<E, BE, D, XC, MarshalMailbox>
where
    E: Clock + commonware_runtime::Metrics + Spawner + Send + 'static,
    BE: BeaconEngineLike<ExecutionData = D::Derived> + Send + Sync + 'static,
    D: DerivedBlockBuilder,
    XC: ExecutedChain,
    MarshalMailbox: BlockFetcher,
{
    pub fn init(context: E, cfg: Config<BE, D, XC, MarshalMailbox>) -> (Self, Mailbox) {
        let (tx, rx) = mpsc::unbounded_channel();
        let mailbox = Mailbox::new(tx);

        let fcu_heartbeat_timer = Box::pin(context.sleep(cfg.fcu_heartbeat_interval));
        let frontier_probe_timer = Box::pin(context.sleep(FRONTIER_PROBE_INTERVAL));

        let finalized_heights_to_backfill =
            (cfg.last_execution_finalized_height + 1)..=cfg.last_consensus_finalized_height.get();

        // Finalized-execution cursor (restart seed): seed from the marshal's durable
        // acked cursor (`last_consensus_finalized_height` = marshal
        // `last_processed_height`), not the reth head. Every acked height is
        // consensus-final and passed `try_derive`'s canonical postcondition before its
        // ack persisted, so the provider's canonical hash there is the finalized hash.
        //
        // The reth head is unsound as the seed: under deferred execution `spec_execute`
        // advances the head at notarization latency and a clean shutdown persists it, so
        // heights in `(acked, head]` are notarized-only and a sibling is still possible
        // (notarize A → nullify → finalize B). A restart straddling that race, seeded
        // from the head, could serve the orphaned sibling as `finalized_executed_hash`
        // to a propose before the marshal re-reconciles the height.
        //
        // `ordering_finalized` is seeded from the same acked height for the same reason:
        // seeded from the reth head it would inherit that tail and could pin the
        // engine-API `finalized` onto an orphaned sibling. The `(acked, head]` derives
        // are reconstructed idempotently by the marshal-driven backfill and the
        // `update_head` reconcile, while the head is left at the speculative tip.
        cfg.executed
            .advance_finalized(cfg.last_consensus_finalized_height.get());
        // The committee module anchors on the cursor just seeded, so tell it after the
        // seed above: the first reads of this process are then taken at the restart
        // anchor rather than at height 0.
        (cfg.anchor_advanced)();

        let pending_finalizations_gauge = Gauge::<i64>::default();
        context.register(
            "pending_finalizations",
            "Count of finalized blocks awaiting derive+import+ack in the executor queue \
             (MAX_PENDING_ACKS=16 marshal-side ceiling).",
            pending_finalizations_gauge.clone(),
        );

        let deferred_height = Gauge::<i64>::default();
        context.register(
            "deferred_height",
            "Height of the finalized block PARKED awaiting its finalization cert (0 = none \
             parked). A sustained non-zero value flags a validator whose cert fetch is durably \
             stuck (Prometheus alert: deferred_height != 0 for > Xm); the gap-gated re-jump \
             auto-recovers every non-isolated park, so it fires only on true isolation.",
            deferred_height.clone(),
        );

        let actor = Self {
            context: ContextCell::new(context),
            beacon_engine: cfg.beacon_engine,
            deriver: cfg.deriver,
            executed: cfg.executed,
            marshal: cfg.marshal,
            mailbox: rx,
            metrics: cfg.metrics,
            sync_metrics: cfg.sync_metrics,
            safety_halt: cfg.safety_halt,
            spawn_unblocked: cfg.spawn_unblocked,
            re_jump: cfg.re_jump,
            randomness: cfg.randomness,
            epocher: cfg.epocher,
            anchor_advanced: cfg.anchor_advanced,
            rejump_fault_streak: 0,
            jump_done: OptionFuture::default(),
            jump_handle: None,
            // Best estimate of the marshal frontier at startup, refined by every
            // `Update::Tip`.
            last_tip_height: cfg.last_consensus_finalized_height,
            // Seeded from the same value `outer.rs` seeds the marshal with, never 0: the
            // stale-dispatch guard keys on this field, and the marshal dispatches blocks
            // before processing its buffered `SetFloor`, so a 0 here would leave the
            // guard inert exactly while old-range blocks can escape.
            marshal_floor: cfg.initial_marshal_floor,
            boundary_fetch: cfg.boundary_fetch,
            boundary_enter: cfg.boundary_enter,
            boundary_read_floor: cfg.boundary_read_floor,
            last_canonicalized: LastCanonicalized {
                forkchoice: ForkchoiceState {
                    head_block_hash: cfg.initial_head.1,
                    // At cold start there is no ordering-final tip above the anchor yet,
                    // so safe == finalized == head == anchor.
                    safe_block_hash: cfg.initial_finalized.1,
                    finalized_block_hash: cfg.initial_finalized.1,
                },
                head_height: cfg.initial_head.0,
                safe_height: cfg.initial_finalized.0,
                finalized_height: cfg.initial_finalized.0,
            },
            ordering_finalized: cfg.last_consensus_finalized_height.get(),
            anchor_finalized: cfg.initial_finalized,
            dpos_activation_block: cfg.dpos_activation_block,
            fcu_heartbeat_interval: cfg.fcu_heartbeat_interval,
            fcu_heartbeat_timer,
            fcu_pace: cfg.fcu_pace,
            frontier_probe_timer,
            probe_prev_tip: cfg.last_consensus_finalized_height,
            probe_step_pending: false,
            probe_fast_left: 0,
            finalized_heights_to_backfill,
            pending_backfill: OptionFuture::default(),
            pending_finalizations: FuturesOrdered::new(),
            pending_finalizations_gauge,
            deferred_height,
            has_advanced_since_init: false,
            spec_head: cfg.initial_head.0.get(),
            spec_executed: BTreeMap::new(),
            parked_spec: BTreeMap::new(),
            peers_for_finalization: cfg.peers_for_finalization,
            deferred: None,
            awaiting_seed: None,
            inflight_ack: None,
        };
        (actor, mailbox)
    }

    pub fn start(mut self) -> Handle<()> {
        spawn_cell!(self.context, self.run().await)
    }

    /// Test-only snapshot of the seed fields `reseed_forward` and `init` must
    /// agree on at a landing. `dpos_activation_block` is excluded: it is a
    /// chain constant `reseed_forward` never touches, so a follower whose
    /// activation differs from its anchor keeps its own value.
    #[cfg(test)]
    fn seed_fields(&self) -> (u64, (Height, B256), Height, Height, u64) {
        (
            self.ordering_finalized,
            self.anchor_finalized,
            self.last_canonicalized.safe_height,
            self.last_canonicalized.finalized_height,
            self.spec_head,
        )
    }

    async fn run(mut self) {
        info_span!("start").in_scope(|| info!("executor starting"));

        // Subscribe before the loop and before this actor's first seed read: a
        // `broadcast` buffers only from the subscription onward, so an event
        // fired before it would be lost. Held as a local so the arm's future
        // never borrows `self`, which the arm's `&mut self` body needs.
        let mut beacon_events = self.randomness.subscribe();
        // Set when the beacon's sender is gone (`RecvError::Closed`).
        let mut beacon_events_closed = false;

        loop {
            // `dispatch_fault` reads the latch only when a fault reaches it, so a
            // latch engaged with no fault in flight — the datadir marker restored
            // at startup — would leave this actor driving reth. Park before
            // pulling any work, retaining every marshal ack.
            if self.safety_halt.is_engaged() {
                self.park_halted(
                    "halt latch engaged before dispatch",
                    eyre::eyre!("SafetyHalt latch engaged; executor parking without deriving"),
                )
                .await;
            }

            // Do not pull more work while a block is deferred awaiting its h+K
            // attested body (the deferred block must derive first), nor while a
            // block is held awaiting its σ (same order, and `on_finalized_block`
            // would otherwise overwrite a live `Exact`), nor while a jump is in
            // flight (the jump is the single EL writer during backfill; a
            // competing startup-drain FCU retargets reth's backfill).
            if self.deferred.is_none()
                && self.awaiting_seed.is_none()
                && self.pending_backfill.is_none()
                && self.jump_done.is_none()
            {
                if let Some(height) = self.finalized_heights_to_backfill.next() {
                    let marshal = self.marshal.clone();
                    self.pending_backfill.replace(
                        async move {
                            (
                                height,
                                marshal.fetch_block_by_height(Height::new(height)).await,
                            )
                        }
                        .boxed(),
                    );
                }
            }

            select! {
                biased;

                (height, maybe_block) = &mut self.pending_backfill => {
                    match maybe_block {
                        Some(block) => {
                            // Synthetic ack: the marshal already acked these
                            // heights on a previous run. Routes through the same
                            // resolve-and-derive path as live dispatch.
                            let (ack, _waiter) = Exact::handle();
                            let span = info_span!("backfill_on_start", %height);
                            if let Err(fault) = self.on_finalized_block(span, block, ack).await {
                                if self.dispatch_fault("backfill", fault).await
                                    == Disposition::Shutdown
                                {
                                    break;
                                }
                            } else if self.finalized_heights_to_backfill.is_empty()
                                && self.sync_metrics.degraded_value(SyncReason::CrashRecover) == 1
                            {
                                // The crash-recover deferral ends here, at the last
                                // drained height, not at a jump landing: this drain
                                // is what walks `(reth tip .. marshal cursor]` back
                                // into reth through the live derive+import path.
                                // `maybe_re_jump` cannot do it and never fires at
                                // boot — `last_tip_height` and `ordering_finalized`
                                // are seeded from the same value, so the gap is 0,
                                // and its gate refuses while this drain is non-empty.
                                self.sync_metrics.recover(SyncReason::CrashRecover);
                                self.sync_metrics.crash_recover_gap_blocks.set(0);
                            }
                        }
                        None => {
                            // Every height this drain asks for is at or below the
                            // marshal floor (its acked cursor), and the marshal never
                            // repairs below its floor, so a miss here is permanent,
                            // not transient — the reachable cause is an EL rolled
                            // back below a range this node jumped over and never
                            // stored. The fault is raised at the true site: a skip
                            // would only relocate it to the later gap-walk, which
                            // re-hits the same height and names the wrong one. Routed
                            // rather than `break`n because a bare break skips the
                            // halt-latch read and drops retained marshal `Exact`s into
                            // Canceled, which is fatal.
                            let floor = *self.finalized_heights_to_backfill.end();
                            let fault = Fault::corruption(eyre::eyre!(
                                "the marshal archive has no block at height {height}. Every \
                                 height of this startup drain is at or below the marshal \
                                 floor {floor} (its acked cursor), and the marshal never \
                                 repairs below its floor — so this hole is permanent, not a \
                                 race. reth's tip is below the marshal floor {floor}: the EL \
                                 is older than a range this node jumped over and the \
                                 consensus archive holds no blocks there. Restore an EL \
                                 snapshot at or above {floor}, or delete the consensus \
                                 archive so the node re-enters as ElFinalized. (Skipping \
                                 would fail later in the gap-walk at the WRONG height.)"
                            ));
                            if self.dispatch_fault("backfill", fault).await
                                == Disposition::Shutdown
                            {
                                break;
                            }
                        }
                    }
                    // `OptionFuture` does not auto-clear after `Poll::Ready`, and
                    // the `pending_finalizations` arm guard below depends on
                    // `is_none()`.
                    *self.pending_backfill = None;
                }

                // Terminal outcome of the spawned re-jump waiter, delivered over
                // the `jump_done` oneshot. `OptionFuture` does not auto-clear after
                // `Poll::Ready`; clear it (and its handle) here, then act on the
                // outcome.
                outcome = &mut self.jump_done => {
                    *self.jump_done = None;
                    self.jump_handle = None;
                    match outcome {
                        Ok(crate::cold_start_jump::JumpOutcome::Landed { landing, hash, floor }) => {
                            if let Err(fault) = self.reseed_forward(landing, hash, floor).await {
                                if self.dispatch_fault("re-jump reseed", fault).await
                                    == Disposition::Shutdown
                                {
                                    break;
                                }
                            }
                            self.rejump_fault_streak = 0;
                        }
                        Ok(crate::cold_start_jump::JumpOutcome::Lagging) => {
                            debug!("steady-state re-jump: lagging / stale target — no-op");
                            self.rejump_fault_streak = 0;
                        }
                        Ok(crate::cold_start_jump::JumpOutcome::InvalidTarget(error)) => {
                            // The jump target is the `(finalization, block)` pair
                            // this node read from its own marshal archive, written
                            // only by `store_finalization` after `verify_delivered`,
                            // so it carries 2f+1 under a committee this node read
                            // itself. There is no upstream to rotate away from.
                            // Either cause that reaches here — reth `Invalid`
                            // mid-EL-sync, or `holds(result) == false` after a
                            // `Valid` — leaves the local EL contradicting an
                            // authenticated certificate, which is corruption: loud
                            // actor death, no further EL writes.
                            let fault = Fault::corruption(eyre::eyre!(
                                "steady-state re-jump onto this node's OWN attested \
                                 archive pair did not land on the attested branch \
                                 (reth answered INVALID mid-EL-sync, or reported Valid \
                                 while not holding the attested result canonically): \
                                 {error:#}"
                            ));
                            if self.dispatch_fault("re-jump landing", fault).await
                                == Disposition::Shutdown
                            {
                                break;
                            }
                        }
                        Ok(crate::cold_start_jump::JumpOutcome::Stalled(error)) => {
                            // Transient transport stall: count it rather than
                            // rotating on one bad tick. At `MAX_UPSTREAM_FAULTS`
                            // consecutive stalls the upstream is failed over; the
                            // gap is re-evaluated on the next tip or heartbeat.
                            self.rejump_fault_streak += 1;
                            warn!(
                                error = %format_args!("{error:#}"),
                                faults = self.rejump_fault_streak,
                                "steady-state re-jump stalled (transport); will re-evaluate \
                                 on the next frontier tip"
                            );
                            if self.rejump_fault_streak >= crate::cert_inlet::MAX_UPSTREAM_FAULTS {
                                warn!("re-jump stalled {} times; rotating upstream",
                                    crate::cert_inlet::MAX_UPSTREAM_FAULTS);
                                self.rotate_upstream().await;
                                self.rejump_fault_streak = 0;
                            }
                        }
                        Ok(crate::cold_start_jump::JumpOutcome::StalledWithPeers(error)) => {
                            // reth is connected but its executed head is frozen: the
                            // wedge is local to reth's pipeline, not a bad upstream
                            // branch, so rotating would not help. Bump the counter,
                            // log, and re-arm — the next tip or heartbeat re-spawns
                            // the waiter, and the floor is never advanced onto an
                            // un-synced tip.
                            self.sync_metrics.el_sync_stalled_with_peers.inc();
                            error!(
                                error = %format_args!("{error:#}"),
                                "steady-state re-jump: reth CONNECTED but executed head frozen \
                                 (EL pipeline wedged); staying deferred + observable, will re-arm \
                                 on the next frontier tip (NON-fatal, NOT rotating — the wedge is \
                                 local to reth, not the upstream)"
                            );
                        }
                        Ok(crate::cold_start_jump::JumpOutcome::L1Fork(error)) => {
                            // The EL-synced head does not descend from the
                            // L1-finalized checkpoint, so there is no upstream to
                            // rotate to: L1 finality itself disagrees. Halt (demote
                            // to verify-only, stop driving reth) and wait for the L1
                            // proof + governance recovery.
                            let fault = Fault::fork_safety(
                                SyncReason::L1Fork,
                                eyre::eyre!(
                                    "L1 checkpoint NOT in local chain — SafetyHalt (stop \
                                     participating, stay up): {error}"
                                ),
                            );
                            if self.dispatch_fault("steady-state re-jump", fault).await
                                == Disposition::Shutdown
                            {
                                break;
                            }
                        }
                        Err(_canceled) => {
                            debug!("steady-state re-jump waiter canceled before completion");
                        }
                    }
                }

                Some((cause, block, ack)) = self.pending_finalizations.next(),
                if self.deferred.is_none()
                    // A block held for its σ must derive before the next is
                    // pulled: strict order, and `on_finalized_block`'s slot
                    // assignment would otherwise drop a live `Exact` into Canceled,
                    // which is fatal to the marshal. Queued entries stay alive,
                    // bounded by the marshal's `MAX_PENDING_ACKS`. `maybe_re_jump`
                    // is deliberately not gated here: that gate is what bounds a
                    // σ-less node's stall.
                    && self.awaiting_seed.is_none()
                    && self.pending_backfill.is_none()
                    && self.finalized_heights_to_backfill.is_empty()
                    // The jump is the single EL writer while in flight: a finalize
                    // FCU here would carry a low finalized hash that retargets reth's
                    // backfill away from the jump tip, so the jump's `Valid`
                    // terminator never fires. Gating the drain also means no new
                    // block parks during a jump — the only parked block a jump can
                    // meet is one parked before it spawned, which `reseed_forward`
                    // disposes by acknowledging. `repoke_deferred` is gated the same
                    // way.
                    && self.jump_done.is_none() => {
                    self.pending_finalizations_gauge
                        .set(self.pending_finalizations.len() as i64);
                    if let Err(fault) = self.on_finalized_block(cause, block, ack).await {
                        if self.dispatch_fault("finalize", fault).await == Disposition::Shutdown {
                            break;
                        }
                    }
                }

                msg = self.mailbox.recv() => {
                    let Some(msg) = msg else {
                        // Not a fault: every sender dropped and the node is tearing
                        // this executor down. The latch still governs whether we may
                        // leave — a halted executor holds marshal `Exact` acks, and
                        // returning here drops them into Canceled.
                        if self.safety_halt.is_engaged() {
                            self.park_halted(
                                "mailbox closed",
                                eyre::eyre!("executor mailbox closed while SafetyHalt engaged"),
                            )
                            .await;
                        }
                        // Counted below the park: `park_halted` never returns, so a
                        // halted executor is still alive holding marshal acks, and
                        // reporting an exit would erase the operator's "torn down" vs
                        // "halted but observable" discriminator.
                        metrics::counter!(
                            "dpos_executor_exit_total", "cause" => "mailbox_closed"
                        )
                        .increment(1);
                        info!("executor mailbox closed (all senders dropped) — clean shutdown");
                        break;
                    };
                    if let Err(fault) = self.handle_message(msg).await {
                        if self.dispatch_fault("message", fault).await == Disposition::Shutdown {
                            break;
                        }
                    }
                }

                // Seed-record wake-up arm. Uses the beacon's
                // `broadcast::Sender<BeaconEvent>` and its `SeedRecorded` variant
                // rather than a `Notify`: a broadcast buffers only from the
                // subscription onward and drops a send with no receiver, whereas a
                // `notify_one` permit survived having no waiter. A held tip's own
                // round seed may land after the delivery that missed it, so this
                // arm re-attempts the finalized-tier derive; a spurious wake is a
                // harmless idempotent re-check.
                event = beacon_events.recv(), if !beacon_events_closed
                    && self.awaiting_seed.is_some()
                    && self.deferred.is_none()
                    && self.jump_done.is_none() => {
                    match classify_seed_wake(&event) {
                        SeedWake::Ignore => continue,
                        SeedWake::Disarm => {
                            beacon_events_closed = true;
                            error!(
                                "beacon wake-up channel closed; held tips will only \
                                 derive on delivery from here on"
                            );
                            continue;
                        }
                        SeedWake::Derive => {}
                    }
                    if let Err(fault) =
                        self.try_eager_finalized_derive(EagerTrigger::Notified).await
                    {
                        if self.dispatch_fault("seed-notify eager derive", fault).await
                            == Disposition::Shutdown
                        {
                            break;
                        }
                    }
                }

                _ = (&mut self.fcu_heartbeat_timer).fuse() => {
                    if let Err(fault) = self.send_forkchoice_update_heartbeat().await {
                        if self.dispatch_fault("fcu heartbeat", fault).await
                            == Disposition::Shutdown
                        {
                            break;
                        }
                    }
                    // Re-evaluate the steady-state re-jump on the heartbeat tick: a
                    // `Stalled` retry otherwise depends on the next tip, and if the
                    // upstream frontier has plateaued while reth's backfill is
                    // stalled no further tip arrives, leaving a silent permanent
                    // wedge.
                    let _ = self.maybe_re_jump(self.last_tip_height).await;
                    // Delivery-independent re-poke of a parked block: a cert landing
                    // at a height at or below the tip fires no tip update, so a
                    // delivery-only re-poke would deadlock on the last catch-up
                    // block. Reuses this tick; a still-missing body just re-stays
                    // parked.
                    if let Err(fault) = self.repoke_deferred().await {
                        if self.dispatch_fault("deferred re-poke", fault).await
                            == Disposition::Shutdown
                        {
                            break;
                        }
                    }
                    // Observation only: reads the clock on this existing tick and
                    // cannot complete, abort or re-key the hold.
                    self.detect_stalled_seed_hold();
                    self.reset_fcu_heartbeat_timer();
                }

                // Frontier probe: the live-follow driver for a validator with no
                // cert-inlet. A productive probe (hinted a new frontier) arms the
                // fast-cadence burst so an actively-following demoted node trails by
                // about one RTT.
                _ = (&mut self.frontier_probe_timer).fuse() => {
                    if self.probe_frontier().await {
                        self.probe_fast_left = FRONTIER_PROBE_FAST_BURST;
                    } else {
                        self.probe_fast_left = self.probe_fast_left.saturating_sub(1);
                    }
                    let interval = if self.probe_fast_left > 0 {
                        FRONTIER_PROBE_INTERVAL_FAST
                    } else {
                        FRONTIER_PROBE_INTERVAL
                    };
                    self.frontier_probe_timer = Box::pin(self.context.sleep(interval));
                }
            }
        }

        // Cancel the read-only re-jump waiter on shutdown so a spawned `sync_to`
        // wait does not outlive the executor task. All `break`s converge here; a
        // SafetyHalt never breaks, since `park_halted` parks forever.
        if let Some(handle) = self.jump_handle.take() {
            handle.abort();
        }
    }

    /// The disposition router. Every fallible executor boundary returns a
    /// [`Fault`]; this is the only place a [`FaultClass`] becomes an action, and
    /// therefore the only place `SafetyHalt::engage` is called. Engaging only
    /// here means a fork-safety verdict cannot be latched without also being
    /// routed.
    ///
    /// Dispositions, off [`FaultClass`]:
    /// - `ForkSafety` → engage + [`Self::park_halted`] (diverges);
    /// - `Corruption` → loud actor death, latch untouched → `Shutdown` → the run
    ///   loop `break`s and the supervisor aborts-all;
    /// - the transient classes + `Defer` → degrade-visible / counted, and the
    ///   loop continues (speculation stays best-effort).
    ///
    /// The is-engaged check runs before the class match so a fault arriving after
    /// the latch is already set parks whatever its class: the latch means "stop
    /// writing to the EL", and the executor is the writer. The other production
    /// engage site is the datadir marker restored at startup, which no fault
    /// reaches — hence the run loop's own pre-class gate.
    async fn dispatch_fault(&mut self, stage: &str, fault: Fault) -> Disposition {
        let (class, cause) = fault.into_parts();
        metrics::counter!(
            "dpos_executor_fault_total",
            "class" => class.as_str(),
            "reason" => class.reason_str(),
        )
        .increment(1);
        if let FaultClass::ForkSafety(reason) = class {
            self.safety_halt.engage(reason);
        }
        if self.safety_halt.is_engaged() {
            self.park_halted(stage, cause).await;
            unreachable!("park_halted never returns");
        }
        match class {
            // Handled above; a `ForkSafety` always leaves the latch engaged.
            FaultClass::ForkSafety(_) => unreachable!("engaged latch parks above"),
            FaultClass::Corruption => {
                self.on_fatal(stage, cause).await;
                Disposition::Shutdown
            }
            FaultClass::TransientExternal(reason) => {
                self.sync_metrics.degrade(reason);
                warn!(
                    error = %format_args!("{cause:#}"),
                    stage,
                    reason = reason.as_str(),
                    "executor degraded on an external/correlated cause; continuing (Decision A, \
                     no self-crash)"
                );
                Disposition::Continue
            }
            FaultClass::Defer(reason) => {
                warn!(
                    error = %format_args!("{cause:#}"),
                    stage,
                    reason = reason.as_str(),
                    "executor work item skipped; the pipeline re-presents it"
                );
                Disposition::Continue
            }
            FaultClass::TransientBounded | FaultClass::TransientConvergent => {
                warn!(
                    error = %format_args!("{cause:#}"),
                    stage,
                    class = class.as_str(),
                    "executor retry budget exhausted at a leaf belt; continuing"
                );
                Disposition::Continue
            }
        }
    }

    /// Log a genuine crash. Reached only from the router's `Corruption` arm,
    /// with the latch disengaged; the caller `break`s the loop and the
    /// supervisor aborts-all.
    async fn on_fatal(&mut self, stage: &str, error: eyre::Report) {
        error_span!("shutdown").in_scope(|| {
            error!(
                error = %format_args!("{error:#}"),
                stage,
                "executor fatal error; shutting down"
            )
        });
    }

    /// Terminal `SafetyHalt` park: stop deriving/driving reth, retain every
    /// marshal `Exact` ack unresolved, and never return (only an external
    /// shutdown aborts the task).
    ///
    /// Acknowledging would durably advance the marshal's `last_processed_height`
    /// past the diverged height, so a restart would skip it forever; dropping
    /// cancels the `Exact`, which the marshal treats as fatal. Holding the acks
    /// does not freeze the marshal: its ack waiters live in an independent arm,
    /// so an unresolved ack merely occupies a dispatch-window slot while the
    /// mailbox and resolver keep serving.
    async fn park_halted(&mut self, stage: &str, error: eyre::Report) {
        error_span!("safety_halt").in_scope(|| {
            error!(
                error = %format_args!("{error:#}"),
                stage,
                "executor SafetyHalt — parking (verify-only, stay observable): marshal acks \
                 retained, no further EL writes; recovery is the L1 SP1 validity proof + \
                 governance"
            )
        });
        if let Some(handle) = self.jump_handle.take() {
            handle.abort();
        }
        let mut retained: Vec<Exact> = Vec::new();
        retained.extend(self.inflight_ack.take());
        if let Some(d) = self.deferred.take() {
            retained.push(d.ack);
        }
        // The seed-held block gets the same treatment as `deferred.ack`.
        if let Some(held) = self.awaiting_seed.take() {
            retained.push(held.ack);
        }
        while let Some(Some((_cause, _block, ack))) =
            self.pending_finalizations.next().now_or_never()
        {
            retained.push(ack);
        }
        loop {
            match self.mailbox.recv().await {
                Some(Message {
                    command: Command::Finalize(update),
                    ..
                }) => {
                    if let Update::Block(_block, ack) = *update {
                        retained.push(ack);
                    }
                }
                Some(_) => {}
                // Every sender dropped (external teardown in flight): keep the
                // retained acks alive and pend until the task is aborted.
                None => futures::future::pending::<()>().await,
            }
        }
    }

    /// Finalized-delivery entry: a block from either the `Update::Block` drain
    /// or the startup backfill walk is stashed in the seed-hold slot and
    /// immediately resolved. The stash precedes the derive so a fatal derive
    /// under an engaged SafetyHalt reaches `park_halted` with this ack retained
    /// rather than dropped.
    ///
    /// Both feeders gate on the slot being empty, so the assignment below cannot
    /// overwrite a live ack.
    async fn on_finalized_block(
        &mut self,
        cause: Span,
        block: OrderBlock,
        ack: Exact,
    ) -> Result<(), Fault> {
        debug_assert!(
            self.awaiting_seed.is_none(),
            "both block feeders gate on an empty seed hold; overwriting it would cancel an ack"
        );
        self.awaiting_seed = Some(HeldForSeed {
            cause,
            order: block,
            ack,
            since: self.context.current(),
            reported: false,
        });
        self.try_eager_finalized_derive(EagerTrigger::Delivery)
            .await
    }

    /// Resolve the held block's σ and derive it, or keep holding.
    ///
    /// σ comes from [`Self::seed_at_own_round`], the block's own agreed round,
    /// and nothing else:
    ///
    /// - `Present`: derive now, hold consumed.
    /// - `Inactive`: derive now with `None`; a beacon-inactive epoch has no σ
    ///   and a pre-bootstrap link must never hold for one.
    /// - `Missing`: restore the hold. The only exit is the seed-record notify;
    ///   the `order.digest()` fallback would derive a different `prev_randao`
    ///   than the network.
    ///
    /// Suppressed while a predecessor is parked or a re-jump is in flight, which
    /// own the strict-order / single-EL-writer invariant.
    ///
    /// `trigger` distinguishes the on-delivery attempt from the event-driven
    /// re-attempt after σ was recorded. A `Notified` hit is counted
    /// `outcome="recovered"`; a `Notified` miss is a silent no-op, so the miss
    /// counter is not inflated on every notify while held.
    async fn try_eager_finalized_derive(&mut self, trigger: EagerTrigger) -> Result<(), Fault> {
        if self.deferred.is_some() || self.jump_done.is_some() {
            return Ok(());
        }
        let Some(held) = self.awaiting_seed.take() else {
            return Ok(());
        };
        let HeldForSeed {
            cause,
            order: block,
            ack,
            since,
            reported,
        } = held;
        let seed = match self.seed_at_own_round(block.height, block.proposal_view) {
            OwnRoundSeed::Present(seed) => Some(seed),
            OwnRoundSeed::Inactive => None,
            OwnRoundSeed::Missing => {
                // `since`/`reported` ride back unchanged: this is the same hold
                // re-entering the slot, not a new one.
                self.awaiting_seed = Some(HeldForSeed {
                    cause,
                    order: block,
                    ack,
                    since,
                    reported,
                });
                if matches!(trigger, EagerTrigger::Delivery) {
                    metrics::counter!(
                        "dpos_executor_eager_finalized_derive_total", "outcome" => "miss"
                    )
                    .increment(1);
                }
                return Ok(());
            }
        };
        let outcome_label = match (&seed, trigger) {
            (None, _) => "inactive",
            (Some(_), EagerTrigger::Delivery) => "hit",
            (Some(_), EagerTrigger::Notified) => "recovered",
        };
        metrics::counter!(
            "dpos_executor_eager_finalized_derive_total", "outcome" => outcome_label
        )
        .increment(1);
        let outcome = self.try_derive(cause, block, ack, seed).await?;
        self.defer_if_needed(outcome, true).await;
        Ok(())
    }

    /// Park a parked outcome in the deferred slot; `Done` is a no-op. `fresh` is
    /// true for a block first derived off the pipeline, which sets the
    /// observability gauge and hints the missing `h + K` body, and false for a
    /// re-poke re-stash, which must not re-hint or re-warn. Parking is terminal,
    /// re-poked event-driven, with no deadline.
    async fn defer_if_needed(&mut self, outcome: DeriveOutcome, fresh: bool) {
        match outcome {
            DeriveOutcome::Done => {}
            DeriveOutcome::NeedAttestation(d) => {
                if fresh {
                    let height = d.order.height;
                    warn!(
                        height,
                        "guard #2: committee-attested body at h+K not backfilled yet; \
                        PARKING derive + hinting peers (event-driven re-poke, no give-up timer)"
                    );
                    self.deferred_height.set(height as i64);
                    if let Some(targets) = (self.peers_for_finalization)() {
                        self.marshal
                            .hint_finalization(Height::new(height + crate::order_block::K), targets)
                            .await;
                    }
                }
                self.deferred = Some(*d);
            }
            DeriveOutcome::NeedParentVisible(d) => {
                if fresh {
                    self.deferred_height.set(d.order.height as i64);
                }
                // No `h + K` hint: the archive holds the body; the EL has just
                // not canonicalized what we already imported.
                self.deferred = Some(*d);
            }
            DeriveOutcome::NeedPrefixSeed(d) => {
                if fresh {
                    self.deferred_height.set(d.order.height as i64);
                }
                // No `h + K` hint either: the body is in the archive, its σ is
                // not, and σ is no longer askable by round. It arrives on its own
                // from the cert inlet, and the re-poke re-runs the walk's own
                // lookup.
                self.deferred = Some(*d);
            }
        }
    }

    /// Re-attempt the parked derive on a marshal delivery or the FCU heartbeat:
    /// a plain "is `h + K`'s body here yet" retry with σ retained in
    /// [`Deferred::seed`]. A still-missing body re-stays parked. Gated on
    /// `jump_done.is_none()` so a parked block is not re-derived mid-jump; a
    /// landed re-jump disposes it via `reseed_forward`. A genuine derive `Err` is
    /// fatal and propagated to the caller.
    async fn repoke_deferred(&mut self) -> Result<(), Fault> {
        if self.jump_done.is_some() {
            return Ok(());
        }
        let Some(d) = self.deferred.take() else {
            return Ok(());
        };
        match self.try_derive(d.cause, d.order, d.ack, d.seed).await? {
            outcome @ (DeriveOutcome::NeedAttestation(_)
            | DeriveOutcome::NeedParentVisible(_)
            | DeriveOutcome::NeedPrefixSeed(_)) => {
                self.defer_if_needed(outcome, false).await;
            }
            DeriveOutcome::Done => {
                self.deferred_height.set(0);
            }
        }
        Ok(())
    }

    fn reset_fcu_heartbeat_timer(&mut self) {
        self.fcu_heartbeat_timer = Box::pin(self.context.sleep(self.fcu_heartbeat_interval));
    }

    /// One frontier-probe tick (see [`ReJump::probe`]). Returns `true` iff the
    /// probe hinted a new frontier, the fast-cadence trigger.
    ///
    /// Outside a fast burst, a tip that advanced since the previous tick means the
    /// marshal is learning finalizations from an independent live source, so the
    /// probe snapshots and returns without network traffic; during a burst that
    /// advance is the probe's own delivery, so the probe keeps firing.
    ///
    /// A probe does two independent things: the ladder step
    /// `Finalized{last(T+1)}` addressed at `committee[T+1]`, where `T` is the
    /// epoch this node last handed to `track`; and the untargeted `Latest`, whose
    /// answered height above the marshal tip becomes one `hint_finalization`.
    ///
    /// Neither answer returns through here — both go through the upstream handler
    /// and the marshal's own `verify_delivered`, so a served step shows up as the
    /// tip moving on the next tick. An unserved step is counted rather than
    /// reacted to: the ladder is the repetition of this tick.
    async fn probe_frontier(&mut self) -> bool {
        let Some(probe) = self.re_jump.as_ref().and_then(|rj| rj.probe.clone()) else {
            return false;
        };
        let advanced = self.last_tip_height > self.probe_prev_tip;
        self.probe_prev_tip = self.last_tip_height;
        if advanced && self.probe_fast_left == 0 {
            // The tip moved, so whatever step was standing was served — by its
            // own answer or by an independent live source; the metric cannot tell
            // the two apart. Clearing the bit here keeps
            // `dpos_frontier_step_unserved_total` from charging an already-answered
            // step on the next frozen tick.
            self.probe_step_pending = false;
            return false;
        }
        // `T` at this tick, never a remembered one: a landing moves it, and the
        // next rung follows the landing.
        let tracked = self
            .re_jump
            .as_ref()
            .and_then(|rj| rj.tracked_epoch.clone())
            .and_then(|f| f());
        let outcome = probe(tracked).await;
        // The step goes on the marshal's resolver, not this probe's:
        // `HintFinalized` is a targeted by-height fetch decoded, BLS-verified
        // and stored by the marshal itself, so the step lands in the one place
        // that moves the tip. Repeated hints for the same height dedup in the
        // resolver, so repeating the step every tick is the ladder, not a poll.
        //
        // The step is gated on `self.marshal_floor`, not the tip: the marshal
        // drops a hint at or below its floor, while a step between the floor and
        // the tip lands in the hole a jumped node carries and is exactly the one
        // it must fetch.
        //
        // The wire cost is set by the resolver, not this cadence: a repeated hint
        // for a pending key is a no-op, and an unanswerable key is retried at the
        // resolver's own backoff without monopolising the fetcher.
        match outcome.step {
            Some((height, _)) if height <= Height::new(self.marshal_floor) => {
                metrics::counter!(FRONTIER_STEP_SKIPPED, "reason" => "at_or_below_the_floor")
                    .increment(1);
                self.probe_step_pending = false;
            }
            Some((height, targets)) => {
                // Tip frozen and a step was still standing from the previous
                // tick: nobody served it.
                if self.probe_step_pending {
                    metrics::counter!(FRONTIER_STEP_UNSERVED).increment(1);
                    debug!(
                        tracked,
                        tip = %self.last_tip_height,
                        "frozen-tip probe: nobody served the ladder step — continuing the \
                         contiguous catch-up from the floor"
                    );
                }
                self.marshal.hint_finalization(height, targets).await;
                self.probe_step_pending = true;
            }
            None => self.probe_step_pending = false,
        }
        let Some(frontier) = outcome.frontier else {
            debug!("frozen-tip probe: upstream get_latest returned None");
            return false;
        };
        if frontier > self.last_tip_height {
            debug!(
                %frontier,
                tip = %self.last_tip_height,
                "frozen-tip probe: hinting marshal toward the upstream frontier"
            );
            if let Some(targets) = (self.peers_for_finalization)() {
                self.marshal.hint_finalization(frontier, targets).await;
                return true;
            }
            debug!("frozen-tip probe: no hint targets (no scheme registered yet)");
        }
        false
    }

    /// Fire the upstream-rotation escape if one is wired. Clones the `Arc` out of
    /// `self.re_jump` so the immutable borrow does not span the `.await`.
    async fn rotate_upstream(&mut self) {
        let rotate = self.re_jump.as_ref().and_then(|rj| rj.rotate.clone());
        if let Some(rotate) = rotate {
            rotate().await;
        }
    }

    /// Send the finalize forkchoice update, retrying a transient transport error
    /// indefinitely: the engine stays up, the retry is degrade-visible while it
    /// runs and cleared on the first transport success. Returns reth's
    /// `ForkchoiceUpdated` intact.
    ///
    /// A semantic `Ok(PayloadStatusEnum::Invalid)` is not an engine error and is
    /// returned untouched for the caller's verdict split.
    ///
    /// Only the transient half of `Err` is looped. Any other class means reth
    /// processed the update and rejected the forkchoice state we named, so
    /// retrying would re-send the same unresolvable hashes forever.
    async fn fcu_retrying_transport(
        &mut self,
        forkchoice: ForkchoiceState,
    ) -> Result<ForkchoiceUpdated, Fault> {
        loop {
            match self
                .beacon_engine
                .fork_choice_updated(forkchoice)
                .pace_el_call(self.fcu_pace)
                .await
            {
                Ok(fcu) => {
                    self.sync_metrics.recover(SyncReason::EngineRetry);
                    return Ok(fcu);
                }
                Err(error) => match error.fault_class() {
                    FaultClass::TransientExternal(_) => {
                        self.sync_metrics.degrade(SyncReason::EngineRetry);
                        self.sync_metrics.engine_transient_retry.inc();
                        warn!(
                            error = %format_args!("{error:#}"),
                            "transient engine-API transport error on the finalize FCU; backing \
                             off + retrying (engine stays up — Decision A, no self-crash)"
                        );
                        self.context.sleep(ENGINE_TRANSPORT_RETRY_BACKOFF).await;
                    }
                    class => {
                        return Err(Fault::new(
                            class,
                            eyre::eyre!("finalize FCU rejected by the EL boundary: {error}"),
                        ))
                    }
                },
            }
        }
    }

    /// Fire-and-forget heartbeat FCU: the next tick is the retry, so a transport
    /// failure is counted, degraded and swallowed. A non-transport class means
    /// reth rejected the forkchoice state and is propagated rather than re-sent
    /// every tick.
    #[instrument(skip_all)]
    async fn send_forkchoice_update_heartbeat(&mut self) -> Result<(), Fault> {
        if self.jump_done.is_some() {
            // A re-jump's `sync_to` drives the EL during backfill; an interleaved
            // heartbeat FCU returns reth `SYNCING`, producing a spurious `Stalled`
            // that would churn upstream rotation.
            debug!("FCU heartbeat suppressed; re-jump in flight (sync_to drives the EL)");
            return Ok(());
        }
        if !self.has_advanced_since_init {
            debug!(
                head = %self.last_canonicalized.forkchoice.head_block_hash,
                finalized = %self.last_canonicalized.forkchoice.finalized_block_hash,
                "FCU heartbeat suppressed; no consensus advance since cold-start init"
            );
            return Ok(());
        }
        info!(
            head = %self.last_canonicalized.forkchoice.head_block_hash,
            finalized = %self.last_canonicalized.forkchoice.finalized_block_hash,
            "FCU heartbeat",
        );
        let resp = self
            .beacon_engine
            .fork_choice_updated(self.last_canonicalized.forkchoice)
            .pace_el_call(self.fcu_pace)
            .await;
        // A heartbeat FCU transport failure is fire-and-forget (the next tick is
        // the retry) but counted and degrade-visible like the finalize FCU; a
        // successful tick clears the reason.
        match resp {
            Ok(_) => self.sync_metrics.recover(SyncReason::EngineRetry),
            Err(error) => match error.fault_class() {
                FaultClass::TransientExternal(_) => {
                    self.sync_metrics.degrade(SyncReason::EngineRetry);
                    self.sync_metrics.engine_transient_retry.inc();
                    warn!(error = %error, "heartbeat FCU failed (transport); counted + degraded");
                }
                class => {
                    return Err(Fault::new(
                        class,
                        eyre::eyre!("heartbeat FCU rejected by the EL boundary: {error}"),
                    ))
                }
            },
        }
        Ok(())
    }

    async fn handle_message(&mut self, message: Message) -> Result<(), Fault> {
        let cause = message.cause;
        match message.command {
            Command::Finalize(finalized) => match *finalized {
                // No FCU here: the tip digest is an ordering digest reth cannot
                // resolve, and the EL never needs devp2p for the DPoS segment —
                // catch-up is marshal backfill of OrderBlocks plus local
                // derivation, so every derived block's parent is locally present
                // by construction.
                //
                // The marshal emits `Update::Tip` whenever it stores a
                // finalization above its tip, and keeps doing so during a wedge,
                // so it is the event the steady-state re-jump reacts to.
                Update::Tip(_round, height, _ordering_digest) => {
                    // Remember the frontier so the heartbeat can re-poke the
                    // re-jump after the upstream frontier plateaus.
                    self.last_tip_height = height;
                    debug!(%height, "ordering tip observed; EL catch-up is backfill+derive");
                    self.maybe_re_jump(height).await?;
                    // A parked block's `h + K` body may have landed silently;
                    // re-poke it.
                    self.repoke_deferred().await?;
                }
                Update::Block(block, ack) => {
                    // An old-range block can escape into this mailbox after
                    // `reseed_forward` raises the floor but before the marshal
                    // processes `SetFloor`. The marshal already pruned it, so
                    // deriving it against the jumped `db_tip` is the deep-overlay
                    // walk the jump avoids, and parking it awaits a pruned `h + K`.
                    // Acknowledge without deriving (dropping the `Exact` would be
                    // Canceled, fatal to the marshal), count it, and re-poke the
                    // deferred block.
                    if block.height <= self.marshal_floor {
                        metrics::counter!("dpos_executor_stale_dispatch_dropped_total")
                            .increment(1);
                        ack.acknowledge();
                        self.repoke_deferred().await?;
                        return Ok(());
                    }
                    self.pending_finalizations
                        .push_back(ready((cause, block, ack)));
                    self.pending_finalizations_gauge
                        .set(self.pending_finalizations.len() as i64);
                    // A delivery may coincide with a parked block's `h + K` body
                    // landing; re-poke it.
                    self.repoke_deferred().await?;
                }
            },
            Command::SpecNotarized(n) => {
                let Notarized { digest, seed } = *n;
                // Speculation stays best-effort, but the class decides that, not
                // this call site: `spec_execute` classifies its own failures
                // `Defer`/`TransientExternal`, which the router logs and continues
                // on, and a `ForkSafety` reaches the router too.
                self.spec_execute(cause.clone(), digest, seed).await?;
                // A live spec advance may unblock a parked out-of-order
                // notarization; drain it now.
                self.try_drain_parked(&cause).await?;
            }
        }
        Ok(())
    }

    /// Steady-state self-healing re-jump (see [`ReJump`]): the marshal tip has
    /// run more than [`ReJump::threshold`] finalized blocks ahead of
    /// `ordering_finalized`, so it sits outside the upstream serving window and
    /// the marshal's backfill resolver finds nothing — the floor freezes.
    ///
    /// The multi-minute backfill never blocks the `select!` loop: the jump runs
    /// as a spawned read-only waiter whose outcome the executor handles on the
    /// `jump_done` select arm, which runs `reseed_forward` (the write half,
    /// shared with the init seed).
    ///
    /// A missing `re_jump`, an already-in-flight jump, a gap at or below
    /// [`ReJump::threshold`], a marshal archive with no pair at the tip, or a
    /// mid-flight startup drain all early-return. A parked block does not gate
    /// it: past the threshold the park is a deep catch-up, the re-jump backfills
    /// the BLS-authenticated `[.. landing]` (the parked height is a finalized
    /// ancestor of the landing), and `reseed_forward` acknowledges the parked
    /// block instead of dropping it.
    async fn maybe_re_jump(&mut self, height: Height) -> Result<(), Fault> {
        let Some(re_jump) = self.re_jump.clone() else {
            return Ok(());
        };
        if self.jump_done.is_some() {
            return Ok(());
        }
        // `height` is the marshal tip: the `Update::Tip` emitted from
        // `store_finalization`, i.e. the highest finalization this node verified
        // and stored. It is the only frontier and the only authenticated one.
        if height.get().saturating_sub(self.ordering_finalized)
            <= re_jump.threshold
            // The startup backfill drain and a jump must not both drive reth's EL.
            || self.pending_backfill.is_some()
            || !self.finalized_heights_to_backfill.is_empty()
        {
            return Ok(());
        }
        // The target is read from this node's own marshal archive at the tip that
        // triggered this. `Update::Tip` fires from `store_finalization` only after
        // the pair is written, so a hit here already passed `verify_delivered`.
        //
        // A miss is not a fault. A floor raise deletes nothing (both finalized
        // archives are immutable), so the reachable miss is the seeded tip:
        // `last_tip_height` starts at `cfg.last_consensus_finalized_height`, and
        // on an empty archive (a fresh datadir) the first heartbeat re-poke names
        // a height nothing was stored at. Skip; the next tip or heartbeat re-arms.
        let Some((finalization, block)) = self.marshal.pair_at(height).await else {
            debug!(
                tip = %height,
                "re-jump trigger fired but the marshal archive has no pair at the tip; \
                 re-evaluating on the next tip"
            );
            return Ok(());
        };
        let target = crate::cert_follow::UpstreamFinalized {
            finalization,
            block,
        };
        info!(
            tip = %height,
            ordering_finalized = self.ordering_finalized,
            "marshal tip ran past the serving window; spawning steady-state re-jump waiter"
        );
        // `re_jump` was cloned out of `self.re_jump` above and is unused after
        // this move, so no second clone is needed.
        let from = self.ordering_finalized;
        let (tx, rx) = oneshot::channel();
        let handle = self
            .context
            .with_label("steady_state_rejump")
            .spawn(move |_| async move {
                let _ = tx.send((re_jump.call)(from, target).await);
            });
        self.jump_done.replace(rx);
        self.jump_handle = Some(handle);
        Ok(())
    }

    /// Fetch and store the epoch-boundary block(s) a floor raise to `floor` would
    /// bury and that this node does not already hold.
    ///
    /// `b` is the largest epoch-terminal height at or below `floor` (needed by
    /// `Inline::genesis(E)`, hence the engine-spawn gate); `b + 1` is the epoch's
    /// first block, read by the promote value-gate for the network-attested key.
    /// Both or neither when both are buried: seeding `b` alone would let the
    /// member promote exactly when the value gate degrades to a no-op.
    ///
    /// Every failure path is a no-op that leaves the node verify-only for the
    /// landing epoch, reported via the seam's warn + counter.
    async fn seed_boundary_below_floor(&mut self, floor: u64, at_hash: B256) {
        let Some(fetch) = self.boundary_fetch.clone() else {
            return;
        };
        let Some(b) = self.epocher.terminal_at_or_below(Height::new(floor)) else {
            return;
        };
        let mut missing = Vec::new();
        for h in [b.get(), b.get() + 1] {
            if h > floor {
                continue; // not buried by this raise
            }
            if self
                .marshal
                .fetch_block_by_height(Height::new(h))
                .await
                .is_none()
            {
                missing.push(h);
            }
        }
        if missing.is_empty() {
            return;
        }
        let mut fetched = Vec::with_capacity(missing.len());
        for h in &missing {
            let Some(uf) = fetch(*h, at_hash).await else {
                fetched.clear();
                break;
            };
            fetched.push(uf);
        }
        if fetched.is_empty() {
            warn!(
                heights = ?missing,
                floor,
                "epoch-boundary seeding incomplete before a re-jump floor raise — this member \
                 stays verify-only (no proposals, no votes) until the next epoch boundary"
            );
            return;
        }
        for uf in fetched {
            let height = uf.block.height;
            let round = uf.finalization.proposal.round;
            self.marshal
                .store_verified_finalization(round, uf.block, uf.finalization)
                .await;
            self.sync_metrics.jump_boundary_refetched.inc();
            info!(
                height,
                floor,
                "seeded epoch-boundary block below the re-jump floor so this member can spawn \
                 its engine in the landing epoch"
            );
        }
    }

    /// Re-seed the executor and marshal at a re-jump landing, the steady-state
    /// mirror of the init seed (the two must agree on field shape). Runs only in
    /// the `jump_done` completion arm, so it is the sole writer of executor state
    /// and `set_floor`.
    async fn reseed_forward(
        &mut self,
        landing_h: u64,
        landing_hash: B256,
        floor: u64,
    ) -> Result<(), Fault> {
        info!(
            landing_h,
            floor, "steady-state re-jump landed; re-seeding executor + marshal floor"
        );
        let landing = Height::new(landing_h);
        self.anchor_finalized = (landing, landing_hash);
        // The landing is the ordering-final tip (`safe`); `floor = landing − K` is
        // the result-final point (`finalized`). `update_finalized` raises the
        // in-memory `finalized_height`/`head` to the landing and `update_safe`
        // raises `safe` to it, while the FCU below re-pins the engine's
        // `finalized` to the floor. The resulting in-memory `finalized_height`
        // over-claim is benign: `result_final` is recomputed from
        // `ordering_finalized`, not from the model's `finalized_height`.
        self.last_canonicalized = self
            .last_canonicalized
            .update_finalized(landing, landing_hash)
            .update_safe(landing, landing_hash);
        // `update_finalized`/`update_safe` advanced the executor's internal model,
        // but reth has so far made the backfilled landing segment visible only by
        // number: the by-hash header index the deriver reads for the parent lags
        // until an FCU lands. `head = landing` canonicalizes the whole segment by
        // hash, so the resumed dispatch's first derive (`floor + 1`) resolves its
        // parent (`floor`) instead of hitting `ParentHeaderMissing`; without it the
        // floor freezes — the steady-state analogue of the cold-start
        // parent-visibility race. `safe = landing` rides the ordering-final tip
        // while `finalized = floor` keeps the two-tier contract (the landing's own
        // result attestation still lags by K).
        if let Some(floor_hash) = self.executed.spec_executed_hash(floor) {
            let resp = self
                .beacon_engine
                .fork_choice_updated(ForkchoiceState {
                    head_block_hash: landing_hash,
                    safe_block_hash: landing_hash,
                    finalized_block_hash: floor_hash,
                })
                .pace_el_call(self.fcu_pace)
                .await;
            if let Err(error) = resp {
                warn!(
                    error = %format_args!("{error:#}"),
                    "reseed_forward canonicalization FCU failed"
                );
            }
        } else {
            warn!(
                floor,
                "reseed_forward: floor hash not present after backfill; relying on the \
                 ParentHeaderMissing derive-retry belt"
            );
        }
        // The cursor goes to the landing, not the floor: the landing is executed
        // post-backfill, and the K blocks below it are governed by the two-tier
        // result-lag. The init seed does the same.
        self.ordering_finalized = self.ordering_finalized.max(landing_h);
        // The first post-jump proposals sample `finalized_executed_hash` at
        // `landing + 1 − K ..= landing`, heights below the landing that only the
        // cursor's provider resolve covers, so the cursor is advanced to the
        // landing rather than merely recorded; a provider miss there returns
        // `None` and the proposal is skipped, never given a wrong hash.
        self.executed.advance_finalized(landing_h);
        (self.anchor_advanced)();
        // The speculative tip and map are stale across a deep jump (their heights
        // sit far below the landing): raise `spec_head` to the landing and drop
        // spec entries at or below it so the next notarization re-speculates
        // forward from there.
        self.spec_head = self.spec_head.max(landing_h);
        self.spec_executed = self.spec_executed.split_off(&(landing_h + 1));
        // Parked notarizations below the landing are stale for the same reason.
        self.parked_spec = self.parked_spec.split_off(&(landing_h + 1));
        // The startup backfill iterator seeded at `init` is drained by height off
        // the loop head and is gated off, not advanced, during an in-flight jump.
        // Left alone, the post-jump drain restarts at its pre-jump height and
        // re-derives the whole jumped range against `db_tip = landing`, one
        // thousands-block overlay walk per derive, until mdbx times out and the
        // spare never converges. The jumped range is redundant here — the same
        // BLS attestation and EL sync that let this reseed advance the cursors
        // covers every height at or below the landing — so fast-forward the
        // iterator to `landing_h + 1`, keeping the original upper bound.
        if !self.finalized_heights_to_backfill.is_empty() {
            let next = *self.finalized_heights_to_backfill.start();
            let end = *self.finalized_heights_to_backfill.end();
            if next <= landing_h {
                let skipped = landing_h.min(end) - next + 1;
                // Correctly empty when the whole remaining span was at or below
                // the landing.
                self.finalized_heights_to_backfill = (landing_h + 1)..=end;
                metrics::counter!("dpos_executor_backfill_fastforward_total").increment(skipped);
                info!(
                    skipped,
                    landing = landing_h,
                    "fast-forwarded startup-backfill iterator past the re-jump landing \
                     (jumped range is BLS-authenticated + EL-synced — re-derive skipped)"
                );
            }
        }
        self.has_advanced_since_init = true;
        // The parked height is a finalized ancestor of the landing and its block
        // is canonical in reth after the jump backfill, so acknowledge it: a drop
        // would cancel the `Exact`, which the marshal treats as fatal. Done before
        // `set_floor` so `SetFloor`'s `pending_acks.clear()` has nothing to
        // cancel.
        if let Some(d) = self.deferred.take() {
            self.deferred_height.set(0);
            d.ack.acknowledge();
        }
        // Same for the seed-held block: the landing is far above it, so the floor
        // move prunes the held height.
        if let Some(held) = self.awaiting_seed.take() {
            held.ack.acknowledge();
        }
        // `Update::Block` deliveries queue unconditionally while the drain arm is
        // gated off during a park plus in-flight jump, up to `MAX_PENDING_ACKS`
        // stale below-landing entries. Draining them post-jump would repopulate
        // `awaiting_seed` with a jumped-over height whose parent the jump pruned,
        // so the gap-walk would find nothing and report a manufactured skip-gap as
        // archive corruption. Entries at or below the landing are canonical
        // post-backfill and are acknowledged without derive; entries above it
        // (none expected, dispatch was stalled below) stay in order. Done before
        // `set_floor` for the same reason as the disposals above. The queue holds
        // only `Ready` futures, so this drain never blocks.
        let mut kept = FuturesOrdered::new();
        let mut pruned = 0u64;
        while let Some((cause, block, ack)) = self.pending_finalizations.next().await {
            if block.height <= landing_h {
                pruned += 1;
                ack.acknowledge();
            } else {
                kept.push_back(ready((cause, block, ack)));
            }
        }
        self.pending_finalizations = kept;
        self.pending_finalizations_gauge
            .set(self.pending_finalizations.len() as i64);
        if pruned > 0 {
            metrics::counter!("dpos_executor_stale_finalization_pruned_total").increment(pruned);
            info!(
                pruned,
                landing = landing_h,
                "pruned stale queued finalizations at/below the re-jump landing \
                 (acked Ok — canonical post-backfill)"
            );
        }
        // `set_floor` is fire-and-forget, so old-range blocks freed by the
        // disposals above can still reach the executor mailbox before the marshal
        // processes it. Recording the floor first lets the `Update::Block` arm
        // acknowledge those stragglers without derive instead of parking them.
        // Seed the boundary blocks this raise is about to bury before raising the
        // floor: a re-jump teleports the floor on a running node that never
        // restarts, so the cold-start seeding cannot run. Keyed on the condition
        // (a terminal at or below the floor is missing locally), not an event, so
        // a node that already holds it does no work.
        self.seed_boundary_below_floor(floor, landing_hash).await;
        self.marshal_floor = floor;
        self.marshal.set_floor(Height::new(floor)).await;
        // Enter the landing epoch: the floor raise just disqualified this epoch's
        // predecessor terminal, so no delivered boundary block can enter it.
        //
        // Keyed on the landing, not the floor — they differ by a whole epoch when
        // the landing sits within K of an epoch start, and the floor would enter
        // the wrong one. The seeding above asks `terminal_at_or_below(floor)`
        // because it repairs the boundary the raise buries; this asks
        // `terminal_at_or_below(landing_h)` because it names the epoch the node is
        // now in.
        //
        // The read floor is published first: the named boundary can sit a whole
        // epoch below the landing, and the state machine resolves its committee
        // reads at `boundary − K`, a height the jump pruned. The raise clamps that
        // read to the landing's result-final point, which the jump backfilled. It
        // uses `floor`, not the landing: the landing is ordering-final (`safe`)
        // while `floor = landing − K` is the result-final point the FCU pins as
        // `finalized`. The raise is monotone on the state-machine side.
        //
        // Fired on every landing, including ones where seeding was a no-op; the
        // state machine's `last_tracked_epoch < next` gate makes a duplicate
        // `Intra`.
        (self.boundary_read_floor)(floor).await;
        if let Some(terminal) = self.epocher.terminal_at_or_below(Height::new(landing_h)) {
            let enter = self.boundary_enter.clone();
            info!(
                landing = landing_h,
                boundary = terminal.get(),
                floor,
                "entering the landing epoch after a re-jump"
            );
            enter(terminal.get());
        }
        // `spec_head` advanced to the landing, so drain parked notarizations just
        // above it. Safe: the `jump_done` arm cleared the in-flight jump and the
        // deferred block was disposed above, so `spec_execute`'s gate is open, and
        // the leading prune already dropped entries at or below the landing.
        self.try_drain_parked(&Span::current()).await
    }

    /// Speculatively derive and import a notarized block, advancing the EL head
    /// ahead of finalization. Forward-only (`spec_head + 1`): a gap or an
    /// already-covered height is left to the finalized path, which keeps this
    /// race-free with finalized delivery in the same loop.
    #[instrument(skip_all, parent = &cause, fields(%digest), err(Debug, level = Level::DEBUG))]
    async fn spec_execute(
        &mut self,
        cause: Span,
        digest: crate::digest::Digest,
        seed: Option<crate::beacon::Seed>,
    ) -> Result<(), Fault> {
        // A deferred finalized block has not been derived yet, so speculating past
        // it would advance head/spec_head over the deferred height and break
        // strict order; the finalized path is the sole authority and must derive
        // first. The mailbox arm stays ungated so shutdown and `Command::Finalize`
        // keep flowing — the guard lives here.
        //
        // A speculative FCU carries a low finalized hash that would retarget
        // reth's backfill away from an in-flight jump's tip, so the jump stays the
        // only EL writer while it runs.
        if self.deferred.is_some() || self.jump_done.is_some() {
            return Ok(());
        }
        let Some(order) = self.marshal.fetch_block_by_digest(digest).await else {
            // The finalized path derives it once the body arrives.
            return Ok(());
        };
        let height = order.height;
        // A gap (a higher height) is parked so `try_drain_parked` re-drives it
        // once `spec_head` catches up; dropping it would lose speculation
        // permanently after a transient fall-behind. A height at or below the tip
        // is a re-notarization the finalized path owns, and overwriting by height
        // keeps the latest sibling.
        if height != self.spec_head + 1 {
            if height > self.spec_head + 1 {
                self.parked_spec.insert(height, ParkedSpec { digest, seed });
            }
            return Ok(());
        }
        let parent_height = height.checked_sub(1).ok_or_else(|| {
            Fault::defer(
                DeferReason::SpecDeriveFailed,
                eyre::eyre!("speculative height 0"),
            )
        })?;
        // A transient parent miss (reth visibility lag) parks the notarization so
        // the next `spec_head` advance retries it instead of losing it.
        let Some(parent_hash) = self.executed.spec_executed_hash(parent_height) else {
            self.parked_spec.insert(height, ParkedSpec { digest, seed });
            return Ok(());
        };

        // Seal the block with the round it was proposed at, not the seed's spin
        // round: speculating with `seed(V0+k)` would guarantee a re-derive and
        // head reorg at the boundary. A mismatch takes the canonical round's
        // bytes from the seed store (a threshold seed is unique per round); a miss
        // skips speculation, since a wrong seed must never be used and the
        // finalized path resolves the height's own round regardless.
        let seed = match seed {
            None => None,
            Some(s) => {
                let canonical = commonware_consensus::types::Round::new(
                    s.target_round.epoch(),
                    commonware_consensus::types::View::new(order.proposal_view),
                );
                if s.target_round == canonical {
                    Some(s)
                } else {
                    metrics::counter!("dpos_spec_seed_recanonicalized_total").increment(1);
                    match self.randomness.seed(canonical) {
                        Some(seed) => Some(seed),
                        None => {
                            debug!(
                                height,
                                notarized_round = ?s.target_round,
                                canonical_round = ?canonical,
                                "skipping speculation: first-seen notarization is a spin round \
                                 and the canonical round's seed is not in the store"
                            );
                            return Ok(());
                        }
                    }
                }
            }
        };

        // The round this speculation derives with (`None` = no beacon), captured
        // before `seed` moves into the deriver; `try_derive` reconciles it against
        // the witness round.
        let seed_round = seed.as_ref().map(|s| s.target_round);
        // Derive+import wall time for the tip paths only; the catch-up paths run
        // back-to-back by design and would read as false saturation.
        let el_apply_started = std::time::Instant::now();
        // A derive failure here is a `Defer`, never `Corruption`, because the
        // finalized path derives this height regardless — and `Fault`'s blanket
        // `From<eyre::Report>` is `Corruption`, so `?` would turn a transient
        // failure into actor death.
        let derived = self
            .deriver
            .derive_and_execute(order, parent_hash, seed)
            .await
            .map_err(|error| {
                Fault::defer(
                    DeferReason::SpecDeriveFailed,
                    error.wrap_err("speculative derive_and_execute failed"),
                )
            })?;
        let derived_hash = derived.evm_hash();
        // Labels this height's derive with the seed round it used, so a later
        // finalized re-derive of the same height can be told apart from this one.
        tracing::info!(
            target: "dpos::derive_seed",
            height,
            path = "speculative-notarization",
            seed_round = ?seed_round,
            evm_hash = %derived_hash,
            "derive-seed: speculative derive path",
        );
        self.submit_finalized_payload(derived).await?;
        metrics::histogram!("dpos_derive_el_apply_duration_seconds", "path" => "speculative")
            .record(el_apply_started.elapsed().as_secs_f64());
        self.record_el_lag();

        // Head only: the block is not finalized, so the result-final cursor stays
        // put and there is no marshal ack.
        let new = self
            .last_canonicalized
            .update_head(Height::new(height), derived_hash);
        // The engine boundary's own class decides: a transport blip degrades and
        // is retried by the next notarization, while a rejected forkchoice state
        // is the permanent local condition `fcu_retrying_transport` also refuses
        // to loop on.
        let fcu = self
            .beacon_engine
            .fork_choice_updated(new.forkchoice)
            .pace_el_call(self.fcu_pace)
            .await
            .map_err(|error| {
                Fault::new(
                    error.fault_class(),
                    eyre::eyre!("speculative FCU failed: {error}"),
                )
            })?;
        // A finalized FCU treats this `Ok(Invalid)` as a safety halt; here it is a
        // plain skip, and the asymmetry is deliberate. Reth's `Invalid` means the
        // head descends from a header it downloaded over devp2p and rejected —
        // evidence about the network, not this node's disk — and this head is
        // notarized but unfinalized, so consensus may still nullify the view and
        // finalize a sibling. Halting would make a branch the protocol may discard
        // permanent.
        //
        // Nothing is lost by waiting: every finalized derive issues an FCU with a
        // head at or above the finalized tip, so an invalid ancestor at or below
        // it re-renders the same verdict on the finalized path within one block,
        // where it engages the latch with committed evidence; only an ancestor
        // strictly inside the speculative segment escapes, and those are exactly
        // the blocks consensus has not committed.
        //
        // The import verdict is judged differently one frame up
        // (`submit_finalized_payload` halts on `Invalid` from either path) because
        // it is a statement about this node's derivation matching reth's
        // re-execution — deterministic, and independent of which branch commits.
        if !(fcu.is_valid() || fcu.is_syncing()) {
            return Err(Fault::defer(
                DeferReason::SpecFcuRejected,
                eyre::eyre!(
                    "EL reported non-valid speculative FCU at height {height}: {:?} — skipping \
                     speculation; the finalized path re-renders this verdict if the branch \
                     commits",
                    fcu.payload_status
                ),
            ));
        }
        self.last_canonicalized = new;
        self.has_advanced_since_init = true;
        self.spec_head = height;
        self.spec_executed.insert(
            height,
            SpecExecuted {
                digest,
                seed_round,
                parent_hash,
            },
        );
        Ok(())
    }

    /// Re-drive parked speculative notarizations that became runnable after a
    /// `spec_head` advance, which happens on out-of-order arrival and on the
    /// finalized reconcile.
    ///
    /// The leading `split_off` is also the prune: it drops parked heights at or
    /// below `spec_head` (already speculated or finalized). Since
    /// `spec_head >= ordering_finalized` always, that is a superset of "prune at
    /// or below finalized" and the map's only bound (about K). A rollback resets
    /// `spec_head` to the finalized height and keeps entries above the new tip so
    /// they re-evaluate against the finalized fork.
    ///
    /// A `spec_execute` failure keeps the entry for the next advance and ends the
    /// drain, handing the [`Fault`] to the run loop's router: it continues on the
    /// classes `spec_execute` produces (`Defer`/`TransientExternal`) and parks on
    /// a `ForkSafety` one. Not recursive — `spec_execute` never calls back here.
    async fn try_drain_parked(&mut self, cause: &Span) -> Result<(), Fault> {
        self.parked_spec = self.parked_spec.split_off(&(self.spec_head + 1));
        let mut resumed = 0u32;
        while let Some(parked) = self.parked_spec.get(&(self.spec_head + 1)).cloned() {
            let next = self.spec_head + 1;
            let before = self.spec_head;
            if let Err(fault) = self
                .spec_execute(cause.clone(), parked.digest, parked.seed)
                .await
            {
                // The entry stays parked for the next advance; the router decides
                // whether the executor also stops.
                debug!(
                    height = next,
                    "parked speculative drain failed; entry retained for the next advance"
                );
                return Err(fault);
            }
            if self.spec_head > before {
                // `spec_execute` advanced past `next`, so speculation resumed from
                // the park.
                self.parked_spec.remove(&next);
                metrics::counter!("dpos_executor_spec_resume_total").increment(1);
                resumed += 1;
            } else {
                // A transient gate held (body not buffered, parent not executed,
                // deferred or jump in flight), so keep the entry and stop; a later
                // advance retries it.
                break;
            }
        }
        if resumed > 0 {
            info!(
                resumed,
                spec_head = self.spec_head,
                parked_remaining = self.parked_spec.len(),
                "resumed speculation from parked notarizations after a spec_head advance"
            );
        }
        Ok(())
    }

    /// Consensus-order tip (`last_tip_height`, fed by `Update::Tip`) minus the
    /// executed EL tip: sustained lag of K or more is the mechanism behind the
    /// verify-time result-gate stall, so it is surfaced as a gauge. Called at
    /// each tip-path apply site, where both values are already in hand.
    fn record_el_lag(&self) {
        let lag = self
            .last_tip_height
            .get()
            .saturating_sub(self.executed.executed_tip());
        metrics::gauge!("dpos_executor_el_lag_blocks").set(lag as f64);
    }

    /// Detector, never a deadline: reports a block held in the seed hold longer
    /// than [`SEED_HOLD_STALL_THRESHOLD`] and changes nothing about it.
    ///
    /// The hold is bounded only if every `impl Beacon` a production node can be
    /// given actually supplies σ — a claim this crate asserts but cannot verify
    /// from inside the executor, so the counter is what makes a violating
    /// implementation surface in the smoke harness.
    ///
    /// Sibling of the `dpos_executor_stray_seed_at_inactive_round_total` counter
    /// below: both detect beliefs this design asserts, both are expected to read
    /// 0, and neither changes a derive.
    ///
    /// No timer is added: this rides the existing FCU heartbeat tick, reads the
    /// clock and returns. A timeout could neither derive (the `order.digest()`
    /// fallback forks) nor skip (a permanent hole), so it would only make a
    /// silent stall loud without restoring liveness.
    fn detect_stalled_seed_hold(&mut self) {
        let now = self.context.current();
        let Some(held) = self.awaiting_seed.as_mut() else {
            return;
        };
        if held.reported {
            return;
        }
        // A backwards clock is not evidence of a stall.
        let Ok(age) = now.duration_since(held.since) else {
            return;
        };
        if age < SEED_HOLD_STALL_THRESHOLD {
            return;
        }
        held.reported = true;
        metrics::counter!("dpos_executor_seed_hold_stalled_total").increment(1);
        warn!(
            height = held.order.height,
            proposal_view = held.order.proposal_view,
            held_for_secs = age.as_secs(),
            "block held for its own round's σ past the detector threshold — it is NOT \
             abandoned and no fallback runs (the only correct exit is σ arriving). A \
             non-zero count means a randomness provider that this node class was assumed \
             to have is not supplying σ"
        );
    }

    /// σ for `height`'s own agreed round — the only seed source of the finalized
    /// derive. A threshold σ is unique per round, so a wrong round can only miss.
    /// A height whose epoch the map cannot name is `Inactive`: the beacon cannot
    /// have been mandatory in an epoch that does not exist.
    fn seed_at_own_round(&self, height: u64, proposal_view: u64) -> OwnRoundSeed {
        use commonware_consensus::types::{Epocher as _, Round, View};
        let Some(info) = self.epocher.containing(Height::new(height)) else {
            return OwnRoundSeed::Inactive;
        };
        let round = Round::new(info.epoch(), View::new(proposal_view));
        if !self.randomness.mandatory_at(round.epoch().get()) {
            // Looked up only to count it; the value is never handed on.
            if self.randomness.seed(round).is_some() {
                metrics::counter!("dpos_executor_stray_seed_at_inactive_round_total").increment(1);
                warn!(
                    height,
                    %round,
                    "σ present at a round the agreed epoch map calls beacon-INACTIVE — IGNORED \
                     (the network derives `None` here); a corrupted or crafted local seed record"
                );
            }
            return OwnRoundSeed::Inactive;
        }
        match self.randomness.seed(round) {
            Some(seed) => OwnRoundSeed::Present(seed),
            None => OwnRoundSeed::Missing,
        }
    }

    /// Derive, import, FCU and ack a finalized block from `seed` — σ of the
    /// block's own round, resolved by [`Self::seed_at_own_round`] (`None` = a
    /// beacon-inactive, seed-independent link). Guard #2 (the `h + K` look-ahead
    /// convergence check) runs when the node is K or more behind; if the attested
    /// body at `h + K` is not backfilled yet, this returns `NeedAttestation`
    /// without mutating finalized state or acking, so the caller parks it and
    /// re-pokes on the delivery stream or FCU heartbeat.
    ///
    /// While this function holds the block's `Exact` in [`Self::inflight_ack`],
    /// the only [`FaultClass`]es it may return are the two the router does not
    /// continue on: `ForkSafety` (parks, and `park_halted` retains the ack) and
    /// `Corruption` (the run loop breaks). A class the router continues on would
    /// orphan that ack in the slot, and the next derive's `inflight_ack =
    /// Some(..)` would drop it — a dropped `Exact` is Canceled, fatal to the
    /// marshal. The transient classes appear only after
    /// `take_inflight_ack().acknowledge()`, where the slot is empty.
    #[instrument(skip_all, parent = &cause, fields(height = order.height), err(Debug))]
    async fn try_derive(
        &mut self,
        cause: Span,
        order: OrderBlock,
        ack: Exact,
        seed: Option<crate::beacon::Seed>,
    ) -> Result<DeriveOutcome, Fault> {
        // Parked in the slot so an `Err` exit, including the safety-halt paths
        // that surface through `?`, leaves the ack alive for `park_halted`; it is
        // taken back at each non-`Err` exit.
        self.inflight_ack = Some(ack);
        let height = order.height;
        // Captured before `order` is consumed by the derive below; the attested
        // result commits `executed_hash(height − K)` and is cross-checked after
        // the derive lands.
        let attested_result = order.result;
        let parent_height = height
            .checked_sub(1)
            .ok_or_else(|| eyre::eyre!("ordering height 0 cannot be finalized"))?;

        // The round feeds the speculation-reuse check below; the value is reused
        // by the re-derive branch and the re-apply loop.
        let finalization_seed = seed;
        let finalization_round = finalization_seed.as_ref().map(|s| s.target_round);

        // Keep the speculatively-executed block only when it is the same ordering
        // block and was speculated with the same seed round the finalized derive
        // resolved; then reth is already canonical, so skip the re-derive and do
        // not roll the head back (the speculative lead above `height` must
        // survive). A digest match with a different round is an anomaly: it is
        // counted and re-derived from the store's σ, the same path a first
        // execution or a sibling-nullified digest takes. `None == None`
        // (no beacon) keeps the fast path.
        let (spec_round, spec_parent) = {
            let entry = self
                .spec_executed
                .get(&height)
                .filter(|s| s.digest == order.digest());
            (entry.map(|s| s.seed_round), entry.map(|s| s.parent_hash))
        };
        if let Some(round) = spec_round {
            if round != finalization_round {
                metrics::counter!("dpos_spec_round_mismatch_total").increment(1);
                warn!(
                    height,
                    spec_round = ?round,
                    finalized_round = ?finalization_round,
                    "speculation round disagrees with the finalized round (both should be \
                     Round(Ep, proposal_view)) — re-deriving from the agreed round"
                );
            }
        }
        // The speculated block may be reused as final only if it descends from the
        // block canonical at `parent_height` now: after a head rollback the parent
        // was re-derived to a different hash, so a block recorded at `height` was
        // executed against the now-orphaned parent and would splice a fork onto
        // the finalized chain if reused. An absent parent is not a match and takes
        // the re-derive path.
        let correctly_speculated = spec_round == Some(finalization_round)
            && spec_parent == self.executed.spec_executed_hash(parent_height)
            && self.executed.spec_executed_hash(height).is_some();

        // Retained for the post-FCU apply-retry loop, since the re-derive branch
        // consumes `finalization_seed`.
        let finalization_seed_retry = finalization_seed.clone();
        // The derive branch consumes `order`, but guard #2's absent-body arm must
        // hand it back to park, so clone it up front. In steady state the derive
        // of `h` runs when `h + 1` is the tip, the gate is false, and no clone
        // happens.
        let behind_by_k = self.last_tip_height.get() >= height + crate::order_block::K;
        let order_for_park = behind_by_k.then(|| order.clone());
        let derived_hash = if correctly_speculated {
            self.executed
                .spec_executed_hash(height)
                .expect("checked is_some above")
        } else {
            // The missing prefix and the delivered height derive through the same
            // call, so no second site has to remember to catch an invisible
            // parent.
            let gap = self.executed.spec_executed_hash(parent_height).is_none();
            // Cloned only when a gap exists: the park needs an owned `order` and
            // `seed`, and the derive consumes both.
            let parked = gap.then(|| (order.clone(), finalization_seed.clone()));
            match self
                .derive_finalized_with_gap_fill(order, finalization_seed)
                .await
            {
                Ok(hash) => hash,
                Err(error) if is_parent_not_visible(error.cause()) => {
                    // A no-gap derive: `block_hash(h)` resolving does not imply the
                    // header read will, because reth canonicalizes on the
                    // engine-tree thread before provider reads see the header. With
                    // no park payload, this transient stays the recoverable `Err`.
                    let Some((order, seed)) = parked else {
                        return Err(error);
                    };
                    warn!(
                        height,
                        parent_height,
                        error = %format_args!("{:#}", error.cause()),
                        "parent still invisible after canonicalization; PARKING \
                         (event-driven re-poke, no give-up timer)"
                    );
                    metrics::counter!("dpos_executor_parent_visibility_park_total").increment(1);
                    return Ok(DeriveOutcome::NeedParentVisible(Box::new(Deferred {
                        cause,
                        order,
                        ack: self.take_inflight_ack(),
                        seed,
                    })));
                }
                Err(error) if is_prefix_seed_missing(error.cause()) => {
                    // The gap walk cannot park (it owns neither `cause` nor the
                    // ack), so it reports the typed leaf and the park happens here.
                    // `parked` is `Some` whenever a prefix exists, since that is
                    // the same `gap` predicate that guarded the clone, but the
                    // fall-through stays instead of an `expect`.
                    let Some((order, seed)) = parked else {
                        return Err(error);
                    };
                    warn!(
                        height,
                        error = %format_args!("{:#}", error.cause()),
                        "gap-walk prefix element has no σ for its own round yet; PARKING \
                         (event-driven re-poke, no give-up timer)"
                    );
                    metrics::counter!("dpos_executor_prefix_seed_park_total").increment(1);
                    return Ok(DeriveOutcome::NeedPrefixSeed(Box::new(Deferred {
                        cause,
                        order,
                        ack: self.take_inflight_ack(),
                        seed,
                    })));
                }
                Err(error) => return Err(error),
            }
        };
        self.record_el_lag();

        // Records this height's finalization provenance: the witness round
        // `fin_proposal_round`, and whether the speculative block was reused with
        // `spec_seed_round` or re-derived. At a diverged height that discriminates
        // a seed decoupled from the agreed round (same rounds, different hash)
        // from a genuinely different round. Read before the `split_off` below
        // prunes `spec_executed`.
        tracing::info!(
            target: "dpos::derive_seed",
            height,
            path = if correctly_speculated { "finalized-spec-reuse" } else { "finalized-rederive" },
            fin_proposal_round = ?finalization_round,
            spec_seed_round = ?spec_round,
            evm_hash = %derived_hash,
            "derive-seed: finalized derive path",
        );

        // Guard #2: the `h + K` look-ahead convergence check, run only when the
        // node is behind (`last_tip_height >= h + K`), where the attested result
        // at `h + K` is already finalized and a wrong derive is caught before the
        // ack. On the catch-up path `spec_executed_hash(h)` is `None` until `h`'s
        // own FCU (reth canonicalizes on FCU, not on insert), so `result_matches`
        // is `None` and this guard stays silent; the `h − K` backward check below
        // then carries the verdict. In steady state the derive of `h` runs when
        // `h + 1` is the tip, so the gate is false.
        if behind_by_k {
            let hk = height + crate::order_block::K;
            match self.marshal.fetch_block_by_height(Height::new(hk)).await {
                Some(block_hk) => {
                    if let Some(false) = crate::order_block::result_matches(
                        block_hk.result,
                        hk,
                        self.dpos_activation_block,
                        |h| self.executed.spec_executed_hash(h),
                    ) {
                        // The attested root at `h + K` disagrees with the derived
                        // hash, so serving would fork: halt before acking.
                        return Err(Fault::fork_safety(
                            SyncReason::ResultDivergence,
                            eyre::eyre!(
                                "guard #2 at {height}: attested result at {hk} disagrees with \
                                 local executed_hash({height}); SafetyHalt — refusing to serve \
                                 a fork"
                            ),
                        ));
                    }
                }
                // The `h + K` body has not landed yet, so park: falling through
                // would reach the unconditional `ack.acknowledge()` and finalize
                // `h` with no convergence check. Returning here also keeps
                // `spec_executed[height]` intact, and the park carries σ so the
                // re-poke re-derives without lookups.
                None => {
                    return Ok(DeriveOutcome::NeedAttestation(Box::new(Deferred {
                        cause,
                        order: order_for_park.expect("order cloned under the behind_by_k gate"),
                        ack: self.take_inflight_ack(),
                        seed: finalization_seed_retry,
                    })))
                }
            }
        }

        // The finalized fork is canonical at `height`: a correct speculation keeps
        // its lead, while speculation above it built on an orphaned sibling and is
        // reset so the next notarization re-speculates forward.
        if correctly_speculated {
            self.spec_head = self.spec_head.max(height);
            // Keep the surviving lead; `split_off` drops the finalized prefix at or
            // below `height`.
            self.spec_executed = self.spec_executed.split_off(&(height + 1));
        } else {
            // Rollback: the head FCU below rolls the EL head to the finalized fork
            // at `height`, so the whole speculative suffix above it was executed
            // against an orphaned parent and is dropped. Parked notarizations above
            // `height` are kept: `try_drain_parked` re-executes them against the new
            // canonical parent.
            self.spec_head = height;
            let dropped_suffix = self.spec_executed.split_off(&(height + 1)).len();
            // `split_off` left the entries at or below `height` in place; clear
            // them too, matching the `correctly_speculated` arm.
            self.spec_executed.clear();
            if dropped_suffix > 0 {
                metrics::counter!("dpos_executor_spec_suffix_invalidated_total").increment(1);
                info!(
                    reorged_height = height,
                    dropped_suffix,
                    "speculative suffix invalidated after a head rollback (orphaned parent)"
                );
            }
        }

        self.ordering_finalized = self.ordering_finalized.max(height);

        // The attested result commits the locally-derived hash at `height − K`; a
        // present mismatch means this node would serve a fork and fails loud, while
        // `None` and a match fall through. The pre-activation window is keyed on
        // the chain activation block, not the cold-start anchor: a deep-catch-up
        // follower anchors at the live frontier yet derives the K blocks below it,
        // which are post-activation and carry real results.
        if let Some(false) = crate::order_block::result_matches(
            attested_result,
            height,
            self.dpos_activation_block,
            |h| self.executed.spec_executed_hash(h),
        ) {
            // The attested result at `height − K` disagrees with what this node
            // executed, so extending would serve a fork. Latch the halt instead of
            // exiting; recovery is the L1 validity proof.
            return Err(Fault::fork_safety(
                SyncReason::ResultDivergence,
                eyre::eyre!(
                    "result divergence at height {height}: attested result \
                     {attested_result:?} != local executed_hash; SafetyHalt — refusing to \
                     serve a forked chain"
                ),
            ));
        }

        // A finalized block was recorded, so wake any per-epoch engine spawn parked
        // on the `Inline::genesis(E)` precondition (the E−1 boundary block landing).
        // `notify_one` stores a permit, so a recording between reconciles is not
        // lost.
        self.spawn_unblocked.notify_one();
        let result_final = crate::order_block::result_final_height(
            self.ordering_finalized,
            self.anchor_finalized.0.get(),
        );

        let mut new = self.last_canonicalized;
        if result_final > new.finalized_height.get() {
            // The result-final block was derived and FCU'd K iterations ago, so its
            // canonical hash should resolve; a transient miss keeps the previous
            // finalized cursor (monotonicity over progress).
            match self.executed.spec_executed_hash(result_final) {
                Some(hash) => new = new.update_finalized(Height::new(result_final), hash),
                None => warn!(
                    result_final,
                    "result-final hash unresolved; keeping previous finalized cursor"
                ),
            }
        }
        // `safe` is the just-finalized tip: `derived_hash == executed_hash(height)`
        // and `height == ordering_finalized`, so `safe` lands about 0 blocks behind
        // head while `finalized` lags by K.
        //
        // `safe` is always canonical-findable at this FCU: `safe <= head` on the
        // same derived chain and this FCU names `head >= height`, so reth commits
        // the head-to-fork segment including `safe` before it validates `safe` and
        // `find_canonical_header(safe)` succeeds. If head canonicalization itself
        // fails, reth returns SYNCING and never reaches the safe check.
        new = new.update_safe(Height::new(height), derived_hash);
        // Move the head onto the finalized block only when speculation did not
        // already place the correct block there, or the speculative lead would be
        // rolled back. A re-derive does move the head, and `update_safe` already
        // pinned `safe` to the same hash, so `safe == head` at the reorg point.
        if !correctly_speculated {
            new = new.update_head(Height::new(height), derived_hash);
        }

        // A transient transport error retries forever (the engine stays up). A
        // semantic `Ok(Invalid)` is returned untouched rather than folded into the
        // transport error, and becomes the safety halt below: reth rejected the
        // locally-derived block, so extending would serve a chain reth disowns.
        let fcu = self.fcu_retrying_transport(new.forkchoice).await?;
        if !(fcu.is_valid() || fcu.is_syncing()) {
            // Halt (verify-only, stay observable) instead of exiting; recovery is
            // the L1 proof.
            return Err(Fault::fork_safety(
                SyncReason::ElInvalid,
                eyre::eyre!(
                    "EL reported non-valid finalize FCU: {:?}; SafetyHalt",
                    fcu.payload_status
                ),
            ));
        }

        // Postcondition: the finalized block is canonical in reth at `height`
        // before this delivery acks. A tolerated SYNCING FCU means "not applied
        // yet", not success, so until the EL serves `derived_hash` at `height`,
        // re-apply (derive + import + FCU) while staying degraded-visible. This
        // holds only above the finalized tier: the loop's only lever is re-sending
        // `new.forkchoice`, `update_head` refuses to move to a block at or below
        // `finalized_height`, and reth will not reorg below its own finalized
        // block. At or below that height the loop can only spin, so it is a
        // verdict, not a retry.
        //
        // The two arms are asymmetric. A conflicting hash is settled: re-reading
        // only delays the fork-safety verdict, since reth will not reorg below its
        // own finalized block. A hash the EL does not have yet may be the devp2p
        // backfill's by-number invisibility, so it gets a bounded re-read first and
        // only an EL that never serves it is corruption.
        let mut el_holds = self.executed.spec_executed_hash(height);
        if el_holds.is_none() && height <= new.finalized_height.get() {
            warn!(
                height,
                finalized_height = new.finalized_height.get(),
                "EL serves no block at a height it holds as finalized; re-reading before \
                 declaring corruption"
            );
            let mut visibility_retries: u32 = 0;
            while el_holds.is_none() && visibility_retries < FINALIZED_TIER_VISIBILITY_RETRIES {
                visibility_retries += 1;
                self.context.sleep(ENGINE_TRANSPORT_RETRY_BACKOFF).await;
                el_holds = self.executed.spec_executed_hash(height);
            }
        }
        if el_holds != Some(derived_hash) && height <= new.finalized_height.get() {
            // Read once: re-reading for the message could report an `EL holds X`
            // equal to the derived hash, a verdict whose own text contradicts it.
            return Err(match el_holds {
                Some(other) => Fault::fork_safety(
                    SyncReason::ResultDivergence,
                    eyre::eyre!(
                        "finalized-tier conflict at height {height}: derived {derived_hash}, \
                         EL holds {other} at or below its finalized height {}; SafetyHalt — \
                         healing this would need an FCU that reorgs reth away from the \
                         authenticated chain",
                        new.finalized_height.get()
                    ),
                ),
                None => Fault::corruption(eyre::eyre!(
                    "EL never served a block at height {height} — at or below its finalized \
                     height {} — across {FINALIZED_TIER_VISIBILITY_RETRIES} re-reads, while \
                     the node derived {derived_hash}",
                    new.finalized_height.get()
                )),
            });
        }
        let mut parent_retries: u32 = 0;
        // A prefix element's σ can land at any moment from the cert inlet, so a
        // re-walk is worth trying here; unlike the fresh-derive site, this loop
        // already holds the ack and has no park route.
        let mut seed_retries: u32 = 0;
        while self.executed.spec_executed_hash(height) != Some(derived_hash) {
            self.sync_metrics.degrade(SyncReason::FinalizeApply);
            warn!(
                height,
                %derived_hash,
                "finalized block not canonical in the EL after FCU; re-applying \
                 (derive + import + FCU) until it lands — engine stays up"
            );
            self.context.sleep(ENGINE_TRANSPORT_RETRY_BACKOFF).await;
            // The marshal still holds the block (the floor only advances on ack);
            // a transient miss just retries.
            let Some(order) = self
                .marshal
                .fetch_block_by_height(Height::new(height))
                .await
            else {
                continue;
            };
            // The same protected gap-walk the first attempt used, not a hand-rolled
            // copy: it submits the delivered element itself and leaves that
            // element's forkchoice to the caller, so its FCU stays here.
            let reapplied = match self
                .derive_finalized_with_gap_fill(order, finalization_seed_retry.clone())
                .await
            {
                Ok(hash) => hash,
                // While `inflight_ack` holds the block's `Exact`, this walk can
                // only return ForkSafety or Corruption (transport is absorbed
                // inline), so filtering on the cause is enough; a
                // `FaultClass::Transient*` disjunct would be dead code.
                Err(error)
                    if is_parent_not_visible(error.cause())
                        && parent_retries < REAPPLY_PARENT_VISIBILITY_RETRIES =>
                {
                    parent_retries += 1;
                    warn!(
                        error = %format_args!("{:#}", error.cause()),
                        height,
                        parent_retries,
                        "re-apply parent not visible yet; re-walking"
                    );
                    continue;
                }
                Err(error) if is_parent_not_visible(error.cause()) => {
                    return Err(Fault::corruption(eyre::eyre!(
                        "re-apply at height {height}: the parent never became visible after \
                         {REAPPLY_PARENT_VISIBILITY_RETRIES} re-walks; the EL is not \
                         canonicalizing what this node imports"
                    )))
                }
                Err(error)
                    if is_prefix_seed_missing(error.cause())
                        && seed_retries < REAPPLY_PARENT_VISIBILITY_RETRIES =>
                {
                    seed_retries += 1;
                    warn!(
                        error = %format_args!("{:#}", error.cause()),
                        height,
                        seed_retries,
                        "re-apply: a prefix element's σ is not in the store yet; re-walking"
                    );
                    continue;
                }
                Err(error) if is_prefix_seed_missing(error.cause()) => {
                    return Err(Fault::corruption(eyre::eyre!(
                        "re-apply at height {height}: a prefix element's σ never arrived \
                         across {REAPPLY_PARENT_VISIBILITY_RETRIES} re-walks"
                    )))
                }
                Err(error) => return Err(error),
            };
            if reapplied != derived_hash {
                return Err(Fault::corruption(eyre::eyre!(
                    "re-apply derived a different hash at height {height}: {reapplied} != \
                     {derived_hash} (non-deterministic derive)"
                )));
            }
            let fcu = self.fcu_retrying_transport(new.forkchoice).await?;
            if !(fcu.is_valid() || fcu.is_syncing()) {
                return Err(Fault::fork_safety(
                    SyncReason::ElInvalid,
                    eyre::eyre!(
                        "EL reported non-valid finalize FCU on re-apply: {:?}; SafetyHalt",
                        fcu.payload_status
                    ),
                ));
            }
        }
        self.sync_metrics.recover(SyncReason::FinalizeApply);

        // Advance the finalized-execution cursor for the result gate. Past the
        // canonical postcondition, `derived_hash` is canonical in reth at `height`
        // in both arms, so the cursor only names the height and
        // `finalized_executed_hash(height)` resolves it from reth's canonical chain
        // (reth is the tier-F store, with no separate hash map). Propose and verify
        // read it via `finalized_executed_hash(h − K)`, so a still-speculative
        // sibling can never be committed as an `OrderBlock` result; the `h − K`
        // backward check above stays the safety net. The cursor lives in the shared
        // executed store, so it survives engine restarts within the process.
        self.executed.advance_finalized(height);
        // Every finalized derive moves the committee module's anchor by one block;
        // a consumer parked on `NotReadable` for an epoch just passed learns it
        // here and nowhere else.
        (self.anchor_advanced)();

        if new != self.last_canonicalized {
            self.has_advanced_since_init = true;
        }
        self.last_canonicalized = new;
        self.reset_fcu_heartbeat_timer();

        self.take_inflight_ack().acknowledge();

        // The finalized reconcile advanced `spec_head` above and the head/safe/
        // finalized FCU has now landed, so resume parked notarized descendants and
        // prune the heights finalization made stale. This runs after the finalize
        // FCU, not at the `spec_head` advance, so a speculative FCU cannot roll the
        // just-finalized head back. The ack above already landed, so a fault here
        // concerns only the speculative tail.
        self.try_drain_parked(&cause).await?;
        Ok(DeriveOutcome::Done)
    }

    fn take_inflight_ack(&mut self) -> Exact {
        self.inflight_ack
            .take()
            .expect("inflight ack set at try_derive entry")
    }

    /// Derives `[first_missing ..= delivered.height]` as one fallible range and
    /// returns the derived hash at the delivered height.
    ///
    /// One range with one `Ok`/`Err` exit: the delivered height is structurally
    /// identical to a prefix element, so a caller catching an invisible parent
    /// need not remember a second site. The target's σ is the caller's
    /// `delivered_seed` and is never re-looked-up — the caller may have parked
    /// with it, and a re-lookup could miss where the parked value derives.
    ///
    /// A prefix σ miss on a beacon-active round surfaces the typed
    /// [`PrefixSeedMissing`](crate::application::PrefixSeedMissing) leaf: this
    /// call holds neither `cause` nor `ack`, so its caller owns the park and
    /// returns [`DeriveOutcome::NeedPrefixSeed`] instead of killing the actor.
    /// The class stays `Corruption` per the fault-class invariant while
    /// `inflight_ack` is held; the class is never reached, the cause is. A
    /// missing block stays fatal, and a re-walk is idempotent because derived
    /// prefix heights advance `first_missing`.
    ///
    /// The landing re-check, canonicalization FCU, gap telemetry and result
    /// cross-check apply to prefix elements only: `try_derive` re-checks the
    /// delivered element in its postcondition loop, so repeating them here would
    /// double every steady-state block's EL round-trips.
    async fn derive_finalized_with_gap_fill(
        &mut self,
        delivered: OrderBlock,
        mut delivered_seed: Option<crate::beacon::Seed>,
    ) -> Result<B256, Fault> {
        let target = delivered.height;
        let mut first_missing = target;
        let mut parent_hash = loop {
            if first_missing == 0 {
                return Err(Fault::corruption(eyre::eyre!(
                    "derive gap reaches height 0 — no executed ancestor"
                )));
            }
            if let Some(hash) = self.executed.spec_executed_hash(first_missing - 1) {
                break hash;
            }
            first_missing -= 1;
        };
        // Moved, not cloned, so the delivered block's tx list stays off the
        // steady-state hot path and the target is never re-fetched from the
        // marshal (`first_missing == target` must stay zero-marshal).
        let mut delivered = Some(delivered);
        if first_missing != target {
            info!(
                first_missing,
                target, "deriving missing prefix from marshal before the delivered block"
            );
        }
        for h in first_missing..=target {
            let (order, seed) = if h == target {
                (
                    delivered
                        .take()
                        .expect("taken exactly once, at h == target"),
                    delivered_seed.take(),
                )
            } else {
                let order = self
                    .marshal
                    .fetch_block_by_height(Height::new(h))
                    .await
                    .ok_or_else(|| {
                        eyre::eyre!("derive gap: marshal has no ordering artifact at height {h}")
                    })?;
                let seed = match self.seed_at_own_round(h, order.proposal_view) {
                    OwnRoundSeed::Present(seed) => Some(seed),
                    OwnRoundSeed::Inactive => None,
                    OwnRoundSeed::Missing => {
                        return Err(Fault::corruption(
                            eyre::eyre!(crate::application::PrefixSeedMissing {
                                height: h,
                                proposal_view: order.proposal_view,
                            })
                            .wrap_err(
                                "derive gap: the walk cannot hold, and deriving with the \
                                 digest fallback would silently fork",
                            ),
                        ));
                    }
                };
                (order, seed)
            };
            // Captured before `order` is consumed: a gap block carries its own
            // committee-attested `result`, cross-checked below like the delivered
            // block — without it a wrong result would be imported unchecked.
            let attested_result = order.result;
            let gap_seed_round = seed.as_ref().map(|s| s.target_round);
            let el_apply_started = std::time::Instant::now();
            let derived = self
                .deriver
                .derive_and_execute(order, parent_hash, seed)
                .await
                .wrap_err_with(|| match h == target {
                    true => "derive_and_execute failed".to_owned(),
                    false => format!("gap derivation failed at height {h}"),
                })?;
            parent_hash = derived.evm_hash();
            if h == target {
                // The delivered element discards the transport flag the prefix
                // arm checks: `try_derive` re-checks this block's landing in its
                // postcondition loop, so `Ok(false)` is retried by the caller
                // rather than killing the actor.
                self.submit_finalized_payload(derived).await?;
                metrics::histogram!("dpos_derive_el_apply_duration_seconds", "path" => "finalized")
                    .record(el_apply_started.elapsed().as_secs_f64());
                break;
            }
            tracing::info!(
                target: "dpos::derive_seed",
                height = h,
                path = "finalized-missing-prefix",
                fin_proposal_round = ?gap_seed_round,
                evm_hash = %parent_hash,
                "derive-seed: gap-walk derive path",
            );
            // The walk advances `parent_hash` with no later landing re-check, so
            // a non-landed import must end the walk here with the honest cause;
            // otherwise the next iteration's derive failure masks it. No retry
            // loop: re-entry is idempotent, since derived prefix heights advance
            // `first_missing`.
            if !self.submit_finalized_payload(derived).await? {
                // `Corruption` though the cause was a transport blip: a class the
                // router continues on would orphan the `Exact` held in
                // `inflight_ack`, so the walk's only dispositions while an ack is
                // in flight are "park forever" or "die loudly".
                return Err(Fault::corruption(eyre::eyre!(
                    "gap-walk import at height {h} hit an engine-API transport failure \
                     (block not landed); aborting the walk — a re-entry re-walks the \
                     idempotent prefix"
                )));
            }
            // The next derive reads the parent by hash, and an import is only in
            // reth's tree-private state until an FCU canonicalizes it. Built
            // literally rather than through `update_head`, which no-ops when
            // `height <= finalized_height` — reachable right after a re-jump,
            // when this walk runs. The response is not inspected: Valid and
            // "parent is visible" diverge, so the next derive is the judge.
            if let Err(error) = self
                .beacon_engine
                .fork_choice_updated(ForkchoiceState {
                    head_block_hash: parent_hash,
                    safe_block_hash: parent_hash,
                    finalized_block_hash: self.last_canonicalized.forkchoice.finalized_block_hash,
                })
                .pace_el_call(self.fcu_pace)
                .await
            {
                // A transport failure is absorbed — the next derive is the judge.
                // A rejected forkchoice state is not: the finalized hash it names
                // is unresolvable in reth, and every later walk re-sends it.
                match error.fault_class() {
                    FaultClass::TransientExternal(_) => warn!(
                        height = h,
                        error = %format_args!("{error:#}"),
                        "gap-walk canonicalization FCU failed; the next derive will \
                         report the parent as missing and the walk will park"
                    ),
                    class => {
                        return Err(Fault::new(
                            class,
                            eyre::eyre!(
                                "gap-walk canonicalization FCU at height {h} rejected by the \
                                 EL boundary: {error}"
                            ),
                        ))
                    }
                }
            }
            // Trustless result cross-check keyed on the chain activation block,
            // not the cold-start anchor: a present-and-mismatched hash means this
            // node would serve a fork. Pre-activation blocks pass; an absent
            // ancestor falls through until it is locally resolved.
            if let Some(false) = crate::order_block::result_matches(
                attested_result,
                h,
                self.dpos_activation_block,
                |q| self.executed.spec_executed_hash(q),
            ) {
                return Err(Fault::fork_safety(
                    SyncReason::ResultDivergence,
                    eyre::eyre!(
                        "result divergence at gap height {h}: attested result \
                         {attested_result:?} != local executed_hash; SafetyHalt — refusing to \
                         serve a forked chain"
                    ),
                ));
            }
        }
        Ok(parent_hash)
    }

    /// Imports the derived block into the EL. Valid is the expected steady state
    /// and Syncing is tolerated for the cold-start/rejoin window; only an invalid
    /// status is fatal, since under the new_payload fallback it means local
    /// derivation diverged from reth's re-execution.
    ///
    /// Returns `Ok(false)` when a transport failure was degraded — the block did
    /// not land. A caller that re-checks landing may ignore the flag; a caller
    /// that advances on the derived hash without one (the gap-walk) must check
    /// it, or a death one iteration later masks the transport cause.
    async fn submit_finalized_payload(&mut self, derived: D::Derived) -> Result<bool, Fault> {
        // Single chokepoint for all three derive paths; recorded before `derived`
        // moves into the EL.
        match derived.beacon_active() {
            Some(true) => self.metrics.seed_active.inc(),
            Some(false) => self.metrics.digest_fallback.inc(),
            None => 0,
        };
        // The verdict rides in `Ok`, transport in `Err(EngineError)`.
        let status = match self
            .beacon_engine
            .import_derived(derived)
            .pace_el_call(self.fcu_pace)
            .await
        {
            Ok(status) => {
                self.sync_metrics.recover(SyncReason::EngineRetry);
                status
            }
            // An import transport error is degraded and counted, not fatal: the
            // block did not land, so the caller's reconvergence retries. The
            // in-process engine channel cannot be reopened by a re-send, so the
            // disposition is not an in-place retry.
            Err(error) if matches!(error.fault_class(), FaultClass::TransientExternal(_)) => {
                self.sync_metrics.degrade(SyncReason::EngineRetry);
                self.sync_metrics.engine_transient_retry.inc();
                warn!(
                    error = %error,
                    "transient engine-API import transport error; degraded + deferring to \
                     reconvergence (engine stays up — Decision A, no self-crash)"
                );
                return Ok(false);
            }
            Err(error) => {
                return Err(Fault::new(
                    error.fault_class(),
                    eyre::eyre!("derived-block import rejected by the EL boundary: {error}"),
                ))
            }
        };
        if !(status.is_valid() || status.is_syncing()) {
            // The latch is engaged by the router, not here: speculative callers do
            // not propagate this `Err`, so a node latched here would keep driving
            // reth.
            return Err(Fault::fork_safety(
                SyncReason::ElInvalid,
                eyre::eyre!(
                    "EL rejected derived block (local derivation diverged?): `{status:?}`; \
                     SafetyHalt"
                ),
            ));
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{digest::Digest, order_block::K};
    use alloy_consensus::{Block as AlloyBlock, BlockBody, Header as AlloyHeader};
    use alloy_primitives::{Bytes, U256};
    use alloy_rpc_types_engine::{ForkchoiceUpdated, PayloadStatus, PayloadStatusEnum};
    use commonware_runtime::{deterministic, Runner as _};
    use reth_ethereum_primitives::TransactionSigned;
    use reth_primitives_traits::SealedBlock as RethSealed;
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };

    type RethExecBlock = RethSealed<reth_ethereum_primitives::Block>;

    thread_local! {
        /// The σ store the default [`Fixture`] serves from and the block helpers
        /// record into. Thread-local because the helpers are free functions with
        /// no fixture in hand: one `#[test]` runs per thread, so a round recorded
        /// by one test can never answer another's lookup.
        static FIXTURE_SEEDS: crate::beacon::testing::SeedStore =
            crate::beacon::testing::SeedStore::new();
    }

    /// The fixture's beacon: the shipped [`crate::beacon::Beacon`] over a real
    /// seed index, so a test exercises the production `seed` and `subscribe`
    /// rather than a stub that happens to agree today.
    fn beacon_over(seeds: crate::beacon::testing::SeedStore) -> Arc<dyn crate::beacon::Beacon> {
        crate::beacon::testing::LiveBeacon::build(crate::beacon::testing::LiveBeaconConfig {
            seeds,
            keys: crate::beacon::testing::keyless_index(),
            ceremony: Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new())),
            acquire: None,
            metrics: crate::beacon::testing::BeaconMetrics::default(),
            chain_id: 0,
            artifacts: crate::beacon::testing::ArtifactStore::new(),
            geometry: tokio::sync::watch::channel(Some((0, 1))).1,
        })
    }

    /// Records the canonical σ for a block proposed at `view` of the first
    /// beacon-active epoch.
    ///
    /// σ is a pure function of the round, so the store doubles as the memo that
    /// keeps threshold recovery to once per round per thread. A test that
    /// overrides the epocher builds its own store: this one cannot name its
    /// epochs.
    fn record_fixture_seed(view: u64) {
        let round = active_round(view);
        FIXTURE_SEEDS.with(|seeds| {
            if seeds.seed(round).is_none() {
                seeds.record(real_witness(round));
            }
        });
    }

    fn sample_order(parent: Digest, height: u64, result: B256) -> OrderBlock {
        OrderBlock {
            parent,
            height,
            proposal_view: 0,
            timestamp: 1_700_000_000 + height,
            gas_limit: 30_000_000,
            extra_data: Bytes::new(),
            result,
            txs: Vec::new(),
            equivocation: None,
        }
    }

    /// A real 2f+1 finalization certificate over `block`'s digest, under a
    /// throwaway four-member committee built once per process.
    ///
    /// It exists so [`FakeMarshal`] can answer `BlockFetcher::pair_at` with a
    /// `Finalization` value, which the jump target's type demands; nothing here
    /// verifies it.
    fn canned_finalization(block: &OrderBlock) -> Finalization<BlsScheme, Digest> {
        use commonware_codec::DecodeExt as _;
        use commonware_consensus::{
            simplex::types::{Finalize, Proposal},
            types::{Epoch, Round, View},
        };
        use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
        use commonware_math::algebra::Random as _;
        use commonware_parallel::Sequential;
        use commonware_utils::{ordered::BiMap, TryCollect as _};
        use fluentbase_bls::{
            fluent_namespace,
            keys::ValidatorBlsKeypair,
            scheme::{build_signer, build_verifier},
            BlsPubkey,
        };
        use rand_08::rngs::StdRng;
        use rand_core::SeedableRng as _;

        struct Canned {
            signers: Vec<BlsScheme>,
            verifier: BlsScheme,
        }
        static CANNED: std::sync::OnceLock<Canned> = std::sync::OnceLock::new();
        let c = CANNED.get_or_init(|| {
            const N: usize = 4;
            let mut rng = StdRng::seed_from_u64(0xFA1E);
            let peer_sks: Vec<_> = (0..N)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let bls_kps: Vec<_> = (0..N)
                .map(|_| ValidatorBlsKeypair::generate(&mut rng))
                .collect();
            let bimap: BiMap<PeerPubkey, BlsPubkey> = peer_sks
                .iter()
                .zip(bls_kps.iter())
                .map(|(p, b)| {
                    (
                        p.public_key(),
                        BlsPubkey::decode(b.public_bytes().as_slice()).unwrap(),
                    )
                })
                .try_collect()
                .unwrap();
            let ns = fluent_namespace(20_994);
            Canned {
                signers: bls_kps
                    .iter()
                    .map(|kp| build_signer(&ns, bimap.clone(), kp, 0, None).expect("member"))
                    .collect(),
                verifier: build_verifier(&ns, bimap, 0, None),
            }
        });
        let round = Round::new(Epoch::new(0), View::new(block.height));
        let prop = Proposal::new(round, View::new(block.height), block.digest());
        let finalizes: Vec<_> = c
            .signers
            .iter()
            .take(3)
            .map(|s| Finalize::sign(s, prop.clone()).expect("sign"))
            .collect();
        Finalization::from_finalizes(&c.verifier, finalizes.iter(), &Sequential).expect("quorum")
    }

    /// The next linked block after `parent`. It carries no σ; a test that needs
    /// one for a specific round files it with [`record_fixture_seed`].
    fn child_of(parent: &OrderBlock) -> OrderBlock {
        sample_order(parent.digest(), parent.height + 1, B256::ZERO)
    }

    /// The first beacon-active epoch, the one every σ-recording helper here keys
    /// on.
    fn active_epoch() -> commonware_consensus::types::Epoch {
        commonware_consensus::types::Epoch::new(
            crate::beacon::testing::DETERMINISTIC_BOOTSTRAP_EPOCH,
        )
    }

    /// The round a height whose epocher is [`beacon_active_epocher`] resolves its
    /// σ at.
    fn active_round(view: u64) -> commonware_consensus::types::Round {
        commonware_consensus::types::Round::new(
            active_epoch(),
            commonware_consensus::types::View::new(view),
        )
    }

    /// An epocher that puts heights 96..=143 — the band every fixture here
    /// anchors in — inside `DETERMINISTIC_BOOTSTRAP_EPOCH`, so `mandatory_at`
    /// answers true and σ is actually consulted.
    ///
    /// A test whose subject is σ must use this and then supply σ for every height
    /// it expects to derive; the default fixture epocher is beacon-inactive and
    /// ignores the store.
    fn beacon_active_epocher() -> crate::epocher::OriginEpocher {
        crate::epocher::OriginEpocher::new(0, std::num::NonZeroU64::new(48).expect("nonzero"))
    }

    /// Builds a self-consistent `OrderBlock` chain `(anchor+1 ..= anchor+count)`
    /// whose `result` commits the hash [`FakeDeriver`] will derive at `height − K`
    /// (zero pre-activation), so the executor's result cross-check passes.
    fn result_consistent_chain(anchor: u64, anchor_hash: B256, count: u64) -> Vec<OrderBlock> {
        let mut orders: Vec<OrderBlock> = Vec::new();
        let mut derived: BTreeMap<u64, B256> = BTreeMap::new();
        derived.insert(anchor, anchor_hash);
        let mut parent = Digest(B256::ZERO);
        let mut parent_evm = anchor_hash;
        for i in 1..=count {
            let height = anchor + i;
            let result = match height.checked_sub(K) {
                Some(h) if h >= anchor => derived[&h],
                _ => B256::ZERO,
            };
            let order = sample_order(parent, height, result);
            parent = order.digest();
            parent_evm = sealed_at(parent_evm, height, order.digest().0).hash();
            derived.insert(height, parent_evm);
            orders.push(order);
        }
        orders
    }

    /// Folds `discriminator` (the ordering digest) into `extra_data` so sibling
    /// orders at the same (parent, height) seal to distinct block hashes —
    /// required to observe a speculative rollback.
    fn sealed_at(parent: B256, number: u64, discriminator: B256) -> RethExecBlock {
        let header = AlloyHeader {
            parent_hash: parent,
            number,
            gas_limit: 30_000_000,
            timestamp: 1_700_000_000 + number,
            difficulty: U256::ZERO,
            extra_data: Bytes::from(discriminator.to_vec()),
            ..Default::default()
        };
        let body: BlockBody<TransactionSigned> = BlockBody::default();
        RethSealed::seal_slow(reth_ethereum_primitives::Block::from(AlloyBlock::new(
            header, body,
        )))
    }

    /// Folds a notarization/finalization seed into the ordering digest the way
    /// production folds `prev_randao = H(threshold-sig)` into mix_hash: two seeds
    /// for the same ordering block seal to distinct executed hashes. A `None`
    /// seed leaves the digest untouched.
    fn seed_folded_discriminator(digest: Digest, seed: &Option<crate::beacon::Seed>) -> B256 {
        match seed {
            Some(s) => alloy_primitives::keccak256(
                [
                    digest.0.as_slice(),
                    crate::beacon::prev_randao_from_seed(s).as_slice(),
                ]
                .concat(),
            ),
            None => digest.0,
        }
    }

    /// Models reth's by-hash header-index lag: a backfilled block is visible by
    /// number immediately, but the by-hash parent read the deriver performs only
    /// resolves once an FCU has canonicalized the segment. Heights ≤ `frontier`
    /// are by-hash-visible; `frontier` defaults to `u64::MAX` (lag disabled), so a
    /// test lowers it to exercise the race. Shared between `FakeChain` (read),
    /// `FakeDeriver` (gate) and `FakeBeacon` (an FCU advances it).
    #[derive(Clone)]
    struct ByHashVisibility {
        hash_height: Arc<Mutex<BTreeMap<B256, u64>>>,
        frontier: Arc<Mutex<u64>>,
        /// Hashes reth resolves no header for, whatever the frontier — the knob
        /// that keeps a re-apply re-walk failing with `ParentHeaderMissing`
        /// instead of converging, so the retry bound is reachable in a test.
        never_visible: Arc<Mutex<std::collections::BTreeSet<B256>>>,
    }

    impl Default for ByHashVisibility {
        fn default() -> Self {
            Self {
                hash_height: Arc::new(Mutex::new(BTreeMap::new())),
                frontier: Arc::new(Mutex::new(u64::MAX)),
                never_visible: Arc::default(),
            }
        }
    }

    impl ByHashVisibility {
        fn register(&self, height: u64, hash: B256) {
            self.hash_height.lock().unwrap().insert(hash, height);
        }
        fn hide(&self, hash: B256) {
            self.never_visible.lock().unwrap().insert(hash);
        }
        /// `true` iff reth would resolve `header(hash)`. An untracked hash counts
        /// as visible — only the explicitly modelled segment participates.
        fn visible(&self, hash: B256) -> bool {
            if self.never_visible.lock().unwrap().contains(&hash) {
                return false;
            }
            let frontier = *self.frontier.lock().unwrap();
            if frontier == u64::MAX {
                return true;
            }
            match self.hash_height.lock().unwrap().get(&hash) {
                Some(&h) => h <= frontier,
                None => true,
            }
        }
        /// Models an FCU(head), which canonicalizes `[.., head]` by hash: raises
        /// the frontier to the head's tracked height (no-op for an untracked one).
        fn canonicalize_up_to(&self, head: B256) {
            if let Some(&h) = self.hash_height.lock().unwrap().get(&head) {
                let mut f = self.frontier.lock().unwrap();
                *f = (*f).max(h);
            }
        }
        fn set_frontier(&self, to: u64) {
            *self.frontier.lock().unwrap() = to;
        }
    }

    /// Shared height→hash canonical map: the deriver inserts on derive
    /// (modelling new_payload + FCU canonicalization) and `ExecutedChain` reads,
    /// mirroring the provider-backed production impl. `vis` carries the by-hash
    /// visibility lag model.
    #[derive(Clone, Default)]
    struct FakeChain {
        canonical: Arc<Mutex<BTreeMap<u64, B256>>>,
        /// The finalized-execution cursor the executor advances past the canonical
        /// postcondition; mirrors the provider-backed production store (tier-F =
        /// canonical chain below the cursor).
        finalized: crate::application::FinalizedCursor,
        vis: ByHashVisibility,
        /// Models reth silently dropping an `InsertExecutedBlock` at a height that
        /// already has a different canonical hash (a same-height sibling reorg):
        /// the canonical map keeps the old hash and the counter decrements.
        sibling_drops: Arc<Mutex<u32>>,
        /// When set, the deriver stops landing blocks into the canonical map;
        /// landing happens only on a successful `import_derived` (see
        /// `FakeBeacon::land_chain`), mirroring the real EL where a failed
        /// `InsertExecutedBlock` leaves nothing behind. Default off.
        land_on_import: Arc<std::sync::atomic::AtomicBool>,
        /// Heights the EL serves nothing for, one decrement per by-number read —
        /// the window where a block is landed but not yet index-visible.
        /// `u32::MAX` models an EL that never serves it.
        missing_reads: Arc<Mutex<BTreeMap<u64, u32>>>,
    }

    impl ExecutedChain for FakeChain {
        fn executed_tip(&self) -> u64 {
            self.canonical
                .lock()
                .unwrap()
                .keys()
                .next_back()
                .copied()
                .unwrap_or(0)
        }
        fn spec_executed_hash(&self, height: u64) -> Option<B256> {
            if let Some(left) = self.missing_reads.lock().unwrap().get_mut(&height) {
                if *left > 0 {
                    *left -= 1;
                    return None;
                }
            }
            self.canonical.lock().unwrap().get(&height).copied()
        }
        fn finalized_executed_hash(&self, height: u64) -> Option<B256> {
            self.finalized
                .resolve(height, |h| self.spec_executed_hash(h))
        }
        fn advance_finalized(&self, height: u64) {
            self.finalized.advance(height);
        }
    }

    type SeedsSeen = Arc<Mutex<Vec<(u64, Option<crate::beacon::Seed>)>>>;

    #[derive(Clone)]
    struct FakeDeriver {
        chain: FakeChain,
        /// Records the (height, seed) passed to each `derive_and_execute`, so a
        /// test can assert the cert-recovered seed reaches the deriver. `Vec`
        /// behind the shared `Arc` so it survives a clone.
        seeds_seen: SeedsSeen,
        /// Heights whose next `derive_and_execute` fails once with a plain `eyre`
        /// error — the transient derive failure the speculative path must survive
        /// without taking the node down.
        derive_fail_once: Arc<Mutex<std::collections::BTreeSet<u64>>>,
    }

    impl FakeDeriver {
        fn new(chain: FakeChain) -> Self {
            Self {
                chain,
                seeds_seen: Arc::new(Mutex::new(Vec::new())),
                derive_fail_once: Arc::default(),
            }
        }
    }

    impl DerivedBlockBuilder for FakeDeriver {
        type Derived = RethExecBlock;

        async fn derive_and_execute(
            &self,
            order: OrderBlock,
            parent_evm_hash: B256,
            seed: Option<crate::beacon::Seed>,
        ) -> eyre::Result<RethExecBlock> {
            // Fold before `seed` moves into `seeds_seen`, so notarize-round vs
            // finalize-round divergence stays observable.
            let discriminator = seed_folded_discriminator(order.digest(), &seed);
            self.seeds_seen.lock().unwrap().push((order.height, seed));
            if self.derive_fail_once.lock().unwrap().remove(&order.height) {
                return Err(eyre::eyre!(
                    "simulated transient derive failure at height {}",
                    order.height
                ));
            }
            // Models the deriver's by-hash parent read: a parent not yet canonical
            // by hash is `ParentHeaderMissing`. The default frontier makes every
            // hash visible.
            if !self.chain.vis.visible(parent_evm_hash) {
                // The real deriver returns this typed error, and
                // `is_parent_not_visible` keys on the type through the walk's
                // `wrap_err` chain: an untyped model would make the park
                // untestable.
                return Err(crate::application::ParentHeaderMissing(parent_evm_hash).into());
            }
            let sealed = sealed_at(parent_evm_hash, order.height, discriminator);
            // With `sibling_drops` armed, a same-height sibling import is silently
            // dropped: the derive succeeds but the canonical map keeps the old
            // hash — the contract violation the try_derive postcondition must
            // survive.
            {
                let mut drops = self.chain.sibling_drops.lock().unwrap();
                let dropped = *drops > 0
                    && self
                        .chain
                        .canonical
                        .lock()
                        .unwrap()
                        .get(&order.height)
                        .is_some_and(|h| *h != sealed.hash());
                if dropped {
                    *drops -= 1;
                    return Ok(sealed);
                }
            }
            // With landing gated on import, the derive alone lands nothing; a
            // successful import does.
            if self
                .chain
                .land_on_import
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Ok(sealed);
            }
            // Last writer wins, modelling a reth reorg: a sibling derived after
            // another replaces the canonical hash.
            self.chain
                .canonical
                .lock()
                .unwrap()
                .insert(order.height, sealed.hash());
            // Registered (by-number present) but not canonicalized: only an FCU
            // makes a block by-hash visible.
            self.chain.vis.register(order.height, sealed.hash());
            Ok(sealed)
        }
    }

    #[derive(Clone, Default)]
    struct FakeBeacon {
        fcu_calls: Arc<Mutex<Vec<ForkchoiceState>>>,
        new_payload_calls: Arc<Mutex<Vec<RethExecBlock>>>,
        /// Overrides the `fork_choice_updated` status; `None` is Valid. Set to
        /// drive Syncing or Invalid through the FCU gate.
        fcu_status: Arc<Mutex<Option<PayloadStatusEnum>>>,
        /// Leading FCU calls that return a transport `Err` (an RPC/channel blip)
        /// before succeeding, decremented per call — the retryable transport half
        /// of the split, distinct from a semantic `Ok(Invalid)`.
        fcu_transport_errs: Arc<Mutex<u32>>,
        /// When set, `fork_choice_updated` returns
        /// `Err(EngineError::anchor_inconsistent)`: reth processed the update and
        /// rejected the state ("unknown finalized/safe hash"). Sticky, not a
        /// countdown — the condition is structurally permanent, so classifying it
        /// as transport would retry without bound.
        fcu_anchor_inconsistent: Arc<Mutex<bool>>,
        /// How many times the arm above fired. The assertion that matters is that
        /// it stays bounded.
        fcu_anchor_rejections: Arc<Mutex<u32>>,
        /// Overrides the `import_derived` status; `None` is Valid.
        import_status: Arc<Mutex<Option<PayloadStatusEnum>>>,
        /// Leading `import_derived` calls that return a transport `Err` (a closed
        /// engine channel) before succeeding, decremented per call — the executor
        /// must degrade and defer, not die.
        import_transport_errs: Arc<Mutex<u32>>,
        /// `Some(chain)` when landing is gated on a successful import (see
        /// `FakeChain::land_on_import`): a `Valid` import inserts the block into
        /// the canonical map and visibility. `None` is the default land-at-derive
        /// model.
        land_chain: Arc<Mutex<Option<FakeChain>>>,
        /// Shared with `FakeChain`/`FakeDeriver`: an FCU canonicalizes `[.., head]`
        /// by hash.
        vis: ByHashVisibility,
    }

    impl BeaconEngineLike for FakeBeacon {
        type ExecutionData = RethExecBlock;

        async fn fork_choice_updated(
            &self,
            state: ForkchoiceState,
        ) -> Result<ForkchoiceUpdated, crate::fault::EngineError> {
            if *self.fcu_anchor_inconsistent.lock().unwrap() {
                *self.fcu_anchor_rejections.lock().unwrap() += 1;
                return Err(crate::fault::EngineError::anchor_inconsistent(
                    "reth rejected the forkchoice state: invalid forkchoice state",
                ));
            }
            {
                let mut errs = self.fcu_transport_errs.lock().unwrap();
                if *errs > 0 {
                    *errs -= 1;
                    // A transport blip: reth was never reached, so nothing was
                    // recorded — the caller must retry.
                    return Err(crate::fault::EngineError::transport(
                        "simulated engine-API transport blip",
                    ));
                }
            }
            self.fcu_calls.lock().unwrap().push(state);
            let status = self
                .fcu_status
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(PayloadStatusEnum::Valid);
            // Only a Valid forkchoice canonicalizes: Syncing means a backfill
            // holds the engine and the segment did not go canonical — the cause
            // the parent-visibility park exists for, which a model raising the
            // frontier here could not express.
            if status == PayloadStatusEnum::Valid {
                self.vis.canonicalize_up_to(state.head_block_hash);
            }
            Ok(ForkchoiceUpdated::from_status(status))
        }

        async fn import_derived(
            &self,
            data: RethExecBlock,
        ) -> Result<PayloadStatus, crate::fault::EngineError> {
            {
                let mut errs = self.import_transport_errs.lock().unwrap();
                if *errs > 0 {
                    *errs -= 1;
                    // A closed engine channel: nothing was imported, so the
                    // executor degrades and defers to reconvergence.
                    return Err(crate::fault::EngineError::transport(
                        "simulated engine tree channel closed",
                    ));
                }
            }
            let status = self
                .import_status
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(PayloadStatusEnum::Valid);
            // With landing gated, only a Valid import lands the block; the
            // transport-failed insert above left nothing behind.
            if let Some(chain) = self.land_chain.lock().unwrap().as_ref() {
                if status == PayloadStatusEnum::Valid {
                    let (height, hash) = (data.number(), data.hash());
                    chain.canonical.lock().unwrap().insert(height, hash);
                    // Registered only: the import puts the block in reth's
                    // tree-private state; the FCU is what canonicalizes it.
                    chain.vis.register(height, hash);
                }
            }
            self.new_payload_calls.lock().unwrap().push(data);
            Ok(PayloadStatus::from_status(status))
        }
    }

    #[derive(Clone, Default)]
    struct FakeMarshal {
        canned: Arc<Mutex<BTreeMap<u64, OrderBlock>>>,
        /// Heights passed to `fetch_block_by_height`, in call order — the
        /// steady-state derive resolves σ locally and must never round-trip the
        /// marshal.
        fetched: Arc<Mutex<Vec<u64>>>,
        /// Heights passed to `hint_finalization`, in call order.
        hints: Arc<Mutex<Vec<u64>>>,
        /// Heights passed to `set_floor`, in call order (the re-jump recorder).
        floors: Arc<Mutex<Vec<u64>>>,
        /// Heights passed to `store_verified_finalization`, in call order.
        stored: Arc<Mutex<Vec<u64>>>,
        /// Interleaved `("store"|"floor", height)` trace, so ordering between the two
        /// is assertable without reasoning about two separate vectors.
        store_floor_order: Arc<Mutex<Vec<(&'static str, u64)>>>,
        /// Biased-select escape model (the `marshal_floor` stale-dispatch guard):
        /// a mailbox sender plus a canned old-range inventory, armed via
        /// [`Self::arm_stale_escape`]. On `set_floor(f)` every inventory block at
        /// height ≤ f is dispatched into the executor mailbox, modelling the acks
        /// freed by `reseed_forward`'s disposals whose slots the marshal's biased
        /// select fills with the next old blocks before it processes `SetFloor`.
        /// Escaped `Exact` waiters are retained so the guard's `acknowledge()`
        /// never hits a dropped receiver.
        dispatch: Arc<Mutex<Option<Mailbox>>>,
        stale_inventory: Arc<Mutex<Vec<OrderBlock>>>,
        escaped_waiters: Arc<Mutex<Vec<commonware_utils::acknowledgement::ExactWaiter>>>,
        /// Set to make [`BlockFetcher::pair_at`] answer `None` at every height.
        /// The default models the real invariant: a marshal emits `Update::Tip(h)`
        /// only from `store_finalization`, which has just written the pair at `h`,
        /// so every tip has a pair behind it. Setting it models a heartbeat
        /// re-poke replaying a tip the floor has since moved past.
        ///
        /// The pair is synthesized because nothing in the re-jump tests reads the
        /// target's content; they script the outcome.
        archive_empty: Arc<Mutex<bool>>,
    }

    impl FakeMarshal {
        /// Arm the biased-select escape: `inventory` blocks at height ≤ the floor
        /// are dispatched into `mailbox` when `set_floor` runs.
        fn arm_stale_escape(&self, mailbox: Mailbox, inventory: Vec<OrderBlock>) {
            *self.dispatch.lock().unwrap() = Some(mailbox);
            *self.stale_inventory.lock().unwrap() = inventory;
        }
    }

    impl BlockFetcher for FakeMarshal {
        async fn fetch_block_by_height(&self, height: Height) -> Option<OrderBlock> {
            self.fetched.lock().unwrap().push(height.get());
            self.canned.lock().unwrap().get(&height.get()).cloned()
        }
        async fn fetch_block_by_digest(&self, digest: crate::digest::Digest) -> Option<OrderBlock> {
            self.canned
                .lock()
                .unwrap()
                .values()
                .find(|o| o.digest() == digest)
                .cloned()
        }
        async fn pair_at(
            &self,
            height: Height,
        ) -> Option<(Finalization<BlsScheme, Digest>, OrderBlock)> {
            if *self.archive_empty.lock().unwrap() {
                return None;
            }
            let block = sample_order(Digest(B256::ZERO), height.get(), B256::ZERO);
            Some((canned_finalization(&block), block))
        }
        async fn hint_finalization(&self, height: Height, _targets: NonEmptyVec<PeerPubkey>) {
            self.hints.lock().unwrap().push(height.get());
        }
        async fn set_floor(&self, height: Height) {
            let floor = height.get();
            if let Some(mailbox) = self.dispatch.lock().unwrap().clone() {
                let escaped: Vec<OrderBlock> = {
                    let mut inv = self.stale_inventory.lock().unwrap();
                    let (escape, keep): (Vec<_>, Vec<_>) =
                        inv.drain(..).partition(|o| o.height <= floor);
                    *inv = keep;
                    escape
                };
                for order in escaped {
                    let (ack, waiter) = Exact::handle();
                    self.escaped_waiters.lock().unwrap().push(waiter);
                    let msg = Message {
                        cause: Span::current(),
                        command: Command::Finalize(Box::new(Update::Block(order, ack))),
                    };
                    let _ = mailbox.send(msg);
                }
            }
            self.floors.lock().unwrap().push(floor);
            self.store_floor_order
                .lock()
                .unwrap()
                .push(("floor", floor));
        }

        async fn store_verified_finalization(
            &self,
            _round: Round,
            block: OrderBlock,
            _finalization: Finalization<BlsScheme, Digest>,
        ) {
            let height = block.height;
            self.canned.lock().unwrap().insert(height, block);
            self.stored.lock().unwrap().push(height);
            self.store_floor_order
                .lock()
                .unwrap()
                .push(("store", height));
        }
    }

    struct Fixture {
        chain: FakeChain,
        beacon: FakeBeacon,
        deriver: FakeDeriver,
        marshal: FakeMarshal,
        anchor_hash: B256,
        /// Re-jump callback injected into the built actor's `Config`. `None` by
        /// default; the re-jump tests set it via `with_re_jump`.
        re_jump: Arc<Mutex<Option<ReJump>>>,
        /// Boundary-seeding seam injected into the built actor's `Config`. `None` by
        /// default; the seeding tests set it via `with_boundary_fetch`.
        boundary_fetch: Arc<Mutex<Option<crate::cert_follow::BoundaryFetchFn>>>,
        /// Epoch-entry seam injected into the built actor's `Config`. A no-op by
        /// default; the entry tests record into a sink via `with_boundary_enter`.
        boundary_enter: Arc<dyn Fn(u64) + Send + Sync>,
        /// Inert by default; the read-floor test records into a sink via
        /// `with_boundary_read_floor`.
        boundary_read_floor: BoundaryReadFloorFn,
        /// Self-heal metrics handle the built actor's `Config` carries, exposed so
        /// the tests assert the `engine_retry` gauge and counter.
        sync_metrics: SyncMetrics,
        /// Fork-safety latch the built actor's `Config` carries, exposed so the
        /// tests assert it engages on divergence or EL-Invalid.
        safety_halt: crate::sync_metrics::SafetyHalt,
        /// FCU-heartbeat interval. Default 60 s so heartbeats never interfere with
        /// fast tests; a park test that relies on the heartbeat re-poke lowers it.
        fcu_heartbeat: Duration,
        /// Randomness handed to the built actor. Default: the real provider over
        /// the thread's `FIXTURE_SEEDS` store, so σ resolves by round as in
        /// production. Replace it via `with_seed_store`, with an empty store to
        /// pin a miss. The default epocher is beacon-inactive, so the store is
        /// consulted only under [`beacon_active_epocher`].
        randomness: std::sync::Arc<dyn crate::beacon::Beacon>,
        /// Block→epoch map handed to the built actor. Default: a single huge
        /// epoch so every test height maps to epoch 0, below
        /// `DETERMINISTIC_BOOTSTRAP_EPOCH` and therefore beacon-inactive, where
        /// the agreed derivation is `None`. A test whose subject is σ overrides
        /// with [`beacon_active_epocher`].
        epocher: crate::epocher::OriginEpocher,
        /// Restart-seed override for `last_execution_finalized_height` (the reth
        /// head). `None` uses `anchor_height`, i.e. head == acked. The
        /// `ordering_finalized`-seed test decouples the two (head ≫ acked with a
        /// speculative tail) to pin that the cursor seeds from the acked cursor.
        last_execution: Option<u64>,
        /// `Config::initial_marshal_floor`. Zero everywhere except the ladder-step
        /// test, which needs a floor below the tip to show the step is judged
        /// against the floor rather than the tip.
        marshal_floor: u64,
    }

    impl Fixture {
        /// Anchor at `anchor_height` already canonical (reth holds it).
        fn new(anchor_height: u64) -> Self {
            let chain = FakeChain::default();
            let anchor_hash = B256::repeat_byte(0xA0);
            chain
                .canonical
                .lock()
                .unwrap()
                .insert(anchor_height, anchor_hash);
            chain.vis.register(anchor_height, anchor_hash);
            // Shared so a beacon FCU advances exactly the frontier the deriver
            // gates on.
            let beacon = FakeBeacon {
                vis: chain.vis.clone(),
                ..Default::default()
            };
            // The latch shares the `SyncMetrics` gauge family, as production wires
            // it, so a test reading `fx.sync_metrics` sees the gauge the latch
            // raised.
            let sync_metrics = SyncMetrics::default();
            let safety_halt = crate::sync_metrics::SafetyHalt::new(sync_metrics.clone());
            Self {
                deriver: FakeDeriver::new(chain.clone()),
                chain,
                beacon,
                marshal: FakeMarshal::default(),
                anchor_hash,
                re_jump: Arc::new(Mutex::new(None)),
                boundary_fetch: Arc::new(Mutex::new(None)),
                boundary_enter: Arc::new(|_| {}),
                boundary_read_floor: Arc::new(|_| Box::pin(async {})),
                sync_metrics,
                safety_halt,
                fcu_heartbeat: Duration::from_secs(60),
                randomness: beacon_over(FIXTURE_SEEDS.with(|seeds| seeds.clone())),
                epocher: crate::epocher::OriginEpocher::new(
                    0,
                    std::num::NonZeroU64::new(1 << 40).expect("nonzero"),
                ),
                last_execution: None,
                marshal_floor: 0,
            }
        }

        /// Overrides `last_execution_finalized_height` (the reth head seed),
        /// decoupling it from the anchor. Set before `build`.
        fn with_last_execution(mut self, height: u64) -> Self {
            self.last_execution = Some(height);
            self
        }

        /// Boot with a non-zero marshal floor — a node that has jumped, so the
        /// ladder step's no-op rule (`HintFinalized` skipped at
        /// `height <= last_processed_height`) has a real boundary. Set before
        /// `build`.
        fn with_marshal_floor(mut self, height: u64) -> Self {
            self.marshal_floor = height;
            self
        }

        /// Overrides the block→epoch map (the epoch-boundary tests need small
        /// epochs). Set before `build`.
        fn with_epocher(mut self, epocher: crate::epocher::OriginEpocher) -> Self {
            self.epocher = epocher;
            self
        }

        /// Replaces the default store with `store` — the test's own σ source, and
        /// (empty) the way to pin a store miss. Set before `build`.
        fn with_seed_store(mut self, store: crate::beacon::testing::SeedStore) -> Self {
            self.randomness = beacon_over(store);
            self
        }

        /// Injects the re-jump callback the built actor's `Config` will carry. Set
        /// before `build`.
        fn with_re_jump(self, re_jump: ReJump) -> Self {
            *self.re_jump.lock().unwrap() = Some(re_jump);
            self
        }

        fn with_boundary_fetch(self, fetch: crate::cert_follow::BoundaryFetchFn) -> Self {
            *self.boundary_fetch.lock().unwrap() = Some(fetch);
            self
        }

        /// Records every epoch-entry height the built actor drives into `sink`. Set
        /// before `build`.
        fn with_boundary_enter(mut self, sink: Arc<Mutex<Vec<u64>>>) -> Self {
            self.boundary_enter = Arc::new(move |h| sink.lock().unwrap().push(h));
            self
        }

        /// Record every read-floor height the built actor publishes into `sink`.
        /// Sharing one sink with [`Self::with_boundary_enter`] records the order of
        /// the two seams. Set before `build`.
        fn with_boundary_read_floor(mut self, sink: Arc<Mutex<Vec<u64>>>) -> Self {
            self.boundary_read_floor = Arc::new(move |h| {
                let sink = sink.clone();
                Box::pin(async move { sink.lock().unwrap().push(h) })
            });
            self
        }

        /// Model the real EL, where only a successful `import_derived` lands a
        /// block: the deriver stops writing the canonical map, so a failed import
        /// leaves the block un-landed and the postcondition loop has something real
        /// to converge on. Set before `build`.
        fn gate_landing_on_import(&self) {
            self.chain
                .land_on_import
                .store(true, std::sync::atomic::Ordering::SeqCst);
            *self.beacon.land_chain.lock().unwrap() = Some(self.chain.clone());
        }

        /// Shrink the FCU-heartbeat interval so a park test that depends on the
        /// heartbeat re-poke resolves in a few virtual ms (≈ real ms) instead of
        /// real seconds. Set before `build`.
        fn with_fcu_heartbeat(mut self, interval: Duration) -> Self {
            self.fcu_heartbeat = interval;
            self
        }

        fn build(
            &self,
            ctx: deterministic::Context,
            anchor_height: u64,
            last_consensus: u64,
        ) -> (
            Actor<deterministic::Context, FakeBeacon, FakeDeriver, FakeChain, FakeMarshal>,
            Mailbox,
        ) {
            // The fixtures build chains anchored at activation, so the cross-check
            // window is unchanged by the anchor/activation split.
            self.build_with_activation(ctx, anchor_height, anchor_height, last_consensus)
        }

        /// `build` with the cold-start anchor decoupled from the chain activation
        /// (the deep-catch-up follower case: anchor ≫ activation).
        fn build_with_activation(
            &self,
            ctx: deterministic::Context,
            anchor_height: u64,
            activation: u64,
            last_consensus: u64,
        ) -> (
            Actor<deterministic::Context, FakeBeacon, FakeDeriver, FakeChain, FakeMarshal>,
            Mailbox,
        ) {
            let anchor_hash = self
                .chain
                .spec_executed_hash(anchor_height)
                .expect("anchor must be canonical");
            Actor::init(
                ctx,
                Config {
                    beacon_engine: self.beacon.clone(),
                    deriver: self.deriver.clone(),
                    executed: self.chain.clone(),
                    marshal: self.marshal.clone(),
                    fcu_heartbeat_interval: self.fcu_heartbeat,
                    last_consensus_finalized_height: Height::new(last_consensus),
                    last_execution_finalized_height: self.last_execution.unwrap_or(anchor_height),
                    initial_finalized: (Height::new(anchor_height), anchor_hash),
                    initial_head: (Height::new(anchor_height), anchor_hash),
                    initial_marshal_floor: self.marshal_floor,
                    boundary_fetch: self.boundary_fetch.lock().unwrap().clone(),
                    boundary_enter: self.boundary_enter.clone(),
                    boundary_read_floor: self.boundary_read_floor.clone(),
                    dpos_activation_block: activation,
                    fcu_pace: Duration::from_millis(0),
                    peers_for_finalization: std::sync::Arc::new(dummy_peers),
                    metrics: ExecutorMetrics::default(),
                    sync_metrics: self.sync_metrics.clone(),
                    safety_halt: self.safety_halt.clone(),
                    spawn_unblocked: std::sync::Arc::new(tokio::sync::Notify::new()),
                    re_jump: self.re_jump.lock().unwrap().clone(),
                    randomness: self.randomness.clone(),
                    epocher: self.epocher.clone(),
                    // No committee module in these fixtures, so the wake-up has
                    // nothing to wake.
                    anchor_advanced: std::sync::Arc::new(|| {}),
                },
            )
        }
    }

    /// One deterministic dummy peer for the finalization-hint target set;
    /// FakeMarshal records the call and ignores the targets' contents.
    fn dummy_peers() -> Option<NonEmptyVec<PeerPubkey>> {
        use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
        let pk = Ed25519PrivateKey::from_seed(99).public_key();
        NonEmptyVec::try_from(vec![pk]).ok()
    }

    /// A real recovered threshold seed for `round`; the executor passes it through
    /// without re-verifying, so any valid `Seed` suffices.
    fn real_seed(round: commonware_consensus::types::Round) -> crate::beacon::Seed {
        use commonware_cryptography::bls12381::{dkg::deal_anonymous, primitives::variant::MinSig};
        use commonware_utils::{test_rng, N3f1, NZU32};
        use fluentbase_bls::beacon::{recover_seed, seed_namespace, sign_seed_partial};
        let mut rng = test_rng();
        let (sharing, shares) =
            deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(5));
        let ns = seed_namespace(b"fluent-test");
        let partials: Vec<_> = shares
            .iter()
            .map(|s| sign_seed_partial(s, &ns, round))
            .collect();
        crate::beacon::Seed {
            target_round: round,
            signature: recover_seed::<N3f1>(&sharing, &partials).expect("recover seed"),
        }
    }

    /// The witness [`SeedStore::record`] takes for the same seed; the deal is
    /// deterministic, so this checks against the key [`real_seed`] signed under.
    fn real_witness(
        round: commonware_consensus::types::Round,
    ) -> crate::beacon::testing::VerifiedSeed {
        use commonware_cryptography::bls12381::{dkg::deal_anonymous, primitives::variant::MinSig};
        use commonware_utils::{test_rng, N3f1, NZU32};
        use fluentbase_bls::beacon::seed_namespace;
        let mut rng = test_rng();
        let (sharing, _) = deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(5));
        crate::beacon::testing::PkOracle::new(*sharing.public(), seed_namespace(b"fluent-test"))
            .witness(round, real_seed(round).signature)
    }

    fn finalize_msg(
        order: OrderBlock,
    ) -> (Message, commonware_utils::acknowledgement::ExactWaiter) {
        let (ack, waiter) = Exact::handle();
        (
            Message {
                cause: Span::current(),
                command: Command::Finalize(Box::new(Update::Block(order, ack))),
            },
            waiter,
        )
    }

    /// Drive the deterministic clock until `cond` holds (bounded — panics on
    /// timeout so a regression fails instead of hanging).
    async fn wait_until(ctx: &deterministic::Context, what: &str, mut cond: impl FnMut() -> bool) {
        for _ in 0..2_000 {
            if cond() {
                return;
            }
            ctx.sleep(Duration::from_millis(1)).await;
        }
        panic!("timed out waiting for: {what}");
    }

    /// Assert the SafetyHalt park posture: the executor's handle stays unresolved,
    /// the halted block's ack is retained un-resolved (so the marshal's
    /// `last_processed_height` cannot advance past the diverged height and it never
    /// sees the fatal Canceled), and an ack delivered after the halt engaged is
    /// retained too.
    async fn assert_parked_retaining_acks(
        ctx: &deterministic::Context,
        mut handle: Handle<()>,
        mut waiter: commonware_utils::acknowledgement::ExactWaiter,
        mailbox: &Mailbox,
        halt: &crate::sync_metrics::SafetyHalt,
        post_halt_order: OrderBlock,
    ) {
        wait_until(ctx, "SafetyHalt engaged", || halt.is_engaged()).await;
        // Wait for the executor to reach the park loop before probing ack state.
        ctx.sleep(Duration::from_millis(20)).await;
        assert!(
            (&mut waiter).now_or_never().is_none(),
            "the halted block's ack must be RETAINED — neither acknowledged (would durably \
             skip the diverged height) nor canceled (kills the marshal)"
        );
        assert!(
            (&mut handle).now_or_never().is_none(),
            "the executor must PARK on a SafetyHalt, not exit (an exit drops the retained acks)"
        );
        let (msg, mut post_waiter) = finalize_msg(post_halt_order);
        mailbox
            .send(msg)
            .expect("executor mailbox stays open while parked");
        ctx.sleep(Duration::from_millis(20)).await;
        assert!(
            (&mut post_waiter).now_or_never().is_none(),
            "an ack dispatched AFTER the halt engaged must be retained too"
        );
    }

    /// A `SpecNotarized` command for `order`; seedless, because speculation keys off
    /// the fetched block's height, not the round.
    fn spec_msg(order: &OrderBlock) -> Message {
        Message {
            cause: Span::current(),
            command: Command::SpecNotarized(Box::new(Notarized {
                digest: order.digest(),
                seed: None,
            })),
        }
    }

    #[test]
    fn update_head_rolls_back_to_finalized_fork() {
        let anchor = B256::repeat_byte(0x10);
        let tail = B256::repeat_byte(0x12);
        let lc = LastCanonicalized {
            forkchoice: ForkchoiceState {
                head_block_hash: tail,
                safe_block_hash: anchor,
                finalized_block_hash: anchor,
            },
            head_height: Height::new(12),
            safe_height: Height::new(10),
            finalized_height: Height::new(10),
        };

        let rolled = lc.update_head(Height::new(10), anchor);
        assert_eq!(rolled.head_height, Height::new(10));
        assert_eq!(rolled.forkchoice.head_block_hash, anchor);

        let other = B256::repeat_byte(0x09);
        let unchanged = lc.update_head(Height::new(9), other);
        assert_eq!(unchanged.head_height, Height::new(12));
        assert_eq!(unchanged.forkchoice.head_block_hash, tail);
    }

    /// A `LastCanonicalized` literal with all three tiers equal at `height`.
    fn lc_at(height: u64, hash: B256) -> LastCanonicalized {
        LastCanonicalized {
            forkchoice: ForkchoiceState {
                head_block_hash: hash,
                safe_block_hash: hash,
                finalized_block_hash: hash,
            },
            head_height: Height::new(height),
            safe_height: Height::new(height),
            finalized_height: Height::new(height),
        }
    }

    // `update_finalized` (result tier) and `update_safe` (ordering tier) advance
    // their own monotone guards; the three hashes stay consistent with the heights
    // and `finalized_height ≤ safe_height ≤ head_height` after each op.
    #[test]
    fn finalized_safe_head_ancestry_holds() {
        let h10 = B256::repeat_byte(0x10);
        let mut lc = lc_at(10, h10);

        let h13 = B256::repeat_byte(0x13);
        lc = lc
            .update_safe(Height::new(13), h13)
            .update_head(Height::new(13), h13);
        assert_eq!(lc.safe_height, Height::new(13));
        assert_eq!(lc.forkchoice.safe_block_hash, h13);
        assert_eq!(lc.head_height, Height::new(13));
        assert_eq!(lc.finalized_height, Height::new(10));
        assert!(lc.finalized_height <= lc.safe_height && lc.safe_height <= lc.head_height);

        let h11 = B256::repeat_byte(0x11);
        let h14 = B256::repeat_byte(0x14);
        lc = lc
            .update_finalized(Height::new(11), h11)
            .update_safe(Height::new(14), h14)
            .update_head(Height::new(14), h14);
        assert_eq!(lc.forkchoice.finalized_block_hash, h11);
        assert_eq!(lc.forkchoice.safe_block_hash, h14);
        assert_eq!(lc.forkchoice.head_block_hash, h14);
        assert!(lc.finalized_height <= lc.safe_height && lc.safe_height <= lc.head_height);
    }

    // An out-of-order or transient lower ordering-final delivery must not roll
    // `safe` backward; its monotone guard is distinct from `finalized_height`.
    #[test]
    fn safe_monotonic_guard() {
        let h10 = B256::repeat_byte(0x10);
        let h13 = B256::repeat_byte(0x13);
        let lc = lc_at(10, h10).update_safe(Height::new(13), h13);

        let stale = B256::repeat_byte(0x99);
        let after = lc.update_safe(Height::new(12), stale);
        assert_eq!(after.safe_height, Height::new(13), "safe must not regress");
        assert_eq!(after.forkchoice.safe_block_hash, h13);
    }

    // The `>=` (not `>`) guard: a same-height re-finalization (sibling reorg at
    // `H == safe_height`) lets the hash follow onto the freshly-finalized tip
    // rather than pin `safe` to an orphaned sibling.
    #[test]
    fn safe_follows_same_height_refinalize() {
        let h10 = B256::repeat_byte(0x10);
        let hash_a = B256::repeat_byte(0xAA);
        let lc = lc_at(10, h10).update_safe(Height::new(13), hash_a);

        let hash_b = B256::repeat_byte(0xBB);
        let after = lc.update_safe(Height::new(13), hash_b);
        assert_eq!(
            after.safe_height,
            Height::new(13),
            "height unchanged (lateral)"
        );
        assert_eq!(
            after.forkchoice.safe_block_hash, hash_b,
            "safe followed the same-height re-finalization onto hash_b"
        );
    }

    // Pre-K window: finalized stays clamped to the anchor while head follows the
    // derived tip; from anchor+K onward finalized is the derived hash K below.
    #[test]
    fn two_tier_finalized_lags_head_by_k_clamped_to_anchor() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // Every delivered height derives at its own delivery, so the ack of
            // the last one (ANCHOR+K+1) is the whole chain landing.
            let chain = result_consistent_chain(ANCHOR, fx.anchor_hash, K + 1);
            let mut waiters = Vec::new();
            for order in &chain {
                let (msg, waiter) = finalize_msg(order.clone());
                mailbox.send(msg).expect("send");
                waiters.push(waiter);
            }
            waiters
                .swap_remove(K as usize)
                .await
                .expect("ack of ANCHOR+K+1");

            {
                let fcus = fx.beacon.fcu_calls.lock().unwrap();
                // Heights ANCHOR+1..=ANCHOR+K-1: finalized pinned to the anchor while
                // safe (ordering-final) climbs to each just-finalized tip.
                for (i, fcu) in fcus[..(K - 1) as usize].iter().enumerate() {
                    let ordering_tip = ANCHOR + 1 + i as u64;
                    assert_eq!(fcu.finalized_block_hash, fx.anchor_hash);
                    assert_eq!(
                        fcu.safe_block_hash,
                        fx.chain.spec_executed_hash(ordering_tip).unwrap(),
                        "safe rides the ordering-final tip even while finalized is clamped"
                    );
                    assert_eq!(
                        fcu.safe_block_hash, fcu.head_block_hash,
                        "no speculative lead ⇒ safe == head"
                    );
                }
                // Height ANCHOR+K+1 results-finalizes ANCHOR+1: the K-lag has
                // passed the clamp.
                let derived_anchor_plus_1 = fx.chain.spec_executed_hash(ANCHOR + 1).unwrap();
                let ordering_tip = fx.chain.spec_executed_hash(ANCHOR + K + 1).unwrap();
                let last = fcus.last().unwrap();
                assert_eq!(last.finalized_block_hash, derived_anchor_plus_1);
                // No speculative lead, so safe equals the ordering-final tip and
                // head, K ahead of finalized once past the clamp.
                assert_eq!(last.safe_block_hash, ordering_tip);
                assert_eq!(last.head_block_hash, ordering_tip);
                assert_eq!(last.safe_block_hash, last.head_block_hash);
                // Every block was imported exactly once.
                assert_eq!(
                    fx.beacon.new_payload_calls.lock().unwrap().len() as u64,
                    K + 1
                );
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A transient transport error at the finalize FCU (a `Result::Err`, an RPC or
    // channel blip) is retried until it succeeds: the block still acks, the loop
    // survives, `engine_transient_retry_total` counts the retries, and the
    // `engine_retry` gauge clears on success.
    #[test]
    fn transient_finalize_fcu_transport_error_retries_then_acks() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            *fx.beacon.fcu_transport_errs.lock().unwrap() = 3;
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // A single pre-K finalize: result ZERO, so no cross-check fires.
            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let (msg, waiter) = finalize_msg(order.clone());
            mailbox.send(msg).expect("send");
            waiter
                .await
                .expect("block acks after the finalize FCU retries past the transport blips");

            assert_eq!(
                fx.sync_metrics.engine_transient_retry.get(),
                3,
                "each transport blip incremented the retry counter"
            );
            assert_eq!(
                fx.sync_metrics.degraded_value(SyncReason::EngineRetry),
                0,
                "the engine_retry gauge clears once the FCU transport succeeds"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // An import transport error is `TransientExternal(EngineRetry)` like its FCU
    // sibling — degraded and counted, engine stays up — and the finalized path's
    // postcondition reconvergence lands the block, so it still acks.
    #[test]
    fn import_transport_error_is_degraded_not_fatal_and_counted() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            *fx.beacon.import_transport_errs.lock().unwrap() = 1;
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // A single pre-K finalize: result ZERO, so no cross-check fires.
            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let (msg, waiter) = finalize_msg(order.clone());
            mailbox.send(msg).expect("send");
            waiter
                .await
                .expect("block still acks — an import transport error is NOT actor-death");

            assert_eq!(
                fx.sync_metrics.engine_transient_retry.get(),
                1,
                "the import transport blip incremented the engine-retry counter"
            );
            assert_eq!(
                fx.sync_metrics.degraded_value(SyncReason::EngineRetry),
                0,
                "the engine_retry gauge clears once the finalize FCU transport succeeds"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // Gated landing: while imports keep failing transport the block does not land,
    // the postcondition loop stays converging, and the ack is neither taken (would
    // durably skip an un-landed height) nor Canceled (kills the marshal). Once
    // transport heals, the next re-apply import lands the block and releases the
    // ack.
    #[test]
    fn unlanded_import_transport_error_holds_the_ack_until_reapply_lands() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const H: u64 = ANCHOR + 1;
            let fx = Fixture::new(ANCHOR);
            fx.gate_landing_on_import();
            *fx.beacon.import_transport_errs.lock().unwrap() = u32::MAX;
            let order = sample_order(Digest(B256::ZERO), H, B256::ZERO);
            // The postcondition re-apply loop re-fetches the order by height.
            fx.marshal.canned.lock().unwrap().insert(H, order.clone());
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            let (msg, waiter) = finalize_msg(order.clone());
            mailbox.send(msg).expect("send");

            let mut waiter = waiter;
            wait_until(&ctx, "finalize_apply degraded", || {
                fx.sync_metrics.degraded_value(SyncReason::FinalizeApply) == 1
            })
            .await;
            assert_eq!(
                fx.chain.spec_executed_hash(H),
                None,
                "the block genuinely did NOT land while imports fail transport"
            );
            assert!(
                (&mut waiter).now_or_never().is_none(),
                "the ack is neither taken nor Canceled while the postcondition \
                 loop is still converging"
            );
            assert!(
                fx.sync_metrics.engine_transient_retry.get() >= 1,
                "the transport failures are counted while converging"
            );

            *fx.beacon.import_transport_errs.lock().unwrap() = 0;
            waiter
                .await
                .expect("acks once the re-apply import actually lands the block");
            assert!(
                fx.chain.spec_executed_hash(H).is_some(),
                "the successful import landed the block"
            );
            assert_eq!(
                fx.sync_metrics.degraded_value(SyncReason::FinalizeApply),
                0,
                "the finalize_apply gauge clears once the EL serves the block"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A heartbeat FCU transport failure is `TransientExternal(EngineRetry)`:
    // fire-and-forget (the next tick is the retry, no loop) but counted and
    // degrade-visible, and a clean tick recovers the gauge.
    #[test]
    fn heartbeat_fcu_transport_failure_is_counted_and_degraded() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            *fx.beacon.fcu_transport_errs.lock().unwrap() = 1;
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            // The heartbeat is suppressed until the first consensus advance.
            actor.has_advanced_since_init = true;

            actor
                .send_forkchoice_update_heartbeat()
                .await
                .expect("heartbeat FCU");
            assert_eq!(
                fx.sync_metrics.engine_transient_retry.get(),
                1,
                "a heartbeat transport failure is now counted (was an invisible warn!)"
            );
            assert_eq!(
                fx.sync_metrics.degraded_value(SyncReason::EngineRetry),
                1,
                "the failed heartbeat raised the engine_retry gauge"
            );

            // A clean heartbeat clears the gauge; the model is fire-and-forget, so
            // the next tick is the retry rather than an in-place loop.
            actor
                .send_forkchoice_update_heartbeat()
                .await
                .expect("heartbeat FCU");
            assert_eq!(
                fx.sync_metrics.degraded_value(SyncReason::EngineRetry),
                0,
                "a clean heartbeat recovers the reason"
            );
        });
    }

    // A semantic `Ok(Invalid)` verdict at the finalize FCU is not a transport error:
    // it is a fork-safety SafetyHalt, so the block does not ack, the executor parks
    // retaining the ack, and the `el_invalid` latch engages.
    #[test]
    fn invalid_finalize_fcu_engages_safety_halt_and_parks() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            *fx.beacon.fcu_status.lock().unwrap() = Some(PayloadStatusEnum::Invalid {
                validation_error: "simulated derivation-divergence verdict".into(),
            });
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            let refused = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let flush = child_of(&refused);
            let post_halt = sample_order(flush.digest(), ANCHOR + 3, B256::ZERO);
            let (msg, waiter) = finalize_msg(refused);
            mailbox.send(msg).expect("send");
            // The flush child triggers the derive and the Invalid FCU; its own ack is
            // held in `awaiting_seed` when the halt engages, and `park_halted`
            // retains it too.
            let (flush_msg, mut flush_waiter) = finalize_msg(flush);
            mailbox.send(flush_msg).expect("send flush child");

            assert_parked_retaining_acks(
                &ctx,
                handle,
                waiter,
                &mailbox,
                &fx.safety_halt,
                post_halt,
            )
            .await;
            assert!(
                (&mut flush_waiter).now_or_never().is_none(),
                "the HELD child's ack (awaiting_seed) must be retained by park_halted too"
            );
            assert_eq!(
                fx.sync_metrics.degraded_value(SyncReason::ElInvalid),
                1,
                "the el_invalid gauge is raised for the alert"
            );
            assert_eq!(
                fx.sync_metrics.engine_transient_retry.get(),
                0,
                "Ok(Invalid) is never mistaken for a retryable transport error"
            );
        });
    }

    // The speculative path is best-effort: `spec_execute` classifies a derive failure
    // `Defer(SpecDeriveFailed)`, so the router logs it and the loop continues. The
    // blanket `From<eyre::Report>` would instead make it `Corruption`, and a
    // transient derive failure would kill the executor.
    #[test]
    fn a_transient_speculative_derive_failure_never_takes_the_node_down() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(ANCHOR + 1, order.clone());
            // Fails at the speculative attempt, succeeds at finalization.
            fx.deriver
                .derive_fail_once
                .lock()
                .unwrap()
                .insert(ANCHOR + 1);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let mut handle = actor.start();

            mailbox.send(spec_msg(&order)).expect("send spec");
            ctx.sleep(Duration::from_millis(20)).await;
            assert!(
                fx.deriver.derive_fail_once.lock().unwrap().is_empty(),
                "the speculative derive must actually have been attempted and failed"
            );
            assert!(
                !fx.safety_halt.is_engaged(),
                "a derive failure is not a fork-safety verdict"
            );
            assert!(
                (&mut handle).now_or_never().is_none(),
                "speculation is best-effort: the executor must still be running"
            );

            let (msg, waiter) = finalize_msg(order.clone());
            mailbox.send(msg).expect("send finalize");
            waiter
                .await
                .expect("the finalized path derives the height regardless");

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // One `Ok(Invalid)` FCU verdict has two dispositions, decided by whether the
    // head is committed:
    //
    //  * speculative head (notarized, not finalized) → skip speculation, no latch:
    //    consensus may still nullify the view and finalize a sibling, so the
    //    verdict does not yet indict anything the chain committed.
    //  * finalized head → SafetyHalt: the block is committed.
    //
    // Nothing is swallowed, only deferred to the path that has committed evidence.
    #[test]
    fn an_invalid_fcu_skips_speculation_but_halts_on_the_finalized_path() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            *fx.beacon.fcu_status.lock().unwrap() = Some(PayloadStatusEnum::Invalid {
                validation_error: "head descends from a header reth rejected over devp2p".into(),
            });
            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(ANCHOR + 1, order.clone());
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let mut handle = actor.start();

            mailbox.send(spec_msg(&order)).expect("send spec");
            ctx.sleep(Duration::from_millis(20)).await;
            assert_eq!(
                fx.beacon
                    .new_payload_calls
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|p| p.number)
                    .collect::<Vec<_>>(),
                vec![ANCHOR + 1],
                "the speculative block imported, so the FCU verdict below was reached"
            );
            assert!(
                !fx.safety_halt.is_engaged(),
                "an Ok(Invalid) on a NON-canonical speculative head must not latch a \
                 permanent, operator-cleared halt"
            );
            assert_eq!(
                fx.sync_metrics.degraded_value(SyncReason::ElInvalid),
                0,
                "no el_invalid alert is raised for a branch consensus may still discard"
            );
            assert!(
                (&mut handle).now_or_never().is_none(),
                "the executor keeps running; the finalized path is the judge"
            );

            let (msg, waiter) = finalize_msg(order.clone());
            mailbox.send(msg).expect("send finalize");
            let post_halt = sample_order(Digest(B256::ZERO), ANCHOR + 4, B256::ZERO);
            assert_parked_retaining_acks(
                &ctx,
                handle,
                waiter,
                &mailbox,
                &fx.safety_halt,
                post_halt,
            )
            .await;
            assert_eq!(
                fx.sync_metrics.degraded_value(SyncReason::ElInvalid),
                1,
                "the finalized path raises the el_invalid alert"
            );
        });
    }

    // A fork-safety verdict raised on the speculative path must reach `park_halted`
    // like any other verdict instead of being logged and swallowed: engaging the
    // latch is the router's job, so a caller cannot reduce the verdict to a warning.
    #[test]
    fn a_fork_safety_verdict_on_the_speculative_path_parks_instead_of_being_logged() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            // An `Invalid` import says our own derivation disagrees with reth's
            // re-execution — deterministic and branch-independent, so it halts from
            // either path, unlike the FCU verdict in the test above.
            *fx.beacon.import_status.lock().unwrap() = Some(PayloadStatusEnum::Invalid {
                validation_error: "local derivation diverged from re-execution".into(),
            });
            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(ANCHOR + 1, order.clone());
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let mut handle = actor.start();

            mailbox.send(spec_msg(&order)).expect("send spec");
            wait_until(&ctx, "SafetyHalt engaged from the speculative path", || {
                fx.safety_halt.is_engaged()
            })
            .await;
            ctx.sleep(Duration::from_millis(20)).await;
            assert!(
                (&mut handle).now_or_never().is_none(),
                "a halted executor PARKS (stays observable) — it must not exit and drop acks"
            );
            assert_eq!(
                fx.safety_halt.reason(),
                Some(SyncReason::ElInvalid),
                "the latch carries the verdict that armed it, not just a bit"
            );
            // A block delivered after the halt must have its ack retained by the
            // park, never acked.
            let (msg, mut waiter) = finalize_msg(order);
            mailbox.send(msg).expect("mailbox stays open while parked");
            ctx.sleep(Duration::from_millis(20)).await;
            assert!(
                (&mut waiter).now_or_never().is_none(),
                "a parked executor derives nothing and acks nothing"
            );
        });
    }

    // reth answers "unknown finalized/safe hash" with
    // `Err(ForkchoiceUpdateError::InvalidState)`: it processed the update and
    // rejected the state we named. That is `Corruption` — loud actor death,
    // deliberately not a SafetyHalt — because it says this node's anchor disagrees
    // with this node's own EL, not that the network disagrees with the chain.
    #[test]
    fn an_unresolvable_forkchoice_anchor_dies_loudly_instead_of_retrying_forever() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            *fx.beacon.fcu_anchor_inconsistent.lock().unwrap() = true;
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let (msg, _waiter) = finalize_msg(order.clone());
            mailbox.send(msg).expect("send finalize");

            let exited = futures::future::select(
                Box::pin(handle),
                Box::pin(ctx.sleep(Duration::from_secs(30))),
            )
            .await;
            assert!(
                matches!(exited, futures::future::Either::Left(_)),
                "a rejected forkchoice STATE is structurally permanent — retrying it forever \
                 is the third infinite loop of this family"
            );
            assert!(
                !fx.safety_halt.is_engaged(),
                "Corruption must not latch the fork-safety halt: nothing here says the \
                 NETWORK disagrees with us"
            );
            assert_eq!(
                fx.sync_metrics.engine_transient_retry.get(),
                0,
                "and it must never be counted as a retryable engine transport blip"
            );
            let rejections = *fx.beacon.fcu_anchor_rejections.lock().unwrap();
            assert!(
                (1..=4).contains(&rejections),
                "the executor must consult the engine and STOP, not spin on it (saw \
                 {rejections} forkchoice-state rejections)"
            );
        });
    }

    // An OrderBlock whose attested `result` disagrees with the locally derived hash
    // at `height − K` means this node would serve a fork, so `try_derive` raises the
    // SafetyHalt; the block does not ack and the executor parks retaining its ack.
    #[test]
    fn result_divergence_engages_safety_halt_and_parks() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // Pre-K window: the result must be ZERO, so no cross-check fires. Each
            // height derives when its child arrives, so the divergent block at
            // ANCHOR+K only derives — and halts — when its own child is delivered.
            let mut parent = Digest(B256::ZERO);
            for i in 1..K {
                let order = sample_order(parent, ANCHOR + i, B256::ZERO);
                parent = order.digest();
                let (msg, _waiter) = finalize_msg(order);
                mailbox.send(msg).expect("send");
            }

            // Height ANCHOR+K commits the hash at ANCHOR — but with a forged value.
            let forged = B256::repeat_byte(0xEE);
            assert_ne!(forged, fx.chain.spec_executed_hash(ANCHOR).unwrap());
            let divergent = sample_order(parent, ANCHOR + K, forged);
            let halt_trigger = child_of(&divergent);
            let post_halt = sample_order(halt_trigger.digest(), ANCHOR + K + 2, B256::ZERO);
            let (msg, waiter) = finalize_msg(divergent);
            mailbox.send(msg).expect("send divergent");
            let (trigger_msg, _w_trigger) = finalize_msg(halt_trigger);
            mailbox
                .send(trigger_msg)
                .expect("send the divergent block's child");

            assert_parked_retaining_acks(
                &ctx,
                handle,
                waiter,
                &mailbox,
                &fx.safety_halt,
                post_halt,
            )
            .await;
            assert_eq!(
                fx.sync_metrics.degraded_value(SyncReason::ResultDivergence),
                1,
                "the result_divergence gauge is raised for the alert"
            );
        });
    }

    // A speculation derived with round A while the agreed round for the height is B:
    // the mismatch makes `correctly_speculated` false, so the executor re-derives
    // SPEC_H with seed_B and reorgs the head onto it. The attested result K blocks
    // later then matches the locally executed hash and no `ResultDivergence` halt
    // fires.
    #[test]
    fn spec_seed_mismatch_rederives_with_the_agreed_seed_no_halt() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const SPEC_H: u64 = ANCHOR + 1; // the notarized-then-finalized height
            let fx = Fixture::new(ANCHOR).with_epocher(beacon_active_epocher());
            let anchor_hash = fx.anchor_hash;

            // Both seeds are for the same ordering block, but round A differs from
            // the agreed round B, so the prev_randao→mix_hash fold seals distinct
            // hashes. A carries the block's own view, so speculation keeps it
            // verbatim rather than re-canonicalising.
            let seed_a = real_seed(Round::new(Epoch::new(0), View::new(SPEC_H)));
            let seed_b = real_seed(active_round(SPEC_H));
            record_fixture_seed(SPEC_H);
            record_fixture_seed(0); // the view every `sample_order` block names

            // The block finalized at SPEC_H is pre-K (result ZERO); its `proposal_view`
            // equals seed_a's view, so speculation keeps seed_a as-is.
            let order_h = OrderBlock {
                proposal_view: SPEC_H,
                ..sample_order(Digest(B256::ZERO), SPEC_H, B256::ZERO)
            };
            let hash_a =
                sealed_at(anchor_hash, SPEC_H, seed_folded_discriminator(order_h.digest(), &Some(seed_a.clone())))
                    .hash();
            let hash_b =
                sealed_at(anchor_hash, SPEC_H, seed_folded_discriminator(order_h.digest(), &Some(seed_b.clone())))
                    .hash();
            assert_ne!(
                hash_a, hash_b,
                "seed_A and seed_B must derive DISTINCT executed hashes (else the mismatch can't surface)"
            );

            // Ordering chain SPEC_H..=SPEC_H+K: SPEC_H+1 is pre-K (ZERO result), +2
            // commits the anchor hash, +K attests hash_B.
            let order_h1 = sample_order(order_h.digest(), SPEC_H + 1, B256::ZERO);
            let order_h2 = sample_order(order_h1.digest(), SPEC_H + 2, anchor_hash);
            let order_hk = sample_order(order_h2.digest(), SPEC_H + K, hash_b);

            // Only the speculated block is fetched by digest, so only it is canned.
            fx.marshal.canned.lock().unwrap().insert(SPEC_H, order_h.clone());

            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            mailbox
                .send(Message {
                    cause: Span::current(),
                    command: Command::SpecNotarized(Box::new(Notarized {
                        digest: order_h.digest(),
                        seed: Some(seed_a.clone()),
                    })),
                })
                .expect("send spec@A");

            // Only speculation landed hash_A: the finalized tier still reads `None`,
            // so hash_A can never be committed as the attested result at SPEC_H+K.
            wait_until(&ctx, "SPEC_H speculated to hash_A", || {
                fx.chain.spec_executed_hash(SPEC_H) == Some(hash_a)
            })
            .await;
            assert_eq!(
                fx.chain.finalized_executed_hash(SPEC_H),
                None,
                "finalized tier empty while only speculated — the gate reads None, not hash_A"
            );

            // Finalizing SPEC_H sees the stored round A differ from the agreed round
            // B, so it re-derives and hash_B becomes canonical there.
            let (m, w) = finalize_msg(order_h.clone());
            mailbox.send(m).expect("send finalize SPEC_H");
            let (m1, w1) = finalize_msg(order_h1);
            mailbox.send(m1).expect("send finalize SPEC_H+1");
            w.await.expect("SPEC_H acks after re-derive with seed_B");

            assert_eq!(
                fx.chain.spec_executed_hash(SPEC_H),
                Some(hash_b),
                "round mismatch re-derived SPEC_H with the agreed seed (hash_B)"
            );
            // The finalized tier records the finalized sibling hash_B, not the
            // speculative hash_A, so the result gate at SPEC_H+K cross-checks against
            // hash_B and no whole-committee halt fires.
            assert_eq!(
                fx.chain.finalized_executed_hash(SPEC_H),
                Some(hash_b),
                "finalized tier records the finalized sibling (hash_B), never the speculative hash_A"
            );
            let spec_h_seeds: Vec<_> = fx
                .deriver
                .seeds_seen
                .lock()
                .unwrap()
                .iter()
                .filter(|(h, _)| *h == SPEC_H)
                .map(|(_, s)| s.clone())
                .collect();
            assert_eq!(
                spec_h_seeds,
                vec![Some(seed_a.clone()), Some(seed_b.clone())],
                "SPEC_H derived at spec (seed_A) then RE-DERIVED at finalize (seed_B)"
            );

            // Advance ordering to SPEC_H+K so the hash_B attestation reaches the
            // result cross-check.
            for order in [order_h2, order_hk] {
                let (m, _w) = finalize_msg(order);
                mailbox.send(m).expect("send chain");
            }
            w1.await.expect("intermediate ack");

            // SPEC_H+K attests hash_B, which the executor holds locally, so the
            // cross-check passes and the chain advances past it.
            wait_until(&ctx, "SPEC_H+K derived", || {
                fx.chain.spec_executed_hash(SPEC_H + K).is_some()
            })
            .await;

            assert!(
                !fx.safety_halt.is_engaged(),
                "FIXED: re-derive with the agreed seed keeps local == attested → NO halt"
            );
            assert_eq!(
                fx.sync_metrics.degraded_value(SyncReason::ResultDivergence),
                0,
                "no result_divergence: the chain advanced past SPEC_H+K cleanly"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // The fast path: a speculation whose seed round equals the finalization cert's
    // round is kept (no re-derive, no head rollback) and no halt fires, so
    // `correctly_speculated` cannot regress to always re-deriving.
    #[test]
    fn spec_same_round_keeps_speculation_no_rederive() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const SPEC_H: u64 = ANCHOR + 1;
            let fx = Fixture::new(ANCHOR).with_epocher(beacon_active_epocher());
            let anchor_hash = fx.anchor_hash;
            // The notarization and the store carry the same round — the honest
            // steady state.
            let seed = real_seed(active_round(SPEC_H));
            record_fixture_seed(SPEC_H);

            let order_h = OrderBlock {
                proposal_view: SPEC_H,
                ..sample_order(Digest(B256::ZERO), SPEC_H, B256::ZERO)
            };
            let hash = sealed_at(
                anchor_hash,
                SPEC_H,
                seed_folded_discriminator(order_h.digest(), &Some(seed.clone())),
            )
            .hash();
            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(SPEC_H, order_h.clone());

            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            mailbox
                .send(Message {
                    cause: Span::current(),
                    command: Command::SpecNotarized(Box::new(Notarized {
                        digest: order_h.digest(),
                        seed: Some(seed.clone()),
                    })),
                })
                .expect("send spec");
            let (m, w) = finalize_msg(order_h.clone());
            mailbox.send(m).expect("send finalize SPEC_H");
            w.await.expect("SPEC_H acks via the kept speculation");

            assert_eq!(
                fx.chain.spec_executed_hash(SPEC_H),
                Some(hash),
                "same-round finalize KEPT the speculation's hash (no re-derive)"
            );
            // The correctly-speculated arm also records the finalized tier with the
            // hash it confirmed, so the steady-state gate resolves immediately.
            assert_eq!(
                fx.chain.finalized_executed_hash(SPEC_H),
                Some(hash),
                "finalized tier records the confirmed speculation on the fast path too"
            );
            assert_eq!(
                fx.deriver
                    .seeds_seen
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(h, _)| *h == SPEC_H)
                    .count(),
                1,
                "SPEC_H derived exactly ONCE (spec); the same-round finalize short-circuited"
            );
            assert!(
                !fx.safety_halt.is_engaged(),
                "no divergence on the same-round fast path"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // The re-apply loop on a still-invisible parent is bounded: above the
    // finalized tier reth keeps a foreign hash at the height and drops every
    // re-derived sibling, so the loop keeps re-walking, and an exhausted budget
    // must end in loud death rather than a silent spin.
    #[test]
    fn reapply_parent_visibility_retry_is_bounded_and_dies_loud() {
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            let runtime = deterministic::Runner::default();
            runtime.start(|ctx| async move {
                const ANCHOR: u64 = 100;
                const H: u64 = ANCHOR + 1;
                let fx = Fixture::new(ANCHOR);
                fx.chain
                    .canonical
                    .lock()
                    .unwrap()
                    .insert(H, B256::repeat_byte(0xEE));
                *fx.chain.sibling_drops.lock().unwrap() = u32::MAX;

                let order = sample_order(Digest(B256::ZERO), H, B256::ZERO);
                fx.marshal.canned.lock().unwrap().insert(H, order.clone());
                let child = child_of(&order);

                let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
                let mut handle = actor.start();
                let (msg, waiter) = finalize_msg(order);
                mailbox.send(msg).expect("send finalize H");
                let (child_msg, _child_waiter) = finalize_msg(child);
                mailbox.send(child_msg).expect("send the flush child");

                let derives_at_h = || {
                    fx.deriver
                        .seeds_seen
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|(h, _)| *h == H)
                        .count()
                };
                wait_until(&ctx, "the re-apply loop is spinning", || {
                    derives_at_h() >= 2
                })
                .await;
                let before_hiding = derives_at_h();
                fx.chain.vis.hide(fx.anchor_hash);

                // The retry bound (~10 virtual s) outlasts `wait_until`'s 2 s
                // horizon, hence this coarser wheel; it stays bounded so an
                // unbounded retry fails by timeout.
                let mut died = false;
                for _ in 0..2_000 {
                    if (&mut handle).now_or_never().is_some() {
                        died = true;
                        break;
                    }
                    ctx.sleep(Duration::from_millis(20)).await;
                }
                assert!(
                    died,
                    "timed out waiting for: the re-apply parent-visibility bound to fail loud"
                );
                assert!(
                    derives_at_h() - before_hiding >= REAPPLY_PARENT_VISIBILITY_RETRIES as usize,
                    "the loop died before spending its retry budget"
                );
                assert!(
                    !fx.safety_halt.is_engaged(),
                    "an unreachable parent is local corruption, not a fork-safety verdict"
                );
                // Corruption is loud actor death, so the in-flight `Exact` is
                // canceled — unlike the SafetyHalt park, which retains it.
                assert!(
                    waiter.await.is_err(),
                    "the corruption exit cancels the in-flight ack"
                );
            });
        });
        assert_eq!(
            counter_at(
                &drain_counters(&snap),
                "dpos_executor_fault_total",
                ("class", "corruption")
            ),
            1,
        );
    }

    // Below the finalized tier the re-apply loop has no lever: `update_head`
    // refuses to move the head to a height at or below `finalized_height`, so
    // reth keeps serving the other hash and the conflict must latch the
    // fork-safety halt instead of spinning.
    #[test]
    fn reapply_below_the_finalized_tier_halts_instead_of_spinning() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            // The cold-start trust anchor is the EL's finalized height, so a block
            // delivered below it is a finalized-tier conflict.
            const L: u64 = 100;
            let fx = Fixture::new(L);
            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(L - 2, B256::repeat_byte(0xB2));
            // What reth already holds at L-1 and will not give up: every
            // re-derived sibling is dropped.
            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(L - 1, B256::repeat_byte(0xEE));
            *fx.chain.sibling_drops.lock().unwrap() = u32::MAX;

            let order = sample_order(Digest(B256::ZERO), L - 1, B256::ZERO);
            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(L - 1, order.clone());
            let child = child_of(&order);
            let post_halt = sample_order(child.digest(), L + 1, B256::ZERO);

            let (actor, mailbox) = fx.build(ctx.clone(), L, L);
            let handle = actor.start();
            let (msg, waiter) = finalize_msg(order);
            mailbox.send(msg).expect("send finalize L-1");
            let (child_msg, _child_waiter) = finalize_msg(child);
            mailbox.send(child_msg).expect("send the flush child");

            // `wait_until` is the bound: ~10 re-apply iterations, then a named
            // panic — a spin fails the test instead of hanging the suite.
            wait_until(&ctx, "SafetyHalt engaged", || fx.safety_halt.is_engaged()).await;
            assert_eq!(
                fx.sync_metrics.degraded_value(SyncReason::ResultDivergence),
                1,
                "a finalized-tier conflict is a divergence verdict, not a retry"
            );
            assert_parked_retaining_acks(
                &ctx,
                handle,
                waiter,
                &mailbox,
                &fx.safety_halt,
                post_halt,
            )
            .await;
        });
    }

    // The other arm of the same gate: the EL serves nothing at a height it holds
    // as finalized. That by-number blind spot is a transient — not a settled
    // conflict — so the gate re-reads and the node heals.
    #[test]
    fn finalized_tier_absent_block_heals_within_the_visibility_belt() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            // The cold-start trust anchor is the EL's finalized height, so a block
            // delivered below it lands in the finalized-tier gate.
            const L: u64 = 100;
            let fx = Fixture::new(L);
            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(L - 2, B256::repeat_byte(0xB2));
            // The EL lands L-1 on derive but stays by-number blind for the next
            // three reads — the gate's own plus two belt re-reads.
            fx.chain.missing_reads.lock().unwrap().insert(L - 1, 3);

            let order = sample_order(Digest(B256::ZERO), L - 1, B256::ZERO);
            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(L - 1, order.clone());
            let child = child_of(&order);

            let (actor, mailbox) = fx.build(ctx.clone(), L, L);
            let mut handle = actor.start();
            let (msg, waiter) = finalize_msg(order);
            mailbox.send(msg).expect("send finalize L-1");
            let (child_msg, _child_waiter) = finalize_msg(child);
            mailbox.send(child_msg).expect("send the flush child");

            waiter
                .await
                .expect("the belt outlives the blind spot, so L-1 acks normally");
            assert_eq!(
                fx.chain
                    .missing_reads
                    .lock()
                    .unwrap()
                    .get(&(L - 1))
                    .copied(),
                Some(0),
                "every blinded read was actually spent"
            );
            assert!(
                !fx.safety_halt.is_engaged(),
                "a transient by-number blind spot is not a fork-safety verdict"
            );
            assert!(
                (&mut handle).now_or_never().is_none(),
                "the executor must survive a height the EL served late"
            );
        });
    }

    // The belt is bounded: an EL that never serves a height it claims as
    // finalized is local corruption, so the actor dies loudly rather than
    // stalling the ack. The latch stays clear and the in-flight `Exact` is
    // canceled, like the other corruption exits.
    #[test]
    fn finalized_tier_absent_block_dies_loud_once_the_belt_is_spent() {
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            let runtime = deterministic::Runner::default();
            runtime.start(|ctx| async move {
                const L: u64 = 100;
                let fx = Fixture::new(L);
                fx.chain
                    .canonical
                    .lock()
                    .unwrap()
                    .insert(L - 2, B256::repeat_byte(0xB2));
                fx.chain
                    .missing_reads
                    .lock()
                    .unwrap()
                    .insert(L - 1, u32::MAX);

                let order = sample_order(Digest(B256::ZERO), L - 1, B256::ZERO);
                fx.marshal
                    .canned
                    .lock()
                    .unwrap()
                    .insert(L - 1, order.clone());
                let child = child_of(&order);

                let (actor, mailbox) = fx.build(ctx.clone(), L, L);
                let mut handle = actor.start();
                let (msg, waiter) = finalize_msg(order);
                mailbox.send(msg).expect("send finalize L-1");
                let (child_msg, _child_waiter) = finalize_msg(child);
                mailbox.send(child_msg).expect("send the flush child");

                // The retry bound (~10 virtual s) outlasts `wait_until`'s 2 s
                // horizon, hence this coarser wheel; it stays bounded so an
                // unbounded belt fails by timeout.
                let mut died = false;
                for _ in 0..2_000 {
                    if (&mut handle).now_or_never().is_some() {
                        died = true;
                        break;
                    }
                    ctx.sleep(Duration::from_millis(20)).await;
                }
                assert!(
                    died,
                    "timed out waiting for: the finalized-tier visibility belt to fail loud"
                );
                // The gate's own read plus the full belt: without the belt exactly
                // one read is spent and the actor dies on the spot.
                let reads_spent = u32::MAX
                    - fx.chain
                        .missing_reads
                        .lock()
                        .unwrap()
                        .get(&(L - 1))
                        .copied()
                        .expect("the blind-spot entry survives");
                assert!(
                    reads_spent > FINALIZED_TIER_VISIBILITY_RETRIES,
                    "the gate died before spending its re-read budget ({reads_spent} reads)"
                );
                assert!(
                    !fx.safety_halt.is_engaged(),
                    "an EL that never serves the height is local corruption, not a \
                     fork-safety verdict"
                );
                assert!(
                    waiter.await.is_err(),
                    "the corruption exit cancels the in-flight ack"
                );
            });
        });
        assert_eq!(
            counter_at(
                &drain_counters(&snap),
                "dpos_executor_fault_total",
                ("class", "corruption")
            ),
            1,
        );
    }

    // With the EL dropping the first two same-height sibling imports, the
    // `try_derive` canonical postcondition keeps re-applying (derive + import +
    // FCU, `finalize_apply` degraded while stuck) instead of acking past the
    // un-applied reorg, then acks once the EL serves the finalized hash. K blocks
    // later the attested result matches and no SafetyHalt fires.
    #[test]
    fn finalized_sibling_reorg_survives_dropped_el_import() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const SPEC_H: u64 = ANCHOR + 1;
            let fx = Fixture::new(ANCHOR).with_epocher(beacon_active_epocher());
            let anchor_hash = fx.anchor_hash;

            // Speculation round A differs from the agreed round B the finalized
            // derive resolves from the store.
            let seed_a = real_seed(Round::new(Epoch::new(0), View::new(SPEC_H)));
            let seed_b = real_seed(active_round(SPEC_H));
            record_fixture_seed(SPEC_H);
            record_fixture_seed(0); // the view every `sample_order` block names

            // `proposal_view == seed_a`'s view keeps the re-canonicalisation a
            // no-op for the speculation.
            let order_h = OrderBlock {
                proposal_view: SPEC_H,
                ..sample_order(Digest(B256::ZERO), SPEC_H, B256::ZERO)
            };
            let hash_a = sealed_at(
                anchor_hash,
                SPEC_H,
                seed_folded_discriminator(order_h.digest(), &Some(seed_a.clone())),
            )
            .hash();
            let hash_b = sealed_at(
                anchor_hash,
                SPEC_H,
                seed_folded_discriminator(order_h.digest(), &Some(seed_b.clone())),
            )
            .hash();
            assert_ne!(hash_a, hash_b, "distinct sibling hashes required");

            // SPEC_H+1 is pre-activation (ZERO result); +2 commits Height(ANCHOR);
            // +K attests hash_B.
            let order_h1 = sample_order(order_h.digest(), SPEC_H + 1, B256::ZERO);
            let order_h2 = sample_order(order_h1.digest(), SPEC_H + 2, anchor_hash);
            let order_hk = sample_order(order_h2.digest(), SPEC_H + K, hash_b);

            // The marshal serves SPEC_H by digest (spec_execute) and by height
            // (the postcondition re-apply loop's re-fetch).
            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(SPEC_H, order_h.clone());

            // The first two same-height sibling imports are silently dropped: two
            // drops prove the loop retries rather than merely re-attempting once.
            *fx.chain.sibling_drops.lock().unwrap() = 2;

            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // Speculate SPEC_H at notarization round A → hash_A canonical.
            mailbox
                .send(Message {
                    cause: Span::current(),
                    command: Command::SpecNotarized(Box::new(Notarized {
                        digest: order_h.digest(),
                        seed: Some(seed_a.clone()),
                    })),
                })
                .expect("send spec@A");

            // Finalize SPEC_H: the round guard routes to the re-derive, whose
            // sibling import the EL drops twice; the ack must not fire until the
            // re-apply loop lands hash_B.
            let (m, w) = finalize_msg(order_h.clone());
            mailbox.send(m).expect("send finalize SPEC_H");
            let (m1, w1) = finalize_msg(order_h1);
            mailbox.send(m1).expect("send finalize SPEC_H+1");
            w.await
                .expect("SPEC_H acks only after the re-apply loop lands hash_B");

            assert_eq!(
                fx.chain.spec_executed_hash(SPEC_H),
                Some(hash_b),
                "the finalized sibling is canonical at SPEC_H despite the dropped \
                 imports (pre-fix: hash_A stayed canonical — the soak3 fork)"
            );
            // spec(A) + finalize first attempt(B, dropped) + re-apply(B, dropped) +
            // re-apply(B, landed) = 4 derives at SPEC_H.
            let spec_h_seeds: Vec<_> = fx
                .deriver
                .seeds_seen
                .lock()
                .unwrap()
                .iter()
                .filter(|(h, _)| *h == SPEC_H)
                .map(|(_, s)| s.clone())
                .collect();
            assert_eq!(
                spec_h_seeds,
                vec![
                    Some(seed_a.clone()),
                    Some(seed_b.clone()),
                    Some(seed_b.clone()),
                    Some(seed_b.clone()),
                ],
                "the re-apply loop re-derived with the FINALIZATION seed until the EL applied it"
            );
            assert_eq!(
                *fx.chain.sibling_drops.lock().unwrap(),
                0,
                "both armed drops were consumed by re-apply attempts"
            );
            assert_eq!(
                fx.sync_metrics.degraded_value(SyncReason::FinalizeApply),
                0,
                "the finalize_apply gauge clears once the EL serves the finalized hash"
            );

            // Advance to SPEC_H+K: the attested hash_B matches the local chain, so
            // it derives cleanly with no SafetyHalt.
            for order in [order_h2, order_hk] {
                let (m, _w) = finalize_msg(order);
                mailbox.send(m).expect("send chain");
            }
            w1.await.expect("intermediate ack");
            wait_until(&ctx, "SPEC_H+K derived (attests hash_B)", || {
                fx.chain.spec_executed_hash(SPEC_H + K).is_some()
            })
            .await;

            assert!(
                !fx.safety_halt.is_engaged(),
                "no SafetyHalt: the re-apply loop prevented the silent fork"
            );
            assert_eq!(
                fx.sync_metrics.degraded_value(SyncReason::ResultDivergence),
                0,
                "no result_divergence raised"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A deep-catch-up follower trust-anchors at the live frontier (anchor ≫
    // activation) and derives the K-below-anchor blocks. Those are post-activation
    // and carry real (non-zero) results, so keying the pre-activation window on
    // the cold-start anchor instead of the chain activation block mis-classifies
    // them as pre-activation (ZERO expected) and shuts the executor down.
    #[test]
    fn below_anchor_post_activation_block_passes_cross_check() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ACTIVATION: u64 = 192;
            const ANCHOR: u64 = 206; // live-frontier cold-start landing
            let fx = Fixture::new(ANCHOR);
            // EL-synced (devp2p) hashes the follower already holds below its
            // anchor; the result-attested hash at ANCHOR−K−2 = 201 is one of them.
            let mut result_at_201 = B256::ZERO;
            for h in (ANCHOR - K - 2)..ANCHOR {
                let hash = B256::repeat_byte((h % 256) as u8);
                fx.chain.canonical.lock().unwrap().insert(h, hash);
                if h == ANCHOR - K - 2 {
                    result_at_201 = hash;
                }
            }
            assert_ne!(result_at_201, B256::ZERO);

            // Marshal floor = ANCHOR − K = 203, so the first dispatched height is
            // 204 — below the anchor (206) but above activation+K (195). Its result
            // commits the already-present derived hash at 204 − K = 201.
            let below_anchor = ANCHOR - K + 1;
            let order = sample_order(Digest(B256::ZERO), below_anchor, result_at_201);

            let (actor, mailbox) = fx.build_with_activation(ctx, ANCHOR, ACTIVATION, ANCHOR);
            let handle = actor.start();

            let (msg, waiter) = finalize_msg(order.clone());
            mailbox
                .send(msg)
                .expect("send below-anchor post-activation block");
            waiter
                .await
                .expect("below-anchor post-activation block must ack (not shut down)");

            drop(mailbox);
            handle.await.expect("executor joins on mailbox close");
        });
    }

    #[test]
    fn backfill_drains_before_live_finalize() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 0;
            let fx = Fixture::new(ANCHOR);
            let chain = result_consistent_chain(ANCHOR, fx.anchor_hash, 4);
            // Heights 1..=3 canned in the marshal (crash-recovery backfill).
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                for order in &chain[..3] {
                    canned.insert(order.height, order.clone());
                }
            }
            let (actor, mailbox) = fx.build(ctx, ANCHOR, 3);
            let handle = actor.start();

            // A live finalize for height 4 lands before the backfill drains; it
            // must still derive after 1..=3.
            let (msg, waiter) = finalize_msg(chain[3].clone());
            mailbox.send(msg).expect("send");
            waiter.await.expect("ack for height 4");

            {
                let payloads = fx.beacon.new_payload_calls.lock().unwrap();
                let heights: Vec<u64> = payloads.iter().map(|p| p.number).collect();
                assert_eq!(heights, vec![1, 2, 3, 4]);
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A delivered artifact whose parent is underived (floor jump, unflushed
    // reth tail) must trigger the marshal gap-walk — derive the prefix in
    // order, then the delivered block — instead of a fatal shutdown.
    #[test]
    fn missing_parent_walks_gap_from_marshal() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let chain = result_consistent_chain(ANCHOR, fx.anchor_hash, 4);
            // Heights 101..=103 exist only in the marshal (not yet derived).
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                for order in &chain[..3] {
                    canned.insert(order.height, order.clone());
                }
            }
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // Height 104 is delivered with an unresolvable parent digest: its real
            // parent 103 is underived, so the gap-walk fills 101..103 first (each
            // element resolves σ at its own round; the walk needs no certs). The
            // result still commits the derived hash at 101.
            let delivered = OrderBlock {
                parent: Digest(B256::ZERO),
                ..chain[3].clone()
            };
            let (msg, waiter) = finalize_msg(delivered.clone());
            mailbox.send(msg).expect("send");
            waiter.await.expect("ack after gap walk");

            {
                let payloads = fx.beacon.new_payload_calls.lock().unwrap();
                let heights: Vec<u64> = payloads.iter().map(|p| p.number).collect();
                assert_eq!(
                    heights,
                    vec![ANCHOR + 1, ANCHOR + 2, ANCHOR + 3, ANCHOR + 4],
                    "gap derived in order before the delivered block"
                );
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // The gap-walk prefix resolves σ the way the main path does — at each
    // element's own round, predicate first — never from the delivered block.
    #[test]
    fn the_gap_walk_prefix_resolves_each_element_at_its_own_round() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let store = crate::beacon::testing::SeedStore::new();
            let seeds: Vec<_> = (101..=104).map(|v| real_seed(active_round(v))).collect();
            for seed in &seeds {
                store.record(real_witness(seed.target_round));
            }
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(store)
                .with_epocher(beacon_active_epocher());
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);

            let at = |view: u64, parent: Digest, result: B256| OrderBlock {
                proposal_view: view,
                ..sample_order(parent, view, result)
            };
            // 101/102 are inside the pre-activation window (result must be ZERO);
            // 103 commits executed_hash(100) = the anchor; 104 commits
            // executed_hash(101), which the walk itself produces.
            let o1 = at(101, Digest(B256::ZERO), B256::ZERO);
            let o2 = at(102, o1.digest(), B256::ZERO);
            let o3 = at(103, o2.digest(), fx.anchor_hash);
            let hash_101 = sealed_at(
                fx.anchor_hash,
                101,
                seed_folded_discriminator(o1.digest(), &Some(seeds[0].clone())),
            )
            .hash();
            let o4 = at(104, o3.digest(), hash_101);
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                for order in [&o1, &o2, &o3] {
                    canned.insert(order.height, (*order).clone());
                }
            }

            let (ack, _waiter) = Exact::handle();
            actor
                .try_derive(Span::current(), o4, ack, Some(seeds[3].clone()))
                .await
                .expect("the walk derives the prefix then the delivered block");

            assert_eq!(
                fx.deriver.seeds_seen.lock().unwrap().as_slice(),
                &[
                    (101, Some(seeds[0].clone())),
                    (102, Some(seeds[1].clone())),
                    (103, Some(seeds[2].clone())),
                    (104, Some(seeds[3].clone())),
                ],
                "each walk element derived from σ of its OWN round, in order"
            );
            assert!(!fx.safety_halt.is_engaged());
        });
    }

    // A gap-walk prefix element on a beacon-active round with no σ parks. The walk
    // owns neither `cause` nor the ack, so it reports the typed leaf and lets
    // `try_derive`, which owns the park, convert it: deriving with the digest
    // fallback would silently fork, so waiting is the only correct answer.
    #[test]
    fn a_gap_walk_prefix_miss_on_a_beacon_active_round_parks() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let store = crate::beacon::testing::SeedStore::new();
            // σ for the delivered height only — the prefix element at 101 has none.
            let delivered_seed = real_seed(active_round(102));
            store.record(real_witness(delivered_seed.target_round));
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(store)
                .with_epocher(beacon_active_epocher());
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);

            let o1 = OrderBlock {
                proposal_view: 101,
                ..sample_order(Digest(B256::ZERO), 101, B256::ZERO)
            };
            fx.marshal.canned.lock().unwrap().insert(101, o1.clone());
            let o2 = OrderBlock {
                proposal_view: 102,
                ..sample_order(o1.digest(), 102, B256::ZERO)
            };

            let (ack, _waiter) = Exact::handle();
            let outcome = actor
                .try_derive(Span::current(), o2, ack, Some(delivered_seed))
                .await
                .expect("a σ-less prefix element parks — it must never fail");

            match outcome {
                DeriveOutcome::NeedPrefixSeed(d) => assert_eq!(
                    d.order.height, 102,
                    "the DELIVERED height is parked; the prefix is re-walked on the re-poke"
                ),
                other => panic!(
                    "parked on the wrong cause: {}",
                    match other {
                        DeriveOutcome::Done => "Done",
                        DeriveOutcome::NeedAttestation(_) => "NeedAttestation",
                        DeriveOutcome::NeedParentVisible(_) => "NeedParentVisible",
                        DeriveOutcome::NeedPrefixSeed(_) => unreachable!(),
                    }
                ),
            }
            assert!(
                fx.deriver
                    .seeds_seen
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|(h, _)| *h != 101),
                "101 must not have been derived with the digest fallback"
            );
            assert!(
                !fx.safety_halt.is_engaged(),
                "a missing σ is a liveness stall, never a fork-safety halt"
            );
        });
    }

    // The park's only exit: σ lands in the same store the walk reads, the parked
    // block is re-poked, and the walk re-runs its own per-element lookup — which is
    // why the park can carry the delivered height's σ without ever carrying `None`
    // for the prefix.
    #[test]
    fn a_parked_prefix_seed_derives_when_sigma_lands() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let store = crate::beacon::testing::SeedStore::new();
            let prefix_seed = real_seed(active_round(101));
            let delivered_seed = real_seed(active_round(102));
            store.record(real_witness(delivered_seed.target_round));
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(store.clone())
                .with_epocher(beacon_active_epocher());
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);

            let o1 = OrderBlock {
                proposal_view: 101,
                ..sample_order(Digest(B256::ZERO), 101, B256::ZERO)
            };
            fx.marshal.canned.lock().unwrap().insert(101, o1.clone());
            let o2 = OrderBlock {
                proposal_view: 102,
                ..sample_order(o1.digest(), 102, B256::ZERO)
            };

            let (ack, waiter) = Exact::handle();
            let outcome = actor
                .try_derive(Span::current(), o2, ack, Some(delivered_seed.clone()))
                .await
                .expect("a σ-less prefix element parks");
            actor.defer_if_needed(outcome, true).await;
            assert!(actor.deferred.is_some(), "the block is parked, not dropped");

            // The σ the park waits for arrives locally; nothing is asked of any peer.
            store.record(real_witness(prefix_seed.target_round));
            actor
                .repoke_deferred()
                .await
                .expect("the re-poke re-walks and completes");

            assert!(actor.deferred.is_none(), "the park is released on success");
            assert_eq!(
                fx.deriver.seeds_seen.lock().unwrap().as_slice(),
                &[(101, Some(prefix_seed)), (102, Some(delivered_seed))],
                "each element derived from σ of its OWN round — the prefix σ was \
                 re-resolved by the walk, never carried through the park"
            );
            waiter
                .await
                .expect("the ack survived the park and was acknowledged");
            assert!(!fx.safety_halt.is_engaged());
        });
    }

    // The beacon-inactive arm is not a miss and must never hold: `None` is the
    // agreed derivation there, so the block derives at its own delivery even with
    // an empty store. Holding would wedge every pre-beacon height forever.
    #[test]
    fn a_beacon_inactive_height_derives_immediately_and_never_holds() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            // Default epocher ⇒ epoch 0 ⇒ beacon-inactive; store deliberately empty.
            let fx = Fixture::new(ANCHOR).with_seed_store(crate::beacon::testing::SeedStore::new());
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);

            let (ack, _w) = Exact::handle();
            actor
                .on_finalized_block(
                    Span::current(),
                    sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO),
                    ack,
                )
                .await
                .expect("inactive epoch derives with `None`");

            assert!(
                actor.awaiting_seed.is_none(),
                "an empty store at a beacon-INACTIVE epoch is not a miss — nothing may hold"
            );
            assert_eq!(
                fx.deriver.seeds_seen.lock().unwrap().as_slice(),
                &[(ANCHOR + 1, None)],
                "derived with the agreed `None`"
            );
        });
    }

    // The gap-walk imports each prefix block and hands its hash to the next derive
    // as a by-hash parent. Without a canonicalization FCU per landed block the
    // walk's second element derives against a parent that exists only in reth's
    // tree-private state and fails with ParentHeaderMissing.
    #[test]
    fn gap_walk_canonicalizes_each_landed_block_for_the_next_by_hash_parent() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 96;
            let fx = Fixture::new(ANCHOR);
            // Everything above the anchor is present by number but invisible by
            // hash until an FCU raises the frontier; without this `visible()`
            // short-circuits to true and the test proves nothing.
            fx.chain.vis.set_frontier(ANCHOR);
            // The two-block prefix exercises parent-to-parent chaining within the
            // walk: 98 derives on a parent the walk imported one iteration earlier.
            // The one-block shape is covered separately.
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                canned.insert(97, sample_order(Digest(B256::ZERO), 97, B256::ZERO));
                canned.insert(98, sample_order(Digest(B256::ZERO), 98, B256::ZERO));
            }
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);

            // `result_target(99, 96) == Height(96)`, so the delivered block commits
            // the anchor hash — a ZERO result would trip the top-level divergence
            // halt before the walk's outcome is observable.
            let (ack, _waiter) = Exact::handle();
            let outcome = actor
                .try_derive(
                    Span::current(),
                    sample_order(Digest(B256::ZERO), 99, fx.anchor_hash),
                    ack,
                    None,
                )
                .await;
            match outcome {
                Ok(DeriveOutcome::Done) => {}
                Ok(_) => panic!("the gap-walk parked instead of completing"),
                Err(error) => panic!(
                    "gap-walk must complete once each landed block is canonicalized: {:#}",
                    error.cause()
                ),
            }

            // Asserted on the recorded set, not on ordering: `FakeBeacon` stores
            // `fcu_calls` with no derive interleaving, and the outcome assertion
            // above already fails without the FCU.
            let heads: Vec<B256> = fx
                .beacon
                .fcu_calls
                .lock()
                .unwrap()
                .iter()
                .map(|s| s.head_block_hash)
                .collect();
            for h in [97u64, 98] {
                let hash = fx.chain.canonical.lock().unwrap()[&h];
                assert!(
                    heads.contains(&hash),
                    "no canonicalization FCU named height {h}'s hash as head"
                );
            }
        });
    }

    /// Drive a one-block gap: 97 missing, 98 delivered, by-hash frontier at the
    /// anchor. The walk's only element is also its last, so the walk returns `Ok`
    /// and the delivered derive is what depends on the canonicalization. Both 97
    /// and 98 are pre-activation (`< anchor + K`), so a ZERO result is the correct
    /// commitment at each.
    ///
    /// The caller decides whether the FCU can land: arm nothing and the walk heals;
    /// arm a transport blip or a SYNCING backfill and it parks. Either way
    /// `try_derive` must not fail.
    async fn derive_across_a_one_block_gap(
        ctx: deterministic::Context,
        fx: &Fixture,
    ) -> (
        Actor<deterministic::Context, FakeBeacon, FakeDeriver, FakeChain, FakeMarshal>,
        DeriveOutcome,
        commonware_utils::acknowledgement::ExactWaiter,
    ) {
        fx.chain.vis.set_frontier(96);
        fx.marshal
            .canned
            .lock()
            .unwrap()
            .insert(97, sample_order(Digest(B256::ZERO), 97, B256::ZERO));
        let (mut actor, _mailbox) = fx.build(ctx, 96, 96);
        let (ack, waiter) = Exact::handle();
        let outcome = actor
            .try_derive(
                Span::current(),
                sample_order(Digest(B256::ZERO), 98, B256::ZERO),
                ack,
                None,
            )
            .await
            .expect("a one-block gap completes or parks — it must never fail");
        (actor, outcome, waiter)
    }

    // A one-block gap whose only walk element is also its last: the block the walk
    // imports is handed straight to the delivered derive as a by-hash parent. The
    // two park tests cover the same shape with the FCU defeated; this one asserts
    // the FCU lands and the derive makes progress.
    #[test]
    fn a_one_block_gap_heals_when_the_canonicalization_fcu_lands() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let fx = Fixture::new(96);
            // Nothing armed: the FCU reaches reth and canonicalizes what it names.
            let (_actor, outcome, _waiter) = derive_across_a_one_block_gap(ctx, &fx).await;

            assert!(
                matches!(outcome, DeriveOutcome::Done),
                "the walk must COMPLETE once its canonicalization FCU lands"
            );
            let landed_97 = fx
                .chain
                .spec_executed_hash(97)
                .expect("the walk's only prefix element landed");
            assert!(
                fx.chain.vis.visible(landed_97),
                "the walk's FCU canonicalized 97 — that is what lets 98 read it by hash"
            );
            let derived_on_97 = sealed_at(
                landed_97,
                98,
                sample_order(Digest(B256::ZERO), 98, B256::ZERO).digest().0,
            )
            .hash();
            assert_eq!(
                fx.chain.spec_executed_hash(98),
                Some(derived_on_97),
                "98 derived ON the block the walk just canonicalized"
            );
        });
    }

    #[test]
    fn one_block_gap_with_an_unlandable_fcu_parks_instead_of_dying() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let fx = Fixture::new(96);
            // The FCU never reaches reth, so 97 lands but is never canonicalized.
            *fx.beacon.fcu_transport_errs.lock().unwrap() = 1;
            let (_actor, outcome, _waiter) = derive_across_a_one_block_gap(ctx, &fx).await;

            assert_parked_at_98(outcome);
            assert!(
                !fx.safety_halt.is_engaged(),
                "an invisible parent is a liveness stall, never a fork-safety halt"
            );
            assert!(
                fx.chain.spec_executed_hash(98).is_none(),
                "the delivered height must NOT be recorded while its parent is invisible"
            );
        });
    }

    // The cause the park primarily exists for: `cold_start_jump` arms a devp2p
    // backfill, reth answers SYNCING while it holds the engine, and a SYNCING
    // forkchoice canonicalizes nothing — so the walk cannot simply await VALID and
    // must survive an FCU that does not land.
    #[test]
    fn a_syncing_backfill_that_canonicalizes_nothing_parks_instead_of_dying() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let fx = Fixture::new(96);
            *fx.beacon.fcu_status.lock().unwrap() = Some(PayloadStatusEnum::Syncing);
            let (_actor, outcome, _waiter) = derive_across_a_one_block_gap(ctx, &fx).await;

            assert_parked_at_98(outcome);
            assert!(
                !fx.beacon.fcu_calls.lock().unwrap().is_empty(),
                "the walk must still SEND the canonicalization FCU under SYNCING"
            );
            assert!(
                !fx.safety_halt.is_engaged(),
                "a backfill holding the engine is a liveness stall, not a fork"
            );
        });
    }

    fn assert_parked_at_98(outcome: DeriveOutcome) {
        match outcome {
            DeriveOutcome::NeedParentVisible(d) => assert_eq!(
                d.order.height, 98,
                "the DELIVERED height is parked — the prefix already landed"
            ),
            DeriveOutcome::NeedAttestation(_) => panic!("parked on the wrong cause"),
            DeriveOutcome::NeedPrefixSeed(_) => panic!("parked on the wrong cause"),
            DeriveOutcome::Done => panic!("the derive cannot be Done: 97 is still invisible"),
        }
    }

    #[test]
    fn a_repoke_completes_the_block_parked_on_parent_visibility() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let fx = Fixture::new(96);
            *fx.beacon.fcu_status.lock().unwrap() = Some(PayloadStatusEnum::Syncing);
            let (mut actor, outcome, _waiter) = derive_across_a_one_block_gap(ctx, &fx).await;
            actor.defer_if_needed(outcome, true).await;
            assert_eq!(
                actor.deferred_height.get(),
                98,
                "the park is observable while it lasts"
            );

            // The backfill goes idle and reth answers VALID again; 97, imported but
            // never canonicalized, is correspondingly absent from the by-number
            // canonical index the re-walk probes, so the re-poke re-walks and the
            // re-issued FCU finally canonicalizes 97.
            *fx.beacon.fcu_status.lock().unwrap() = None;
            fx.chain.canonical.lock().unwrap().remove(&97);
            actor
                .repoke_deferred()
                .await
                .expect("a re-poke past the stall must not be fatal");

            assert!(actor.deferred.is_none(), "the parked block completed");
            assert_eq!(actor.deferred_height.get(), 0, "the park gauge cleared");
            assert!(
                fx.chain.spec_executed_hash(98).is_some(),
                "the re-poke derived the delivered height on the now-visible parent"
            );
        });
    }

    // A gap block (filled by `derive_finalized_with_gap_fill`, not the top-level
    // delivery) carries its own attested `result`, so a forged value on a gap-range
    // block must fail loud like the top-level cross-check; otherwise a
    // committee-attested wrong result is imported unchecked. The halt fires inside
    // `derive_finalized_with_gap_fill`, below the `inflight_ack` slot, and the
    // executor parks retaining the ack.
    #[test]
    fn gap_block_result_divergence_engages_safety_halt_and_parks() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let chain = result_consistent_chain(ANCHOR, fx.anchor_hash, K + 2);
            // Forge the `result` on the gap block at ANCHOR+K (index K-1), the first
            // gap height past the pre-activation window: its cross-check fires
            // against the already-canonical derived hash at ANCHOR.
            let forged_idx = (K - 1) as usize;
            let forged = B256::repeat_byte(0xEE);
            assert_ne!(forged, fx.chain.spec_executed_hash(ANCHOR).unwrap());
            let mut forged_chain = chain.clone();
            forged_chain[forged_idx].result = forged;
            // All gap heights ANCHOR+1 ..= ANCHOR+K+1 exist only in the marshal.
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                for order in &forged_chain[..(K + 1) as usize] {
                    canned.insert(order.height, order.clone());
                }
            }
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // The top height (ANCHOR+K+1) is delivered with an unresolvable parent so
            // the gap-walk fills ANCHOR+1 ..= ANCHOR+K first, hitting the forged gap
            // block at ANCHOR+K. The delivered block itself has a consistent result.
            let delivered = OrderBlock {
                parent: Digest(B256::ZERO),
                ..forged_chain[(K + 1) as usize].clone()
            };
            let halt_trigger = child_of(&delivered);
            let post_halt = sample_order(halt_trigger.digest(), ANCHOR + K + 3, B256::ZERO);
            let (msg, waiter) = finalize_msg(delivered);
            mailbox.send(msg).expect("send");
            let (trigger_msg, _w_trigger) = finalize_msg(halt_trigger);
            mailbox
                .send(trigger_msg)
                .expect("send the delivered block's child");

            // The forged gap block must abort the walk → no ack; the executor
            // halts parked, retaining the delivered block's ack.
            assert_parked_retaining_acks(
                &ctx,
                handle,
                waiter,
                &mailbox,
                &fx.safety_halt,
                post_halt,
            )
            .await;
        });
    }

    // The tip digest is an ordering digest reth cannot resolve, so `Update::Tip`
    // must never become an FCU target.
    #[test]
    fn tip_is_inert_for_forkchoice() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            let tip_digest = Digest(B256::repeat_byte(0xDD));
            mailbox
                .send(Message {
                    cause: Span::current(),
                    command: Command::Finalize(Box::new(Update::Tip(
                        commonware_consensus::types::Round::new(
                            commonware_consensus::types::Epoch::new(0),
                            commonware_consensus::types::View::new(5),
                        ),
                        // Below ANCHOR+1+K so guard #2 stays cold: this test is
                        // about the tip's FCU-inertness, not catch-up.
                        Height::new(ANCHOR + 2),
                        tip_digest,
                    ))),
                })
                .expect("send tip");

            // Drain barrier: one real finalize (+ its flush child).
            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let (msg, waiter) = finalize_msg(order.clone());
            mailbox.send(msg).expect("send");
            waiter.await.expect("ack");

            {
                let fcus = fx.beacon.fcu_calls.lock().unwrap();
                assert_eq!(fcus.len(), 1, "tip produced no FCU; only the finalize did");
                assert_eq!(
                    fcus[0].head_block_hash,
                    fx.chain.spec_executed_hash(ANCHOR + 1).unwrap()
                );
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // Speculative execution imports the block at notarization, advancing the head
    // ahead of finalization; the matching finalization reconciles without
    // re-deriving and keeps the head where speculation put it.
    #[test]
    fn speculation_advances_head_then_reconciles_without_redrive() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            // Body present in the marshal buffer (we voted to notarize it).
            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(ANCHOR + 1, order.clone());

            mailbox.send(spec_msg(&order)).expect("send spec");
            // Finalize the same order — reconciliation must skip the re-derive.
            let (msg, waiter) = finalize_msg(order.clone());
            mailbox.send(msg).expect("send finalize");
            waiter.await.expect("ack");

            {
                let payloads = fx.beacon.new_payload_calls.lock().unwrap();
                let heights: Vec<u64> = payloads.iter().map(|p| p.number).collect();
                assert_eq!(
                    heights,
                    vec![ANCHOR + 1],
                    "imported once at notarization; finalize skipped the re-derive"
                );
                let derived = fx.chain.spec_executed_hash(ANCHOR + 1).unwrap();
                let fcus = fx.beacon.fcu_calls.lock().unwrap();
                assert_eq!(
                    fcus.last().unwrap().head_block_hash,
                    derived,
                    "head sits on the speculatively-executed block"
                );
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A notarized block that then gets nullified (a sibling finalizes) is rolled
    // back: the finalized sibling is derived and the head reorgs onto it.
    #[test]
    fn speculation_rolls_back_to_finalized_sibling() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // `extra_data` distinguishes the siblings: ANCHOR+1 is in the
            // pre-activation window, so both commit `result == ZERO`.
            let order_a = OrderBlock {
                extra_data: Bytes::from_static(b"A"),
                ..sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO)
            };
            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(ANCHOR + 1, order_a.clone());
            mailbox.send(spec_msg(&order_a)).expect("send spec A");

            // A different sibling B finalizes (A was nullified).
            let order_b = OrderBlock {
                extra_data: Bytes::from_static(b"B"),
                ..sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO)
            };
            let (msg, waiter) = finalize_msg(order_b.clone());
            mailbox.send(msg).expect("send finalize B");
            waiter.await.expect("ack");

            {
                let payloads = fx.beacon.new_payload_calls.lock().unwrap();
                assert_eq!(
                    payloads.len(),
                    2,
                    "A speculated, then B re-derived on rollback"
                );
                let hash_b = sealed_at(fx.anchor_hash, ANCHOR + 1, order_b.digest().0).hash();
                let hash_a = sealed_at(fx.anchor_hash, ANCHOR + 1, order_a.digest().0).hash();
                assert_ne!(hash_a, hash_b, "siblings must seal to distinct hashes");
                let fcus = fx.beacon.fcu_calls.lock().unwrap();
                assert_eq!(
                    fcus.last().unwrap().head_block_hash,
                    hash_b,
                    "head reorged onto the finalized sibling"
                );
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A block on a beacon-active round whose σ has not landed is held, not
    // parked: no derive, no ack, no park (`deferred` stays empty), no hint, and
    // no marshal fetch. Recording σ into the actor's seed store fires the
    // seed-record Notify, and the executor's real `seed_notify` select! arm —
    // not a hand-driven call — derives and acks it; that arm is the hold's only
    // exit.
    #[test]
    fn a_held_block_derives_when_its_seed_lands_through_the_real_notify_arm() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let h = ANCHOR + 1;
            let store = crate::beacon::testing::SeedStore::new();
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(store.clone())
                .with_epocher(beacon_active_epocher());
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            let o1 = OrderBlock {
                proposal_view: h,
                ..sample_order(Digest(B256::ZERO), h, B256::ZERO)
            };
            let (m1, mut w1) = finalize_msg(o1.clone());
            mailbox.send(m1).expect("send h");
            ctx.sleep(Duration::from_millis(20)).await;
            assert!(
                fx.beacon.new_payload_calls.lock().unwrap().is_empty(),
                "the block must be HELD underived until its σ lands"
            );
            assert!(
                (&mut w1).now_or_never().is_none(),
                "the held ack must stay pending (never acked before derive)"
            );
            assert!(
                fx.marshal.hints.lock().unwrap().is_empty(),
                "a hold is not a park: no hint is issued"
            );
            assert!(
                fx.marshal.fetched.lock().unwrap().is_empty(),
                "a hold asks the marshal for nothing"
            );

            let seed = real_seed(active_round(h));
            store.record(real_witness(seed.target_round));
            // `wait_until` panics after 2000 virtual ms, so a regression fails
            // here instead of hanging on the ack below.
            wait_until(&ctx, "the notify arm derived the held block", || {
                fx.chain.finalized_executed_hash(h).is_some()
            })
            .await;
            w1.await.expect("h acks once its σ lands");
            assert_eq!(
                fx.deriver.seeds_seen.lock().unwrap().as_slice(),
                &[(h, Some(seed))],
                "the late σ is what the derive used"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // No per-height finalization cert exists (the executor has no cert lookup),
    // `spec_executed` is empty (a restarted / lagging / following node), and no
    // successor has been delivered: the height derives with the real threshold
    // seed of its own round and acks.
    #[test]
    fn a_height_derives_from_its_own_round_without_any_cert_or_speculation() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let seed = real_seed(active_round(ANCHOR + 1));
            let store = crate::beacon::testing::SeedStore::new();
            store.record(real_witness(seed.target_round));
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(store)
                .with_epocher(beacon_active_epocher());
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            let order = OrderBlock {
                proposal_view: ANCHOR + 1,
                ..sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO)
            };
            let (msg, waiter) = finalize_msg(order);
            mailbox.send(msg).expect("send h");
            waiter.await.expect("h derives + acks from the store alone");

            assert_eq!(
                fx.deriver.seeds_seen.lock().unwrap().as_slice(),
                &[(ANCHOR + 1, Some(seed))],
                "σ of h's own round reached the deriver verbatim"
            );
            assert!(
                fx.marshal.fetched.lock().unwrap().is_empty(),
                "no successor was needed — zero marshal fetches"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A pre-bootstrap link — a height whose epoch the agreed map calls
    // beacon-inactive — derives immediately with the `order.digest()` fallback:
    // no hold, no hint. `None` is the agreed derivation there, not a miss;
    // holding would wedge every pre-beacon height forever.
    #[test]
    fn pre_bootstrap_link_derives_with_fallback_without_hinting() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let (msg, waiter) = finalize_msg(order.clone());
            mailbox.send(msg).expect("send");
            waiter.await.expect("ack");

            assert_eq!(
                fx.beacon.new_payload_calls.lock().unwrap().len(),
                1,
                "derived immediately with the fallback"
            );
            assert!(
                fx.marshal.hints.lock().unwrap().is_empty(),
                "a pre-bootstrap link must not hint a re-fetch"
            );
            // The deriver received `None` (the fallback), not a fabricated seed.
            assert_eq!(
                fx.deriver.seeds_seen.lock().unwrap().as_slice(),
                &[(ANCHOR + 1, None)],
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // σ is in the store for the block's own round, but the agreed epoch map
    // calls that epoch beacon-inactive, so the derive must ignore it and use
    // `None` — what the rest of the network derives.
    #[test]
    fn a_seed_at_a_beacon_inactive_round_is_ignored_not_obeyed() {
        use commonware_consensus::types::{Epoch, Epocher as _, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const VIEW: u64 = 77;
            // The default fixture's single huge epocher puts every height in
            // epoch 0, below `DETERMINISTIC_BOOTSTRAP_EPOCH` — beacon-inactive.
            // Filed directly rather than through `record_fixture_seed`, which
            // keys on the beacon-active epoch: the round wanted here is the one
            // this fixture's epocher names, and it is the inactive one.
            let round = Round::new(Epoch::new(0), View::new(VIEW));
            FIXTURE_SEEDS.with(|seeds| seeds.record(real_witness(round)));
            assert!(
                FIXTURE_SEEDS.with(|seeds| seeds.seed(round).is_some()),
                "premise: σ IS recorded for the round this block names"
            );

            let fx = Fixture::new(ANCHOR);
            assert!(
                fx.epocher
                    .containing(Height::new(ANCHOR + 1))
                    .expect("nameable")
                    .epoch()
                    .get()
                    < crate::beacon::testing::DETERMINISTIC_BOOTSTRAP_EPOCH,
                "premise: the block's own epoch is beacon-INACTIVE"
            );
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            let order = OrderBlock {
                proposal_view: VIEW,
                ..sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO)
            };
            let (msg, waiter) = finalize_msg(order);
            mailbox.send(msg).expect("send h");
            waiter.await.expect("ack");

            assert_eq!(
                fx.deriver.seeds_seen.lock().unwrap().as_slice(),
                &[(ANCHOR + 1, None)],
                "the stray σ must not reach the deriver — the network derives `None` here"
            );
            assert!(
                !fx.safety_halt.is_engaged(),
                "ignoring is fork-safe; halting is not"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // At the first block of the first beacon-active epoch `witness_link` keys the
    // wire field on the parent's epoch (`mandatory_at(1)` — false), so an honest
    // block there carries no seed, while the beacon is active at its own epoch
    // and σ for `Round(2, 16)` exists: only keying the derive at the block's own
    // round finds it.
    //
    // The successor derives with σ(2, 17), its own round, not with the edge's.
    #[test]
    fn the_bootstrap_edge_derives_from_its_own_round_though_the_wire_carries_none() {
        use commonware_consensus::types::{Epoch, Epocher as _, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let epocher =
                crate::epocher::OriginEpocher::new(0, std::num::NonZeroU64::new(8).expect("nz"));
            const ANCHOR: u64 = 15;
            const EDGE: u64 = 16;
            assert_eq!(
                epocher.containing(Height::new(EDGE)).unwrap().first(),
                Height::new(EDGE),
                "test premise: EDGE is the FIRST block of its epoch"
            );
            assert_eq!(
                epocher.containing(Height::new(EDGE)).unwrap().epoch(),
                Epoch::new(crate::beacon::testing::DETERMINISTIC_BOOTSTRAP_EPOCH),
                "test premise: that epoch is the bootstrap epoch — the first beacon-ACTIVE one"
            );

            let store = crate::beacon::testing::SeedStore::new();
            let e = Epoch::new(crate::beacon::testing::DETERMINISTIC_BOOTSTRAP_EPOCH);
            let seed_edge = real_seed(Round::new(e, View::new(EDGE)));
            let seed_next = real_seed(Round::new(e, View::new(EDGE + 1)));
            store.record(real_witness(seed_edge.target_round));
            store.record(real_witness(seed_next.target_round));

            let fx = Fixture::new(ANCHOR)
                .with_seed_store(store)
                .with_epocher(epocher);
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);

            let o_edge = OrderBlock {
                proposal_view: EDGE,
                ..sample_order(Digest(B256::ZERO), EDGE, B256::ZERO)
            };
            let o_next = OrderBlock {
                proposal_view: EDGE + 1,
                ..sample_order(o_edge.digest(), EDGE + 1, B256::ZERO)
            };

            for order in [o_edge, o_next] {
                let (ack, _w) = Exact::handle();
                actor
                    .on_finalized_block(Span::current(), order, ack)
                    .await
                    .unwrap();
            }

            assert_eq!(
                fx.deriver.seeds_seen.lock().unwrap().as_slice(),
                &[(EDGE, Some(seed_edge)), (EDGE + 1, Some(seed_next))],
                "each height derives from σ of ITS OWN round — the edge from the first \
                 beacon-active round, its successor from the next"
            );
            assert!(!fx.safety_halt.is_engaged());
        });
    }

    /// Carries a real recovered seed, so `spec_executed[h].seed_round`
    /// reconciles against the round the finalized derive resolves.
    fn spec_msg_seeded(order: &OrderBlock, seed: crate::beacon::Seed) -> Message {
        Message {
            cause: Span::current(),
            command: Command::SpecNotarized(Box::new(Notarized {
                digest: order.digest(),
                seed: Some(seed),
            })),
        }
    }

    // Speculation must run at every height at the tip, so the reth head advances
    // at notarization latency while the finalized derive rides exactly one block
    // behind.
    #[test]
    fn speculation_runs_at_every_height_at_the_tip() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const N: u64 = 4;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // A linked seedless chain ANCHOR+1..=ANCHOR+N whose `result` fields
            // commit the hashes the deriver will produce: every height now
            // derives at its own delivery, so every cross-check actually runs.
            let chain = result_consistent_chain(ANCHOR, fx.anchor_hash, N);
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                for order in &chain {
                    canned.insert(order.height, order.clone());
                }
            }

            // The real tip interleaving: notarization of h, then finalization of
            // h−1 (finalization lags one round behind).
            let mut waiters = Vec::new();
            for i in 1..=N {
                let order = &chain[(i - 1) as usize];
                mailbox.send(spec_msg(order)).expect("send spec");
                if i >= 2 {
                    let (m, w) = finalize_msg(chain[(i - 2) as usize].clone());
                    mailbox.send(m).expect("send finalize");
                    waiters.push(w);
                }
                // Per height: speculation ran (the EL head advanced at
                // notarization latency) — payload for `h` present.
                let target = ANCHOR + i;
                wait_until(&ctx, "speculative import at h", || {
                    fx.beacon
                        .new_payload_calls
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|p| p.number == target)
                })
                .await;
            }
            // Every finalized height (through N−2, whose child arrived) acked.
            for w in waiters.drain(..N as usize - 2) {
                w.await.expect("finalized ack");
            }

            {
                // Each height imported exactly once — the finalized reconcile
                // reused every speculation (correctly_speculated, no re-derive).
                let payloads = fx.beacon.new_payload_calls.lock().unwrap();
                let heights: Vec<u64> = payloads.iter().map(|p| p.number).collect();
                assert_eq!(
                    heights,
                    (1..=N).map(|i| ANCHOR + i).collect::<Vec<_>>(),
                    "speculation ran at EVERY height, once (no park, no re-derive)"
                );
                // The head is at the notarization tip (ANCHOR+N) while `safe`
                // (the finalized derive) is at ANCHOR+N−2 — one block behind the
                // last delivered finalization, which itself lags notarization.
                let fcus = fx.beacon.fcu_calls.lock().unwrap();
                let last = fcus.last().unwrap();
                assert_eq!(
                    last.head_block_hash,
                    fx.chain.spec_executed_hash(ANCHOR + N).unwrap(),
                    "the EL head rides speculation (notarization latency)"
                );
            }
            assert!(
                fx.marshal.hints.lock().unwrap().is_empty(),
                "steady state: no park, no hint"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // At the tip (`last_tip < h + K`) guard #2 never fires, so the derive path
    // issues no `fetch_block_by_height`: σ is resolved locally and every
    // delivered block is its own walk element.
    #[test]
    fn steady_state_derive_issues_no_marshal_fetches() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            let chain = result_consistent_chain(ANCHOR, fx.anchor_hash, 3);
            let mut waiters = Vec::new();
            for order in chain {
                let (m, w) = finalize_msg(order);
                mailbox.send(m).expect("send");
                waiters.push(w);
            }
            for w in waiters {
                w.await.expect("ack");
            }

            assert!(
                fx.marshal.fetched.lock().unwrap().is_empty(),
                "steady state: no h+K fetch (guard #2 cold) and no by-height re-fetch"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // Guard #2 with `last_tip >= h + K`: a catching-up node whose
    // committee-attested block at `h + K` disagrees with the hash it derived
    // engages SafetyHalt(ResultDivergence) before the ack. The fixture's
    // `FakeDeriver` lands the derived hash at derive time (`land_on_import` off,
    // the default), so the guard sees it immediately; reth's FCU-only
    // canonicalisation would read `None` and the halt come K blocks later.
    #[test]
    fn guard2_convergence_mismatch_engages_safety_halt() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const H: u64 = ANCHOR + 1;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            let order_h = sample_order(Digest(B256::ZERO), H, B256::ZERO);
            // The attested root at H+K commits a different hash than the derive
            // produces, so the guard sees a fork.
            let forged = B256::repeat_byte(0xEE);
            let order_hk = sample_order(Digest(B256::ZERO), H + K, forged);
            fx.marshal.canned.lock().unwrap().insert(H + K, order_hk);

            // The node is behind: the finalized frontier is already past H+K.
            mailbox.send(tip_msg(H + K)).expect("send tip");
            let (m, w) = finalize_msg(order_h.clone());
            mailbox.send(m).expect("send finalize");

            let post_halt = sample_order(Digest(B256::ZERO), H + 2, B256::ZERO);
            assert_parked_retaining_acks(&ctx, handle, w, &mailbox, &fx.safety_halt, post_halt)
                .await;
            assert_eq!(
                fx.sync_metrics.degraded_value(SyncReason::ResultDivergence),
                1,
                "the result_divergence gauge is raised for the alert"
            );
        });
    }

    // Guard #2's absent-body arm — the executor's only park: the node is behind
    // (`tip >= h + K`) but the attested body at `h + K` is not backfilled, so
    // the executor parks before the ack and before the `split_off` prune, hints
    // exactly `h + K`, holds the next delivery queued behind the park (the drain
    // is gated on it), and keeps speculation suppressed (the `spec_execute`
    // guard is not narrowed). When the body lands, the re-poke re-derives with
    // the retained seed and the queued child derives right after.
    #[test]
    fn guard2_body_absent_parks_then_derives_when_body_lands() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const H: u64 = ANCHOR + 1;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            let order_h = sample_order(Digest(B256::ZERO), H, B256::ZERO);
            let child = child_of(&order_h);

            mailbox
                .send(tip_msg(H + K))
                .expect("send tip (node behind)");
            let (m, w) = finalize_msg(order_h.clone());
            mailbox.send(m).expect("send finalize");
            let (mc, wc) = finalize_msg(child.clone());
            mailbox.send(mc).expect("send child");

            // The derive ran (import happened) but the ack is withheld, parked on
            // the absent H+K body with exactly H+K hinted.
            ctx.sleep(Duration::from_millis(50)).await;
            {
                let payloads = fx.beacon.new_payload_calls.lock().unwrap();
                let heights: Vec<u64> = payloads.iter().map(|p| p.number).collect();
                assert!(
                    !heights.is_empty(),
                    "H derived + imported before parking on the absent h+K body"
                );
                // H may import more than once — every later delivery re-pokes the
                // park, and the re-poke re-derives. What must not appear is H+1:
                // the drain is gated while a block is parked.
                assert!(
                    heights.iter().all(|n| *n == H),
                    "only H derived; H+1 stays queued behind the park"
                );
            }
            assert_eq!(
                *fx.marshal.hints.lock().unwrap(),
                vec![H + K],
                "the park hints a SINGLE height: h + K"
            );
            assert!(
                !fx.safety_halt.is_engaged(),
                "an absent-body park is not a fork"
            );

            // Speculation stays suppressed while parked — the guard is kept.
            let spec_order = child_of(&child);
            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(spec_order.height, spec_order.clone());
            mailbox
                .send(spec_msg(&spec_order))
                .expect("send spec while parked");
            ctx.sleep(Duration::from_millis(20)).await;
            assert!(
                !fx.beacon
                    .new_payload_calls
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|p| p.number == spec_order.height),
                "speculation must stay suppressed while a block is parked (guard kept)"
            );

            // The H+K body lands, attesting the hash the derive produced; the tip
            // re-poke re-derives with the retained σ and acks.
            let attested = fx.chain.spec_executed_hash(H).unwrap();
            let order_hk = sample_order(Digest(B256::ZERO), H + K, attested);
            fx.marshal.canned.lock().unwrap().insert(H + K, order_hk);
            // Same-height tip: the re-poke is the event; keeping the tip at H+K
            // leaves guard #2 cold for the held child (tip < child + K), so the
            // child derives and acks below without needing an H+K+1 body.
            mailbox.send(tip_msg(H + K)).expect("send tip re-poke");
            w.await.expect("parked block acks once the h+K body lands");

            // The child was queued behind the park, never dropped: the drain
            // resumes the moment the park clears and derives it.
            wc.await
                .expect("the queued child derives after the park clears");

            assert!(!fx.safety_halt.is_engaged(), "clean convergence → no halt");
            drop(mailbox);
            let _ = handle.await;
        });
    }

    // The guard-#2 park's delivery-independent backstop: a body landing at
    // `height <= tip` fires no `Update::Tip`, so the FCU-heartbeat re-poke
    // clears the park (a reused tick, no new timer). The re-poke re-derives from
    // the retained σ (`Deferred::seed`) with zero lookups.
    #[test]
    fn guard2_park_clears_on_heartbeat_repoke_without_delivery() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const H: u64 = ANCHOR + 1;
            let fx = Fixture::new(ANCHOR).with_fcu_heartbeat(Duration::from_millis(20));
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            let order_h = sample_order(Digest(B256::ZERO), H, B256::ZERO);
            mailbox
                .send(tip_msg(H + K))
                .expect("send tip (node behind)");
            let (m, w) = finalize_msg(order_h.clone());
            mailbox.send(m).expect("send finalize");
            ctx.sleep(Duration::from_millis(50)).await;
            assert_eq!(
                *fx.marshal.hints.lock().unwrap(),
                vec![H + K],
                "parked on the absent h+K body"
            );

            // The body lands silently (no tip, no delivery) — only the heartbeat
            // (auto-advanced by the deterministic clock) can re-poke the park.
            let attested = fx.chain.spec_executed_hash(H).unwrap();
            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(H + K, sample_order(Digest(B256::ZERO), H + K, attested));
            w.await
                .expect("parked block acks on the heartbeat re-poke (no delivery event)");

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A held block is never acked before it is derived, even at shutdown: an ack
    // would durably advance the marshal's `last_processed_height` past an
    // underived height — a permanent hole. The drop (→ Canceled) is deliberate;
    // the withheld ack is the restart self-heal.
    #[test]
    fn held_block_is_never_acked_at_shutdown() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            // Beacon-active epoch + an empty store: the only way to hold a block.
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(crate::beacon::testing::SeedStore::new())
                .with_epocher(beacon_active_epocher());
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let (m, w) = finalize_msg(order);
            mailbox.send(m).expect("send h (becomes the held tip)");
            ctx.sleep(Duration::from_millis(20)).await;

            // Dropping the mailbox stops the executor while it holds `h`.
            drop(mailbox);
            handle.await.expect("executor exits on mailbox close");

            // The ack resolved canceled (dropped), not Ok: the executor did not
            // acknowledge a block it never derived.
            assert!(
                w.await.is_err(),
                "the held ack must resolve Canceled at shutdown — an Ok would durably \
                 skip the underived height on restart"
            );
            assert!(
                fx.chain.spec_executed_hash(ANCHOR + 1).is_none(),
                "the held height was never derived"
            );
            assert!(
                fx.beacon.new_payload_calls.lock().unwrap().is_empty(),
                "no import happened for the held height"
            );
        });
    }

    // A node that stopped while holding `h` for its σ re-dispatches `h` (the
    // marshal's `last_processed_height` never advanced) and derives it once σ is
    // there, with no hole — the hash equals the one a never-stopped node
    // derives. `awaiting_seed` needs no persistence; the withheld ack is the
    // durable record.
    #[test]
    fn restart_after_stop_while_holding_rederives_without_hole() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let store = crate::beacon::testing::SeedStore::new();
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(store.clone())
                .with_epocher(beacon_active_epocher());
            let order = OrderBlock {
                proposal_view: ANCHOR + 1,
                ..sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO)
            };
            let seed = real_seed(active_round(ANCHOR + 1));
            // The golden value a never-stopped node derives for `h`.
            let golden = sealed_at(
                fx.anchor_hash,
                ANCHOR + 1,
                seed_folded_discriminator(order.digest(), &Some(seed.clone())),
            )
            .hash();

            // Deliver `h`, stop while holding it.
            {
                let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
                let handle = actor.start();
                let (m, w) = finalize_msg(order.clone());
                mailbox.send(m).expect("send h");
                ctx.sleep(Duration::from_millis(20)).await;
                drop(mailbox);
                let _ = handle.await;
                assert!(w.await.is_err(), "held ack dropped at stop (never Ok)");
                assert!(
                    fx.chain.spec_executed_hash(ANCHOR + 1).is_none(),
                    "hole before restart"
                );
            }

            // The restarted run: the marshal re-dispatches from
            // `last_processed + 1` = the held height. σ lands while `h` is held
            // and the notify arm derives it. (A fresh metrics label: a real
            // restart is a fresh process.)
            {
                let (actor, mailbox) = fx.build(ctx.with_label("restart"), ANCHOR, ANCHOR);
                let handle = actor.start();
                let (m, w) = finalize_msg(order.clone());
                mailbox.send(m).expect("re-dispatch h");
                store.record(real_witness(seed.target_round));
                w.await.expect("h acks on the restarted run — no hole");
                assert_eq!(
                    fx.chain.spec_executed_hash(ANCHOR + 1),
                    Some(golden),
                    "the restarted derive equals the never-stopped node's hash"
                );
                drop(mailbox);
                let _ = handle.await;
            }
        });
    }

    // A first-seen spin notarization must not speculate with the spin round's
    // seed: the round is re-canonicalised to the block's own `proposal_view`.
    // Without a `SeedIndex` entry for the canonical round the speculation is
    // skipped (never speculate with a known-wrong seed); once σ for that round
    // lands the finalized path derives it exactly once, no reorg.
    #[test]
    fn spin_notarization_without_canonical_seed_skips_speculation() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const H: u64 = ANCHOR + 1;
            const V0: u64 = 40;
            let store = crate::beacon::testing::SeedStore::new();
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(store.clone())
                .with_epocher(beacon_active_epocher());
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            let order = OrderBlock {
                proposal_view: V0,
                ..sample_order(Digest(B256::ZERO), H, B256::ZERO)
            };
            fx.marshal.canned.lock().unwrap().insert(H, order.clone());
            let canonical = active_round(V0);
            let seed_v0 = real_seed(canonical);
            let seed_spin = real_seed(active_round(V0 + 30));

            // First-seen notarization at a spin round, store empty → skip (no
            // import).
            mailbox
                .send(spec_msg_seeded(&order, seed_spin))
                .expect("send spin spec");
            ctx.sleep(Duration::from_millis(20)).await;
            assert!(
                fx.beacon.new_payload_calls.lock().unwrap().is_empty(),
                "must NOT speculate with a known-wrong (spin-round) seed"
            );

            // σ for the canonical round lands; the finalized path derives from it
            // — exactly once, no reorg.
            store.record(real_witness(canonical));
            let (m, w) = finalize_msg(order.clone());
            mailbox.send(m).expect("send finalize");
            w.await.expect("ack");
            assert_eq!(
                fx.deriver.seeds_seen.lock().unwrap().as_slice(),
                &[(H, Some(seed_v0))],
                "derived once, from σ of the block's own round"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A node that holds the canonical round's seed in its `SeedIndex`
    // re-canonicalises the spin notarization and speculates with the same seed
    // everyone else uses; the finalized reconcile then reuses the speculation
    // (rounds match, no re-derive, no reorg).
    #[test]
    fn spin_notarization_recanonicalises_from_seed_store() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const H: u64 = ANCHOR + 1;
            const V0: u64 = 40;
            let store = crate::beacon::testing::SeedStore::new();
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(store.clone())
                .with_epocher(beacon_active_epocher());
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            let order = OrderBlock {
                proposal_view: V0,
                ..sample_order(Digest(B256::ZERO), H, B256::ZERO)
            };
            fx.marshal.canned.lock().unwrap().insert(H, order.clone());
            let canonical = active_round(V0);
            let seed_v0 = real_seed(canonical);
            let seed_spin = real_seed(active_round(V0 + 30));
            store.record(real_witness(canonical));

            // Spin-round notarization → re-canonicalised to (e, V0) via the store.
            mailbox
                .send(spec_msg_seeded(&order, seed_spin))
                .expect("send spin spec");
            // Finalize resolves the same round → reconcile reuses the spec.
            let (m, w) = finalize_msg(order.clone());
            mailbox.send(m).expect("send finalize");
            w.await.expect("ack");

            assert_eq!(
                fx.deriver.seeds_seen.lock().unwrap().as_slice(),
                &[(H, Some(seed_v0))],
                "speculated ONCE with the re-canonicalised seed; the finalize reused it"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // Two nodes whose local cert state named different spin rounds for the same
    // height both derive it from σ of the block's own agreed round, so their
    // hashes agree.
    #[test]
    fn nodes_with_divergent_local_cert_state_derive_identically_from_the_agreed_round() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let order = OrderBlock {
                proposal_view: 40,
                ..sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO)
            };
            record_fixture_seed(40);
            let agreed = real_seed(active_round(40));

            let mut hashes = Vec::new();
            for node in 0..2 {
                let fx = Fixture::new(ANCHOR).with_epocher(beacon_active_epocher());
                let (actor, mailbox) =
                    fx.build(ctx.with_label(&format!("node{node}")), ANCHOR, ANCHOR);
                let handle = actor.start();
                let (m, w) = finalize_msg(order.clone());
                mailbox.send(m).expect("send h");
                w.await.expect("ack");
                assert_eq!(
                    fx.deriver.seeds_seen.lock().unwrap().as_slice(),
                    &[(ANCHOR + 1, Some(agreed.clone()))],
                    "both nodes resolved σ at the block's own agreed round"
                );
                hashes.push(fx.chain.spec_executed_hash(ANCHOR + 1).unwrap());
                drop(mailbox);
                let _ = handle.await;
            }
            assert_eq!(
                hashes[0], hashes[1],
                "the round is agreed data — every node derives the identical hash"
            );
        });
    }

    // A multi-height speculative lead (spec_head 3 ahead) where a sibling
    // finalizes mid-lead must roll back exactly at the diverging height: the
    // finalized sibling is re-derived and the speculative entries strictly above
    // it (split_off) are dropped so the next notarization re-speculates forward.
    #[test]
    fn multi_height_speculation_rolls_back_at_sibling_mid_lead() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // Build a speculative lead of 3: ANCHOR+1, +2, +3 (each parent links
            // to the prior digest so they chain). All in the pre-activation window
            // (anchor+K = 103) → finalized blocks commit `result == ZERO`; the +2
            // siblings are distinguished by `extra_data`.
            let o1 = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let o2a = OrderBlock {
                extra_data: Bytes::from_static(b"A"),
                ..sample_order(o1.digest(), ANCHOR + 2, B256::ZERO)
            };
            let o3 = sample_order(o2a.digest(), ANCHOR + 3, B256::ZERO);
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                canned.insert(ANCHOR + 1, o1.clone());
                canned.insert(ANCHOR + 2, o2a.clone());
                canned.insert(ANCHOR + 3, o3.clone());
            }
            mailbox.send(spec_msg(&o1)).expect("spec 1");
            mailbox.send(spec_msg(&o2a)).expect("spec 2a");
            mailbox.send(spec_msg(&o3)).expect("spec 3");

            // Finalize ANCHOR+1 as speculated (no re-derive), then a sibling B at
            // ANCHOR+2 finalizes — o2a was nullified. Rollback derives B at +2;
            // the +3 speculation (built on the orphaned o2a) is discarded.
            let (m1, w1) = finalize_msg(o1.clone());
            mailbox.send(m1).expect("send finalize 1");

            let o2b = OrderBlock {
                extra_data: Bytes::from_static(b"B"),
                ..sample_order(o1.digest(), ANCHOR + 2, B256::ZERO)
            };
            let (m2b, w2b) = finalize_msg(o2b.clone());
            mailbox.send(m2b).expect("send finalize 2b");
            w1.await.expect("ack 1");
            w2b.await.expect("ack 2b");

            {
                let payloads = fx.beacon.new_payload_calls.lock().unwrap();
                let heights: Vec<u64> = payloads.iter().map(|p| p.number).collect();
                // 3 speculative imports (101,102,103) + 1 rollback re-derive (102).
                assert_eq!(
                    heights,
                    vec![ANCHOR + 1, ANCHOR + 2, ANCHOR + 3, ANCHOR + 2],
                    "speculated 3-deep then re-derived the finalized sibling at +2"
                );
                let hash_b = sealed_at(
                    fx.chain.spec_executed_hash(ANCHOR + 1).unwrap(),
                    ANCHOR + 2,
                    o2b.digest().0,
                )
                .hash();
                let fcus = fx.beacon.fcu_calls.lock().unwrap();
                let last = fcus.last().unwrap();
                assert_eq!(
                    last.head_block_hash, hash_b,
                    "head reorged back onto the finalized sibling at +2"
                );
                // The `>=` guard let `safe` follow the same-height sibling reorg
                // onto the finalized hash — never stuck on the orphaned o2a.
                // (FakeBeacon returns Valid unconditionally and does not model
                // reth's `find_canonical_header`, so this value assert is the
                // only thing that catches an orphan-safe bug.)
                assert_eq!(
                    last.safe_block_hash, hash_b,
                    "safe followed the reorg onto the finalized sibling (not orphaned o2a)"
                );
                assert_eq!(
                    last.safe_block_hash, last.head_block_hash,
                    "no surviving spec lead after the rollback ⇒ safe == head"
                );
                // `safe` is a block reth was told about (imported) at a height
                // ≤ head before the FCU named it — the precondition reth's real
                // `find_canonical_header(safe) == Some` relies on.
                assert!(
                    payloads.iter().any(|p| p.hash() == last.safe_block_hash),
                    "safe was imported (new_payload'd) before the FCU named it"
                );
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A notarization for a height ahead of `spec_head` (a gap) is parked, not
    // dropped, and resumes speculation once `spec_head` catches up via the
    // finalized path. Here 103's notarization arrives while `spec_head ==
    // ANCHOR (100)` — a gap — and it is the only spec message for 103; 103's own
    // finalized derive needs its child 104, which is never delivered, so a
    // speculatively executed 103 can only come from the drain firing when the
    // finalized derive of 102 advances `spec_head` to 102.
    #[test]
    fn parked_gap_notarization_resumes_speculation_on_finalized_advance() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            // `result`-consistent: 103 now derives on the finalized path too, so
            // its committed result must match what the deriver produces at 100.
            let chain = result_consistent_chain(ANCHOR, fx.anchor_hash, 3);
            let (o1, o2, o3) = (chain[0].clone(), chain[1].clone(), chain[2].clone());
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                for order in &chain {
                    canned.insert(order.height, order.clone());
                }
            }

            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // The notarization for 103 arrives while spec_head is still the anchor
            // (100): a gap (103 > 101), so it is parked.
            mailbox
                .send(spec_msg(&o3))
                .expect("spec 103 (gap → parked)");

            // Finalize 101, 102, 103. 102's derive advances `spec_head` to 102,
            // which is what fires the drain for the parked 103 notarization.
            for order in [o1.clone(), o2.clone(), o3.clone()] {
                let (m, _w) = finalize_msg(order);
                mailbox.send(m).expect("finalize");
            }

            // 103 becomes executed only via the parked drain (no 104, so no
            // finalized derive of 103 and no re-sent live notarization).
            wait_until(&ctx, "parked 103 resumed via drain", || {
                fx.chain.spec_executed_hash(ANCHOR + 3).is_some()
            })
            .await;

            // Exactly one derive of 103 (the drained speculation), on top of the
            // finalized 102 — no double-derive, no rollback.
            let payloads_103: Vec<u64> = fx
                .beacon
                .new_payload_calls
                .lock()
                .unwrap()
                .iter()
                .map(|p| p.number)
                .filter(|n| *n == ANCHOR + 3)
                .collect();
            assert_eq!(
                payloads_103,
                vec![ANCHOR + 3],
                "103 derived exactly once, via the parked-notarization drain"
            );
            assert!(
                !fx.safety_halt.is_engaged(),
                "no divergence on the drain path"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // On a `spec_head` advance the drain drops every parked height ≤ `spec_head`
    // (finalized or already speculated, so stale); a not-yet-drainable higher
    // entry survives. Direct-call so the pre/post parked map is inspectable.
    #[test]
    fn drain_prunes_parked_at_or_below_spec_head() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR); // spec_head = 100
            let cause = Span::current();

            // Park three gap heights (102, 103, 105 — all > spec_head+1 = 101).
            for h in [ANCHOR + 2, ANCHOR + 3, ANCHOR + 5] {
                let o = sample_order(Digest(B256::ZERO), h, B256::ZERO);
                fx.marshal.canned.lock().unwrap().insert(h, o.clone());
                actor
                    .spec_execute(cause.clone(), o.digest(), None)
                    .await
                    .unwrap();
            }
            assert_eq!(
                actor.parked_spec.keys().copied().collect::<Vec<_>>(),
                vec![ANCHOR + 2, ANCHOR + 3, ANCHOR + 5],
                "all three gap notarizations parked"
            );

            // Finalization advanced spec_head to 104; drop 105's body so the drain
            // stops at 105 (keeping it) — isolating the prune from the drain.
            actor.spec_head = ANCHOR + 4;
            fx.marshal.canned.lock().unwrap().remove(&(ANCHOR + 5));
            actor.try_drain_parked(&cause).await.expect("drain");

            assert_eq!(
                actor.parked_spec.keys().copied().collect::<Vec<_>>(),
                vec![ANCHOR + 5],
                "102 and 103 pruned (≤ spec_head 104); 105 (> spec_head, body missing) kept"
            );
        });
    }

    // A later sibling notarization at the same parked height replaces the earlier
    // guess (a wrong guess is safe — `correctly_speculated` reconciles it at
    // finalization).
    #[test]
    fn later_sibling_overwrites_parked_entry() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const GAP: u64 = ANCHOR + 3; // > spec_head+1 ⇒ parked
            let fx = Fixture::new(ANCHOR);
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR); // spec_head = 100
            let cause = Span::current();

            // Two siblings at the same height, distinct proposal_view + extra_data,
            // so distinct digests. FakeMarshal keys `canned` by height, so swap the
            // buffered body between calls to make each fetchable by its own digest.
            let earlier = OrderBlock {
                proposal_view: GAP,
                extra_data: Bytes::from_static(b"A"),
                ..sample_order(Digest(B256::ZERO), GAP, B256::ZERO)
            };
            let later = OrderBlock {
                proposal_view: GAP + 1,
                extra_data: Bytes::from_static(b"B"),
                ..sample_order(Digest(B256::ZERO), GAP, B256::ZERO)
            };
            assert_ne!(earlier.digest(), later.digest(), "distinct sibling digests");

            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(GAP, earlier.clone());
            actor
                .spec_execute(cause.clone(), earlier.digest(), None)
                .await
                .unwrap();
            assert_eq!(
                actor.parked_spec.get(&GAP).unwrap().digest,
                earlier.digest()
            );

            fx.marshal.canned.lock().unwrap().insert(GAP, later.clone());
            actor
                .spec_execute(cause.clone(), later.digest(), None)
                .await
                .unwrap();
            assert_eq!(
                actor.parked_spec.get(&GAP).unwrap().digest,
                later.digest(),
                "the later sibling overwrote the earlier parked entry at the same height"
            );
            assert_eq!(actor.parked_spec.len(), 1, "still one entry at the height");
        });
    }

    // The drain keeps a parked entry (and stops) when its block body is not
    // buffered yet — the body may arrive later; a later advance retries.
    #[test]
    fn drain_keeps_parked_entry_when_body_not_buffered() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const GAP: u64 = ANCHOR + 2;
            let fx = Fixture::new(ANCHOR);
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR); // spec_head = 100
            let cause = Span::current();

            // Park 102 (gap; body canned so spec_execute can learn its height).
            let o = sample_order(Digest(B256::ZERO), GAP, B256::ZERO);
            fx.marshal.canned.lock().unwrap().insert(GAP, o.clone());
            actor
                .spec_execute(cause.clone(), o.digest(), None)
                .await
                .unwrap();
            assert!(actor.parked_spec.contains_key(&GAP));

            // spec_head catches up to 101 so 102 == spec_head+1 is drainable — but
            // the body is now gone (not yet re-buffered).
            actor.spec_head = ANCHOR + 1;
            fx.marshal.canned.lock().unwrap().remove(&GAP);
            actor.try_drain_parked(&cause).await.expect("drain");

            assert!(
                actor.parked_spec.contains_key(&GAP),
                "body-not-buffered ⇒ entry KEPT for a later retry"
            );
            assert_eq!(
                actor.spec_head,
                ANCHOR + 1,
                "no advance while the body is missing"
            );
            assert!(
                fx.chain.spec_executed_hash(GAP).is_none(),
                "102 not executed while its body is unavailable"
            );
        });
    }

    // A live notarization for exactly spec_head+1 whose body is not yet buffered
    // is dropped without parking: the body fetch precedes the height gate, and
    // the height is unknowable without the body. The loss is self-healing — a
    // later higher notarization parks, and the next finalized advance re-drains
    // speculation past the lost height.
    #[test]
    fn bodyless_live_notarization_drop_self_heals_via_later_park() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            // `result`-consistent: 103 derives on the finalized path too, so its
            // committed result must match what the deriver produces at 100.
            let chain = result_consistent_chain(ANCHOR, fx.anchor_hash, 3);
            let (o1, o2, o3) = (chain[0].clone(), chain[1].clone(), chain[2].clone());
            // 101's body is deliberately not buffered (its live notarization is
            // the residual drop); 103's is (it parks).
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                canned.insert(ANCHOR + 2, o2.clone());
                canned.insert(ANCHOR + 3, o3.clone());
            }

            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // The residual: 101 == spec_head+1 but its body is missing → dropped,
            // not parked (height unknowable). 103 is a gap → parked.
            mailbox
                .send(spec_msg(&o1))
                .expect("spec 101 (bodyless → dropped)");
            mailbox
                .send(spec_msg(&o3))
                .expect("spec 103 (gap → parked)");

            // The finalized path crosses the lost height: 101 and 102 derive at
            // their own deliveries (spec_head→102) — the drain then resumes
            // speculation at the parked 103.
            for order in [o1.clone(), o2.clone(), o3.clone()] {
                let (m, _w) = finalize_msg(order);
                mailbox.send(m).expect("finalize");
            }
            wait_until(&ctx, "speculation resumed above the lost height", || {
                fx.chain.spec_executed_hash(ANCHOR + 3).is_some()
            })
            .await;

            // 101 was derived by the finalized path only (its live spec was the
            // residual drop); 103 exactly once, via the drained park.
            let heights: Vec<u64> = fx
                .beacon
                .new_payload_calls
                .lock()
                .unwrap()
                .iter()
                .map(|p| p.number)
                .collect();
            assert_eq!(
                heights.iter().filter(|h| **h == ANCHOR + 1).count(),
                1,
                "the bodyless live notarization for 101 was dropped; only the finalized derive ran"
            );
            assert_eq!(
                heights.iter().filter(|h| **h == ANCHOR + 3).count(),
                1,
                "103 derived exactly once — speculation self-healed via the parked drain"
            );
            assert!(!fx.safety_halt.is_engaged());

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // The parent-not-executed park gate: a notarization at exactly spec_head+1
    // whose parent has not executed is parked, and the drain executes it once
    // the parent lands (a spec_head advance retries it).
    #[test]
    fn parent_missing_notarization_parks_then_drains_when_parent_lands() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let cause = Span::current();

            // spec_head at 101 while 101 is not executed (only the anchor 100 is)
            // — the fall-behind shape where the next notarization's parent is
            // still missing.
            actor.spec_head = ANCHOR + 1;
            let o2 = sample_order(Digest(B256::ZERO), ANCHOR + 2, B256::ZERO);
            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(ANCHOR + 2, o2.clone());

            actor
                .spec_execute(cause.clone(), o2.digest(), None)
                .await
                .unwrap();
            assert!(
                actor.parked_spec.contains_key(&(ANCHOR + 2)),
                "parent-missing at spec_head+1 ⇒ PARKED (pre-fix: dropped)"
            );
            assert!(fx.chain.spec_executed_hash(ANCHOR + 2).is_none());

            // The parent lands (finalized path executed 101) — the drain retries
            // the parked 102 and speculation advances.
            let parent_hash = B256::repeat_byte(0xB1);
            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(ANCHOR + 1, parent_hash);
            actor.try_drain_parked(&cause).await.expect("drain");

            assert!(
                fx.chain.spec_executed_hash(ANCHOR + 2).is_some(),
                "the parked notarization executed once its parent landed"
            );
            assert_eq!(
                actor.spec_head,
                ANCHOR + 2,
                "spec_head advanced via the drain"
            );
            assert!(
                actor.parked_spec.is_empty(),
                "the drained entry was removed"
            );
        });
    }

    // A speculative lead h..h+2 where h finalizes as a sibling (seed-round
    // mismatch) rolls back and re-derives; h+1 then finalizes with the same
    // ordering digest that was speculated and must re-derive on the finalized
    // parent instead of reusing the orphaned speculation.
    #[test]
    fn rolled_back_sibling_child_is_rederived_not_reused() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR).with_epocher(beacon_active_epocher());
            let anchor = fx.anchor_hash;

            // 101 is speculated with a round the agreed map does not name (a
            // divergent local cert state at the block's own view, kept verbatim),
            // so the finalized derive re-keys it and rolls back. 102/103
            // speculate with their own agreed rounds, so only 101 rolls back.
            let spec_seed_101 = real_seed(Round::new(Epoch::new(0), View::new(101)));
            let agreed_101 = real_seed(active_round(101));
            let seed_102 = real_seed(active_round(102));
            let seed_103 = real_seed(active_round(103));
            for view in [101, 102, 103] {
                record_fixture_seed(view);
            }

            let o1 = OrderBlock {
                proposal_view: 101,
                ..sample_order(Digest(B256::ZERO), 101, B256::ZERO)
            };
            let o2 = OrderBlock {
                proposal_view: 102,
                ..sample_order(o1.digest(), 102, B256::ZERO)
            };
            let o3 = OrderBlock {
                proposal_view: 103,
                // 103 − K = the anchor, whose derived hash is `anchor`.
                ..sample_order(o2.digest(), 103, anchor)
            };
            {
                let mut c = fx.marshal.canned.lock().unwrap();
                c.insert(101, o1.clone());
                c.insert(102, o2.clone());
                c.insert(103, o3.clone());
            }

            let hash_spec_101 = sealed_at(
                anchor,
                101,
                seed_folded_discriminator(o1.digest(), &Some(spec_seed_101.clone())),
            )
            .hash();
            let hash_fin_101 = sealed_at(
                anchor,
                101,
                seed_folded_discriminator(o1.digest(), &Some(agreed_101.clone())),
            )
            .hash();
            // Fork-A 102 was speculated on the orphaned 101 (hash_spec_101); the
            // finalized re-derive lands it on the new canonical 101 (hash_fin_101).
            let hash_spec_102 = sealed_at(
                hash_spec_101,
                102,
                seed_folded_discriminator(o2.digest(), &Some(seed_102.clone())),
            )
            .hash();
            let hash_fin_102 = sealed_at(
                hash_fin_101,
                102,
                seed_folded_discriminator(o2.digest(), &Some(seed_102.clone())),
            )
            .hash();
            assert_ne!(
                hash_spec_101, hash_fin_101,
                "101 spec vs finalize differ (seed round)"
            );
            assert_ne!(
                hash_spec_102, hash_fin_102,
                "102 orphaned-parent spec vs finalized-parent re-derive must differ"
            );

            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            let spec = |digest, seed| Message {
                cause: Span::current(),
                command: Command::SpecNotarized(Box::new(Notarized { digest, seed })),
            };
            mailbox
                .send(spec(o1.digest(), Some(spec_seed_101.clone())))
                .unwrap();
            mailbox
                .send(spec(o2.digest(), Some(seed_102.clone())))
                .unwrap();
            mailbox
                .send(spec(o3.digest(), Some(seed_103.clone())))
                .unwrap();
            // Precondition: fork-A 102 is speculated on the orphaned 101.
            wait_until(&ctx, "fork-A 102 speculated", || {
                fx.chain.spec_executed_hash(102) == Some(hash_spec_102)
            })
            .await;

            // Each height derives at its own delivery: 101 (round mismatch →
            // rollback + re-derive), then 102 (must re-derive, not reuse fork-A),
            // then 103.
            for order in [o1.clone(), o2.clone(), o3.clone()] {
                let (m, _w) = finalize_msg(order);
                mailbox.send(m).unwrap();
            }
            wait_until(&ctx, "102 re-derived on the finalized fork", || {
                fx.chain.spec_executed_hash(102) == Some(hash_fin_102)
            })
            .await;

            assert_eq!(
                fx.chain.spec_executed_hash(101),
                Some(hash_fin_101),
                "101 re-derived with the agreed seed"
            );
            assert_eq!(
                fx.chain.spec_executed_hash(102),
                Some(hash_fin_102),
                "102 re-derived on the finalized 101 (fork-A speculation NOT reused)"
            );
            // 103 derives on top of the re-derived 102, so the final head is
            // 103's hash — but it must descend from hash_fin_102, and the head
            // must have visited hash_fin_102 on the way.
            assert!(
                fx.beacon
                    .fcu_calls
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|f| f.head_block_hash == hash_fin_102),
                "head ADVANCED onto the re-derived 102 (pre-fix it stayed stuck at 101)"
            );
            let hash_fin_103 = sealed_at(
                hash_fin_102,
                103,
                seed_folded_discriminator(o3.digest(), &Some(seed_103.clone())),
            )
            .hash();
            assert_eq!(
                fx.chain.spec_executed_hash(103),
                Some(hash_fin_103),
                "103 landed on the RE-DERIVED 102, not on the orphaned fork-A"
            );
            let derives_102 = fx
                .beacon
                .new_payload_calls
                .lock()
                .unwrap()
                .iter()
                .filter(|p| p.number == 102)
                .count();
            assert_eq!(
                derives_102, 2,
                "102 re-derived at finalize (reuse would be exactly one import)"
            );
            assert!(
                !fx.safety_halt.is_engaged(),
                "clean re-heal, no divergence halt"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // Parent-linkage, isolated: a speculated block whose recorded parent does
    // not match the block canonical at `height − 1` is rejected by
    // `correctly_speculated` and re-derived even though the seed round and the
    // ordering digest both match. The seed value is identical on both paths, so
    // the parent is the only difference between the reuse hash and the re-derive
    // hash.
    #[test]
    fn stale_parent_speculation_is_rejected_despite_matching_round() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR).with_epocher(beacon_active_epocher());
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let cause = Span::current();

            // 101 speculated with the same σ the finalized derive resolves for
            // its own round ⇒ the round clause passes on both paths.
            let seed = real_seed(active_round(101));
            record_fixture_seed(101);
            let o1 = OrderBlock {
                proposal_view: 101,
                ..sample_order(Digest(B256::ZERO), 101, B256::ZERO)
            };
            fx.marshal.canned.lock().unwrap().insert(101, o1.clone());
            actor
                .spec_execute(cause.clone(), o1.digest(), Some(seed.clone()))
                .await
                .unwrap();
            let hash_spec = fx.chain.spec_executed_hash(101).unwrap();

            // A parent reorg with no rollback event of its own: the block canonical
            // at 100 changes out from under the recorded speculation (stored parent
            // == `anchor`).
            let stale_parent = B256::repeat_byte(0xEE);
            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(ANCHOR, stale_parent);

            // Finalize 101. Same digest, same round → only parent-linkage can
            // reject the reuse.
            let (held_ack, _hw) = Exact::handle();
            actor
                .on_finalized_block(cause.clone(), o1.clone(), held_ack)
                .await
                .unwrap();

            let hash_after = fx.chain.spec_executed_hash(101).unwrap();
            let expected_rederive = sealed_at(
                stale_parent,
                101,
                seed_folded_discriminator(o1.digest(), &Some(seed.clone())),
            )
            .hash();
            assert_ne!(
                hash_after, hash_spec,
                "parent-linkage forced a re-derive despite matching seed round + digest"
            );
            assert_eq!(
                hash_after, expected_rederive,
                "re-derived on the CURRENT canonical parent, not the orphaned spec parent"
            );
            assert!(!actor.safety_halt.is_engaged());
        });
    }

    // A delivered finalized `h` whose own agreed round `Round(0, proposal_view)`
    // is in the seed index is derived and finalized-recorded at delivery, before
    // its child `h+1` exists — closing the recorded_tip = delivered_tip − 1 lag
    // that stalled the finalized-tier result gate.
    #[test]
    fn eager_finalized_derive_records_before_child_arrives() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let h = ANCHOR + 1;
            let store = crate::beacon::testing::SeedStore::new();
            let seed = real_seed(active_round(h));
            store.record(real_witness(seed.target_round));
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(store)
                .with_epocher(beacon_active_epocher());
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let cause = Span::current();

            // `proposal_view == h` ⇒ the eager round is `Round(0, h)`, the exact
            // key recorded above (epoch 0: the fixture's single huge epoch).
            let o1 = OrderBlock {
                proposal_view: h,
                ..sample_order(Digest(B256::ZERO), h, B256::ZERO)
            };
            fx.marshal.canned.lock().unwrap().insert(h, o1.clone());

            let (ack, _w) = Exact::handle();
            actor.on_finalized_block(cause.clone(), o1.clone(), ack).await.unwrap();

            let derived = fx.chain.spec_executed_hash(h).expect("h derived eagerly at delivery");
            assert_eq!(
                fx.chain.finalized_executed_hash(h),
                Some(derived),
                "finalized-tier hash recorded at h BEFORE h+1 is delivered (propose(h+K) would pass)"
            );
            assert!(actor.awaiting_seed.is_none(), "the eager derive CONSUMED the hold");
            assert!(!fx.safety_halt.is_engaged());
        });
    }

    // The fixture's default σ source is a live store, not a negative provider:
    // a witnessed link files σ under the parent's own round, so `h` derives from
    // the store at its own delivery with no `with_seed_store` and no child. A
    // default that silently answered `None` would leave such a link deriving from
    // the child body instead — invisible here, a hang once the child stops
    // carrying it.
    #[test]
    fn the_default_fixture_serves_a_recorded_round_from_its_own_store() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let h = ANCHOR + 1;
            let fx = Fixture::new(ANCHOR).with_epocher(beacon_active_epocher());
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);

            let o1 = OrderBlock {
                proposal_view: h,
                ..sample_order(Digest(B256::ZERO), h, B256::ZERO)
            };
            fx.marshal.canned.lock().unwrap().insert(h, o1.clone());
            let seed = real_seed(active_round(h));
            // Files σ for h's round with no block involved at all: the only
            // source `on_finalized_block` can be reading below is the store.
            record_fixture_seed(o1.proposal_view);

            let (ack, _w) = Exact::handle();
            actor
                .on_finalized_block(Span::current(), o1, ack)
                .await
                .unwrap();

            assert!(
                actor.awaiting_seed.is_none(),
                "the eager derive CONSUMED the hold"
            );
            assert_eq!(
                fx.deriver.seeds_seen.lock().unwrap().as_slice(),
                &[(h, Some(seed))],
                "σ resolved from the fixture's own store, keyed by h's round"
            );
        });
    }

    // With the round absent from the store — and the epoch beacon-active, so
    // `None` is not the agreed answer — the delivered block stays held. The only
    // exit is σ arriving; there is no fallback and no deadline.
    #[test]
    fn a_store_miss_on_a_beacon_active_round_holds_the_block() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let h = ANCHOR + 1;
            // Store present but empty — the opt-out from the fixture default, and
            // the only difference from the hit test is the missing round entry
            // (isolates the miss branch).
            let store = crate::beacon::testing::SeedStore::new();
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(store)
                .with_epocher(beacon_active_epocher());
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let cause = Span::current();

            let o1 = OrderBlock {
                proposal_view: h,
                ..sample_order(Digest(B256::ZERO), h, B256::ZERO)
            };
            fx.marshal.canned.lock().unwrap().insert(h, o1.clone());

            let (ack, _w) = Exact::handle();
            actor
                .on_finalized_block(cause.clone(), o1.clone(), ack)
                .await
                .unwrap();

            assert!(
                fx.chain.spec_executed_hash(h).is_none(),
                "MISS: h NOT derived at delivery — held for its σ"
            );
            assert!(
                actor.awaiting_seed.is_some(),
                "the tip stays HELD on a store miss"
            );
        });
    }

    // `h` speculated as sibling A, then delivered finalized as a different
    // sibling B while the round is in the store — the eager derive takes the
    // re-derive path, records B, and never leaves the speculated A behind.
    #[test]
    fn eager_derive_reorgs_a_speculated_sibling() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let h = ANCHOR + 1;
            let store = crate::beacon::testing::SeedStore::new();
            let seed = real_seed(active_round(h));
            store.record(real_witness(seed.target_round));
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(store)
                .with_epocher(beacon_active_epocher());
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let cause = Span::current();

            let sib_a = OrderBlock {
                proposal_view: h,
                ..sample_order(Digest(B256::ZERO), h, B256::ZERO)
            };
            fx.marshal.canned.lock().unwrap().insert(h, sib_a.clone());
            actor
                .spec_execute(cause.clone(), sib_a.digest(), Some(seed.clone()))
                .await
                .unwrap();
            let hash_a = fx.chain.spec_executed_hash(h).unwrap();

            // Sibling B: distinct extra_data ⇒ distinct digest + sealed hash.
            let sib_b = OrderBlock {
                extra_data: Bytes::from_static(b"B"),
                proposal_view: h,
                ..sample_order(Digest(B256::ZERO), h, B256::ZERO)
            };
            fx.marshal.canned.lock().unwrap().insert(h, sib_b.clone());

            let (ack, _w) = Exact::handle();
            actor
                .on_finalized_block(cause.clone(), sib_b.clone(), ack)
                .await
                .unwrap();

            let expected_b = sealed_at(
                fx.anchor_hash,
                h,
                seed_folded_discriminator(sib_b.digest(), &Some(seed.clone())),
            )
            .hash();
            assert_eq!(
                fx.chain.spec_executed_hash(h),
                Some(expected_b),
                "eager re-derived to sibling B"
            );
            assert_ne!(expected_b, hash_a, "never left as the speculated sibling A");
            assert_eq!(
                fx.chain.finalized_executed_hash(h),
                Some(expected_b),
                "finalized tier records B, never the speculated A"
            );
            assert!(!fx.safety_halt.is_engaged());
        });
    }

    // Once `h` is eager-consumed, delivering its child `h+1` must not re-derive
    // `h` (the hold is gone) — `h+1` simply becomes the new held tip (its own
    // round is not in the store ⇒ a miss ⇒ hold).
    #[test]
    fn child_delivery_after_eager_does_not_double_derive() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let h = ANCHOR + 1;
            let store = crate::beacon::testing::SeedStore::new();
            let seed_h = real_seed(active_round(h));
            store.record(real_witness(seed_h.target_round));
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(store)
                .with_epocher(beacon_active_epocher());
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let cause = Span::current();

            let o_h = OrderBlock {
                proposal_view: h,
                ..sample_order(Digest(B256::ZERO), h, B256::ZERO)
            };
            let child = OrderBlock {
                proposal_view: h + 1,
                ..sample_order(o_h.digest(), h + 1, B256::ZERO)
            };
            {
                let mut c = fx.marshal.canned.lock().unwrap();
                c.insert(h, o_h.clone());
                c.insert(h + 1, child.clone());
            }

            let (ack_h, _wh) = Exact::handle();
            actor
                .on_finalized_block(cause.clone(), o_h.clone(), ack_h)
                .await
                .unwrap();
            assert!(actor.awaiting_seed.is_none(), "h eager-consumed");
            let hash_h = fx.chain.spec_executed_hash(h).unwrap();

            // Child h+1 delivered: held is empty (h consumed) ⇒ h+1 held; its own
            // round `Round(0, h+1)` is not in the store ⇒ a miss ⇒ hold.
            let (ack_c, _wc) = Exact::handle();
            actor
                .on_finalized_block(cause.clone(), child.clone(), ack_c)
                .await
                .unwrap();

            assert_eq!(
                fx.chain.spec_executed_hash(h),
                Some(hash_h),
                "the child delivery did NOT re-derive h"
            );
            assert!(
                fx.chain.spec_executed_hash(h + 1).is_none(),
                "h+1 is HELD (its round not in store)"
            );
            assert!(actor.awaiting_seed.is_some(), "h+1 is now the held tip");
            let payloads_h: Vec<u64> = fx
                .beacon
                .new_payload_calls
                .lock()
                .unwrap()
                .iter()
                .map(|p| p.number)
                .filter(|n| *n == h)
                .collect();
            assert_eq!(
                payloads_h,
                vec![h],
                "h derived exactly once — no double-derive"
            );
        });
    }

    // `h` is the last block of epoch e, so its child crosses into e+1 and
    // `witness_link`'s boundary adjustment (`ec − 1`) pins the wire field's round
    // epoch to e — exactly `epocher.containing(h).epoch()`. With the store
    // populated under `Round(e, view)` the eager derive hits with the correctly
    // computed epoch-e round and records `h` before the child exists.
    #[test]
    fn eager_derive_hits_at_the_epoch_boundary_with_the_parent_epoch_round() {
        use commonware_consensus::types::{Epocher as _, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            // origin 0, length 8: epoch 2 = heights 16..=23; h = 23 is its last
            // block (the child at 24 is the first block of epoch 3). Epoch 2 is
            // `DETERMINISTIC_BOOTSTRAP_EPOCH`, the first beacon-active one — a
            // boundary below it would derive `None` and never consult the store.
            let epocher = crate::epocher::OriginEpocher::new(
                0,
                std::num::NonZeroU64::new(8).expect("nonzero"),
            );
            const ANCHOR: u64 = 22;
            const H: u64 = 23;
            assert_eq!(
                epocher.containing(Height::new(H)).unwrap().last(),
                Height::new(H),
                "test premise: h is the last block of its epoch"
            );
            let e = epocher.containing(Height::new(H)).unwrap().epoch();
            assert_eq!(e, active_epoch());

            let store = crate::beacon::testing::SeedStore::new();
            let seed = real_seed(Round::new(e, View::new(H)));
            store.record(real_witness(seed.target_round));
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(store)
                .with_epocher(epocher);
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);

            let o_h = OrderBlock {
                proposal_view: H,
                ..sample_order(Digest(B256::ZERO), H, B256::ZERO)
            };
            fx.marshal.canned.lock().unwrap().insert(H, o_h.clone());
            let (ack, _w) = Exact::handle();
            actor
                .on_finalized_block(Span::current(), o_h, ack)
                .await
                .unwrap();

            let derived = fx
                .chain
                .spec_executed_hash(H)
                .expect("boundary h derived eagerly");
            assert_eq!(
                fx.chain.finalized_executed_hash(H),
                Some(derived),
                "epoch-e round HIT: recorded before the epoch-(e+1) child exists"
            );
            assert!(
                actor.awaiting_seed.is_none(),
                "hold consumed on the boundary hit"
            );
            assert!(!fx.safety_halt.is_engaged());
        });
    }

    // Negative twin: the same boundary height with the store populated only under
    // the next epoch's round `Round(e+1, view)` must miss — the eager round is a
    // pure function of h's own epoch, so a neighbouring epoch's entry (same view)
    // can never false-hit and yield a cross-epoch seed.
    #[test]
    fn eager_derive_never_false_hits_a_neighbouring_epoch_round() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let epocher = crate::epocher::OriginEpocher::new(
                0,
                std::num::NonZeroU64::new(8).expect("nonzero"),
            );
            const ANCHOR: u64 = 22;
            const H: u64 = 23; // last block of epoch 2, the bootstrap epoch
            let store = crate::beacon::testing::SeedStore::new();
            // Same view, wrong epoch (e+1 = 3): the only entry in the store.
            let wrong = real_seed(Round::new(Epoch::new(3), View::new(H)));
            store.record(real_witness(wrong.target_round));
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(store)
                .with_epocher(epocher);
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);

            let o_h = OrderBlock {
                proposal_view: H,
                ..sample_order(Digest(B256::ZERO), H, B256::ZERO)
            };
            fx.marshal.canned.lock().unwrap().insert(H, o_h.clone());
            let (ack, _w) = Exact::handle();
            actor
                .on_finalized_block(Span::current(), o_h, ack)
                .await
                .unwrap();

            assert!(
                fx.chain.spec_executed_hash(H).is_none(),
                "epoch-(e+1) entry did NOT false-hit: h stays underived"
            );
            assert!(actor.awaiting_seed.is_some(), "MISS: held for its σ");
        });
    }

    // `h` is finalized-delivered before its seed is recorded, so the on-delivery
    // eager derive misses and `h` is held. The notarization for `h`'s round then
    // records the seed into the shared index, and the executor's seed-notify
    // `select!` arm re-runs the eager derive: `h` is derived and
    // finalized-recorded with no further finalized delivery. This drives the
    // arm's body directly, not its wakeup.
    #[test]
    fn seed_notify_recovers_a_held_tip_after_a_late_seed_record() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let h = ANCHOR + 1;
            // Store starts empty: the delivery-time eager derive must miss.
            let store = crate::beacon::testing::SeedStore::new();
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(store.clone())
                .with_epocher(beacon_active_epocher());
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let cause = Span::current();

            let o1 = OrderBlock {
                proposal_view: h,
                ..sample_order(Digest(B256::ZERO), h, B256::ZERO)
            };
            fx.marshal.canned.lock().unwrap().insert(h, o1.clone());

            // Deliver h with the seed not yet recorded → eager miss → held.
            let (ack, _w) = Exact::handle();
            actor
                .on_finalized_block(cause.clone(), o1.clone(), ack)
                .await
                .unwrap();
            assert!(
                fx.chain.spec_executed_hash(h).is_none(),
                "delivery missed: h held, not derived"
            );
            assert!(actor.awaiting_seed.is_some(), "h is HELD after the miss");

            // The notarization for h's round arrives at the Reporter: it records
            // the seed (which fires the notify permit). The seed-notify arm then
            // re-runs the eager derive — model that by driving the arm's body.
            let seed = real_seed(active_round(h));
            store.record(real_witness(seed.target_round));
            actor
                .try_eager_finalized_derive(EagerTrigger::Notified)
                .await
                .unwrap();

            let derived = fx
                .chain
                .spec_executed_hash(h)
                .expect("the notify arm derived h WITHOUT a further finalized delivery");
            assert_eq!(
                fx.chain.finalized_executed_hash(h),
                Some(derived),
                "the notify arm ran the FINALIZED-tier derive: recorded_tip advanced to h"
            );
            assert!(
                actor.awaiting_seed.is_none(),
                "the notified eager derive CONSUMED the hold"
            );
            assert!(!fx.safety_halt.is_engaged());
        });
    }

    // A notify re-attempt must not spuriously derive when nothing is held (the
    // arm's `awaiting_seed.is_some()` guard is false), or when a tip is held but
    // the store still misses its round (the seed has not landed yet, so a later
    // notify will derive it). Neither path may advance the EL or touch the hold.
    #[test]
    fn seed_notify_is_a_noop_without_hold_or_seed() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let h = ANCHOR + 1;
            // Empty store: the opt-out from the fixture default, so both arms
            // below are reached with nothing recorded for h's round.
            let store = crate::beacon::testing::SeedStore::new();
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(store)
                .with_epocher(beacon_active_epocher());
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let cause = Span::current();

            // No hold: the arm's guard (`awaiting_seed.is_some()`) is false, so
            // the body is a no-op even if driven directly (the take() early
            // returns).
            assert!(actor.awaiting_seed.is_none(), "premise: nothing held");
            actor
                .try_eager_finalized_derive(EagerTrigger::Notified)
                .await
                .unwrap();
            assert!(
                fx.chain.spec_executed_hash(h).is_none(),
                "no-hold notify derived nothing"
            );
            assert!(
                actor.awaiting_seed.is_none(),
                "no-hold notify created no hold"
            );

            // Held tip but the store still misses its round → held-and-quiet.
            let o1 = OrderBlock {
                proposal_view: h,
                ..sample_order(Digest(B256::ZERO), h, B256::ZERO)
            };
            fx.marshal.canned.lock().unwrap().insert(h, o1.clone());
            let (ack, _w) = Exact::handle();
            actor
                .on_finalized_block(cause.clone(), o1.clone(), ack)
                .await
                .unwrap();
            assert!(actor.awaiting_seed.is_some(), "h held (store still empty)");

            actor
                .try_eager_finalized_derive(EagerTrigger::Notified)
                .await
                .unwrap();
            assert!(
                fx.chain.spec_executed_hash(h).is_none(),
                "store-still-missing notify did NOT derive h (silent no-op)"
            );
            assert!(
                actor.awaiting_seed.is_some(),
                "the hold is retained on a notify miss"
            );
        });
    }

    // A rollback invalidates the `spec_executed` suffix but keeps the parked
    // notarizations above the reorg point, and the post-rollback drain
    // re-executes them against the new canonical parent. Fork-A 102 is a live
    // speculation (the invalidated suffix); fork-B 102 is a parked gap
    // notarization. After 101 finalizes as fork-B's sibling, the drain speculates
    // fork-B 102 on the re-derived 101 — not the orphaned fork-A 101.
    #[test]
    fn rollback_keeps_parked_notarizations_that_redrain_on_the_new_parent() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let anchor = fx.anchor_hash;
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR); // spec_head = 100
            let cause = Span::current();

            // Fork A (speculated live) and fork B (finalized) siblings at 101/102.
            let o1a = sample_order(Digest(B256::ZERO), 101, B256::ZERO);
            let o1b = OrderBlock {
                extra_data: Bytes::from_static(b"B"),
                ..sample_order(Digest(B256::ZERO), 101, B256::ZERO)
            };
            let o2a = sample_order(o1a.digest(), 102, B256::ZERO);
            let o2b = sample_order(o1b.digest(), 102, B256::ZERO);
            {
                let mut c = fx.marshal.canned.lock().unwrap();
                c.insert(101, o1a.clone());
                c.insert(102, o2b.clone()); // fetched-by-digest for the parked drain
            }

            // Park fork-B 102 as a gap (102 > spec_head+1 while spec_head == 100).
            actor
                .spec_execute(cause.clone(), o2b.digest(), None)
                .await
                .unwrap();
            assert!(
                actor.parked_spec.contains_key(&102),
                "fork-B 102 parked as a gap"
            );

            // Build the fork-A live lead 101,102 (spec_execute does not drain, so
            // the parked entry survives).
            actor
                .spec_execute(cause.clone(), o1a.digest(), None)
                .await
                .unwrap();
            fx.marshal.canned.lock().unwrap().insert(102, o2a.clone());
            actor
                .spec_execute(cause.clone(), o2a.digest(), None)
                .await
                .unwrap();
            fx.marshal.canned.lock().unwrap().insert(102, o2b.clone()); // restore for the drain
            assert_eq!(actor.spec_head, 102, "fork-A lead reached 102");
            let hash_o2a = fx.chain.spec_executed_hash(102).unwrap();

            // Finalize 101 as fork-B's sibling (digest mismatch → rollback). This
            // invalidates the `spec_executed` suffix {102=o2a}, keeps parked{102=
            // o2b}, and its internal drain re-executes o2b on the re-derived 101.
            let hash_o1b = sealed_at(anchor, 101, o1b.digest().0).hash();
            let (ack, _w) = Exact::handle();
            actor
                .try_derive(cause.clone(), o1b.clone(), ack, None)
                .await
                .unwrap();

            assert_eq!(
                fx.chain.spec_executed_hash(101),
                Some(hash_o1b),
                "101 re-derived as fork B"
            );
            let expected_o2b = sealed_at(hash_o1b, 102, o2b.digest().0).hash();
            assert_eq!(
                fx.chain.spec_executed_hash(102),
                Some(expected_o2b),
                "parked fork-B 102 re-drained onto the NEW canonical 101 (not orphaned fork-A)"
            );
            assert_ne!(
                expected_o2b, hash_o2a,
                "the drain did not reuse the orphaned fork-A 102"
            );
            assert_eq!(
                actor.spec_head, 102,
                "speculation resumed to 102 via the drain"
            );
            assert!(
                actor.parked_spec.is_empty(),
                "the drained parked entry was removed"
            );
            assert!(!actor.safety_halt.is_engaged());
        });
    }

    // A speculative head advance never moves `safe`/`finalized`: `spec_execute`
    // calls `update_head` only. After finalizing anchor+1 (which sets safe =
    // h(anchor+1)), speculating +2 and +3 climbs head to h(anchor+3) while safe
    // stays at h(anchor+1) and finalized stays at the anchor — the load-bearing
    // `head > safe` speculative lead.
    #[test]
    fn safe_unchanged_across_speculative_head_advance() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // Finalize anchor+1 (sets safe = head = h(anchor+1); finalized
            // clamped at the anchor in the pre-K window). +2 is not finalized —
            // it is the speculative lead this test is about.
            let o1 = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let o2 = sample_order(o1.digest(), ANCHOR + 2, B256::ZERO);
            let (m1, w1) = finalize_msg(o1.clone());
            mailbox.send(m1).expect("send finalize 1");
            w1.await.expect("ack 1");

            let safe_after_finalize = fx.chain.spec_executed_hash(ANCHOR + 1).unwrap();
            {
                let fcus = fx.beacon.fcu_calls.lock().unwrap();
                let last = fcus.last().unwrap();
                assert_eq!(last.safe_block_hash, safe_after_finalize);
                assert_eq!(last.finalized_block_hash, fx.anchor_hash);
            }

            // Speculate +2 and +3 (notarized only) — each parent is canonical
            // from the prior FCU.
            let o3 = sample_order(o2.digest(), ANCHOR + 3, B256::ZERO);
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                canned.insert(ANCHOR + 2, o2.clone());
                canned.insert(ANCHOR + 3, o3.clone());
            }
            // Spec messages are processed FIFO; dropping the mailbox makes the
            // loop drain them then exit on `recv() == None`, so awaiting the
            // handle is the barrier that guarantees +2/+3 speculation has run.
            mailbox.send(spec_msg(&o2)).expect("spec 2");
            mailbox.send(spec_msg(&o3)).expect("spec 3");
            drop(mailbox);
            let _ = handle.await;

            {
                let fcus = fx.beacon.fcu_calls.lock().unwrap();
                let last = fcus.last().unwrap();
                assert_eq!(
                    last.head_block_hash,
                    fx.chain.spec_executed_hash(ANCHOR + 3).unwrap(),
                    "head climbed to the speculative tip +3"
                );
                assert_eq!(
                    last.safe_block_hash, safe_after_finalize,
                    "safe stayed at the ordering-final tip — spec never moves safe"
                );
                assert_eq!(
                    last.finalized_block_hash, fx.anchor_hash,
                    "finalized stayed clamped at the anchor"
                );
            }
        });
    }

    // A SYNCING status (both import and FCU) is the tolerated cold-start /
    // rejoin window — the block still derives and acks.
    #[test]
    fn syncing_status_is_tolerated_through_the_gate() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            *fx.beacon.fcu_status.lock().unwrap() = Some(PayloadStatusEnum::Syncing);
            *fx.beacon.import_status.lock().unwrap() = Some(PayloadStatusEnum::Syncing);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let (msg, waiter) = finalize_msg(order.clone());
            mailbox.send(msg).expect("send");
            waiter.await.expect("SYNCING is tolerated → block acks");

            assert_eq!(
                fx.beacon.new_payload_calls.lock().unwrap().len(),
                1,
                "block derived under SYNCING"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // Speculative path: the seed recovered from the notarization cert (the
    // `SpecNotarized` command) reaches the deriver during speculative
    // execution, and the same-round reconcile keeps the speculation (the
    // deriver runs exactly once).
    #[test]
    fn notarization_seed_reaches_deriver_on_speculation() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let seed = real_seed(active_round(ANCHOR + 1));
            record_fixture_seed(ANCHOR + 1);
            let fx = Fixture::new(ANCHOR).with_epocher(beacon_active_epocher());
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // `proposal_view` matches the notarization round (the honest steady
            // state), so the re-canonicalisation is a no-op.
            let order = OrderBlock {
                proposal_view: ANCHOR + 1,
                ..sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO)
            };
            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(ANCHOR + 1, order.clone());

            // Speculative command carrying the notarization seed.
            mailbox
                .send(spec_msg_seeded(&order, seed.clone()))
                .expect("send spec");
            // Finalize the same order: the store answers the same round, so the
            // reconcile keeps the speculation (no re-derive).
            let (m, w) = finalize_msg(order.clone());
            mailbox.send(m).expect("send finalize");
            w.await.expect("ack");

            {
                let seen = fx.deriver.seeds_seen.lock().unwrap();
                assert_eq!(
                    seen.as_slice(),
                    &[(ANCHOR + 1, Some(seed))],
                    "the notarization seed reached the deriver during speculation"
                );
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    use crate::cold_start_jump::JUMP_THRESHOLD;

    /// An `Update::Tip` command at `height` (the marshal-frontier event the
    /// steady-state re-jump reacts to). The round view is a stand-in.
    fn tip_msg(height: u64) -> Message {
        use commonware_consensus::types::{Epoch, Round, View};
        Message {
            cause: Span::current(),
            command: Command::Finalize(Box::new(Update::Tip(
                Round::new(Epoch::new(0), View::new(height)),
                Height::new(height),
                Digest(B256::repeat_byte(0xDD)),
            ))),
        }
    }

    /// The catch-up ack barrier: deliver `order` and await its ack. The re-jump
    /// tests park the frontier far ahead, so guard #2 is armed (`tip >= h + K`);
    /// a canned result-consistent attested block at `h + K` lets the guard
    /// converge instead of parking.
    async fn finalize_and_ack_behind(fx: &Fixture, mailbox: &Mailbox, order: OrderBlock) {
        let parent = fx
            .chain
            .spec_executed_hash(order.height - 1)
            .expect("parent must be canonical for the barrier finalize");
        let expected = sealed_at(parent, order.height, order.digest().0).hash();
        fx.marshal.canned.lock().unwrap().insert(
            order.height + K,
            sample_order(Digest(B256::ZERO), order.height + K, expected),
        );
        let (msg, waiter) = finalize_msg(order.clone());
        mailbox.send(msg).expect("send barrier finalize");
        waiter.await.expect("barrier finalize acks");
    }

    /// Clonable script for a re-jump callback's terminal outcome. `JumpOutcome`
    /// itself is not `Clone` (its `eyre::Report` variants), so the test scripts a
    /// clonable descriptor and the `Fn` (which may be called more than once)
    /// rebuilds a fresh `JumpOutcome` per call.
    #[derive(Clone)]
    enum Scripted {
        Landed {
            landing: u64,
            hash: B256,
            floor: u64,
        },
        Lagging,
        Stalled(String),
        StalledWithPeers(String),
        InvalidTarget(String),
        L1Fork(String),
    }

    impl Scripted {
        fn build(&self) -> crate::cold_start_jump::JumpOutcome {
            use crate::cold_start_jump::JumpOutcome;
            match self {
                Scripted::Landed {
                    landing,
                    hash,
                    floor,
                } => JumpOutcome::Landed {
                    landing: *landing,
                    hash: *hash,
                    floor: *floor,
                },
                Scripted::Lagging => JumpOutcome::Lagging,
                Scripted::Stalled(s) => JumpOutcome::Stalled(eyre::eyre!(s.clone())),
                Scripted::StalledWithPeers(s) => {
                    JumpOutcome::StalledWithPeers(eyre::eyre!(s.clone()))
                }
                Scripted::InvalidTarget(s) => JumpOutcome::InvalidTarget(eyre::eyre!(s.clone())),
                Scripted::L1Fork(s) => JumpOutcome::L1Fork(eyre::eyre!(s.clone())),
            }
        }
    }

    /// The `from` heights recorded by a recording re-jump callback.
    type RejumpCalls = Arc<Mutex<Vec<u64>>>;
    fn recording_re_jump(scripted: Scripted) -> (ReJump, RejumpCalls) {
        let (cb, calls, _targets) = recording_re_jump_with_targets(scripted);
        (cb, calls)
    }

    /// As [`recording_re_jump`], also recording the target height the executor read
    /// from its marshal archive at the tip it triggered on.
    fn recording_re_jump_with_targets(
        scripted: Scripted,
    ) -> (ReJump, RejumpCalls, Arc<Mutex<Vec<u64>>>) {
        let calls: RejumpCalls = Arc::new(Mutex::new(Vec::new()));
        let targets: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
        let calls_cl = calls.clone();
        let targets_cl = targets.clone();
        let call: ReJumpFn = Arc::new(move |from, target| {
            calls_cl.lock().unwrap().push(from);
            targets_cl.lock().unwrap().push(target.block.height);
            let scripted = scripted.clone();
            Box::pin(async move { scripted.build() })
        });
        (
            ReJump {
                call,
                threshold: JUMP_THRESHOLD,
                rotate: None,
                probe: None,
                tracked_epoch: None,
            },
            calls,
            targets,
        )
    }

    /// As [`recording_re_jump`], with a `rotate` escape that counts its invocations.
    /// `scripts` is saturating: call N returns `scripts[min(N, len−1)]`, so one element
    /// is a single outcome and a longer vec scripts a per-call sequence.
    fn recording_re_jump_with_rotate(
        scripts: Vec<Scripted>,
    ) -> (ReJump, RejumpCalls, Arc<std::sync::atomic::AtomicU32>) {
        assert!(!scripts.is_empty(), "need at least one scripted outcome");
        let calls: RejumpCalls = Arc::new(Mutex::new(Vec::new()));
        let calls_cl = calls.clone();
        let scripts = Arc::new(scripts);
        let idx = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let call: ReJumpFn = Arc::new(move |from, _target| {
            calls_cl.lock().unwrap().push(from);
            let i = idx
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                .min(scripts.len() - 1);
            let scripted = scripts[i].clone();
            Box::pin(async move { scripted.build() })
        });
        let rotations = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let rotate: crate::cert_inlet::RotateUpstream = {
            let rotations = rotations.clone();
            Arc::new(move || {
                let rotations = rotations.clone();
                Box::pin(async move {
                    rotations.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }) as BoxFuture<'static, ()>
            })
        };
        (
            ReJump {
                call,
                threshold: JUMP_THRESHOLD,
                rotate: Some(rotate),
                probe: None,
                tracked_epoch: None,
            },
            calls,
            rotations,
        )
    }

    // A gap past JUMP_THRESHOLD spawns the read-only waiter, and its completion arm
    // re-seeds the anchor to the landing and advances the marshal floor to `landing − K`.
    #[test]
    fn re_jump_fires_and_reseeds_anchor_and_marshal_floor() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let landing_h = ANCHOR + JUMP_THRESHOLD + 5_000;
            let landing_hash = B256::repeat_byte(0xE1);
            let floor = landing_h - K;
            let (cb, calls) = recording_re_jump(Scripted::Landed {
                landing: landing_h,
                hash: landing_hash,
                floor,
            });
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                .expect("send tip");
            // Let the spawned waiter's completion arm re-seed before the barrier below.
            ctx.sleep(Duration::from_millis(10)).await;

            // Barrier finalize at landing + 1: its parent must be canonical, so it
            // only acks after the completion arm re-seeded the landing.
            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(landing_h, landing_hash);
            finalize_and_ack_behind(
                &fx,
                &mailbox,
                sample_order(Digest(B256::ZERO), landing_h + 1, B256::ZERO),
            )
            .await;

            assert_eq!(
                *calls.lock().unwrap(),
                vec![ANCHOR],
                "re-jump invoked once with from = ordering_finalized (the cold-start anchor)"
            );
            assert_eq!(
                *fx.marshal.floors.lock().unwrap(),
                vec![floor],
                "running marshal floor advanced to landing − K (completion arm ran)"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A jump that is in flight gates the drain arm, so `Update::Block` deliveries
    // queue as stale below-landing entries; `reseed_forward` must prune and ack them
    // Ok (canonical post-backfill, never Canceled) before reopening the drain.
    #[test]
    fn reseed_prunes_stale_queued_finalizations_no_missing_artifact_fatal() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let landing_h = ANCHOR + JUMP_THRESHOLD + 5_000;
            let landing_hash = B256::repeat_byte(0xE1);
            let floor = landing_h - K;
            // A jump held in flight so stale deliveries accumulate while the drain is gated.
            let gate = Arc::new(tokio::sync::Notify::new());
            let gate_cl = gate.clone();
            let call: ReJumpFn = Arc::new(move |_from, _target| {
                let gate = gate_cl.clone();
                Box::pin(async move {
                    gate.notified().await;
                    crate::cold_start_jump::JumpOutcome::Landed {
                        landing: landing_h,
                        hash: landing_hash,
                        floor,
                    }
                })
            });
            let cb = ReJump {
                call,
                threshold: JUMP_THRESHOLD,
                rotate: None,
                probe: None,
                tracked_epoch: None,
            };
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            mailbox.send(tip_msg(landing_h + 10)).expect("send tip");
            ctx.sleep(Duration::from_millis(5)).await;

            let o1 = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let o2 = sample_order(o1.digest(), ANCHOR + 2, B256::ZERO);
            let (m1, w1) = finalize_msg(o1);
            let (m2, w2) = finalize_msg(o2);
            mailbox.send(m1).expect("queue stale 1");
            mailbox.send(m2).expect("queue stale 2");
            ctx.sleep(Duration::from_millis(5)).await;

            gate.notify_one();
            w1.await
                .expect("stale delivery 1 acked Ok by the prune (not Canceled)");
            w2.await
                .expect("stale delivery 2 acked Ok by the prune (not Canceled)");
            assert_eq!(
                *fx.marshal.floors.lock().unwrap(),
                vec![floor],
                "reseed completed (floor advanced)"
            );

            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(landing_h, landing_hash);
            finalize_and_ack_behind(
                &fx,
                &mailbox,
                sample_order(Digest(B256::ZERO), landing_h + 1, B256::ZERO),
            )
            .await;

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // The prune keys on the landing: a queued entry above it survives with its ack
    // untouched, while entries at or below it are pruned and acked Ok.
    #[test]
    fn reseed_keeps_queued_finalizations_above_landing() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let landing_h = ANCHOR + JUMP_THRESHOLD + 5_000;
            let fx = Fixture::new(ANCHOR);
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);

            let below = sample_order(Digest(B256::ZERO), landing_h - 1, B256::ZERO);
            let above = sample_order(Digest(B256::ZERO), landing_h + 2, B256::ZERO);
            let (ack_below, w_below) = Exact::handle();
            let (ack_above, mut w_above) = Exact::handle();
            actor
                .pending_finalizations
                .push_back(ready((Span::current(), below, ack_below)));
            actor
                .pending_finalizations
                .push_back(ready((Span::current(), above, ack_above)));

            actor
                .reseed_forward(landing_h, B256::repeat_byte(0xE1), landing_h - K)
                .await
                .expect("reseed_forward");

            w_below
                .await
                .expect("below-landing entry pruned + acked Ok");
            assert_eq!(
                actor.pending_finalizations.len(),
                1,
                "the above-landing entry survived the prune"
            );
            assert!(
                (&mut w_above).now_or_never().is_none(),
                "the kept entry's ack is untouched (neither Ok nor Canceled)"
            );
        });
    }

    // `reseed_forward` acks its disposals before `set_floor`, and `set_floor` is
    // fire-and-forget: the marshal can dispatch freed old-range blocks into the
    // executor mailbox before it processes the floor, so `Update::Block` must ack a
    // below-floor block without deriving instead of parking on a pruned `h + K`.
    #[test]
    fn stale_dispatch_below_floor_acked_without_derive() {
        use metrics_util::debugging::{DebugValue, DebuggingRecorder};
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            let runtime = deterministic::Runner::default();
            runtime.start(|ctx| async move {
                const ANCHOR: u64 = 100;
                let landing_h = ANCHOR + JUMP_THRESHOLD + 5_000;
                let landing_hash = B256::repeat_byte(0xE1);
                let floor = landing_h - K;
                // Gate the jump so the escape below is deterministic.
                let gate = Arc::new(tokio::sync::Notify::new());
                let gate_cl = gate.clone();
                let call: ReJumpFn = Arc::new(move |_from, _target| {
                    let gate = gate_cl.clone();
                    Box::pin(async move {
                        gate.notified().await;
                        crate::cold_start_jump::JumpOutcome::Landed {
                            landing: landing_h,
                            hash: landing_hash,
                            floor,
                        }
                    })
                });
                let cb = ReJump {
                    call,
                    threshold: JUMP_THRESHOLD,
                    rotate: None,
                    probe: None,
                    tracked_epoch: None,
                };
                let fx = Fixture::new(ANCHOR).with_re_jump(cb);
                let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
                fx.marshal.arm_stale_escape(
                    mailbox.clone(),
                    vec![
                        sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO),
                        sample_order(Digest(B256::ZERO), ANCHOR + 2, B256::ZERO),
                    ],
                );
                let mut handle = actor.start();

                mailbox.send(tip_msg(landing_h + 10)).expect("send tip");
                ctx.sleep(Duration::from_millis(5)).await;

                gate.notify_one();
                wait_until(&ctx, "reseed floor advanced", || {
                    !fx.marshal.floors.lock().unwrap().is_empty()
                })
                .await;
                ctx.sleep(Duration::from_millis(20)).await;

                assert!(
                    fx.beacon
                        .new_payload_calls
                        .lock()
                        .unwrap()
                        .iter()
                        .all(|p| p.number > floor),
                    "a below-floor escaped block must be acked-without-derive (no import)"
                );
                assert!(
                    fx.marshal.hints.lock().unwrap().iter().all(|&h| h > floor),
                    "no NeedAttestation park hint on a pruned h + K"
                );
                assert!(
                    !fx.safety_halt.is_engaged(),
                    "acking a stale escaped block is not a fork — no halt"
                );
                assert!(
                    (&mut handle).now_or_never().is_none(),
                    "the executor stays up (no missing-artifact fatal)"
                );

                // The escape model holds a mailbox clone; release it so the channel closes.
                *fx.marshal.dispatch.lock().unwrap() = None;
                drop(mailbox);
                let _ = handle.await;
            });
        });
        let stale = snap
            .snapshot()
            .into_vec()
            .into_iter()
            .filter(|(k, ..)| k.key().name() == "dpos_executor_stale_dispatch_dropped_total")
            .map(|(.., v)| match v {
                DebugValue::Counter(c) => c,
                _ => 0,
            })
            .sum::<u64>();
        assert!(
            stale >= 1,
            "the escaped below-floor block was acked-without-derive and counted"
        );
    }

    // The trigger reads the marshal tip alone, so a probe's `Latest` answer far past
    // the serving window buys one by-height hint and no re-jump; the next frozen tip
    // still measures the gap off the tip.
    #[test]
    fn a_frozen_tip_spawns_no_re_jump_however_far_ahead_the_probe_claims_the_chain_is() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let (mut cb, calls, _targets) = recording_re_jump_with_targets(Scripted::Lagging);
            let far = ANCHOR + JUMP_THRESHOLD + 5_010;
            let (probe_rj, ticks) = scripted_probe(vec![ProbeOutcome {
                frontier: Some(Height::new(far)),
                step: None,
            }]);
            cb.probe = probe_rj.probe;
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (mut actor, _mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);

            actor
                .maybe_re_jump(Height::new(ANCHOR + 5))
                .await
                .expect("no fault");
            ctx.sleep(Duration::from_millis(10)).await;
            // The probe's answer is believed only as one hint_finalization, which the
            // marshal verifies itself.
            actor.probe_frontier().await;
            assert_eq!(
                ticks.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "the probe body never ran — the loud source never spoke"
            );
            assert_eq!(
                fx.marshal.hints.lock().unwrap().clone(),
                vec![far],
                "the probe's `Latest` bought one by-height hint and nothing else"
            );
            actor
                .maybe_re_jump(Height::new(ANCHOR + 5))
                .await
                .expect("no fault");
            ctx.sleep(Duration::from_millis(10)).await;

            assert!(
                calls.lock().unwrap().is_empty(),
                "a re-jump was spawned on a height no one authenticated: {:?}",
                calls.lock().unwrap()
            );
        });
    }

    // The re-jump target is the archive pair at the tip the trigger fired on, not a
    // height a peer answered.
    #[test]
    fn the_re_jump_target_is_the_pair_at_the_tip_the_trigger_fired_on() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let tip = ANCHOR + JUMP_THRESHOLD + 5_010;
            let (cb, calls, targets) = recording_re_jump_with_targets(Scripted::Lagging);
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (mut actor, _mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);

            actor
                .maybe_re_jump(Height::new(tip))
                .await
                .expect("no fault");
            ctx.sleep(Duration::from_millis(10)).await;
            assert_eq!(
                *calls.lock().unwrap(),
                vec![ANCHOR],
                "the deep gap did not spawn the waiter"
            );
            assert_eq!(
                *targets.lock().unwrap(),
                vec![tip],
                "the jump target was not the archive pair at the triggering tip"
            );

            // With no pair at the tip the trigger declines rather than guesses.
            *fx.marshal.archive_empty.lock().unwrap() = true;
            actor
                .maybe_re_jump(Height::new(tip + 1))
                .await
                .expect("no fault");
            ctx.sleep(Duration::from_millis(10)).await;
            assert_eq!(
                *calls.lock().unwrap(),
                vec![ANCHOR],
                "the trigger spawned a jump with no archive pair behind the tip"
            );
        });
    }

    /// A `ReJump` whose probe replays canned outcomes, one per tick (saturating on
    /// the last), and counts the ticks that reached the probe body. Only
    /// `Actor::probe_frontier`'s handling of a named step is under test.
    fn scripted_probe(
        outcomes: Vec<ProbeOutcome>,
    ) -> (ReJump, Arc<std::sync::atomic::AtomicUsize>) {
        assert!(!outcomes.is_empty(), "need at least one scripted outcome");
        let ticks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let outcomes = Arc::new(outcomes);
        let probe: FrontierProbeFn = {
            let ticks = ticks.clone();
            let outcomes = outcomes.clone();
            Arc::new(move |_tracked| {
                let i = ticks
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    .min(outcomes.len() - 1);
                let outcome = outcomes[i].clone();
                Box::pin(async move { outcome })
            })
        };
        (
            ReJump {
                call: Arc::new(|_, _| Box::pin(async { Scripted::Lagging.build() })),
                threshold: JUMP_THRESHOLD,
                rotate: None,
                probe: Some(probe),
                tracked_epoch: Some(Arc::new(|| Some(3))),
            },
            ticks,
        )
    }

    fn step_at(height: u64) -> Option<(Height, NonEmptyVec<PeerPubkey>)> {
        Some((Height::new(height), dummy_peers().expect("one peer")))
    }

    /// A `CertUpstream` that answers nothing, so the probe's `Latest` is `None` and
    /// the only observable is the ladder step — which must be takeable without it.
    #[derive(Clone)]
    struct SilentUpstream;

    impl crate::cert_follow::CertUpstream for SilentUpstream {
        async fn get_finalization(
            &self,
            _height: Height,
        ) -> Option<crate::cert_follow::UpstreamFinalized> {
            None
        }
        async fn get_latest(&self) -> Option<crate::cert_follow::UpstreamFinalized> {
            None
        }
        async fn rotate(&self) {}
    }

    /// A committee module with a frozen geometry and a readable record at every
    /// epoch — the two things `dpos::frontier_probe` reads to name a rung.
    fn committee_with_participants(
        activation: u64,
        interval: u64,
    ) -> Arc<crate::committee::testing::SchemeCommittee> {
        use commonware_codec::DecodeExt as _;
        use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
        use commonware_math::algebra::Random as _;
        use commonware_utils::TryFromIterator as _;
        use fluentbase_bls::{keys::ValidatorBlsKeypair, BlsPubkey};
        use fluentbase_staking_reader::reader::{
            ConsensusKeys, ValidatorSetSnapshot, ValidatorWithKeys,
        };
        use rand_08::rngs::StdRng;
        use rand_core::SeedableRng as _;

        const N: usize = 4;
        let mut rng = StdRng::seed_from_u64(0xB101);
        let peers: Vec<PeerPubkey> = (0..N)
            .map(|_| Ed25519PrivateKey::random(&mut rng).public_key())
            .collect();
        let bls: Vec<BlsPubkey> = (0..N)
            .map(|_| {
                let kp = ValidatorBlsKeypair::generate(&mut rng);
                BlsPubkey::decode(kp.public_bytes().as_slice()).expect("bls pubkey")
            })
            .collect();
        crate::committee::testing::SchemeCommittee::with_geometry(
            |_| None,
            move |epoch| {
                let snap = ValidatorSetSnapshot {
                    block_hash: B256::ZERO,
                    block_number: 0,
                    epoch,
                    validators: peers
                        .iter()
                        .zip(bls.iter())
                        .map(|(peer, bls)| ValidatorWithKeys {
                            address: alloy_primitives::Address::ZERO,
                            keys: ConsensusKeys {
                                peer_pubkey: peer.clone(),
                                bls_pubkey: *bls,
                                activation_epoch: 0,
                            },
                            tombstoned: false,
                        })
                        .collect(),
                    weights: Some(vec![1u128; N]),
                };
                Some(crate::committee::CommitteeRecord {
                    epoch,
                    members: peers
                        .iter()
                        .zip(bls.iter())
                        .map(|(peer, bls)| crate::committee::Member {
                            address: alloy_primitives::Address::ZERO,
                            peer: peer.clone(),
                            bls: *bls,
                        })
                        .collect(),
                    weights: vec![1u128; N],
                    changed: false,
                    snapshot: (0, B256::ZERO),
                    participants: commonware_utils::ordered::Set::try_from_iter(
                        peers.iter().cloned(),
                    )
                    .expect("non-empty"),
                    bls: crate::scheme::epoch_committee_from_snapshot(&snap).expect("committee"),
                })
            },
            crate::committee::Geometry::new(activation, interval),
        )
    }

    // A follower-shaped re-jump wires the production probe and `local_tracked_epoch`,
    // as `launch_follower` does, over an executor whose tip is frozen: it must name
    // and put its own ladder rung `last(T+1)` on the marshal.
    #[test]
    fn a_follower_shaped_re_jump_puts_its_own_ladder_step_on_the_marshal() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const INTERVAL: u64 = 32;
            // `last(2)` under `(activation 0, interval 32)`: the terminal an
            // execution-stalled node parks on.
            const PARKED: u64 = 95;
            let committee: Arc<dyn crate::committee::Committee> =
                committee_with_participants(0, INTERVAL);
            let cursor = crate::FinalizedCursor::default();
            cursor.advance(PARKED);

            let re_jump = ReJump {
                call: Arc::new(|_, _| Box::pin(async { Scripted::Lagging.build() })),
                threshold: JUMP_THRESHOLD,
                rotate: None,
                probe: Some(crate::dpos::frontier_probe(
                    SilentUpstream,
                    committee.clone(),
                )),
                tracked_epoch: Some(crate::dpos::local_tracked_epoch(
                    committee.clone(),
                    cursor.clone(),
                )),
            };
            let fx = Fixture::new(PARKED).with_re_jump(re_jump);
            // `last_consensus` seeds both `last_tip_height` and `probe_prev_tip`, so
            // the tip is frozen from the first tick.
            let (mut actor, _mailbox) = fx.build(ctx, PARKED, PARKED);

            actor.probe_frontier().await;

            let hints = fx.marshal.hints.lock().unwrap().clone();
            assert_eq!(
                hints,
                vec![159],
                "a frozen-tip follower named no rung (or the wrong one): `T` is \
                 `epoch_of(95) + 1 = 3` and the rung is `last(4) = 159`"
            );
            assert!(
                hints[0] > PARKED,
                "the rung is at or below the frozen tip — the marshal would discard it \
                 and nothing would move"
            );
        });
    }

    // The ladder is the repetition of the tick: a step the marshal can act on is put
    // again on the next frozen tick, not once, which makes an unserved step harmless
    // (the node keeps walking contiguously and asks again).
    #[test]
    fn a_ladder_step_is_put_on_the_marshal_again_on_every_frozen_tick() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const STEP: u64 = 191;
            let (re_jump, ticks) = scripted_probe(vec![ProbeOutcome {
                // A `Latest` answer above the step, at a distinct height so the
                // step's hint can be told apart from the untargeted frontier hint the
                // same tick also puts.
                frontier: Some(Height::new(STEP + 9)),
                step: step_at(STEP),
            }]);
            let fx = Fixture::new(ANCHOR).with_re_jump(re_jump);
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);

            actor.probe_frontier().await;
            actor.probe_frontier().await;

            assert_eq!(
                ticks.load(std::sync::atomic::Ordering::Relaxed),
                2,
                "the probe body did not run on both frozen ticks"
            );
            let hints = fx.marshal.hints.lock().unwrap().clone();
            assert_eq!(
                hints.iter().filter(|h| **h == STEP).count(),
                2,
                "the ladder step was not re-put on the second frozen tick — the ladder is the \
                 repetition of this tick and nothing else: {hints:?}"
            );
        });
    }

    // The step is judged against the marshal floor, not the tip: the marshal drops a
    // hint at or below the floor, while a step between the floor and the tip lands in
    // the hole a jumped node carries and is exactly the one it must fetch.
    #[test]
    fn a_ladder_step_is_skipped_at_the_marshal_floor_and_put_inside_the_hole() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 200;
            const FLOOR: u64 = 150;
            // Tip 200, floor 150: heights 151..=200 are the hole the jump left, and
            // the `Latest` witness is above both.
            let (re_jump, _ticks) = scripted_probe(vec![
                ProbeOutcome {
                    frontier: Some(Height::new(ANCHOR)),
                    step: step_at(FLOOR),
                },
                ProbeOutcome {
                    frontier: Some(Height::new(ANCHOR)),
                    step: step_at(FLOOR + 10),
                },
            ]);
            let fx = Fixture::new(ANCHOR)
                .with_marshal_floor(FLOOR)
                .with_re_jump(re_jump);
            // `last_consensus` seeds both `last_tip_height` and `probe_prev_tip`, so
            // the tip is frozen from the first tick.
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);

            actor.probe_frontier().await;
            assert!(
                fx.marshal.hints.lock().unwrap().is_empty(),
                "a step AT the marshal floor was put — the marshal discards it: {:?}",
                fx.marshal.hints.lock().unwrap()
            );

            actor.probe_frontier().await;
            assert!(
                fx.marshal.hints.lock().unwrap().contains(&(FLOOR + 10)),
                "a step inside the floor..tip HOLE was suppressed — that is the one range a \
                 jumped node needs and the marshal would accept: {:?}",
                fx.marshal.hints.lock().unwrap()
            );
        });
    }

    // `reseed_forward` raises the executed cursor to the landing, not the floor: the
    // landing is executed post-backfill and the `K` below it are governed by the
    // two-tier result lag.
    #[test]
    fn reseed_forward_off_by_k_raises_cursor_to_landing() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let landing = ANCHOR + JUMP_THRESHOLD + 5_000;
            let landing_hash = B256::repeat_byte(0xE1);
            let floor = landing - K;
            let fx = Fixture::new(ANCHOR);
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);

            actor
                .reseed_forward(landing, landing_hash, floor)
                .await
                .expect("reseed_forward");

            let (ordering_finalized, anchor_finalized, safe_height, finalized_height, spec_head) =
                actor.seed_fields();
            assert_eq!(
                ordering_finalized, landing,
                "off-by-K: cursor raised to the LANDING, not the floor ({floor})"
            );
            assert_eq!(anchor_finalized, (Height::new(landing), landing_hash));
            // The in-memory `finalized_height` is raised to the landing (the FCU
            // re-pins the engine tag to the floor); `safe` rides the landing too.
            assert_eq!(finalized_height, Height::new(landing));
            assert_eq!(
                safe_height,
                Height::new(landing),
                "safe raised to the landing"
            );
            assert_eq!(
                spec_head, landing,
                "stale-spec: spec_head raised to the landing"
            );

            // Advancing to the landing (not a landing-only entry) is what lets the
            // cursor's provider cover the below-landing gate sampled by the first
            // post-jump proposals.
            let below = B256::repeat_byte(0xE0);
            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(landing - 1, below);
            assert_eq!(
                fx.chain.finalized_executed_hash(landing - 1),
                Some(below),
                "below-landing gate resolves via the advanced cursor → provider"
            );
            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(landing + 1, B256::repeat_byte(0xE2));
            assert_eq!(
                fx.chain.finalized_executed_hash(landing + 1),
                None,
                "above the landing stays None until the finalized reconcile records it"
            );
        });
    }

    // The startup drain's `None` arm is fatal: the drain walks up to the marshal
    // floor, which never heals below, and `maybe_re_jump` refuses to spawn while the
    // drain is non-empty. Only this node's own two stores disagree, so it is not a
    // SafetyHalt.
    #[test]
    fn a_startup_drain_over_a_hole_the_marshal_cannot_repair_dies_loudly() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100; // reth's last block = the drain's lower bound
            const CURSOR: u64 = ANCHOR + 2; // the marshal's acked cursor = its floor
            let fx = Fixture::new(ANCHOR);
            // `FakeMarshal::canned` is empty, so the first drained height comes back
            // `None`.
            let (actor, _mailbox) = fx.build(ctx.clone(), ANCHOR, CURSOR);
            let handle = actor.start();

            let exited = futures::future::select(
                Box::pin(handle),
                Box::pin(ctx.sleep(Duration::from_secs(30))),
            )
            .await;
            assert!(
                matches!(exited, futures::future::Either::Left(_)),
                "the startup drain hit a height the marshal cannot serve and the executor \
                 kept running — a below-floor hole never heals, so this is a permanent \
                 silent stall, not a wait"
            );
            assert!(
                !fx.safety_halt.is_engaged(),
                "a hole between this node's OWN two stores is Corruption, not a fork-safety \
                 halt"
            );
        });
    }

    // A backfill iterator pending at a low height when a jump lands far above it:
    // `reseed_forward` must fast-forward it to `landing + 1`, else the post-jump
    // drain re-derives the whole jumped range. The iterator is the only source of
    // backfill heights for the deriver, so yielding nothing ≤ landing is the guarantee.
    #[test]
    fn reseed_forward_fast_forwards_backfill_past_landing() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const NEXT: u64 = 377; // pending pre-jump backfill height (init: anchor+1)
            const ANCHOR: u64 = NEXT - 1;
            let landing = ANCHOR + JUMP_THRESHOLD + 4_000;
            let landing_hash = B256::repeat_byte(0xE1);
            let floor = landing - K;
            // End above the landing so the assertion also pins that the original upper
            // bound is preserved (only the ≤ landing prefix is skipped).
            let end = landing + 50;

            let fx = Fixture::new(ANCHOR);
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, end);
            assert_eq!(
                *actor.finalized_heights_to_backfill.start(),
                NEXT,
                "backfill iterator pending at the pre-jump low height"
            );
            let len_before = actor.finalized_heights_to_backfill.clone().count();

            actor
                .reseed_forward(landing, landing_hash, floor)
                .await
                .expect("reseed_forward");

            let remaining: Vec<u64> = actor.finalized_heights_to_backfill.clone().collect();
            assert_eq!(
                remaining.first().copied(),
                Some(landing + 1),
                "next drained backfill height is landing+1"
            );
            assert_eq!(
                remaining,
                ((landing + 1)..=end).collect::<Vec<_>>(),
                "original upper bound preserved; only the ≤ landing prefix skipped"
            );
            assert!(
                remaining.iter().all(|&h| h > landing),
                "no backfill height ≤ landing survives (deriver never re-derives the jumped range)"
            );
            // The shrink equals the skipped [NEXT ..= landing] prefix the metric counts.
            assert_eq!(
                len_before - remaining.len(),
                (landing - NEXT + 1) as usize,
                "skipped count spans [NEXT ..= landing]"
            );
        });
    }

    // When the landing is at or below the iterator's next-to-yield height there is
    // nothing ≤ landing to skip, so the iterator is untouched.
    #[test]
    fn reseed_forward_backfill_noop_when_landing_below_next() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const NEXT: u64 = 501;
            const ANCHOR: u64 = NEXT - 1;
            let end = NEXT + 20;
            let landing = NEXT - 1; // strictly below the next-to-yield height
            let landing_hash = B256::repeat_byte(0xE3);
            let floor = landing.saturating_sub(K);

            let fx = Fixture::new(ANCHOR);
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, end);
            let before: Vec<u64> = actor.finalized_heights_to_backfill.clone().collect();

            actor
                .reseed_forward(landing, landing_hash, floor)
                .await
                .expect("reseed_forward");

            let after: Vec<u64> = actor.finalized_heights_to_backfill.clone().collect();
            assert_eq!(
                before, after,
                "landing below the iterator's next-to-yield leaves the backfill range untouched"
            );
            assert_eq!(after.first().copied(), Some(NEXT), "still pending at NEXT");
        });
    }

    // A fresh process starts with the finalized-execution cursor at 0 but a provider
    // populated to the marshal-acked cursor T; `init` must seed the cursor at T so the
    // result gate serves provider hashes for h ≤ T — the first K post-restart
    // proposals sample T+1−K..T — instead of None.
    #[test]
    fn init_seeds_result_gate_floor_at_marshal_acked() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const T: u64 = 100; // the pre-restart marshal-acked cursor
            let fx = Fixture::new(T);
            let persisted_below = B256::repeat_byte(0x99);
            // Persisted pre-restart canonical content: T−1 (acked, final) and a
            // speculative tail block above the acked point.
            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(T - 1, persisted_below);
            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(T + 1, B256::repeat_byte(0xAA));

            // `build` wires the acked cursor and the reth head both to T: the common
            // case where they coincide.
            let (_actor, _mailbox) = fx.build(ctx, T, T);

            assert_eq!(
                fx.chain.finalized_executed_hash(T),
                Some(fx.anchor_hash),
                "init cursor at T: the acked tip resolves via the provider"
            );
            assert_eq!(
                fx.chain.finalized_executed_hash(T - 1),
                Some(persisted_below),
                "heights below the acked point resolve via the provider"
            );
            // The persisted speculative tail above the acked point must not be served
            // as finalized: the startup reconcile can still reorg it.
            assert_eq!(
                fx.chain.finalized_executed_hash(T + 1),
                None,
                "above the floor stays None until the finalized reconcile records it"
            );
        });
    }

    // A clean shutdown persists reth's head, which under deferred execution carries a
    // speculative tail above the marshal-acked cursor; heights in `(acked, head]` are
    // notarized-only and a sibling can still finalize. The floor must seed from the
    // acked cursor, not the head, or a restart straddling a nullify race serves the
    // orphaned sibling as finalized.
    #[test]
    fn init_floor_excludes_speculative_tail_above_acked() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ACKED: u64 = 100; // marshal last_processed (durable, finalized)
            const RETH_HEAD: u64 = ACKED + 1; // clean-shutdown speculative tail
                                              // `Fixture::new` seeds `fx.anchor_hash` at
                                              // RETH_HEAD, the speculative-tail block.
            let fx = Fixture::new(RETH_HEAD);
            let acked_hash = B256::repeat_byte(0x77);
            fx.chain.canonical.lock().unwrap().insert(ACKED, acked_hash);

            // reth head (`last_execution`) = RETH_HEAD; the marshal-acked cursor
            // (`last_consensus`, the floor seed) = ACKED < RETH_HEAD.
            let (_actor, _mailbox) = fx.build(ctx, RETH_HEAD, ACKED);

            // The acked cursor resolves via the provider: the floor is seeded there.
            assert_eq!(
                fx.chain.finalized_executed_hash(ACKED),
                Some(acked_hash),
                "the marshal-acked height resolves via the floor→provider fallback"
            );
            // The speculative tail sits above the acked floor: the provider has it, but
            // it is notarized-only and a sibling can still finalize, so it is not served.
            assert_eq!(
                fx.chain.finalized_executed_hash(RETH_HEAD),
                None,
                "a speculative-tail height above the acked cursor is NOT served \
                 by the floor even though the provider holds it (soundness)"
            );
        });
    }

    // `ordering_finalized` (the result-final cursor) must seed from the marshal-acked
    // cursor, not the reth head: seeded from the head, the first finalized delivery at
    // acked+1 computes `result_final = head − K` and pins the engine-API `finalized`
    // onto the orphanable speculative hash at `acked + N − K`; seeded from the acked
    // cursor the tier stays at the anchor and the head rolls onto the re-derived block.
    #[test]
    fn ordering_finalized_seeds_from_acked_not_reth_head() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ACKED: u64 = 100; // anchor == activation == marshal-acked cursor
            const N: u64 = 10; // reth speculative-tail depth (> K)
            let reth_head = ACKED + N;
            let spec_final = ACKED + N - K; // the height a head-seed would pin finalized to
            let fx = Fixture::new(ACKED).with_last_execution(reth_head);
            let anchor_hash = fx.anchor_hash;
            // Persisted speculative tail above the acked cursor: acked+1 carries a
            // sibling, and acked+N−K a distinct spec hash.
            let sibling = B256::repeat_byte(0x51);
            let spec_final_hash = B256::repeat_byte(0x57);
            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(ACKED + 1, sibling);
            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(spec_final, spec_final_hash);

            let (actor, mailbox) = fx.build(ctx.clone(), ACKED, ACKED);
            let handle = actor.start();

            // One finalized delivery at acked+1; its flush child supplies the witness
            // so it derives, and with no tip guard #2 stays cold.
            finalize_and_ack_behind(
                &fx,
                &mailbox,
                sample_order(Digest(B256::ZERO), ACKED + 1, B256::ZERO),
            )
            .await;

            let rederived = fx
                .chain
                .spec_executed_hash(ACKED + 1)
                .expect("acked+1 re-derived");
            let fcu = *fx
                .beacon
                .fcu_calls
                .lock()
                .unwrap()
                .last()
                .expect("a finalize FCU landed");
            assert_eq!(
                fcu.finalized_block_hash, anchor_hash,
                "finalized tier stays anchor-consistent (NOT the acked+N−K spec hash)"
            );
            assert_ne!(
                fcu.finalized_block_hash, spec_final_hash,
                "finalized must not be pinned onto the speculative tail"
            );
            assert_eq!(
                fcu.head_block_hash, rederived,
                "update_head rolled the head onto the re-derived block"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // `reseed_forward` must produce the same seed fields as cold-start `init`
    // at the landing, so the two actors' `seed_fields` snapshots match.
    #[test]
    fn reseed_forward_agrees_with_init() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let landing = ANCHOR + JUMP_THRESHOLD + 5_000;
            let landing_hash = B256::repeat_byte(0xE1);
            let floor = landing - K;

            let fx_init = Fixture::new(landing);
            fx_init
                .chain
                .canonical
                .lock()
                .unwrap()
                .insert(landing, landing_hash);
            // Distinct labels so the two actors' `pending_finalizations` gauges do
            // not collide in the shared metrics registry.
            let (init_actor, _m1) = fx_init.build(ctx.with_label("init"), landing, landing);
            let init_fields = init_actor.seed_fields();

            let fx_re = Fixture::new(ANCHOR);
            let (mut re_actor, _m2) = fx_re.build(ctx.with_label("reseed"), ANCHOR, ANCHOR);
            re_actor
                .reseed_forward(landing, landing_hash, floor)
                .await
                .expect("reseed_forward");
            let re_fields = re_actor.seed_fields();

            assert_eq!(
                init_fields, re_fields,
                "reseed_forward must mirror init's seed at the landing (never diverge)"
            );
        });
    }

    // `reseed_forward` must issue the canonicalization FCU that mirrors cold-start
    // `init`: it makes the backfilled `floor` visible by hash, so deriving
    // `floor + 1` fails before the reseed and succeeds after.
    #[test]
    fn reseed_forward_fcu_makes_backfilled_floor_visible_by_hash() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let landing = ANCHOR + JUMP_THRESHOLD + 5_000;
            let landing_hash = B256::repeat_byte(0xE1);
            let floor = landing - K;
            let floor_hash = B256::repeat_byte(0xF0);

            let fx = Fixture::new(ANCHOR);
            {
                let mut canon = fx.chain.canonical.lock().unwrap();
                canon.insert(floor, floor_hash);
                canon.insert(landing, landing_hash);
            }
            fx.chain.vis.register(floor, floor_hash);
            fx.chain.vis.register(landing, landing_hash);
            fx.chain.vis.set_frontier(floor - 1);

            assert!(
                !fx.chain.vis.visible(floor_hash),
                "floor must be by-hash-invisible before the reseed FCU"
            );
            let pre = fx
                .deriver
                .derive_and_execute(
                    sample_order(Digest(B256::ZERO), floor + 1, B256::ZERO),
                    floor_hash,
                    None,
                )
                .await;
            assert!(
                pre.is_err(),
                "derive on a by-hash-invisible parent must fail (ParentHeaderMissing)"
            );

            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            actor
                .reseed_forward(landing, landing_hash, floor)
                .await
                .expect("reseed_forward");

            {
                let fcus = fx.beacon.fcu_calls.lock().unwrap();
                let reseed_fcu = fcus
                    .last()
                    .expect("reseed_forward must issue a canonicalization FCU");
                assert_eq!(
                    reseed_fcu.head_block_hash, landing_hash,
                    "FCU head = landing"
                );
                assert_eq!(
                    reseed_fcu.safe_block_hash, landing_hash,
                    "FCU safe = landing (ordering-final tip)"
                );
                assert_eq!(
                    reseed_fcu.finalized_block_hash, floor_hash,
                    "FCU finalized = floor (two-tier; never finalize ahead of the result tier)"
                );
            }

            assert!(
                fx.chain.vis.visible(floor_hash),
                "the reseed FCU must make the backfilled floor visible by hash"
            );
            let post = fx
                .deriver
                .derive_and_execute(
                    sample_order(Digest(B256::ZERO), floor + 1, B256::ZERO),
                    floor_hash,
                    None,
                )
                .await;
            assert!(
                post.is_ok(),
                "derive on the now-visible floor succeeds (the floor no longer freezes)"
            );
        });
    }

    // The re-jump callback is not invoked while the gap is at most JUMP_THRESHOLD:
    // the inlet's ordinary pulls still cover the serving window.
    #[test]
    fn re_jump_is_noop_within_serving_window() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let (cb, calls) = recording_re_jump(Scripted::Lagging);
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD))
                .expect("send tip");

            finalize_and_ack_behind(
                &fx,
                &mailbox,
                sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO),
            )
            .await;

            assert!(
                calls.lock().unwrap().is_empty(),
                "a gap ≤ JUMP_THRESHOLD must NOT invoke the re-jump"
            );
            assert!(
                fx.marshal.floors.lock().unwrap().is_empty(),
                "no re-jump ⇒ no set_floor"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A `StalledWithPeers` outcome (a connected but wedged EL) is non-fatal: no
    // upstream rotation, no floor advance, but the observability counter is bumped.
    #[test]
    fn re_jump_stalled_with_peers_is_nonfatal_does_not_rotate_and_bumps_counter() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let (cb, calls, rotations) =
                recording_re_jump_with_rotate(vec![Scripted::StalledWithPeers(
                    "reth CONNECTED but executed head frozen".into(),
                )]);
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                .expect("send tip");
            ctx.sleep(Duration::from_millis(10)).await;

            finalize_and_ack_behind(
                &fx,
                &mailbox,
                sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO),
            )
            .await;

            assert_eq!(
                *calls.lock().unwrap(),
                vec![ANCHOR],
                "re-jump was invoked (gap > threshold) and hit the connected-but-wedged net"
            );
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "a connected-but-wedged EL stall must NOT rotate the upstream (local reth wedge)"
            );
            assert!(
                fx.marshal.floors.lock().unwrap().is_empty(),
                "a StalledWithPeers re-jump must NOT advance the marshal floor (stays deferred)"
            );
            assert_eq!(
                fx.sync_metrics.el_sync_stalled_with_peers.get(),
                1,
                "the connected-but-wedged observability counter must be bumped"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A `Stalled` outcome (an `EL_SYNC_NO_PROGRESS` transport stall) is non-fatal:
    // the executor keeps running and a follow-up finalize still acks.
    #[test]
    fn re_jump_stalled_is_nonfatal() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let (cb, calls) =
                recording_re_jump(Scripted::Stalled("reth EL-sync stalled for 120s".into()));
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                .expect("send tip");
            ctx.sleep(Duration::from_millis(10)).await;

            finalize_and_ack_behind(
                &fx,
                &mailbox,
                sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO),
            )
            .await;

            assert_eq!(
                *calls.lock().unwrap(),
                vec![ANCHOR],
                "re-jump was invoked (gap > threshold) and stalled"
            );
            assert!(
                fx.marshal.floors.lock().unwrap().is_empty(),
                "a Stalled re-jump must NOT advance the marshal floor"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A `Lagging` (stale / shallow) target is a no-op: no re-seed, no `set_floor`,
    // and the executor keeps running.
    #[test]
    fn re_jump_lagging_is_noop() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let (cb, calls) = recording_re_jump(Scripted::Lagging);
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                .expect("send tip");
            ctx.sleep(Duration::from_millis(10)).await;

            finalize_and_ack_behind(
                &fx,
                &mailbox,
                sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO),
            )
            .await;

            assert_eq!(
                *calls.lock().unwrap(),
                vec![ANCHOR],
                "re-jump was invoked (gap > threshold) but returned Lagging"
            );
            assert!(
                fx.marshal.floors.lock().unwrap().is_empty(),
                "Lagging must NOT advance the marshal floor"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A landing contradicting an authenticated certificate is `Fault::corruption`
    // and rotates nothing: the target came from this node's own archive, so there
    // is no upstream to rotate away from.
    #[test]
    fn re_jump_invalid_target_is_corruption_and_does_not_rotate() {
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            let runtime = deterministic::Runner::default();
            runtime.start(|ctx| async move {
                const ANCHOR: u64 = 100;
                let (cb, calls, rotations) =
                    recording_re_jump_with_rotate(vec![Scripted::InvalidTarget(
                        "reth rejected the served tip as INVALID".into(),
                    )]);
                let fx = Fixture::new(ANCHOR).with_re_jump(cb);
                let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
                let mut handle = actor.start();

                mailbox
                    .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                    .expect("send tip");
                ctx.sleep(Duration::from_millis(20)).await;

                assert_eq!(
                    *calls.lock().unwrap(),
                    vec![ANCHOR],
                    "re-jump was invoked (gap > threshold) and returned InvalidTarget"
                );
                assert_eq!(
                    rotations.load(std::sync::atomic::Ordering::Relaxed),
                    0,
                    "an InvalidTarget on a target read from this node's own archive rotated \
                     the upstream — there is nobody to rotate away from"
                );
                assert!(
                    fx.marshal.floors.lock().unwrap().is_empty(),
                    "an InvalidTarget re-jump must NOT advance the marshal floor"
                );
                assert!(
                    (&mut handle).now_or_never().is_some(),
                    "the executor kept running after its EL contradicted an authenticated \
                     certificate — §5.4 files that as corruption, which shuts it down"
                );
                assert!(
                    !fx.safety_halt.is_engaged(),
                    "corruption is a loud death, not the fork-safety latch (that one is L1Fork)"
                );
            });
        });
        assert_eq!(
            counter_at(
                &drain_counters(&snap),
                "dpos_executor_fault_total",
                ("class", "corruption")
            ),
            1,
            "the landing contradiction must reach the fault router as a Corruption"
        );
    }

    // A steady-state `L1Fork` (the EL-synced head does not descend from the
    // L1-finalized checkpoint) is distinct from `AuthFailed`: L1 finality itself
    // disagrees, so there is no honest upstream to rotate to and the executor
    // engages the fork-safety latch and parks, retaining marshal acks un-resolved
    // so the marshal stays alive.
    #[test]
    fn re_jump_l1_fork_engages_safety_halt_and_does_not_rotate() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let (cb, calls, rotations) = recording_re_jump_with_rotate(vec![Scripted::L1Fork(
                "L1 Rollup checkpoint is NOT in the local chain after EL-sync".into(),
            )]);
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                .expect("send tip");

            wait_until(&ctx, "L1Fork SafetyHalt engaged", || {
                fx.safety_halt.is_engaged()
            })
            .await;
            ctx.sleep(Duration::from_millis(20)).await;
            let (msg, mut post_waiter) =
                finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO));
            mailbox
                .send(msg)
                .expect("executor mailbox stays open while parked");
            ctx.sleep(Duration::from_millis(20)).await;
            assert!(
                (&mut post_waiter).now_or_never().is_none(),
                "a SafetyHalted executor retains the finalize ack (no derive, no cancel)"
            );
            assert!(
                fx.chain.spec_executed_hash(ANCHOR + 1).is_none(),
                "a SafetyHalted executor derives nothing further"
            );
            let mut handle = handle;
            assert!(
                (&mut handle).now_or_never().is_none(),
                "the executor parks (does not exit) on an L1-fork SafetyHalt"
            );

            assert_eq!(
                *calls.lock().unwrap(),
                vec![ANCHOR],
                "re-jump was invoked and returned L1Fork"
            );
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "an L1 fork does NOT rotate (no honest upstream — L1 finality disagrees)"
            );
            assert!(
                fx.safety_halt.is_engaged(),
                "an L1 fork engages the fork-safety latch (SafetyHalt)"
            );
            assert_eq!(
                fx.sync_metrics.degraded_value(SyncReason::L1Fork),
                1,
                "the l1_fork gauge is raised for the alert"
            );
        });
    }

    // A single `Stalled` does not rotate (an honest transient stall is tolerated);
    // at `MAX_UPSTREAM_FAULTS` consecutive stalls the upstream is failed over once
    // and the streak resets, so a further stall does not rotate.
    #[test]
    fn re_jump_stalled_rotates_after_streak_then_resets() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let (cb, calls, rotations) = recording_re_jump_with_rotate(vec![Scripted::Stalled(
                "reth EL-sync stalled".into(),
            )]);
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // `Stalled` never reseeds, so `ordering_finalized` stays at ANCHOR and
            // the same tip re-triggers once the prior jump's `jump_done` arm clears.
            for _ in 0..crate::cert_inlet::MAX_UPSTREAM_FAULTS {
                mailbox
                    .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                    .expect("send tip");
                ctx.sleep(Duration::from_millis(10)).await;
            }
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "exactly ONE rotate at the MAX_UPSTREAM_FAULTS-th consecutive stall"
            );

            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                .expect("send tip");
            ctx.sleep(Duration::from_millis(10)).await;
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "the streak reset after rotating: one post-reset stall must not re-rotate"
            );
            assert_eq!(
                calls.lock().unwrap().len(),
                crate::cert_inlet::MAX_UPSTREAM_FAULTS as usize + 1,
                "every tip spawned a fresh re-jump (no jump skipped / doubled)"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // While the tip is held (`awaiting_seed`), a shallow gap (≤ JUMP_THRESHOLD)
    // does not start a re-jump; only a deep gap (> JUMP_THRESHOLD) does.
    #[test]
    fn re_jump_does_not_start_on_shallow_gap_while_block_held() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let (cb, calls) = recording_re_jump(Scripted::Lagging);
            let fx = Fixture::new(ANCHOR)
                .with_re_jump(cb)
                .with_seed_store(crate::beacon::testing::SeedStore::new())
                .with_epocher(beacon_active_epocher());
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            let (msg, _waiter) =
                finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO));
            mailbox.send(msg).expect("send held block");

            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD))
                .expect("send shallow tip");
            ctx.sleep(Duration::from_millis(10)).await;

            assert!(
                calls.lock().unwrap().is_empty(),
                "re-jump must NOT start on a shallow gap (Case A: hold untouched)"
            );
            assert!(
                fx.beacon.new_payload_calls.lock().unwrap().is_empty(),
                "the block stayed held (no premature derive)"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // The seed hold does not gate `maybe_re_jump`: that gate bounds a σ-less
    // node's stall, so a block held for its σ must not suppress the spawn.
    //
    // `reseed_forward` disposes the held block with `acknowledge()` — a dropped
    // `Exact` is a Canceled ack, fatal to the marshal — so it is pruned, not
    // skipped.
    #[test]
    fn a_deep_gap_spawns_a_rejump_even_while_a_block_is_held_for_its_seed() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let landing_h = ANCHOR + JUMP_THRESHOLD + 5_000;
            let landing_hash = B256::repeat_byte(0xE1);
            let floor = landing_h - K;
            let (cb, calls) = recording_re_jump(Scripted::Landed {
                landing: landing_h,
                hash: landing_hash,
                floor,
            });
            let fx = Fixture::new(ANCHOR)
                .with_re_jump(cb)
                .with_seed_store(crate::beacon::testing::SeedStore::new())
                .with_epocher(beacon_active_epocher());
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            let (m1, mut w1) =
                finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO));
            mailbox.send(m1).expect("send held block");
            ctx.sleep(Duration::from_millis(20)).await;
            assert!(
                (&mut w1).now_or_never().is_none(),
                "premise: the block is HELD (unacked) when the deep tip arrives"
            );

            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                .expect("send deep tip");

            wait_until(&ctx, "the re-jump spawned despite the held block", || {
                !calls.lock().unwrap().is_empty()
            })
            .await;
            assert_eq!(
                *calls.lock().unwrap(),
                vec![ANCHOR],
                "re-jump SPAWNED once despite the held block (durably-stuck recovery)"
            );
            w1.await.expect(
                "held block disposed via acknowledge (Ok, never Canceled) on the landed re-jump",
            );
            assert_eq!(
                *fx.marshal.floors.lock().unwrap(),
                vec![floor],
                "the landed reseed advanced the marshal floor"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A hole in the marshal's backfill range cannot self-heal (`get_block` is
    // local-only), so the executor fails at the backfill site rather than
    // warn-and-skipping to a later gap-walk at the wrong height.
    #[test]
    fn backfill_hole_is_fatal_at_the_backfill_site() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            // last_consensus = ANCHOR + 2 ⇒ backfill range 101..=102; `canned` empty.
            let (actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR + 2);
            let handle = actor.start();
            let _ = handle.await;
            assert!(
                fx.beacon.new_payload_calls.lock().unwrap().is_empty(),
                "a backfill hole must shut down before any block derives"
            );
        });
    }

    use metrics_util::{
        debugging::{DebugValue, DebuggingRecorder, Snapshotter},
        CompositeKey,
    };

    /// `Snapshotter::snapshot` resets every counter it reads, so a test takes one
    /// snapshot and queries this drained copy.
    fn drain_counters(snap: &Snapshotter) -> Vec<(CompositeKey, u64)> {
        snap.snapshot()
            .into_vec()
            .into_iter()
            .filter_map(|(key, _, _, value)| match value {
                DebugValue::Counter(count) => Some((key, count)),
                _ => None,
            })
            .collect()
    }

    /// Total of an unlabelled counter — the shape both detector counters use.
    fn counter_total(drained: &[(CompositeKey, u64)], name: &str) -> u64 {
        drained
            .iter()
            .filter(|(composite, _)| composite.key().name() == name)
            .map(|(_, count)| count)
            .sum()
    }

    fn counter_at(drained: &[(CompositeKey, u64)], name: &str, label: (&str, &str)) -> u64 {
        drained
            .iter()
            .filter(|(composite, _)| {
                let key = composite.key();
                key.name() == name
                    && key
                        .labels()
                        .any(|l| l.key() == label.0 && l.value() == label.1)
            })
            .map(|(_, count)| count)
            .sum()
    }

    // The detector stays silent below the threshold, reports once when the hold
    // outlives it (not once per call), and changes nothing: the block stays held
    // and derives when σ lands.
    //
    // `HeldForSeed::since` must ride through `try_eager_finalized_derive`'s
    // restore — resetting it there would keep the detector silent under a stream
    // of unrelated seed records, the exact condition it exists to report.
    //
    // The hold is back-dated rather than waited out: the deterministic runtime
    // advances virtual time in 1 ms cycles, so sleeping past a 60 s threshold
    // would dominate the suite.
    #[test]
    fn a_fresh_seed_hold_is_silent_and_a_stalled_one_reports_once() {
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        const NAME: &str = "dpos_executor_seed_hold_stalled_total";
        metrics::with_local_recorder(&recorder, || {
            let runtime = deterministic::Runner::default();
            runtime.start(|ctx| async move {
                const ANCHOR: u64 = 100;
                let h = ANCHOR + 1;
                let store = crate::beacon::testing::SeedStore::new();
                let fx = Fixture::new(ANCHOR)
                    .with_seed_store(store.clone())
                    .with_epocher(beacon_active_epocher());
                let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);

                let order = OrderBlock {
                    proposal_view: h,
                    ..sample_order(Digest(B256::ZERO), h, B256::ZERO)
                };
                let (ack, mut waiter) = Exact::handle();
                actor
                    .on_finalized_block(Span::current(), order, ack)
                    .await
                    .expect("beacon-active round, empty store");
                assert!(actor.awaiting_seed.is_some(), "premise: the block is HELD");

                let reports = || counter_total(&drain_counters(&snap), NAME);

                actor.detect_stalled_seed_hold();
                assert_eq!(
                    reports(),
                    0,
                    "a hold younger than the threshold must not be reported"
                );

                let backdate = |actor: &mut Actor<_, _, _, _, _>| {
                    let held = actor.awaiting_seed.as_mut().expect("still held");
                    held.since = held
                        .since
                        .checked_sub(SEED_HOLD_STALL_THRESHOLD)
                        .expect("representable");
                };

                backdate(&mut actor);
                actor.detect_stalled_seed_hold();
                actor.detect_stalled_seed_hold();
                actor.detect_stalled_seed_hold();
                assert_eq!(reports(), 1, "one stall is one event — not one per call");

                // An unrelated σ misses and re-holds the block; clearing `reported`
                // isolates whether `since` survived the restore.
                store.record(real_witness(active_round(h + 500)));
                actor
                    .try_eager_finalized_derive(EagerTrigger::Notified)
                    .await
                    .expect("unrelated σ is a miss");
                assert!(actor.awaiting_seed.is_some(), "the miss re-held the block");
                actor.awaiting_seed.as_mut().expect("held").reported = false;
                actor.detect_stalled_seed_hold();
                assert_eq!(
                    reports(),
                    1,
                    "`since` must survive the miss re-hold — a reset would make this \
                     hold look fresh, silencing the detector under any stream of \
                     unrelated seed records"
                );

                assert!(
                    fx.beacon.new_payload_calls.lock().unwrap().is_empty(),
                    "the detector must not derive the held block"
                );
                assert!(
                    (&mut waiter).now_or_never().is_none(),
                    "the detector must not resolve the held ack"
                );
                assert!(!fx.safety_halt.is_engaged(), "a detector never halts");

                store.record(real_witness(active_round(h)));
                actor
                    .try_eager_finalized_derive(EagerTrigger::Notified)
                    .await
                    .expect("σ landed");
                assert!(
                    actor.awaiting_seed.is_none() && fx.chain.finalized_executed_hash(h).is_some(),
                    "the hold survived the detector intact and derived when σ arrived"
                );
                waiter.await.expect("acked after the stall");
            });
        });
    }

    // The detector runs off the FCU heartbeat tick and nothing else.
    #[test]
    fn the_fcu_heartbeat_is_what_reports_a_stalled_seed_hold() {
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            let runtime = deterministic::Runner::default();
            runtime.start(|ctx| async move {
                const ANCHOR: u64 = 100;
                let h = ANCHOR + 1;
                let fx = Fixture::new(ANCHOR)
                    .with_seed_store(crate::beacon::testing::SeedStore::new())
                    .with_epocher(beacon_active_epocher())
                    .with_fcu_heartbeat(Duration::from_millis(20));
                let (mut actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);

                // Hold and back-date before the actor is spawned, so a couple of
                // heartbeat ticks are all the timeline this needs.
                let order = OrderBlock {
                    proposal_view: h,
                    ..sample_order(Digest(B256::ZERO), h, B256::ZERO)
                };
                let (ack, _waiter) = Exact::handle();
                actor
                    .on_finalized_block(Span::current(), order, ack)
                    .await
                    .expect("held");
                let held = actor.awaiting_seed.as_mut().expect("held");
                held.since = held
                    .since
                    .checked_sub(SEED_HOLD_STALL_THRESHOLD)
                    .expect("representable");

                let handle = actor.start();
                ctx.sleep(Duration::from_millis(100)).await;

                let reports = counter_total(
                    &drain_counters(&snap),
                    "dpos_executor_seed_hold_stalled_total",
                );
                assert_eq!(
                    reports, 1,
                    "five heartbeat ticks over a stalled hold: reported, and reported once"
                );

                drop(mailbox);
                let _ = handle.await;
            });
        });
    }

    // The backfill hole seen at the router: a bare `break` exits the loop without
    // reading the halt latch, so an already-halted node would drop every retained
    // marshal `Exact` into Canceled (fatal to the marshal); the hole must be
    // routed as a Corruption.
    #[test]
    fn backfill_hole_routes_through_the_fault_router() {
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            let runtime = deterministic::Runner::default();
            runtime.start(|ctx| async move {
                const ANCHOR: u64 = 100;
                let fx = Fixture::new(ANCHOR);
                let (actor, _mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR + 2);
                let mut handle = actor.start();
                ctx.sleep(Duration::from_millis(20)).await;
                assert!(
                    (&mut handle).now_or_never().is_some(),
                    "the routed Corruption shuts the executor down"
                );
            });
        });
        assert_eq!(
            counter_at(
                &drain_counters(&snap),
                "dpos_executor_fault_total",
                ("class", "corruption")
            ),
            1,
            "the backfill hole must reach the fault router, not break the loop behind its back"
        );
    }

    // Teardown, not a fault: every sender dropped with the latch clear is the node
    // shutting this executor down, so it exits with the `mailbox_closed` cause and
    // raises no fault.
    #[test]
    fn mailbox_close_exits_cleanly_when_not_halted() {
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            let runtime = deterministic::Runner::default();
            runtime.start(|ctx| async move {
                const ANCHOR: u64 = 100;
                let fx = Fixture::new(ANCHOR);
                let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
                let mut handle = actor.start();
                ctx.sleep(Duration::from_millis(5)).await;
                drop(mailbox);
                ctx.sleep(Duration::from_millis(20)).await;
                assert!(
                    (&mut handle).now_or_never().is_some(),
                    "a closed mailbox with the latch clear must resolve the actor handle"
                );
                assert!(!fx.safety_halt.is_engaged(), "teardown is not a fault");
            });
        });
        let drained = drain_counters(&snap);
        assert_eq!(
            counter_at(
                &drained,
                "dpos_executor_fault_total",
                ("class", "corruption")
            ),
            0,
        );
        assert_eq!(
            counter_at(
                &drained,
                "dpos_executor_exit_total",
                ("cause", "mailbox_closed")
            ),
            1,
        );
    }

    // The latch can already be engaged at start: `SafetyHalt::restore_marker`
    // re-engages from the datadir marker before any actor spawns, so no fault
    // reaches the router's own is-engaged check; the top-of-loop gate must park the
    // actor instead of driving reth for a chain it has already refused.
    #[test]
    fn a_marker_restored_latch_parks_the_executor_before_it_drives_reth() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            fx.safety_halt.engage(SyncReason::ResultDivergence);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // A block plus the flush child it needs to derive, so a missing gate
            // shows up as real EL traffic rather than a held tip.
            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let child = child_of(&order);
            let post_halt = sample_order(child.digest(), ANCHOR + 3, B256::ZERO);
            let (msg, waiter) = finalize_msg(order);
            mailbox.send(msg).expect("send finalize");
            let (child_msg, _child_waiter) = finalize_msg(child);
            mailbox.send(child_msg).expect("send the flush child");
            ctx.sleep(Duration::from_millis(20)).await;

            assert!(
                fx.deriver.seeds_seen.lock().unwrap().is_empty(),
                "the executor DERIVED with the latch engaged"
            );
            assert!(
                fx.beacon.new_payload_calls.lock().unwrap().is_empty(),
                "the executor IMPORTED into reth with the latch engaged"
            );
            assert!(
                fx.beacon.fcu_calls.lock().unwrap().is_empty(),
                "the executor drove reth's FORKCHOICE with the latch engaged"
            );

            assert_parked_retaining_acks(
                &ctx,
                handle,
                waiter,
                &mailbox,
                &fx.safety_halt,
                post_halt,
            )
            .await;
        });
    }

    // The arm's latch check is defence in depth: the only production engage sites
    // are the router (which parks and never returns) and the startup marker restore
    // (covered by the top-of-loop gate). A latch engaged from outside this actor
    // while the loop sits in `select!` would otherwise be missed, and a `break`
    // here would drop the held marshal `Exact` into Canceled — fatal to the marshal.
    #[test]
    fn mailbox_close_while_halted_parks_and_retains_acks() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            // Beacon-active epoch with an empty store, so the block is held and its
            // ack is un-resolved when the latch trips.
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(crate::beacon::testing::SeedStore::new())
                .with_epocher(beacon_active_epocher());
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let mut handle = actor.start();

            let (msg, mut waiter) =
                finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO));
            mailbox.send(msg).expect("send finalize");
            ctx.sleep(Duration::from_millis(20)).await;
            assert!(
                (&mut waiter).now_or_never().is_none(),
                "the held tip's ack is the one the park must retain"
            );

            // Engage mid-flight with the loop parked in `select!` and no timer due
            // (the fixture's heartbeat is 60s), then close.
            fx.safety_halt.engage(SyncReason::ResultDivergence);
            drop(mailbox);
            ctx.sleep(Duration::from_millis(20)).await;

            assert!(
                (&mut waiter).now_or_never().is_none(),
                "the held block's ack must stay RETAINED — a Canceled ack kills the marshal"
            );
            assert!(
                (&mut handle).now_or_never().is_none(),
                "a mailbox close under an engaged latch must PARK, not return"
            );
        });
    }

    // While a jump is in flight (`jump_done` armed) the executor is the only EL
    // writer: a finalize-derive or speculative execute would retarget reth's
    // backfill and starve the jump's `Valid` terminator, so both are suppressed and
    // the queued finalize drains once the jump completes.
    #[test]
    fn no_derive_or_spec_while_jump_in_flight_then_drains() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            // The re-jump waiter hangs until `release`, then lands as a no-op
            // `Lagging` (no floor change, so the queued ANCHOR+1 stays derivable).
            let release = Arc::new(tokio::sync::Notify::new());
            let calls: RejumpCalls = Arc::new(Mutex::new(Vec::new()));
            let calls_cl = calls.clone();
            let release_cl = release.clone();
            let call: ReJumpFn = Arc::new(move |from, _target| {
                calls_cl.lock().unwrap().push(from);
                let release = release_cl.clone();
                Box::pin(async move {
                    release.notified().await;
                    crate::cold_start_jump::JumpOutcome::Lagging
                })
            });
            let cb = ReJump {
                call,
                threshold: JUMP_THRESHOLD,
                rotate: None,
                probe: None,
                tracked_epoch: None,
            };
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                .expect("send tip");
            ctx.sleep(Duration::from_millis(10)).await;
            assert_eq!(calls.lock().unwrap().len(), 1, "the jump spawned");

            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(ANCHOR + 1, order.clone());
            let (fin, _w) = finalize_msg(order.clone());
            mailbox.send(fin).expect("send finalize");
            mailbox.send(spec_msg(&order)).expect("send spec");
            ctx.sleep(Duration::from_millis(10)).await;
            assert!(
                fx.beacon.new_payload_calls.lock().unwrap().is_empty(),
                "no finalize-derive or spec-execute may fire while a jump is in flight"
            );

            release.notify_one();
            let (fin_child, _wc) = finalize_msg(child_of(&order));
            mailbox.send(fin_child).expect("send child");
            ctx.sleep(Duration::from_millis(20)).await;
            assert!(
                !fx.beacon.new_payload_calls.lock().unwrap().is_empty(),
                "the queued finalize drains once the jump completes"
            );
            drop(mailbox);
            let _ = handle.await;
        });
    }

    // After a far re-jump, `reseed_forward` raises `spec_head` to the landing, so a
    // `SpecNotarized` at landing+1 speculates; a stale pre-jump `spec_head` would
    // drop it silently.
    #[test]
    fn re_jump_resets_stale_spec_head() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let landing_h = ANCHOR + JUMP_THRESHOLD + 5_000;
            let landing_hash = B256::repeat_byte(0xE1);
            let floor = landing_h - K;
            let (cb, _calls) = recording_re_jump(Scripted::Landed {
                landing: landing_h,
                hash: landing_hash,
                floor,
            });
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // Make the landing canonical so the speculation's parent read succeeds.
            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(landing_h, landing_hash);

            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                .expect("send tip");
            ctx.sleep(Duration::from_millis(10)).await;

            let order = sample_order(Digest(B256::ZERO), landing_h + 1, B256::ZERO);
            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(landing_h + 1, order.clone());
            // The node is behind the deep tip, so the finalized derive needs the
            // attested body at landing+1+K whose result commits the speculated hash.
            let spec_hash = sealed_at(landing_hash, landing_h + 1, order.digest().0).hash();
            fx.marshal.canned.lock().unwrap().insert(
                landing_h + 1 + K,
                sample_order(Digest(B256::ZERO), landing_h + 1 + K, spec_hash),
            );
            mailbox.send(spec_msg(&order)).expect("send spec");

            let (msg, waiter) = finalize_msg(order.clone());
            mailbox.send(msg).expect("send finalize");
            waiter.await.expect("ack");

            {
                let payloads = fx.beacon.new_payload_calls.lock().unwrap();
                let heights: Vec<u64> = payloads.iter().map(|p| p.number).collect();
                assert_eq!(
                    heights,
                    vec![landing_h + 1],
                    "spec_head was raised to the landing ⇒ landing+1 speculated once \
                     (finalize skipped the re-derive)"
                );
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A dropped `Exact` is fatal to the real marshal's `run`; this harness pins
    // that a SafetyHalt keeps the marshal serving instead.
    mod real_marshal {
        use super::*;
        use crate::cert_inlet::NoopResolver;
        use commonware_consensus::{
            marshal::{
                core::{Actor as MarshalActor, Buffer as MarshalBuffer, Mailbox as CwMailbox},
                resolver::handler,
                standard::Standard,
            },
            simplex::types::{Activity, Finalization, Finalize, Proposal},
            types::{Epoch, Round, View, ViewDelta},
            Reporter,
        };
        use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
        use commonware_math::algebra::Random as _;
        use commonware_p2p::Recipients;
        use commonware_parallel::Sequential;
        use commonware_runtime::buffer::paged::CacheRef;
        use commonware_utils::{
            channel::oneshot as cw_oneshot, ordered::BiMap, NZUsize, TryCollect as _,
        };
        use fluentbase_bls::{
            fluent_namespace,
            keys::ValidatorBlsKeypair,
            scheme::{build_signer, build_verifier},
            BlsPubkey, Scheme as BlsScheme,
        };
        use rand_08::rngs::StdRng;
        use rand_core::SeedableRng as _;

        type StdVariant = Standard<OrderBlock>;
        type RealMailbox = CwMailbox<BlsScheme, StdVariant>;

        /// Body-less [`MarshalBuffer`]: bodies are made local via `verified()`
        /// before their finalization is reported, so every lookup here misses.
        #[derive(Clone)]
        struct NoopBuffer;

        impl MarshalBuffer<StdVariant> for NoopBuffer {
            type PublicKey = PeerPubkey;
            type CachedBlock = OrderBlock;

            async fn find_by_digest(&self, _digest: Digest) -> Option<OrderBlock> {
                None
            }
            async fn find_by_commitment(&self, _commitment: Digest) -> Option<OrderBlock> {
                None
            }
            async fn subscribe_by_digest(
                &self,
                _digest: Digest,
            ) -> cw_oneshot::Receiver<OrderBlock> {
                // Leaking the sender keeps the receiver unresolved rather than
                // canceled, so the marshal keeps the subscription open.
                let (tx, rx) = cw_oneshot::channel();
                std::mem::forget(tx);
                rx
            }
            async fn subscribe_by_commitment(
                &self,
                _commitment: Digest,
            ) -> cw_oneshot::Receiver<OrderBlock> {
                let (tx, rx) = cw_oneshot::channel();
                std::mem::forget(tx);
                rx
            }
            async fn finalized(&self, _commitment: Digest) {}
            async fn send(
                &self,
                _round: Round,
                _block: OrderBlock,
                _recipients: Recipients<PeerPubkey>,
            ) {
            }
        }

        /// Forwards the real marshal's `Update`s into the executor mailbox; the
        /// `Exact` ack rides inside the command, as in production.
        #[derive(Clone)]
        struct ForwardToExecutor(Mailbox);

        impl Reporter for ForwardToExecutor {
            type Activity = Update<OrderBlock>;

            async fn report(&mut self, activity: Update<OrderBlock>) {
                let _ = self.0.send(Message {
                    cause: Span::current(),
                    command: Command::Finalize(Box::new(activity)),
                });
            }
        }

        struct Committee {
            signers: Vec<BlsScheme>,
            verifier: BlsScheme,
        }

        fn committee(seed: u64) -> Committee {
            const N: usize = 4;
            let mut rng = StdRng::seed_from_u64(seed);
            let peer_sks: Vec<_> = (0..N)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let bls_kps: Vec<_> = (0..N)
                .map(|_| ValidatorBlsKeypair::generate(&mut rng))
                .collect();
            let bimap: BiMap<PeerPubkey, BlsPubkey> = peer_sks
                .iter()
                .zip(bls_kps.iter())
                .map(|(p, b)| {
                    use commonware_codec::DecodeExt as _;
                    (
                        p.public_key(),
                        BlsPubkey::decode(b.public_bytes().as_slice()).unwrap(),
                    )
                })
                .try_collect()
                .unwrap();
            let ns = fluent_namespace(20_994);
            let signers = bls_kps
                .iter()
                .map(|kp| build_signer(&ns, bimap.clone(), kp, 0, None).expect("member"))
                .collect();
            let verifier = build_verifier(&ns, bimap, 0, None);
            Committee { signers, verifier }
        }

        /// A 2f+1 finalization certificate over `block`'s digest.
        fn certify(c: &Committee, block: &OrderBlock) -> Finalization<BlsScheme, Digest> {
            let round = Round::new(Epoch::new(0), View::new(block.height));
            let prop = Proposal::new(round, View::new(block.height - 1), block.digest());
            let finalizes: Vec<_> = c
                .signers
                .iter()
                .take(3)
                .map(|s| Finalize::sign(s, prop.clone()).expect("sign"))
                .collect();
            Finalization::from_finalizes(&c.verifier, finalizes.iter(), &Sequential)
                .expect("quorum")
        }

        /// Make `block` local and report its finalization so the marshal stores
        /// it and dispatches `Update::Block` to the executor.
        async fn finalize_via_marshal(
            marshal: &mut RealMailbox,
            c: &Committee,
            block: &OrderBlock,
        ) {
            let round = Round::new(Epoch::new(0), View::new(block.height));
            marshal.verified(round, block.clone()).await;
            marshal
                .report(Activity::Finalization(certify(c, block)))
                .await;
        }

        #[test]
        fn safety_halt_keeps_marshal_alive_serving_blocks() {
            let runtime = deterministic::Runner::default();
            runtime.start(|ctx| async move {
                let c = committee(7);

                let page_cache = CacheRef::from_pooler(
                    &ctx,
                    crate::outer::PAGE_CACHE_PAGE_SIZE,
                    crate::outer::PAGE_CACHE_CAPACITY,
                );
                let finalizations = crate::outer::init_finalizations_archive(
                    &ctx,
                    "halt-liveness",
                    page_cache.clone(),
                )
                .await
                .expect("init finalizations archive");
                let blocks = crate::outer::init_finalized_blocks_archive(&ctx, "halt-liveness")
                    .await
                    .expect("init finalized blocks archive");
                let module = crate::committee::testing::SchemeCommittee::new({
                    let verifier = c.verifier.clone();
                    move |_| Some(verifier.clone())
                });
                let provider = crate::outer::EpochSchemeProvider::new(module);
                let (marshal_actor, marshal_mailbox, last_processed) = MarshalActor::init(
                    ctx.with_label("marshal"),
                    finalizations,
                    blocks,
                    commonware_consensus::marshal::Config {
                        provider,
                        epocher: crate::epocher::OriginEpocher::new(
                            0,
                            std::num::NonZeroU64::new(1_000).unwrap(),
                        ),
                        partition_prefix: "halt-liveness".into(),
                        mailbox_size: 64,
                        view_retention_timeout: ViewDelta::new(100),
                        prunable_items_per_section: std::num::NonZeroU64::new(4_096).unwrap(),
                        replay_buffer: NZUsize!(1024),
                        key_write_buffer: NZUsize!(1024),
                        value_write_buffer: NZUsize!(1024),
                        block_codec_config: (),
                        max_repair: NZUsize!(20),
                        max_pending_acks: crate::outer::MAX_PENDING_ACKS,
                        page_cache,
                        strategy: Sequential,
                    },
                )
                .await;
                assert_eq!(last_processed.get(), 0, "fresh archives");

                let fx = Fixture::new(0);
                let anchor_hash = fx.anchor_hash;
                let (executor, exec_mailbox) = Actor::init(
                    ctx.clone(),
                    Config {
                        beacon_engine: fx.beacon.clone(),
                        deriver: fx.deriver.clone(),
                        executed: fx.chain.clone(),
                        marshal: marshal_mailbox.clone(),
                        fcu_heartbeat_interval: Duration::from_secs(60),
                        last_consensus_finalized_height: Height::new(0),
                        last_execution_finalized_height: 0,
                        initial_finalized: (Height::new(0), anchor_hash),
                        initial_head: (Height::new(0), anchor_hash),
                        initial_marshal_floor: 0,
                        boundary_fetch: None,
                        boundary_enter: std::sync::Arc::new(|_| {}),
                        boundary_read_floor: std::sync::Arc::new(|_| Box::pin(async {})),
                        dpos_activation_block: 0,
                        fcu_pace: Duration::from_millis(0),
                        peers_for_finalization: std::sync::Arc::new(dummy_peers),
                        metrics: ExecutorMetrics::default(),
                        sync_metrics: fx.sync_metrics.clone(),
                        safety_halt: fx.safety_halt.clone(),
                        spawn_unblocked: std::sync::Arc::new(tokio::sync::Notify::new()),
                        re_jump: None,
                        // Negative provider on purpose: every height here sits in
                        // epoch 0 under the epocher below, which is beacon-inactive,
                        // so the derive resolves `None` and nothing can be held.
                        randomness: crate::beacon::absent_unregistered(),
                        epocher: crate::epocher::OriginEpocher::new(
                            0,
                            std::num::NonZeroU64::new(1 << 40).expect("nonzero"),
                        ),
                        anchor_advanced: std::sync::Arc::new(|| {}),
                    },
                );
                let _executor_handle = executor.start();

                // Held so the marshal's resolver channel never closes.
                let (_resolver_tx, resolver_rx) = mpsc::channel::<handler::Message<Digest>>(8);
                let mut marshal_handle = marshal_actor.start(
                    ForwardToExecutor(exec_mailbox),
                    NoopBuffer,
                    (
                        resolver_rx,
                        NoopResolver::<handler::Request<Digest>, PeerPubkey>::default(),
                    ),
                );
                let mut marshal = marshal_mailbox;

                // Contiguous finalized chain: heights 1..K-1 are pre-activation
                // (result must be zero), height K carries a forged result so the
                // executor's cross-check halts there.
                let mut parent = Digest(B256::ZERO);
                for h in 1..K {
                    let block = sample_order(parent, h, B256::ZERO);
                    parent = block.digest();
                    finalize_via_marshal(&mut marshal, &c, &block).await;
                }
                let forged = B256::repeat_byte(0xEE);
                assert_ne!(forged, fx.chain.spec_executed_hash(0).unwrap());
                let divergent = sample_order(parent, K, forged);
                let div_digest = divergent.digest();
                finalize_via_marshal(&mut marshal, &c, &divergent).await;
                wait_until(&ctx, "pre-K heights derived + acked", || {
                    fx.chain.spec_executed_hash(K - 1).is_some()
                })
                .await;

                // K+1 is dispatched into the halted executor to show the park
                // retains its ack instead of dropping it.
                let post_halt = sample_order(div_digest, K + 1, B256::ZERO);
                finalize_via_marshal(&mut marshal, &c, &post_halt).await;
                wait_until(&ctx, "SafetyHalt engaged", || fx.safety_halt.is_engaged()).await;
                ctx.sleep(Duration::from_millis(50)).await;

                // Liveness proof: a dead marshal (closed mailbox) returns `None`
                // from `get_block`, and a frozen one never answers at all.
                assert!(
                    (&mut marshal_handle).now_or_never().is_none(),
                    "the marshal actor must still be running after the SafetyHalt \
                     (a resolved handle = the Canceled-ack death)"
                );
                let served = marshal.get_block(Height::new(1)).await;
                assert_eq!(
                    served.map(|b| b.height),
                    Some(1),
                    "the halted node's marshal must still serve blocks by height"
                );

                // The diverged height is never acked, so nothing derives past it.
                assert!(
                    fx.chain.spec_executed_hash(K + 1).is_none(),
                    "no derive past the halted height"
                );
            });
        }

        /// The storage behavior boundary seeding relies on: an entry stored before
        /// the floor rises stays readable below it, while a below-floor write after
        /// the raise is dropped, so seeding must precede `set_floor`.
        #[test]
        fn injected_boundary_survives_set_floor_and_is_readable_below_floor() {
            let runtime = deterministic::Runner::default();
            runtime.start(|ctx| async move {
                let c = committee(11);
                let page_cache = CacheRef::from_pooler(
                    &ctx,
                    crate::outer::PAGE_CACHE_PAGE_SIZE,
                    crate::outer::PAGE_CACHE_CAPACITY,
                );
                let finalizations = crate::outer::init_finalizations_archive(
                    &ctx,
                    "seed-below-floor",
                    page_cache.clone(),
                )
                .await
                .expect("init finalizations archive");
                let blocks = crate::outer::init_finalized_blocks_archive(&ctx, "seed-below-floor")
                    .await
                    .expect("init finalized blocks archive");
                let module = crate::committee::testing::SchemeCommittee::new({
                    let verifier = c.verifier.clone();
                    move |_| Some(verifier.clone())
                });
                let provider = crate::outer::EpochSchemeProvider::new(module);
                let (marshal_actor, mut marshal, _last) = MarshalActor::init(
                    ctx.with_label("marshal"),
                    finalizations,
                    blocks,
                    commonware_consensus::marshal::Config {
                        provider,
                        epocher: crate::epocher::OriginEpocher::new(
                            0,
                            std::num::NonZeroU64::new(1_000).unwrap(),
                        ),
                        partition_prefix: "seed-below-floor".into(),
                        mailbox_size: 64,
                        view_retention_timeout: ViewDelta::new(100),
                        prunable_items_per_section: std::num::NonZeroU64::new(4_096).unwrap(),
                        replay_buffer: NZUsize!(1024),
                        key_write_buffer: NZUsize!(1024),
                        value_write_buffer: NZUsize!(1024),
                        block_codec_config: (),
                        max_repair: NZUsize!(20),
                        max_pending_acks: crate::outer::MAX_PENDING_ACKS,
                        page_cache,
                        strategy: Sequential,
                    },
                )
                .await;
                // An executor mailbox and resolver sender are held so neither
                // channel closes under the actor; nothing here reads either.
                let fx = Fixture::new(0);
                let (_executor, exec_mailbox) = fx.build(ctx.clone(), 0, 0);
                let (_resolver_tx, resolver_rx) = mpsc::channel::<handler::Message<Digest>>(8);
                let _handle = marshal_actor.start(
                    ForwardToExecutor(exec_mailbox),
                    NoopBuffer,
                    (
                        resolver_rx,
                        NoopResolver::<handler::Request<Digest>, PeerPubkey>::default(),
                    ),
                );

                let seeded = sample_order(Digest(B256::ZERO), 40, B256::ZERO);
                finalize_via_marshal(&mut marshal, &c, &seeded).await;

                marshal.set_floor(Height::new(100)).await;

                assert_eq!(
                    marshal.get_block(Height::new(40)).await.map(|b| b.height),
                    Some(40),
                    "an entry stored before the raise must stay readable below the floor"
                );

                let late = sample_order(Digest(B256::ZERO), 41, B256::ZERO);
                finalize_via_marshal(&mut marshal, &c, &late).await;
                assert!(
                    marshal.get_block(Height::new(41)).await.is_none(),
                    "a below-floor store attempted AFTER the raise must be dropped — if this \
                     starts passing, seeding no longer depends on ordering and the gate is gone"
                );
            });
        }

        /// Epoch geometry for the seeding tests: length-100 epochs from 0, so a
        /// floor of 997 buries the terminal 899 and the first block 900.
        fn seeding_epocher() -> crate::epocher::OriginEpocher {
            crate::epocher::OriginEpocher::new(0, std::num::NonZeroU64::new(100).unwrap())
        }

        /// Serves an authenticated pair for every height in `serve`, and nothing else.
        fn seam(c: &Committee, serve: Vec<u64>) -> crate::cert_follow::BoundaryFetchFn {
            let certs: std::collections::BTreeMap<u64, crate::cert_follow::UpstreamFinalized> =
                serve
                    .into_iter()
                    .map(|h| {
                        let block = sample_order(Digest(B256::ZERO), h, B256::ZERO);
                        let finalization = certify(c, &block);
                        (
                            h,
                            crate::cert_follow::UpstreamFinalized {
                                finalization,
                                block,
                            },
                        )
                    })
                    .collect();
            std::sync::Arc::new(move |height: u64, _at: B256| {
                let hit = certs.get(&height).cloned();
                Box::pin(async move { hit }) as futures::future::BoxFuture<'static, _>
            })
        }

        /// Both buried heights are seeded, and both strictly before the floor
        /// rises — the ordering the storage gate makes load-bearing.
        #[test]
        fn reseed_forward_injects_missing_boundary_pair_before_set_floor() {
            let runtime = deterministic::Runner::default();
            runtime.start(|ctx| async move {
                let c = committee(3);
                let landing = 1_000;
                let floor = landing - K;
                let fx = Fixture::new(0)
                    .with_epocher(seeding_epocher())
                    .with_boundary_fetch(seam(&c, vec![899, 900]));
                let (mut actor, _mailbox) = fx.build(ctx, 0, 0);

                actor
                    .reseed_forward(landing, B256::repeat_byte(0xAA), floor)
                    .await
                    .expect("reseed_forward");

                assert_eq!(
                    *fx.marshal.store_floor_order.lock().unwrap(),
                    vec![("store", 899), ("store", 900), ("floor", floor)],
                    "both boundary heights must be stored, and both before the floor rises"
                );
            });
        }

        /// Keyed on the condition, not the event: a node that already holds the
        /// pair does no fetch and no store, even though a jump just landed.
        #[test]
        fn reseed_forward_skips_present_boundary() {
            let runtime = deterministic::Runner::default();
            runtime.start(|ctx| async move {
                let c = committee(4);
                let landing = 1_000;
                let floor = landing - K;
                let fx = Fixture::new(0)
                    .with_epocher(seeding_epocher())
                    .with_boundary_fetch(seam(&c, vec![899, 900]));
                for h in [899, 900] {
                    fx.marshal
                        .canned
                        .lock()
                        .unwrap()
                        .insert(h, sample_order(Digest(B256::ZERO), h, B256::ZERO));
                }
                let (mut actor, _mailbox) = fx.build(ctx, 0, 0);

                actor
                    .reseed_forward(landing, B256::repeat_byte(0xAB), floor)
                    .await
                    .expect("reseed_forward");

                assert!(
                    fx.marshal.stored.lock().unwrap().is_empty(),
                    "nothing to seed when the boundary pair is already local"
                );
            });
        }

        /// Both-or-neither: seeding only the terminal would let the member promote
        /// at exactly the moment the value gate degrades to a no-op.
        #[test]
        fn reseed_forward_injects_neither_when_one_fetch_fails() {
            let runtime = deterministic::Runner::default();
            runtime.start(|ctx| async move {
                let c = committee(5);
                let landing = 1_000;
                let floor = landing - K;
                let fx = Fixture::new(0)
                    .with_epocher(seeding_epocher())
                    .with_boundary_fetch(seam(&c, vec![899]));
                let (mut actor, _mailbox) = fx.build(ctx, 0, 0);

                actor
                    .reseed_forward(landing, B256::repeat_byte(0xAC), floor)
                    .await
                    .expect("reseed_forward");

                assert!(
                    fx.marshal.stored.lock().unwrap().is_empty(),
                    "a partial pair must inject nothing"
                );
            });
        }

        /// The re-jump itself enters the landing epoch: the floor raise
        /// disqualifies the predecessor terminal from ever being dispatched, and a
        /// delivered boundary block is the only other entry edge.
        #[test]
        fn reseed_forward_enters_the_landing_epoch() {
            let runtime = deterministic::Runner::default();
            runtime.start(|ctx| async move {
                let c = committee(6);
                let landing = 1_050;
                let floor = landing - K;
                let entered = Arc::new(Mutex::new(Vec::new()));
                let fx = Fixture::new(0)
                    .with_epocher(seeding_epocher())
                    .with_boundary_fetch(seam(&c, vec![999, 1_000]))
                    .with_boundary_enter(entered.clone());
                let (mut actor, _mailbox) = fx.build(ctx, 0, 0);

                actor
                    .reseed_forward(landing, B256::repeat_byte(0xAD), floor)
                    .await
                    .expect("reseed_forward");

                assert_eq!(
                    *entered.lock().unwrap(),
                    vec![999],
                    "the entry names the terminal of the epoch preceding the landing's own"
                );
            });
        }

        /// Condition-keyed on the landing, not on whether seeding found anything:
        /// a node that already holds the pair seeds nothing and must still enter.
        #[test]
        fn reseed_forward_enters_even_when_the_boundary_is_already_present() {
            let runtime = deterministic::Runner::default();
            runtime.start(|ctx| async move {
                let c = committee(7);
                let landing = 1_050;
                let floor = landing - K;
                let entered = Arc::new(Mutex::new(Vec::new()));
                let fx = Fixture::new(0)
                    .with_epocher(seeding_epocher())
                    .with_boundary_fetch(seam(&c, vec![999, 1_000]))
                    .with_boundary_enter(entered.clone());
                for h in [999, 1_000] {
                    fx.marshal
                        .canned
                        .lock()
                        .unwrap()
                        .insert(h, sample_order(Digest(B256::ZERO), h, B256::ZERO));
                }
                let (mut actor, _mailbox) = fx.build(ctx, 0, 0);

                actor
                    .reseed_forward(landing, B256::repeat_byte(0xAE), floor)
                    .await
                    .expect("reseed_forward");

                assert!(
                    fx.marshal.stored.lock().unwrap().is_empty(),
                    "nothing to seed when the boundary pair is already local"
                );
                assert_eq!(
                    *entered.lock().unwrap(),
                    vec![999],
                    "the entry is not gated on seeding having stored anything"
                );
            });
        }

        /// The read floor published before the entry is the floor (result-final),
        /// not the landing, so the entry's first committee read resolves in the
        /// window the jump left this node with.
        #[test]
        fn reseed_forward_publishes_the_read_floor_before_entering() {
            let runtime = deterministic::Runner::default();
            runtime.start(|ctx| async move {
                let c = committee(9);
                let landing = 1_050;
                let floor = landing - K;
                let calls = Arc::new(Mutex::new(Vec::new()));
                let fx = Fixture::new(0)
                    .with_epocher(seeding_epocher())
                    .with_boundary_fetch(seam(&c, vec![999, 1_000]))
                    .with_boundary_read_floor(calls.clone())
                    .with_boundary_enter(calls.clone());
                let (mut actor, _mailbox) = fx.build(ctx, 0, 0);

                actor
                    .reseed_forward(landing, B256::repeat_byte(0xB0), floor)
                    .await
                    .expect("reseed_forward");

                assert_eq!(
                    *calls.lock().unwrap(),
                    vec![floor, 999],
                    "read floor = landing − K, published before the entry at terminal 999"
                );
            });
        }

        /// The entry keys on `terminal_at_or_below(landing)`; the seed keys on
        /// `terminal_at_or_below(floor)`. At a landing within K of an epoch start
        /// the two differ by a whole epoch, and the floor names the epoch just left.
        #[test]
        fn reseed_forward_entry_height_is_keyed_on_the_landing_not_the_floor() {
            let runtime = deterministic::Runner::default();
            runtime.start(|ctx| async move {
                let c = committee(8);
                // 1000 is the first block of epoch 10; the floor 997 still sits in epoch 9.
                let landing = 1_000;
                let floor = landing - K;
                let entered = Arc::new(Mutex::new(Vec::new()));
                let fx = Fixture::new(0)
                    .with_epocher(seeding_epocher())
                    .with_boundary_fetch(seam(&c, vec![899, 900]))
                    .with_boundary_enter(entered.clone());
                let (mut actor, _mailbox) = fx.build(ctx, 0, 0);

                actor
                    .reseed_forward(landing, B256::repeat_byte(0xAF), floor)
                    .await
                    .expect("reseed_forward");

                assert_eq!(
                    *fx.marshal.stored.lock().unwrap(),
                    vec![899, 900],
                    "the seed still keys on the FLOOR — this is what makes the two differ here"
                );
                assert_eq!(
                    *entered.lock().unwrap(),
                    vec![999],
                    "the entry keys on the LANDING; 899 would enter the epoch this node left"
                );
            });
        }
    }

    // The wake-up arm disarms on a closed beacon channel instead of spinning on it.
    #[tokio::test]
    async fn a_closed_beacon_channel_disarms_the_wake_up_arm_instead_of_spinning() {
        let (tx, mut rx) = tokio::sync::broadcast::channel(4);
        tx.send(crate::beacon::BeaconEvent::SeedRecorded).unwrap();
        assert_eq!(
            classify_seed_wake(&rx.recv().await),
            SeedWake::Derive,
            "a live seed record still fires the eager derive"
        );

        drop(tx);
        // The receiver is ready on every poll, twice in a row and without awaiting.
        let first = rx.recv().await;
        assert!(
            matches!(first, Err(tokio::sync::broadcast::error::RecvError::Closed)),
            "a dropped sender closes the receiver"
        );
        assert_eq!(classify_seed_wake(&first), SeedWake::Disarm);
        let second = rx.recv().await;
        assert!(
            matches!(
                second,
                Err(tokio::sync::broadcast::error::RecvError::Closed)
            ),
            "and stays closed — this is why the arm has to disarm, not `continue`"
        );
        assert_eq!(classify_seed_wake(&second), SeedWake::Disarm);
    }

    // The other two event classes are wake-ups for other consumers of the same
    // channel: no derive, and the arm stays armed.
    #[test]
    fn the_wake_up_arm_ignores_the_other_two_event_classes_and_derives_on_lagged() {
        assert_eq!(
            classify_seed_wake(&Ok(crate::beacon::BeaconEvent::KeyAvailable)),
            SeedWake::Ignore
        );
        assert_eq!(
            classify_seed_wake(&Ok(crate::beacon::BeaconEvent::ParticipationChanged)),
            SeedWake::Ignore
        );
        assert_eq!(
            classify_seed_wake(&Err(tokio::sync::broadcast::error::RecvError::Lagged(3))),
            SeedWake::Derive,
            "a dropped run of events is a wake-up like any other; the re-check is idempotent"
        );
    }
}
