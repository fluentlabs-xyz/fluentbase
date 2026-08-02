//! Executor: drives the reth EL from ordering-finalized [`OrderBlock`]s —
//! derive → execute (import via `new_payload`) → two-tier FCU.
//!
//! Three-tier forkchoice: `head` follows the locally derived (speculative)
//! executed tip; `safe` rides the BFT ORDERING-finalized tip (~0 lag,
//! content-immutable the moment it is finalized); `finalized` follows RESULT
//! finality = `ordering_finalized − K` (clamped to the cold-start anchor), i.e.
//! the height whose derived hash the committee has attested by agreeing the
//! OrderBlock K heights above it. Invariant `finalized ⊆ safe ⊆ head` holds at
//! every FCU (result-final ⊆ ordering-final ⊆ speculative tip, one chain).
//!
//! Derive pipeline (Design B′, the seed witness): the beacon seed for height
//! `h` rides in block `h+1`'s `parent_seed` (the witness), so `h` is derived at
//! the DISPATCH of `h+1`, from the child body already in hand. The executor
//! holds at most ONE delivered-but-underived finalized block — the tip, in
//! [`Actor::awaiting_child`] — and derives it the moment its successor arrives.
//! No park, no re-poke, no by-height re-fetch, no timer; the executor never
//! reads a certificate on the derive path.
//!
//! Ack flow: the marshal's `Exact` ack fires only after derive + import, so
//! marshal backpressure (MAX_PENDING_ACKS) IS execution backpressure.
//!
//! Ack invariant (normative): the executor NEVER acknowledges a block it has not
//! derived, and NEVER drops an `Exact` while the marshal is alive — a dropped
//! `Exact` cancels, and the marshal treats a Canceled ack as fatal (its `run`
//! returns), killing the component that serves blocks + certs to peers. Every
//! ack is therefore (a) acknowledged after derive+import, (b) held in
//! [`Actor::awaiting_child`] until the child arrives, (c) parked with a
//! deferred block (guard #2's absent `h+K` body — the only park), or (d)
//! RETAINED un-resolved forever by [`Actor::park_halted`] when a Phase-3
//! `SafetyHalt` engages — the halt posture is "stop participating, stay
//! observable", so progress stops (the marshal's `last_processed_height`
//! freezes) while the marshal keeps serving peers. The sole path that disposes a
//! parked/held ack differently is `reseed_forward`, which `acknowledge()`s it
//! because the floor MOVES past the parked height (pruned, not skipped). At
//! SHUTDOWN the held ack is dropped DELIBERATELY: the executor and the marshal
//! die together at the runtime drop (a dropped task is never polled again, so
//! the marshal's fatal ack arm cannot observe the cancellation), and the
//! withheld ack IS the restart self-heal — `last_processed_height` advances
//! only on `Ok`, so the restarted marshal re-dispatches the held height and it
//! derives on the next run. Acking it at shutdown would durably skip it forever.

use crate::digest::Digest;
use crate::{
    application::{BeaconEngineLike, DerivedBlock as _, DerivedBlockBuilder, ExecutedChain},
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
use commonware_runtime::{
    spawn_cell, Clock, ContextCell, FutureExt as _, Handle, Metrics as _, Pacer, Spawner,
};
use commonware_utils::{
    acknowledgement::Exact, channel::oneshot, futures::OptionFuture, vec::NonEmptyVec,
    Acknowledgement as _,
};
use eyre::{ensure, WrapErr as _};
use fluentbase_bls::PeerPubkey;
use fluentbase_bls::Scheme as BlsScheme;
use futures::{
    future::{ready, BoxFuture, Ready},
    stream::FuturesOrdered,
    FutureExt as _, StreamExt as _,
};
use prometheus_client::metrics::gauge::Gauge;
use std::{collections::BTreeMap, ops::RangeInclusive, pin::Pin, time::Duration};
use tokio::{select, sync::mpsc};
use tracing::{debug, error, error_span, info, info_span, instrument, warn, Level, Span};

/// One executor command paired with its tracing span (preserves the causal
/// `parent` for `#[instrument]`).
pub struct Message {
    pub cause: Span,
    pub command: Command,
}

pub enum Command {
    /// Derive + import a finalized ordering artifact (`Update::Block`) or
    /// refresh the catch-up target (`Update::Tip`).
    Finalize(Box<Update<OrderBlock>>),
    /// A block was NOTARIZED (round-1 quorum) — speculatively derive + import
    /// it now, ahead of finalization, to hide execution latency under the
    /// finalization rounds. Best-effort: `try_derive` (finalized path) stays the sole
    /// authority and reconciles (skip-if-matched / re-derive + reorg). Boxed to
    /// keep the enum small (mirrors `Finalize`).
    SpecNotarized(Box<Notarized>),
}

/// Payload of [`Command::SpecNotarized`]: the ordering digest + the seed
/// recovered from the Notarization certificate (the round rides in
/// `seed.target_round`). The block body is fetched from the marshal by digest
/// at execution time.
pub struct Notarized {
    pub digest: crate::digest::Digest,
    pub seed: Option<crate::beacon::seed::Seed>,
}

/// Value stored per speculatively-executed height in [`Actor::spec_executed`]:
/// the notarized ordering DIGEST, the ROUND of the seed the speculation was
/// derived with (`None` on a no-beacon, seed-independent height), and the EVM
/// hash of the PARENT the block was speculatively executed against. The
/// finalized-path reconcile ([`Actor::try_derive`]'s `correctly_speculated`)
/// keeps the speculation only when ALL THREE match the finalized fork — same
/// ordering block, same seed round, AND parent-linked to the block that is
/// canonical at `height − 1` NOW. After the §4.1 re-canonicalisation the round
/// sides are `Round::new(Ep, block.proposal_view)` — a pure function of the same
/// agreed block — so a digest match with a DIFFERENT round is an ANOMALY, not
/// routine churn: it is counted (`dpos_spec_round_mismatch_total`, expected 0)
/// and the block re-derives from the witness (the agreed value). The digest half
/// stays a real branch (a speculated sibling that lost to a nullify/re-propose).
/// The `parent_hash` half guards the deep-speculation reorg: a head rollback at
/// `height − 1` re-derives the parent to a DIFFERENT hash, so a speculated block
/// still present at `height` was executed against a now-orphaned parent (wrong
/// pre-state) and MUST re-derive — the same fork-safety family as the spec-seed-
/// blind divergence. (Rollback also proactively invalidates the suffix; this
/// check is the belt-and-suspenders that also catches a stale parent with no
/// rollback event.)
#[derive(Clone)]
struct SpecExecuted {
    digest: crate::digest::Digest,
    seed_round: Option<commonware_consensus::types::Round>,
    parent_hash: B256,
}

/// What caused a [`Actor::try_eager_finalized_derive`] attempt — used only to
/// pick the metric label so the record-vs-delivery race stays observable.
///
/// - `Delivery`: the attempt made when the block is delivered/held (the normal
///   record-lag closer). A miss here is the transient race and is counted
///   (`outcome="miss"`); a hit is `outcome="hit"`.
/// - `Notified`: an event-driven re-attempt fired by the executor's seed-notify
///   `select!` arm ([`SeedStore::notifier`]) when a block is still held and a
///   seed was just recorded (the seed for its round may have JUST landed). A hit
///   here is counted `outcome="recovered"` — the race fired and self-healed
///   WITHOUT a further finalized delivery (the deadlock-breaker, since finality
///   only advances via new blocks). A miss is a silent no-op. Replaces the
///   former `SpecNotarized` Poke: the notify permit makes the re-attempt correct
///   regardless of mailbox ordering, with no lost-notification window.
#[derive(Clone, Copy)]
enum EagerTrigger {
    Delivery,
    Notified,
}

/// A notarized speculative block PARKED (rather than dropped) because it arrived
/// ahead of `spec_head` (a gap) or before its parent had executed. Holds exactly
/// the data [`Command::SpecNotarized`] carries so [`Actor::try_drain_parked`] can
/// re-drive [`Actor::spec_execute`] verbatim once `spec_head` catches up.
///
/// Parking restores the speculation invariant: once the executor falls behind
/// (e.g. after a sibling-nullification rollback) an incoming notarization for a
/// height beyond `spec_head + 1` would otherwise be dropped forever, and
/// speculation would stay dead until finalization independently caught the tip
/// up — the death spiral. Overwrite-by-height is deliberate: a later-view sibling
/// notarization at the same height replaces the earlier guess (a wrong guess is
/// safe — `correctly_speculated` reconciles it at finalization).
#[derive(Clone)]
struct ParkedSpec {
    digest: crate::digest::Digest,
    seed: Option<crate::beacon::seed::Seed>,
}

#[derive(Clone)]
pub struct Mailbox {
    tx: mpsc::UnboundedSender<Message>,
}

impl Mailbox {
    fn new(tx: mpsc::UnboundedSender<Message>) -> Self {
        Self { tx }
    }

    /// Test-only constructor used by `application.rs` unit tests to inject a
    /// drain-only mailbox without spawning a real executor.
    #[cfg(test)]
    pub(crate) fn new_for_test(tx: mpsc::UnboundedSender<Message>) -> Self {
        Self { tx }
    }

    /// Sync send — `tokio::sync::mpsc::UnboundedSender::send` never blocks.
    // SendError<Message> carries the rejected message verbatim so the
    // caller can retry; boxing solely to silence the lint would add an
    // alloc on the hot path.
    #[allow(clippy::result_large_err)]
    pub fn send(&self, msg: Message) -> Result<(), mpsc::error::SendError<Message>> {
        self.tx.send(msg)
    }
}

// LastCanonicalized — monotonic projection of forkchoice state.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LastCanonicalized {
    forkchoice: ForkchoiceState,
    head_height: Height,
    /// Ordering-final tier (the BFT cert tip): the engine-API `safe` tag. Its
    /// OWN monotone guard, distinct from `finalized_height` — `safe` rides the
    /// just-finalized ordering tip (~0 lag) while `finalized` lags by K (the
    /// committee-attested result). Invariant: `finalized ⊆ safe ⊆ head`.
    safe_height: Height,
    finalized_height: Height,
}

impl LastCanonicalized {
    /// Result-final tier (committee-attested execution, `ordering − K`). Sets
    /// ONLY `finalized` — `safe` is the ordering-final tier, advanced by
    /// `update_safe`. The `head >=` clause is kept so a finalized delivery with
    /// no speculative lead still pushes `head` (mirrors `update_safe`).
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
    /// Guard is `height >= self.safe_height` (NOT strict `>`), mirroring
    /// `update_head`'s finalized-fork allow: monotone in HEIGHT (never rolls
    /// backward) but lets the HASH FOLLOW a same-height re-finalization. A
    /// same-height sibling reorg (`height == safe_height`) re-pins `safe` to
    /// the freshly-finalized canonical hash the caller passes; a strict `>`
    /// would pin `safe` to an orphaned sibling after `head` reorgs away from it
    /// → `safe ⊄ head` → reth `-38002` (or a silent orphan-`safe`). Do NOT
    /// tighten to `>`.
    ///
    /// A `height < safe_height` delivery is a NO-OP, and this is LEGITIMATE (not
    /// asserted against): a deep-catch-up follower's `init` seeds `safe_height`
    /// at the cold-start anchor (the live frontier), then the executor derives
    /// the K blocks BELOW that anchor (marshal floor = `anchor − K`). Those
    /// below-anchor finalized deliveries call `update_safe` with a height below
    /// the seeded `safe_height`; the no-op keeps `safe` at the anchor (it must
    /// not roll back below where the node trust-anchored).
    ///
    /// Touches ONLY `safe_*`: `head` is owned by `update_finalized`'s head
    /// clause + `update_head`, so there is no `update_safe`-vs-`update_head`
    /// head-write interaction in the rollback path (D1/D13).
    fn update_safe(mut self, height: Height, hash: B256) -> Self {
        if height >= self.safe_height {
            self.safe_height = height;
            self.forkchoice.safe_block_hash = hash;
        }
        self
    }

    fn update_head(mut self, height: Height, hash: B256) -> Self {
        // A lower-height head on the finalized fork (a legitimate reorg of an
        // unfinalized tail — e.g. the migration cold-start where reth's head
        // sits on an orphaned tail) MUST be allowed to roll the head back.
        if height > self.finalized_height || hash == self.forkchoice.finalized_block_hash {
            self.head_height = height;
            self.forkchoice.head_block_hash = hash;
        }
        self
    }
}

// BlockFetcher — minimal trait so we don't depend on the full marshal Mailbox type.

pub trait BlockFetcher: Clone + Send + Sync + 'static {
    fn fetch_block_by_height(
        &self,
        height: Height,
    ) -> impl std::future::Future<Output = Option<OrderBlock>> + Send;

    /// Best-effort LOCAL lookup of a block by its ordering digest. Used by the
    /// speculative path: at notarization the body is in the marshal buffer (we
    /// voted on it), so a `None` simply means "not local yet" → skip
    /// speculation (the finalized path will derive it).
    fn fetch_block_by_digest(
        &self,
        digest: crate::digest::Digest,
    ) -> impl std::future::Future<Output = Option<OrderBlock>> + Send;

    /// Ask peers for the finalization at `height` (fills `finalizations_by_height`
    /// durably). Fire-and-forget; the marshal skips it if already local.
    fn hint_finalization(
        &self,
        height: Height,
        targets: NonEmptyVec<PeerPubkey>,
    ) -> impl std::future::Future<Output = ()> + Send;

    /// Advance the RUNNING marshal's in-order dispatch floor to `height` (prunes
    /// below + resumes contiguous dispatch from `floor + 1`). Raises-only.
    /// Threaded onto the trait seam (the concrete `Mailbox::set_floor` lives on
    /// the marshal mailbox) so the steady-state re-jump can re-seed a running
    /// marshal and the test [`FakeMarshal`] can record the call.
    fn set_floor(&self, height: Height) -> impl std::future::Future<Output = ()> + Send;

    /// Store an already-authenticated finalization+block through the sanctioned
    /// inlet ingress (`verified`, then `report(Finalization)`).
    ///
    /// Exists so a caller can seed an entry BEFORE raising the floor past it. The
    /// ordering is the caller's responsibility and it is load-bearing: these two
    /// messages and [`Self::set_floor`] share one mailbox that the marshal drains a
    /// message per loop turn, and the below-floor write gate is evaluated at
    /// message-processing time — so a store enqueued first lands and stays readable
    /// forever, while one enqueued after `set_floor` is dropped.
    fn store_verified_finalization(
        &self,
        round: Round,
        block: OrderBlock,
        finalization: Finalization<BlsScheme, Digest>,
    ) -> impl std::future::Future<Output = ()> + Send;
}

/// Explicit impl for the concrete marshal mailbox.
/// Orphan rule OK — BlockFetcher local, Mailbox foreign.
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

/// Idle cadence of the frozen-tip frontier probe ([`ReJump::probe`]) — the
/// discovery tick for a node whose marshal tip stopped advancing. One
/// `get_latest` resolver fetch against one peer, far under the 16/s
/// frontier-channel quota, and skipped entirely while the tip advances via
/// consensus or an inlet.
const FRONTIER_PROBE_INTERVAL: Duration = Duration::from_secs(1);

/// Fast catch-up cadence: while probes are actively discovering new frontier
/// heights (a demoted plane-native validator in steady-state live-follow), the
/// probe re-arms at this interval so the node trails the chain by ~one RTT
/// rather than a whole idle tick — byte-identical multi-node convergence (and
/// operator finalized-lag expectations) match the WS-inlet PUSH path only when
/// the pull loop is this tight. 5/s per node, still well under the 16/s
/// per-peer quota.
const FRONTIER_PROBE_INTERVAL_FAST: Duration = Duration::from_millis(200);

/// How many probe ticks stay fast after the last PRODUCTIVE probe (one that
/// hinted a new frontier). 15 ticks ≈ 3 s at the fast cadence — enough to
/// bridge the ~1 blk/s arrival gaps so a continuously-following node never
/// decays to the idle tick between blocks, while a truly caught-up or wedged
/// node decays within ~3 s.
const FRONTIER_PROBE_FAST_BURST: u8 = 15;

/// Backoff between transient engine-API TRANSPORT retries at the finalize FCU
/// (#14 self-heal). Short — an RPC/channel blip clears in ms; the durable-stuck
/// signal is the `dpos_sync_degraded{reason=engine_retry}` gauge, not a crash
/// (Decision A: never `process::exit` on an external/correlated cause).
const ENGINE_TRANSPORT_RETRY_BACKOFF: Duration = Duration::from_millis(200);

/// Returns the current committee's peers to target for a finalization re-fetch,
/// or `None` if no committee is known yet. Re-invoked per retry so it tracks the
/// catch-up walk's advancing epoch.
pub type PeersForFinalization =
    std::sync::Arc<dyn Fn() -> Option<NonEmptyVec<PeerPubkey>> + Send + Sync>;

/// Steady-state self-healing re-jump callback. Invoked from the `Update::Tip`
/// arm when the marshal's frontier runs > [`crate::cold_start_jump::JUMP_THRESHOLD`]
/// finalized blocks ahead of the highest derived ordering height (the upstream's
/// serving window is exactly that wide, so beyond it `UpstreamResolver::fetch`
/// returns nothing forever → the marshal floor freezes → the executor wedges).
/// The callback re-runs the SAME forward-only, BLS-verified
/// [`crate::cold_start_jump::cold_start_jump`] the cold-start path uses, fast-
/// forwarding reth via one FCU + devp2p backfill.
///
/// The generics of the underlying `cold_start_jump` (upstream / committee source
/// / EL-sync) are ERASED behind this boxed `Fn` so the executor [`Actor`] gains
/// NO new generic params. The executor SPAWNS the future as a READ-ONLY waiter
/// (the same spawned-fetch idiom the inlet uses) and reacts to its terminal
/// [`crate::cold_start_jump::JumpOutcome`] on a `oneshot` `select!` arm — NOT an
/// in-task poll. The jump's only reth touch is the read-side `sync_to` FCU, which
/// reth ancestor-skips when backward, so the spawned waiter cannot corrupt the
/// executor's own forward FCUs.
///
/// The `u64` argument is the trigger's `from` = the executor's current
/// `ordering_finalized`. It returns the typed terminal
/// [`crate::cold_start_jump::JumpOutcome`] (the spawn owns the whole backfill
/// wait, so there is no in-progress variant): `Landed` ⇒ re-seed + advance the
/// running marshal floor; `Lagging` ⇒ no-op; `Stalled` ⇒ NON-fatal transport
/// stall (re-evaluated on the next `Update::Tip`); `AuthFailed` ⇒ NON-fatal (#1
/// self-heal): a forged far-ahead target fails `verify_jump_authenticated`, so the
/// executor ROTATES the upstream + stays up-degraded (`reason=auth_rotate`) and
/// re-jumps onto the honest source on the next tip — it never serves the forged
/// branch and never crashes (Decision A).
pub type ReJumpFn = std::sync::Arc<
    dyn Fn(u64) -> BoxFuture<'static, crate::cold_start_jump::JumpOutcome> + Send + Sync,
>;

/// Erased `CertUpstream::get_latest().map(|uf| uf.block.height)` — the upstream
/// frontier-discovery probe the executor fires when the marshal tip FREEZES (see
/// [`ReJump::probe`]). Trusted height-only, exactly like
/// [`ReJump::upstream_frontier`]: a lying peer can at most inflate the frontier
/// (a wasted hint fetch / a re-jump that fails closed at
/// `verify_jump_authenticated`).
pub type FrontierProbeFn =
    std::sync::Arc<dyn Fn() -> BoxFuture<'static, Option<Height>> + Send + Sync>;

/// Erased "this node's history now begins here" publisher: raises
/// `EpochTransition`'s read-height floor to the `u64` argument. The re-jump
/// landing's twin of [`Config::boundary_enter`], and AWAITED where that one is
/// fire-and-forget — the floor must be in place before the entry's first committee
/// read, and the state machine sits behind an async mutex, so a spawn would race
/// the very entry this unblocks.
pub type BoundaryReadFloorFn = std::sync::Arc<dyn Fn(u64) -> BoxFuture<'static, ()> + Send + Sync>;

/// The steady-state re-jump callback bundled with the signal its trigger reads.
#[derive(Clone)]
pub struct ReJump {
    /// The forward-only, BLS-verified jump (the `u64` arg is `from` =
    /// `ordering_finalized`); see the callback notes above.
    pub call: ReJumpFn,
    /// The TRUE upstream frontier, advanced by the cert-inlet on EVERY received
    /// cert (pre-verify, height-only) — see
    /// [`crate::cert_inlet::LiveFrontierTee::upstream_frontier`]. The marshal's
    /// STORED frontier (`last_tip_height`, fed by `Update::Tip`) FREEZES during
    /// the "committee[E] not committed" defer deadlock — the inlet keeps
    /// receiving certs but stores none, so no `Update::Tip` fires — which is
    /// exactly when the re-jump is needed. The trigger therefore measures the gap
    /// against `max(marshal tip, upstream_frontier)` so a frozen marshal tip can
    /// never mask a real deep gap. HEIGHT-ONLY: it MUST NOT feed the committee
    /// read (that stays the verified-only `live_height`), else a malicious
    /// upstream could steer committee selection. SHARED between the inlet (writer)
    /// and this re-jump (reader) on BOTH the follower AND the validator-with-upstream
    /// (Rule Y symmetry — the inlet-fed joiner advances it on every cert); a
    /// no-upstream validator has no `ReJump` at all (this field never exists there).
    pub upstream_frontier: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Forward-only re-jump need-gate, mirrored into the spawned jump's own gate
    /// (`cold_start_jump_with_threshold`). The defer deadlock is EPOCH-relative
    /// ("≥2 epochs behind ⇒ `committee[E]` uncommitted"), so its recovery gate is
    /// epoch-relative too: `min(JUMP_THRESHOLD, epoch_block_interval)` (real-prod
    /// epochs ≫ 1024 keep the 1024 serving-window size; a compressed test epoch
    /// scales down to ~1 epoch so a short-epoch node heals within an epoch instead
    /// of waiting for a fixed 1024-block gap).
    ///
    /// BOTH node kinds use the epoch-relative value — the follower and the
    /// validator-with-upstream alike (`dpos.rs` computes the same
    /// `JUMP_THRESHOLD.min(interval)` on each path). Only the tests construct a
    /// bare `JUMP_THRESHOLD`. The COLD-START jump is different and keeps the fixed
    /// `JUMP_THRESHOLD` (`cold_start_jump` vs `cold_start_jump_with_threshold`),
    /// which is why a short restart gap reaches this gate but not that one.
    pub threshold: u64,
    /// The inlet's EXISTING upstream-rotation escape ([`crate::cert_inlet::RotateUpstream`]),
    /// the SAME `CertUpstream::rotate_callback()` the data-fault inlet uses. Fired
    /// when the re-jump's terminal outcome is a fault (Rule L): `BadTarget` (forgeable
    /// structural mismatch), `InvalidTarget` (reth-rejected mid-sync), and `AuthFailed`
    /// (#1 — a forged/unagreed POST-sync branch) rotate immediately; `Stalled` rotates
    /// after `MAX_UPSTREAM_FAULTS` (an honest transient stall must not insta-rotate).
    /// `Option` so unit tests / a no-rotate config leave it `None`.
    pub rotate: Option<crate::cert_inlet::RotateUpstream>,
    /// Upstream frontier-discovery probe, fired from the executor's 1 s probe
    /// tick whenever `last_tip_height` did NOT advance since the previous tick
    /// (the marshal is learning no new finalizations). This is the LIVE-FOLLOW
    /// driver for a validator with no cert-inlet (the plane-native default): a
    /// ROTATED-OUT validator participates in no consensus and has no inlet, so
    /// without the probe its marshal tip freezes at the demotion boundary — no
    /// `Update::Tip`, no prehints, a frontier atomic stuck at 0 (the inlet was
    /// its only writer) ⇒ a permanent silent wedge. The probe feeds
    /// `upstream_frontier` (deep gaps → the re-jump trigger) and
    /// `hint_finalization(frontier)` (small gaps → the marshal fetches, verifies
    /// against the epoch scheme, and the normal `Update::Tip` pipeline walks the
    /// gap). Self-silencing while live: an advancing tip skips the network probe
    /// entirely. `None` in unit tests.
    pub probe: Option<FrontierProbeFn>,
}

/// A finalized block PARKED by guard #2 (the node is ≥ K behind — `last_tip
/// >= h + K` — but the committee-attested body at `h + K` is not backfilled
/// yet, so the convergence check cannot run). The ONLY park in the executor.
/// The witness seed was already in hand when the park was taken, so the
/// re-poke re-derives with ZERO lookups. The `pending_finalizations` drain is
/// paused while this is `Some`, which preserves strict derive order and lets
/// the marshal's `MAX_PENDING_ACKS` backpressure bound the queue. Event-driven:
/// re-poked by the marshal's live `Update::Tip`/`Update::Block` delivery stream
/// + the FCU heartbeat — NEVER a wall-clock give-up (§8.11).
struct Deferred {
    cause: Span,
    order: OrderBlock,
    ack: Exact,
    /// The witness (block `h+1`'s `parent_seed`), retained with the park so
    /// `repoke_deferred` is a plain "is `h + K`'s body here yet" retry.
    seed: Option<crate::beacon::seed::Seed>,
}

/// Result of attempting to derive a finalized block.
enum DeriveOutcome {
    /// Derived + imported + FCU'd + acked.
    Done,
    /// Guard #2 could not run: the node is ≥ K behind but the committee-attested
    /// body at `height + K` is not backfilled yet. The park payload (block, ack
    /// AND the already-resolved witness) is handed back to be PARKED + re-poked
    /// event-driven; boxed to keep the hot `Done` arm small.
    NeedAttestation(Box<Deferred>),
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
    /// The marshal floor this node boots with — the SAME value `outer.rs` sends in
    /// its buffered `SetFloor`. Seeds the stale-dispatch guard so it is live from
    /// tick zero rather than from the first `reseed_forward`.
    pub initial_marshal_floor: u64,
    /// Authenticated by-height seam for boundary seeding at `reseed_forward`.
    pub boundary_fetch: Option<crate::cert_follow::BoundaryFetchFn>,
    /// Epoch-entry seam — see [`crate::outer::OuterBuilder::boundary_enter`]. Invoked once
    /// per successful re-jump landing.
    pub boundary_enter: std::sync::Arc<dyn Fn(u64) + Send + Sync>,
    /// Read-floor seam — see [`crate::outer::OuterBuilder::boundary_read_floor`]. Awaited
    /// immediately BEFORE [`Self::boundary_enter`] on a re-jump landing.
    pub boundary_read_floor: BoundaryReadFloorFn,
    /// Chain-wide sequencer→DPoS activation block — the origin of the
    /// `result_target` pre-activation window (`height < activation + K` ⇒
    /// `result` MUST be ZERO). A CHAIN constant, NOT this node's cold-start
    /// anchor: a deep-catch-up follower trust-anchors at the live frontier
    /// (`initial_finalized` ≫ activation) yet still derives the K-below-anchor
    /// blocks, which are post-activation and carry real (non-zero) results.
    /// Keying the cross-check on the anchor would mis-classify those as
    /// pre-activation and reject the chain.
    pub dpos_activation_block: u64,
    pub fcu_pace: Duration,
    pub peers_for_finalization: PeersForFinalization,
    /// Shared `round → recovered seed` map (cross-epoch singleton from
    /// `outer.rs`), for re-canonicalising the SPECULATIVE seed round to the
    /// block's own `proposal_view` (the witness rollout). `None` in tests /
    /// followers ⇒ the re-canonicalise arm degrades to "skip speculation on a
    /// spin-round notarization" (never speculate with a known-wrong seed).
    pub seed_store: Option<crate::beacon::certify::SeedStore>,
    /// Cross-epoch block→epoch map (the same singleton threaded into marshal +
    /// `epoch_manager`, `outer.rs`). Used ONLY by the eager finalized-derive
    /// path to form `h`'s own seed round `Round(epocher.containing(h).epoch(),
    /// h.proposal_view)` — the EXACT round the child witness carries (matches
    /// `application::witness_link`'s `parent_epoch`), so a `SeedStore` hit is
    /// byte-identical to the child witness and a wrong epoch can only MISS.
    pub epocher: crate::epocher::OriginEpocher,
    /// Beacon counters (cross-epoch singleton from `dpos.rs::launch`). The
    /// executor increments `seed_active` / `digest_fallback` per derived block.
    pub beacon_metrics: crate::beacon::metrics::BeaconMetrics,
    /// Self-heal observability handle (cross-launch singleton from
    /// `dpos.rs::launch`, already registered there). The executor raises
    /// `dpos_sync_degraded{reason=engine_retry}` while retrying a transient
    /// engine-API TRANSPORT error at the finalize FCU (#14) and
    /// `{reason=auth_rotate}` while rotating/backing-off after a steady-state
    /// re-jump `AuthFailed` (#1).
    pub sync_metrics: SyncMetrics,
    /// Fork-safety latch (Phase 3). The executor ENGAGES it on #2/#3 result
    /// divergence, #15 an EL `Ok(Invalid)` verdict, and #10 an L1-fork re-jump —
    /// halting instead of extending a rejected branch. Engaging stops the executor
    /// driving reth, demotes the node to verify-only permanently (never
    /// re-promoted), and keeps marshal + `consensus`-RPC alive (the OuterEngine
    /// supervisor parks rather than abort-all). Cross-launch singleton from
    /// `dpos.rs::launch`, shared with `epoch_manager` + the supervisor.
    pub safety_halt: crate::sync_metrics::SafetyHalt,
    /// Fired on every ordering-finalized advance so [`crate::epoch_manager`] can
    /// re-poke a per-epoch engine spawn parked on the `Inline::genesis(E)`
    /// precondition (the E-1 boundary block landing in marshal storage IS an
    /// executor finalized-advance). Event-driven re-poke, no clock poll.
    pub spawn_unblocked: std::sync::Arc<tokio::sync::Notify>,
    /// Steady-state self-healing re-jump (see [`ReJump`]). `Some` on any
    /// upstream-configured node (follower or validator-with-upstream); `None`
    /// for a plain validator (it catches up on the consensus-plane treadmill)
    /// and in tests that do not exercise the re-jump.
    pub re_jump: Option<ReJump>,
}

pub struct Actor<E, BE, D, XC, MarshalMailbox> {
    context: ContextCell<E>,
    beacon_engine: BE,
    deriver: D,
    executed: XC,
    marshal: MarshalMailbox,
    mailbox: mpsc::UnboundedReceiver<Message>,
    beacon_metrics: crate::beacon::metrics::BeaconMetrics,
    /// Self-heal stuck-detector (see [`Config::sync_metrics`]).
    sync_metrics: SyncMetrics,
    /// Fork-safety latch (see [`Config::safety_halt`]).
    safety_halt: crate::sync_metrics::SafetyHalt,
    spawn_unblocked: std::sync::Arc<tokio::sync::Notify>,
    /// Steady-state self-healing re-jump callback (see [`ReJump`]). Fired from
    /// the `Update::Tip` arm when the frontier runs > `JUMP_THRESHOLD` ahead of
    /// `ordering_finalized`.
    re_jump: Option<ReJump>,
    /// Consecutive steady-state re-jump `Stalled` outcomes since the last reset. At
    /// `MAX_UPSTREAM_FAULTS` the executor `rotate()`s (Rule L) and resets. Reset to 0
    /// on ANY executor-side `rotate()` (incl. the `BadTarget` arm) and on
    /// `Landed`/`Lagging` (progress / upstream agrees we're caught up) — so a streak
    /// accrued on URL A NEVER carries into URL B (critic r2: cross-URL carryover →
    /// A→B→A oscillation). A SECOND, independent streak from the inlet's data-fault
    /// counter ([`crate::cert_inlet::CertInlet`]); both feed the SAME `rotate()` sink,
    /// deduped at the WS actor.
    rejump_fault_streak: u32,
    /// Completion channel of the in-flight spawned re-jump waiter. `Some` ⇒ a
    /// re-jump is running; its terminal [`crate::cold_start_jump::JumpOutcome`]
    /// is consumed in a dedicated `select!` arm (mirror of `pending_backfill`'s
    /// OptionFuture + manual clear). The waiter is SPAWNED (a read-only `sync_to`
    /// wait), so the executor's `select!` loop stays responsive during the
    /// multi-minute backfill.
    jump_done: OptionFuture<oneshot::Receiver<crate::cold_start_jump::JumpOutcome>>,
    /// Handle of the spawned re-jump waiter, aborted on shutdown so the spawned
    /// `sync_to` wait does not outlive the executor task.
    jump_handle: Option<Handle<()>>,
    /// Highest marshal-frontier height observed via `Update::Tip`. The FCU
    /// heartbeat re-pokes `maybe_re_jump` with THIS height so a re-jump whose
    /// transport `Stalled` (or whose reth backfill stalled) is re-evaluated even
    /// if the upstream frontier has plateaued (no further `Update::Tip` to
    /// re-trigger it). Without the heartbeat re-poke the Stalled retry depends
    /// solely on the next tip → a plateaued frontier is a silent permanent wedge.
    last_tip_height: Height,

    /// The RUNNING marshal floor (landing − K), mirrored from the last
    /// `reseed_forward`'s `set_floor`. `0` on every non-jump path (inert). Guards
    /// the `Update::Block` arm against the stale-backlog ESCAPE: `set_floor` is
    /// fire-and-forget, and each disposal `acknowledge()` in `reseed_forward` frees
    /// a marshal slot whose biased select can dispatch the next OLD-range block into
    /// the executor mailbox BEFORE the marshal processes `SetFloor`. Such escaped
    /// `≤ floor` deliveries are acked-without-derive here (the marshal already
    /// pruned them; deriving against `db_tip = landing` is the deep-overlay walk the
    /// jump exists to avoid) rather than parked on a pruned `h + K` (permanent
    /// deferred park) or re-fetched to a jump-pruned child (witness-gap fatal).
    /// Keyed STRICTLY on the marshal floor, never on `anchor`/`safe_height`: legit
    /// below-safe deliveries in the `anchor − K + 1 ..= anchor` deep-catch-up window
    /// must still derive.
    marshal_floor: u64,
    /// Authenticated by-height seam for seeding an epoch-boundary block the floor is
    /// about to bury (`reseed_forward`). `None` without an upstream.
    /// Used by `seed_boundary_below_floor` to locate the epoch terminal a floor
    /// raise would bury; the pre-existing `epocher` field below supplies the
    /// geometry, so seeding can never disagree with the gate it satisfies.
    boundary_fetch: Option<crate::cert_follow::BoundaryFetchFn>,
    /// Epoch-entry seam — see [`crate::outer::OuterBuilder::boundary_enter`].
    boundary_enter: std::sync::Arc<dyn Fn(u64) + Send + Sync>,
    /// Read-floor seam — see [`crate::outer::OuterBuilder::boundary_read_floor`].
    boundary_read_floor: BoundaryReadFloorFn,

    last_canonicalized: LastCanonicalized,
    /// Highest ordering-finalized height processed; drives the result-final
    /// cursor (`− K`, clamped to the anchor). Restart-seeded from the marshal's
    /// DURABLE ACKED cursor (`last_consensus_finalized_height`), NOT the reth
    /// head — same soundness argument as the finalized-execution cursor (see the
    /// seed comment in `init`); the reth head is a SPECULATIVE tip that may carry
    /// a nullified sibling above the ack.
    ordering_finalized: u64,
    /// Anchor floor for the finalized cursor: the cold-start finalized point
    /// is result-final by construction (committee-external trust root).
    anchor_finalized: (Height, B256),
    /// Chain-wide activation block for the `result_target` pre-activation
    /// window (see [`Config::dpos_activation_block`]). Distinct from
    /// `anchor_finalized.0` (the cold-start trust/finalized floor): they
    /// coincide only on the FreshMigration signer path.
    dpos_activation_block: u64,

    fcu_heartbeat_interval: Duration,
    fcu_heartbeat_timer: Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
    fcu_pace: Duration,

    /// Tick driving [`ReJump::probe`] (the frozen-tip frontier probe). Armed
    /// unconditionally; the arm no-ops when no probe is wired or the tip advanced.
    /// Cadence: [`FRONTIER_PROBE_INTERVAL_FAST`] while `probe_fast_left > 0`
    /// (active catch-up), else the idle [`FRONTIER_PROBE_INTERVAL`].
    frontier_probe_timer: Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
    /// `last_tip_height` snapshot at the previous probe tick — the frozen-tip
    /// detector (tip advanced since ⇒ the marshal is live ⇒ skip the network probe).
    probe_prev_tip: Height,
    /// Fast-cadence hysteresis: reset to [`FRONTIER_PROBE_FAST_BURST`] on every
    /// PRODUCTIVE probe (one that hinted a new frontier), decremented per tick
    /// otherwise. While non-zero the probe fires every tick (even if the tip just
    /// advanced — that advance was the probe's own delivery, not an independent
    /// live source) so a demoted follower trails by ~one RTT, not a whole tick.
    probe_fast_left: u8,

    finalized_heights_to_backfill: RangeInclusive<u64>,
    pending_backfill: OptionFuture<BoxFuture<'static, (u64, Option<OrderBlock>)>>,
    pending_finalizations: FuturesOrdered<Ready<(Span, OrderBlock, Exact)>>,

    /// Ops-visibility gauge for `pending_finalizations.len()`. Alert on
    /// sustained values > 4 — indicates EL is falling behind consensus
    /// (`MAX_PENDING_ACKS = 16` is the marshal-side ceiling).
    pending_finalizations_gauge: Gauge<i64>,

    /// Height of the block currently PARKED by guard #2 awaiting the
    /// committee-attested `h + K` body (0 = none). Set on a fresh park, reset on
    /// derive / re-jump disposal. `0` in steady state — the pipeline HOLDS the
    /// tip, it does not park it. A constant non-zero value is the durably-parked
    /// signal for a Prometheus alert (`deferred_height != 0 for > Xm`) — the
    /// "how long" lives in the alert, NOT an in-process counter (there is
    /// deliberately no executor wall-clock, §8.11).
    deferred_height: Gauge<i64>,

    /// Heartbeat FCUs are suppressed until consensus advances from the
    /// cold-start snapshot, so a stale initial head is never re-sent over a
    /// canonical chain that moved without us.
    has_advanced_since_init: bool,

    /// Highest height the executor has imported (speculatively OR finalized).
    /// Speculation only fires for `spec_head + 1`, and is tracked here rather
    /// than via `executed_tip()` to avoid reth's `best_number` lag race.
    spec_head: u64,
    /// Heights speculatively executed at notarization but not yet finalized:
    /// height → the notarized ordering digest AND the speculation's seed round
    /// (see [`SpecExecuted`]). On finalized delivery a digest AND seed-round
    /// match means the speculation was correct (skip re-derive, keep the head
    /// lead); a digest mismatch (notarized-then-nullified, sibling finalized) OR
    /// a seed-round mismatch (notarized at round A, finalized at round B) forces
    /// a re-derive + head reorg with the finalization seed back onto the
    /// finalized fork.
    spec_executed: BTreeMap<u64, SpecExecuted>,
    /// Notarized speculative blocks that arrived AHEAD of `spec_head` (a gap)
    /// or before their parent had executed — PARKED here instead of dropped,
    /// keyed by height. Re-driven by [`Self::try_drain_parked`] on the next
    /// `spec_head` advance (the live spec tail OR the finalized reconcile), so
    /// speculation resumes after any transient fall-behind. Bounded to ≈K by
    /// the drain's leading `split_off` (entries ≤ `spec_head` are already
    /// executed speculatively or finalized ⇒ stale) — no arbitrary cap.
    parked_spec: BTreeMap<u64, ParkedSpec>,

    peers_for_finalization: PeersForFinalization,
    /// A finalized block PARKED by guard #2 (see [`Deferred`]); held with its
    /// `Exact` ack AND its already-resolved witness. The `pending_finalizations`
    /// drain is paused while this is `Some` (preserves strict order). Re-poked
    /// event-driven off the marshal's live delivery stream
    /// (`Update::Tip`/`Update::Block`) PLUS the existing FCU heartbeat (the
    /// last-catch-up-block completeness backstop: a body landing at
    /// `height <= tip` fires no `Update::Tip`, so pure delivery re-poke would
    /// deadlock there — [[dpos-deferred-catchup-invariants]] #3). There is NO
    /// give-up timer: a never-arriving body keeps the block parked, never shuts
    /// the executor down (§8.11).
    deferred: Option<Deferred>,

    /// The highest delivered, not-yet-derived finalized block — the tip of the
    /// one-block-lookahead pipeline. Its witness is a function of block `h+1`,
    /// so it is HELD (not parked: no gauge, no hint, no re-poke) and derived the
    /// moment its child arrives via [`Actor::on_finalized_block`]. Two
    /// dispositions for the held ack, and they are NOT the same: on a re-jump
    /// (`reseed_forward`) it is acked `Ok` — the floor MOVES, so the height is
    /// pruned, not skipped; on shutdown / task exit it is DROPPED, deliberately,
    /// never acked — the withheld ack IS the restart self-heal (module docs).
    /// Needs no persistence. On a `SafetyHalt` it joins `park_halted`'s retained
    /// set, exactly like `deferred.ack`.
    awaiting_child: Option<(Span, OrderBlock, Exact)>,

    /// See [`Config::seed_store`]. Read by `spec_execute`'s §4.1 round
    /// re-canonicalisation AND the eager finalized-derive path
    /// ([`Self::try_eager_finalized_derive`]).
    seed_store: Option<crate::beacon::certify::SeedStore>,

    /// See [`Config::epocher`]. Read ONLY by [`Self::try_eager_finalized_derive`]
    /// to form `h`'s own agreed seed round.
    epocher: crate::epocher::OriginEpocher,

    /// The `Exact` ack of the block currently inside [`Self::try_derive`], moved
    /// into this slot at entry and taken back at every non-`Err` exit (the
    /// `NeedAttestation` parks and the final `acknowledge()`). On an `Err` exit the ack
    /// stays here instead of being dropped inside `try_derive`'s frame — so a
    /// `SafetyHalt` `Err` reaches [`Self::park_halted`] with the ack alive and
    /// retainable (the ack-invariant in the module docs). `None` whenever
    /// `try_derive` is not on the stack.
    inflight_ack: Option<Exact>,
}

impl<E, BE, D, XC, MarshalMailbox> Actor<E, BE, D, XC, MarshalMailbox>
where
    E: Clock + commonware_runtime::Metrics + Pacer + Spawner + Send + 'static,
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

        // Finalized-execution cursor (restart seed): seed from the marshal's
        // DURABLE ACKED cursor (`last_consensus_finalized_height` = commonware
        // marshal `last_processed_height`, LATEST_KEY 0xFF), NOT the reth head
        // (`last_execution_finalized_height` = `provider.last_block_number()`).
        // Every ACKED height is consensus-FINALIZED (unique — no sibling) and
        // passed the `try_derive` canonical postcondition before its ack
        // persisted, so the provider's canonical hash there IS the finalized
        // hash — the cursor's stated invariant ([`FinalizedCursor`]).
        //
        // The reth head is UNSOUND as the seed: under deferred execution
        // `spec_execute` advances the head at NOTARIZATION latency and a clean
        // shutdown persists it, so heights in `(acked, head]` are notarized-only
        // and a sibling is still possible (notarize A → nullify → finalize B). A
        // restart straddling such a nullify race, seeded from the head, could
        // serve the orphaned sibling as `finalized_executed_hash` to a propose
        // before the marshal re-reconciles the height — committing a wrong
        // result (bundle-20260716T150148Z divergence, re-entered through
        // restart; see .claude/tasks/2026_07_17__design_root_fixes/
        // family2_finalized_tier.md §1.1).
        //
        // Seeding the cursor lets `finalized_executed_hash` resolve reth's
        // canonical chain across a restart (a fresh process starts the cursor at
        // 0; without the seed the first K post-restart proposals/verifies read
        // None — a coordinated ≥ f+1 restart would wedge permanently). A provider
        // miss at h ≤ cursor (a crash lost the reth tail above the ack ⇒ the
        // backfill range `(last_execution+1..=last_consensus)` re-derives it, or
        // a deep prune) returns None ⇒ propose-skip, never a wrong hash.
        //
        // `ordering_finalized` (the result-final cursor) is seeded from the SAME
        // acked height for the SAME reason (:812): seeded from the reth head it
        // would inherit the notarized-only `(acked, head]` tail — a straddled
        // nullify race then makes `result_final = ordering_finalized − K` and the
        // `finalized_executed_hash` reads under it resolve reth's canonical chain
        // at heights whose sibling is still live, so a restart could pin the
        // engine-API `finalized` onto (or serve K-below result attestations over)
        // an orphaned sibling. Seeding at the acked height keeps every consumer of
        // `ordering_finalized` (result_final, re-jump gap/`from`, the `.max`
        // self-update) at or below a uniquely-finalized height; the
        // `(acked, head]` derives are reconstructed idempotently by the
        // marshal-driven backfill (`new_payload` on a known block = VALID) and the
        // `update_head` reconcile — the head is left seeded at the speculative tip
        // (`spec_head`, :827), which is allowed to lead.
        cfg.executed
            .advance_finalized(cfg.last_consensus_finalized_height.get());

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
            beacon_metrics: cfg.beacon_metrics,
            sync_metrics: cfg.sync_metrics,
            safety_halt: cfg.safety_halt,
            spawn_unblocked: cfg.spawn_unblocked,
            re_jump: cfg.re_jump,
            seed_store: cfg.seed_store,
            epocher: cfg.epocher,
            rejump_fault_streak: 0,
            jump_done: OptionFuture::default(),
            jump_handle: None,
            // Best estimate of the marshal frontier at startup; refined by every
            // `Update::Tip`. Drives the heartbeat re-poke (see field doc).
            last_tip_height: cfg.last_consensus_finalized_height,
            // Seeded from the SAME value `outer.rs` seeds the marshal with, never 0.
            // The guard below keys strictly on this field, and `MarshalActor::run`
            // performs its startup `try_dispatch_blocks` BEFORE it processes the
            // buffered `SetFloor` — so a 0 here leaves the stale-dispatch guard inert
            // for exactly the window in which old-range blocks can escape into the
            // deep-overlay derive it exists to prevent. Pre-existing on every
            // jump-landing boot; unrelated to boundary seeding.
            marshal_floor: cfg.initial_marshal_floor,
            boundary_fetch: cfg.boundary_fetch,
            boundary_enter: cfg.boundary_enter,
            boundary_read_floor: cfg.boundary_read_floor,
            last_canonicalized: LastCanonicalized {
                forkchoice: ForkchoiceState {
                    head_block_hash: cfg.initial_head.1,
                    // At cold-start there is no ordering-final tip above the
                    // anchor yet: safe == finalized == head == anchor. They
                    // diverge only once the chain advances (Phase 2).
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
            awaiting_child: None,
            inflight_ack: None,
        };
        (actor, mailbox)
    }

    pub fn start(mut self) -> Handle<()> {
        spawn_cell!(self.context, self.run().await)
    }

    /// Test-only snapshot of the SEED fields `reseed_forward` and `init` must
    /// agree on at a given landing — used to pin that the steady-state reseed
    /// mirror never diverges from the cold-start seed
    /// (`tests::reseed_forward_agrees_with_init`). `dpos_activation_block` is
    /// deliberately EXCLUDED: it is a chain constant `reseed_forward` never
    /// touches (the landing carries no new activation), so a follower whose
    /// activation ≠ anchor must keep its own value.
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

        // The seed-record notifier (piece D, family2_finalized_tier.md §2.2):
        // `SeedStore::record` fires this on every recorded round. A clone of the
        // `Arc<Notify>` (not `self.seed_store`) so the arm's future borrows a
        // LOCAL, never `self` — no borrow conflict with the arm's `&mut self`
        // body. `None` when the node runs seedless (followers / tests) — the arm
        // then parks forever and never fires.
        let seed_notify: Option<std::sync::Arc<tokio::sync::Notify>> =
            self.seed_store.as_ref().map(|s| s.notifier());

        loop {
            // Do not pull more work while a block is deferred awaiting its h+K
            // attested body (guard #2) — the deferred block must derive first
            // (strict order) — nor while a
            // jump is in flight (bugs 6/7: the jump is the SINGLE EL writer during
            // backfill; a competing startup-drain FCU retargets reth's backfill).
            if self.deferred.is_none()
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
                            // Synthetic ack (the marshal already acked these
                            // heights on a previous run); routes through the
                            // SAME one-block-lookahead pipeline as live dispatch.
                            let (ack, _waiter) = Exact::handle();
                            let span = info_span!("backfill_on_start", %height);
                            if let Err(error) = self.on_finalized_block(span, block, ack).await {
                                self.on_fatal("backfill", error).await;
                                break;
                            }
                        }
                        None => {
                            // bug 10: a hole in the marshal's OWN floor..=last_finalized
                            // inventory cannot self-heal (`get_block` is local-only), so a
                            // skip merely relocates + mislabels the fatal — the later
                            // gap-walk (`derive_missing_prefix`) re-hits the same height and
                            // fails naming the WRONG height. Fail loud AT the true site.
                            error_span!("backfill_on_start", %height).in_scope(|| error!(
                                "marshal has no block at height {height} inside its own \
                                 floor..=last_finalized range — the finalized archives are \
                                 inconsistent (a hole below the floor cannot self-heal); \
                                 shutting down instead of skipping (a skip fails later in the \
                                 gap-walk at the WRONG height)"));
                            break;
                        }
                    }
                    // Restore post-completion .is_none() invariant — upstream
                    // OptionFuture does not auto-clear after Poll::Ready, but the
                    // pending_finalizations arm guard below depends on it.
                    *self.pending_backfill = None;
                }

                // Terminal outcome of the SPAWNED steady-state re-jump waiter
                // (`maybe_re_jump`), delivered over the `jump_done` oneshot.
                // OptionFuture does NOT auto-clear after Poll::Ready (cf.
                // pending_backfill) — clear it (and its handle) here, then act on
                // the outcome.
                outcome = &mut self.jump_done => {
                    *self.jump_done = None;
                    self.jump_handle = None;
                    match outcome {
                        Ok(crate::cold_start_jump::JumpOutcome::Landed { landing, hash, floor }) => {
                            self.reseed_forward(landing, hash, floor).await;
                            // Progress: clear any stale fault tally + the #1 rotate gauge.
                            self.rejump_fault_streak = 0;
                            self.sync_metrics.recover(SyncReason::AuthRotate);
                        }
                        Ok(crate::cold_start_jump::JumpOutcome::Lagging) => {
                            debug!("steady-state re-jump: lagging / stale target — no-op");
                            // The upstream's own `get_latest` says we're caught up
                            // (U2) — clear any stale tally + the #1 rotate gauge.
                            self.rejump_fault_streak = 0;
                            self.sync_metrics.recover(SyncReason::AuthRotate);
                        }
                        Ok(crate::cold_start_jump::JumpOutcome::BadTarget(error)) => {
                            // Rule S/L: a forgeable PRE-anchor structural mismatch is
                            // NON-fatal but a bad-upstream signal — rotate immediately.
                            warn!(
                                error = %format_args!("{error:#}"),
                                "steady-state re-jump target structurally invalid \
                                (forgeable); rotating upstream (NON-fatal)"
                            );
                            self.rotate_upstream().await;
                            self.rejump_fault_streak = 0; // ANY rotate resets (critic r2)
                        }
                        Ok(crate::cold_start_jump::JumpOutcome::InvalidTarget(error)) => {
                            // reth itself rendered an Invalid verdict on the served
                            // branch mid-sync (el_sync_calls >= 1) — a stronger,
                            // more actionable signal than BadTarget's pre-sync
                            // structural check, but the same NON-fatal bad-upstream
                            // treatment: rotate immediately.
                            warn!(
                                error = %format_args!("{error:#}"),
                                "steady-state re-jump target rejected by reth as \
                                INVALID during EL-sync; rotating upstream (NON-fatal)"
                            );
                            self.rotate_upstream().await;
                            self.rejump_fault_streak = 0; // ANY rotate resets (critic r2)
                        }
                        Ok(crate::cold_start_jump::JumpOutcome::Stalled(error)) => {
                            // NON-fatal transient transport stall: count toward the
                            // streak; an honest momentary stall must not insta-rotate.
                            // At MAX consecutive stalls the upstream is failed over
                            // (Rule L). The gap is re-evaluated on the next
                            // `Update::Tip` / heartbeat re-poke regardless.
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
                            // Connected-but-wedged EL pipeline (soak v43): reth had
                            // peers but its executed head stayed frozen — the divergence
                            // root cause is unknown + DETERMINISTIC, so rotating the
                            // upstream would not help (the wedge is local to reth's
                            // pipeline, not a bad-upstream branch). Do NOT rotate: bump
                            // the observability counter, ERROR-log, and RE-ARM — the next
                            // `Update::Tip` / heartbeat re-poke re-spawns the waiter
                            // (`maybe_re_jump`), so the node keeps re-attempting on the
                            // heartbeat cadence while the refill stays DEFERRED (the
                            // floor is never advanced onto an un-synced tip — chain-safe).
                            // Each re-attempt re-wedges + re-logs + re-increments, so the
                            // node is observably stuck instead of silently frozen at the
                            // 6-h backstop.
                            self.sync_metrics.el_sync_stalled_with_peers.inc();
                            error!(
                                error = %format_args!("{error:#}"),
                                "steady-state re-jump: reth CONNECTED but executed head frozen \
                                 (EL pipeline wedged); staying deferred + observable, will re-arm \
                                 on the next frontier tip (NON-fatal, NOT rotating — the wedge is \
                                 local to reth, not the upstream)"
                            );
                        }
                        Ok(crate::cold_start_jump::JumpOutcome::AuthFailed(error)) => {
                            // #1 SELF-HEAL (2026-07-09, Decision A — never crash on a
                            // forged UPSTREAM): a POST-sync committee-BLS / L1 rejection
                            // means the CURRENT upstream served a forged/unagreed
                            // far-ahead branch. Route it through the SAME `rotate_upstream`
                            // escape `BadTarget`/`InvalidTarget` use — with ≥2 upstreams
                            // this moves to the next URL and the next tip/heartbeat
                            // re-jumps onto the honest source; with a SINGLE upstream
                            // `rotate()` is a NO-OP (`upstream.rs` round-robin at len==1),
                            // so the node stays UP-degraded and re-polls the same upstream
                            // on the next tip (recovery from a forged source genuinely
                            // needs ≥2 upstreams). NEVER participates on the forged branch:
                            // the engine hasn't signed it, and reth reorgs off it on the
                            // next jump's FCU. Bounded by the SAME streak reset as the
                            // other rotate arms.
                            warn!(
                                error = %format_args!("{error:#}"),
                                "steady-state re-jump failed authentication (forged \
                                upstream); rotating upstream + staying up-degraded (NON-fatal)"
                            );
                            self.sync_metrics.degrade(SyncReason::AuthRotate);
                            self.rotate_upstream().await;
                            self.rejump_fault_streak = 0; // ANY rotate resets (critic r2)
                        }
                        Ok(crate::cold_start_jump::JumpOutcome::L1Fork(error)) => {
                            // #10 SafetyHalt (Phase 3): the EL-synced head does NOT
                            // descend from the L1-FINALIZED checkpoint — a fork
                            // against L1 finality, the strongest trust root. Unlike
                            // `AuthFailed` (a forged UPSTREAM → rotate to an honest
                            // one), there is nothing to rotate to; HALT (demote to
                            // verify-only, stop driving reth, stay observable) and
                            // wait for the L1 proof + governance recovery.
                            self.safety_halt.engage(SyncReason::L1Fork);
                            self.on_fatal(
                                "steady-state re-jump",
                                eyre::eyre!(
                                    "L1 checkpoint NOT in local chain — SafetyHalt (stop \
                                     participating, stay up): {error}"
                                ),
                            )
                            .await;
                            break;
                        }
                        Err(_canceled) => {
                            // The spawned waiter dropped its sender (task aborted /
                            // panicked) without sending — nothing to act on.
                            debug!("steady-state re-jump waiter canceled before completion");
                        }
                    }
                }

                Some((cause, block, ack)) = self.pending_finalizations.next(),
                if self.deferred.is_none()
                    && self.pending_backfill.is_none()
                    && self.finalized_heights_to_backfill.is_empty()
                    // bugs 6/7: the jump is the SINGLE EL writer while in flight — a
                    // finalize FCU here (`try_derive`) carries a LOW finalized hash that
                    // retargets reth's backfill away from the jump tip, so the jump's
                    // `Valid` terminator never fires (feeds bug 5's ceiling trip). Gating
                    // the drain on `jump_done.is_none()` also means no NEW block parks
                    // during a jump — the ONLY parked block a jump can meet is one parked
                    // BEFORE it spawned (§4.3), which `reseed_forward` disposes via
                    // `ack.acknowledge()` (Ok). `repoke_deferred` is gated on
                    // `jump_done.is_none()` too, so that held block stays untouched
                    // mid-jump. Queued acks stay ALIVE (never dropped).
                    && self.jump_done.is_none() => {
                    self.pending_finalizations_gauge
                        .set(self.pending_finalizations.len() as i64);
                    if let Err(error) = self.on_finalized_block(cause, block, ack).await {
                        self.on_fatal("finalize", error).await;
                        break;
                    }
                }

                msg = self.mailbox.recv() => {
                    let Some(msg) = msg else { break; };
                    if let Err(error) = self.handle_message(msg).await {
                        self.on_fatal("message", error).await;
                        break;
                    }
                }

                // Seed-record notify arm (replaces the `SpecNotarized` Poke): a
                // HELD tip's own round seed just landed in the `SeedStore` — the
                // record-vs-delivery race the on-delivery eager derive missed.
                // Gated on a tip being HELD and no predecessor parked / jump in
                // flight (the same suppression `try_eager_finalized_derive`
                // enforces internally — the guard just avoids a redundant wake).
                // `notify_one` stores a permit even with no parked waiter, so a
                // record that lands between the miss lookup and this await is not
                // lost; a spurious wake (a permit from an unrelated round) is a
                // harmless idempotent re-check (a miss re-holds). Fires the
                // FINALIZED-tier derive, so a SafetyHalt-class error PROPAGATES.
                _ = async {
                    match seed_notify.as_ref() {
                        Some(n) => n.notified().await,
                        None => std::future::pending::<()>().await,
                    }
                }, if self.awaiting_child.is_some()
                    && self.deferred.is_none()
                    && self.jump_done.is_none() => {
                    if let Err(error) =
                        self.try_eager_finalized_derive(EagerTrigger::Notified).await
                    {
                        self.on_fatal("seed-notify eager derive", error).await;
                        break;
                    }
                }

                _ = (&mut self.fcu_heartbeat_timer).fuse() => {
                    self.send_forkchoice_update_heartbeat().await;
                    // Re-evaluate the steady-state re-jump on the heartbeat tick.
                    // The re-jump's `Stalled` retry otherwise depends solely on the
                    // next `Update::Tip`; if the upstream frontier has plateaued
                    // while reth's backfill is the thing stalled, no further tip
                    // arrives → silent permanent wedge. `maybe_re_jump` self-gates
                    // on the gap / in-flight, so it is a no-op whenever the node is
                    // not actually behind. (Cannot error.)
                    let _ = self.maybe_re_jump(self.last_tip_height).await;
                    // Delivery-independent re-poke of a parked block (§4.5): a cert
                    // landing at `height <= tip` fires no `Update::Tip`, so pure
                    // delivery re-poke deadlocks on the last catch-up block
                    // ([[dpos-deferred-catchup-invariants]] #3). This existing tick
                    // (reused, no new timer) re-checks `get_finalization` — it never
                    // shuts down; a still-missing body just re-stays parked.
                    if let Err(error) = self.repoke_deferred().await {
                        self.on_fatal("deferred re-poke", error).await;
                        break;
                    }
                    self.reset_fcu_heartbeat_timer();
                }

                // Frozen-tip frontier probe (see `ReJump::probe`): the live-follow
                // driver for a validator with no cert-inlet (plane-native). A no-op
                // whenever the tip advanced since the last tick or no probe is wired.
                // A productive probe (hinted a new frontier) arms the fast-cadence
                // burst so an actively-following demoted node trails by ~one RTT.
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

        // Cancel the read-only re-jump waiter on shutdown (mirror of the
        // subsystem aborts in `outer.rs`) so a spawned `sync_to` wait does not
        // outlive the executor task. All `break`s converge here (a SafetyHalt
        // never breaks — `on_fatal` parks forever instead).
        if let Some(handle) = self.jump_handle.take() {
            handle.abort();
        }
    }

    /// Route a fatal executor error. With the fork-safety latch engaged (a
    /// Phase-3 `SafetyHalt`) this parks FOREVER via [`Self::park_halted`] and
    /// never returns; otherwise it logs and returns so the caller `break`s the
    /// loop — a genuine crash, which the OuterEngine supervisor answers with
    /// abort-all.
    async fn on_fatal(&mut self, stage: &str, error: eyre::Report) {
        if self.safety_halt.is_engaged() {
            self.park_halted(stage, error).await;
            unreachable!("park_halted never returns");
        }
        error_span!("shutdown").in_scope(|| {
            error!(
                error = %format_args!("{error:#}"),
                stage,
                "executor fatal error; shutting down"
            )
        });
    }

    /// Terminal Phase-3 `SafetyHalt` park: stop deriving/driving reth, RETAIN
    /// every marshal `Exact` ack un-resolved, and never return (only a real
    /// external shutdown aborts the task).
    ///
    /// Why retention (the ack-invariant in the module docs): acknowledging would
    /// durably advance the marshal's `last_processed_height` past the diverged
    /// height (a restart would then skip it forever — silently "resolving" the
    /// divergence), while DROPPING cancels the `Exact` and the marshal treats a
    /// Canceled ack as fatal (`error!("application did not acknowledge block");
    /// return` in commonware marshal/core/actor.rs) — killing the component that
    /// serves blocks + certs to peers, i.e. a zombie node. Holding the acks does
    /// NOT freeze the marshal: its main loop is a `select_loop!` with the ack
    /// waiters in an independent arm, so an unresolved ack merely occupies one of
    /// the 16 dispatch-window slots while the mailbox + resolver arms keep
    /// serving. The marshal may keep dispatching blocks up to that window after
    /// the halt engages; every late-arriving ack is retained here too.
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
        // The one-block-lookahead HELD tip (same treatment as `deferred.ack`):
        // never acked (would durably skip an underived height), never dropped
        // (a Canceled ack kills the marshal).
        if let Some((_cause, _block, ack)) = self.awaiting_child.take() {
            retained.push(ack);
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
                // retained acks alive and pend until the task is aborted — the
                // marshal may still be draining.
                None => futures::future::pending::<()>().await,
            }
        }
    }

    /// The one-block-lookahead pipeline entry (§4): a finalized block `C`
    /// arriving from EITHER source (the `Update::Block` drain or the startup
    /// backfill walk) first supplies the WITNESS for the held tip — `h` is
    /// derived at the dispatch of `h+1`, from the child body already in hand —
    /// then becomes the held tip itself. The child is stored BEFORE the held
    /// block derives so a fatal derive under an engaged SafetyHalt reaches
    /// `park_halted` with the child's ack retainable ("park the parent, hold
    /// the child" — if the parent parks in `deferred`, the child stays here and
    /// the paused drain preserves strict order).
    async fn on_finalized_block(
        &mut self,
        cause: Span,
        block: OrderBlock,
        ack: Exact,
    ) -> eyre::Result<()> {
        let held = self.awaiting_child.take();
        let child_height = block.height;
        let child_seed = block.parent_seed.clone();
        self.awaiting_child = Some((cause, block, ack));
        let Some((held_cause, held_block, held_ack)) = held else {
            // No held tip (first delivery, or the previous tip was already
            // eager-derived): still try to derive the just-arrived block NOW
            // from the SeedStore — the record-lag closer (see below).
            return self
                .try_eager_finalized_derive(EagerTrigger::Delivery)
                .await;
        };
        // Steady-state fast path: the arriving block IS the held tip's child
        // (the marshal dispatches strictly contiguously), so the witness is in
        // hand with ZERO I/O. A non-contiguous arrival is a tripwire, not an
        // assumption — fall through to an async by-height re-fetch (correct,
        // just slower) rather than silently witnessing `h` with a non-child.
        let witness = if child_height == held_block.height + 1 {
            child_seed
        } else {
            metrics::counter!("dpos_witness_child_refetch_total").increment(1);
            warn!(
                held = held_block.height,
                arrived = child_height,
                "witness fast path missed (non-contiguous delivery); re-fetching the child \
                 by height"
            );
            match self
                .marshal
                .fetch_block_by_height(Height::new(held_block.height + 1))
                .await
            {
                Some(child) => child.parent_seed,
                None => {
                    // A GENUINE gap: `h < tip` yet `h+1` is absent from the
                    // marshal's own archive — the pre-existing
                    // hole-below-the-floor class, which cannot self-heal
                    // (`get_block` is local-only). Family-5 taxonomy:
                    // `FaultClass::Corruption` — `Err` with the latch NOT
                    // engaged → supervisor abort-all (an idiosyncratic
                    // local-archive inconsistency, NOT a correlated cause —
                    // loud actor death is correct). Fail loud at the true
                    // site; park the ack in the inflight slot so a latched
                    // SafetyHalt retains it instead of dropping it in this frame.
                    self.inflight_ack = Some(held_ack);
                    return Err(eyre::eyre!(
                        "witness gap: marshal has no block at height {} (child of held {}) \
                         inside its own dispatched range — the finalized archives are \
                         inconsistent",
                        held_block.height + 1,
                        held_block.height
                    ));
                }
            }
        };
        let outcome = self
            .try_derive(held_cause, held_block, held_ack, witness)
            .await?;
        self.defer_if_needed(outcome, true).await;
        // The just-arrived block is now the HELD tip. Its own witness normally
        // only arrives inside ITS child (`h+1`), so a plain lookahead records
        // `h` one child late — recorded_tip = delivered_tip − 1. The
        // finalized-tier result gate reads `finalized_executed_hash(h−K)`, so
        // that −1 leaves ZERO margin at `propose(h)` and any nullify pushes the
        // recorded tip below `h−K` → propose-skip → nullify storm → stall
        // (bundle-20260716T150148Z + 162805Z). Close the lag: derive the held
        // tip NOW from the SeedStore (its own agreed round's seed is present the
        // moment its notarization was processed) instead of waiting for `h+1`.
        self.try_eager_finalized_derive(EagerTrigger::Delivery)
            .await
    }

    /// EAGER finalized derive (record-lag closer, variant iii of
    /// bundle-20260716T150148Z + bundle-20260716T162805Z): derive the block currently HELD in
    /// [`Self::awaiting_child`] NOW — from its OWN agreed seed round in the local
    /// [`SeedStore`] — instead of holding it until its child delivers the witness.
    ///
    /// The round is `Round(epocher.containing(h).epoch(), h.proposal_view)` — a
    /// pure function of AGREED data (rule SA: `proposal_view` rides in the
    /// committee-signed digest; the epoch is the exact block→epoch map that
    /// `application::witness_link` uses for the child's `parent_seed`). A
    /// threshold seed is UNIQUE per round, so a store HIT is byte-identical to
    /// the witness the child would later deliver, and a wrong round can only
    /// MISS — never yield a wrong seed. On HIT the hold is CONSUMED entirely (`h`
    /// is derived+acked here; the child's later delivery finds `awaiting_child`
    /// empty and simply holds `h+1` — no double-derive). On MISS (no store,
    /// non-voted round, restart before the round was processed, or `h` below the
    /// epocher origin) the block stays HELD — the EXACT prior lookahead behavior.
    ///
    /// Suppressed while a predecessor is PARKED (`deferred`) or a re-jump is in
    /// flight (`jump_done`): those own the strict-order / single-EL-writer
    /// invariant, so the tip is HELD there (mirrors `spec_execute`'s guard).
    ///
    /// `trigger` distinguishes the on-delivery attempt from an event-driven
    /// re-attempt fired by the executor's seed-notify `select!` arm when a seed
    /// was just recorded (the record-vs-delivery race self-heal — see
    /// [`Self::run`]'s seed-notify arm, driven by [`SeedStore::notifier`]). A
    /// `Notified` HIT is counted `outcome="recovered"` (the race fired and was
    /// closed without a further finalized delivery); a `Notified` MISS is a
    /// silent no-op (the seed is still not recorded — a later notify or the
    /// child witness will derive it), so the miss counter is NOT inflated on
    /// every notify while held.
    async fn try_eager_finalized_derive(&mut self, trigger: EagerTrigger) -> eyre::Result<()> {
        use commonware_consensus::types::{Epocher as _, Round, View};
        if self.deferred.is_some() || self.jump_done.is_some() {
            return Ok(());
        }
        let Some(store) = self.seed_store.clone() else {
            return Ok(());
        };
        let Some((cause, block, ack)) = self.awaiting_child.take() else {
            return Ok(());
        };
        let round = self
            .epocher
            .containing(Height::new(block.height))
            .map(|info| Round::new(info.epoch(), View::new(block.proposal_view)));
        let seed = round.and_then(|round| {
            store
                .lookup(round)
                .map(|signature| crate::beacon::seed::Seed {
                    target_round: round,
                    signature,
                })
        });
        let Some(seed) = seed else {
            // MISS — restore the hold; the child witness derives it as before.
            self.awaiting_child = Some((cause, block, ack));
            if matches!(trigger, EagerTrigger::Delivery) {
                metrics::counter!(
                    "dpos_executor_eager_finalized_derive_total", "outcome" => "miss"
                )
                .increment(1);
            }
            return Ok(());
        };
        let outcome_label = match trigger {
            EagerTrigger::Delivery => "hit",
            EagerTrigger::Notified => "recovered",
        };
        metrics::counter!(
            "dpos_executor_eager_finalized_derive_total", "outcome" => outcome_label
        )
        .increment(1);
        let outcome = self.try_derive(cause, block, ack, Some(seed)).await?;
        self.defer_if_needed(outcome, true).await;
        Ok(())
    }

    /// PARK a `NeedAttestation` outcome in the deferred slot (a `Done` outcome is
    /// a no-op). `fresh` is `true` for a block first derived off the pipeline —
    /// which hints the missing `h + K` body, sets the observability gauge, and
    /// `warn!`s once — and `false` for a re-poke re-stash (the block is already
    /// parked; do not re-hint/re-warn). There is NO deadline: parking is the
    /// terminal behaviour, re-poked event-driven, never a shutdown (§8.11).
    async fn defer_if_needed(&mut self, outcome: DeriveOutcome, fresh: bool) {
        if let DeriveOutcome::NeedAttestation(d) = outcome {
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
    }

    /// Re-attempt the parked derive on a marshal delivery event or the FCU
    /// heartbeat — a plain "is `h + K`'s body here yet" retry (the witness is
    /// retained in [`Deferred::seed`]; ZERO lookups). Event-driven, NEVER a
    /// shutdown: a still-missing body re-stays parked. Gated on
    /// `jump_done.is_none()` so a parked block is not re-derived mid-jump (a
    /// landed re-jump disposes it via `reseed_forward`). A genuine derive `Err`
    /// (FCU/execution fault) IS fatal — propagated to the caller (the only
    /// surviving shutdown, matching the normal derive arms).
    async fn repoke_deferred(&mut self) -> eyre::Result<()> {
        if self.jump_done.is_some() {
            return Ok(());
        }
        let Some(d) = self.deferred.take() else {
            return Ok(());
        };
        match self.try_derive(d.cause, d.order, d.ack, d.seed).await? {
            outcome @ DeriveOutcome::NeedAttestation(_) => {
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

    /// One frontier-probe tick (see [`ReJump::probe`]). Returns `true` iff this
    /// probe was PRODUCTIVE (hinted a new frontier — the fast-cadence trigger).
    ///
    /// Outside a fast burst, a tip that advanced since the previous tick means the
    /// marshal is learning finalizations from an independent live source
    /// (in-committee consensus or an inlet) — snapshot and return without network
    /// traffic. DURING a burst that advance is the probe's own delivery, so the
    /// probe keeps firing. A probe asks the upstream for its frontier; feeds it
    /// into `upstream_frontier` (the deep-gap re-jump trigger reads
    /// `max(tip, this)`) and, when it is ahead of the known tip,
    /// `hint_finalization(frontier)` so the marshal fetches + verifies it and the
    /// ordinary `Update::Tip` pipeline (marshal gap-repair → contiguous block
    /// dispatch → one-block-lookahead derive) takes over the follow.
    async fn probe_frontier(&mut self) -> bool {
        let Some(probe) = self.re_jump.as_ref().and_then(|rj| rj.probe.clone()) else {
            return false;
        };
        let advanced = self.last_tip_height > self.probe_prev_tip;
        self.probe_prev_tip = self.last_tip_height;
        if advanced && self.probe_fast_left == 0 {
            return false;
        }
        let Some(frontier) = probe().await else {
            debug!("frozen-tip probe: upstream get_latest returned None");
            return false;
        };
        if let Some(rj) = &self.re_jump {
            rj.upstream_frontier
                .fetch_max(frontier.get(), std::sync::atomic::Ordering::Relaxed);
        }
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

    /// Fire the upstream-rotation escape (Rule L) if one is wired. Clones the Arc out
    /// of `self.re_jump` so the immutable borrow does not span the `.await` (the
    /// caller writes `self.rejump_fault_streak` after this returns).
    async fn rotate_upstream(&mut self) {
        let rotate = self.re_jump.as_ref().and_then(|rj| rj.rotate.clone());
        if let Some(rotate) = rotate {
            rotate().await;
        }
    }

    /// Send the finalize forkchoice update, retrying a transient TRANSPORT error
    /// FOREVER (#14 self-heal — the engine STAYS UP; `dpos_sync_degraded{reason=
    /// engine_retry}=1` + `engine_transient_retry_total++` while retrying, cleared
    /// on the first transport success). Returns reth's `ForkchoiceUpdated` intact.
    ///
    /// **Fork-safety split (D1):** a semantic `Ok(PayloadStatusEnum::Invalid)` is
    /// NOT a transport error — it arrives as `Ok(..)` (never folded into `Err`;
    /// `RethImporter::fork_choice_updated` map_errs ONLY the engine-handle channel
    /// error) and is returned here UNTOUCHED so the caller's existing `ensure!`
    /// leaves it on its CURRENT shutdown path (#15 SafetyHalt is Phase 3, out of
    /// scope). This helper NEVER converts an `Invalid` verdict — it only retries
    /// the `Result::Err` transport half.
    async fn fcu_retrying_transport(&mut self, forkchoice: ForkchoiceState) -> ForkchoiceUpdated {
        loop {
            match self
                .beacon_engine
                .fork_choice_updated(forkchoice)
                .pace(&self.context, self.fcu_pace)
                .await
            {
                Ok(fcu) => {
                    self.sync_metrics.recover(SyncReason::EngineRetry);
                    return fcu;
                }
                Err(error) => {
                    self.sync_metrics.degrade(SyncReason::EngineRetry);
                    self.sync_metrics.engine_transient_retry.inc();
                    warn!(
                        error = %format_args!("{error:#}"),
                        "transient engine-API transport error on the finalize FCU; backing off + \
                         retrying (engine stays up — Decision A, no self-crash)"
                    );
                    self.context.sleep(ENGINE_TRANSPORT_RETRY_BACKOFF).await;
                }
            }
        }
    }

    #[instrument(skip_all)]
    async fn send_forkchoice_update_heartbeat(&mut self) {
        if self.jump_done.is_some() {
            // A re-jump's `sync_to` is the EL driver during backfill; an interleaved
            // heartbeat FCU returns reth `SYNCING`
            // (engine/tree/src/tree/mod.rs:1173-1177: `if !backfill_sync_state.is_idle()
            // { return ...syncing() }`), producing a spurious reth-side `Stalled` that —
            // now that `Stalled` rotates — would churn rotation. The re-jump is the
            // single EL writer while in flight.
            debug!("FCU heartbeat suppressed; re-jump in flight (sync_to drives the EL)");
            return;
        }
        if !self.has_advanced_since_init {
            debug!(
                head = %self.last_canonicalized.forkchoice.head_block_hash,
                finalized = %self.last_canonicalized.forkchoice.finalized_block_hash,
                "FCU heartbeat suppressed; no consensus advance since cold-start init"
            );
            return;
        }
        info!(
            head = %self.last_canonicalized.forkchoice.head_block_hash,
            finalized = %self.last_canonicalized.forkchoice.finalized_block_hash,
            "FCU heartbeat",
        );
        let resp = self
            .beacon_engine
            .fork_choice_updated(self.last_canonicalized.forkchoice)
            .pace(&self.context, self.fcu_pace)
            .await;
        // GAP-2 CLOSURE (family 5): a heartbeat FCU transport failure is
        // `FaultClass::TransientExternal(EngineRetry)` — fire-and-forget (the
        // next heartbeat tick is the retry, no loop), but now COUNTED +
        // degrade-visible like the finalize FCU, instead of a bare `warn!`
        // invisible to the taxonomy. A successful tick clears the reason.
        match resp {
            Ok(_) => self.sync_metrics.recover(SyncReason::EngineRetry),
            Err(error) => {
                self.sync_metrics.degrade(SyncReason::EngineRetry);
                self.sync_metrics.engine_transient_retry.inc();
                warn!(error = %error, "heartbeat FCU failed (transport); counted + degraded");
            }
        }
    }

    async fn handle_message(&mut self, message: Message) -> eyre::Result<()> {
        let cause = message.cause;
        match message.command {
            Command::Finalize(finalized) => match *finalized {
                // No FCU here: the tip digest is an ORDERING digest reth
                // cannot resolve, and under F-type the EL never needs devp2p
                // for the DPoS segment — catch-up is marshal backfill of
                // OrderBlocks + local derivation, so every derived block's
                // parent is locally present by construction. (A devp2p
                // fast-sync that skips derivation toward an attested `result`
                // hash is a deferred optimization, not a liveness need.)
                //
                // The marshal emits `Update::Tip` every time it stores a
                // finalization above its tip — it FIRES during a wedge (the inlet
                // keeps storing frontier certs even while contiguous dispatch is
                // stalled), so it is the event the steady-state self-healing
                // re-jump reacts to (no timer / poll).
                Update::Tip(_round, height, _ordering_digest) => {
                    // Remember the frontier so the heartbeat can re-poke the
                    // re-jump even if the upstream frontier later plateaus.
                    self.last_tip_height = height;
                    debug!(%height, "ordering tip observed; EL catch-up is backfill+derive");
                    self.maybe_re_jump(height).await?;
                    // The live finalization heartbeat: a parked block's h+K body
                    // may have landed silently — re-poke it (no-op if a jump the
                    // line above just spawned is now in flight, or nothing is
                    // parked).
                    self.repoke_deferred().await?;
                }
                Update::Block(block, ack) => {
                    // STALE-DISPATCH GUARD: an OLD-range block that escaped into
                    // this mailbox after `reseed_forward` raised the floor but
                    // before the marshal processed `SetFloor` (see `marshal_floor`).
                    // The marshal already pruned it; deriving it against the jumped
                    // `db_tip` is the deep-overlay walk the jump avoids, and parking
                    // it awaits a pruned `h + K` (permanent deferred) or re-fetches a
                    // jump-pruned child (witness-gap fatal). Ack it Ok — the
                    // sanctioned acknowledge-without-derive (NEVER drop an `Exact`: a
                    // dropped ack is Canceled, fatal to the marshal) — count it, and
                    // re-poke the deferred block exactly as the normal arm does.
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
                    // A delivery event may coincide with a parked block's h+K
                    // body landing — re-poke it (no-op when nothing is parked).
                    self.repoke_deferred().await?;
                }
            },
            Command::SpecNotarized(n) => {
                let Notarized { digest, seed } = *n;
                if let Err(error) = self.spec_execute(cause.clone(), digest, seed).await {
                    // Speculation is best-effort: a failure here is logged, never
                    // fatal — `try_derive` (finalized path) will derive the block at
                    // finalization regardless.
                    warn!(
                        error = %format_args!("{error:#}"),
                        %digest,
                        "speculative execution skipped"
                    );
                }
                // A live spec advance may unblock a parked out-of-order
                // notarization (e.g. h+1 parked, then h arrives and advances
                // spec_head) — drain it now. Best-effort (never fatal).
                self.try_drain_parked(&cause).await;
                // NOTE: the eager-derive re-attempt for a HELD tip whose seed
                // landed late (the record-vs-delivery race) is NO LONGER poked
                // from here. It is now the executor's seed-notify `select!` arm
                // (driven by `SeedStore`'s per-record `Notify`), which fires
                // directly on the seed record — correct regardless of this
                // mailbox's ordering, with no lost-notification window.
            }
        }
        Ok(())
    }

    /// Steady-state self-healing re-jump (see [`ReJump`]). The marshal's frontier
    /// (the `Update::Tip` height) has run > [`crate::cold_start_jump::JUMP_THRESHOLD`]
    /// finalized blocks ahead of the highest derived ordering height
    /// (`ordering_finalized`) — the upstream serving window is exactly that wide,
    /// so beyond it the marshal's backfill resolver finds nothing and the floor
    /// freezes forever.
    ///
    /// This does NOT block the `select!` loop on the (multi-minute) backfill: it
    /// SPAWNS the re-jump as a READ-ONLY waiter (the same spawned-fetch idiom the
    /// inlet uses) and the executor reacts to its terminal
    /// [`crate::cold_start_jump::JumpOutcome`] on the `jump_done` `oneshot`
    /// select-arm. The completion arm then runs `reseed_forward` (the WRITE,
    /// shared with `init`'s seed) — so the executor stays the sole writer of
    /// executor state + `set_floor` (§9.6).
    ///
    /// Gates: a missing `re_jump`, an already-in-flight jump (`jump_done` is
    /// `Some` — never spawn a second), a gap ≤ `JUMP_THRESHOLD`, or a mid-flight
    /// startup drain all early-return without spawning. A parked (`deferred`)
    /// block does NOT gate this off: once the gap runs past `JUMP_THRESHOLD` the
    /// situation is no longer "wait for this block's cert" but a deep catch-up
    /// (the durably-stuck-fetch case, §4.3) — the re-jump backfills the
    /// committee-BLS-authenticated `[.. landing]` (the parked height is a finalized
    /// ancestor of the landing), and `reseed_forward` disposes the parked block by
    /// `ack.acknowledge()` (Ok, never Canceled). In Case (A) the gap stays small,
    /// the gap test early-returns, and the park proceeds untouched.
    async fn maybe_re_jump(&mut self, height: Height) -> eyre::Result<()> {
        let Some(re_jump) = self.re_jump.clone() else {
            return Ok(());
        };
        // A jump is already in flight — don't spawn a second.
        if self.jump_done.is_some() {
            return Ok(());
        }
        // The marshal's STORED frontier (`height`) FREEZES under the
        // committee-not-committed defer deadlock; the inlet-advanced
        // `upstream_frontier` does not. Trigger off the larger of the two so a
        // frozen marshal tip cannot mask a real deep gap (see `ReJump`).
        let upstream_frontier = re_jump
            .upstream_frontier
            .load(std::sync::atomic::Ordering::Relaxed);
        let frontier = height.get().max(upstream_frontier);
        if frontier.saturating_sub(self.ordering_finalized)
            <= re_jump.threshold
            // Symmetric closure to the startup-drain's jump-gate (bugs 6/7): a jump
            // and the startup backfill drain must not both drive reth's EL — never
            // start a jump while the drain is mid-flight.
            || self.pending_backfill.is_some()
            || !self.finalized_heights_to_backfill.is_empty()
        {
            return Ok(());
        }
        info!(
            tip = %height,
            upstream_frontier,
            ordering_finalized = self.ordering_finalized,
            "frontier ran past the serving window; spawning steady-state re-jump waiter"
        );
        // Spawn the whole `cold_start_jump` (sync_to wait + auth + L1) as a
        // READ-ONLY waiter and react to its completion on the `jump_done` arm.
        // `re_jump` is already owned (cloned out of `self.re_jump` above) and
        // unused after this move — no second clone needed.
        let from = self.ordering_finalized;
        let (tx, rx) = oneshot::channel();
        let handle = self
            .context
            .with_label("steady_state_rejump")
            .spawn(move |_| async move {
                let _ = tx.send((re_jump.call)(from).await);
            });
        self.jump_done.replace(rx);
        self.jump_handle = Some(handle);
        Ok(())
    }

    /// Re-seed the executor + marshal at a re-jump landing — the steady-state
    /// MIRROR of `init`'s seed (the two MUST agree on field shape; pinned by
    /// `tests::reseed_forward_agrees_with_init`). Runs ONLY in the `jump_done`
    /// completion arm (in the executor task), so it is the sole writer of
    /// executor state + `set_floor`.
    /// Fetch + store the epoch-boundary block(s) a floor raise to `floor` would bury
    /// and that this node does not already hold.
    ///
    /// `b` = the largest epoch-terminal height at or below `floor`, which is what
    /// `Inline::genesis(E)` and therefore the engine-spawn gate needs; `b + 1` is the
    /// epoch's first block, which the promote VALUE-gate reads for the
    /// network-attested key. Both-or-neither when both are buried: seeding `b` alone
    /// would let the member promote at exactly the moment the value gate degrades to
    /// a no-op.
    ///
    /// Every failure path is a no-op that leaves today's behaviour (verify-only for
    /// the landing epoch) — loudly, via the seam's own warn + counter.
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
                continue; // above the floor — ordinary repair fetches it
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

    async fn reseed_forward(&mut self, landing_h: u64, landing_hash: B256, floor: u64) {
        info!(
            landing_h,
            floor, "steady-state re-jump landed; re-seeding executor + marshal floor"
        );
        let landing = Height::new(landing_h);
        self.anchor_finalized = (landing, landing_hash);
        // The landing IS the ordering-final tip (`safe`); `floor = landing − K`
        // is the result-final floor (`finalized`). `update_finalized(landing,…)`
        // raises the in-memory `finalized_height`/`head` to the landing (mirrors
        // `init`'s seed at the landing — pinned by `reseed_forward_agrees_with_init`);
        // it no longer writes `safe`. `update_safe(landing,…)` raises `safe` to
        // the landing. The FCU below re-pins the engine-API `finalized` to the
        // floor (the landing's own result attestation still lags by K) while
        // `safe` rides the landing — the in-memory `finalized_height` over-claim
        // is benign because `result_final` is recomputed from `ordering_finalized`,
        // not the model's `finalized_height` (B1 option a).
        self.last_canonicalized = self
            .last_canonicalized
            .update_finalized(landing, landing_hash)
            .update_safe(landing, landing_hash);
        // PARENT-VISIBILITY FCU (mirror of cold-start `init`'s floor-seed FCU in
        // `dpos.rs`): `update_finalized`/`update_safe` advanced the executor's
        // INTERNAL model, but reth has so far made the backfilled landing segment
        // visible only by NUMBER (the devp2p backfill index). The by-HASH header
        // index that the deriver's `derive_sync` reads for the parent
        // (`header(parent_hash)`) lags until an FCU lands. `head = landing`
        // canonicalizes the whole `[old_canonical+1 ..= landing]` segment by hash
        // (reth inserts every segment element synchronously), so the resumed
        // contiguous dispatch's first derive (`floor + 1`) resolves its parent
        // (= `floor`); `safe = landing` rides the ordering-final tip (the landing
        // IS BFT-final) while `finalized = floor` honours the two-tier contract
        // (the landing's own result attestation still lags by K). `floor ≤
        // landing` and both lie on the segment `head = landing` just made
        // canonical ⇒ `finalized ⊆ safe ⊆ head`. WITHOUT this FCU, `floor + 1`'s
        // derive hits `ParentHeaderMissing` and the floor freezes — the
        // steady-state analogue of the cold-start parent-visibility race.
        // `cold_start_jump::sync_to` already awaited the landing body, and
        // `floor` is backfilled, so the by-NUMBER `executed_hash(floor)` resolves
        // here (the typed ParentHeaderMissing derive-retry is the belt for the
        // transient miss).
        if let Some(floor_hash) = self.executed.spec_executed_hash(floor) {
            let resp = self
                .beacon_engine
                .fork_choice_updated(ForkchoiceState {
                    head_block_hash: landing_hash,
                    safe_block_hash: landing_hash,
                    finalized_block_hash: floor_hash,
                })
                .pace(&self.context, self.fcu_pace)
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
        // OFF-BY-K FIX: raise the executed cursor to the LANDING, not the floor.
        // The landing IS executed post-backfill; the K below-landing blocks are
        // governed by the two-tier result-lag, not by pinning the cursor at the
        // floor. This matches what `init` does (it seeds the executed tip, not
        // the floor).
        self.ordering_finalized = self.ordering_finalized.max(landing_h);
        // Finalized-execution cursor: the jump is BLS-authenticated and the EL
        // synced through the landing, so every height ≤ landing is final —
        // canonical ancestors of a finalized block cannot have siblings.
        // Advancing to the LANDING (not just recording it) is required: the first
        // post-jump proposals at landing+1..landing+K sample
        // `finalized_executed_hash` at landing+1−K..landing — heights BELOW the
        // landing that only the cursor's provider resolve covers (a deep-history
        // provider miss there returns None ⇒ propose-skip, never a wrong hash).
        // Mirrors the `init` seed (monotone).
        self.executed.advance_finalized(landing_h);
        // STALE-SPEC FIX: the speculative tip / map are stale across a deep jump
        // (their heights are far below the landing). Raise `spec_head` to the
        // landing and drop spec entries at/below it so the next notarization
        // re-speculates forward from the landing.
        self.spec_head = self.spec_head.max(landing_h);
        self.spec_executed = self.spec_executed.split_off(&(landing_h + 1));
        // Parked speculative notarizations below the landing are stale across a
        // deep jump (same rationale as `spec_executed` above) — drop them.
        self.parked_spec = self.parked_spec.split_off(&(landing_h + 1));
        // STARTUP-BACKFILL FAST-FORWARD (bundle-20260717T120838Z): the
        // `[last_execution+1 ..= last_consensus]` backfill iterator seeded at
        // `init` is drained by-height off the loop head, but is gated off during
        // the in-flight jump (`jump_done.is_none()` at the drain site) and is
        // NEVER advanced by the reseed above. Un-forwarded, the post-jump drain
        // resumes at its PRE-jump height and re-derives the ENTIRE jumped
        // `[.. landing]` range against `db_tip = landing` — a ~thousands-block
        // overlay walk per derive → mdbx timeouts → the spare never converges. The
        // range is redundant here: the SAME BLS-attestation + EL-sync trust that
        // let the reseed advance `ordering_finalized`/`advance_finalized`/`spec_head`
        // through the landing already covers every height ≤ landing (canonical
        // ancestors of a finalized block have no siblings). Fast-forward the
        // iterator so its next yielded height is `landing_h + 1`, preserving the
        // original upper bound. `RangeInclusive::start()` is the next-to-yield
        // lower bound; `is_empty()` (exhausted or start>end) and a next above the
        // landing both no-op.
        if !self.finalized_heights_to_backfill.is_empty() {
            let next = *self.finalized_heights_to_backfill.start();
            let end = *self.finalized_heights_to_backfill.end();
            if next <= landing_h {
                let skipped = landing_h.min(end) - next + 1;
                // `(landing_h + 1) ..= end` is a correct EMPTY range when the whole
                // remaining span was ≤ landing (landing_h + 1 > end).
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
        // Dispose a block PARKED across this jump (§4.3: the gap-gated re-jump is
        // now permitted while `deferred.is_some()`, so a durably-stuck-fetch node
        // recovers). The re-jump devp2p-backfilled + BLS-authenticated `[.. landing]`,
        // and the parked height is a finalized ancestor of the landing, so its
        // derived block is now canonical in reth — ACK it Ok. This MUST be
        // `acknowledge()` (Ok, never Canceled) and NOT a drop: a drop cancels the
        // `Exact` → the marshal treats a Canceled ack as FATAL → the recover-stall
        // cascade. It is the ONE sanctioned acknowledge-without-derive (the
        // module-doc ack invariant): the floor MOVES past the parked height, so
        // the height is pruned, not skipped. Done BEFORE `set_floor` so
        // `SetFloor`'s `pending_acks.clear()` has nothing left to cancel.
        if let Some(d) = self.deferred.take() {
            self.deferred_height.set(0);
            d.ack.acknowledge();
        }
        // Same disposition for the one-block-lookahead HELD tip: the landing is
        // far above it, so the held height is pruned by the floor move —
        // `acknowledge()` (Ok), never a drop (a dropped `Exact` is a Canceled
        // ack, fatal to the marshal). The one new object the jump path knows
        // about.
        if let Some((_cause, _block, ack)) = self.awaiting_child.take() {
            ack.acknowledge();
        }
        // STALE FINALIZATION BACKLOG PRUNE (bundle-20260716T034647Z):
        // `Update::Block` deliveries queue UNCONDITIONALLY while the drain arm
        // is gated off during a park + in-flight jump — up to MAX_PENDING_ACKS
        // stale below-landing entries. Un-pruned, the stale backlog drains
        // post-jump, re-populates `awaiting_child` with a jumped-over height,
        // and the first genuine post-floor dispatch's non-contiguous witness
        // re-fetch of the (jump-pruned) child returns None → the witness-gap
        // fatal misclassifies a jump-MANUFACTURED skip-gap as archive
        // corruption. The fatal itself stays valid for the genuine
        // hole-below-the-floor class (#8); this removes its false trigger at
        // the source. Entries ≤ landing are canonical post-backfill — the SAME
        // sanctioned acknowledge-without-derive as the deferred/held disposals
        // above (`acknowledge()` Ok, never a drop: a dropped `Exact` is a
        // Canceled ack, fatal to the marshal). Entries above the landing (none
        // expected — dispatch was stalled below it) are kept in order. Done
        // BEFORE `set_floor` for the same reason as the disposals above. The
        // queue holds only `Ready` futures, so this drain never blocks.
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
        // Advance the RUNNING marshal floor (raises-only; prunes below; resumes
        // contiguous dispatch from `floor + 1`). `set_floor` is fire-and-forget, so
        // OLD-range blocks freed by the disposals above can still escape into the
        // executor mailbox before the marshal processes `SetFloor` — record the
        // floor so the `Update::Block` arm acks-without-derive those stragglers
        // instead of parking/re-fetching them.
        // Seed the epoch-boundary block(s) this floor raise is about to bury, BEFORE
        // raising it — the twin of the cold-start seeding in `outer.rs`, needed here
        // too because a steady-state re-jump teleports the floor on a RUNNING node
        // that never restarts. Condition-keyed ("a terminal at/below the floor is
        // missing locally"), not event-keyed, so a node that already holds it does no
        // work and a node that jumped before this shipped still heals.
        self.seed_boundary_below_floor(floor, landing_hash).await;
        self.marshal_floor = floor;
        self.marshal.set_floor(Height::new(floor)).await;
        // Enter the LANDING epoch. Nothing else will: the only other entry edge is a
        // delivered boundary block, and the floor raise above just disqualified this
        // epoch's predecessor terminal from ever being dispatched.
        //
        // KEYED ON THE LANDING, NOT THE FLOOR — and they are different heights. The seed
        // above asks `terminal_at_or_below(floor)` because it repairs the boundary the floor
        // raise is about to bury. The entry asks `terminal_at_or_below(landing_h)` because it
        // names the epoch this node is now IN. The two coincide only when the landing sits in
        // the epoch starting just above the floor; when the landing is within K of an epoch
        // start they differ by a whole epoch, and using the floor here silently enters the
        // wrong one.
        //
        // Condition-keyed, not event-keyed: fired on every landing, including the one where
        // seeding was a no-op because the pair was already local. Idempotent — the state
        // machine's gate is `last_tracked_epoch < next`, so a duplicate is `Intra` and costs
        // one spawned task that breaks on the first `pending_boundary() == None`.
        //
        // Publish the new read floor FIRST. The entry below names a boundary that can sit a
        // whole epoch below the landing (`terminal_at_or_below` returns the PREVIOUS epoch's
        // terminal unless the landing is itself one), and the state machine resolves its
        // committee reads at `boundary − K` — a height this node no longer has after a jump
        // that teleported the floor past it. On a pruned EL every staticcall there fails as
        // an opaque backend error, which the boundary hook retries forever without ever
        // entering. Raising the floor to `floor` clamps that read to the landing's
        // result-final point instead, which the jump just backfilled.
        //
        // `floor`, NOT the landing: the landing is ordering-final (`safe`) while
        // `floor = landing − K` is the result-final point the FCU above pins as `finalized`.
        // The raise is monotone on the state-machine side, so a later/duplicate landing
        // cannot walk it backwards.
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
        // `spec_head` advanced (to the landing) — drain any parked notarization
        // just above it, keeping "drain after every spec_head advance" uniform.
        // Safe here: the `jump_done` arm cleared the in-flight jump BEFORE this
        // call and `deferred` was disposed above, so `spec_execute`'s
        // deferred/jump gate is open. The leading prune already ran (the
        // `split_off` above); entries above the landing may be live.
        self.try_drain_parked(&Span::current()).await;
    }

    /// Speculatively derive + import a NOTARIZED block, advancing the EL head
    /// ahead of finalization. Strictly forward-only (`spec_head + 1`); a gap or
    /// an already-covered height is left to `try_derive` (finalized path), which keeps
    /// this path race-free with finalized delivery (both run in this one loop).
    #[instrument(skip_all, parent = &cause, fields(%digest), err(Debug, level = Level::DEBUG))]
    async fn spec_execute(
        &mut self,
        cause: Span,
        digest: crate::digest::Digest,
        seed: Option<crate::beacon::seed::Seed>,
    ) -> eyre::Result<()> {
        // A finalized block is deferred awaiting its h+K attested body
        // (guard #2 — a strict-order pause).
        // Speculating past it would advance head/spec_head OVER the deferred
        // height, leaking the strict-order invariant (self-healing, but the
        // finalized path is the sole authority — let it derive first). The
        // mailbox arm is intentionally NOT gated (shutdown + Command::Finalize
        // enqueue must keep flowing); the guard lives here.
        //
        // The jump guard is symmetric (bug 7): a speculative FCU carries a low
        // finalized hash that retargets reth's backfill away from the jump tip, so
        // the jump is the SINGLE EL writer while in flight — suppress spec here too
        // (matching the heartbeat + finalize-arm suppression).
        if self.deferred.is_some() || self.jump_done.is_some() {
            return Ok(());
        }
        let Some(order) = self.marshal.fetch_block_by_digest(digest).await else {
            // Body not in the local buffer yet — finalized path will derive it.
            return Ok(());
        };
        let height = order.height;
        // Only speculate the immediate next block. A higher height (gap) is
        // PARKED so `try_drain_parked` re-drives it once `spec_head` catches up
        // (the death-spiral fix: pre-fix a gap notarization was dropped, so a
        // transient fall-behind permanently lost speculation). A height at/below
        // the tip (re-notarization, already executed) is dropped as before — the
        // finalized path owns it. Overwrite-by-height keeps the latest sibling.
        if height != self.spec_head + 1 {
            if height > self.spec_head + 1 {
                self.parked_spec.insert(height, ParkedSpec { digest, seed });
            }
            return Ok(());
        }
        let parent_height = height
            .checked_sub(1)
            .ok_or_else(|| eyre::eyre!("speculative height 0"))?;
        // Parent must be locally present; a transient miss (reth visibility
        // lag) PARKS the notarization (height == spec_head + 1) so the next
        // `spec_head` advance retries it — pre-fix this dropped the notarization
        // and the finalized path was the only retry.
        let Some(parent_hash) = self.executed.spec_executed_hash(parent_height) else {
            self.parked_spec.insert(height, ParkedSpec { digest, seed });
            return Ok(());
        };

        // §4.1 (P2): re-canonicalise the speculative round to the block's OWN
        // `proposal_view` — the same pure-agreed-data round the finalized
        // witness pins (rule PIN). A first-seen notarization at a SPIN round
        // (mid-spin rejoin; body not buffered at V0) must not seal the block
        // with `seed(V0+k)` — that guarantees a re-derive + head reorg at the
        // boundary. On a round mismatch take the canonical round's bytes from
        // `SeedStore` (a threshold seed is unique per round, so the store is a
        // byte source for an already-pinned round); on a miss SKIP speculating —
        // never speculate with a known-wrong seed (the finalized path derives
        // this height from the witness in the child regardless).
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
                    match self.seed_store.as_ref().and_then(|st| st.lookup(canonical)) {
                        Some(signature) => Some(crate::beacon::seed::Seed {
                            target_round: canonical,
                            signature,
                        }),
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

        // The round of the (re-canonicalised) seed this speculation is derived
        // with (`None` = no-beacon). Captured BEFORE `seed` is moved into the
        // deriver; reconciled against the WITNESS round in `try_derive`.
        let seed_round = seed.as_ref().map(|s| s.target_round);
        // EXEC-SATURATION observability: per-block derive+import wall time on the
        // TIP paths only (this speculative path + try_derive's finalized re-derive).
        // The catch-up paths (gap-walk, re-apply retry) run back-to-back by design
        // and would read as false saturation of the 1 blk/s interval.
        let el_apply_started = std::time::Instant::now();
        let derived = self
            .deriver
            .derive_and_execute(order, parent_hash, seed)
            .await
            .wrap_err("speculative derive_and_execute failed")?;
        let derived_hash = derived.evm_hash();
        // DERIVE-SEED TELEMETRY (fork-root byte-confirm): label the speculative
        // (notarization-path) derive for `height` with the seed round it used, so
        // the derive.rs chokepoint line for this height can be attributed to the
        // spec path (vs a later finalized re-derive of the same height).
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

        // Advance the head only; the result-final cursor stays put (the block
        // is not finalized) and there is no marshal ack.
        let new = self
            .last_canonicalized
            .update_head(Height::new(height), derived_hash);
        let fcu = self
            .beacon_engine
            .fork_choice_updated(new.forkchoice)
            .pace(&self.context, self.fcu_pace)
            .await
            .wrap_err("speculative FCU failed")?;
        ensure!(
            fcu.is_valid() || fcu.is_syncing(),
            "EL reported non-valid speculative FCU: {:?}",
            fcu.payload_status
        );
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

    /// Re-drive PARKED speculative notarizations that became runnable now that
    /// `spec_head` advanced. Called after every `spec_head` advance — the live
    /// spec tail (out-of-order notarization arrival) and the finalized reconcile
    /// (the death-spiral recovery: finalization catches the tip up, then a
    /// notarized-but-unfinalized descendant resumes speculation).
    ///
    /// The leading `split_off` is ALSO the finalization PRUNE: it drops every
    /// parked height ≤ `spec_head` (already executed speculatively or finalized ⇒
    /// stale). Because `spec_head ≥ ordering_finalized` always, this is a correct
    /// superset of "prune ≤ finalized" and is the map's only bound (≈K) — no
    /// arbitrary cap. On a rollback (`correctly_speculated == false` resets
    /// `spec_head` to the finalized height) it keeps entries strictly above the
    /// new tip so they re-evaluate against the finalized fork.
    ///
    /// Best-effort (returns `()`): a `spec_execute` failure is logged and the
    /// entry KEPT for the next advance — speculation is never fatal (the
    /// finalized path derives the block regardless). NOT recursive: `spec_execute`
    /// does not call back into this drain; the loop lives here.
    async fn try_drain_parked(&mut self, cause: &Span) {
        // Prune stale entries (≤ spec_head): finalized OR already speculated.
        self.parked_spec = self.parked_spec.split_off(&(self.spec_head + 1));
        let mut resumed = 0u32;
        while let Some(parked) = self.parked_spec.get(&(self.spec_head + 1)).cloned() {
            let next = self.spec_head + 1;
            let before = self.spec_head;
            if let Err(error) = self
                .spec_execute(cause.clone(), parked.digest, parked.seed)
                .await
            {
                // Transient derive/FCU failure — keep the entry, retry on the
                // next advance (best-effort; the finalized path is the authority).
                warn!(
                    error = %format_args!("{error:#}"),
                    height = next,
                    "parked speculative drain failed; retrying on next advance"
                );
                break;
            }
            if self.spec_head > before {
                // `spec_execute` advanced past `next` ⇒ speculation resumed from
                // park (not a live event). Drop the entry and count the resume.
                self.parked_spec.remove(&next);
                metrics::counter!("dpos_executor_spec_resume_total").increment(1);
                resumed += 1;
            } else {
                // A transient gate held (body not buffered yet, parent not
                // executed, deferred/jump in flight) — keep the entry and stop;
                // a later advance retries. `spec_execute` never re-parks `next`
                // here (its height == spec_head + 1, not a gap).
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
    }

    /// EXEC-SATURATION observability: the deferred executor's lag = consensus
    /// ORDER tip height (`last_tip_height`, fed by `Update::Tip`) minus the
    /// executed EL tip it has applied. Sustained lag ≥ K is the mechanism behind
    /// the verify-time result-gate stall (soak: 5× view overruns under heavy
    /// blocks) — surfaced as a gauge so INFO+WARN soak bundles need no DEBUG.
    /// Called at each TIP-path apply site, where both values are already in hand.
    fn record_el_lag(&self) {
        let lag = self
            .last_tip_height
            .get()
            .saturating_sub(self.executed.executed_tip());
        metrics::gauge!("dpos_executor_el_lag_blocks").set(lag as f64);
    }

    /// The §4 witness-downgrade refusal (agreed-data monotonicity): a block that
    /// itself carries a `parent_seed` witness sits on a beacon-active link, so
    /// epochs being non-decreasing, its OWN witness (in the child) MUST be
    /// present too. The vote gate makes a missing witness unreachable on-chain;
    /// a corrupted/legacy on-disk archive can still present one, and letting it
    /// fall through to the `order.digest()` fallback would derive a different
    /// `prev_randao` than the network — a SILENT FORK. SafetyHalt-class: refuse
    /// loudly (halt), never derive.
    ///
    /// Residual (reviewer-assessed): the FIRST beacon-active height — whose own
    /// `parent_seed` is `None` because its parent is pre-bootstrap — is not
    /// detectable by this monotonicity check; a corrupted `None` witness there
    /// falls through to the fallback derive and is instead caught by the
    /// `result_matches` cross-check K blocks later (a DELAYED halt, not a fork).
    fn refuse_witness_downgrade(
        &self,
        order: &OrderBlock,
        witness: &Option<crate::beacon::seed::Seed>,
    ) -> eyre::Result<()> {
        if witness.is_none() && order.parent_seed.is_some() {
            self.safety_halt.engage(SyncReason::ResultDivergence);
            return Err(eyre::eyre!(
                "block {} is on a beacon-active link (it carries a parent_seed witness itself) \
                 but its child presents NO witness — corrupted/legacy archive; deriving with the \
                 digest fallback would silently fork; SafetyHalt",
                order.height
            ));
        }
        Ok(())
    }

    /// Derive + import + FCU + ack a finalized block from its WITNESS (`seed` =
    /// the child block's `parent_seed`; `None` = a pre-bootstrap,
    /// seed-independent link). Guard #2 (the `h + K` look-ahead convergence
    /// check) runs whenever the node is ≥ K behind; if the attested body at
    /// `h + K` is not backfilled yet this returns `NeedAttestation` — WITHOUT
    /// mutating any finalized state or acking — so the caller PARKS it and
    /// re-pokes event-driven (the delivery stream + the FCU heartbeat).
    #[instrument(skip_all, parent = &cause, fields(height = order.height), err(Debug))]
    async fn try_derive(
        &mut self,
        cause: Span,
        order: OrderBlock,
        ack: Exact,
        seed: Option<crate::beacon::seed::Seed>,
    ) -> eyre::Result<DeriveOutcome> {
        // Parked in the slot so an `Err` exit (including every SafetyHalt path,
        // several of which surface through `?`) leaves the ack ALIVE for
        // `park_halted` instead of dropping it in this frame (a drop cancels →
        // the marshal dies). Taken back at each non-`Err` exit.
        self.inflight_ack = Some(ack);
        let height = order.height;
        // Captured before `order` is consumed by `derive_and_execute` below; the
        // attested result commits `executed_hash(height − K)`, cross-checked after
        // the derive lands.
        let attested_result = order.result;
        let parent_height = height
            .checked_sub(1)
            .ok_or_else(|| eyre::eyre!("ordering height 0 cannot be finalized"))?;

        // §4: a beacon-active link presenting no witness is a corrupted archive —
        // refuse loudly (halt) rather than fork on the digest fallback.
        self.refuse_witness_downgrade(&order, &seed)?;

        // The WITNESS seed for this height (block `height+1`'s `parent_seed`,
        // already validated at the child's vote time under the committee
        // multisig). Its ROUND completes the speculation-reuse invariant just
        // below, and its VALUE is reused verbatim by the re-derive branch and
        // the re-apply loop. `None` = pre-bootstrap link (seed-independent).
        let finalization_seed = seed;
        let finalization_round = finalization_seed.as_ref().map(|s| s.target_round);

        // Reconcile against speculation: keep the speculatively-executed block
        // ONLY when it is the SAME ordering block AND was speculated with the
        // SAME seed round as the witness — then reth is already canonical here,
        // so skip the re-derive and, crucially, do NOT roll the head back (the
        // speculative lead at `height+1..` must survive). After §4.1 both rounds
        // are `Round::new(Ep, block.proposal_view)`, so a digest match with a
        // DIFFERENT round is an ANOMALY (two paths disagreeing about the
        // canonical round) — counted, then re-derived from the witness (the
        // agreed value), the SAME path a first execution or a sibling-nullified
        // digest mismatch takes. `None == None` (no-beacon) keeps the fast path.
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
                    witness_round = ?finalization_round,
                    "speculation round disagrees with the witness round (both should be \
                     Round(Ep, proposal_view)) — re-deriving from the witness"
                );
            }
        }
        // PARENT-LINKAGE (the deep-speculation reorg guard): the speculated block
        // may be REUSED as final only if it descends from the block that IS
        // canonical at `parent_height` NOW. After a head rollback at `height − 1`,
        // the parent was re-derived to a DIFFERENT hash; a speculated block still
        // recorded at `height` was executed against the now-orphaned parent (wrong
        // pre-state) and would splice a forked block onto the finalized chain if
        // reused — the same fork-safety family as the spec-seed-blind divergence.
        // Absence of the parent (`None`) is not a match, so a missing parent takes
        // the re-derive path (which walks `derive_missing_prefix`).
        let correctly_speculated = spec_round == Some(finalization_round)
            && spec_parent == self.executed.spec_executed_hash(parent_height)
            && self.executed.spec_executed_hash(height).is_some();

        // Retained for the post-FCU apply-retry loop below — the re-derive branch
        // consumes `finalization_seed` (one 48-B signature clone per block).
        let finalization_seed_retry = finalization_seed.clone();
        // Guard #2 (below) runs when the node is ≥ K behind. The derive branch
        // CONSUMES `order`, but guard #2's absent-body arm must hand `order`
        // back to PARK it — clone it up front (rare path, one clone). Zero cost
        // in steady state: the derive of `h` runs when `h+1` is the tip, so
        // `last_tip_height >= h + K` is false and no clone/fetch occurs.
        let behind_by_k = self.last_tip_height.get() >= height + crate::order_block::K;
        let order_for_park = behind_by_k.then(|| order.clone());
        let derived_hash = if correctly_speculated {
            // Already derived via spec_execute with a seed of the SAME round as
            // the witness — reth is canonical here, no re-derive needed.
            self.executed
                .spec_executed_hash(height)
                .expect("checked is_some above")
        } else {
            let parent_hash = match self.executed.spec_executed_hash(parent_height) {
                Some(hash) => hash,
                // The marshal can hold finalized artifacts the EL hasn't derived
                // yet (restart with an unflushed reth tail; repair landing ahead
                // of dispatch). Derivation is strictly sequential, so walk the
                // missing prefix out of the marshal and derive it first; a
                // genuinely unfillable gap stays fatal (visible, not wedged).
                // `order` is the walk's final witness source (it is the child of
                // the walk's top element).
                None => self.derive_missing_prefix(parent_height, &order).await?,
            };

            // EXEC-SATURATION observability (see spec_execute for scope rationale).
            let el_apply_started = std::time::Instant::now();
            let derived = self
                .deriver
                .derive_and_execute(order, parent_hash, finalization_seed)
                .await
                .wrap_err("derive_and_execute failed")?;
            let derived_hash = derived.evm_hash();
            self.submit_finalized_payload(derived).await?;
            metrics::histogram!("dpos_derive_el_apply_duration_seconds", "path" => "finalized")
                .record(el_apply_started.elapsed().as_secs_f64());
            derived_hash
        };
        self.record_el_lag();

        // DERIVE-SEED TELEMETRY (fork-root byte-confirm): the finalized derive's
        // provenance for `height`. `fin_proposal_round` is the WITNESS round
        // (`Round(Ep, proposal_view)`, pinned by rule PIN at the child's vote).
        // `path` records whether the speculative block was REUSED (and the round
        // it was speculated with, `spec_seed_round`) or RE-derived. Compared
        // cross-node at a diverged height this is the (a)-vs-(b) discriminator:
        // (a) SAME `fin_proposal_round` on both sides but a different resulting
        // hash / prev_randao (derive.rs line) ⇒ seed decoupled from the agreed
        // round; (b) DIFFERENT `fin_proposal_round` ⇒ the diverged node holds a
        // genuinely different witness for this height.
        // Read `spec_executed` BEFORE the split_off below prunes it.
        tracing::info!(
            target: "dpos::derive_seed",
            height,
            path = if correctly_speculated { "finalized-spec-reuse" } else { "finalized-rederive" },
            fin_proposal_round = ?finalization_round,
            spec_seed_round = ?spec_round,
            evm_hash = %derived_hash,
            "derive-seed: finalized derive path",
        );

        // GUARD #2 (re-gated to `last_tip_height >= h + K` — fires only when the
        // node is BEHIND): the immediate `h + K` look-ahead convergence check.
        // Because `last_tip_height >= h + K`, the committee-attested
        // `order.result` at `h + K` — which commits `executed_hash(h)` — is
        // ALREADY finalized, so a wrong derive is caught IMMEDIATELY, not K
        // blocks downstream, and it front-runs the ack (and the `split_off`
        // prune below). This is the ONLY code-proven result-divergence detector
        // on the catch-up path; the steady state is covered by the separate
        // `h − K` backward cross-check further down, and costs NOTHING here (the
        // derive of `h` runs when `h+1` is the tip, so the gate is false).
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
                        // The network-attested root at `h + K` disagrees with the
                        // hash we derived → we would serve a fork. Halt
                        // (verify-only, stay observable) BEFORE acking.
                        self.safety_halt.engage(SyncReason::ResultDivergence);
                        return Err(eyre::eyre!(
                            "guard #2 at {height}: attested result at {hk} disagrees with \
                             local executed_hash({height}); SafetyHalt — refusing to serve a fork"
                        ));
                    }
                }
                // The `h + K` body is not backfilled yet (tip >= h+K, but the block
                // hasn't landed). PARK — a fall-through would reach the unconditional
                // `ack.acknowledge()` and finalize `h` with NO convergence check.
                // Returning here (BEFORE the `split_off` prune) keeps
                // `spec_executed[height]` intact, and the park CARRIES the witness,
                // so the re-poke re-derives with zero lookups.
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

        // The finalized fork is now canonical at `height`. Any speculation
        // above it that built on a now-orphaned sibling is invalid; reset the
        // speculative tip so the next notarization re-speculates forward. A
        // correct speculation keeps its lead.
        if correctly_speculated {
            self.spec_head = self.spec_head.max(height);
            // Keep the surviving lead above `height`; drop the finalized prefix
            // (≤ height) — `split_off` returns the > height suffix.
            self.spec_executed = self.spec_executed.split_off(&(height + 1));
        } else {
            // ROLLBACK: the finalized fork replaced the speculated sibling at
            // `height` and the head FCU (below) rolls the EL head back to it, so
            // the ENTIRE speculative suffix above `height` was executed against a
            // now-orphaned parent — INVALIDATE it (drop, do not retain the way a
            // correct speculation does). Parked notarizations above `height` are
            // KEPT: the post-FCU `try_drain_parked` legitimately re-executes them
            // against the new canonical parent (the re-heal path).
            self.spec_head = height;
            let dropped_suffix = self.spec_executed.split_off(&(height + 1)).len();
            // `split_off` left the ≤ height entries in place (finalized/stale after
            // the rollback) — clear them too, matching the `correctly_speculated`
            // arm which drops everything ≤ height.
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

        // Trustless result cross-check (the SAME property `FluentApp::verify`
        // enforces on the BFT path): the attested result commits the locally-derived
        // hash at `height − K`. A present-and-mismatched hash means this node would
        // serve a fork — fail loud (the loop arm shuts down on `Err`). Absence
        // (`None`, not yet resolved) and a match fall through. The pre-activation
        // window is keyed on the CHAIN activation block (not the cold-start
        // trust anchor `anchor_finalized.0`): a deep-catch-up follower anchors at
        // the live frontier yet derives the K-below-anchor blocks, which are
        // post-activation and carry real (non-zero) results.
        if let Some(false) = crate::order_block::result_matches(
            attested_result,
            height,
            self.dpos_activation_block,
            |h| self.executed.spec_executed_hash(h),
        ) {
            // #2/#3 SafetyHalt (Phase 3): the committee-attested result at
            // `height − K` disagrees with what THIS node executed. Extending here
            // would serve a fork. Latch the halt (demote to verify-only, stop
            // driving reth, keep marshal/RPC alive via the supervisor park) rather
            // than `process::exit`; recovery is the L1 SP1 validity proof.
            self.safety_halt.engage(SyncReason::ResultDivergence);
            return Err(eyre::eyre!(
                "result divergence at height {height}: attested result {attested_result:?} != \
                 local executed_hash; SafetyHalt — refusing to serve a forked chain"
            ));
        }

        // A finalized block was recorded ⇒ the marshal now holds another finalized
        // block. Wake any per-epoch engine spawn parked on the `Inline::genesis(E)`
        // precondition (the E-1 boundary block landing). `notify_one` stores a permit
        // so a finalized block recorded between reconciles is not lost; the reconciler
        // gates on a pending parked spawn.
        self.spawn_unblocked.notify_one();
        let result_final = crate::order_block::result_final_height(
            self.ordering_finalized,
            self.anchor_finalized.0.get(),
        );

        let mut new = self.last_canonicalized;
        if result_final > new.finalized_height.get() {
            // The result-final block was derived+FCU'd K iterations ago, so
            // its canonical hash is resolvable; a transient miss keeps the
            // previous finalized cursor (monotonicity over progress).
            match self.executed.spec_executed_hash(result_final) {
                Some(hash) => new = new.update_finalized(Height::new(result_final), hash),
                None => warn!(
                    result_final,
                    "result-final hash unresolved; keeping previous finalized cursor"
                ),
            }
        }
        // Ordering-final tier → engine-API `safe`: the just-finalized tip.
        // `derived_hash` == executed_hash(height) (whether re-derived or
        // correctly-speculated) and `height == ordering_finalized` here, so
        // `safe` lands ~0 blocks behind head while `finalized` lags by K.
        //
        // `safe` is ALWAYS reth-canonical-findable at this FCU: `safe ≤ head` on
        // the same derived chain (D2), and this same FCU names `head ≥ height`;
        // reth commits the whole head→fork segment (incl. `safe`) into the
        // canonical in-memory state (`on_canonical_chain_update`) BEFORE it
        // validates `safe` (`ensure_consistent_forkchoice_state`), so
        // `find_canonical_header(safe)` is `Some` → no `-38002`. If head
        // canonicalization itself fails (a missing block), reth returns SYNCING
        // via `handle_missing_block` and never reaches the safe check.
        new = new.update_safe(Height::new(height), derived_hash);
        // Move the head onto the finalized block only when speculation did not
        // already place the correct block here (else we would roll back the
        // speculative lead). A re-derive/rollback DOES move the head (reorg) —
        // and `update_safe`'s `>=` guard above already re-pinned `safe` to the
        // same `derived_hash`, so `safe == head` at the reorg point (never an
        // orphaned sibling).
        if !correctly_speculated {
            new = new.update_head(Height::new(height), derived_hash);
        }

        // #14 SELF-HEAL: a transient TRANSPORT error retries forever (engine stays
        // up + `reason=engine_retry` gauge). A semantic `Ok(Invalid)` verdict is
        // returned untouched (never folded into the transport `Err` — D1) and is
        // the #15 SafetyHalt below: reth rejected our locally-derived block, so
        // extending would serve a chain reth itself disowns.
        let fcu = self.fcu_retrying_transport(new.forkchoice).await;
        if !(fcu.is_valid() || fcu.is_syncing()) {
            // #15 SafetyHalt (Phase 3): halt (verify-only, stop driving reth,
            // stay observable) instead of exiting — recovery is the L1 proof.
            self.safety_halt.engage(SyncReason::ElInvalid);
            return Err(eyre::eyre!(
                "EL reported non-valid finalize FCU: {:?}; SafetyHalt",
                fcu.payload_status
            ));
        }

        // POSTCONDITION (fork-safety): the finalized block is reth-CANONICAL at
        // `height` before this delivery acks. A tolerated SYNCING FCU means "not
        // applied yet", NOT success — the soak3 fork @ 9924: the re-derived
        // finalized sibling was silently dropped by the EL (`InsertExecutedBlock`
        // height gate), the reorg FCU answered SYNCING, and every later parent
        // lookup at `height` returned the stale speculative sibling — a permanent
        // fork. Until the EL actually serves `derived_hash` at `height`, re-apply
        // (re-derive + import + FCU) forever — Decision A: degraded-visible
        // (`dpos_sync_degraded{reason=finalize_apply}`), never proceed, never exit.
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
            let Some(parent_hash) = self.executed.spec_executed_hash(parent_height) else {
                continue;
            };
            let derived = match self
                .deriver
                .derive_and_execute(order, parent_hash, finalization_seed_retry.clone())
                .await
            {
                Ok(derived) => derived,
                Err(error) => {
                    warn!(
                        error = %format_args!("{error:#}"),
                        height,
                        "re-apply derive failed; retrying"
                    );
                    continue;
                }
            };
            ensure!(
                derived.evm_hash() == derived_hash,
                "re-apply derived a different hash at height {height}: {} != {derived_hash} \
                 (non-deterministic derive)",
                derived.evm_hash()
            );
            self.submit_finalized_payload(derived).await?;
            let fcu = self.fcu_retrying_transport(new.forkchoice).await;
            if !(fcu.is_valid() || fcu.is_syncing()) {
                self.safety_halt.engage(SyncReason::ElInvalid);
                return Err(eyre::eyre!(
                    "EL reported non-valid finalize FCU on re-apply: {:?}; SafetyHalt",
                    fcu.payload_status
                ));
            }
        }
        self.sync_metrics.recover(SyncReason::FinalizeApply);

        // Advance the FINALIZED-execution cursor for the result gate. Past the
        // canonical postcondition above, `derived_hash` is reth-canonical at
        // `height` and beyond reorg (both arms reach here — the re-derive /
        // rollback arm with the freshly-finalized sibling, the correctly-
        // speculated arm with the spec hash the finalization CONFIRMED), so the
        // cursor need only NAME the height: `finalized_executed_hash(height)`
        // then resolves `derived_hash` straight from reth's canonical chain (reth
        // is the tier-F store — no separate hash map). Propose + verify read this
        // via `finalized_executed_hash(h−K)` so a still-speculative sibling can
        // never be committed as an OrderBlock `result` (closes the seed-blind
        // result-commit fork at its SOURCE, bundle-20260716T150148Z; the h−K
        // backward cross-check above stays the safety net). The cursor lives in
        // the shared executed store, not the per-epoch engine, so it survives
        // engine restarts within the process.
        self.executed.advance_finalized(height);

        if new != self.last_canonicalized {
            self.has_advanced_since_init = true;
        }
        self.last_canonicalized = new;
        self.reset_fcu_heartbeat_timer();

        self.take_inflight_ack().acknowledge();

        // The finalized reconcile advanced `spec_head` (above, at the
        // `correctly_speculated` branch) and the head/safe/finalized FCU has now
        // landed. Resume speculation from any parked notarized-unfinalized
        // descendant (the death-spiral recovery) AND prune parked heights the
        // finalization made stale. Placed AFTER the finalize FCU — not at the
        // `spec_head` advance itself — so a speculative FCU cannot roll the just-
        // finalized head back. Best-effort.
        self.try_drain_parked(&cause).await;
        Ok(DeriveOutcome::Done)
    }

    /// Take the ack parked at [`Self::try_derive`]'s entry (see `inflight_ack`).
    fn take_inflight_ack(&mut self) -> Exact {
        self.inflight_ack
            .take()
            .expect("inflight ack set at try_derive entry")
    }

    /// Derive the missing `..=target` prefix from the marshal's archive:
    /// probe backward to the highest executed ancestor, then fetch + derive +
    /// import forward. BLOCKS-ONLY (§4): the witness for gap height `h` is block
    /// `h+1`'s `parent_seed` — the next element of the walk, and for the top
    /// element (`h == target`) the `delivered` block the caller is holding
    /// (`delivered.height == target + 1`). No certs, no extra fetches, no hints.
    /// Returns the derived hash AT `target`. A missing BLOCK stays fatal (the
    /// pre-existing "hole below the floor cannot self-heal" class); the re-walk
    /// on a retry is idempotent — already-derived prefix heights advance
    /// `first_missing`.
    async fn derive_missing_prefix(
        &mut self,
        target: u64,
        delivered: &OrderBlock,
    ) -> eyre::Result<B256> {
        let mut first_missing = target;
        let mut parent_hash = loop {
            if first_missing == 0 {
                return Err(eyre::eyre!(
                    "derive gap reaches height 0 — no executed ancestor"
                ));
            }
            if let Some(hash) = self.executed.spec_executed_hash(first_missing - 1) {
                break hash;
            }
            first_missing -= 1;
        };
        info!(
            first_missing,
            target, "deriving missing prefix from marshal before the delivered block"
        );
        let mut order = self
            .marshal
            .fetch_block_by_height(Height::new(first_missing))
            .await
            .ok_or_else(|| {
                eyre::eyre!(
                    "derive gap: marshal has no ordering artifact at height {first_missing}"
                )
            })?;
        for h in first_missing..=target {
            // The WITNESS for `h` = block `h+1`'s `parent_seed`: the next walk
            // element (fetched once — it becomes the next iteration's `order`),
            // or the caller-held `delivered` block at the top of the walk.
            let (seed, next_order) = if h == target {
                (delivered.parent_seed.clone(), None)
            } else {
                let child = self
                    .marshal
                    .fetch_block_by_height(Height::new(h + 1))
                    .await
                    .ok_or_else(|| {
                        eyre::eyre!(
                            "derive gap: marshal has no ordering artifact at height {}",
                            h + 1
                        )
                    })?;
                (child.parent_seed.clone(), Some(child))
            };
            self.refuse_witness_downgrade(&order, &seed)?;
            // Captured before `order` is consumed: each gap block carries its OWN
            // committee-attested `result` commitment, which must be cross-checked
            // exactly like the top-level delivered block — otherwise a wrong
            // `result` on a gap-range block (the byzantine-vrf defense) would be
            // imported unchecked.
            let attested_result = order.result;
            // Derive-seed telemetry: gap-walk witness round, captured before
            // `seed` moves into the deriver.
            let gap_seed_round = seed.as_ref().map(|s| s.target_round);
            let derived = self
                .deriver
                .derive_and_execute(order, parent_hash, seed)
                .await
                .wrap_err_with(|| format!("gap derivation failed at height {h}"))?;
            parent_hash = derived.evm_hash();
            // DERIVE-SEED TELEMETRY (fork-root byte-confirm): label the finalized
            // GAP-WALK derive so a height derived via prefix catch-up (vs top-level
            // `try_derive`) is attributable; `fin_proposal_round == gap_seed_round`
            // (the witness round of block h+1).
            tracing::info!(
                target: "dpos::derive_seed",
                height = h,
                path = "finalized-missing-prefix",
                fin_proposal_round = ?gap_seed_round,
                evm_hash = %parent_hash,
                "derive-seed: gap-walk derive path",
            );
            // The walk ADVANCES `parent_hash` onto this block with no later
            // landing re-check, so a transport-degraded (non-landed) import must
            // end the walk HERE with the honest cause — otherwise the next
            // iteration's derive dies under "gap derivation failed at height
            // {h+1}" masking the transport failure. No retry loop: the walk is
            // idempotent and re-enters (already-derived prefix heights advance
            // `first_missing`).
            if !self.submit_finalized_payload(derived).await? {
                return Err(eyre::eyre!(
                    "gap-walk import at height {h} hit an engine-API transport failure \
                     (block not landed); aborting the walk — a re-entry re-walks the \
                     idempotent prefix"
                ));
            }
            // SAME trustless result cross-check as `try_derive` (keyed on the
            // CHAIN activation block, NOT the cold-start anchor): the attested
            // result commits the locally-derived hash at `h − K`. A
            // present-and-mismatched hash means this node would serve a fork —
            // fail loud. Pre-activation gap blocks (`result == ZERO`) still pass
            // (`result_matches` returns `Some(true)`). Absence (`None`, the K-back
            // height not yet derived) falls through; once `h` is ≥ K above the
            // walk's first derived height the ancestor is locally resolved.
            if let Some(false) = crate::order_block::result_matches(
                attested_result,
                h,
                self.dpos_activation_block,
                |q| self.executed.spec_executed_hash(q),
            ) {
                // #2/#3 SafetyHalt (Phase 3) — same fork-safety latch as the
                // top-level cross-check, on a gap-range block.
                self.safety_halt.engage(SyncReason::ResultDivergence);
                return Err(eyre::eyre!(
                    "result divergence at gap height {h}: attested result {attested_result:?} != \
                     local executed_hash; SafetyHalt — refusing to serve a forked chain"
                ));
            }
            // Hand the already-fetched child to the next iteration (each walk
            // element is fetched exactly once). `None` only at `h == target`,
            // where the range is exhausted anyway.
            match next_order {
                Some(next) => order = next,
                None => break,
            }
        }
        Ok(parent_hash)
    }

    /// Import the derived block into the EL. VALID is the expected steady
    /// state (single-execution insert acks synthetically; the new_payload
    /// fallback re-executes a block whose parent was derived one iteration
    /// ago); SYNCING is tolerated for the cold-start/rejoin window. Only a
    /// genuinely INVALID status is fatal — under the fallback it means local
    /// derivation diverged from reth's re-execution.
    ///
    /// Returns `Ok(true)` when the EL ACCEPTED the import (Valid/Syncing) and
    /// `Ok(false)` when a TRANSPORT failure was degraded — the block did NOT
    /// land. Callers whose reconvergence re-checks landing (the finalized
    /// postcondition loop; speculation reconciled at finalization) may ignore
    /// the flag; a caller that would ADVANCE on the derived hash without a
    /// landing re-check (the gap-walk) MUST check it, or the death one
    /// iteration later masks the transport cause.
    async fn submit_finalized_payload(&mut self, derived: D::Derived) -> eyre::Result<bool> {
        // Single chokepoint for all three derive paths (spec / finalized / gap):
        // record this block's beacon outcome before the value is moved into the EL.
        match derived.beacon_active() {
            Some(true) => self.beacon_metrics.seed_active.inc(),
            Some(false) => self.beacon_metrics.digest_fallback.inc(),
            None => 0,
        };
        // TRANSPORT-vs-VERDICT split (family 5, type-level via `BeaconEngineLike`):
        // the verdict rides in `Ok`, transport in `Err(TransportError)`.
        let status = match self
            .beacon_engine
            .import_derived(derived)
            .pace(&self.context, self.fcu_pace)
            .await
        {
            Ok(status) => {
                self.sync_metrics.recover(SyncReason::EngineRetry);
                status
            }
            // GAP-1 CLOSURE: an import TRANSPORT error is now `FaultClass::
            // TransientExternal(EngineRetry)` — the SAME class as its FCU sibling
            // — instead of the former `?`→actor-death asymmetry. Degrade-visible
            // + counted, engine stays UP (Decision A: never actor-death on a
            // correlated engine-transport cause). The block is not landed, so the
            // caller's reconvergence retries: the finalized path's canonical
            // POSTCONDITION re-apply loop re-derives + re-imports until it lands;
            // the speculative path re-derives at finalization; the gap-walk
            // re-reads the parent. In-process `RethImporter` transport is a closed
            // engine channel (a re-send cannot reopen it, and `D::Derived` is
            // non-`Clone`), so the disposition — not an in-place infinite retry —
            // is what unifies the two engine entry points.
            Err(transport) => {
                self.sync_metrics.degrade(SyncReason::EngineRetry);
                self.sync_metrics.engine_transient_retry.inc();
                warn!(
                    error = %transport,
                    "transient engine-API import transport error; degraded + deferring to \
                     reconvergence (engine stays up — Decision A, no self-crash)"
                );
                return Ok(false);
            }
        };
        if !(status.is_valid() || status.is_syncing()) {
            // #15 SafetyHalt (Phase 3): under the new_payload fallback an `Invalid`
            // import means local derivation diverged from reth's re-execution —
            // halt (verify-only, stay observable) rather than exit.
            self.safety_halt.engage(SyncReason::ElInvalid);
            return Err(eyre::eyre!(
                "EL rejected derived block (local derivation diverged?): `{status:?}`; SafetyHalt"
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
    use alloy_primitives::{Address, Bytes, U256};
    use alloy_rpc_types_engine::{ForkchoiceUpdated, PayloadStatus, PayloadStatusEnum};
    use commonware_runtime::{deterministic, Runner as _};
    use reth_ethereum_primitives::TransactionSigned;
    use reth_primitives_traits::SealedBlock as RethSealed;
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };

    type RethExecBlock = RethSealed<reth_ethereum_primitives::Block>;

    fn sample_order(parent: Digest, height: u64, result: B256) -> OrderBlock {
        OrderBlock {
            parent,
            height,
            proposal_view: 0,
            timestamp: 1_700_000_000 + height,
            fee_recipient: Address::ZERO,
            gas_limit: 30_000_000,
            extra_data: Bytes::new(),
            result,
            txs: Vec::new(),
            beacon_outcome: None,
            dkg_logs: Vec::new(),
            parent_seed: None,
        }
    }

    /// The FLUSH CHILD for `parent`: under the one-block-lookahead pipeline a
    /// height only derives when its child is delivered, so tests append one
    /// extra linked block (whose own ack stays HELD — the pipeline's steady
    /// state). `witness` becomes the child's `parent_seed`, i.e. the seed the
    /// executor derives `parent` with.
    fn child_of(parent: &OrderBlock, witness: Option<crate::beacon::seed::Seed>) -> OrderBlock {
        OrderBlock {
            parent_seed: witness,
            ..sample_order(parent.digest(), parent.height + 1, B256::ZERO)
        }
    }

    /// Build a self-consistent OrderBlock chain `(anchor+1 ..= anchor+count)`
    /// whose `result` field commits the hash the [`FakeDeriver`] WILL derive at
    /// `height − K` (ZERO in the pre-activation window) — so the executor's
    /// trustless result cross-check passes. Mirrors `FakeDeriver`'s derive shape
    /// (`sealed_at(parent_evm_hash, height, digest)`) exactly.
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

    /// `discriminator` (the ordering digest) is folded into `extra_data` so two
    /// sibling orders at the same (parent, height) seal to DISTINCT block hashes
    /// — required to observe a speculative rollback (sibling reorg).
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

    /// Fold a notarization/finalization seed into the ordering digest exactly as
    /// production folds `prev_randao = H(threshold-sig)` into the derived header's
    /// mix_hash: two DIFFERENT seeds for the SAME ordering block (identical digest)
    /// seal to DISTINCT executed hashes, mirroring the real chain. A `None`
    /// (no-beacon) seed leaves the digest untouched, so every existing seedless
    /// test seals byte-identically to before this fold existed.
    fn seed_folded_discriminator(digest: Digest, seed: &Option<crate::beacon::seed::Seed>) -> B256 {
        match seed {
            Some(s) => alloy_primitives::keccak256(
                [
                    digest.0.as_slice(),
                    crate::beacon::seed::prev_randao_from_seed(s).as_slice(),
                ]
                .concat(),
            ),
            None => digest.0,
        }
    }

    /// Models reth's by-HASH header-index lag — the parent-visibility race. A
    /// backfilled block is visible by NUMBER (`executed_hash`) immediately, but
    /// the by-HASH read the deriver's `derive_sync` performs on the parent
    /// (`header(parent_hash)`) only resolves once an FCU has canonicalized the
    /// segment. Heights ≤ `frontier` are by-hash-visible; `frontier` defaults to
    /// `u64::MAX` (lag disabled), so existing tests are unaffected and a test
    /// lowers it to exercise the race. Shared (cloned) between `FakeChain` (read),
    /// `FakeDeriver` (gate) and `FakeBeacon` (an FCU advances it).
    #[derive(Clone)]
    struct ByHashVisibility {
        hash_height: Arc<Mutex<BTreeMap<B256, u64>>>,
        frontier: Arc<Mutex<u64>>,
    }

    impl Default for ByHashVisibility {
        fn default() -> Self {
            Self {
                hash_height: Arc::new(Mutex::new(BTreeMap::new())),
                frontier: Arc::new(Mutex::new(u64::MAX)),
            }
        }
    }

    impl ByHashVisibility {
        fn register(&self, height: u64, hash: B256) {
            self.hash_height.lock().unwrap().insert(hash, height);
        }
        /// `true` iff reth would resolve `header(hash)`. An untracked hash is
        /// treated as visible (only the explicitly-modelled segment participates).
        fn visible(&self, hash: B256) -> bool {
            let frontier = *self.frontier.lock().unwrap();
            if frontier == u64::MAX {
                return true;
            }
            match self.hash_height.lock().unwrap().get(&hash) {
                Some(&h) => h <= frontier,
                None => true,
            }
        }
        /// Model an FCU(head): reth canonicalizes `[.., head]` by hash. Raises the
        /// frontier to the head's tracked height (no-op for an untracked head).
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
    /// (modelling new_payload+FCU canonicalization), the ExecutedChain
    /// reads — mirrors the provider-backed production impl. `vis` carries the
    /// by-HASH visibility lag model (default-disabled).
    #[derive(Clone, Default)]
    struct FakeChain {
        canonical: Arc<Mutex<BTreeMap<u64, B256>>>,
        /// The FINALIZED-execution cursor the executor advances past the
        /// canonical postcondition — mirrors the provider-backed production
        /// store (tier-F = canonical chain below the cursor).
        finalized: crate::application::FinalizedCursor,
        vis: ByHashVisibility,
        /// Pre-fix reth `InsertExecutedBlock` contract violation model: while > 0,
        /// an import at a height that ALREADY has a DIFFERENT canonical hash (a
        /// same-height sibling reorg) is silently dropped — the canonical map keeps
        /// the old hash — and the counter decrements. The soak3-fork EL behavior.
        sibling_drops: Arc<Mutex<u32>>,
        /// Landing model switch (family-5 gap-1 fidelity): when set, the DERIVER
        /// no longer lands blocks into the canonical map — landing happens only
        /// on a SUCCESSFUL `import_derived` (see `FakeBeacon::land_chain`),
        /// mirroring the real EL where a transport-failed `InsertExecutedBlock`
        /// leaves nothing behind. Default off (land-at-derive, the historical
        /// model most tests rely on). Armed via `Fixture::gate_landing_on_import`.
        land_on_import: Arc<std::sync::atomic::AtomicBool>,
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
            self.canonical.lock().unwrap().get(&height).copied()
        }
        fn finalized_executed_hash(&self, height: u64) -> Option<B256> {
            // Canonical chain below the cursor — mirrors `ProviderExecutedChain`.
            self.finalized
                .resolve(height, |h| self.spec_executed_hash(h))
        }
        fn advance_finalized(&self, height: u64) {
            self.finalized.advance(height);
        }
    }

    type SeedsSeen = Arc<Mutex<Vec<(u64, Option<crate::beacon::seed::Seed>)>>>;

    #[derive(Clone)]
    struct FakeDeriver {
        chain: FakeChain,
        /// Records the (height, seed) passed to each `derive_and_execute` so a
        /// test can assert the cert-recovered seed actually reaches the deriver.
        /// Mutex<Vec> so it survives the deriver clone (Arc-shared).
        seeds_seen: SeedsSeen,
    }

    impl FakeDeriver {
        fn new(chain: FakeChain) -> Self {
            Self {
                chain,
                seeds_seen: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl DerivedBlockBuilder for FakeDeriver {
        type Derived = RethExecBlock;

        async fn derive_and_execute(
            &self,
            order: OrderBlock,
            parent_evm_hash: B256,
            seed: Option<crate::beacon::seed::Seed>,
        ) -> eyre::Result<RethExecBlock> {
            // Fold the seed into the sealed hash (prev_randao→mix_hash model)
            // BEFORE `seed` is moved into `seeds_seen`, so notarize-round vs
            // finalize-round divergence is observable in-test.
            let discriminator = seed_folded_discriminator(order.digest(), &seed);
            self.seeds_seen.lock().unwrap().push((order.height, seed));
            // Model derive_sync's by-HASH parent read: a parent not yet canonical
            // by hash is `ParentHeaderMissing`. Default frontier = MAX ⇒ always
            // visible (no-op for tests that don't exercise the lag).
            if !self.chain.vis.visible(parent_evm_hash) {
                return Err(eyre::eyre!(
                    "parent header {parent_evm_hash} not yet visible by hash \
                     (ParentHeaderMissing)"
                ));
            }
            let sealed = sealed_at(parent_evm_hash, order.height, discriminator);
            // Pre-fix reth model (`sibling_drops` armed): a SAME-HEIGHT SIBLING
            // import is silently dropped — the derive succeeds but the canonical
            // map keeps the old hash (the soak3-fork EL contract violation the
            // try_derive postcondition must survive).
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
            // Gated-landing model (gap-1 fidelity): the derive alone lands
            // nothing — a successful import does (FakeBeacon::land).
            if self
                .chain
                .land_on_import
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Ok(sealed);
            }
            // Last writer wins, modelling a reth reorg: a finalized sibling
            // derived after a speculative one replaces the canonical hash.
            self.chain
                .canonical
                .lock()
                .unwrap()
                .insert(order.height, sealed.hash());
            // A derived (new_payload'd + FCU'd) block is canonical ⇒ visible by
            // hash, so it can serve as the next block's by-hash parent.
            self.chain.vis.register(order.height, sealed.hash());
            self.chain.vis.canonicalize_up_to(sealed.hash());
            Ok(sealed)
        }
    }

    #[derive(Clone, Default)]
    struct FakeBeacon {
        fcu_calls: Arc<Mutex<Vec<ForkchoiceState>>>,
        new_payload_calls: Arc<Mutex<Vec<RethExecBlock>>>,
        /// Override for the `fork_choice_updated` status; `None` ⇒ Valid. Set to
        /// drive SYNCING / INVALID through the FCU gate.
        fcu_status: Arc<Mutex<Option<PayloadStatusEnum>>>,
        /// #14: leading FCU calls that return a transport `Result::Err` (an RPC/channel
        /// blip) before succeeding — decremented per call. Models the retryable
        /// transport half of the split (distinct from a semantic `Ok(Invalid)`).
        fcu_transport_errs: Arc<Mutex<u32>>,
        /// Override for the `import_derived` status; `None` ⇒ Valid.
        import_status: Arc<Mutex<Option<PayloadStatusEnum>>>,
        /// Gap-1 (family 5): leading `import_derived` calls that return a
        /// transport `Err(TransportError)` (a closed engine channel) before
        /// succeeding — decremented per call. Models the import transport half of
        /// the split; the executor must degrade + defer, NOT actor-death.
        import_transport_errs: Arc<Mutex<u32>>,
        /// `Some(chain)` when landing is GATED on a successful import (see
        /// `FakeChain::land_on_import`): a `Valid` import inserts the block into
        /// the canonical map + visibility. `None` = the default land-at-derive
        /// model. Armed via `Fixture::gate_landing_on_import`.
        land_chain: Arc<Mutex<Option<FakeChain>>>,
        /// By-hash visibility shared with `FakeChain`/`FakeDeriver`: an FCU
        /// canonicalizes `[.., head]` by hash (the visibility model). Default-disabled.
        vis: ByHashVisibility,
    }

    impl BeaconEngineLike for FakeBeacon {
        type ExecutionData = RethExecBlock;

        async fn fork_choice_updated(
            &self,
            state: ForkchoiceState,
        ) -> Result<ForkchoiceUpdated, crate::fault::TransportError> {
            {
                let mut errs = self.fcu_transport_errs.lock().unwrap();
                if *errs > 0 {
                    *errs -= 1;
                    // A transport blip: reth was never reached, so nothing is
                    // recorded/canonicalized — the caller must retry.
                    return Err(crate::fault::TransportError::new(
                        "simulated engine-API transport blip",
                    ));
                }
            }
            self.fcu_calls.lock().unwrap().push(state);
            // Model reth: the FCU makes the segment up to `head` visible by hash.
            self.vis.canonicalize_up_to(state.head_block_hash);
            let status = self
                .fcu_status
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(PayloadStatusEnum::Valid);
            Ok(ForkchoiceUpdated::from_status(status))
        }

        async fn import_derived(
            &self,
            data: RethExecBlock,
        ) -> Result<PayloadStatus, crate::fault::TransportError> {
            {
                let mut errs = self.import_transport_errs.lock().unwrap();
                if *errs > 0 {
                    *errs -= 1;
                    // A closed engine channel: nothing imported — the executor
                    // degrades + defers to reconvergence (never actor-death).
                    return Err(crate::fault::TransportError::new(
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
            // Gated-landing model: a SUCCESSFUL (Valid) import is what lands the
            // block — mirrors the real EL where the transport-failed insert above
            // left nothing behind.
            if let Some(chain) = self.land_chain.lock().unwrap().as_ref() {
                if status == PayloadStatusEnum::Valid {
                    let (height, hash) = (data.number(), data.hash());
                    chain.canonical.lock().unwrap().insert(height, hash);
                    chain.vis.register(height, hash);
                    chain.vis.canonicalize_up_to(hash);
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
        /// zero-I/O-fast-path witness (`awaiting_child` must derive from the
        /// child body in hand, never round-trip the marshal).
        fetched: Arc<Mutex<Vec<u64>>>,
        /// Heights passed to `hint_finalization`, in call order.
        hints: Arc<Mutex<Vec<u64>>>,
        /// Heights passed to `set_floor`, in call order (the re-jump recorder).
        floors: Arc<Mutex<Vec<u64>>>,
        /// Heights passed to `store_verified_finalization`, in call order. Recorded
        /// in the SAME `Vec` ordering domain as `floors` is compared against, so a
        /// test can assert boundary seeding happened strictly BEFORE the floor rose —
        /// which is the whole correctness argument for the injection.
        stored: Arc<Mutex<Vec<u64>>>,
        /// Interleaved `("store"|"floor", height)` trace, so ordering between the two
        /// is assertable without reasoning about two separate vectors.
        store_floor_order: Arc<Mutex<Vec<(&'static str, u64)>>>,
        /// Biased-select escape model (the `marshal_floor` stale-dispatch guard):
        /// a mailbox sender + a canned OLD-range inventory, armed via
        /// [`Self::arm_stale_escape`]. On `set_floor(f)` every inventory block at
        /// height ≤ f is dispatched into the executor mailbox — modelling the acks
        /// that `reseed_forward`'s disposals free, whose slots the marshal's biased
        /// select fills with the next OLD blocks BEFORE it processes `SetFloor`.
        /// The escaped `Exact` waiters are retained so the guard's `acknowledge()`
        /// never hits a dropped receiver. Inert (empty/`None`) on every other test.
        dispatch: Arc<Mutex<Option<Mailbox>>>,
        stale_inventory: Arc<Mutex<Vec<OrderBlock>>>,
        escaped_waiters: Arc<Mutex<Vec<commonware_utils::acknowledgement::ExactWaiter>>>,
    }

    impl FakeMarshal {
        /// Arm the biased-select escape: `inventory` blocks at height ≤ the floor
        /// are dispatched into `mailbox` when `reseed_forward` calls `set_floor`.
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
        /// default (the re-jump tests set it via `with_re_jump`).
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
        /// Self-heal metrics handle the built actor's `Config` carries — exposed so
        /// #14/#1 tests assert the `engine_retry` / `auth_rotate` gauges + counters.
        sync_metrics: SyncMetrics,
        /// Fork-safety latch the built actor's `Config` carries — exposed so the
        /// Phase-3 SafetyHalt tests assert it engages on divergence / EL-Invalid.
        safety_halt: crate::sync_metrics::SafetyHalt,
        /// FCU-heartbeat interval. Default 60 s so heartbeats never interfere with
        /// fast tests; the park tests that rely on the heartbeat re-poke lower it
        /// (the deterministic clock steps ~1 ms/iteration in real time, so a large
        /// virtual interval is real seconds) via `with_fcu_heartbeat`.
        fcu_heartbeat: Duration,
        /// `SeedStore` handed to the built actor (`None` by default — the §4.1
        /// re-canonicalise arm then degrades to skip-speculation on a
        /// spin-round notarization). Set via `with_seed_store`.
        seed_store: Option<crate::beacon::certify::SeedStore>,
        /// Block→epoch map handed to the built actor. Default: a single huge
        /// epoch so every test height maps to epoch 0 (matching the
        /// `Epoch::new(0)` seed rounds the tests build). The epoch-boundary
        /// eager-derive tests override via `with_epocher`.
        epocher: crate::epocher::OriginEpocher,
        /// Restart-seed override for `last_execution_finalized_height` (the reth
        /// head = `provider.last_block_number()`). `None` ⇒ the historical
        /// `anchor_height` (head == acked == anchor). The
        /// `ordering_finalized`-seed test decouples the two (head ≫ acked with a
        /// speculative tail) to pin that the cursor seeds from the ACKED cursor.
        last_execution: Option<u64>,
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
            // Share the by-hash visibility so a beacon FCU advances exactly the
            // frontier the deriver gates on (the visibility model is coherent end-to-end).
            let beacon = FakeBeacon {
                vis: chain.vis.clone(),
                ..Default::default()
            };
            // The latch shares the SAME `SyncMetrics` gauge family, exactly as
            // production wires it (`SafetyHalt::new(sync_metrics.clone())`), so a test
            // reading `fx.sync_metrics` sees the gauge the latch raised.
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
                seed_store: None,
                epocher: crate::epocher::OriginEpocher::new(
                    0,
                    std::num::NonZeroU64::new(1 << 40).expect("nonzero"),
                ),
                last_execution: None,
            }
        }

        /// Override `last_execution_finalized_height` (the reth head seed),
        /// decoupling it from the anchor. Set BEFORE `build`.
        fn with_last_execution(mut self, height: u64) -> Self {
            self.last_execution = Some(height);
            self
        }

        /// Override the block→epoch map (the epoch-boundary eager-derive
        /// tests need real, small epochs). Set BEFORE `build`.
        fn with_epocher(mut self, epocher: crate::epocher::OriginEpocher) -> Self {
            self.epocher = epocher;
            self
        }

        /// Give the built actor a `SeedStore` (the §4.1 re-canonicalise byte
        /// source). Set BEFORE `build`.
        fn with_seed_store(mut self, store: crate::beacon::certify::SeedStore) -> Self {
            self.seed_store = Some(store);
            self
        }

        /// Inject the steady-state re-jump callback the built actor's `Config`
        /// will carry. Set BEFORE `build`.
        fn with_re_jump(self, re_jump: ReJump) -> Self {
            *self.re_jump.lock().unwrap() = Some(re_jump);
            self
        }

        fn with_boundary_fetch(self, fetch: crate::cert_follow::BoundaryFetchFn) -> Self {
            *self.boundary_fetch.lock().unwrap() = Some(fetch);
            self
        }

        /// Record every epoch-entry height the built actor drives into `sink`.
        /// Set BEFORE `build`.
        fn with_boundary_enter(mut self, sink: Arc<Mutex<Vec<u64>>>) -> Self {
            self.boundary_enter = Arc::new(move |h| sink.lock().unwrap().push(h));
            self
        }

        /// Record every read-floor height the built actor publishes into `sink`.
        /// Sharing one sink with [`Self::with_boundary_enter`] also records the
        /// ORDER of the two seams. Set BEFORE `build`.
        fn with_boundary_read_floor(mut self, sink: Arc<Mutex<Vec<u64>>>) -> Self {
            self.boundary_read_floor = Arc::new(move |h| {
                let sink = sink.clone();
                Box::pin(async move { sink.lock().unwrap().push(h) })
            });
            self
        }

        /// Switch to the GATED-landing model (family-5 gap-1 fidelity): the
        /// deriver stops landing blocks into the canonical map; only a
        /// SUCCESSFUL `import_derived` lands them — a transport-failed import
        /// genuinely leaves the block un-landed, so the postcondition loop has
        /// something real to converge on. Set BEFORE `build`.
        fn gate_landing_on_import(&self) {
            self.chain
                .land_on_import
                .store(true, std::sync::atomic::Ordering::SeqCst);
            *self.beacon.land_chain.lock().unwrap() = Some(self.chain.clone());
        }

        /// Shrink the FCU-heartbeat interval so a park test that depends on the
        /// heartbeat re-poke resolves in a few virtual ms (≈ real ms) instead of
        /// real seconds. Set BEFORE `build`.
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
            // The fixtures build chains anchored AT activation (anchor ==
            // activation), so the cross-check window is unchanged by the split.
            self.build_with_activation(ctx, anchor_height, anchor_height, last_consensus)
        }

        /// `build` with the cold-start anchor DECOUPLED from the chain activation
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
                    initial_marshal_floor: 0,
                    boundary_fetch: self.boundary_fetch.lock().unwrap().clone(),
                    boundary_enter: self.boundary_enter.clone(),
                    boundary_read_floor: self.boundary_read_floor.clone(),
                    dpos_activation_block: activation,
                    fcu_pace: Duration::from_millis(0),
                    peers_for_finalization: std::sync::Arc::new(dummy_peers),
                    beacon_metrics: crate::beacon::metrics::BeaconMetrics::default(),
                    sync_metrics: self.sync_metrics.clone(),
                    safety_halt: self.safety_halt.clone(),
                    spawn_unblocked: std::sync::Arc::new(tokio::sync::Notify::new()),
                    re_jump: self.re_jump.lock().unwrap().clone(),
                    seed_store: self.seed_store.clone(),
                    epocher: self.epocher.clone(),
                },
            )
        }
    }

    /// One deterministic dummy peer for the finalization-hint target set
    /// (FakeMarshal ignores the targets' contents — it only records the call).
    fn dummy_peers() -> Option<NonEmptyVec<PeerPubkey>> {
        use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
        let pk = Ed25519PrivateKey::from_seed(99).public_key();
        NonEmptyVec::try_from(vec![pk]).ok()
    }

    /// A real recovered threshold seed for `round` (the executor passes it
    /// through verbatim; it never re-verifies, so any valid `Seed` suffices).
    fn real_seed(round: commonware_consensus::types::Round) -> crate::beacon::seed::Seed {
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
        crate::beacon::seed::Seed {
            target_round: round,
            signature: recover_seed::<N3f1>(&sharing, &partials).expect("recover seed"),
        }
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

    /// Pin the SafetyHalt PARK posture (the marshal-zombie fix): the executor is
    /// still RUNNING (unresolved handle — pre-fix it exited, dropping every ack),
    /// the halted block's ack is RETAINED un-resolved (neither Ok nor Canceled,
    /// so the marshal's `last_processed_height` cannot advance past the diverged
    /// height and the marshal never sees the fatal Canceled), and an ack
    /// delivered AFTER the halt engaged (the marshal keeps dispatching up to its
    /// 16-block window) is retained too.
    async fn assert_parked_retaining_acks(
        ctx: &deterministic::Context,
        mut handle: Handle<()>,
        mut waiter: commonware_utils::acknowledgement::ExactWaiter,
        mailbox: &Mailbox,
        halt: &crate::sync_metrics::SafetyHalt,
        post_halt_order: OrderBlock,
    ) {
        wait_until(ctx, "SafetyHalt engaged", || halt.is_engaged()).await;
        // Let the executor reach the park loop before probing ack/handle state.
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

    /// A `SpecNotarized` command for `order` (seedless; the executor keys
    /// speculation off the fetched block's height, not the round).
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

    /// A pure `LastCanonicalized` literal seeded at the anchor (all three tiers
    /// equal), used by the `update_safe` unit tests below.
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

    // `update_finalized` (result tier) + `update_safe` (ordering tier) advance
    // their OWN monotone guards; `finalized_height ≤ safe_height ≤ head_height`
    // and the three hashes stay consistent with the heights after each op.
    #[test]
    fn finalized_safe_head_ancestry_holds() {
        let h10 = B256::repeat_byte(0x10);
        let mut lc = lc_at(10, h10);

        // ordering-final advances to 13 (safe + head), result-final still 10.
        let h13 = B256::repeat_byte(0x13);
        lc = lc
            .update_safe(Height::new(13), h13)
            .update_head(Height::new(13), h13);
        assert_eq!(lc.safe_height, Height::new(13));
        assert_eq!(lc.forkchoice.safe_block_hash, h13);
        assert_eq!(lc.head_height, Height::new(13));
        assert_eq!(lc.finalized_height, Height::new(10));
        assert!(lc.finalized_height <= lc.safe_height && lc.safe_height <= lc.head_height);

        // result-final catches up to 11 (= 14 − K), safe to the new tip 14.
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

    // An out-of-order / transient lower ordering-final delivery must NOT roll
    // `safe` backward (its own monotone guard, distinct from `finalized_height`).
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
    // `H == safe_height`) lets the HASH follow onto the freshly-finalized tip —
    // never pinning `safe` to an orphaned sibling.
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

    // Pre-K window: finalized stays clamped to the anchor while head follows
    // the derived tip; from anchor+K onward finalized = derived hash of −K.
    #[test]
    fn two_tier_finalized_lags_head_by_k_clamped_to_anchor() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // Pipeline shift: a height only derives when its child arrives, so
            // send the whole chain (+1 flush child) and await the ack of the
            // last DERIVED height (ANCHOR+K+1; the flush child stays held).
            let chain = result_consistent_chain(ANCHOR, fx.anchor_hash, K + 2);
            let mut waiters = Vec::new();
            for order in &chain {
                let (msg, waiter) = finalize_msg(order.clone());
                mailbox.send(msg).expect("send");
                waiters.push(waiter);
            }
            waiters
                .swap_remove(K as usize)
                .await
                .expect("ack of ANCHOR+K+1 (derived on the flush child's arrival)");

            {
                let fcus = fx.beacon.fcu_calls.lock().unwrap();
                // Heights ANCHOR+1..=ANCHOR+K-1: finalized pinned to the anchor
                // while safe (ordering-final) climbs to each just-finalized tip.
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
                // Height ANCHOR+K: result_final = ANCHOR (still the anchor hash);
                // height ANCHOR+K+1: result_final = ANCHOR+1 = derived hash.
                let derived_anchor_plus_1 = fx.chain.spec_executed_hash(ANCHOR + 1).unwrap();
                let ordering_tip = fx.chain.spec_executed_hash(ANCHOR + K + 1).unwrap();
                let last = fcus.last().unwrap();
                assert_eq!(last.finalized_block_hash, derived_anchor_plus_1);
                // safe = the ordering-final tip = head (no spec lead), K ahead of
                // finalized once past the clamp.
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

    // #14 SELF-HEAL: a transient TRANSPORT error at the finalize FCU (a Result::Err,
    // an RPC/channel blip) is retried FOREVER — the block still acks (the loop
    // survives, no shutdown), `engine_transient_retry_total` counts the retries, and
    // the `engine_retry` gauge clears on success.
    #[test]
    fn transient_finalize_fcu_transport_error_retries_then_acks() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            // The next 3 finalize FCUs blip with a transport error before succeeding.
            *fx.beacon.fcu_transport_errs.lock().unwrap() = 3;
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // A single pre-K finalize (result ZERO ⇒ no cross-check): its finalize
            // FCU eats 3 transport blips, retries, and still acks. The flush
            // child triggers the derive (pipeline shift).
            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let (msg, waiter) = finalize_msg(order.clone());
            mailbox.send(msg).expect("send");
            let (flush, _w_flush) = finalize_msg(child_of(&order, None));
            mailbox.send(flush).expect("send flush child");
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

    // GAP-1 (family 5): an import-derived TRANSPORT error is now the SAME
    // `FaultClass::TransientExternal(EngineRetry)` class as its FCU sibling —
    // degraded + counted, engine STAYS UP — instead of the former
    // `?`→actor-death asymmetry. The block does NOT die: the finalized path's
    // canonical POSTCONDITION reconvergence lands it (here the derive already made
    // it canonical, so the block acks immediately), and the transport failure is
    // visible in `engine_transient_retry_total`.
    #[test]
    fn import_transport_error_is_degraded_not_fatal_and_counted() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            // The next import_derived blips with a closed-channel transport error.
            *fx.beacon.import_transport_errs.lock().unwrap() = 1;
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // A single pre-K finalize (result ZERO ⇒ no cross-check). The flush
            // child triggers the held block's derive → import (which blips).
            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let (msg, waiter) = finalize_msg(order.clone());
            mailbox.send(msg).expect("send");
            let (flush, _w_flush) = finalize_msg(child_of(&order, None));
            mailbox.send(flush).expect("send flush child");
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

    // GAP-1 fidelity (gated landing — the real EL model where a transport-failed
    // `InsertExecutedBlock` leaves NOTHING behind): while imports keep failing
    // transport, the block genuinely does not land — the postcondition loop
    // stays converging (`finalize_apply=1`), and the ack is neither taken (would
    // durably skip an un-landed height) nor Canceled (kills the marshal). Once
    // transport heals, the next re-apply import LANDS the block and releases the
    // ack.
    #[test]
    fn unlanded_import_transport_error_holds_the_ack_until_reapply_lands() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const H: u64 = ANCHOR + 1;
            let fx = Fixture::new(ANCHOR);
            fx.gate_landing_on_import();
            // Every import fails transport until the test heals it explicitly.
            *fx.beacon.import_transport_errs.lock().unwrap() = u32::MAX;
            let order = sample_order(Digest(B256::ZERO), H, B256::ZERO);
            // The postcondition re-apply loop re-fetches the order by height.
            fx.marshal.canned.lock().unwrap().insert(H, order.clone());
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            let (msg, waiter) = finalize_msg(order.clone());
            mailbox.send(msg).expect("send");
            let (flush, _w_flush) = finalize_msg(child_of(&order, None));
            mailbox.send(flush).expect("send flush child");

            // Mid-convergence: degraded-visible, block un-landed, ack pending.
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

            // Heal the transport: the next re-apply import lands the block.
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

    // GAP-2 (family 5): a heartbeat FCU transport failure was a bare `warn!`
    // invisible to the taxonomy; it is now `TransientExternal(EngineRetry)` —
    // fire-and-forget (the next tick is the retry, no loop) but COUNTED +
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

            actor.send_forkchoice_update_heartbeat().await;
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

            // A subsequent clean heartbeat clears the gauge (fire-and-forget: the
            // NEXT tick is the retry, no in-place loop).
            actor.send_forkchoice_update_heartbeat().await;
            assert_eq!(
                fx.sync_metrics.degraded_value(SyncReason::EngineRetry),
                0,
                "a clean heartbeat recovers the reason"
            );
        });
    }

    // #14/#15 split: a SEMANTIC `Ok(Invalid)` verdict at the finalize FCU is NOT a
    // transport error — UNLIKE the retried transport `Err`, it is the #15 Phase-3
    // SafetyHalt: the block does NOT ack, the executor stops driving reth (the
    // subsystem exits so the OuterEngine supervisor can park the rest), and the
    // `el_invalid` fork-safety latch engages (demote-verify-only, `l1`/RPC stay up).
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
            let flush = child_of(&refused, None);
            let post_halt = sample_order(flush.digest(), ANCHOR + 3, B256::ZERO);
            let (msg, waiter) = finalize_msg(refused);
            mailbox.send(msg).expect("send");
            // The flush child triggers the derive (and the Invalid FCU). Its own
            // ack sits in `awaiting_child` when the halt engages — it must be
            // retained too (the park_halted awaiting_child clause).
            let (flush_msg, mut flush_waiter) = finalize_msg(flush);
            mailbox.send(flush_msg).expect("send flush child");

            // An Ok(Invalid) finalize FCU must NOT ack — the block is refused and
            // the executor parks retaining the ack (SafetyHalt, not a plain crash).
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
                "the HELD child's ack (awaiting_child) must be retained by park_halted too"
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

    // An OrderBlock whose attested `result` disagrees with the locally-derived
    // hash at `height − K` means this node would serve a fork; `try_derive` engages
    // the #2/#3 SafetyHalt (the same trustless property `FluentApp::verify` enforces
    // on the BFT path). The block does NOT ack — and the executor PARKS retaining
    // the ack un-resolved (halted-but-observable) instead of exiting: an exit
    // drops the `Exact`, the marshal reads the cancellation as fatal and dies,
    // and the "stay up" posture degrades to a zombie serving nobody.
    #[test]
    fn result_divergence_engages_safety_halt_and_parks() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // Pre-K window: result MUST be ZERO (no cross-check fires). Under the
            // pipeline shift each height derives when its child arrives, so the
            // divergent block at ANCHOR+K only derives (and halts) when its own
            // child is delivered.
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
            let halt_trigger = child_of(&divergent, None);
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

    // FIXED behavior of the formerly seed-blind fork-safety bug (bundle block
    // 5252), restated under the WITNESS: a block speculatively executed with a
    // seed of round A but whose child carries a witness of a DIFFERENT round B.
    // `spec_executed` records the speculation's seed ROUND, and
    // `correctly_speculated` requires the stored round to equal the witness
    // round. On the A≠B mismatch (after §4.1 an ANOMALY — both sides should be
    // Round(Ep, proposal_view)) the executor RE-DERIVES SPEC_H with the witness
    // seed_B (the agreed value) and reorgs the head onto it — so K blocks later
    // the committee-attested result (seed_B → hash_B) MATCHES the locally
    // executed hash and NO `ResultDivergence` SafetyHalt fires.
    #[test]
    fn spec_seed_mismatch_rederives_with_witness_seed_no_halt() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const SPEC_H: u64 = ANCHOR + 1; // the notarized-then-finalized height
            let fx = Fixture::new(ANCHOR);
            let anchor_hash = fx.anchor_hash;

            // Two seeds for the SAME ordering block: spec round A ≠ witness
            // round B ⇒ (via the prev_randao→mix_hash fold) DISTINCT hashes.
            let seed_a = real_seed(Round::new(Epoch::new(0), View::new(SPEC_H)));
            let seed_b = real_seed(Round::new(Epoch::new(0), View::new(SPEC_H + 1)));

            // The ordering block finalized at SPEC_H (result ZERO — pre-K
            // window). `proposal_view == seed_a`'s view so the §4.1
            // re-canonicalisation keeps the spec seed as-is.
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

            // Ordering chain SPEC_H..=SPEC_H+K+1. Every child carries a witness
            // for its parent (a beacon-active chain stays beacon-active — the
            // downgrade refusal is monotone): SPEC_H's witness is seed_B (the
            // mismatch under test); later witnesses are per-height seeds.
            let seed_1 = real_seed(Round::new(Epoch::new(0), View::new(SPEC_H + 1)));
            let seed_2 = real_seed(Round::new(Epoch::new(0), View::new(SPEC_H + 2)));
            let seed_3 = real_seed(Round::new(Epoch::new(0), View::new(SPEC_H + K)));
            let order_h1 = OrderBlock {
                parent_seed: Some(seed_b.clone()),
                ..sample_order(order_h.digest(), SPEC_H + 1, B256::ZERO) // pre-K → ZERO
            };
            let order_h2 = OrderBlock {
                parent_seed: Some(seed_1.clone()),
                ..sample_order(order_h1.digest(), SPEC_H + 2, anchor_hash) // Height(ANCHOR)
            };
            let order_hk = OrderBlock {
                parent_seed: Some(seed_2.clone()),
                ..sample_order(order_h2.digest(), SPEC_H + K, hash_b) // attests hash_B
            };
            let flush = OrderBlock {
                parent_seed: Some(seed_3.clone()),
                ..sample_order(order_hk.digest(), SPEC_H + K + 1, B256::ZERO)
            };

            // Only the speculated block is fetched-by-digest (spec_execute).
            fx.marshal.canned.lock().unwrap().insert(SPEC_H, order_h.clone());

            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // (1) Speculatively execute SPEC_H with seed_A → hash_A.
            mailbox
                .send(Message {
                    cause: Span::current(),
                    command: Command::SpecNotarized(Box::new(Notarized {
                        digest: order_h.digest(),
                        seed: Some(seed_a.clone()),
                    })),
                })
                .expect("send spec@A");

            // CRUX of the result-gate fix (bundle-20260716T150148Z): once the
            // speculation lands, the SPECULATIVE head shows hash_A — but the
            // FINALIZED tier is still empty. A proposer/verifier sampling
            // `finalized_executed_hash(SPEC_H)` therefore gets `None` and SKIPS
            // the view; it can NEVER commit hash_A as the result at SPEC_H+K.
            wait_until(&ctx, "SPEC_H speculated to hash_A", || {
                fx.chain.spec_executed_hash(SPEC_H) == Some(hash_a)
            })
            .await;
            assert_eq!(
                fx.chain.finalized_executed_hash(SPEC_H),
                None,
                "finalized tier empty while only speculated — the gate reads None, not hash_A"
            );

            // (2) FINALIZE SPEC_H; its child (order_h1) carries the seed_B
            // witness. `correctly_speculated` sees stored round A ≠ witness
            // round B ⇒ re-derive → hash_B becomes canonical at SPEC_H.
            let (m, w) = finalize_msg(order_h.clone());
            mailbox.send(m).expect("send finalize SPEC_H");
            let (m1, w1) = finalize_msg(order_h1);
            mailbox.send(m1).expect("send finalize SPEC_H+1");
            w.await.expect("SPEC_H acks after re-derive with seed_B");

            // The seed-blind reuse is GONE: hash_B (the agreed witness value) is
            // canonical at SPEC_H, NOT the seed_A speculation.
            assert_eq!(
                fx.chain.spec_executed_hash(SPEC_H),
                Some(hash_b),
                "round mismatch re-derived SPEC_H with the witness seed (hash_B)"
            );
            // The FINALIZED tier now reflects the finalized sibling (hash_B),
            // NEVER the speculated hash_A: the result gate at SPEC_H+K commits
            // and cross-checks against hash_B, so the whole-committee SafetyHalt
            // of the bundle cannot recur.
            assert_eq!(
                fx.chain.finalized_executed_hash(SPEC_H),
                Some(hash_b),
                "finalized tier records the finalized sibling (hash_B), never the speculative hash_A"
            );
            // The deriver ran TWICE for SPEC_H — once at spec (seed_A), once at
            // finalize (a REAL re-derive with seed_B).
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

            // (3) Advance ordering past SPEC_H+K so the seed_B attestation
            // reaches the result cross-check (each height derives on its
            // child's arrival).
            for order in [order_h2, order_hk, flush] {
                let (m, _w) = finalize_msg(order);
                mailbox.send(m).expect("send chain");
            }
            w1.await.expect("intermediate ack");

            // (4) The SPEC_H+K block attests hash_B; the executor HOLDS hash_B ⇒
            // the cross-check passes and the chain advances past it.
            wait_until(&ctx, "SPEC_H+K derived", || {
                fx.chain.spec_executed_hash(SPEC_H + K).is_some()
            })
            .await;

            assert!(
                !fx.safety_halt.is_engaged(),
                "FIXED: re-derive with the witness seed keeps local == attested → NO halt"
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

    // Property (a) — the fast path is INTACT: when the speculation's seed round
    // EQUALS the finalization cert's seed round (the common case), the block is
    // KEPT (no re-derive, no head rollback) and no halt fires. Guards the added
    // seed-round check against regressing `correctly_speculated` toward "always
    // re-derive" (property (c)).
    #[test]
    fn spec_same_round_keeps_speculation_no_rederive() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const SPEC_H: u64 = ANCHOR + 1;
            let fx = Fixture::new(ANCHOR);
            let anchor_hash = fx.anchor_hash;
            // Notarization AND the child's witness carry the SAME-round seed
            // (the honest steady state: both are Round(Ep, proposal_view)).
            let seed = real_seed(Round::new(Epoch::new(0), View::new(SPEC_H)));

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
            // The child carries the SAME-round witness — its arrival triggers
            // SPEC_H's finalized reconcile.
            let (m_child, _w_child) = finalize_msg(child_of(&order_h, Some(seed.clone())));
            mailbox.send(m_child).expect("send flush child");
            w.await.expect("SPEC_H acks via the kept speculation");

            assert_eq!(
                fx.chain.spec_executed_hash(SPEC_H),
                Some(hash),
                "same-round finalize KEPT the speculation's hash (no re-derive)"
            );
            // The correctly-speculated arm ALSO records the finalized tier (the
            // spec hash the finalization CONFIRMED) — the steady-state gate
            // resolves immediately, no behaviour change vs the pre-fix happy path.
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

    // TDD REPRODUCTION of the soak3 fork @ height 9924 (epoch 79) — the REAL
    // mechanism, pinned from the failure bundle: the seed-round guard WORKED
    // (spec@View(90) ≠ fin@View(91) → re-derive with the finalization seed), but
    // the EL silently DROPPED the same-height sibling import (pre-fix reth
    // `InsertExecutedBlock` skipped any `number <= canonical_block_number()`) and
    // answered SYNCING to the reorg FCU — which `try_derive` tolerated as success.
    // The stale speculative sibling stayed canonical, every later parent lookup
    // (`executed_hash(height)`) extended it, and the ≥2f+1 spec-blind majority
    // attested the wrong result — the CORRECT minority SafetyHalted.
    //
    // The once-suspected alternative — a node holding a finalization whose round
    // differs from the quorum's — is UNREACHABLE at ≤f byzantine: commonware
    // `construct_nullify` refuses after an own finalize vote (simplex
    // voter/round.rs:327-336), so the finalization of round R and the
    // nullification of R required to re-propose the same height at R+1 cannot
    // both assemble (their vote sets are disjoint and 2·quorum > n + f).
    // Finalization per height is UNIQUE; the fork was purely the EL apply drop.
    //
    // This test arms the pre-fix EL model (`sibling_drops` = 2): the `try_derive`
    // canonical postcondition must keep RE-APPLYING (derive + import + FCU,
    // `dpos_sync_degraded{reason=finalize_apply}` raised while stuck) instead of
    // acking past the un-applied reorg — then ack once the EL actually serves the
    // finalized hash. K blocks later the attested result matches and NO SafetyHalt
    // fires. Pre-postcondition this test FAILS exactly like the soak: hash_A stays
    // canonical and the H+K attestation halts the (correct) node.
    #[test]
    fn finalized_sibling_reorg_survives_dropped_el_import() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const SPEC_H: u64 = ANCHOR + 1;
            let fx = Fixture::new(ANCHOR);
            let anchor_hash = fx.anchor_hash;

            // Spec round A ≠ witness round B (the agreed witness value).
            let seed_a = real_seed(Round::new(Epoch::new(0), View::new(SPEC_H)));
            let seed_b = real_seed(Round::new(Epoch::new(0), View::new(SPEC_H + 1)));

            // `proposal_view == seed_a`'s view keeps the §4.1 re-canonicalisation
            // a no-op for the speculation.
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

            // A beacon-active chain stays beacon-active: every child carries a
            // witness for its parent (the downgrade refusal is monotone).
            let seed_1 = real_seed(Round::new(Epoch::new(0), View::new(SPEC_H + 2)));
            let seed_2 = real_seed(Round::new(Epoch::new(0), View::new(SPEC_H + 3)));
            let seed_3 = real_seed(Round::new(Epoch::new(0), View::new(SPEC_H + 4)));
            let order_h1 = OrderBlock {
                parent_seed: Some(seed_b.clone()),
                ..sample_order(order_h.digest(), SPEC_H + 1, B256::ZERO) // pre-K → ZERO
            };
            let order_h2 = OrderBlock {
                parent_seed: Some(seed_1.clone()),
                ..sample_order(order_h1.digest(), SPEC_H + 2, anchor_hash) // commits ANCHOR
            };
            let order_hk = OrderBlock {
                parent_seed: Some(seed_2.clone()),
                ..sample_order(order_h2.digest(), SPEC_H + K, hash_b) // attests hash_B
            };
            let flush = OrderBlock {
                parent_seed: Some(seed_3.clone()),
                ..sample_order(order_hk.digest(), SPEC_H + K + 1, B256::ZERO)
            };

            // The marshal serves SPEC_H by digest (spec_execute) AND by height
            // (the postcondition re-apply loop's re-fetch).
            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(SPEC_H, order_h.clone());

            // Pre-fix reth: the first TWO same-height sibling imports are silently
            // dropped (the soak EL dropped every one; two drops prove the loop
            // RETRIES rather than merely re-attempting once).
            *fx.chain.sibling_drops.lock().unwrap() = 2;

            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // (1) Speculate SPEC_H at notarization round A → hash_A canonical.
            mailbox
                .send(Message {
                    cause: Span::current(),
                    command: Command::SpecNotarized(Box::new(Notarized {
                        digest: order_h.digest(),
                        seed: Some(seed_a.clone()),
                    })),
                })
                .expect("send spec@A");

            // (2) Finalize SPEC_H; its child carries the round-B witness: the
            // guard routes to the re-derive, whose sibling import the EL DROPS
            // twice; the ack must not fire until the re-apply loop lands hash_B.
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

            // (3) Advance past SPEC_H+K: the attested hash_B matches the local
            // chain → derives cleanly, NO SafetyHalt (pre-fix: ResultDivergence
            // here). Each height derives on its child's arrival.
            for order in [order_h2, order_hk, flush] {
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
    // activation) and derives the K-below-anchor blocks. Those are
    // POST-activation and carry real (non-zero) results — keying the
    // pre-activation window on the cold-start anchor instead of the chain
    // activation block mis-classifies them as pre-activation (expect ZERO) and
    // shuts the executor down (the smoke-byzantine-vrf full-node wedge).
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

            // Marshal floor = ANCHOR − K = 203 ⇒ first dispatched height 204,
            // BELOW the anchor (206) but ABOVE activation+K (195). Its result
            // commits the already-present derived hash at 204 − K = 201.
            let below_anchor = ANCHOR - K + 1; // 204
            let order = sample_order(Digest(B256::ZERO), below_anchor, result_at_201);

            let (actor, mailbox) = fx.build_with_activation(ctx, ANCHOR, ACTIVATION, ANCHOR);
            let handle = actor.start();

            let (msg, waiter) = finalize_msg(order.clone());
            mailbox
                .send(msg)
                .expect("send below-anchor post-activation block");
            let (flush, _w_flush) = finalize_msg(child_of(&order, None));
            mailbox.send(flush).expect("send flush child");
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
            let chain = result_consistent_chain(ANCHOR, fx.anchor_hash, 5);
            // Heights 1..=3 canned in the marshal (crash-recovery backfill).
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                for order in &chain[..3] {
                    canned.insert(order.height, order.clone());
                }
            }
            let (actor, mailbox) = fx.build(ctx, ANCHOR, 3);
            let handle = actor.start();

            // Live finalizes for heights 4+5 land BEFORE backfill drains (5 is
            // the flush child that triggers 4's derive).
            let (msg, waiter) = finalize_msg(chain[3].clone());
            mailbox.send(msg).expect("send");
            let (flush, _w_flush) = finalize_msg(chain[4].clone());
            mailbox.send(flush).expect("send flush child");
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
            // Heights 101..=103 exist ONLY in the marshal (not yet derived).
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                for order in &chain[..3] {
                    canned.insert(order.height, order.clone());
                }
            }
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // Deliver height 104 directly with an UNRESOLVABLE parent digest — its
            // real parent 103 is underived, so the gap-walk fills 101..103 first
            // (103's witness = the delivered block itself; the walk needs no
            // certs). The result still commits the derived hash at 101.
            let delivered = OrderBlock {
                parent: Digest(B256::ZERO),
                ..chain[3].clone()
            };
            let (msg, waiter) = finalize_msg(delivered.clone());
            mailbox.send(msg).expect("send");
            let (flush, _w_flush) = finalize_msg(child_of(&delivered, None));
            mailbox.send(flush).expect("send flush child");
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

    // A GAP block (filled by `derive_missing_prefix`, not the top-level delivery)
    // carries its OWN attested `result`; a forged value on a gap-range block must
    // fail loud just like the top-level cross-check — otherwise a
    // committee-attested wrong result on a gap block is imported unchecked (the
    // byzantine-vrf defense). Here ANCHOR+K (the first POST-pre-activation gap
    // height) commits a forged hash; the gap-walk derives it, the cross-check
    // engages the SafetyHalt, and the executor parks retaining the ack. This
    // pins the `?`-propagated halt path (the engage fires INSIDE
    // `derive_missing_prefix`, below the `inflight_ack` slot).
    #[test]
    fn gap_block_result_divergence_engages_safety_halt_and_parks() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let chain = result_consistent_chain(ANCHOR, fx.anchor_hash, K + 2);
            // Forge the `result` on the gap block at ANCHOR+K (index K-1): it is
            // the first gap height past the pre-activation window, so its cross-check
            // fires against the derived hash at ANCHOR (already canonical).
            let forged_idx = (K - 1) as usize;
            let forged = B256::repeat_byte(0xEE);
            assert_ne!(forged, fx.chain.spec_executed_hash(ANCHOR).unwrap());
            let mut forged_chain = chain.clone();
            forged_chain[forged_idx].result = forged;
            // All gap heights ANCHOR+1 ..= ANCHOR+K+1 exist ONLY in the marshal.
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                for order in &forged_chain[..(K + 1) as usize] {
                    canned.insert(order.height, order.clone());
                }
            }
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // Deliver the TOP height (ANCHOR+K+1) with an unresolvable parent so the
            // gap-walk fills ANCHOR+1 ..= ANCHOR+K first — hitting the forged gap
            // block at ANCHOR+K. The delivered block itself has a consistent result.
            let delivered = OrderBlock {
                parent: Digest(B256::ZERO),
                ..forged_chain[(K + 1) as usize].clone()
            };
            let halt_trigger = child_of(&delivered, None);
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

    // The tip digest is an ordering digest reth cannot resolve — Update::Tip
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
                        // Below ANCHOR+1+K so guard #2 stays cold — this test is
                        // about the tip's FCU-inertness, not the catch-up guard.
                        Height::new(ANCHOR + 2),
                        tip_digest,
                    ))),
                })
                .expect("send tip");

            // Drain barrier: one real finalize (+ its flush child).
            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let (msg, waiter) = finalize_msg(order.clone());
            mailbox.send(msg).expect("send");
            let (flush, _w_flush) = finalize_msg(child_of(&order, None));
            mailbox.send(flush).expect("send flush child");
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

    // Speculative execution imports the block at NOTARIZATION (advancing the
    // head ahead of finalization); the matching finalization reconciles WITHOUT
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
            // Finalize the SAME order — reconciliation must skip the re-derive.
            let (msg, waiter) = finalize_msg(order.clone());
            mailbox.send(msg).expect("send finalize");
            let (flush, _w_flush) = finalize_msg(child_of(&order, None));
            mailbox.send(flush).expect("send flush child");
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

    // A notarized block that then gets nullified (a SIBLING finalizes) must be
    // rolled back: the finalized sibling is derived and the head reorgs onto it.
    #[test]
    fn speculation_rolls_back_to_finalized_sibling() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // Speculatively execute sibling A (notarized at ANCHOR+1). The
            // siblings are distinguished by `extra_data` (ANCHOR+1 is in the
            // pre-activation window, so both commit `result == ZERO`).
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

            // But a different sibling B finalizes (A was nullified).
            let order_b = OrderBlock {
                extra_data: Bytes::from_static(b"B"),
                ..sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO)
            };
            let (msg, waiter) = finalize_msg(order_b.clone());
            mailbox.send(msg).expect("send finalize B");
            let (flush, _w_flush) = finalize_msg(child_of(&order_b, None));
            mailbox.send(flush).expect("send flush child");
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

    // (b, Fix 1) The tip is HELD, not parked: delivering `h` alone produces NO
    // derive, NO ack, NO park (deferred stays empty), NO hint, and NOT EVEN a
    // `fetch_block_by_height(h+1)` probe (the executor knows the child does not
    // exist yet). Delivering `h+1` derives `h` IN THAT SAME handler, from the
    // child body in hand, with zero marshal fetches — and `h+1` becomes the new
    // held tip.
    #[test]
    fn tip_is_held_not_parked_until_child_arrives() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            let o1 = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let (m1, mut w1) = finalize_msg(o1.clone());
            mailbox.send(m1).expect("send h");
            ctx.sleep(Duration::from_millis(20)).await;
            assert!(
                fx.beacon.new_payload_calls.lock().unwrap().is_empty(),
                "the tip must be HELD underived until its child arrives"
            );
            assert!(
                (&mut w1).now_or_never().is_none(),
                "the held tip's ack must stay pending (never acked before derive)"
            );
            assert!(
                fx.marshal.hints.lock().unwrap().is_empty(),
                "a hold is not a park: no hint is issued"
            );
            assert!(
                fx.marshal.fetched.lock().unwrap().is_empty(),
                "the executor must not even ASK the marshal for the nonexistent child"
            );

            // The child arrives → `h` derives + acks in that same handler, from
            // the child body in hand (still zero marshal fetches).
            let (m2, _w2) = finalize_msg(child_of(&o1, None));
            mailbox.send(m2).expect("send h+1");
            w1.await.expect("h acks the moment its child arrives");
            assert_eq!(
                fx.beacon
                    .new_payload_calls
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|p| p.number)
                    .collect::<Vec<_>>(),
                vec![ANCHOR + 1],
                "exactly the held tip derived; the child became the new held tip"
            );
            assert!(
                fx.marshal.fetched.lock().unwrap().is_empty(),
                "the witness came from the child body in hand — zero marshal fetches"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // (a) THE soak7 BUG, reproduced at the executor contract — the headline
    // test. No per-height finalization cert exists ANYWHERE (the executor no
    // longer even has a cert lookup), `spec_executed` is EMPTY (a restarted /
    // lagging / following node), and the child block carries the witness ⇒ the
    // height derives with the REAL threshold seed and acks. Pre-B′: permanent
    // park (CertMissing → PARK on every re-poke, forever, network-wide).
    #[test]
    fn witness_derives_without_any_cert_or_speculation() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let seed = real_seed(Round::new(Epoch::new(0), View::new(ANCHOR + 1)));
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let (msg, waiter) = finalize_msg(order.clone());
            mailbox.send(msg).expect("send h");
            // The child carries the parent's seed — the witness.
            let (child_msg, _w_child) = finalize_msg(child_of(&order, Some(seed.clone())));
            mailbox.send(child_msg).expect("send h+1 (the witness)");
            waiter
                .await
                .expect("h derives + acks from the witness alone");

            {
                let seen = fx.deriver.seeds_seen.lock().unwrap();
                assert_eq!(
                    seen.as_slice(),
                    &[(ANCHOR + 1, Some(seed))],
                    "the witness seed reached the deriver verbatim"
                );
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A pre-bootstrap link (both the block's own `parent_seed` and its child's
    // witness are `None`) derives immediately with the agreed `order.digest()`
    // fallback — no refusal, no hint. The existing pre-beacon invariant must
    // not regress.
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
            let (flush, _w_flush) = finalize_msg(child_of(&order, None));
            mailbox.send(flush).expect("send flush child");
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

    // (c″, §4) `parent_seed == None` at a BEACON-ACTIVE height (the block being
    // derived carries a witness itself, so its child MUST carry one too) is a
    // corrupted/legacy archive ⇒ LOUD REFUSAL (SafetyHalt-class), never a
    // silent `order.digest()` fallback derive — that would fork.
    #[test]
    fn seedless_child_on_beacon_active_link_refuses_derive() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let seed = real_seed(Round::new(Epoch::new(0), View::new(ANCHOR)));
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // The block being derived is itself on a beacon-active link (it
            // carries a witness for ITS parent)…
            let order = OrderBlock {
                parent_seed: Some(seed),
                ..sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO)
            };
            // …but its child presents NO witness (corrupted archive).
            let corrupt_child = child_of(&order, None);
            let post_halt = sample_order(corrupt_child.digest(), ANCHOR + 3, B256::ZERO);
            let (msg, waiter) = finalize_msg(order);
            mailbox.send(msg).expect("send h");
            let (child_msg, _w_child) = finalize_msg(corrupt_child);
            mailbox.send(child_msg).expect("send corrupt child");

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
                fx.beacon.new_payload_calls.lock().unwrap().is_empty(),
                "must NOT derive with the digest fallback on a beacon-active link (silent fork)"
            );
        });
    }

    /// A `SpecNotarized` command carrying a real recovered seed (populates
    /// `spec_executed[h].seed_round` — reconciled against the witness round at
    /// the finalized derive).
    fn spec_msg_seeded(order: &OrderBlock, seed: crate::beacon::seed::Seed) -> Message {
        Message {
            cause: Span::current(),
            command: Command::SpecNotarized(Box::new(Notarized {
                digest: order.digest(),
                seed: Some(seed),
            })),
        }
    }

    // (Fix 1 🔴) THE PIPELINE-SHIFT REGRESSION TEST: speculation MUST run at
    // every height at the tip, so the reth head advances at NOTARIZATION
    // latency while the finalized derive rides exactly one block behind. Under
    // a park-based design (revision 4's `NeedChild`) `deferred.is_some()` at
    // every height switches speculation OFF via the `spec_execute` guard and
    // this test fails on every clause.
    #[test]
    fn speculation_runs_at_every_height_at_the_tip() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const N: u64 = 4;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // A linked seedless chain ANCHOR+1..=ANCHOR+N.
            let mut chain = vec![sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO)];
            for i in 2..=N {
                let parent = chain.last().unwrap();
                chain.push(sample_order(parent.digest(), ANCHOR + i, B256::ZERO));
            }
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
                // Assert per height: speculation ran (the EL head advanced at
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
                // Each height imported exactly ONCE — the finalized reconcile
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

    // (f) STEADY STATE IS ZERO-COST: at the tip (`last_tip < h + K`) guard #2
    // never fires and the derive path issues NO `fetch_block_by_height` at all
    // (the witness is the child body in hand).
    #[test]
    fn steady_state_derive_issues_no_marshal_fetches() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            let o1 = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let o2 = child_of(&o1, None);
            let o3 = child_of(&o2, None);
            let (m1, w1) = finalize_msg(o1);
            let (m2, w2) = finalize_msg(o2);
            let (m3, _w3) = finalize_msg(o3);
            mailbox.send(m1).expect("send 1");
            mailbox.send(m2).expect("send 2");
            mailbox.send(m3).expect("send 3");
            w1.await.expect("ack 1");
            w2.await.expect("ack 2");

            assert!(
                fx.marshal.fetched.lock().unwrap().is_empty(),
                "steady state: no h+K fetch (guard #2 cold) and no witness re-fetch"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // (d) GUARD #2, re-gated to `last_tip >= h + K`: a catching-up node whose
    // committee-attested block at `h + K` disagrees with the hash it derived
    // from the witness engages SafetyHalt(ResultDivergence) BEFORE the ack —
    // immediately, not K blocks downstream.
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
            // The attested root at H+K commits a DIFFERENT hash than the witness
            // derive produces ⇒ a fork the guard must catch.
            let forged = B256::repeat_byte(0xEE);
            let order_hk = sample_order(Digest(B256::ZERO), H + K, forged);
            fx.marshal.canned.lock().unwrap().insert(H + K, order_hk);

            // The node is BEHIND: the finalized frontier is already past H+K.
            mailbox.send(tip_msg(H + K)).expect("send tip");
            let (m, w) = finalize_msg(order_h.clone());
            mailbox.send(m).expect("send finalize");
            let (mc, _wc) = finalize_msg(child_of(&order_h, None));
            mailbox.send(mc).expect("send child (triggers the derive)");

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

    // (e) GUARD #2's absent-body arm — the executor's ONLY park: the node is
    // behind (`tip >= h + K`) but the attested body at `h + K` is not
    // backfilled yet ⇒ PARK before the ack and before the `split_off` prune,
    // hint exactly `h + K`, keep the CHILD held in `awaiting_child` (park the
    // parent, hold the child), and keep speculation suppressed while parked
    // (the `spec_execute` guard is kept, not narrowed). When the body lands,
    // the re-poke re-derives with the RETAINED witness (zero lookups) and the
    // held child derives right after — nothing is lost.
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
            let child = child_of(&order_h, None);

            mailbox
                .send(tip_msg(H + K))
                .expect("send tip (node behind)");
            let (m, w) = finalize_msg(order_h.clone());
            mailbox.send(m).expect("send finalize");
            let (mc, wc) = finalize_msg(child.clone());
            mailbox.send(mc).expect("send child");

            // The derive ran (import happened) but the ack is withheld — parked
            // on the absent H+K body, with exactly H+K hinted.
            ctx.sleep(Duration::from_millis(50)).await;
            assert_eq!(
                fx.beacon.new_payload_calls.lock().unwrap().len(),
                1,
                "derived + imported, then parked on the absent h+K body"
            );
            assert_eq!(
                *fx.marshal.hints.lock().unwrap(),
                vec![H + K],
                "the park hints a SINGLE height: h + K"
            );
            assert!(
                !fx.safety_halt.is_engaged(),
                "an absent-body park is not a fork"
            );

            // Speculation stays suppressed while parked — the guard is KEPT.
            let spec_order = child_of(&child, None);
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

            // The H+K body lands (attesting the hash the witness derive
            // produced) → the tip re-poke re-derives with the RETAINED witness
            // and acks; the held child derives on the next delivery.
            let attested = fx.chain.spec_executed_hash(H).unwrap();
            let order_hk = sample_order(Digest(B256::ZERO), H + K, attested);
            fx.marshal.canned.lock().unwrap().insert(H + K, order_hk);
            // Same-height tip: the re-poke is the event; keeping the tip at
            // H+K leaves guard #2 COLD for the held child (tip < child + K), so
            // the child derives + acks below without needing an H+K+1 body.
            mailbox.send(tip_msg(H + K)).expect("send tip re-poke");
            w.await.expect("parked block acks once the h+K body lands");

            // The child was retained across the park: the next delivery derives it.
            let (m3, _w3) = finalize_msg(spec_order);
            mailbox.send(m3).expect("send grandchild");
            wc.await
                .expect("the held child derives after the park clears");

            assert!(!fx.safety_halt.is_engaged(), "clean convergence → no halt");
            drop(mailbox);
            let _ = handle.await;
        });
    }

    // (e′) The guard-#2 park's DELIVERY-INDEPENDENT backstop: a body landing at
    // `height <= tip` fires no `Update::Tip`, so the FCU-heartbeat re-poke is
    // what clears the park ([[dpos-deferred-catchup-invariants]] #3 — reused
    // tick, no new timer). The re-poke re-derives from the RETAINED witness
    // (`Deferred::seed`) with zero lookups.
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
            let (mc, _wc) = finalize_msg(child_of(&order_h, None));
            mailbox
                .send(mc)
                .expect("send child (triggers derive → park)");
            ctx.sleep(Duration::from_millis(50)).await;
            assert_eq!(
                *fx.marshal.hints.lock().unwrap(),
                vec![H + K],
                "parked on the absent h+K body"
            );

            // The body lands SILENTLY (no tip, no delivery) — only the heartbeat
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

    // (Fix A-1 🟠) The held block is NEVER acked before it is derived — even at
    // shutdown. Acking it would durably advance the marshal's
    // `last_processed_height` past an underived height: a PERMANENT hole. The
    // drop (→ Canceled) is the deliberate disposition; the withheld ack is the
    // restart self-heal.
    #[test]
    fn held_block_is_never_acked_at_shutdown() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let (m, w) = finalize_msg(order);
            mailbox.send(m).expect("send h (becomes the held tip)");
            ctx.sleep(Duration::from_millis(20)).await;

            // Stop the executor while it holds `h`.
            drop(mailbox);
            handle.await.expect("executor exits on mailbox close");

            // The ack resolved CANCELED (dropped), not Ok: the executor did not
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

    // (Fix A-2 🟠) The restart SELF-HEAL: a node that stopped while holding `h`
    // re-dispatches `h` (the marshal's `last_processed_height` never advanced),
    // derives it from `h+1`'s witness, and the derived chain has NO hole — the
    // hash equals the one a never-stopped node derives. `awaiting_child` needs
    // no persistence; the withheld ack is the durable record.
    #[test]
    fn restart_after_stop_while_holding_rederives_without_hole() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let child = child_of(&order, None);
            // The golden value a never-stopped node derives for `h`.
            let golden = sealed_at(fx.anchor_hash, ANCHOR + 1, order.digest().0).hash();

            // Run 1: deliver `h`, stop while holding it.
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

            // Run 2 ("restart"): the marshal re-dispatches from
            // `last_processed + 1` = the held height — model it by re-delivering
            // `h`, then `h+1`. `h` derives from the witness and acks. (A fresh
            // metrics label: a real restart is a fresh process.)
            {
                let (actor, mailbox) = fx.build(ctx.with_label("restart"), ANCHOR, ANCHOR);
                let handle = actor.start();
                let (m, w) = finalize_msg(order.clone());
                mailbox.send(m).expect("re-dispatch h");
                let (mc, _wc) = finalize_msg(child);
                mailbox.send(mc).expect("dispatch h+1");
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

    // (P2 🟡) A FIRST-SEEN SPIN NOTARIZATION must not speculate with the spin
    // round's seed: §4.1 re-canonicalises the round to the block's own
    // `proposal_view`. Without a `SeedStore` entry for the canonical round the
    // speculation is SKIPPED (never speculate with a known-wrong seed); the
    // finalized path then derives from the witness and NO sibling reorg occurs.
    #[test]
    fn spin_notarization_without_canonical_seed_skips_speculation() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const H: u64 = ANCHOR + 1;
            const V0: u64 = 40;
            let fx = Fixture::new(ANCHOR); // no SeedStore wired
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            let order = OrderBlock {
                proposal_view: V0,
                ..sample_order(Digest(B256::ZERO), H, B256::ZERO)
            };
            fx.marshal.canned.lock().unwrap().insert(H, order.clone());
            let seed_v0 = real_seed(Round::new(Epoch::new(0), View::new(V0)));
            let seed_spin = real_seed(Round::new(Epoch::new(0), View::new(V0 + 30)));

            // First-seen notarization at a SPIN round → skip (no import).
            mailbox
                .send(spec_msg_seeded(&order, seed_spin))
                .expect("send spin spec");
            ctx.sleep(Duration::from_millis(20)).await;
            assert!(
                fx.beacon.new_payload_calls.lock().unwrap().is_empty(),
                "must NOT speculate with a known-wrong (spin-round) seed"
            );

            // The finalized path derives from the witness (round V0) — exactly
            // once, no reorg.
            let (m, w) = finalize_msg(order.clone());
            mailbox.send(m).expect("send finalize");
            let (mc, _wc) = finalize_msg(child_of(&order, Some(seed_v0.clone())));
            mailbox.send(mc).expect("send child");
            w.await.expect("ack");
            assert_eq!(
                fx.deriver.seeds_seen.lock().unwrap().as_slice(),
                &[(H, Some(seed_v0))],
                "derived once, from the canonical (witness) seed"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // (P2 🟡, the SeedStore arm) A node that HOLDS the canonical round's seed in
    // its `SeedStore` re-canonicalises the spin notarization and speculates with
    // the SAME seed everyone else uses — the finalized reconcile then reuses the
    // speculation (rounds match; no re-derive, no reorg).
    #[test]
    fn spin_notarization_recanonicalises_from_seed_store() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const H: u64 = ANCHOR + 1;
            const V0: u64 = 40;
            let store = crate::beacon::certify::SeedStore::new();
            let fx = Fixture::new(ANCHOR).with_seed_store(store.clone());
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            let order = OrderBlock {
                proposal_view: V0,
                ..sample_order(Digest(B256::ZERO), H, B256::ZERO)
            };
            fx.marshal.canned.lock().unwrap().insert(H, order.clone());
            let canonical = Round::new(Epoch::new(0), View::new(V0));
            let seed_v0 = real_seed(canonical);
            let seed_spin = real_seed(Round::new(Epoch::new(0), View::new(V0 + 30)));
            store.record(canonical, seed_v0.signature);

            // Spin-round notarization → re-canonicalised to (0, V0) via the store.
            mailbox
                .send(spec_msg_seeded(&order, seed_spin))
                .expect("send spin spec");
            // Finalize with the canonical witness → reconcile REUSES the spec.
            let (m, w) = finalize_msg(order.clone());
            mailbox.send(m).expect("send finalize");
            let (mc, _wc) = finalize_msg(child_of(&order, Some(seed_v0.clone())));
            mailbox.send(mc).expect("send child");
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

    // (c′) F4 CROSS-NODE CONVERGENCE — the fork hazard B′ kills: two nodes whose
    // local cert state named DIFFERENT spin rounds for the same height derive it
    // from the SAME child witness ⇒ identical hash. (Under the pre-B′
    // `lookup_seed` path each derived from its own local cert's round.)
    #[test]
    fn nodes_with_divergent_local_cert_state_derive_identically_from_witness() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let seed = real_seed(Round::new(Epoch::new(0), View::new(40)));
            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let child = child_of(&order, Some(seed));

            let mut hashes = Vec::new();
            for node in 0..2 {
                let fx = Fixture::new(ANCHOR);
                let (actor, mailbox) =
                    fx.build(ctx.with_label(&format!("node{node}")), ANCHOR, ANCHOR);
                let handle = actor.start();
                let (m, w) = finalize_msg(order.clone());
                mailbox.send(m).expect("send h");
                let (mc, _wc) = finalize_msg(child.clone());
                mailbox.send(mc).expect("send child");
                w.await.expect("ack");
                hashes.push(fx.chain.spec_executed_hash(ANCHOR + 1).unwrap());
                drop(mailbox);
                let _ = handle.await;
            }
            assert_eq!(
                hashes[0], hashes[1],
                "the witness is agreed data — every node derives the identical hash"
            );
        });
    }

    // A multi-height speculative lead (spec_head 3 ahead) where a SIBLING
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

            // Finalize ANCHOR+1 as speculated (no re-derive), then a SIBLING B at
            // ANCHOR+2 finalizes — o2a was nullified. Rollback derives B at +2;
            // the +3 speculation (built on the orphaned o2a) is discarded. Each
            // finalized height derives when its child arrives (pipeline shift),
            // so 2b needs a flush child at +3.
            let (m1, w1) = finalize_msg(o1.clone());
            mailbox.send(m1).expect("send finalize 1");

            let o2b = OrderBlock {
                extra_data: Bytes::from_static(b"B"),
                ..sample_order(o1.digest(), ANCHOR + 2, B256::ZERO)
            };
            let (m2b, w2b) = finalize_msg(o2b.clone());
            mailbox.send(m2b).expect("send finalize 2b");
            w1.await.expect("ack 1");
            let (m3b, _w3b) = finalize_msg(child_of(&o2b, None));
            mailbox.send(m3b).expect("send flush child 3b");
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
                // The `>=` guard let `safe` FOLLOW the same-height sibling reorg
                // onto the finalized hash — never stuck on the orphaned o2a.
                // (FakeBeacon returns Valid unconditionally and does not model
                // reth's `find_canonical_header`, so this VALUE assert is the
                // only thing that catches an orphan-safe bug.)
                assert_eq!(
                    last.safe_block_hash, hash_b,
                    "safe followed the reorg onto the finalized sibling (not orphaned o2a)"
                );
                assert_eq!(
                    last.safe_block_hash, last.head_block_hash,
                    "no surviving spec lead after the rollback ⇒ safe == head"
                );
                // D9 proxy: `safe` is a block reth was told about (imported) at a
                // height ≤ head before the FCU named it — the precondition reth's
                // real `find_canonical_header(safe) == Some` relies on.
                assert!(
                    payloads.iter().any(|p| p.hash() == last.safe_block_hash),
                    "safe was imported (new_payload'd) before the FCU named it"
                );
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // (a) THE DEATH SPIRAL, in miniature (soak bundle-20260715T163059Z): a
    // notarization for a height AHEAD of `spec_head` (a gap) is PARKED, not
    // dropped, and resumes speculation once `spec_head` catches up via the
    // finalized path. Pre-fix the gap notarization was silently dropped, so once
    // the executor fell behind it lost its speculative lead permanently. Here
    // 103's notarization arrives while `spec_head == ANCHOR (100)` — a gap
    // (103 > 101) — and the ONLY spec message for 103 is that parked one; 103's
    // own finalized derive needs its child 104, which is never delivered. So a
    // speculatively-executed 103 can ONLY come from the drain firing when the
    // finalized derive of 102 advances `spec_head` to 102.
    #[test]
    fn parked_gap_notarization_resumes_speculation_on_finalized_advance() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let o1 = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let o2 = sample_order(o1.digest(), ANCHOR + 2, B256::ZERO);
            let o3 = sample_order(o2.digest(), ANCHOR + 3, B256::ZERO);
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                canned.insert(ANCHOR + 1, o1.clone());
                canned.insert(ANCHOR + 2, o2.clone());
                canned.insert(ANCHOR + 3, o3.clone());
            }

            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // The notarization for 103 arrives while spec_head is still the anchor
            // (100): a gap (103 > 101) ⇒ PARKED (pre-fix: dropped forever).
            mailbox
                .send(spec_msg(&o3))
                .expect("spec 103 (gap → parked)");

            // Finalize 101, 102, 103. Each derives when its child is delivered, so
            // delivering 102 derives 101 (spec_head→101) and delivering 103 derives
            // 102 (spec_head→102 ⇒ the drain fires for the parked 103). 103 itself
            // stays `awaiting_child` — its finalized derive would need 104.
            for order in [o1.clone(), o2.clone(), o3.clone()] {
                let (m, _w) = finalize_msg(order);
                mailbox.send(m).expect("finalize");
            }

            // 103 becomes executed ONLY via the parked-drain (no 104 ⇒ no finalized
            // derive of 103, no re-sent live notarization). Pre-fix this times out.
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

    // (b) PRUNE: on a `spec_head` advance the drain drops every parked height ≤
    // `spec_head` (finalized OR already speculated ⇒ stale); a not-yet-drainable
    // higher entry survives. Direct-call so the pre/post parked map is inspectable.
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
            // STOPS at 105 (keeping it) — isolating the prune from the drain.
            actor.spec_head = ANCHOR + 4;
            fx.marshal.canned.lock().unwrap().remove(&(ANCHOR + 5));
            actor.try_drain_parked(&cause).await;

            assert_eq!(
                actor.parked_spec.keys().copied().collect::<Vec<_>>(),
                vec![ANCHOR + 5],
                "102 and 103 pruned (≤ spec_head 104); 105 (> spec_head, body missing) kept"
            );
        });
    }

    // (c) OVERWRITE-BY-HEIGHT: a later sibling notarization at the same parked
    // height replaces the earlier guess (a wrong guess is safe —
    // `correctly_speculated` reconciles it at finalization).
    #[test]
    fn later_sibling_overwrites_parked_entry() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const GAP: u64 = ANCHOR + 3; // > spec_head+1 ⇒ parked
            let fx = Fixture::new(ANCHOR);
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR); // spec_head = 100
            let cause = Span::current();

            // Two siblings at the SAME height, distinct proposal_view + extra_data
            // ⇒ distinct digests. FakeMarshal keys `canned` by height, so swap the
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

    // (d) The drain KEEPS a parked entry (and stops) when its block body is not
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
            actor.try_drain_parked(&cause).await;

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

    // ACCEPTED RESIDUAL + SELF-HEAL: a live notarization for exactly spec_head+1
    // whose BODY is not yet buffered is dropped WITHOUT parking (the body fetch
    // precedes the height gate — the height is unknowable without the body, so
    // it structurally cannot be parked). Bounded and self-healing: a later
    // higher notarization PARKS, and the next finalized advance re-drains
    // speculation past the lost height. This pins the self-heal.
    #[test]
    fn bodyless_live_notarization_drop_self_heals_via_later_park() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let o1 = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let o2 = sample_order(o1.digest(), ANCHOR + 2, B256::ZERO);
            let o3 = sample_order(o2.digest(), ANCHOR + 3, B256::ZERO);
            // 101's body is deliberately NOT buffered (its live notarization is
            // the residual drop); 103's is (it parks).
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                canned.insert(ANCHOR + 2, o2.clone());
                canned.insert(ANCHOR + 3, o3.clone());
            }

            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // The residual: 101 == spec_head+1 but its body is missing → dropped,
            // NOT parked (height unknowable). 103 is a gap → parked.
            mailbox
                .send(spec_msg(&o1))
                .expect("spec 101 (bodyless → dropped)");
            mailbox
                .send(spec_msg(&o3))
                .expect("spec 103 (gap → parked)");

            // The finalized path crosses the lost height: 101 derives on 102's
            // delivery, 102 derives on 103's delivery (spec_head→102) — the drain
            // then resumes speculation at the parked 103.
            for order in [o1.clone(), o2.clone(), o3.clone()] {
                let (m, _w) = finalize_msg(order);
                mailbox.send(m).expect("finalize");
            }
            wait_until(&ctx, "speculation resumed above the lost height", || {
                fx.chain.spec_executed_hash(ANCHOR + 3).is_some()
            })
            .await;

            // 101 was derived by the finalized path ONLY (its live spec was the
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

    // The parent-not-executed PARK gate: a notarization at exactly spec_head+1
    // whose PARENT has not executed is parked (pre-fix: dropped), and the drain
    // executes it once the parent lands (a spec_head advance retries it).
    #[test]
    fn parent_missing_notarization_parks_then_drains_when_parent_lands() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let cause = Span::current();

            // spec_head at 101 while 101 is NOT executed (only the anchor 100 is)
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
            actor.try_drain_parked(&cause).await;

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

    // (a) THE BUNDLE CRASH, in miniature (soak bundle-20260715T184433Z): a
    // speculative lead h..h+2 where h finalizes as a SIBLING (seed-round
    // mismatch) → rollback + re-derive; THEN h+1 finalizes with the SAME ordering
    // digest that was speculated. Pre-fix the orphaned-parent speculated h+1
    // survived the rollback (`split_off` RETAINED the suffix) and
    // `correctly_speculated` — checking only seed-round + executed-hash-present —
    // REUSED it: head stayed at h while finalization advanced (the K-lag
    // underflow), and h+1 sat on the wrong (View-h-a) pre-state → next re-derive
    // fatal. With suffix-invalidation + parent-linkage, h+1 RE-DERIVES on the
    // finalized parent and head advances onto it.
    #[test]
    fn rolled_back_sibling_child_is_rederived_not_reused() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100; // 101..103 ≤ anchor+K ⇒ pre-activation (result ZERO)
            let fx = Fixture::new(ANCHOR);
            let anchor = fx.anchor_hash;

            // 101 is speculated with seed round 101 but its child's witness is
            // round 102 (the mismatch → rollback). 102/103 speculate with rounds
            // equal to their `proposal_view` so ONLY 101 rolls back.
            let spec_seed_101 = real_seed(Round::new(Epoch::new(0), View::new(101)));
            let witness_101 = real_seed(Round::new(Epoch::new(0), View::new(102)));
            let seed_102 = real_seed(Round::new(Epoch::new(0), View::new(102)));
            let seed_103 = real_seed(Round::new(Epoch::new(0), View::new(103)));

            let o1 = OrderBlock {
                proposal_view: 101,
                ..sample_order(Digest(B256::ZERO), 101, B256::ZERO)
            };
            let o2 = OrderBlock {
                proposal_view: 102,
                parent_seed: Some(witness_101.clone()), // 101's witness
                ..sample_order(o1.digest(), 102, B256::ZERO)
            };
            let o3 = OrderBlock {
                proposal_view: 103,
                parent_seed: Some(seed_102.clone()), // 102's witness (round 102)
                ..sample_order(o2.digest(), 103, B256::ZERO)
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
                seed_folded_discriminator(o1.digest(), &Some(witness_101.clone())),
            )
            .hash();
            // Fork-A 102 was speculated on the ORPHANED 101 (hash_spec_101); the
            // finalized re-derive lands it on the NEW canonical 101 (hash_fin_101).
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
            // The load-bearing precondition: fork-A 102 is speculated on the
            // orphaned 101 (the block pre-fix reused it as final).
            wait_until(&ctx, "fork-A 102 speculated", || {
                fx.chain.spec_executed_hash(102) == Some(hash_spec_102)
            })
            .await;

            // Each height derives when its child arrives: deliver 101,102,103.
            // 102's delivery derives 101 (round mismatch → rollback+rederive);
            // 103's delivery derives 102 (must RE-DERIVE, not reuse fork-A).
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
                "101 re-derived with the witness seed"
            );
            assert_eq!(
                fx.chain.spec_executed_hash(102),
                Some(hash_fin_102),
                "102 re-derived on the finalized 101 (fork-A speculation NOT reused)"
            );
            assert_eq!(
                fx.beacon
                    .fcu_calls
                    .lock()
                    .unwrap()
                    .last()
                    .unwrap()
                    .head_block_hash,
                hash_fin_102,
                "head ADVANCED onto the re-derived 102 (pre-fix it stayed stuck at 101)"
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

    // (b) PARENT-LINKAGE, isolated: a speculated block whose recorded parent no
    // longer matches the block canonical at `height − 1` is REJECTED by
    // `correctly_speculated` and re-derived — even though the seed ROUND and the
    // ordering DIGEST both match the witness. Here the seed VALUE is identical on
    // both paths, so the only difference between the reuse hash and the re-derive
    // hash is the PARENT — proof the parent-linkage clause (not the round clause)
    // forced the re-derive.
    #[test]
    fn stale_parent_speculation_is_rejected_despite_matching_round() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let cause = Span::current();

            // 101 speculated (and, at finalize, witnessed) with the SAME seed of
            // round 101 == proposal_view ⇒ the round clause passes on both paths.
            let seed = real_seed(Round::new(Epoch::new(0), View::new(101)));
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

            // A parent reorg with NO rollback event of its own: the block canonical
            // at 100 changes out from under the recorded speculation (stored parent
            // == `anchor`).
            let stale_parent = B256::repeat_byte(0xEE);
            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(ANCHOR, stale_parent);

            // Finalize 101 from its witness (the child supplies it). Same digest,
            // same round → only parent-linkage can reject the reuse.
            let child = OrderBlock {
                parent_seed: Some(seed.clone()),
                ..sample_order(o1.digest(), 102, B256::ZERO)
            };
            let (held_ack, _hw) = Exact::handle();
            actor
                .on_finalized_block(cause.clone(), o1.clone(), held_ack)
                .await
                .unwrap();
            let (child_ack, _cw) = Exact::handle();
            actor
                .on_finalized_block(cause.clone(), child, child_ack)
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

    // EAGER FINALIZED DERIVE (record-lag closer, bundle-20260716T162805Z): a
    // delivered finalized `h` whose OWN agreed round `Round(0, proposal_view)` is
    // in the SeedStore is derived + finalized-recorded AT DELIVERY, before its
    // child `h+1` exists — closing the recorded_tip = delivered_tip − 1 lag that
    // livelocked the finalized-tier result gate (nullify storm / stall).
    #[test]
    fn eager_finalized_derive_records_before_child_arrives() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let h = ANCHOR + 1;
            let store = crate::beacon::certify::SeedStore::new();
            let seed = real_seed(Round::new(Epoch::new(0), View::new(h)));
            store.record(seed.target_round, seed.signature);
            let fx = Fixture::new(ANCHOR).with_seed_store(store);
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
            assert!(actor.awaiting_child.is_none(), "the eager derive CONSUMED the hold");
            assert!(!fx.safety_halt.is_engaged());
        });
    }

    // MISS FALLBACK: with the round ABSENT from the store the delivered tip stays
    // HELD (the exact prior one-block-lookahead behavior) — eager did NOT run, so
    // `h` is recorded only once its child supplies the witness.
    #[test]
    fn eager_miss_holds_the_tip_for_the_child_witness() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let h = ANCHOR + 1;
            // Store present but EMPTY — the only difference from the hit test is
            // the missing round entry (isolates the miss branch).
            let store = crate::beacon::certify::SeedStore::new();
            let fx = Fixture::new(ANCHOR).with_seed_store(store);
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
                "MISS: h NOT derived at delivery — held for the child witness"
            );
            assert!(
                actor.awaiting_child.is_some(),
                "the tip stays HELD on a store miss"
            );
        });
    }

    // EAGER + REORG: `h` speculated as sibling A, then finalized-DELIVERED as a
    // DIFFERENT sibling B while the round is in the store — the eager derive takes
    // the re-derive path (the v21-shape assertion at the eager site), records B,
    // and NEVER leaves the speculated A behind.
    #[test]
    fn eager_derive_reorgs_a_speculated_sibling() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let h = ANCHOR + 1;
            let store = crate::beacon::certify::SeedStore::new();
            let seed = real_seed(Round::new(Epoch::new(0), View::new(h)));
            store.record(seed.target_round, seed.signature);
            let fx = Fixture::new(ANCHOR).with_seed_store(store);
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

    // CHILD-DELIVERY-AFTER-EAGER: once `h` is eager-consumed, delivering its child
    // `h+1` must NOT re-derive `h` (the hold is gone) — `h+1` simply becomes the
    // new held tip (its own round is not in the store ⇒ a miss ⇒ hold).
    #[test]
    fn child_delivery_after_eager_does_not_double_derive() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let h = ANCHOR + 1;
            let store = crate::beacon::certify::SeedStore::new();
            let seed_h = real_seed(Round::new(Epoch::new(0), View::new(h)));
            store.record(seed_h.target_round, seed_h.signature);
            let fx = Fixture::new(ANCHOR).with_seed_store(store);
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let cause = Span::current();

            let o_h = OrderBlock {
                proposal_view: h,
                ..sample_order(Digest(B256::ZERO), h, B256::ZERO)
            };
            let child = OrderBlock {
                parent_seed: Some(seed_h.clone()),
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
            assert!(actor.awaiting_child.is_none(), "h eager-consumed");
            let hash_h = fx.chain.spec_executed_hash(h).unwrap();

            // Child h+1 delivered: held is empty (h consumed) ⇒ h+1 held; its own
            // round `Round(0, h+1)` is NOT in the store ⇒ a miss ⇒ hold.
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
            assert!(actor.awaiting_child.is_some(), "h+1 is now the held tip");
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

    // EPOCH-BOUNDARY eager derive (the divergence-critical epoch identity): `h` is
    // the LAST block of epoch e, so its child crosses into e+1 and `witness_link`'s
    // boundary adjustment (`ec − 1`) pins the witness round's epoch to e — which is
    // exactly `epocher.containing(h).epoch()`. With the store populated under
    // `Round(e, view)` the eager derive HITS with the correctly computed epoch-e
    // round and records `h` before the child exists.
    #[test]
    fn eager_derive_hits_at_the_epoch_boundary_with_the_parent_epoch_round() {
        use commonware_consensus::types::{Epoch, Epocher as _, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            // origin 0, length 8: epoch 1 = heights 8..=15; h = 15 is its LAST
            // block (the child at 16 is the first block of epoch 2).
            let epocher = crate::epocher::OriginEpocher::new(
                0,
                std::num::NonZeroU64::new(8).expect("nonzero"),
            );
            const ANCHOR: u64 = 14;
            const H: u64 = 15;
            assert_eq!(
                epocher.containing(Height::new(H)).unwrap().last(),
                Height::new(H),
                "test premise: h is the last block of its epoch"
            );
            let e = epocher.containing(Height::new(H)).unwrap().epoch();
            assert_eq!(e, Epoch::new(1));

            let store = crate::beacon::certify::SeedStore::new();
            let seed = real_seed(Round::new(e, View::new(H)));
            store.record(seed.target_round, seed.signature);
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
                actor.awaiting_child.is_none(),
                "hold consumed on the boundary hit"
            );
            assert!(!fx.safety_halt.is_engaged());
        });
    }

    // Negative twin: the SAME boundary height with the store populated ONLY under
    // the NEXT epoch's round `Round(e+1, view)` must MISS — the eager round is a
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
            const ANCHOR: u64 = 14;
            const H: u64 = 15; // last block of epoch 1
            let store = crate::beacon::certify::SeedStore::new();
            // SAME view, WRONG epoch (e+1 = 2): the only entry in the store.
            let wrong = real_seed(Round::new(Epoch::new(2), View::new(H)));
            store.record(wrong.target_round, wrong.signature);
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
            assert!(
                actor.awaiting_child.is_some(),
                "MISS: held for the child witness"
            );
        });
    }

    // SEED-NOTIFY RE-ATTEMPT (the deadlock-breaker, bundle-20260716T203448Z;
    // migrated from the deleted `SpecNotarized` Poke — the race in miniature):
    // `h` is finalized-delivered BEFORE its seed is recorded → the on-delivery
    // eager derive MISSES → `h` is HELD. Then the notarization for `h`'s round
    // lands: the Reporter records the seed into the shared SeedStore (which fires
    // the `Notify`), and the executor's seed-notify `select!` arm re-runs the
    // eager derive. `h` is derived + finalized-recorded WITHOUT any further
    // finalized delivery — the exact event that a stalled chain cannot produce.
    // Drives the arm's BODY (`try_eager_finalized_derive(Notified)`) directly —
    // the arm's WAKEUP (no lost notification) is covered by certify.rs's
    // `seed_store_record_notifies_without_a_lost_wakeup`.
    #[test]
    fn seed_notify_recovers_a_held_tip_after_a_late_seed_record() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let h = ANCHOR + 1;
            // Store starts EMPTY: the delivery-time eager derive must miss.
            let store = crate::beacon::certify::SeedStore::new();
            let fx = Fixture::new(ANCHOR).with_seed_store(store.clone());
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let cause = Span::current();

            let o1 = OrderBlock {
                proposal_view: h,
                ..sample_order(Digest(B256::ZERO), h, B256::ZERO)
            };
            fx.marshal.canned.lock().unwrap().insert(h, o1.clone());

            // Deliver h with the seed NOT yet recorded → eager MISS → HELD.
            let (ack, _w) = Exact::handle();
            actor
                .on_finalized_block(cause.clone(), o1.clone(), ack)
                .await
                .unwrap();
            assert!(
                fx.chain.spec_executed_hash(h).is_none(),
                "delivery missed: h held, not derived"
            );
            assert!(actor.awaiting_child.is_some(), "h is HELD after the miss");

            // The notarization for h's round arrives at the Reporter: it records
            // the seed (which fires the notify permit). The seed-notify arm then
            // re-runs the eager derive — model that by driving the arm's body.
            let seed = real_seed(Round::new(Epoch::new(0), View::new(h)));
            store.record(seed.target_round, seed.signature);
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
                actor.awaiting_child.is_none(),
                "the notified eager derive CONSUMED the hold"
            );
            assert!(!fx.safety_halt.is_engaged());
        });
    }

    // SEED-NOTIFY NO-OP (b): a notify re-attempt must NOT spuriously derive when
    // either (i) nothing is held (the arm's `awaiting_child.is_some()` guard is
    // false), or (ii) a tip is held but the store STILL misses its round (the
    // seed has not landed yet — a later notify or the child witness will derive
    // it). Neither path may advance the EL or touch the hold.
    #[test]
    fn seed_notify_is_a_noop_without_hold_or_seed() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let h = ANCHOR + 1;
            let store = crate::beacon::certify::SeedStore::new();
            let fx = Fixture::new(ANCHOR).with_seed_store(store);
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let cause = Span::current();

            // (i) No hold: the arm's guard (`awaiting_child.is_some()`) is false,
            // so the body is a no-op even if driven directly (the take() early
            // returns).
            assert!(actor.awaiting_child.is_none(), "premise: nothing held");
            actor
                .try_eager_finalized_derive(EagerTrigger::Notified)
                .await
                .unwrap();
            assert!(
                fx.chain.spec_executed_hash(h).is_none(),
                "no-hold notify derived nothing"
            );
            assert!(
                actor.awaiting_child.is_none(),
                "no-hold notify created no hold"
            );

            // (ii) Held tip but the store still misses its round → held-and-quiet.
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
            assert!(actor.awaiting_child.is_some(), "h held (store still empty)");

            actor
                .try_eager_finalized_derive(EagerTrigger::Notified)
                .await
                .unwrap();
            assert!(
                fx.chain.spec_executed_hash(h).is_none(),
                "store-still-missing notify did NOT derive h (silent no-op)"
            );
            assert!(
                actor.awaiting_child.is_some(),
                "the hold is retained on a notify miss"
            );
        });
    }

    // (c) RE-HEAL SYNERGY: a rollback INVALIDATES the `spec_executed` suffix but
    // KEEPS the parked notarizations above the reorg point, and the post-rollback
    // drain re-executes them against the NEW canonical parent. Fork-A 102 is a
    // live speculation (the invalidated suffix); fork-B 102 is a parked gap
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

            // PARK fork-B 102 as a gap (102 > spec_head+1 while spec_head == 100).
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
            // invalidates the `spec_executed` suffix {102=o2a}, KEEPS parked{102=
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

    // A speculative head advance NEVER moves `safe`/`finalized`: `spec_execute`
    // calls `update_head` only. After finalizing anchor+1 (which sets safe =
    // h(anchor+1)), speculating +2 and +3 climbs head to h(anchor+3) while safe
    // stays at h(anchor+1) and finalized stays at the anchor — the load-bearing
    // `head > safe` speculative lead (the whole point of the split).
    #[test]
    fn safe_unchanged_across_speculative_head_advance() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // Finalize anchor+1 first (sets safe = head = h(anchor+1); finalized
            // clamped at the anchor in the pre-K window). Its child (anchor+2,
            // also finalized) triggers the derive and becomes the held tip.
            let o1 = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let o2 = sample_order(o1.digest(), ANCHOR + 2, B256::ZERO);
            let (m1, w1) = finalize_msg(o1.clone());
            mailbox.send(m1).expect("send finalize 1");
            let (m2f, _w2f) = finalize_msg(o2.clone());
            mailbox.send(m2f).expect("send finalize 2 (flush child)");
            w1.await.expect("ack 1");

            let safe_after_finalize = fx.chain.spec_executed_hash(ANCHOR + 1).unwrap();
            {
                let fcus = fx.beacon.fcu_calls.lock().unwrap();
                let last = fcus.last().unwrap();
                assert_eq!(last.safe_block_hash, safe_after_finalize);
                assert_eq!(last.finalized_block_hash, fx.anchor_hash);
            }

            // Speculate +2 (the held finalized tip — spec runs ahead of its
            // finalized derive) and +3 (notarized only) — each parent is
            // canonical from the prior FCU.
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

    // (The former `invalid_fcu_status_is_fatal` — import VALID, FCU INVALID —
    // is now byte-identical in behavior to
    // `invalid_finalize_fcu_engages_safety_halt_and_parks` above, which also
    // asserts the gauges; the duplicate was removed with the ack-retention park.)

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
            let (flush, _w_flush) = finalize_msg(child_of(&order, None));
            mailbox.send(flush).expect("send flush child");
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

    // Speculative path: the seed recovered from the NOTARIZATION cert (the
    // `SpecNotarized` command) reaches the deriver during speculative
    // execution, and the same-round witness reconcile keeps the speculation
    // (the deriver runs exactly once).
    #[test]
    fn notarization_seed_reaches_deriver_on_speculation() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let round = Round::new(Epoch::new(0), View::new(ANCHOR + 1));
            let seed = real_seed(round);
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // `proposal_view` matches the notarization round (the honest steady
            // state), so the §4.1 re-canonicalisation is a no-op.
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
            // Finalize the same order; its child carries the SAME-round witness
            // (skips the re-derive — the spec import already recorded the seed).
            let (m, w) = finalize_msg(order.clone());
            mailbox.send(m).expect("send finalize");
            let (mc, _wc) = finalize_msg(child_of(&order, Some(seed.clone())));
            mailbox.send(mc).expect("send child");
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

    // ───────────────────────── steady-state re-jump (finding #6) ─────────────

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

    /// The catch-up ACK BARRIER: deliver `order` (witness-less) + its flush
    /// child and await `order`'s ack. The re-jump tests park the frontier far
    /// ahead, so guard #2 is armed (`tip >= h + K`) — can a result-consistent
    /// attested block at `h + K` so the guard converges instead of parking.
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
        let (flush, _w_flush) = finalize_msg(child_of(&order, None));
        mailbox.send(flush).expect("send barrier flush child");
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
        BadTarget(String),
        InvalidTarget(String),
        AuthFailed(String),
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
                Scripted::BadTarget(s) => JumpOutcome::BadTarget(eyre::eyre!(s.clone())),
                Scripted::InvalidTarget(s) => JumpOutcome::InvalidTarget(eyre::eyre!(s.clone())),
                Scripted::AuthFailed(s) => JumpOutcome::AuthFailed(eyre::eyre!(s.clone())),
                Scripted::L1Fork(s) => JumpOutcome::L1Fork(eyre::eyre!(s.clone())),
            }
        }
    }

    /// A re-jump callback recording each `from` it was invoked with and returning
    /// the scripted [`crate::cold_start_jump::JumpOutcome`].
    type RejumpCalls = Arc<Mutex<Vec<u64>>>;
    fn recording_re_jump(scripted: Scripted) -> (ReJump, RejumpCalls) {
        let (cb, calls, _frontier) = recording_re_jump_with_frontier(scripted);
        (cb, calls)
    }

    /// As [`recording_re_jump`] but also returns the `upstream_frontier` atomic so
    /// a test can simulate the cert-inlet advancing it (the deadlock path, where
    /// the marshal `Update::Tip` height stays frozen).
    fn recording_re_jump_with_frontier(
        scripted: Scripted,
    ) -> (ReJump, RejumpCalls, Arc<std::sync::atomic::AtomicU64>) {
        let calls: RejumpCalls = Arc::new(Mutex::new(Vec::new()));
        let calls_cl = calls.clone();
        let call: ReJumpFn = Arc::new(move |from| {
            calls_cl.lock().unwrap().push(from);
            let scripted = scripted.clone();
            Box::pin(async move { scripted.build() })
        });
        let upstream_frontier = Arc::new(std::sync::atomic::AtomicU64::new(0));
        (
            ReJump {
                call,
                upstream_frontier: upstream_frontier.clone(),
                threshold: JUMP_THRESHOLD,
                rotate: None,
                probe: None,
            },
            calls,
            upstream_frontier,
        )
    }

    /// As [`recording_re_jump`] but wires a RECORDING `rotate` escape (the
    /// recording-rotate idiom from `cert_inlet::tests`), returning the rotation-count
    /// atomic so a test can assert Rule-L failover fired the expected number of times.
    /// `scripts` is a SATURATING sequence — call N returns `scripts[min(N, len−1)]` —
    /// so a single-element vec is the single-outcome case and a longer vec scripts a
    /// per-call outcome sequence (the cross-URL streak-reset test needs Stalled→…→
    /// BadTarget→Stalled).
    fn recording_re_jump_with_rotate(
        scripts: Vec<Scripted>,
    ) -> (ReJump, RejumpCalls, Arc<std::sync::atomic::AtomicU32>) {
        assert!(!scripts.is_empty(), "need at least one scripted outcome");
        let calls: RejumpCalls = Arc::new(Mutex::new(Vec::new()));
        let calls_cl = calls.clone();
        let scripts = Arc::new(scripts);
        let idx = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let call: ReJumpFn = Arc::new(move |from| {
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
                upstream_frontier: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                threshold: JUMP_THRESHOLD,
                rotate: Some(rotate),
                probe: None,
            },
            calls,
            rotations,
        )
    }

    // (a) The re-jump FIRES when `Update::Tip.height − ordering_finalized >
    // JUMP_THRESHOLD`: the executor SPAWNS the read-only waiter, and its
    // `oneshot` completion arm re-seeds the anchor (finalized cursor moves to the
    // landing) + advances the running marshal floor via `set_floor(floor)`. The
    // OFF-BY-K assertion (`ordering_finalized == landing`, not `floor`) is pinned
    // directly in `reseed_forward_off_by_k_raises_cursor_to_landing` (the cursor
    // is private); here we assert the observable reseed + floor advance.
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

            // Frontier far beyond the serving window ⇒ trigger (spawns the waiter).
            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                .expect("send tip");
            // Yield so the deterministic runtime drives the spawned waiter to
            // completion + its `jump_done` arm re-seeds before the barrier below.
            ctx.sleep(Duration::from_millis(10)).await;

            // Barrier: a real finalize at landing+1 — its parent (the re-seeded
            // landing) must be canonical for the parent read to succeed. It acks
            // only AFTER the spawned waiter's completion arm has re-seeded (the
            // marshal floor recorder confirms the reseed ran).
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

    // STALE FINALIZATION BACKLOG PRUNE (bundle-20260716T034647Z): deliveries
    // queued while the drain arm is gated off by an IN-FLIGHT jump are stale
    // below-landing blocks; reseed_forward must prune them (ack Ok — canonical
    // post-backfill, never Canceled) so the reopened drain does not re-populate
    // `awaiting_child` with a jumped-over height — pre-fix, the first genuine
    // post-floor dispatch's non-contiguous witness re-fetch of the jump-pruned
    // child hit the "witness gap" fatal (a jump-MANUFACTURED skip-gap
    // misclassified as archive corruption). Post-fix: backlog pruned+acked, the
    // next post-floor dispatch derives and acks cleanly, executor stays up.
    #[test]
    fn reseed_prunes_stale_queued_finalizations_no_witness_gap_fatal() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let landing_h = ANCHOR + JUMP_THRESHOLD + 5_000;
            let landing_hash = B256::repeat_byte(0xE1);
            let floor = landing_h - K;
            // A jump that stays IN FLIGHT until the test releases it — the
            // window in which stale deliveries accumulate.
            let gate = Arc::new(tokio::sync::Notify::new());
            let gate_cl = gate.clone();
            let call: ReJumpFn = Arc::new(move |_from| {
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
                upstream_frontier: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                threshold: JUMP_THRESHOLD,
                rotate: None,
                probe: None,
            };
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // Spawn the jump (blocked on the gate).
            mailbox.send(tip_msg(landing_h + 10)).expect("send tip");
            ctx.sleep(Duration::from_millis(5)).await;

            // Two stale below-landing deliveries queue while the drain arm is
            // gated off (jump in flight).
            let o1 = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let o2 = sample_order(o1.digest(), ANCHOR + 2, B256::ZERO);
            let (m1, w1) = finalize_msg(o1);
            let (m2, w2) = finalize_msg(o2);
            mailbox.send(m1).expect("queue stale 1");
            mailbox.send(m2).expect("queue stale 2");
            ctx.sleep(Duration::from_millis(5)).await;

            // Release the jump → reseed_forward prunes the stale backlog.
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

            // The drain reopened CLEAN: the next post-floor dispatch derives +
            // acks. Pre-fix the stale 101/102 drained first, 102 became the held
            // tip, and THIS arrival's non-contiguous witness re-fetch of 103
            // (jump-pruned, marshal None) was the witness-gap fatal.
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

    // The stale-backlog prune keys on the LANDING: a queued entry ABOVE it (not
    // covered by the jump's backfill) survives with its ack untouched; entries
    // at/below are pruned + acked Ok. Direct-call so the queue is inspectable.
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
                .await;

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

    // STALE-DISPATCH GUARD (`marshal_floor`): `reseed_forward`'s disposals
    // `acknowledge()` acks BEFORE `set_floor`; in the real marshal each freed slot
    // lets the biased select dispatch the next OLD-range block into the executor
    // mailbox before it processes `SetFloor` (fire-and-forget). Such escaped
    // `≤ floor` deliveries drain via `handle_message`'s `Update::Block` arm; without
    // the guard the deep-gap case derives them and PARKS on a pruned `h + K`
    // (guard #2 `NeedAttestation`, permanent) — here modelled by `FakeMarshal`
    // dispatching a below-floor block at `set_floor`. Post-fix the arm acks it Ok
    // (never derives, never parks) and counts it. Revert-check: with the guard
    // removed the escaped block imports (`new_payload` at `≤ floor`) and hints its
    // pruned `h + K`.
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
                // A jump gated until the test releases it (mirrors the stale-prune
                // test) so the escape is deterministic.
                let gate = Arc::new(tokio::sync::Notify::new());
                let gate_cl = gate.clone();
                let call: ReJumpFn = Arc::new(move |_from| {
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
                    upstream_frontier: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                    threshold: JUMP_THRESHOLD,
                    rotate: None,
                    probe: None,
                };
                let fx = Fixture::new(ANCHOR).with_re_jump(cb);
                let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
                // OLD-range blocks the marshal's biased select escapes into the
                // mailbox when `reseed_forward` calls `set_floor` — both far below
                // the floor, so a derive would park on a pruned `h + K`.
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
                // Drain the escaped below-floor deliveries through the guard.
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
                    "the executor stays up (no witness-gap fatal)"
                );

                // The escape model holds a Mailbox CLONE — release it so the
                // executor's channel closes and the run loop exits.
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

    // DEADLOCK PATH: under the "committee[E] not committed" defer, the inlet
    // stores nothing → the marshal `Update::Tip` height FREEZES just above the
    // anchor, so the OLD trigger (gap measured off the marshal tip) never fired
    // and the follower wedged forever. The inlet keeps advancing
    // `upstream_frontier` off the deferred certs, so the trigger — now measuring
    // `max(tip, upstream_frontier) − ordering_finalized` — fires on a LOW (frozen)
    // tip once the true frontier runs past the serving window. This test sends a
    // low tip while the frontier is far ahead and asserts the re-jump still fires
    // (it would NOT under the pre-fix tip-only gate).
    #[test]
    fn re_jump_fires_off_upstream_frontier_when_marshal_tip_frozen() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let (cb, calls, frontier) = recording_re_jump_with_frontier(Scripted::Lagging);
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // The inlet has advanced the TRUE frontier far past the serving window
            // (deferred certs climb it) while the marshal tip is frozen at the
            // anchor.
            frontier.store(
                ANCHOR + JUMP_THRESHOLD + 5_010,
                std::sync::atomic::Ordering::Relaxed,
            );
            // A LOW (frozen) marshal tip — gap off the tip alone is ≤ JUMP_THRESHOLD,
            // so the pre-fix gate would NOT fire. The fix maxes in the frontier.
            mailbox.send(tip_msg(ANCHOR + 5)).expect("send low tip");
            ctx.sleep(Duration::from_millis(10)).await;

            assert_eq!(
                *calls.lock().unwrap(),
                vec![ANCHOR],
                "re-jump fires off the upstream frontier even with a frozen marshal tip"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // OFF-BY-K (direct, cursor is private): `reseed_forward` raises the executed
    // cursor to the LANDING, not the floor — the landing IS executed
    // post-backfill; the K below-landing blocks are governed by the two-tier
    // result-lag. (Pre-fix it pinned the cursor at `floor`, lagging by K.)
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

            actor.reseed_forward(landing, landing_hash, floor).await;

            let (ordering_finalized, anchor_finalized, safe_height, finalized_height, spec_head) =
                actor.seed_fields();
            assert_eq!(
                ordering_finalized, landing,
                "off-by-K: cursor raised to the LANDING, not the floor ({floor})"
            );
            assert_eq!(anchor_finalized, (Height::new(landing), landing_hash));
            // B1 option (a): the in-memory `finalized_height` is raised to the
            // LANDING (the FCU re-pins the engine tag to the floor); `safe` rides
            // the landing too.
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

            // Finalized cursor advanced to the LANDING: the first post-jump
            // proposals at landing+1..landing+K sample the gate at
            // landing+1−K..landing — heights BELOW the landing that only the
            // cursor's provider resolve can serve. Advancing only to the landing
            // (vs a landing-only entry) is what covers them; else None →
            // K-block propose-skip/false-vote gap post-jump.
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

    // STARTUP-BACKFILL FAST-FORWARD (bundle-20260717T120838Z, the v33 fresh-spare
    // freeze in miniature): a fresh spare's `[last_execution+1 ..= last_consensus]`
    // backfill iterator is pending at a LOW height (377) when a fast-jump lands far
    // above it. `reseed_forward` must fast-forward the iterator so its next yielded
    // height is `landing + 1` — else the post-jump drain resumes at 377 and
    // re-derives the whole jumped `[.. landing]` range (mdbx-timeout freeze). The
    // drain site (`self.finalized_heights_to_backfill.next()`) is the ONLY source
    // of backfill heights reaching the deriver, so asserting the iterator yields
    // nothing ≤ landing IS the "deriver never sees the jumped range" guarantee.
    #[test]
    fn reseed_forward_fast_forwards_backfill_past_landing() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const NEXT: u64 = 377; // pending pre-jump backfill height (init: anchor+1)
            const ANCHOR: u64 = NEXT - 1;
            let landing = ANCHOR + JUMP_THRESHOLD + 4_000;
            let landing_hash = B256::repeat_byte(0xE1);
            let floor = landing - K;
            // Backfill end ABOVE the landing so we also pin that the ORIGINAL upper
            // bound is preserved (only the ≤ landing prefix is skipped).
            let end = landing + 50;

            let fx = Fixture::new(ANCHOR);
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, end);
            // Pre-jump: the iterator is pending at the low height.
            assert_eq!(
                *actor.finalized_heights_to_backfill.start(),
                NEXT,
                "backfill iterator pending at the pre-jump low height"
            );
            let len_before = actor.finalized_heights_to_backfill.clone().count();

            actor.reseed_forward(landing, landing_hash, floor).await;

            // The next drained backfill height is landing+1 — the entire jumped
            // range [NEXT ..= landing] is skipped, and every remaining height is
            // strictly above the landing (the deriver never sees the jumped range).
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
            // Skipped exactly the [NEXT ..= landing] prefix: the iterator shrank by
            // that many heights (the count the fast-forward metric increments by).
            assert_eq!(
                len_before - remaining.len(),
                (landing - NEXT + 1) as usize,
                "skipped count spans [NEXT ..= landing]"
            );
        });
    }

    // NO-OP boundary: when the landing is at/below the iterator's next-to-yield
    // height, there is nothing ≤ landing to skip, so the iterator is untouched.
    // Uses landing == NEXT-1 (the highest landing that skips nothing).
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

            actor.reseed_forward(landing, landing_hash, floor).await;

            let after: Vec<u64> = actor.finalized_heights_to_backfill.clone().collect();
            assert_eq!(
                before, after,
                "landing below the iterator's next-to-yield leaves the backfill range untouched"
            );
            assert_eq!(after.first().copied(), Some(NEXT), "still pending at NEXT");
        });
    }

    // RESTART SIMULATION (review blocker): a fresh process starts with the
    // finalized-execution cursor at 0 but a provider populated up to the
    // marshal-acked cursor T (`last_consensus_finalized_height` = marshal
    // `last_processed`). Each acked height is consensus-finalized (unique) and
    // passed the canonical postcondition pre-restart — no sibling can exist
    // there. `init` seeds the cursor at T, so the result gate serves provider
    // hashes for h ≤ T (the first K post-restart proposals sample T+1−K..T)
    // instead of None — pre-fix, a coordinated ≥f+1 restart wedged the committee
    // permanently (propose skips + verify false-bias, the cursor never seeding).
    #[test]
    fn init_seeds_result_gate_floor_at_marshal_acked() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const T: u64 = 100; // the pre-restart marshal-acked cursor
            let fx = Fixture::new(T);
            let persisted_below = B256::repeat_byte(0x99);
            // Persisted pre-restart canonical content: T−1 (acked, final) and a
            // speculative tail block ABOVE the acked point.
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

            // `build` wires last_consensus_finalized_height (the acked cursor,
            // the cursor seed source) = T; reth head also = T here (the clean
            // common case where the acked cursor and the head coincide).
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
            // The persisted SPECULATIVE tail above the acked point must NOT be
            // served as finalized — it can still be reorged by the startup
            // reconcile.
            assert_eq!(
                fx.chain.finalized_executed_hash(T + 1),
                None,
                "above the floor stays None until the finalized reconcile records it"
            );
        });
    }

    // SOUNDNESS (family2_finalized_tier.md §1.1): a clean shutdown persists
    // reth's HEAD, and under deferred execution the head carries a SPECULATIVE
    // tail ABOVE the marshal-acked cursor (`spec_execute` advances the head at
    // notarization latency). Heights in `(acked, head]` are notarized-only — a
    // sibling can still finalize (notarize A → nullify → finalize B). The floor
    // MUST seed from the acked cursor (`last_consensus_finalized_height`), NOT
    // the reth head (`last_execution_finalized_height`) — else a restart
    // straddling a nullify race serves the orphaned speculative sibling as a
    // finalized result (the whole-committee bundle-20260716T150148Z divergence,
    // re-entered through restart). Pre-fix (floor = reth head) the assertion
    // below returned `Some(the speculative hash)`.
    #[test]
    fn init_floor_excludes_speculative_tail_above_acked() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ACKED: u64 = 100; // marshal last_processed (durable, finalized)
            const RETH_HEAD: u64 = ACKED + 1; // clean-shutdown speculative tail
                                              // `Fixture::new` seeds `fx.anchor_hash` at RETH_HEAD (the persisted
                                              // provider head — the speculative-tail block).
            let fx = Fixture::new(RETH_HEAD);
            let acked_hash = B256::repeat_byte(0x77);
            fx.chain.canonical.lock().unwrap().insert(ACKED, acked_hash);

            // reth head (last_execution) = RETH_HEAD, marshal acked cursor
            // (last_consensus, the floor seed) = ACKED < RETH_HEAD.
            let (_actor, _mailbox) = fx.build(ctx, RETH_HEAD, ACKED);

            // The acked cursor resolves via the provider (floor seeded there;
            // beyond reorg).
            assert_eq!(
                fx.chain.finalized_executed_hash(ACKED),
                Some(acked_hash),
                "the marshal-acked height resolves via the floor→provider fallback"
            );
            // The speculative tail sits ABOVE the acked floor: the provider HAS
            // it (clean shutdown persisted the head), but it is notarized-only
            // and a sibling can still finalize ⇒ it MUST NOT be served.
            assert_eq!(
                fx.chain.finalized_executed_hash(RETH_HEAD),
                None,
                "a speculative-tail height above the acked cursor is NOT served \
                 by the floor even though the provider holds it (soundness)"
            );
        });
    }

    // SOUNDNESS (Fix 1, same family as `init_floor_excludes_speculative_tail`):
    // `ordering_finalized` (the result-final cursor) MUST seed from the marshal-
    // acked cursor, NOT the reth head. Seeded from the head (= acked + N), the
    // first finalized delivery at acked+1 computes `result_final = head − K` and
    // pins the engine-API `finalized` (and `head`) onto the SPECULATIVE tail hash
    // at `acked + N − K` — an orphanable sibling. Seeded from the acked cursor the
    // finalized tier stays at the anchor and `update_head` rolls the head onto the
    // re-derived block. Revert-check: with the seed reverted to
    // `last_execution_finalized_height`, both asserts below observe the
    // `acked + N − K` speculative hash instead.
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
            // SIBLING (≠ the witnessed re-derive), acked+N−K a distinct spec hash.
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

            // One finalized delivery at acked+1 (its flush child supplies the
            // witness so it derives). No tip ⇒ guard #2 cold.
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

    // `reseed_forward` is the steady-state MIRROR of `init`'s seed: at a given
    // landing the two MUST agree on field shape (the "never diverge" pin). Seed
    // one actor via `init` at the landing and reseed another there from a stale
    // anchor; their `seed_fields` snapshots must be byte-identical.
    #[test]
    fn reseed_forward_agrees_with_init() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let landing = ANCHOR + JUMP_THRESHOLD + 5_000;
            let landing_hash = B256::repeat_byte(0xE1);
            let floor = landing - K;

            // (1) Cold-start `init` AT the landing (the reference seed). The
            // finalized cursor `init` seeds is the executed tip (landing), with
            // the anchor at the same point.
            let fx_init = Fixture::new(landing);
            fx_init
                .chain
                .canonical
                .lock()
                .unwrap()
                .insert(landing, landing_hash);
            // Distinct labels so the two actors' `pending_finalizations` gauges
            // do not collide in the shared deterministic metrics registry.
            let (init_actor, _m1) = fx_init.build(ctx.with_label("init"), landing, landing);
            let init_fields = init_actor.seed_fields();

            // (2) A second actor cold-started at the STALE anchor, then reseeded
            // forward to the landing.
            let fx_re = Fixture::new(ANCHOR);
            let (mut re_actor, _m2) = fx_re.build(ctx.with_label("reseed"), ANCHOR, ANCHOR);
            re_actor.reseed_forward(landing, landing_hash, floor).await;
            let re_fields = re_actor.seed_fields();

            assert_eq!(
                init_fields, re_fields,
                "reseed_forward must mirror init's seed at the landing (never diverge)"
            );
        });
    }

    // PARENT-VISIBILITY (non-blind): `reseed_forward` MUST issue the
    // canonicalization FCU that mirrors cold-start `init`. With the by-HASH
    // visibility model the test is NON-blind — the backfilled `floor` is present
    // by NUMBER but INVISIBLE by hash until the FCU lands, so deriving `floor + 1`
    // (parent = `floor`) ParentHeaderMissing-fails BEFORE the reseed and succeeds
    // AFTER it. Pre-fix (no FCU in `reseed_forward`) the floor would stay
    // invisible and the floor would freeze.
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
            // Post-devp2p-backfill, pre-FCU state: floor + landing are present by
            // NUMBER and tracked for the by-hash model, but the by-hash frontier
            // sits BELOW floor (the segment is not yet canonical by hash).
            {
                let mut canon = fx.chain.canonical.lock().unwrap();
                canon.insert(floor, floor_hash);
                canon.insert(landing, landing_hash);
            }
            fx.chain.vis.register(floor, floor_hash);
            fx.chain.vis.register(landing, landing_hash);
            fx.chain.vis.set_frontier(floor - 1);

            // (a) Before the reseed FCU: floor is invisible by hash, so deriving
            // floor+1 on top of it ParentHeaderMissing-fails.
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
            actor.reseed_forward(landing, landing_hash, floor).await;

            // The reseed issued the canonicalization FCU: head = safe = landing
            // (covers the whole segment; the landing is BFT ordering-final),
            // finalized = floor (two-tier, never ahead of the result tier).
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

            // (b) After the reseed FCU canonicalized the segment: floor is visible,
            // so deriving floor+1 on top of it now succeeds.
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

    // (b) NO-OP when the gap ≤ JUMP_THRESHOLD: the inlet's ordinary pulls still
    // cover the serving window, so the re-jump callback is never invoked.
    #[test]
    fn re_jump_is_noop_within_serving_window() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let (cb, calls) = recording_re_jump(Scripted::Lagging);
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // Tip exactly AT the threshold (gap == JUMP_THRESHOLD, not >): no fire.
            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD))
                .expect("send tip");

            // Barrier finalize to flush the mailbox past the tip.
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

    // (c'') Connected-but-wedged EL pipeline (soak v43): a `StalledWithPeers`
    // outcome is NON-fatal, must NOT rotate the upstream (the wedge is local to
    // reth, not a bad-upstream branch), must NOT advance the marshal floor (the
    // refill stays deferred — chain-safe), and BUMPS the observability counter so a
    // deterministic re-wedge is visible instead of silent.
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
            // Yield so the spawned waiter completes + its `jump_done` arm runs.
            ctx.sleep(Duration::from_millis(10)).await;

            // Follow-up finalize: must STILL ack ⇒ the loop survived the wedge.
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

    // (c) AuthFailed is now a NON-fatal rotate-and-stay-up-degraded self-heal (#1) —
    // see `re_jump_auth_failed_rotates_and_stays_up_degraded` below (with the rotate
    // recorder). The old fail-closed-shutdown assertion was removed in the same change.

    // (c') THE transient-stall-crash regression test: a `Stalled` outcome (an
    // `EL_SYNC_NO_PROGRESS` transport stall) is NON-fatal — the executor KEEPS
    // RUNNING and a follow-up finalize still acks. Pre-fix, `sync_to`'s `?`
    // propagated the stall as a fatal `Err` and froze the whole chain.
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
            // Yield so the spawned waiter completes + its `jump_done` arm runs.
            ctx.sleep(Duration::from_millis(10)).await;

            // Follow-up finalize: must STILL ack ⇒ the loop survived the stall.
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

    // (d) `Lagging` (stale / shallow target) is a no-op: no re-seed, no set_floor,
    // the executor keeps running.
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
            // Yield so the spawned waiter completes + its `jump_done` arm runs.
            ctx.sleep(Duration::from_millis(10)).await;

            // Barrier finalize: still acks ⇒ the loop survived a Lagging re-jump.
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

    // (d') Rule S/L: a `BadTarget` outcome (a forgeable PRE-anchor structural
    // mismatch served by an untrusted upstream) is NON-fatal — the executor KEEPS
    // RUNNING (a follow-up finalize still acks, no floor advance) AND it rotates the
    // upstream exactly once (a structurally-bad upstream is failed over). A
    // signature-free attacker-controlled input must never crash a node.
    #[test]
    fn re_jump_bad_target_rotates() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let (cb, calls, rotations) = recording_re_jump_with_rotate(vec![Scripted::BadTarget(
                "payload != block digest".into(),
            )]);
            let entered = Arc::new(Mutex::new(Vec::new()));
            let fx = Fixture::new(ANCHOR)
                .with_re_jump(cb)
                .with_boundary_enter(entered.clone());
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                .expect("send tip");
            // Yield so the spawned waiter completes + its `jump_done` arm runs.
            ctx.sleep(Duration::from_millis(10)).await;

            // Follow-up finalize: must STILL ack ⇒ the loop survived BadTarget.
            finalize_and_ack_behind(
                &fx,
                &mailbox,
                sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO),
            )
            .await;

            assert_eq!(
                *calls.lock().unwrap(),
                vec![ANCHOR],
                "re-jump was invoked (gap > threshold) and returned BadTarget"
            );
            assert!(
                entered.lock().unwrap().is_empty(),
                "a non-Landed outcome runs no reseed and must enter no epoch"
            );
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "a BadTarget re-jump rotates the upstream exactly once (Rule L)"
            );
            assert!(
                fx.marshal.floors.lock().unwrap().is_empty(),
                "a BadTarget re-jump must NOT advance the marshal floor"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // (d''') Same NON-fatal rotate-immediately treatment as BadTarget, but for the
    // POST-sync-attempt InvalidTarget outcome (reth itself rejected the served
    // branch mid-EL-sync, el_sync_calls >= 1) — kept as a distinct outcome from
    // BadTarget precisely so BadTarget's own el_sync_calls==0 invariant stays
    // pinned, but the executor's reaction to both is identical (rotate, reset).
    #[test]
    fn re_jump_invalid_target_rotates() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let (cb, calls, rotations) =
                recording_re_jump_with_rotate(vec![Scripted::InvalidTarget(
                    "reth rejected the served tip as INVALID".into(),
                )]);
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                .expect("send tip");
            // Yield so the spawned waiter completes + its `jump_done` arm runs.
            ctx.sleep(Duration::from_millis(10)).await;

            // Follow-up finalize: must STILL ack ⇒ the loop survived InvalidTarget.
            finalize_and_ack_behind(
                &fx,
                &mailbox,
                sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO),
            )
            .await;

            assert_eq!(
                *calls.lock().unwrap(),
                vec![ANCHOR],
                "re-jump was invoked (gap > threshold) and returned InvalidTarget"
            );
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "an InvalidTarget re-jump rotates the upstream exactly once, same as BadTarget"
            );
            assert!(
                fx.marshal.floors.lock().unwrap().is_empty(),
                "an InvalidTarget re-jump must NOT advance the marshal floor"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // #1 SELF-HEAL (2026-07-09): a steady-state re-jump `AuthFailed` (a forged/unagreed
    // POST-sync branch) is NON-fatal — the executor rotates the upstream + stays
    // up-degraded (`auth_rotate=1`) instead of the old `break`/shutdown, and never
    // advances the marshal floor onto the forged branch. With a SINGLE upstream
    // `rotate()` is a no-op, so this same arm keeps the node UP and re-polls on the
    // next tip (recovery from a forged source genuinely needs ≥2 upstreams).
    #[test]
    fn re_jump_auth_failed_rotates_and_stays_up_degraded() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let (cb, calls, rotations) = recording_re_jump_with_rotate(vec![Scripted::AuthFailed(
                "post-sync committee-BLS rejected the served branch".into(),
            )]);
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                .expect("send tip");
            ctx.sleep(Duration::from_millis(10)).await;

            // Follow-up finalize STILL acks ⇒ the loop survived AuthFailed (no
            // shutdown — the pre-fix behaviour would have broken the loop here).
            finalize_and_ack_behind(&fx, &mailbox, sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO)).await;

            assert_eq!(
                *calls.lock().unwrap(),
                vec![ANCHOR],
                "re-jump was invoked and returned AuthFailed"
            );
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "an AuthFailed re-jump rotates the upstream (route around the forged source)"
            );
            assert!(
                fx.marshal.floors.lock().unwrap().is_empty(),
                "an AuthFailed re-jump must NEVER advance the marshal floor (never serve the forged branch)"
            );
            assert_eq!(
                fx.sync_metrics.degraded_value(SyncReason::AuthRotate),
                1,
                "the node stays up-degraded under auth_rotate (it never crashed)"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // #10 SafetyHalt (Phase 3): a steady-state re-jump `L1Fork` (the EL-synced head
    // does NOT descend from the L1-finalized checkpoint) is DISTINCT from `AuthFailed`
    // — there is no honest upstream to rotate to (L1 finality itself disagrees), so
    // the executor HALTS: it engages the fork-safety latch (`l1_fork=1`), does NOT
    // rotate, and PARKS retaining marshal acks (a follow-up finalize is neither
    // derived nor acked — the ack is retained un-resolved so the marshal stays alive).
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
            // The parked executor keeps its mailbox open and RETAINS the ack of a
            // post-halt finalize (never derives it, never cancels the marshal).
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

    // (d'') Rule L: a single `Stalled` must NOT insta-rotate (an honest transient
    // stall is tolerated); only at MAX_UPSTREAM_FAULTS consecutive stalls does the
    // executor fail the upstream over — exactly ONCE — and then the streak resets,
    // so a further single stall does not re-rotate. Mirrors the inlet's
    // `consecutive_data_faults_rotate_once_…` for the SECOND (re-jump) streak.
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

            // Drive MAX_UPSTREAM_FAULTS consecutive Stalled re-jumps. Stalled never
            // reseeds, so `ordering_finalized` stays at ANCHOR and the same tip
            // re-triggers each time once the prior jump's `jump_done` arm has cleared.
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

            // Streak reset after the rotate: one more stall must NOT re-rotate.
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

    // (d''') CRITIC r2 regression: ANY executor-side rotate() resets the streak, so a
    // tally accrued on URL A never carries into URL B. Accrue MAX_UPSTREAM_FAULTS−1
    // Stalleds (streak just below the rotate threshold), then ONE BadTarget (which
    // rotates + resets), then ONE Stalled — the post-BadTarget stall must NOT rotate
    // (proving the BadTarget arm reset the streak; pre-fix it would have tipped to
    // MAX on URL B's first stall → A→B→A oscillation).
    #[test]
    fn re_jump_bad_target_resets_cross_url_streak() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let mut scripts: Vec<Scripted> = (0..crate::cert_inlet::MAX_UPSTREAM_FAULTS - 1)
                .map(|_| Scripted::Stalled("transient stall".into()))
                .collect();
            scripts.push(Scripted::BadTarget("payload != block digest".into()));
            scripts.push(Scripted::Stalled(
                "first stall on the rotated-to URL".into(),
            ));
            let total = scripts.len();
            let (cb, calls, rotations) = recording_re_jump_with_rotate(scripts);
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            for _ in 0..total {
                mailbox
                    .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                    .expect("send tip");
                ctx.sleep(Duration::from_millis(10)).await;
            }

            assert_eq!(
                calls.lock().unwrap().len(),
                total,
                "every scripted outcome was driven"
            );
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "ONLY the BadTarget rotated; the post-BadTarget stall did NOT (streak reset \
                 to 0, so URL A's MAX−1 tally never carried into URL B)"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // Case (A) no-regression: while the tip is HELD (`awaiting_child`), a
    // SHALLOW gap (≤ JUMP_THRESHOLD) must NOT start a re-jump — the hold
    // proceeds untouched. Only a deep gap (> JUMP_THRESHOLD) engages the
    // re-jump.
    #[test]
    fn re_jump_does_not_start_on_shallow_gap_while_block_held() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let (cb, calls) = recording_re_jump(Scripted::Lagging);
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // Finalize a block with no child yet ⇒ it is HELD.
            let (msg, _waiter) =
                finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO));
            mailbox.send(msg).expect("send held block");

            // A SHALLOW frontier tip (gap == JUMP_THRESHOLD, not >) → the gap test
            // early-returns → no re-jump, even though a block is held.
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

    // (f′, Fix A) `reseed_forward` disposes the HELD block with `acknowledge()`
    // (Ok, NEVER a drop — a dropped `Exact` is a Canceled ack, fatal to the
    // marshal): the floor moves past the held height, so it is pruned, not
    // skipped. The one new object the jump path must know about.
    #[test]
    fn deep_gap_while_holding_spawns_rejump_and_acks_held_block() {
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

            // The tip is HELD (its child never arrives — the node is about to
            // jump far past it).
            let (m1, w1) = finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO));
            mailbox.send(m1).expect("send held block");

            // A DEEP frontier tip (gap > JUMP_THRESHOLD) while holding → re-jump spawns.
            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                .expect("send deep tip");

            // The Landed reseed disposes the held block via `acknowledge()` → Ok.
            w1.await.expect(
                "held block disposed via acknowledge (Ok, never Canceled) on the landed re-jump",
            );

            assert_eq!(
                *calls.lock().unwrap(),
                vec![ANCHOR],
                "re-jump SPAWNED once despite the held block (durably-stuck recovery)"
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

    // bug 10: a hole in the marshal's own floor..=last_finalized backfill range
    // cannot self-heal (`get_block` is local-only), so the executor must fail loud
    // AT the backfill site — not warn-and-skip (which relocates + mislabels the
    // fatal to a later gap-walk at the WRONG height). Backfill range = 101..=102
    // with an empty marshal → the first fetch returns None → immediate shutdown,
    // before any block derives.
    #[test]
    fn backfill_hole_is_fatal_at_the_backfill_site() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            // last_consensus = ANCHOR + 2 ⇒ backfill range 101..=102; `canned` empty.
            let (actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR + 2);
            let handle = actor.start();
            // The actor shuts down from the backfill-None fatal (handle joins);
            // pre-fix it would warn-and-skip and keep running (heartbeat loop).
            let _ = handle.await;
            assert!(
                fx.beacon.new_payload_calls.lock().unwrap().is_empty(),
                "a backfill hole must shut down before any block derives"
            );
        });
    }

    // bugs 6/7: while a jump is in flight (`jump_done` armed) the executor is the
    // SINGLE EL writer — NO finalize-derive and NO speculative execute may fire
    // (their FCUs would retarget reth's backfill, starving the jump's `Valid`
    // terminator). A finalize + a spec delivered during the jump must NOT derive;
    // once the jump completes (a no-op `Lagging` landing here) the queued finalize
    // drains.
    #[test]
    fn no_derive_or_spec_while_jump_in_flight_then_drains() {
        use std::sync::atomic::AtomicU64;
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            // A re-jump waiter that HANGS until `release`, then lands as a no-op
            // (`Lagging` — no floor change, so the queued ANCHOR+1 stays derivable).
            let release = Arc::new(tokio::sync::Notify::new());
            let calls: RejumpCalls = Arc::new(Mutex::new(Vec::new()));
            let calls_cl = calls.clone();
            let release_cl = release.clone();
            let call: ReJumpFn = Arc::new(move |from| {
                calls_cl.lock().unwrap().push(from);
                let release = release_cl.clone();
                Box::pin(async move {
                    release.notified().await;
                    crate::cold_start_jump::JumpOutcome::Lagging
                })
            });
            let cb = ReJump {
                call,
                upstream_frontier: Arc::new(AtomicU64::new(0)),
                threshold: JUMP_THRESHOLD,
                rotate: None,
                probe: None,
            };
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // Trigger the jump (far tip) — the waiter hangs, so `jump_done` stays armed.
            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                .expect("send tip");
            ctx.sleep(Duration::from_millis(10)).await;
            assert_eq!(calls.lock().unwrap().len(), 1, "the jump spawned");

            // Deliver a finalize + a spec WHILE the jump is in flight → neither derives.
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

            // Release the jump (no-op landing) → `jump_done` clears → the queued
            // finalize drains (ANCHOR+1 becomes the held tip) and its child's
            // arrival derives it.
            release.notify_one();
            let (fin_child, _wc) = finalize_msg(child_of(&order, None));
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

    // (f) STALE-SPEC: after a far re-jump, a `SpecNotarized` at landing+1 must
    // speculate ⇒ `spec_head == landing` (raised by `reseed_forward`). Pre-fix,
    // `spec_head` was the stale pre-jump tip, so landing+1 != spec_head+1 and the
    // speculation was silently dropped.
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

            // Trigger the re-jump (re-seeds spec_head to the landing).
            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                .expect("send tip");
            // Yield so the spawned waiter completes + its `jump_done` arm re-seeds
            // spec_head to the landing before the speculation below.
            ctx.sleep(Duration::from_millis(10)).await;

            // A notarized block at landing+1 — must speculate (height == spec_head
            // + 1) ONLY if spec_head was raised to the landing.
            let order = sample_order(Digest(B256::ZERO), landing_h + 1, B256::ZERO);
            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(landing_h + 1, order.clone());
            // The node is still behind the deep tip ⇒ guard #2 fires at the
            // finalized derive — can the attested block at landing+1+K, whose
            // result commits the (speculated) hash at landing+1.
            let spec_hash = sealed_at(landing_hash, landing_h + 1, order.digest().0).hash();
            fx.marshal.canned.lock().unwrap().insert(
                landing_h + 1 + K,
                sample_order(Digest(B256::ZERO), landing_h + 1 + K, spec_hash),
            );
            mailbox.send(spec_msg(&order)).expect("send spec");

            // Drain barrier: finalize the SAME order (+ its child, which
            // triggers the derive) — reconciliation skips the re-derive iff the
            // speculation landed first.
            let (msg, waiter) = finalize_msg(order.clone());
            mailbox.send(msg).expect("send finalize");
            let (mc, _wc) = finalize_msg(child_of(&order, None));
            mailbox.send(mc).expect("send child");
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

    // ── REAL-marshal SafetyHalt liveness ─────────────────────────────────────
    // The commonware marshal treats a Canceled `Exact` ack as fatal (its `run`
    // returns), so the pre-fix halt path (executor exits, dropping the ack)
    // killed the marshal — the component that serves blocks + certs to peers —
    // leaving a zombie node. This harness runs the REAL `marshal::core::Actor`
    // (real archives, real dispatch/ack pipeline) against the real executor and
    // pins the fixed posture end-to-end: after a provoked result divergence the
    // marshal is still polled AND still answers a block-by-height request. It
    // also distinguishes the fix from the freeze failure mode — if holding the
    // ack blocked the marshal's loop, `get_block` would never answer.
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

        /// Body-less [`MarshalBuffer`]: every body in this harness is made local
        /// via `verified()` BEFORE its finalization is reported, so the buffer is
        /// dead weight that must only satisfy the `start` bound (the follower's
        /// production stack relies on the same verified-cache-first lookup).
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
                // Sender leaked: "never resolves", not "canceled" — the marshal
                // keeps the subscription open instead of tearing it down.
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

        /// The `FluentApp` reporter seam, minus everything but the executor
        /// forward: `Update::Block` acks travel INSIDE the command, exactly like
        /// production (`application.rs::report`).
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
                .map(|kp| build_signer(&ns, bimap.clone(), kp, None).expect("member"))
                .collect();
            let verifier = build_verifier(&ns, bimap, None, None);
            Committee { signers, verifier }
        }

        /// A real 2f+1 finalization cert over `block`'s digest, signed by `c`.
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

        /// Make `block` local + report its finalization — the marshal stores it
        /// and (contiguously) dispatches `Update::Block` to the executor.
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

                // Real marshal over real (deterministic-runtime) archives.
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
                .await;
                let blocks =
                    crate::outer::init_finalized_blocks_archive(&ctx, "halt-liveness").await;
                let provider = crate::outer::EpochSchemeProvider::new();
                provider.register(Epoch::new(0), c.verifier.clone());
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

                // Real executor (fake EL) fed by the REAL marshal dispatch.
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
                        beacon_metrics: crate::beacon::metrics::BeaconMetrics::default(),
                        sync_metrics: fx.sync_metrics.clone(),
                        safety_halt: fx.safety_halt.clone(),
                        spawn_unblocked: std::sync::Arc::new(tokio::sync::Notify::new()),
                        re_jump: None,
                        seed_store: None,
                        epocher: crate::epocher::OriginEpocher::new(
                            0,
                            std::num::NonZeroU64::new(1 << 40).expect("nonzero"),
                        ),
                    },
                );
                let _executor_handle = executor.start();

                // Keep a live sender so the marshal's resolver_rx never closes.
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

                // Contiguous finalized chain: heights 1..K-1 in the pre-activation
                // window (result MUST be ZERO), height K commits executed_hash(0) —
                // forged, so the executor's cross-check halts at K. Under the
                // one-block-lookahead pipeline each height derives when its
                // child is dispatched, so K's forged cross-check fires when the
                // NEXT block (K+1) is dispatched.
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

                // Dispatching K+1 triggers K's derive → the forged cross-check
                // engages the halt; the executor parks retaining K's ack AND
                // the held K+1 ack.
                let post_halt = sample_order(div_digest, K + 1, B256::ZERO);
                finalize_via_marshal(&mut marshal, &c, &post_halt).await;
                wait_until(&ctx, "SafetyHalt engaged", || fx.safety_halt.is_engaged()).await;
                ctx.sleep(Duration::from_millis(50)).await;

                // THE LIVENESS PROOF, both failure modes: a dead marshal (pre-fix
                // Canceled-ack exit) has a closed mailbox → `get_block` returns
                // None; a frozen marshal (ack awaited inline) never answers →
                // `wait_until` times out.
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

                // Progress stopped: the diverged height is never acked, so the
                // executor never derives past it and no further ack can advance
                // the marshal's processed height.
                assert!(
                    fx.chain.spec_executed_hash(K + 1).is_none(),
                    "no derive past the halted height"
                );
            });
        }

        /// The storage claim boundary seeding rests on: an entry stored BEFORE the
        /// floor rises past it survives the raise and stays readable underneath it.
        ///
        /// Three facts have to hold together for that to be true — `SetFloor` only
        /// advances the cursor (immutable archives keep what they hold), the by-height
        /// read consults no floor, and the below-floor WRITE gate is evaluated when the
        /// message is processed rather than when it is sent. The last one is what makes
        /// ordering on the single mailbox the whole mechanism, so the negative half is
        /// asserted too: the same store attempted AFTER the raise is dropped. If that
        /// ever stops being true the seeding is silently a no-op, which is exactly the
        /// failure this test exists to make loud.
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
                .await;
                let blocks =
                    crate::outer::init_finalized_blocks_archive(&ctx, "seed-below-floor").await;
                let provider = crate::outer::EpochSchemeProvider::new();
                provider.register(Epoch::new(0), c.verifier.clone());
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
                // A live executor mailbox + resolver sender so neither channel closes
                // under the actor; nothing in this test reads from either.
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

        /// Epoch geometry for the seeding tests: epochs are `[0,99]`, `[100,199]`, …
        /// so a floor of 997 buries the terminal 899 and the first block 900.
        fn seeding_epocher() -> crate::epocher::OriginEpocher {
            crate::epocher::OriginEpocher::new(0, std::num::NonZeroU64::new(100).unwrap())
        }

        /// A seam that serves an authenticated pair for every height in `serve`, and
        /// nothing for any other height.
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

        /// Both buried heights are seeded, and both land STRICTLY before the floor
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
                    .await;

                assert_eq!(
                    *fx.marshal.store_floor_order.lock().unwrap(),
                    vec![("store", 899), ("store", 900), ("floor", floor)],
                    "both boundary heights must be stored, and both before the floor rises"
                );
            });
        }

        /// Keyed on the CONDITION, not the event: a node that already holds the pair
        /// does no fetch and no store, even though a jump just landed.
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
                    .await;

                assert!(
                    fx.marshal.stored.lock().unwrap().is_empty(),
                    "nothing to seed when the boundary pair is already local"
                );
            });
        }

        /// Both-or-neither. Seeding only the terminal would satisfy the engine-spawn
        /// gate while leaving the promote VALUE-gate — which reads the epoch's FIRST
        /// block — with nothing to compare against, promoting the member at exactly
        /// the moment that check degrades to a no-op.
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
                    .await;

                assert!(
                    fx.marshal.stored.lock().unwrap().is_empty(),
                    "a partial pair must inject nothing"
                );
            });
        }

        /// The re-jump itself enters the landing epoch. Nothing else can: the floor
        /// raise disqualifies the predecessor terminal from ever being dispatched, and
        /// a delivered boundary block is the only other entry edge.
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
                    .await;

                assert_eq!(
                    *entered.lock().unwrap(),
                    vec![999],
                    "the entry names the terminal of the epoch preceding the landing's own"
                );
            });
        }

        /// Condition-keyed on the LANDING, not on whether seeding found anything to do.
        /// A node that already holds the pair seeds nothing and must still enter.
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
                    .await;

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

        /// The read floor published to the epoch state machine is the FLOOR (result-final),
        /// not the landing (ordering-final only) — and it is published BEFORE the entry,
        /// which is the whole point: the entry's first committee read must already resolve
        /// inside the window the jump left this node with. Both seams record into one sink,
        /// so the assertion pins the value and the order together.
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
                    .await;

                assert_eq!(
                    *calls.lock().unwrap(),
                    vec![floor, 999],
                    "read floor = landing − K, published before the entry at terminal 999"
                );
            });
        }

        /// The entry keys on `terminal_at_or_below(landing)`; the seed beside it keys on
        /// `terminal_at_or_below(floor)`. At a landing within K of an epoch start the two
        /// are a whole epoch apart, and the floor's answer names the epoch just LEFT.
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
                    .await;

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
}
