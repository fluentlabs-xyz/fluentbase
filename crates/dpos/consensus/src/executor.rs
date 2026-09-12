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
//! Derive pipeline: the beacon seed for height `h` is σ of `h`'s OWN agreed
//! round — `Round(epoch(h), h.proposal_view)` — read from the local seed store,
//! so `h` derives at ITS OWN delivery and no block carries another block's
//! randomness. The resolution is PREDICATE FIRST: `mandatory_at(epoch(h))`
//! decides before the store is consulted, so a σ filed at a round the agreed
//! epoch map calls beacon-INACTIVE is ignored (and counted), never obeyed —
//! ignoring derives exactly what the rest of the network derives, where halting
//! would turn one bad journal record into an outage. A MISS on a beacon-active
//! round HOLDS the block in [`Actor::awaiting_seed`]; the only exit is "σ
//! arrived" (the seed-record notify), never a timer and never the
//! `order.digest()` fallback — deriving with the fallback on a beacon-active
//! link re-rolls `prev_randao` and forks.
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
//! [`Actor::awaiting_seed`] until σ arrives, (c) parked with a
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

/// Pacing of an execution-layer call (`fork_choice_updated`, `import_derived`).
///
/// Production runs under commonware's tokio runtime, whose `Pacer::pace`
/// returns the future unchanged (`runtime/src/tokio/runtime.rs:773-785`;
/// `Cell<C>` only delegates, `runtime/src/utils/cell.rs:191-201`), so the
/// `fcu_pace` latency has never delayed an EL call on a node. The deterministic
/// implementation — a `Waiter` that blocks the OS thread until the future is
/// ready — was reachable only through the runtime's `external` feature, which
/// this crate no longer enables: under it the deterministic runtime sleeps a
/// real millisecond per cycle and never skips idle time, which pins every test
/// in the crate to wall-clock time. This local extension keeps the call sites
/// and the `fcu_pace` knob in place with the production (tokio) semantics.
trait PaceElCall: std::future::Future + Sized {
    /// Run the call immediately; `expected_latency` is documentation only.
    fn pace_el_call(self, _expected_latency: Duration) -> Self {
        self
    }
}

impl<F: std::future::Future> PaceElCall for F {}

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
    pub seed: Option<crate::beacon::Seed>,
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
/// and the block re-derives from σ of its own round (the agreed value). The digest half
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
/// - `Notified`: an event-driven re-attempt fired by the executor's wake-up
///   `select!` arm ([`Beacon::subscribe`](crate::beacon::Beacon::subscribe)) when
///   a block is still held and a seed was just recorded (the seed for its round
///   may have JUST landed). A hit here is counted `outcome="recovered"` — the race
///   fired and self-healed WITHOUT a further finalized delivery (the
///   deadlock-breaker, since finality only advances via new blocks). A miss is a
///   silent no-op.
///
///   WHAT MAKES THE RE-ATTEMPT CORRECT IS NO LONGER A STORED PERMIT. It used to
///   be: the arm waited on a `notify_one`, which holds one permit even with no
///   waiter parked, so a record racing the miss could not be lost. The stream is
///   a `broadcast` now, and a broadcast DROPS a send that has no receiver — so the
///   property rests entirely on `subscribe()` being taken before this actor's
///   first seed read (`Actor::run`, ahead of the loop). Move that subscription
///   into the loop, or behind an `awaiting_seed.is_some()` guard, and a σ landing
///   in the window is gone for good: the held height never derives and the marshal
///   ack is held forever.
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
    /// A wake-up for ANOTHER consumer: no derive, stay armed.
    Ignore,
    /// The beacon's sender is gone. A closed `broadcast` receiver returns
    /// `Closed` IMMEDIATELY and forever, so an armed arm would spin without ever
    /// awaiting while a tip is HELD. The arm disarms itself instead — the park
    /// the `Notify` shape had for free, since it also held its own sender.
    Disarm,
}

/// The wake-up arm's whole classification, out of line so it can be pinned by a
/// test: the spin this closes is only observable through a live `select!`.
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

    /// LOCAL read of the `(finalization, block)` pair the marshal archived at
    /// `height`, or `None` on an archive miss. No network, no verification, and
    /// none needed: the only writer of that archive is `store_finalization` AFTER
    /// `verify_delivered` (CW `marshal/core/actor.rs:1404-1463`), so what comes
    /// back is already committee-authenticated — which is what makes it usable as
    /// a jump TARGET (§5.2).
    ///
    /// On the trait rather than on the concrete mailbox because the target is read
    /// through it (`maybe_re_jump`), and the unit tests drive that path through
    /// [`FakeMarshal`]. Separate from [`Self::fetch_block_by_height`] rather than
    /// composed out of it: the two answer different questions (a body for derive
    /// vs an attested pair for the jump) and a test that counts one must not see
    /// the other.
    fn pair_at(
        &self,
        height: Height,
    ) -> impl std::future::Future<Output = Option<(Finalization<BlsScheme, Digest>, OrderBlock)>> + Send;

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

    async fn pair_at(
        &self,
        height: Height,
    ) -> Option<(Finalization<BlsScheme, Digest>, OrderBlock)> {
        // ONE body for this concrete mailbox, not two (review B1-15): the same
        // question already has an implementation on the same type under
        // `FrontierMarshal`, which is what `plane_upstream::serve` answers a
        // `Finalized{h}` fetch from. The two traits exist for different reasons —
        // this one is the executor's erased seam, that one is the frontier
        // producer's — but the read is identical, and duplicating it is how the
        // `Identifier::Latest` trap it avoids (a block finalizing between the two
        // awaits pairs `fin@h` with `block@h+1`) would get fixed in one copy only.
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

/// How many times the finalized re-apply loop may re-walk on a still-invisible
/// parent before it dies loudly. At `ENGINE_TRANSPORT_RETRY_BACKOFF` per
/// iteration this is the SAME ~10 s budget `derive_with_visibility_retry`
/// already spends on this exact transient (`application.rs`'s `DEADLINE`),
/// expressed as a count because the loop's own backoff sets the cadence. The
/// bound is the point: an unbounded retry here is the silent spin this gate
/// exists to remove.
const REAPPLY_PARENT_VISIBILITY_RETRIES: u32 = 50;

/// How many times the finalized-tier postcondition re-reads the EL before an
/// absent block at a finalized height becomes a corruption verdict. At
/// `ENGINE_TRANSPORT_RETRY_BACKOFF` per re-read this is the SAME ~10 s budget
/// `REAPPLY_PARENT_VISIBILITY_RETRIES` spends, for the same reason: a height the
/// devp2p backfill just landed is by-NUMBER invisible for a moment
/// (`reseed_forward` leans on a belt for exactly this), and killing a node that
/// would have healed is the worse error.
const FINALIZED_TIER_VISIBILITY_RETRIES: u32 = 50;

/// Returns the current committee's peers to target for a finalization re-fetch,
/// or `None` if no committee is known yet. Re-invoked per retry so it tracks the
/// catch-up walk's advancing epoch.
pub type PeersForFinalization =
    std::sync::Arc<dyn Fn() -> Option<NonEmptyVec<PeerPubkey>> + Send + Sync>;

/// Steady-state self-healing re-jump callback. Invoked from the `Update::Tip`
/// arm when the marshal tip runs more than [`ReJump::threshold`] finalized
/// blocks ahead of the highest derived ordering height (the upstream's serving
/// window is exactly that wide, so beyond it `UpstreamResolver::fetch` returns
/// nothing forever → the marshal floor freezes → the executor wedges). The
/// callback runs the SAME forward-only [`crate::cold_start_jump::jump_to_target`]
/// the cold-start path runs, fast-forwarding reth via one FCU + devp2p backfill.
///
/// The generics of the underlying jump (committee source / EL-sync) are ERASED
/// behind this boxed `Fn` so the executor [`Actor`] gains NO new generic params.
/// The executor SPAWNS the future as a READ-ONLY waiter (the same spawned-fetch
/// idiom the inlet uses) and reacts to its terminal
/// [`crate::cold_start_jump::JumpOutcome`] on a `oneshot` `select!` arm — NOT an
/// in-task poll. The jump's only reth touch is the read-side `sync_to` FCU, which
/// reth ancestor-skips when backward, so the spawned waiter cannot corrupt the
/// executor's own forward FCUs.
///
/// TWO arguments, and the second is the §5.2 change: `from` = the trigger's
/// `ordering_finalized`, and the TARGET — the `(finalization, block)` pair the
/// executor read out of its OWN marshal archive at the tip it is triggering on.
/// The callback no longer asks anybody for a target: the only writer of that
/// archive is `store_finalization` after `verify_delivered` (CW
/// `marshal/core/actor.rs:1404-1463`), so the target is already
/// committee-authenticated before the jump sees it.
///
/// It returns the typed terminal [`crate::cold_start_jump::JumpOutcome`] (the
/// spawn owns the whole backfill wait, so there is no in-progress variant):
/// `Landed` ⇒ re-seed + advance the running marshal floor; `Lagging` ⇒ no-op;
/// `Stalled` ⇒ NON-fatal transport stall (re-evaluated on the next
/// `Update::Tip`); `InvalidTarget` ⇒ the EL did not land on the attested branch.
/// There is no `BadTarget` / `AuthFailed` any more: the two `verify_jump_*` stages
/// behind them re-checked what the target already carried, and pass Б2 removed
/// stages and variants together.
pub type ReJumpFn = std::sync::Arc<
    dyn Fn(
            u64,
            crate::cert_follow::UpstreamFinalized,
        ) -> BoxFuture<'static, crate::cold_start_jump::JumpOutcome>
        + Send
        + Sync,
>;

/// `dpos_frontier_step_unserved_total` — one probe tick where the tip was frozen,
/// the ladder step `Finalized{last(T+1)}` was REQUESTED, and nobody in
/// `committee[T+1]` served it inside the fetch bound. §5.4 calls this outcome
/// "догон вместо прыжка": the node is not stuck, it keeps walking contiguously
/// from the floor and the next tick asks again.
const FRONTIER_STEP_UNSERVED: &str = "dpos_frontier_step_unserved_total";

/// `dpos_frontier_step_skipped_total{reason}` — one probe tick where the ladder
/// step was NOT put at all. Either this node cannot NAME `committee[T+1]`
/// (`no_tracked_epoch`, `no_geometry`, `out_of_window`, `not_readable`,
/// `read_failed`, `no_participants`), or the marshal would discard it
/// (`at_or_below_the_floor`). The probe then asks `Latest` alone.
///
/// `above_the_frontier` is GONE (§5.2, review A2-01): the step used to wait for
/// an unauthenticated `Latest` height to witness that the network had produced
/// `last(T+1)`, and that witness went with the unauthenticated trigger input it
/// shared. A step nobody has costs one unanswered fetch, which §5.4 already
/// calls "догон вместо прыжка".
pub(crate) const FRONTIER_STEP_SKIPPED: &str = "dpos_frontier_step_skipped_total";

/// What one frontier probe tick produced — one answer and one ADDRESS.
///
/// The probe does two things per tick (§5.2 "Триггер и лестница"), and only one
/// of them is a network call here. It ASKS the upstream for `Latest`, whose
/// height is the hint / re-jump driver. And it NAMES the ladder step —
/// `Finalized{last(T+1)}` and the committee to address it at — which the
/// executor then puts on the marshal's own resolver, because that resolver is
/// the one whose deliveries end in `store_finalization`, the single writer §5.2
/// names. Naming it here rather than fetching it here is what keeps this file
/// from becoming a second writer of the same finalization.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProbeOutcome {
    /// The height of a `Latest` answer that passed `deliver`, or `None` when the
    /// probe was not served / the answer was refused or dropped as
    /// unauthenticated.
    ///
    /// ONE consumer left: `hint_finalization(frontier)` when it stands above the
    /// marshal tip — a HINT, which the marshal answers by fetching the height and
    /// running its own `verify_delivered` before anything is stored. It no longer
    /// feeds the jump trigger (that reads the marshal tip alone, §5.2) and no
    /// longer gates the ladder step (`servable` is gone, review A2-01), so an
    /// unauthenticated height can now buy exactly one by-height fetch and nothing
    /// else.
    pub frontier: Option<Height>,
    /// `(last(T+1), committee[T+1])` — the ladder step and its addressees.
    /// `None` when this node cannot name them yet: no epoch tracked, no frozen
    /// geometry, or `committee[T+1]` outside the read window / not readable at
    /// this anchor. The probe then asks `Latest` alone.
    pub step: Option<(Height, NonEmptyVec<PeerPubkey>)>,
}

/// Erased frontier probe (see [`ReJump::probe`]): asks the upstream for `Latest`
/// and names the ladder step for the tracked epoch `T`. The argument is `T` —
/// `None` before the node has tracked any epoch, which is the one state with no
/// step to take.
///
/// The `Latest` HEIGHT it returns is still trusted height-only, and that is now a
/// much narrower claim: the answer it came from passed the five checks of
/// [`crate::plane_upstream::FrontierHandler`]'s `deliver`, so an inflated tip is
/// refused at the channel instead of at the end of a wasted backfill.
pub type FrontierProbeFn =
    std::sync::Arc<dyn Fn(Option<u64>) -> BoxFuture<'static, ProbeOutcome> + Send + Sync>;

/// Erased "which epoch did this node last hand to `track`" — `T` of §5.2, read
/// from the process's ONE `EpochTransition` (`last_tracked_epoch`, which advances
/// only once the boundary trigger has been delivered,
/// `epoch_transition.rs:768-791`).
///
/// `None` until the first epoch is tracked. Synchronous and cheap: it reads a
/// cell the transition's own bridge forwarder writes, never the transition's
/// async mutex, so a probe tick can never block on the epoch machine it is
/// asking about.
pub type TrackedEpochFn = std::sync::Arc<dyn Fn() -> Option<u64> + Send + Sync>;

/// Erased "this node's history now begins here" publisher: raises
/// `EpochTransition`'s read-height floor to the `u64` argument. The re-jump
/// landing's twin of [`Config::boundary_enter`], and AWAITED where that one is
/// fire-and-forget — the floor must be in place before the entry's first committee
/// read, and the state machine sits behind an async mutex, so a spawn would race
/// the very entry this unblocks.
pub type BoundaryReadFloorFn = std::sync::Arc<dyn Fn(u64) -> BoxFuture<'static, ()> + Send + Sync>;

/// The committee module's "my anchor moved" wake-up, erased to the ONE verb the
/// executor owes it.
///
/// The module reads every epoch committee at `executed_state_hash(anchor)`
/// where `anchor` is this node's ordering-finalized cursor — the very cursor
/// this actor raises. Nothing else in the process knows when that happens, so
/// the executor tells it, STRICTLY AFTER each of the three
/// `ExecutedChain::advance_finalized` calls (init seed, jump landing, finalized
/// derive). Calling before merely loses one wake-up, which the next one makes
/// good; not calling at all leaves every consumer parked on
/// `CommitteeError::NotReadable` until some other read happens to succeed.
///
/// A one-verb handle rather than the `Arc<dyn Committee>` itself, deliberately:
/// the executor must not grow a committee read of its own beside the two tiers
/// it already reconciles.
pub type AnchorAdvancedFn = std::sync::Arc<dyn Fn() + Send + Sync>;

/// The steady-state re-jump callback bundled with the signal its trigger reads.
#[derive(Clone)]
pub struct ReJump {
    /// The forward-only jump onto a target read from this node's own marshal
    /// archive (`(from, target)`); see the callback notes above.
    pub call: ReJumpFn,
    /// Forward-only re-jump need-gate, mirrored into the spawned jump's own gate
    /// (`jump_to_target`). The defer deadlock is EPOCH-relative
    /// ("≥2 epochs behind ⇒ `committee[E]` uncommitted"), so its recovery gate is
    /// epoch-relative too: `min(JUMP_THRESHOLD, epoch_block_interval)` (real-prod
    /// epochs ≫ 1024 keep the 1024 serving-window size; a compressed test epoch
    /// scales down to ~1 epoch so a short-epoch node heals within an epoch instead
    /// of waiting for a fixed 1024-block gap).
    ///
    /// BOTH node kinds use the epoch-relative value — the follower and the
    /// validator-with-upstream alike (`dpos.rs` computes the same
    /// `JUMP_THRESHOLD.min(interval)` on each path). Only the tests construct a
    /// bare `JUMP_THRESHOLD`. There is no cold-start jump beside it any more (pass
    /// Б2): a node with an empty archive anchors at its own EL-finalized tag and
    /// climbs from there, so this is the only jump gate in the system.
    pub threshold: u64,
    /// The inlet's EXISTING upstream-rotation escape ([`crate::cert_inlet::RotateUpstream`]),
    /// the SAME `CertUpstream::rotate_callback()` the data-fault inlet uses. Fired
    /// when the re-jump's terminal outcome is a fault (Rule L). After pass Б2 the
    /// ONLY arm that fires it is `Stalled`, and only after `MAX_UPSTREAM_FAULTS`
    /// consecutive ones (an honest transient stall must not insta-rotate) — the two
    /// insta-rotating arms, `BadTarget` and `AuthFailed`, are gone with the stages
    /// that produced them. `InvalidTarget` does NOT rotate (review B1-04): its
    /// target is this node's own attested archive pair, so the contradiction is
    /// local and §5.4 files it as `Fault::corruption`.
    /// `Option` so unit tests / a no-rotate config leave it `None`.
    pub rotate: Option<crate::cert_inlet::RotateUpstream>,
    /// Upstream frontier-discovery probe, fired from the executor's 1 s probe
    /// tick whenever `last_tip_height` did NOT advance since the previous tick
    /// (the marshal is learning no new finalizations). This is the LIVE-FOLLOW
    /// driver for a validator with no cert-inlet (the plane-native default): a
    /// ROTATED-OUT validator participates in no consensus and has no inlet, so
    /// without the probe its marshal tip freezes at the demotion boundary — no
    /// `Update::Tip`, no prehints ⇒ a permanent silent wedge.
    ///
    /// Everything the probe produces now lands on the MARSHAL and nowhere else:
    /// the ladder step `Finalized{last(T+1)}` addressed at `committee[T+1]`, and
    /// `hint_finalization(frontier)` when the answered `Latest` stands above the
    /// tip. The marshal fetches, verifies against the epoch scheme, stores, and
    /// the normal `Update::Tip` pipeline walks the gap — which is also how the
    /// jump trigger learns anything, since it reads that tip alone (§5.2).
    /// Self-silencing while live: an advancing tip skips the network probe
    /// entirely. `None` in unit tests.
    pub probe: Option<FrontierProbeFn>,
    /// `T` for the ladder step the probe takes — see [`TrackedEpochFn`]. `None`
    /// in unit tests and wherever no `EpochTransition` is wired; the probe then
    /// asks `Latest` alone, exactly as it did before the ladder existed.
    pub tracked_epoch: Option<TrackedEpochFn>,
}

/// A finalized block PARKED by guard #2 (the node is ≥ K behind — `last_tip
/// >= h + K` — but the committee-attested body at `h + K` is not backfilled
/// yet, so the convergence check cannot run). The ONLY park in the executor.
/// The seed was already resolved when the park was taken, so the
/// re-poke re-derives with ZERO lookups. The `pending_finalizations` drain is
/// paused while this is `Some`, which preserves strict derive order and lets
/// the marshal's `MAX_PENDING_ACKS` backpressure bound the queue. Event-driven:
/// re-poked by the marshal's live `Update::Tip`/`Update::Block` delivery stream
/// + the FCU heartbeat — NEVER a wall-clock give-up (§8.11).
struct Deferred {
    cause: Span,
    order: OrderBlock,
    ack: Exact,
    /// The resolved σ for this block's own round, retained with the park so
    /// `repoke_deferred` is a plain "is `h + K`'s body here yet" retry.
    seed: Option<crate::beacon::Seed>,
}

/// A finalized block HELD for σ of its own round — the executor's only hold, and
/// NOT a park: no gauge, no hint, no re-poke, no deadline. The sole exit is σ
/// arriving on the seed-record notify.
struct HeldForSeed {
    cause: Span,
    order: OrderBlock,
    ack: Exact,
    /// When the block FIRST entered the hold. PRESERVED across every miss
    /// re-hold, so [`Actor::detect_stalled_seed_hold`] measures the age of the
    /// HOLD rather than the age of the last failed lookup — a notify storm that
    /// reset this would silence the detector exactly when it matters most.
    since: SystemTime,
    /// This hold has already been reported. The warn + counter fire ONCE per
    /// hold, not once per heartbeat: a stall outliving its threshold is one
    /// event, and repeating it every tick would bury the log it exists to make
    /// readable.
    reported: bool,
}

/// How long a block may sit in [`Actor::awaiting_seed`] before the DETECTOR
/// reports it. **Not a deadline** — nothing derives, skips or aborts when it
/// elapses (see [`Actor::detect_stalled_seed_hold`]); it only decides when a
/// stall becomes visible.
///
/// 60 s is 60 blocks of ordering at the 1 blk/s target rate — progress the
/// executor has not followed. Every honest source of a hold is far shorter: σ is
/// filed from the SAME finalization certificate that makes the marshal dispatch
/// the body, so the record-vs-delivery race is one certificate wide, and the
/// slowest legitimate case — a follower whose σ quarantines until `PK_epoch`
/// lands — is bounded by one artifact fetch. It is also far below the horizon
/// where the re-jump takes over (`JUMP_THRESHOLD` = 1024 blocks ≈ 17 min at
/// 1 blk/s), so a stall is named as a SEED hold before a deep-gap jump can paper
/// over it.
const SEED_HOLD_STALL_THRESHOLD: Duration = Duration::from_secs(60);

/// What [`Actor::seed_at_own_round`] found for a height's own agreed round.
enum OwnRoundSeed {
    /// The beacon is active in this height's epoch and σ is in the store.
    Present(crate::beacon::Seed),
    /// The beacon is NOT mandatory in this height's epoch (or the epocher cannot
    /// name it): the agreed derivation is `None` and no σ can change that.
    Inactive,
    /// Beacon-active, σ not recorded yet — the block must WAIT, never fall back.
    Missing,
}

/// Result of attempting to derive a finalized block.
enum DeriveOutcome {
    /// Derived + imported + FCU'd + acked.
    Done,
    /// Guard #2 could not run: the node is ≥ K behind but the committee-attested
    /// body at `height + K` is not backfilled yet. The park payload (block, ack
    /// AND the already-resolved σ) is handed back to be PARKED + re-poked
    /// event-driven; boxed to keep the hot `Done` arm small.
    NeedAttestation(Box<Deferred>),
    /// The gap-walk's canonicalization FCU could not land — reth answers SYNCING
    /// while a backfill holds the engine exclusively, and the cold-start jump
    /// starts exactly such a backfill. The parent stays invisible, so the derive
    /// cannot proceed and MUST NOT be fatal: park and re-poke, the way reth warns
    /// and heals rather than dying. Same payload as [`Self::NeedAttestation`]; a
    /// distinct variant because the fresh-park side effects differ (no `h + K`
    /// hint — nothing is missing from the archive here).
    NeedParentVisible(Box<Deferred>),
    /// A gap-walk PREFIX element sits on a beacon-active round whose σ is not in
    /// the store yet. The walk reports the typed leaf; THIS is where it parks.
    /// The payload carries the DELIVERED height's σ, never the prefix element's —
    /// the prefix lookup re-runs inside the walk on every re-poke, so the value
    /// that was missing is re-resolved rather than carried forward. That is why
    /// the main path's "a park would carry `None`" objection does not reach this
    /// arm: there is no `None` to carry.
    NeedPrefixSeed(Box<Deferred>),
}

/// What the run loop must do after [`Actor::dispatch_fault`] disposed of a
/// fault. A `ForkSafety` fault never produces one of these — the router parks
/// forever instead of returning.
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
    /// The randomness handle (cross-epoch singleton from `outer.rs`). The
    /// executor reads exactly three operations off it:
    /// [`crate::beacon::Beacon::mandatory_at`], the network-agreed "is the beacon active in
    /// this epoch" that gates every seed lookup; [`crate::beacon::Beacon::seed`], to
    /// re-canonicalise the SPECULATIVE seed round to the block's own
    /// `proposal_view` and to resolve the finalized derive's own round; and
    /// [`crate::beacon::Beacon::subscribe`], to wake when a seed lands. A provider with no
    /// seeds degrades both to "skip speculation on a spin-round notarization"
    /// and "hold the tip until σ arrives" — never to speculating with a
    /// known-wrong seed.
    pub randomness: std::sync::Arc<dyn crate::beacon::Beacon>,
    /// Cross-epoch block→epoch map (the same singleton threaded into marshal +
    /// `epoch_manager`, `outer.rs`). Used to form `h`'s own seed round
    /// `Round(epocher.containing(h).epoch(), h.proposal_view)` — a pure function
    /// of AGREED data, so every honest node resolves the identical σ and a wrong
    /// epoch can only MISS, never yield a wrong seed.
    pub epocher: crate::epocher::OriginEpocher,
    /// The committee module's anchor wake-up — see [`AnchorAdvancedFn`]. Called
    /// at the three sites that raise the finalized-execution cursor, and
    /// nowhere else.
    pub anchor_advanced: AnchorAdvancedFn,
    /// The executor's own counters (cross-launch singleton from
    /// `dpos.rs::launch`, already registered there): `seed_active` /
    /// `digest_fallback`, one increment per derived block.
    pub metrics: ExecutorMetrics,
    /// Self-heal observability handle (cross-launch singleton from
    /// `dpos.rs::launch`, already registered there). The executor raises
    /// `dpos_sync_degraded{reason=engine_retry}` while retrying a transient
    /// engine-API TRANSPORT error at the finalize FCU (#14), and CLEARS
    /// `{reason=crash_recover}` when the STARTUP BACKFILL DRAIN finishes — the #12
    /// cold start (`dpos.rs`, `RecoverOutcome::DeferToElSync`) anchors at reth's
    /// tip and defers closing its EL gap to exactly that drain, not to a jump.
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

/// The two counters the executor owns: one per derived block, saying whether
/// `prev_randao` was the verified threshold seed or the digest fallback.
///
/// Split out of `BeaconMetrics` with both family names unchanged. Owned HERE and
/// registered on BOTH node classes, because the executor runs on both and cannot
/// tell which it is on — making these "beacon-owned on a validator, absent-owned
/// on a follower" would register them twice on every follower, which
/// `prometheus_client::Registry` accepts silently and only a scrape reveals.
#[derive(Clone, Debug, Default)]
pub struct ExecutorMetrics {
    /// A block's `prev_randao` was the verified threshold seed (`assurance=true`).
    pub seed_active: prometheus_client::metrics::counter::Counter,
    /// A beacon-active block fell back to `order.digest()` (seed absent or failed
    /// σ-verify vs `PK_E`). The Stage-2 certify hook Nullifies a beacon-active
    /// boundary before it finalizes, so this counts the LOCAL pre-Nullify observation
    /// on a node that derived ahead of the Nullify; smoke D1 asserts it is 0
    /// post-anchor on a healthy chain.
    pub digest_fallback: prometheus_client::metrics::counter::Counter,
}

impl ExecutorMetrics {
    /// Register both counters. Call ONCE per process, against the SAME context
    /// the other metric structs are registered against — commonware prefixes
    /// each family with the context's label path, so a labelled child context
    /// would rename them in the scrape without any gate noticing.
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
    /// Steady-state self-healing re-jump callback (see [`ReJump`]). Fired from
    /// the `Update::Tip` arm when the frontier runs > `JUMP_THRESHOLD` ahead of
    /// `ordering_finalized`.
    re_jump: Option<ReJump>,
    /// Consecutive steady-state re-jump `Stalled` outcomes since the last reset. At
    /// `MAX_UPSTREAM_FAULTS` the executor `rotate()`s (Rule L) and resets. Reset to 0
    /// on that rotate and on `Landed`/`Lagging` (progress / the gap closed under the
    /// threshold) — so a streak accrued on URL A NEVER carries into URL B (critic r2:
    /// cross-URL carryover → A→B→A oscillation). A SECOND, independent streak from the inlet's data-fault
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
    /// deferred park).
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
    /// A ladder step was put on the previous probe tick and the tip has not moved
    /// since. The ONLY thing it decides is whether
    /// `dpos_frontier_step_unserved_total` ticks: a step is "unserved" exactly
    /// when it was asked for, a whole tick passed, and the tip stayed frozen
    /// (this arm runs only at a frozen tip). Not a state machine — the ladder is
    /// the repetition of the tick, and this bit is a metric's memory.
    probe_step_pending: bool,
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
    /// `Exact` ack AND its already-resolved σ. The `pending_finalizations`
    /// drain is paused while this is `Some` (preserves strict order). Re-poked
    /// event-driven off the marshal's live delivery stream
    /// (`Update::Tip`/`Update::Block`) PLUS the existing FCU heartbeat (the
    /// last-catch-up-block completeness backstop: a body landing at
    /// `height <= tip` fires no `Update::Tip`, so pure delivery re-poke would
    /// deadlock there — [[dpos-deferred-catchup-invariants]] #3). There is NO
    /// give-up timer: a never-arriving body keeps the block parked, never shuts
    /// the executor down (§8.11).
    deferred: Option<Deferred>,

    /// The delivered, not-yet-derived finalized block whose σ has not landed
    /// yet. Beacon-ACTIVE rounds only: an inactive round derives `None`
    /// immediately and never reaches this slot. It is HELD (not parked: no
    /// gauge, no hint, no re-poke, no deadline) and derived by the seed-record
    /// notify arm the moment σ lands — the ONLY exit, since nothing else can
    /// supply the value. Both block feeders (the `pending_finalizations` drain
    /// and the startup-backfill walk) are gated on this slot being empty, which
    /// is what keeps a second delivery from overwriting a live `Exact`.
    /// Two dispositions for the held ack, and they are NOT the same: on a re-jump
    /// (`reseed_forward`) it is acked `Ok` — the floor MOVES, so the height is
    /// pruned, not skipped; on shutdown / task exit it is DROPPED, deliberately,
    /// never acked — the withheld ack IS the restart self-heal (module docs).
    /// Needs no persistence. On a `SafetyHalt` it joins `park_halted`'s retained
    /// set, exactly like `deferred.ack`.
    awaiting_seed: Option<HeldForSeed>,

    /// See [`Config::randomness`]. Read by `spec_execute`'s §4.1 round
    /// re-canonicalisation AND by [`Self::seed_at_own_round`], the sole seed
    /// source of the finalized derive.
    randomness: std::sync::Arc<dyn crate::beacon::Beacon>,

    /// See [`Config::epocher`]. Read ONLY by [`Self::seed_at_own_round`] to form
    /// `h`'s own agreed seed round.
    epocher: crate::epocher::OriginEpocher,

    /// See [`Config::anchor_advanced`]. Called from the three sites that raise
    /// the finalized-execution cursor, immediately after each one.
    anchor_advanced: AnchorAdvancedFn,

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
        // result (divergence, re-entered through restart).
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
        // The committee module anchors on the cursor just seeded — tell it, so
        // the first reads of this process are taken at the restart anchor rather
        // than at height 0. STRICTLY AFTER the seed above (see
        // [`AnchorAdvancedFn`]).
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

        // The beacon's wake-up stream (piece D, family2_finalized_tier.md §2.2).
        // SUBSCRIBED HERE, before the loop and therefore before this actor's first
        // seed read: a `broadcast` buffers from the subscription onward, so an
        // event fired before it would be lost — and the first read below is what
        // decides whether a height is HELD at all. Held as a LOCAL so the arm's
        // future never borrows `self`, which the arm's `&mut self` body needs. A
        // beacon with no seeds never publishes: the arm parks forever, exactly as
        // the old `None` did.
        let mut beacon_events = self.randomness.subscribe();
        // Set when the beacon's sender is gone (`RecvError::Closed`). The arm's
        // guard reads it, because a closed `broadcast` receiver returns `Closed`
        // IMMEDIATELY and forever: left armed, the arm would spin without ever
        // awaiting while a tip is HELD. The HEAD shape parked on a `Notify` whose
        // sender it also held, so there was nothing to close; disarming keeps that
        // behaviour (park forever) instead of a hot loop.
        let mut beacon_events_closed = false;

        loop {
            // PRE-CLASS latch gate. `dispatch_fault` reads the latch only when a FAULT
            // reaches it, so a latch engaged with NO fault in flight — the datadir
            // marker restored at startup (`sync_metrics::SafetyHalt::restore_marker`)
            // — left this actor driving reth anyway. The latch means "stop writing to
            // the EL", and this is the writer: park before pulling any work, retaining
            // every marshal ack.
            if self.safety_halt.is_engaged() {
                self.park_halted(
                    "halt latch engaged before dispatch",
                    eyre::eyre!("SafetyHalt latch engaged; executor parking without deriving"),
                )
                .await;
            }

            // Do not pull more work while a block is deferred awaiting its h+K
            // attested body (guard #2) — the deferred block must derive first
            // (strict order) — nor while a block is HELD awaiting its σ (same
            // strict-order reason, and `on_finalized_block` would otherwise
            // overwrite a live `Exact`) — nor while a
            // jump is in flight (bugs 6/7: the jump is the SINGLE EL writer during
            // backfill; a competing startup-drain FCU retargets reth's backfill).
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
                            // Synthetic ack (the marshal already acked these
                            // heights on a previous run); routes through the SAME
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
                                // #12 ENDS HERE, and NOT at a jump landing (4.2 Б2).
                                // `dpos.rs`'s `RecoverOutcome::DeferToElSync` anchors the
                                // cold start at reth's tip and raises
                                // `dpos_sync_degraded{reason=crash_recover}`; THIS drain is
                                // what walks `(reth tip .. marshal cursor]` back into reth,
                                // block by block, through the same derive+import path live
                                // dispatch uses. `maybe_re_jump` cannot do it and never
                                // fires there: `last_tip_height` and `ordering_finalized`
                                // are BOTH seeded from `last_consensus_finalized_height`
                                // (`:1264`, `:1289`), so their difference is 0 at boot, and
                                // the gate additionally refuses to spawn while this drain
                                // is non-empty (`maybe_re_jump`, the
                                // `finalized_heights_to_backfill` clause). The last drained
                                // height is therefore the one moment the deferral is over.
                                self.sync_metrics.recover(SyncReason::CrashRecover);
                                self.sync_metrics.crash_recover_gap_blocks.set(0);
                            }
                        }
                        None => {
                            // bug 10, named by its CAUSE (4.2 Б2 fix-1, B2-02).
                            //
                            // The range this drain walks is
                            // `(reth's last block .. the marshal's acked cursor]` (`init`),
                            // and that cursor IS the marshal's finalized floor — the same
                            // `last_processed_height` under `LATEST_KEY` that `SetFloor`
                            // compares against and refuses to move below
                            // (`.claude/COMMONWARE_INTERNALS.md:190-193`: repair starts at
                            // `last_processed_height.next()`, `HintFinalized` skips `<=`
                            // it, `store_finalization` drops `<=` it). So EVERY height the
                            // drain asks for is at or below the floor, and a miss here is
                            // a hole the marshal will never repair from anywhere — not a
                            // transient. The reachable cause is an EL rolled back (a
                            // snapshot restore) below a range this node once JUMPED OVER
                            // and therefore never stored.
                            //
                            // Fail loud AT the true site: a skip merely relocates +
                            // mislabels the fatal — the later gap-walk
                            // (`derive_finalized_with_gap_fill`) re-hits the same height
                            // and fails naming the WRONG one. Routed rather than `break`n:
                            // a bare break leaves the loop WITHOUT reading the halt latch,
                            // so an already-halted node exits and drops every retained
                            // marshal `Exact` into Canceled — which the marshal treats as
                            // fatal.
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
                            if let Err(fault) = self.reseed_forward(landing, hash, floor).await {
                                if self.dispatch_fault("re-jump reseed", fault).await
                                    == Disposition::Shutdown
                                {
                                    break;
                                }
                            }
                            // Progress: clear any stale fault tally. The #12
                            // crash-recover gauge is NOT cleared here — that deferral
                            // ends at the startup drain's last height, not at a jump
                            // landing (see the `pending_backfill` arm above).
                            self.rejump_fault_streak = 0;
                        }
                        Ok(crate::cold_start_jump::JumpOutcome::Lagging) => {
                            debug!("steady-state re-jump: lagging / stale target — no-op");
                            // The gap closed under the threshold on its own — clear
                            // any stale tally.
                            self.rejump_fault_streak = 0;
                        }
                        Ok(crate::cold_start_jump::JumpOutcome::InvalidTarget(error)) => {
                            // §5.4 "Посадка не на заверенную ветку" / "reth Invalid":
                            // `Fault::corruption`, and NOT the rotation this arm used
                            // to do (review B1-04).
                            //
                            // The target of a steady-state jump is the
                            // `(finalization, block)` pair this node read out of its
                            // OWN marshal archive (`maybe_re_jump`), which only
                            // `store_finalization` writes and only after
                            // `verify_delivered` — so the pair carries 2f+1 under a
                            // committee this node read itself. There is no upstream
                            // that chose it and therefore nobody to rotate AWAY from:
                            // rotating would move the `Latest`/by-height seam and
                            // leave the contradiction standing.
                            //
                            // Two causes reach here, and both say the same thing
                            // about THIS node: reth rendered `Invalid` on the
                            // attested branch mid-EL-sync, or `holds(result) == false`
                            // after a `Valid` — the EL sat down somewhere other than
                            // the branch a quorum attested. Either way the local EL
                            // contradicts an authenticated certificate, which is the
                            // corruption class: loud actor death, no further EL
                            // writes, the supervisor aborts-all.
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
                        Ok(crate::cold_start_jump::JumpOutcome::L1Fork(error)) => {
                            // #10 SafetyHalt (Phase 3): the EL-synced head does NOT
                            // descend from the L1-FINALIZED checkpoint — a fork
                            // against L1 finality, the strongest trust root. There
                            // is nothing to rotate to — L1 finality itself disagrees
                            // — so HALT (demote to verify-only, stop driving reth,
                            // stay observable) and wait for the L1 proof + governance
                            // recovery.
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
                            // The spawned waiter dropped its sender (task aborted /
                            // panicked) without sending — nothing to act on.
                            debug!("steady-state re-jump waiter canceled before completion");
                        }
                    }
                }

                Some((cause, block, ack)) = self.pending_finalizations.next(),
                if self.deferred.is_none()
                    // A block HELD for its σ must derive before the next one is
                    // pulled: strict order, and `on_finalized_block`'s slot
                    // assignment would otherwise drop a live `Exact` (Canceled →
                    // fatal to the marshal). Queued entries stay ALIVE meanwhile,
                    // bounded by the marshal's MAX_PENDING_ACKS, exactly as under
                    // the `deferred` gate. `maybe_re_jump` is deliberately NOT
                    // gated on this — that gate is what bounds a σ-less node's
                    // stall.
                    && self.awaiting_seed.is_none()
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
                    if let Err(fault) = self.on_finalized_block(cause, block, ack).await {
                        if self.dispatch_fault("finalize", fault).await == Disposition::Shutdown {
                            break;
                        }
                    }
                }

                msg = self.mailbox.recv() => {
                    let Some(msg) = msg else {
                        // Not a fault — every sender dropped, the node is tearing this
                        // executor down. The latch still governs whether we may LEAVE:
                        // a halted executor holds marshal `Exact` acks, and returning
                        // here drops them into Canceled, which the marshal treats as
                        // fatal.
                        if self.safety_halt.is_engaged() {
                            self.park_halted(
                                "mailbox closed",
                                eyre::eyre!("executor mailbox closed while SafetyHalt engaged"),
                            )
                            .await;
                        }
                        // Counted BELOW the park: `park_halted` never returns, so a halted
                        // executor is still alive holding marshal acks — reporting an exit
                        // for it would destroy the operator's only "torn down" vs "halted
                        // but observable" discriminator.
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

                // Seed-record notify arm (replaces the `SpecNotarized` Poke): a
                // HELD tip's own round seed just landed in the `SeedStore` — the
                // record-vs-delivery race the on-delivery eager derive missed.
                // Gated on a tip being HELD and no predecessor parked / jump in
                // flight (the same suppression `try_eager_finalized_derive`
                // enforces internally — the guard just avoids a redundant wake).
                // The subscription is taken before this actor's first seed read,
                // so a record that lands between the miss lookup and this await is
                // buffered rather than lost; a spurious wake (an event for an
                // unrelated round) is a harmless idempotent re-check (a miss
                // re-holds). Fires the FINALIZED-tier derive, so a
                // SafetyHalt-class error PROPAGATES.
                event = beacon_events.recv(), if !beacon_events_closed
                    && self.awaiting_seed.is_some()
                    && self.deferred.is_none()
                    && self.jump_done.is_none() => {
                    match classify_seed_wake(&event) {
                        SeedWake::Ignore => continue,
                        SeedWake::Disarm => {
                            // The beacon is gone. Disarm rather than re-poll: one
                            // line, then this arm never runs again for the life of
                            // the actor.
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
                    if let Err(fault) = self.repoke_deferred().await {
                        if self.dispatch_fault("deferred re-poke", fault).await
                            == Disposition::Shutdown
                        {
                            break;
                        }
                    }
                    // Observation only (see `detect_stalled_seed_hold`): reads the
                    // clock on this existing tick and cannot complete, abort or
                    // re-key the hold.
                    self.detect_stalled_seed_hold();
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
        // never breaks — `park_halted` parks forever instead; `on_fatal` only
        // logs, and the router then returns `Shutdown`).
        if let Some(handle) = self.jump_handle.take() {
            handle.abort();
        }
    }

    /// The ONE disposition router (family 5). Every fallible executor boundary
    /// returns a [`Fault`]; this is the only place a [`FaultClass`] becomes an
    /// action, and therefore the only place `SafetyHalt::engage` is called.
    ///
    /// That relocation IS the fix, not a tidy-up. The arming sites used to
    /// engage the latch themselves and then return an untyped `eyre::Report`,
    /// trusting every frame above them to propagate it — so a `ForkSafety`
    /// verdict raised on the speculative path engaged the latch and was then
    /// reduced to `warn!("speculative execution skipped")`, leaving a node that
    /// had latched "I refuse this chain" still driving reth forward. Engaging
    /// only here means a fork-safety verdict cannot be latched without also
    /// being routed.
    ///
    /// Dispositions, straight off [`FaultClass`]:
    /// - `ForkSafety` → engage + [`Self::park_halted`] (DIVERGES);
    /// - `Corruption` → loud actor death, latch untouched → `Shutdown` →
    ///   the run loop `break`s and the supervisor aborts-all;
    /// - the transient classes + `Defer` → degrade-visible / counted, and the
    ///   loop CONTINUES (speculation stays best-effort).
    ///
    /// The is-engaged check runs BEFORE the class match so a fault arriving after
    /// the latch is already set parks whatever its class: the latch means "stop
    /// writing to the EL", and the executor is the writer. There are exactly two
    /// production engage sites — this router, and the datadir marker restored at
    /// startup (`sync_metrics::SafetyHalt::restore_marker`). The marker case is
    /// why this check is not sufficient on its own: no fault need ever reach the
    /// router, so the run loop carries its own pre-class gate.
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

    /// Log a genuine crash. Reached ONLY from the router's `Corruption` arm,
    /// which has already ruled out an engaged latch — the caller `break`s the
    /// loop and the OuterEngine supervisor answers with abort-all.
    async fn on_fatal(&mut self, stage: &str, error: eyre::Report) {
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
        // The seed-held block (same treatment as `deferred.ack`): never acked
        // (would durably skip an underived height), never dropped (a Canceled
        // ack kills the marshal).
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
                // retained acks alive and pend until the task is aborted — the
                // marshal may still be draining.
                None => futures::future::pending::<()>().await,
            }
        }
    }

    /// The finalized-delivery entry: a block arriving from EITHER source (the
    /// `Update::Block` drain or the startup backfill walk) is stashed in the
    /// seed-hold slot and immediately resolved. The stash happens BEFORE the
    /// derive so a fatal derive under an engaged SafetyHalt reaches
    /// `park_halted` with this ack retainable rather than dropped in this frame
    /// (a dropped `Exact` is Canceled, which the marshal treats as fatal).
    ///
    /// Both feeders are gated on the slot being empty (`awaiting_seed.is_none()`
    /// in the drain guard and in the startup-drain feed), so the `Some(..)`
    /// assignment below can never overwrite a live ack.
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
    /// σ comes from [`Self::seed_at_own_round`] — the block's OWN agreed round,
    /// predicate first — and nothing else. Three dispositions:
    ///
    /// - `Present`: derive now, hold consumed.
    /// - `Inactive`: the agreed derivation at a beacon-inactive epoch IS `None`,
    ///   so derive now with `None`. This is not a miss and must never hold: a
    ///   pre-bootstrap link has no σ to wait for.
    /// - `Missing`: beacon-active with no σ yet — RESTORE the hold. The only
    ///   exit is the seed-record notify; there is no timer, and the
    ///   `order.digest()` fallback is not an option (it would derive a different
    ///   `prev_randao` than the network — a silent fork).
    ///
    /// Suppressed while a predecessor is PARKED (`deferred`) or a re-jump is in
    /// flight (`jump_done`): those own the strict-order / single-EL-writer
    /// invariant, so the block stays in the slot (mirrors `spec_execute`'s
    /// guard). Both feeders are gated on the same two, so the suppression cannot
    /// strand a block behind a delivery it will never see.
    ///
    /// `trigger` distinguishes the on-delivery attempt from the event-driven
    /// re-attempt fired by [`Self::run`]'s seed-notify arm when σ was just
    /// recorded (the record-vs-delivery race self-heal). A `Notified` HIT is
    /// counted `outcome="recovered"` (the race fired and was closed without a
    /// further finalized delivery); a `Notified` MISS is a silent no-op, so the
    /// miss counter is NOT inflated on every notify while held.
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
                // `since`/`reported` ride back UNCHANGED: this is the same hold
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

    /// PARK a parked-outcome in the deferred slot (a `Done` outcome is a no-op).
    /// `fresh` is `true` for a block first derived off the pipeline — which sets
    /// the observability gauge and, for guard #2, hints the missing `h + K` body
    /// and `warn!`s once — and `false` for a re-poke re-stash (the block is
    /// already parked; do not re-hint/re-warn). There is NO deadline: parking is
    /// the terminal behaviour, re-poked event-driven, never a shutdown (§8.11).
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
                // No `h + K` hint: the archive is not missing anything — the EL
                // simply has not canonicalized what we already imported. The
                // `warn!` was emitted at the park site, which holds the parent
                // height this is waiting on.
                self.deferred = Some(*d);
            }
            DeriveOutcome::NeedPrefixSeed(d) => {
                if fresh {
                    self.deferred_height.set(d.order.height as i64);
                }
                // No `h + K` hint either: the body is in the archive, its σ is
                // not, and σ is not askable by round any more (the by-round pull
                // retired with `TAG_SEED_RETIRED`). It arrives on its own from the
                // cert inlet, and the re-poke re-runs the walk's own lookup.
                self.deferred = Some(*d);
            }
        }
    }

    /// Re-attempt the parked derive on a marshal delivery event or the FCU
    /// heartbeat — a plain "is `h + K`'s body here yet" retry (σ is retained in
    /// [`Deferred::seed`]; ZERO lookups). Event-driven, NEVER a
    /// shutdown: a still-missing body re-stays parked. Gated on
    /// `jump_done.is_none()` so a parked block is not re-derived mid-jump (a
    /// landed re-jump disposes it via `reseed_forward`). A genuine derive `Err`
    /// (FCU/execution fault) IS fatal — propagated to the caller (the only
    /// surviving shutdown, matching the normal derive arms).
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

    /// One frontier-probe tick (see [`ReJump::probe`]). Returns `true` iff this
    /// probe was PRODUCTIVE (hinted a new frontier — the fast-cadence trigger).
    ///
    /// Outside a fast burst, a tip that advanced since the previous tick means the
    /// marshal is learning finalizations from an independent live source
    /// (in-committee consensus or an inlet) — snapshot and return without network
    /// traffic. DURING a burst that advance is the probe's own delivery, so the
    /// probe keeps firing.
    ///
    /// A probe does TWO things (§5.2 "Триггер и лестница"), and the frozen tip is
    /// what earns both: the LADDER STEP `Finalized{last(T+1)}` addressed at
    /// `committee[T+1]`, where `T` is the epoch this node last handed to `track`;
    /// and the untargeted `Latest`, whose answered height — when it stands above
    /// the marshal tip — becomes one `hint_finalization(frontier)`.
    ///
    /// They are INDEPENDENT now, which is the §5.2 shape and not the one this
    /// file had: the step used to wait on the `Latest` arm to witness that the
    /// network had produced `last(T+1)` (`servable`), and that predicate went with
    /// the unauthenticated trigger input it propped up (review A2-01/A2-04 — they
    /// held each other up and had to go together). The step is now put on every
    /// frozen tick where this node can NAME it and the marshal would not discard
    /// it, and nothing on this path reads a height it did not authenticate.
    ///
    /// Neither answer comes back through here — both go through
    /// [`crate::plane_upstream::FrontierHandler::deliver`] and the marshal's own
    /// `verify_delivered`, so a served step shows up as the tip MOVING on the next
    /// tick. What does come back is whether anyone served it, and an unserved step
    /// is counted rather than reacted to: there is no state machine here, the
    /// ladder IS the repetition of this tick.
    async fn probe_frontier(&mut self) -> bool {
        let Some(probe) = self.re_jump.as_ref().and_then(|rj| rj.probe.clone()) else {
            return false;
        };
        let advanced = self.last_tip_height > self.probe_prev_tip;
        self.probe_prev_tip = self.last_tip_height;
        if advanced && self.probe_fast_left == 0 {
            // The tip MOVED, so whatever step was standing was served — by the
            // step's own answer or by an independent live source, and this metric
            // cannot tell the two apart anyway. Clearing the bit here is what
            // stops `dpos_frontier_step_unserved_total` from counting a SUCCESS:
            // tick N puts a step (pending), tick N+1 sees the tip grow and returns
            // here, tick N+2 finds the tip frozen again and would charge the
            // earlier, already-answered step to whoever did not serve this one.
            self.probe_step_pending = false;
            return false;
        }
        // `T` at THIS tick, never a remembered one: a landing moves it, and the
        // step that follows a landing is the next rung of the ladder.
        let tracked = self
            .re_jump
            .as_ref()
            .and_then(|rj| rj.tracked_epoch.clone())
            .and_then(|f| f());
        let outcome = probe(tracked).await;
        // THE LADDER STEP. Put on the MARSHAL's resolver, not on this probe's:
        // `HintFinalized{height, targets}` is a targeted by-height fetch whose
        // answer is decoded, BLS-verified and stored by the marshal itself
        // (`marshal/core/actor.rs:632-646`, then the `verify_delivered` path), so
        // the step lands in the one place that moves the tip and there is no
        // second writer. The `targets` travel with it: on a plane validator the
        // marshal's own resolver addresses them, and on a WS-upstream validator
        // `outer.rs`'s dispatcher routes a TARGETED `Finalized` to the plane for
        // exactly that reason (the single-upstream resolver drops target lists).
        //
        // Repeated hints for the same height dedup in the resolver, so repeating
        // the step every tick is the ladder and not a poll: there is no automaton,
        // only this tick happening again.
        //
        // ONE CONDITION, and it is the marshal's own. A step at or below the
        // marshal FLOOR is a no-op there (`HintFinalized` skipped when
        // `height <= last_processed_height`, `marshal/core/actor.rs:633-635`), so
        // putting it is a fetch nobody acts on. `self.marshal_floor` is this
        // executor's mirror of exactly that value — seeded from the same
        // `initial_marshal_floor` `outer.rs` sends in its buffered `SetFloor` and
        // moved by every `reseed_forward`.
        //
        // The FLOOR and not the tip, and the difference is a whole defect class
        // (review A2-10): a node that jumped holds nothing between its floor and
        // its tip, and a step landing in that HOLE is precisely the one the marshal
        // would accept and act on. Gating on the tip suppressed exactly those.
        //
        // THE SECOND CONDITION IS GONE (review A2-01). `servable` asked the
        // `Latest` answer of this same tick to witness that the network had
        // PRODUCED `last(T+1)`, because nothing local tells "a node genuinely
        // behind" from "a node at the live tip whose marshal froze for a second"
        // apart. That witness was the last unauthenticated input on this path, and
        // it was only tolerable while the jump trigger read the same height; with
        // the trigger on the marshal tip alone it has no reason to exist. What it
        // cost to keep: a step inside a jumper's own hole was suppressed whenever
        // its `Latest` source was silent. What it costs to drop: on a node already
        // at the live tip the step names a height nobody has yet, and an
        // unanswered targeted fetch is exactly §5.4's "догон вместо прыжка" — the
        // `unserved` counter below.
        //
        // THE PRICE, in requests and not in ticks (review B1-07): this arm runs
        // every frozen tick (1 s), but the WIRE cost is set by the resolver, not by
        // this cadence. A repeated hint for a key already pending is a no-op
        // (`resolver/src/p2p/engine.rs:229-252`, `is_new`), and an unanswerable key
        // is re-sent once per `timeout` + `fetch_retry_timeout` — 5 s + 500 ms, so
        // ≈ 0.18 requests/s per node, one key, deduplicated. It also does not
        // monopolise the fetcher: `pending` is a `PrioritySet` ordered by next-try
        // time (`fetcher.rs:119`, `utils/src/priority_set.rs:145-149`), so a fresh
        // by-height repair key sorts AHEAD of this key's retry.
        match outcome.step {
            Some((height, _)) if height <= Height::new(self.marshal_floor) => {
                metrics::counter!(FRONTIER_STEP_SKIPPED, "reason" => "at_or_below_the_floor")
                    .increment(1);
                self.probe_step_pending = false;
            }
            Some((height, targets)) => {
                // Tick taken, tip frozen (checked above), a step was standing from
                // the previous tick and nothing moved: nobody served it.
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
    /// NOT an engine error — it arrives as `Ok(..)` (never folded into `Err`) and
    /// is returned here UNTOUCHED for the caller's verdict split. This helper
    /// never converts a verdict.
    ///
    /// Only the TRANSIENT half of the `Err` is looped. An [`EngineError`] whose
    /// class is not `TransientExternal` — reth PROCESSED the update and rejected
    /// the forkchoice state we named, i.e. it cannot resolve our own
    /// finalized/safe hash — is propagated as its own class instead. Retrying
    /// that re-sends the same unresolvable hashes forever: the loop had no exit
    /// because the importer flattened every `Err` into one transport class.
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

    /// Fire-and-forget heartbeat FCU: the next tick IS the retry, so a transport
    /// failure is counted + degraded and swallowed here. A non-transport class
    /// (reth rejected the forkchoice STATE) is NOT swallowed — the heartbeat
    /// would otherwise re-send the same unresolvable anchor every tick forever.
    #[instrument(skip_all)]
    async fn send_forkchoice_update_heartbeat(&mut self) -> Result<(), Fault> {
        if self.jump_done.is_some() {
            // A re-jump's `sync_to` is the EL driver during backfill; an interleaved
            // heartbeat FCU returns reth `SYNCING`
            // (engine/tree/src/tree/mod.rs:1173-1177: `if !backfill_sync_state.is_idle()
            // { return ...syncing() }`), producing a spurious reth-side `Stalled` that —
            // now that `Stalled` rotates — would churn rotation. The re-jump is the
            // single EL writer while in flight.
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
        // GAP-2 CLOSURE (family 5): a heartbeat FCU transport failure is
        // `FaultClass::TransientExternal(EngineRetry)` — fire-and-forget (the
        // next heartbeat tick is the retry, no loop), but now COUNTED +
        // degrade-visible like the finalize FCU, instead of a bare `warn!`
        // invisible to the taxonomy. A successful tick clears the reason.
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
                    // it awaits a pruned `h + K` (permanent deferred). Ack it Ok — the
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
                // Speculation stays best-effort — but the CLASS decides that, not
                // this call site. `spec_execute` classifies its own failures
                // `Defer`/`TransientExternal`, which the router logs and
                // continues on; a `ForkSafety` it cannot classify away reaches
                // the router too, instead of being reduced to a `warn!`.
                self.spec_execute(cause.clone(), digest, seed).await?;
                // A live spec advance may unblock a parked out-of-order
                // notarization (e.g. h+1 parked, then h arrives and advances
                // spec_head) — drain it now.
                self.try_drain_parked(&cause).await?;
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

    /// Steady-state self-healing re-jump (see [`ReJump`]). The marshal TIP (the
    /// `Update::Tip` height, §5.2's one frontier) has run more than
    /// [`ReJump::threshold`] finalized blocks ahead of the highest derived
    /// ordering height (`ordering_finalized`) — the upstream serving window is
    /// that wide, so beyond it the marshal's backfill resolver finds nothing and
    /// the floor freezes forever.
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
    /// `Some` — never spawn a second), a gap ≤ [`ReJump::threshold`], a marshal
    /// archive with no pair at the tip, or a mid-flight startup drain all
    /// early-return without spawning. A parked (`deferred`)
    /// block does NOT gate this off: once the gap runs past the threshold the
    /// situation is no longer "wait for this block's cert" but a deep catch-up
    /// (the durably-stuck-fetch case, §4.3) — the re-jump backfills the
    /// committee-BLS-authenticated `[.. landing]` (the parked height is a finalized
    /// ancestor of the landing), and `reseed_forward` disposes the parked block by
    /// `ack.acknowledge()` (Ok, never Canceled). In Case (A) the gap stays small,
    /// the gap test early-returns, and the park proceeds untouched.
    async fn maybe_re_jump(&mut self, height: Height) -> Result<(), Fault> {
        let Some(re_jump) = self.re_jump.clone() else {
            return Ok(());
        };
        // A jump is already in flight — don't spawn a second.
        if self.jump_done.is_some() {
            return Ok(());
        }
        // §5.2: ONE frontier, and it is the marshal tip. `height` is the
        // `Update::Tip` the marshal emits from `store_finalization`, i.e. the
        // highest finalization this node VERIFIED and stored — there is no second
        // signal, and no unauthenticated one. The `max(tip, upstream_frontier)`
        // this line used to take existed because the tip freezes under the
        // "committee[E] not committed" defer deadlock; it no longer can freeze
        // silently, because the frozen-tip probe puts the ladder step
        // `Finalized{last(T+1)}` on every frozen tick and a served step moves the
        // tip through `verify_delivered` (`probe_frontier`).
        if height.get().saturating_sub(self.ordering_finalized)
            <= re_jump.threshold
            // Symmetric closure to the startup-drain's jump-gate (bugs 6/7): a jump
            // and the startup backfill drain must not both drive reth's EL — never
            // start a jump while the drain is mid-flight.
            || self.pending_backfill.is_some()
            || !self.finalized_heights_to_backfill.is_empty()
        {
            return Ok(());
        }
        // THE TARGET, read from this node's OWN marshal archive at the tip it is
        // triggering on. `Update::Tip` fires from `store_finalization` only after
        // the pair is written (CW `marshal/core/actor.rs:1404-1463`), so the two
        // reads below hit an entry that already passed `verify_delivered` — the
        // jump no longer asks anyone what to aim at.
        //
        // A miss is not a fault, and the case it covers is NOT "the floor moved"
        // (review B1-08): a floor raise deletes nothing, because both finalized
        // archives are `immutable::Archive`, whose `prune` is a no-op (CW
        // `marshal/store.rs:223-226`, `:261-264`) — the same fact §5.2 leans on for
        // retention. The reachable miss is the SEEDED tip: `last_tip_height` starts
        // at `cfg.last_consensus_finalized_height` (`:1260`), and on a datadir whose
        // marshal archive is empty — a fresh one after the pre-engine cold-start
        // jump — the very first heartbeat re-poke names a height nothing was ever
        // stored at. Skip and let the next `Update::Tip` / heartbeat re-arm.
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
        // Spawn the whole jump (sync_to wait + landing check + L1) as a READ-ONLY
        // waiter and react to its completion on the `jump_done` arm. `re_jump` is
        // already owned (cloned out of `self.re_jump` above) and unused after this
        // move — no second clone needed.
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
        // The anchor just moved by a whole jump; the committee module re-reads
        // its own height and republishes its readable ceiling.
        (self.anchor_advanced)();
        // STALE-SPEC FIX: the speculative tip / map are stale across a deep jump
        // (their heights are far below the landing). Raise `spec_head` to the
        // landing and drop spec entries at/below it so the next notarization
        // re-speculates forward from the landing.
        self.spec_head = self.spec_head.max(landing_h);
        self.spec_executed = self.spec_executed.split_off(&(landing_h + 1));
        // Parked speculative notarizations below the landing are stale across a
        // deep jump (same rationale as `spec_executed` above) — drop them.
        self.parked_spec = self.parked_spec.split_off(&(landing_h + 1));
        // STARTUP-BACKFILL FAST-FORWARD: the
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
        // Same disposition for the seed-held block: the landing is
        // far above it, so the held height is pruned by the floor move —
        // `acknowledge()` (Ok), never a drop (a dropped `Exact` is a Canceled
        // ack, fatal to the marshal). The one new object the jump path knows
        // about.
        if let Some(held) = self.awaiting_seed.take() {
            held.ack.acknowledge();
        }
        // STALE FINALIZATION BACKLOG PRUNE:
        // `Update::Block` deliveries queue UNCONDITIONALLY while the drain arm
        // is gated off during a park + in-flight jump — up to MAX_PENDING_ACKS
        // stale below-landing entries. Un-pruned, the stale backlog drains
        // post-jump, re-populates `awaiting_seed` with a jumped-over height whose
        // parent the jump pruned, so the gap-walk's marshal fetch returns None →
        // the missing-artifact fatal misclassifies a jump-MANUFACTURED skip-gap
        // as archive corruption. The fatal itself stays valid for the genuine
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
        self.try_drain_parked(&Span::current()).await
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
        seed: Option<crate::beacon::Seed>,
    ) -> Result<(), Fault> {
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
        let parent_height = height.checked_sub(1).ok_or_else(|| {
            Fault::defer(
                DeferReason::SpecDeriveFailed,
                eyre::eyre!("speculative height 0"),
            )
        })?;
        // Parent must be locally present; a transient miss (reth visibility
        // lag) PARKS the notarization (height == spec_head + 1) so the next
        // `spec_head` advance retries it — pre-fix this dropped the notarization
        // and the finalized path was the only retry.
        let Some(parent_hash) = self.executed.spec_executed_hash(parent_height) else {
            self.parked_spec.insert(height, ParkedSpec { digest, seed });
            return Ok(());
        };

        // §4.1 (P2): re-canonicalise the speculative round to the block's OWN
        // `proposal_view` — the same pure-agreed-data round the finalized derive
        // resolves at (rule PIN). A first-seen notarization at a SPIN round
        // (mid-spin rejoin; body not buffered at V0) must not seal the block
        // with `seed(V0+k)` — that guarantees a re-derive + head reorg at the
        // boundary. On a round mismatch take the canonical round's bytes from
        // `SeedStore` (a threshold seed is unique per round, so the store is a
        // byte source for an already-pinned round); on a miss SKIP speculating —
        // never speculate with a known-wrong seed (the finalized path resolves
        // this height's own round regardless).
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

        // The round of the (re-canonicalised) seed this speculation is derived
        // with (`None` = no-beacon). Captured BEFORE `seed` is moved into the
        // deriver; reconciled against the WITNESS round in `try_derive`.
        let seed_round = seed.as_ref().map(|s| s.target_round);
        // EXEC-SATURATION observability: per-block derive+import wall time on the
        // TIP paths only (this speculative path + try_derive's finalized re-derive).
        // The catch-up paths (gap-walk, re-apply retry) run back-to-back by design
        // and would read as false saturation of the 1 blk/s interval.
        let el_apply_started = std::time::Instant::now();
        // Speculation is BEST-EFFORT and stays so: a derive failure here is
        // `Defer`, never `Corruption`, because the finalized path derives this
        // height from its own round regardless. Classified explicitly —
        // `Fault`'s blanket `From<eyre::Report>` is `Corruption`, so leaning on
        // `?` here would turn a transient derive failure into actor death.
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
        // The engine boundary's own class decides: a transport blip degrades and
        // is retried by the next notarization, while a rejected forkchoice STATE
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
        // SPECULATIVE-vs-FINALIZED VERDICT SPLIT. The finalized FCU treats this
        // same `Ok(Invalid)` as #15 `SafetyHalt(ElInvalid)`; here it is a plain
        // skip, and the asymmetry is deliberate.
        //
        // An `Ok(Invalid)` on an FCU is reth's `check_invalid_ancestor`: the head
        // we named descends from a header sitting in reth's `invalid_headers`
        // cache, which reth populated from blocks IT downloaded over devp2p and
        // rejected. That is evidence about the network, not about this node's
        // disk — but this head is NOT canonical. It is notarized-but-unfinalized,
        // and consensus may still nullify the view and finalize a sibling, in
        // which case nothing this verdict indicted was ever committed. Halting
        // here would convert a branch the protocol is allowed to discard into a
        // permanent, operator-cleared halt.
        //
        // Nothing is lost by waiting: every finalized derive issues an FCU whose
        // head is at or above the finalized tip, so an invalid ancestor at or
        // below that tip re-renders the SAME verdict on the finalized path within
        // one block — where it engages the latch with committed evidence. The
        // only verdict this arm swallows is one whose invalid ancestor lies
        // strictly inside the speculative segment, i.e. exactly the blocks
        // consensus has not committed.
        //
        // The IMPORT verdict is judged differently one frame up
        // (`submit_finalized_payload` halts on `Invalid` from either path)
        // because it is a statement about OUR derivation matching reth's
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
    /// A `spec_execute` failure KEEPS the entry for the next advance and ends
    /// the drain, then hands the [`Fault`] to the run loop's router — which
    /// continues on the classes `spec_execute` actually produces (`Defer` /
    /// `TransientExternal`), so speculation stays best-effort, and parks on a
    /// `ForkSafety` one, which the old `warn!`-and-continue silently discarded.
    /// NOT recursive: `spec_execute` does not call back into this drain; the loop
    /// lives here.
    async fn try_drain_parked(&mut self, cause: &Span) -> Result<(), Fault> {
        // Prune stale entries (≤ spec_head): finalized OR already speculated.
        self.parked_spec = self.parked_spec.split_off(&(self.spec_head + 1));
        let mut resumed = 0u32;
        while let Some(parked) = self.parked_spec.get(&(self.spec_head + 1)).cloned() {
            let next = self.spec_head + 1;
            let before = self.spec_head;
            if let Err(fault) = self
                .spec_execute(cause.clone(), parked.digest, parked.seed)
                .await
            {
                // The entry stays parked for the next advance whatever the class;
                // the router decides whether the executor also stops.
                debug!(
                    height = next,
                    "parked speculative drain failed; entry retained for the next advance"
                );
                return Err(fault);
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
        Ok(())
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

    /// DETECTOR, never a deadline: report a block that has sat in the seed hold
    /// longer than [`SEED_HOLD_STALL_THRESHOLD`] and change NOTHING about it.
    ///
    /// The hold is bounded only if every `impl Beacon` a production node class can
    /// be given actually supplies σ — a claim about the two implementations behind
    /// the boundary that this crate asserts and that the executor cannot verify
    /// from the inside. The follower's own seed recording was once an
    /// empty no-op that satisfied its signature, compiled, and was invisible to
    /// every name-based search; this counter is what makes the NEXT such
    /// counter-example surface in the smoke harness instead of in a review
    /// months later.
    ///
    /// Sibling of the `dpos_executor_stray_seed_at_inactive_round_total` counter
    /// below — both are detectors for beliefs this design asserts, both are
    /// expected to read 0, and neither changes a derive.
    ///
    /// No timer is added: this rides the EXISTING FCU heartbeat tick, reads the
    /// clock and returns. A timeout could neither derive (the `order.digest()`
    /// fallback forks) nor skip (a permanent hole), so it would convert a silent
    /// stall into a loud one without restoring liveness.
    fn detect_stalled_seed_hold(&mut self) {
        let now = self.context.current();
        let Some(held) = self.awaiting_seed.as_mut() else {
            return;
        };
        if held.reported {
            return;
        }
        // A backwards clock is not evidence of a stall — say nothing.
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

    /// σ for `height`'s OWN agreed round — the ONLY seed source of the finalized
    /// derive.
    ///
    /// The round is `Round(epocher.containing(h).epoch(), h.proposal_view)`, a
    /// pure function of AGREED data (rule SA: `proposal_view` rides in the
    /// committee-signed digest, and the epoch comes from the same block→epoch
    /// map every node holds). A threshold σ is UNIQUE per round, so every honest
    /// node resolves the identical value and a wrong round can only MISS.
    ///
    /// PREDICATE FIRST, store second. `mandatory_at(epoch(h))` — the
    /// network-agreed "is the beacon active here", independent of anything local
    /// — decides BEFORE the store is read. Store-first ordering would let a σ
    /// filed at a round the agreed map calls beacon-INACTIVE be USED, which is
    /// how the journal (replayed without re-verification by design) or a crafted
    /// record could steer one node's `prev_randao` away from the network's.
    /// Inverting the order closes that: a stray σ at an inactive round is
    /// IGNORED and counted, never obeyed and never fatal — ignoring derives
    /// exactly what the rest of the network derives, so it is fork-safe and
    /// self-healing, where halting would turn one bad record into an outage.
    ///
    /// A height whose epoch the map cannot name (below the epocher origin) is
    /// INACTIVE, never unwrapped: the beacon cannot have been mandatory in an
    /// epoch that does not exist.
    fn seed_at_own_round(&self, height: u64, proposal_view: u64) -> OwnRoundSeed {
        use commonware_consensus::types::{Epocher as _, Round, View};
        let Some(info) = self.epocher.containing(Height::new(height)) else {
            return OwnRoundSeed::Inactive;
        };
        let round = Round::new(info.epoch(), View::new(proposal_view));
        if !self.randomness.mandatory_at(round.epoch().get()) {
            // Looked up ONLY to count it: the value is never handed on.
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

    /// Derive + import + FCU + ack a finalized block from `seed` — σ of the
    /// block's OWN round, already resolved by [`Self::seed_at_own_round`]
    /// (`None` = a beacon-inactive, seed-independent link). Guard #2 (the
    /// `h + K` look-ahead convergence check) runs whenever the node is ≥ K behind; if the attested body at
    /// `h + K` is not backfilled yet this returns `NeedAttestation` — WITHOUT
    /// mutating any finalized state or acking — so the caller PARKS it and
    /// re-pokes event-driven (the delivery stream + the FCU heartbeat).
    ///
    /// FAULT-CLASS INVARIANT: while this function holds the block's `Exact` in
    /// [`Self::inflight_ack`], the only [`FaultClass`]es it may return are the
    /// two the router does NOT continue on — `ForkSafety` (parks, and
    /// `park_halted` retains the ack) and `Corruption` (the run loop breaks). A
    /// class the router continues on would leave that ack orphaned in the slot,
    /// and the NEXT derive's `inflight_ack = Some(..)` would drop it — a dropped
    /// `Exact` is Canceled, which the marshal treats as fatal. The transient
    /// classes appear only AFTER `take_inflight_ack().acknowledge()`, where the
    /// slot is empty (the tail `try_drain_parked`).
    #[instrument(skip_all, parent = &cause, fields(height = order.height), err(Debug))]
    async fn try_derive(
        &mut self,
        cause: Span,
        order: OrderBlock,
        ack: Exact,
        seed: Option<crate::beacon::Seed>,
    ) -> Result<DeriveOutcome, Fault> {
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

        // Its ROUND completes the speculation-reuse invariant just below, and
        // its VALUE is reused verbatim by the re-derive branch and the re-apply
        // loop.
        let finalization_seed = seed;
        let finalization_round = finalization_seed.as_ref().map(|s| s.target_round);

        // Reconcile against speculation: keep the speculatively-executed block
        // ONLY when it is the SAME ordering block AND was speculated with the
        // SAME seed round the finalized derive resolved — then reth is already
        // canonical here,
        // so skip the re-derive and, crucially, do NOT roll the head back (the
        // speculative lead at `height+1..` must survive). After §4.1 both rounds
        // are `Round::new(Ep, block.proposal_view)`, so a digest match with a
        // DIFFERENT round is an ANOMALY (two paths disagreeing about the
        // canonical round) — counted, then re-derived from the store's σ (the
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
                    finalized_round = ?finalization_round,
                    "speculation round disagrees with the finalized round (both should be \
                     Round(Ep, proposal_view)) — re-deriving from the agreed round"
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
        // the re-derive path (which walks `derive_finalized_with_gap_fill`).
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
            // Already derived via spec_execute with a seed of the SAME round the
            // finalized derive resolved — reth is canonical here, no re-derive.
            self.executed
                .spec_executed_hash(height)
                .expect("checked is_some above")
        } else {
            // ONE range, ONE Ok/Err: the missing prefix (the marshal can hold
            // finalized artifacts the EL hasn't derived yet — restart with an
            // unflushed reth tail, repair landing ahead of dispatch) and the
            // delivered height derive through the same call, so there is no
            // second site that must separately remember to catch an invisible
            // parent. Two structurally identical sites with only one of them
            // protected is how this defect class survived.
            let gap = self.executed.spec_executed_hash(parent_height).is_none();
            // Cloned ONLY when a gap exists (the rare path) — the park needs an
            // owned `order` + `seed`, and the derive consumes both. `Seed` is
            // Clone-not-Copy, which is why the retry path above already clones it
            // for the same reason.
            let parked = gap.then(|| (order.clone(), finalization_seed.clone()));
            match self
                .derive_finalized_with_gap_fill(order, finalization_seed)
                .await
            {
                Ok(hash) => hash,
                Err(error) if is_parent_not_visible(error.cause()) => {
                    // No-gap path: `block_hash(h)` resolving does NOT imply the
                    // header read will (reth canonicalizes eagerly on the
                    // engine-tree thread, so a block is by-number resolvable
                    // milliseconds before provider reads see its header — see
                    // `ParentHeaderMissing`). That transient has no park payload
                    // and stays the recoverable `Err` it has always been; turning
                    // it into a panic here would be a regression.
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
                    // A prefix element on a beacon-active round has no σ yet. The
                    // walk cannot hold (it owns neither `cause` nor the ack), so it
                    // reports the typed leaf and the park happens HERE. `parked` is
                    // `Some` in every reachable case — a prefix exists only when the
                    // walk's backward probe fails at `target - 1`, which IS the
                    // `gap` predicate that guarded the clone — but the fall-through
                    // stays rather than an `expect`, matching the arm above.
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

        // DERIVE-SEED TELEMETRY (fork-root byte-confirm): the finalized derive's
        // provenance for `height`. `fin_proposal_round` is the WITNESS round
        // (`Round(Ep, proposal_view)`, pinned by rule PIN at the child's vote).
        // `path` records whether the speculative block was REUSED (and the round
        // it was speculated with, `spec_seed_round`) or RE-derived. Compared
        // cross-node at a diverged height this is the (a)-vs-(b) discriminator:
        // (a) SAME `fin_proposal_round` on both sides but a different resulting
        // hash / prev_randao (derive.rs line) ⇒ seed decoupled from the agreed
        // round; (b) DIFFERENT `fin_proposal_round` ⇒ the diverged node resolved
        // a genuinely different round for this height.
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
        // ALREADY finalized, so a wrong derive WOULD be caught here, before the
        // ack (and the `split_off` prune below) — but only when `spec_executed_hash(h)`
        // is `Some`. On the catch-up path it is `None` until `h`'s own FCU
        // (reth canonicalises on FCU, not on insert), `result_matches` is `None`,
        // and this guard stays silent; the verdict then comes from the `h − K`
        // backward cross-check further down at `h + K` (R-006 scenario 1, pinned
        // by `testbed::tests::guard_two_on_the_catch_up_path_reads_a_pre_fcu_height`).
        // The steady state is covered by that backward check too; this costs NOTHING here (the
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
                // The `h + K` body is not backfilled yet (tip >= h+K, but the block
                // hasn't landed). PARK — a fall-through would reach the unconditional
                // `ack.acknowledge()` and finalize `h` with NO convergence check.
                // Returning here (BEFORE the `split_off` prune) keeps
                // `spec_executed[height]` intact, and the park CARRIES σ, so the
                // re-poke re-derives with zero lookups.
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
            return Err(Fault::fork_safety(
                SyncReason::ResultDivergence,
                eyre::eyre!(
                    "result divergence at height {height}: attested result \
                     {attested_result:?} != local executed_hash; SafetyHalt — refusing to \
                     serve a forked chain"
                ),
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
        let fcu = self.fcu_retrying_transport(new.forkchoice).await?;
        if !(fcu.is_valid() || fcu.is_syncing()) {
            // #15 SafetyHalt (Phase 3): halt (verify-only, stop driving reth,
            // stay observable) instead of exiting — recovery is the L1 proof.
            return Err(Fault::fork_safety(
                SyncReason::ElInvalid,
                eyre::eyre!(
                    "EL reported non-valid finalize FCU: {:?}; SafetyHalt",
                    fcu.payload_status
                ),
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
        // ...but ONLY where re-applying can converge, i.e. ABOVE the finalized
        // tier. The loop's only lever is re-sending `new.forkchoice`, whose head
        // `update_head` refuses to move to a block at or below `finalized_height`,
        // and reth will not reorg below its own finalized block either. At
        // `height <= finalized_height` the loop is therefore a silent 200 ms spin
        // with `finalize_apply` degraded forever. Healing it WOULD need an FCU that
        // reorgs reth away from the BLS-authenticated chain — a silent fork traded
        // for a visible stall, so this is a verdict, not a retry.
        //
        // The two arms are deliberately ASYMMETRIC. A CONFLICTING hash is settled:
        // reth will not reorg below its own finalized block, so re-reading only
        // delays the fork-safety verdict. NOTHING at the height is the transient
        // `reseed_forward` already answers with a belt — a height the devp2p
        // backfill just landed is by-NUMBER invisible for a moment — so it gets a
        // bounded re-read first, and only an EL that never serves it is corruption.
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
            // Bound once: re-reading for the message could report `EL holds X` with X equal
            // to the derived hash, i.e. a permanent, marker-persisted verdict whose own text
            // contradicts it.
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
        // Same budget, different transient: a prefix element's σ can land at any
        // moment from the cert inlet (the store is shared), so a re-walk is worth
        // trying here — unlike at the fresh-derive site, this loop already holds
        // the ack and has no park route.
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
            // The SAME protected walk the first attempt used, not a hand-rolled
            // copy of it: the old body derived straight against
            // `spec_executed_hash(parent_height)` and `continue`d when that was
            // absent, so a re-apply after a rollback that orphaned the parent
            // silently span instead of gap-filling it. The walk submits the
            // delivered element itself; its FCU stays here (the walk deliberately
            // leaves the delivered element's forkchoice to the caller).
            let reapplied = match self
                .derive_finalized_with_gap_fill(order, finalization_seed_retry.clone())
                .await
            {
                Ok(hash) => hash,
                // The only classes this walk can return while `inflight_ack` holds
                // the block's `Exact` are ForkSafety and Corruption (transport is
                // absorbed inline), so the CAUSE is the whole filter — a
                // `FaultClass::Transient*` disjunct here would be dead code that
                // reads as a retry guarantee.
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
        // result-commit fork at its SOURCE; the h−K
        // backward cross-check above stays the safety net). The cursor lives in
        // the shared executed store, not the per-epoch engine, so it survives
        // engine restarts within the process.
        self.executed.advance_finalized(height);
        // Every finalized derive moves the committee module's anchor by one
        // block. This is the steady-state wake-up: a consumer parked on
        // `NotReadable` for an epoch whose commit height has just been passed
        // learns it here and nowhere else.
        (self.anchor_advanced)();

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
        // finalized head back. The ack above already landed, so a fault here is
        // about the SPECULATIVE tail only — the router continues on the transient
        // classes it can produce and parks on a fork-safety one.
        self.try_drain_parked(&cause).await?;
        Ok(DeriveOutcome::Done)
    }

    /// Take the ack parked at [`Self::try_derive`]'s entry (see `inflight_ack`).
    fn take_inflight_ack(&mut self) -> Exact {
        self.inflight_ack
            .take()
            .expect("inflight ack set at try_derive entry")
    }

    /// Derive `[first_missing ..= delivered.height]` — the missing prefix AND the
    /// delivered block — as ONE fallible range, returning the derived hash at the
    /// delivered height. When nothing is missing the range is a single element and
    /// this is the ordinary finalized derive; `first_missing` is found by probing
    /// backward to the highest executed ancestor.
    ///
    /// One range with ONE `Ok`/`Err` exit is the point: the delivered height is
    /// structurally identical to a prefix element (it derives against a parent the
    /// walk just imported), so a caller catching an invisible parent must not have
    /// to remember a second site. Each element is acquired at the TOP of its own
    /// iteration — `delivered.take()` at the target, a marshal fetch below it —
    /// and σ comes from the same place the main path reads it: the store, at that
    /// element's own round, PREDICATE FIRST ([`Self::seed_at_own_round`]). The
    /// target's σ is the caller's `delivered_seed` and is NOT re-looked-up: the
    /// caller may have PARKED with it, a park has no deadline, and a re-lookup
    /// could miss where the parked value derives. No certs, no lookahead, no
    /// hints.
    ///
    /// A prefix miss on a beacon-ACTIVE round reports the typed
    /// [`PrefixSeedMissing`](crate::application::PrefixSeedMissing) leaf. This
    /// call holds neither `cause` nor `ack`, so it cannot hold the block itself —
    /// but its CALLER owns the park, matches on the leaf and returns
    /// [`DeriveOutcome::NeedPrefixSeed`], so a σ-less prefix element waits instead
    /// of killing the actor. The `Fault` class stays `Corruption` because that is
    /// what the fault-class invariant permits while `inflight_ack` is held; the
    /// class is never reached, the cause is.
    ///
    /// A missing BLOCK stays fatal (the pre-existing "hole below the floor cannot
    /// self-heal" class); a re-walk on a retry is idempotent — already-derived
    /// prefix heights advance `first_missing`.
    ///
    /// The landing re-check, the canonicalization FCU, the gap telemetry and the
    /// result cross-check apply to the PREFIX elements only. The delivered element
    /// is the caller's: `try_derive` re-checks its landing in the postcondition
    /// loop, sends its FCU and runs its own cross-check, so repeating them here
    /// would double every steady-state block's EL round-trips.
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
        // Held until the walk reaches its top: moving rather than cloning keeps
        // the delivered block's tx list off the steady-state hot path, and the
        // target is NEVER re-fetched from the marshal (the steady-state walk —
        // `first_missing == target` — must stay zero-marshal).
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
                        // The CLASS stays `Corruption` while `inflight_ack` holds the
                        // block's `Exact` (the fault-class invariant at `try_derive`);
                        // the caller matches on the CAUSE and parks before the router
                        // ever sees the class, exactly as it does for
                        // `ParentHeaderMissing`.
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
            // Captured before `order` is consumed: each gap block carries its OWN
            // committee-attested `result` commitment, which must be cross-checked
            // exactly like the top-level delivered block — otherwise a wrong
            // `result` on a gap-range block (the byzantine-vrf defense) would be
            // imported unchecked.
            let attested_result = order.result;
            // Derive-seed telemetry: the walked element's own seed round, captured
            // before `seed` moves into the deriver.
            let gap_seed_round = seed.as_ref().map(|s| s.target_round);
            // EXEC-SATURATION observability (see spec_execute for scope rationale);
            // recorded for the DELIVERED element only, the scope it had before that
            // derive moved inside this walk.
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
                // The DELIVERED element deliberately DISCARDS the transport flag
                // the prefix arm below checks: `try_derive` re-checks this block's
                // landing in its postcondition loop, so an `Ok(false)` here is
                // retried by the caller instead of killing the actor. The two
                // call sites are NOT symmetry-debt — see
                // `submit_finalized_payload`'s contract.
                self.submit_finalized_payload(derived).await?;
                metrics::histogram!("dpos_derive_el_apply_duration_seconds", "path" => "finalized")
                    .record(el_apply_started.elapsed().as_secs_f64());
                break;
            }
            // DERIVE-SEED TELEMETRY (fork-root byte-confirm): label the finalized
            // GAP-WALK derive so a height derived via prefix catch-up (vs top-level
            // `try_derive`) is attributable; `fin_proposal_round == gap_seed_round`
            // (the walked block's OWN round).
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
                // `Corruption` (actor death), NOT a transient class, even though
                // the CAUSE was a transport blip. `try_derive` is holding this
                // block's `Exact` in `inflight_ack` right now: a class the router
                // continues on would leave that ack orphaned, and the next
                // derive's `inflight_ack = Some(..)` would DROP it — a dropped
                // `Exact` is Canceled, which the marshal treats as fatal. The
                // walk's disposition while an ack is in flight can only be
                // "park forever" (fork-safety) or "die loudly".
                return Err(Fault::corruption(eyre::eyre!(
                    "gap-walk import at height {h} hit an engine-API transport failure \
                     (block not landed); aborting the walk — a re-entry re-walks the \
                     idempotent prefix"
                )));
            }
            // The walk hands `parent_hash` to the NEXT derive, which reads the
            // parent BY HASH — and an `InsertExecuted` import is only in reth's
            // tree-private state until an FCU canonicalizes it. Same literal-state
            // shape as `reseed_forward`: head = safe = the block just landed (BFT
            // ordering-final), finalized left on the result tier so the two-tier
            // contract holds. Built literally rather than through `update_head`,
            // which silently no-ops when `height <= finalized_height` — reachable
            // right after a re-jump, exactly when this walk runs. The response is
            // NOT inspected: VALID and "parent is visible" diverge in both
            // directions, so the next derive is the honest judge.
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
                // A transport failure is absorbed as before (the next derive is
                // the honest judge). A rejected forkchoice STATE is not: the
                // finalized hash this walk names is unresolvable in reth, which
                // every subsequent walk re-sends unchanged.
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
    async fn submit_finalized_payload(&mut self, derived: D::Derived) -> Result<bool, Fault> {
        // Single chokepoint for all three derive paths (spec / finalized / gap):
        // record this block's beacon outcome before the value is moved into the EL.
        match derived.beacon_active() {
            Some(true) => self.metrics.seed_active.inc(),
            Some(false) => self.metrics.digest_fallback.inc(),
            None => 0,
        };
        // TRANSPORT-vs-VERDICT split (family 5, type-level via `BeaconEngineLike`):
        // the verdict rides in `Ok`, transport in `Err(EngineError)`.
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
            // #15 SafetyHalt (Phase 3): under the new_payload fallback an `Invalid`
            // import means local derivation diverged from reth's re-execution —
            // halt (verify-only, stay observable) rather than exit. The latch is
            // engaged by the ROUTER, not here: this function used to engage it and
            // then rely on every caller propagating the `Err`, and the speculative
            // callers did not — leaving a latched node still driving reth.
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
        /// record into. Thread-local rather than a `static` because the helpers
        /// are free functions with no fixture in hand: one `#[test]` runs per
        /// thread, so a round one test recorded can never answer another's
        /// lookup.
        static FIXTURE_SEEDS: crate::beacon::testing::SeedStore =
            crate::beacon::testing::SeedStore::new();
    }

    /// Record the canonical σ for a block proposed at `view` of epoch 0 — the
    /// epoch the fixture's default single huge epocher puts every test height in.
    ///
    /// σ is a pure function of the round, so every writer of one round writes the
    /// same bytes; the store therefore doubles as the memo that keeps the
    /// threshold recovery to once per round per thread. A test that overrides the
    /// epocher builds its own store (`Fixture::with_seed_store`) — this one
    /// cannot name its epochs.
    fn record_fixture_seed(view: u64) {
        let round = active_round(view);
        FIXTURE_SEEDS.with(|seeds| {
            if seeds.lookup(round).is_none() {
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

    /// A REAL 2f+1 finalization certificate over `block`'s digest, under a
    /// throwaway four-member committee built once per process.
    ///
    /// It exists so [`FakeMarshal`] can answer `BlockFetcher::pair_at` with a
    /// `Finalization` VALUE — the jump target's type demands one. Nothing in this
    /// module verifies it: the re-jump callback is scripted, and the production
    /// gates that would check it run over the production archive on the stand.
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

    /// The next linked block after `parent` — a plain link now that nothing
    /// rides on the child. A test that needs σ for a specific round files it
    /// with [`record_fixture_seed`] instead; the child no longer carries one.
    fn child_of(parent: &OrderBlock) -> OrderBlock {
        sample_order(parent.digest(), parent.height + 1, B256::ZERO)
    }

    /// The first beacon-ACTIVE epoch, and the one every σ-recording helper in
    /// this module keys on.
    fn active_epoch() -> commonware_consensus::types::Epoch {
        commonware_consensus::types::Epoch::new(
            crate::beacon::testing::DETERMINISTIC_BOOTSTRAP_EPOCH,
        )
    }

    /// `Round(DETERMINISTIC_BOOTSTRAP_EPOCH, view)` — the round a height whose
    /// epocher is [`beacon_active_epocher`] resolves its σ at.
    fn active_round(view: u64) -> commonware_consensus::types::Round {
        commonware_consensus::types::Round::new(
            active_epoch(),
            commonware_consensus::types::View::new(view),
        )
    }

    /// An epocher that puts heights 96..=143 — the band every fixture in this
    /// module anchors in — inside `DETERMINISTIC_BOOTSTRAP_EPOCH`, so
    /// `mandatory_at` answers TRUE and σ is actually consulted.
    ///
    /// The DEFAULT fixture epocher puts every height in epoch 0, which is
    /// beacon-INACTIVE: there the agreed derivation is `None` and a σ in the
    /// store is ignored. A test whose subject is σ — a value reaching the
    /// deriver, or a block HELD waiting for one — must use this, and must then
    /// supply σ for EVERY height it expects to derive.
    fn beacon_active_epocher() -> crate::epocher::OriginEpocher {
        crate::epocher::OriginEpocher::new(0, std::num::NonZeroU64::new(48).expect("nonzero"))
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
        /// Hashes reth resolves NO header for, whatever the frontier — the one
        /// knob that keeps a re-apply re-walk failing with `ParentHeaderMissing`
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
        /// Arm [`Self::never_visible`] for `hash`.
        fn hide(&self, hash: B256) {
            self.never_visible.lock().unwrap().insert(hash);
        }
        /// `true` iff reth would resolve `header(hash)`. An untracked hash is
        /// treated as visible (only the explicitly-modelled segment participates).
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
        /// Heights the EL serves NOTHING for, one decrement per by-NUMBER read —
        /// the post-devp2p-backfill window where a block is landed but not yet
        /// index-visible. `u32::MAX` models an EL that never serves it.
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
            // Canonical chain below the cursor — mirrors `ProviderExecutedChain`.
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
        /// Records the (height, seed) passed to each `derive_and_execute` so a
        /// test can assert the cert-recovered seed actually reaches the deriver.
        /// Mutex<Vec> so it survives the deriver clone (Arc-shared).
        seeds_seen: SeedsSeen,
        /// Heights whose NEXT `derive_and_execute` fails with a plain (untyped)
        /// `eyre` error, then succeeds — the transient derive failure the
        /// speculative path must survive without taking the node down.
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
            // Fold the seed into the sealed hash (prev_randao→mix_hash model)
            // BEFORE `seed` is moved into `seeds_seen`, so notarize-round vs
            // finalize-round divergence is observable in-test.
            let discriminator = seed_folded_discriminator(order.digest(), &seed);
            self.seeds_seen.lock().unwrap().push((order.height, seed));
            if self.derive_fail_once.lock().unwrap().remove(&order.height) {
                return Err(eyre::eyre!(
                    "simulated transient derive failure at height {}",
                    order.height
                ));
            }
            // Model derive_sync's by-HASH parent read: a parent not yet canonical
            // by hash is `ParentHeaderMissing`. Default frontier = MAX ⇒ always
            // visible (no-op for tests that don't exercise the lag).
            if !self.chain.vis.visible(parent_evm_hash) {
                // The TYPED error the real deriver returns (`node/src/derive.rs`),
                // not a look-alike string: `is_parent_not_visible` keys on the type
                // through the walk's `wrap_err` chain, so an untyped model would
                // make the park untestable.
                return Err(crate::application::ParentHeaderMissing(parent_evm_hash).into());
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
            // Registered (by-number present) but NOT canonicalized: only an FCU
            // makes a block by-hash visible. Modelling them as one event is what
            // hid the gap-walk parent-visibility defect from every test.
            self.chain.vis.register(order.height, sealed.hash());
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
        /// Item 5: `fork_choice_updated` returns
        /// `Err(EngineError::anchor_inconsistent)` — reth PROCESSED the update and
        /// rejected the state ("unknown finalized/safe hash"). Sticky, not a
        /// countdown: the condition is structurally permanent, which is exactly
        /// why classifying it as transport made the retry loop unbounded.
        fcu_anchor_inconsistent: Arc<Mutex<bool>>,
        /// How many times the arm above fired. The assertion that matters is that
        /// this stays BOUNDED: the pre-fix classification retried the same
        /// unresolvable hashes without limit.
        fcu_anchor_rejections: Arc<Mutex<u32>>,
        /// Override for the `import_derived` status; `None` ⇒ Valid.
        import_status: Arc<Mutex<Option<PayloadStatusEnum>>>,
        /// Gap-1 (family 5): leading `import_derived` calls that return a
        /// transport `Err(EngineError)` (a closed engine channel) before
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
                    // A transport blip: reth was never reached, so nothing is
                    // recorded/canonicalized — the caller must retry.
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
            // Only a VALID forkchoice canonicalizes. SYNCING means a backfill
            // holds the engine and reth did NOT make the segment canonical — the
            // cause the parent-visibility park exists for, so a model that raised
            // the frontier here could not express it at all.
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
                    // A closed engine channel: nothing imported — the executor
                    // degrades + defers to reconvergence (never actor-death).
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
            // Gated-landing model: a SUCCESSFUL (Valid) import is what lands the
            // block — mirrors the real EL where the transport-failed insert above
            // left nothing behind.
            if let Some(chain) = self.land_chain.lock().unwrap().as_ref() {
                if status == PayloadStatusEnum::Valid {
                    let (height, hash) = (data.number(), data.hash());
                    chain.canonical.lock().unwrap().insert(height, hash);
                    // Registered only: an import lands the block in reth's
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
        /// Set to make [`BlockFetcher::pair_at`] answer `None` at every height.
        /// The default (`false`) models the real invariant — a marshal emits
        /// `Update::Tip(h)` only from `store_finalization`, which has just written
        /// the pair at `h` (CW `marshal/core/actor.rs:1404-1463`), so every tip the
        /// executor sees has a pair behind it. Setting it models the one case
        /// where it does not: a heartbeat re-poke replaying a tip the floor has
        /// since moved past.
        ///
        /// A synthesized pair rather than a per-test canned one because nothing in
        /// this module's re-jump tests reads the target's CONTENT — they script
        /// the outcome (`Scripted`) — and priming a real certificate per height in
        /// twenty tests would buy nothing. The stand runs the production jump over
        /// the production archive, and that is where the content matters.
        archive_empty: Arc<Mutex<bool>>,
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
        /// the #14 tests assert the `engine_retry` gauge + counter. (It used to name
        /// `auth_rotate` alongside it; that reason went with `SyncReason::AuthRotate`
        /// in pass Б2.4, and no test asserts it any more.)
        sync_metrics: SyncMetrics,
        /// Fork-safety latch the built actor's `Config` carries — exposed so the
        /// Phase-3 SafetyHalt tests assert it engages on divergence / EL-Invalid.
        safety_halt: crate::sync_metrics::SafetyHalt,
        /// FCU-heartbeat interval. Default 60 s so heartbeats never interfere with
        /// fast tests; the park tests that rely on the heartbeat re-poke lower it
        /// (the deterministic clock steps ~1 ms/iteration in real time, so a large
        /// virtual interval is real seconds) via `with_fcu_heartbeat`.
        fcu_heartbeat: Duration,
        /// Randomness handed to the built actor. Default: the real provider over
        /// the thread's `FIXTURE_SEEDS` store, which the block helpers file into,
        /// so σ resolves by round exactly as production does. Replace it via
        /// `with_seed_store` — with a store of the test's own, or with an EMPTY
        /// one to pin a MISS. Note the DEFAULT epocher is beacon-INACTIVE, so
        /// the store is only consulted under [`beacon_active_epocher`].
        randomness: std::sync::Arc<dyn crate::beacon::Beacon>,
        /// Block→epoch map handed to the built actor. Default: a single huge
        /// epoch so every test height maps to epoch 0 — below
        /// `DETERMINISTIC_BOOTSTRAP_EPOCH`, i.e. beacon-INACTIVE, where the
        /// agreed derivation is `None` and no block can be held for its σ. A
        /// test whose subject IS σ overrides with [`beacon_active_epocher`].
        epocher: crate::epocher::OriginEpocher,
        /// Restart-seed override for `last_execution_finalized_height` (the reth
        /// head = `provider.last_block_number()`). `None` ⇒ the historical
        /// `anchor_height` (head == acked == anchor). The
        /// `ordering_finalized`-seed test decouples the two (head ≫ acked with a
        /// speculative tail) to pin that the cursor seeds from the ACKED cursor.
        last_execution: Option<u64>,
        /// `Config::initial_marshal_floor` — `0` (inert) everywhere except the
        /// ladder-step test, which needs a floor BELOW the tip to show the step is
        /// judged against the floor and not against the tip.
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
                randomness: crate::beacon::testing::for_seeds(
                    FIXTURE_SEEDS.with(|seeds| seeds.clone()),
                ),
                epocher: crate::epocher::OriginEpocher::new(
                    0,
                    std::num::NonZeroU64::new(1 << 40).expect("nonzero"),
                ),
                last_execution: None,
                marshal_floor: 0,
            }
        }

        /// Override `last_execution_finalized_height` (the reth head seed),
        /// decoupling it from the anchor. Set BEFORE `build`.
        fn with_last_execution(mut self, height: u64) -> Self {
            self.last_execution = Some(height);
            self
        }

        /// Boot with a non-zero marshal floor — a node that has jumped, so its
        /// marshal holds nothing below `height` and the ladder step's no-op rule
        /// (`HintFinalized` skipped at `height <= last_processed_height`) has a
        /// real boundary to be tested against. Set BEFORE `build`.
        fn with_marshal_floor(mut self, height: u64) -> Self {
            self.marshal_floor = height;
            self
        }

        /// Override the block→epoch map (the epoch-boundary eager-derive
        /// tests need real, small epochs). Set BEFORE `build`.
        fn with_epocher(mut self, epocher: crate::epocher::OriginEpocher) -> Self {
            self.epocher = epocher;
            self
        }

        /// Replace the default store with `store` — the test's own σ source, and
        /// (empty) the way to opt out of the default and pin a store MISS. Set
        /// BEFORE `build`.
        fn with_seed_store(mut self, store: crate::beacon::testing::SeedStore) -> Self {
            self.randomness = crate::beacon::testing::for_seeds(store);
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
                    // No committee module in these fixtures — the wake-up has
                    // nothing to wake. Counting the calls is not this file's
                    // property to assert: what the module needs is that they sit
                    // after `advance_finalized`, which is a property of THIS
                    // file's source and is read there.
                    anchor_advanced: std::sync::Arc::new(|| {}),
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

    /// The same seed behind the witness [`SeedStore::record`] now takes. The
    /// deal is deterministic, so the key this checks against is the key
    /// [`real_seed`] signed under.
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

            // A subsequent clean heartbeat clears the gauge (fire-and-forget: the
            // NEXT tick is the retry, no in-place loop).
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
            let flush = child_of(&refused);
            let post_halt = sample_order(flush.digest(), ANCHOR + 3, B256::ZERO);
            let (msg, waiter) = finalize_msg(refused);
            mailbox.send(msg).expect("send");
            // The flush child triggers the derive (and the Invalid FCU). Its own
            // ack sits in `awaiting_seed` when the halt engages — it must be
            // retained too (the park_halted awaiting_seed clause).
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

    // FAULT-BOUNDARY (family 5). The speculative path is best-effort, and the
    // taxonomy must not quietly change that: `spec_execute` classifies a derive
    // failure `Defer(SpecDeriveFailed)`, so the router logs it and the loop
    // continues. Without the explicit classification the blanket
    // `From<eyre::Report>` would make it `Corruption` and a transient derive
    // failure would start killing the executor.
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

            // The finalized path is the authority and still lands the height.
            let (msg, waiter) = finalize_msg(order.clone());
            mailbox.send(msg).expect("send finalize");
            waiter
                .await
                .expect("the finalized path derives the height regardless");

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // THE ASYMMETRY, PINNED. One and the same `Ok(Invalid)` FCU verdict has two
    // dispositions, and which one applies is decided by whether the head is
    // committed:
    //
    //  * SPECULATIVE head (notarized, not finalized) → skip speculation, no
    //    latch. Consensus may still nullify the view and finalize a sibling, so
    //    the verdict does not yet indict anything the chain committed.
    //  * FINALIZED head → #15 SafetyHalt. The block IS committed.
    //
    // The second half is what makes the first half safe: nothing is swallowed,
    // it is only deferred to the path that has committed evidence.
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

            // Same verdict, now on a committed head.
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

    // THE ORIGINAL DEFECT. `submit_finalized_payload` used to engage the latch
    // itself and return an untyped `Err`, and the speculative caller reduced that
    // `Err` to `warn!("speculative execution skipped")` — so the node latched "I
    // refuse this chain" and then kept driving reth forward with it. Engaging
    // moved into the router, so a fork-safety verdict raised on the speculative
    // path now reaches `park_halted` like any other.
    #[test]
    fn a_fork_safety_verdict_on_the_speculative_path_parks_instead_of_being_logged() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            // An `Invalid` IMPORT is a statement about our own derivation against
            // reth's re-execution — deterministic and branch-independent, so it
            // halts from either path (unlike the FCU verdict above).
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
            // Pre-fix the executor kept consuming work after latching. A block
            // delivered now must have its ack RETAINED by the park, never acked.
            let (msg, mut waiter) = finalize_msg(order);
            mailbox.send(msg).expect("mailbox stays open while parked");
            ctx.sleep(Duration::from_millis(20)).await;
            assert!(
                (&mut waiter).now_or_never().is_none(),
                "a parked executor derives nothing and acks nothing"
            );
        });
    }

    // ITEM 5. reth answers "unknown finalized/safe hash" with
    // `Err(ForkchoiceUpdateError::InvalidState)` — it PROCESSED the update and
    // rejected the state we named. The importer used to flatten that into the
    // transport class, so `fcu_retrying_transport` re-sent the same unresolvable
    // hashes forever. It is `Corruption` now: loud actor death, and deliberately
    // NOT a SafetyHalt, because it says this node's anchor disagrees with this
    // node's own EL, not that the network disagrees with the chain.
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

            // The whole point: this TERMINATES. Pre-fix the FCU loop had no exit.
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

    // FIXED behavior of the formerly seed-blind fork-safety bug (bundle block
    // 5252): a block speculatively executed with a seed of round A while the
    // AGREED round for that height is B. `spec_executed` records the
    // speculation's seed ROUND, and `correctly_speculated` requires it to equal
    // the round the finalized derive resolved. On the A≠B mismatch (after §4.1
    // an ANOMALY — both sides should be Round(Ep, proposal_view)) the executor
    // RE-DERIVES SPEC_H with seed_B (the agreed value) and reorgs the head onto
    // it — so K blocks later the committee-attested result (seed_B → hash_B)
    // MATCHES the locally executed hash and NO `ResultDivergence` SafetyHalt
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

            // Two seeds for the SAME ordering block: the speculation's round A ≠
            // the AGREED round B ⇒ (via the prev_randao→mix_hash fold) DISTINCT
            // hashes. A names the block's own VIEW, so §4.1 keeps it verbatim
            // rather than re-canonicalising — a divergent local cert state, not a
            // spin round. B is the round the finalized derive resolves.
            let seed_a = real_seed(Round::new(Epoch::new(0), View::new(SPEC_H)));
            let seed_b = real_seed(active_round(SPEC_H));
            record_fixture_seed(SPEC_H);
            record_fixture_seed(0); // the view every `sample_order` block names

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

            // Ordering chain SPEC_H..=SPEC_H+K. Each element derives from σ of
            // its OWN round: SPEC_H from seed_B, the rest from `Round(e, 0)` —
            // `sample_order`'s view — both filed above.
            // SPEC_H+1 pre-K ⇒ ZERO; +2 commits Height(ANCHOR); +K attests hash_B.
            let order_h1 = sample_order(order_h.digest(), SPEC_H + 1, B256::ZERO);
            let order_h2 = sample_order(order_h1.digest(), SPEC_H + 2, anchor_hash);
            let order_hk = sample_order(order_h2.digest(), SPEC_H + K, hash_b);

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

            // CRUX of the result-gate fix: once the
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

            // (2) FINALIZE SPEC_H. `correctly_speculated` sees the stored
            // speculation round A ≠ the agreed round B ⇒ re-derive → hash_B
            // becomes canonical at SPEC_H.
            let (m, w) = finalize_msg(order_h.clone());
            mailbox.send(m).expect("send finalize SPEC_H");
            let (m1, w1) = finalize_msg(order_h1);
            mailbox.send(m1).expect("send finalize SPEC_H+1");
            w.await.expect("SPEC_H acks after re-derive with seed_B");

            // The seed-blind reuse is GONE: hash_B (the agreed value) is
            // canonical at SPEC_H, NOT the seed_A speculation.
            assert_eq!(
                fx.chain.spec_executed_hash(SPEC_H),
                Some(hash_b),
                "round mismatch re-derived SPEC_H with the agreed seed (hash_B)"
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

            // (3) Advance ordering to SPEC_H+K so the hash_B attestation reaches
            // the result cross-check.
            for order in [order_h2, order_hk] {
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

    // Property (a) — the fast path is INTACT: when the speculation's seed round
    // EQUALS the finalization cert's seed round (the common case), the block is
    // KEPT (no re-derive, no head rollback) and no halt fires. Guards the added
    // seed-round check against regressing `correctly_speculated` toward "always
    // re-derive" (property (c)).
    #[test]
    fn spec_same_round_keeps_speculation_no_rederive() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const SPEC_H: u64 = ANCHOR + 1;
            let fx = Fixture::new(ANCHOR).with_epocher(beacon_active_epocher());
            let anchor_hash = fx.anchor_hash;
            // The notarization and the store carry the SAME round (the honest
            // steady state: both are `Round(epoch(h), proposal_view)`).
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
    // The re-apply retry on a still-invisible parent is BOUNDED: an unbounded one
    // re-creates the silent spin the finalized-tier gate removes. Above the
    // finalized tier (so that gate does not fire), reth keeps a foreign hash at
    // the height and drops every re-derived sibling, so the loop is really
    // entered; the parent is then hidden MID-LOOP — the first derive must succeed
    // for the loop to exist at all — and the walk fails `ParentHeaderMissing`
    // forever. The bound must convert that into loud death, not a spin.
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

                // The bound is 50 * ENGINE_TRANSPORT_RETRY_BACKOFF ~= 10 virtual s,
                // which outlasts `wait_until`'s 2 s horizon — hence the coarser
                // local wait, still bounded so an unbounded retry fails by timeout.
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
                // DROPPED (Canceled) — the opposite of the SafetyHalt park, which
                // retains it.
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
    // refuses to move the head to `height <= finalized_height`, so every iteration
    // re-sends the SAME forkchoice and reth (which will not reorg below its own
    // finalized block) keeps serving the other hash. Pre-gate that is a silent
    // 200 ms spin with `finalize_apply` degraded forever; the conflict is a
    // fork-safety verdict, so it must LATCH.
    #[test]
    fn reapply_below_the_finalized_tier_halts_instead_of_spinning() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            // The cold-start trust anchor IS the EL's finalized height, so a block
            // delivered below it is a finalized-tier conflict by construction.
            const L: u64 = 100;
            let fx = Fixture::new(L);
            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(L - 2, B256::repeat_byte(0xB2));
            // What reth already holds at L-1, and will not give up: the re-derived
            // sibling is dropped forever (the soak3 `InsertExecutedBlock` model).
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

            // `wait_until` is the bound: 2000 virtual ms, ~10 re-apply iterations at
            // ENGINE_TRANSPORT_RETRY_BACKOFF, then a named panic — a spin fails the
            // test instead of hanging the suite.
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

    // The OTHER arm of the same gate: the EL serves NOTHING at a height it holds
    // as finalized. That is the post-devp2p-backfill by-NUMBER blind spot
    // `reseed_forward` answers with a belt, not a settled conflict, so the gate
    // re-reads and the node heals. Pre-belt this is immediate actor death.
    #[test]
    fn finalized_tier_absent_block_heals_within_the_visibility_belt() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            // The cold-start trust anchor IS the EL's finalized height, so a block
            // delivered below it lands in the finalized-tier gate by construction.
            const L: u64 = 100;
            let fx = Fixture::new(L);
            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(L - 2, B256::repeat_byte(0xB2));
            // The EL lands L-1 on derive but stays by-NUMBER blind for the next
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

    // The belt is BOUNDED: an EL that never serves a height it claims as finalized
    // is local corruption, so the actor dies LOUD rather than stalling the ack
    // forever. Corruption, not fork-safety — the latch must stay clear and the
    // in-flight `Exact` is dropped (Canceled), exactly like the other corruption
    // exits in this file.
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

                // The bound is 50 * ENGINE_TRANSPORT_RETRY_BACKOFF ~= 10 virtual s,
                // which outlasts `wait_until`'s 2 s horizon — hence the coarser
                // local wait, still bounded so an unbounded belt fails by timeout.
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
            let fx = Fixture::new(ANCHOR).with_epocher(beacon_active_epocher());
            let anchor_hash = fx.anchor_hash;

            // Speculation round A ≠ the AGREED round B the finalized derive
            // resolves from the store.
            let seed_a = real_seed(Round::new(Epoch::new(0), View::new(SPEC_H)));
            let seed_b = real_seed(active_round(SPEC_H));
            record_fixture_seed(SPEC_H);
            record_fixture_seed(0); // the view every `sample_order` block names

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

            // SPEC_H+1 pre-K ⇒ ZERO; +2 commits Height(ANCHOR); +K attests hash_B.
            let order_h1 = sample_order(order_h.digest(), SPEC_H + 1, B256::ZERO);
            let order_h2 = sample_order(order_h1.digest(), SPEC_H + 2, anchor_hash);
            let order_hk = sample_order(order_h2.digest(), SPEC_H + K, hash_b);

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

            // (2) Finalize SPEC_H: the round guard routes to the re-derive, whose
            // sibling import the EL DROPS twice; the ack must not fire until the
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

            // (3) Advance to SPEC_H+K: the attested hash_B matches the local
            // chain → derives cleanly, NO SafetyHalt (pre-fix: ResultDivergence
            // here).
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

            // A live finalize for height 4 lands BEFORE the backfill drains; it
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
            // (each element resolves σ at its own round; the walk needs no
            // certs). The result still commits the derived hash at 101.
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

    // The gap-walk PREFIX resolves σ the same way the main path does — at each
    // element's OWN round, predicate first — and never from the delivered block.
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
            // 101/102 are inside the pre-activation window (result MUST be ZERO);
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

    // A gap-walk PREFIX element on a beacon-ACTIVE round with no σ PARKS. The walk
    // owns neither `cause` nor the ack, so it cannot hold the block itself — it
    // reports the typed leaf and `try_derive`, which owns the park, converts it.
    // Deriving with the digest fallback would silently fork, so waiting is the only
    // correct answer.
    #[test]
    fn a_gap_walk_prefix_miss_on_a_beacon_active_round_parks() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let store = crate::beacon::testing::SeedStore::new();
            // σ for the DELIVERED height only — the prefix element at 101 has none.
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

    // The park's only exit: σ lands in the SAME store the walk reads (in production
    // the cert inlet writes it), the parked block is re-poked, and the walk re-runs
    // its own per-element lookup — which is why the park can carry the DELIVERED
    // height's σ without ever carrying `None` for the prefix.
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

            // The arrival the park waits for. Nothing is asked of any peer.
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

    // The beacon-INACTIVE arm is not a miss and must NEVER hold: `None` is the
    // agreed derivation there, so the block derives at its own delivery even with
    // an EMPTY store. Holding would wedge every pre-beacon height forever.
    #[test]
    fn a_beacon_inactive_height_derives_immediately_and_never_holds() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            // Default epocher ⇒ epoch 0 ⇒ beacon-INACTIVE; store deliberately empty.
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

    // PARENT-VISIBILITY, gap-walk: the walk imports each prefix block and hands
    // its hash to the NEXT derive as a by-hash parent. Without a canonicalization
    // FCU per landed block the walk's SECOND element derives against a parent that
    // only exists in reth's tree-private state — pre-fix this fails with
    // ParentHeaderMissing at height 98.
    #[test]
    fn gap_walk_canonicalizes_each_landed_block_for_the_next_by_hash_parent() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 96;
            let fx = Fixture::new(ANCHOR);
            // Arm the by-hash lag: everything above the anchor is present by
            // NUMBER but invisible by HASH until an FCU raises the frontier.
            // Without this `visible()` short-circuits to true and the test proves
            // nothing.
            fx.chain.vis.set_frontier(ANCHOR);
            // A TWO-block prefix exercises parent-to-parent chaining WITHIN the
            // walk: 98 derives on a parent the walk itself imported one iteration
            // earlier. The one-block shape — the one observed live — is covered
            // separately, where the walk's only element is also its last.
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

            // The mechanism, not just the outcome. Asserted on the RECORDED SET:
            // `FakeBeacon` stores `fcu_calls` with no derive interleaving, so
            // "issued before the next derive" is not directly observable — and does
            // not need to be, since the outcome assertion above already fails
            // without the FCU.
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

    /// Drive a ONE-block gap: 97 missing, 98 delivered, by-hash frontier at the
    /// anchor. This is the shape observed live (`first_missing == target`), where
    /// the walk's only element is also its last — so the walk returns `Ok` and the
    /// DELIVERED derive is what depends on the canonicalization, which is exactly
    /// what an entry-only catch misses. Both 97 and 98 are pre-activation
    /// (`< anchor + K`), so a ZERO result is the correct commitment at each.
    ///
    /// The CALLER decides whether the FCU can land: arm nothing and the walk heals;
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

    // THE SHAPE THAT CRASHED LIVE, healed: a one-block gap where the walk's only
    // element is also its last, so the block the walk imports is handed straight to
    // the DELIVERED derive as a by-hash parent. The two park tests cover this same
    // shape with the FCU defeated — without this one it would be asserted to fail
    // safe and never asserted to make progress.
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
    // forkchoice canonicalizes NOTHING — which is why the fix cannot simply await
    // VALID, and why the walk must survive an FCU that does not land.
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

            // The backfill goes idle: reth answers VALID again, and 97 — imported
            // but never canonicalized — is correspondingly absent from the
            // by-number canonical index the re-walk probes (`provider.block_hash`
            // in production). So the re-poke RE-WALKS, and it is the re-issued FCU
            // that finally canonicalizes 97.
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

    // A GAP block (filled by `derive_finalized_with_gap_fill`, not the top-level delivery)
    // carries its OWN attested `result`; a forged value on a gap-range block must
    // fail loud just like the top-level cross-check — otherwise a
    // committee-attested wrong result on a gap block is imported unchecked (the
    // byzantine-vrf defense). Here ANCHOR+K (the first POST-pre-activation gap
    // height) commits a forged hash; the gap-walk derives it, the cross-check
    // engages the SafetyHalt, and the executor parks retaining the ack. This
    // pins the `?`-propagated halt path (the engage fires INSIDE
    // `derive_finalized_with_gap_fill`, below the `inflight_ack` slot).
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

    // A block on a beacon-ACTIVE round whose σ has not landed is HELD, not
    // parked: no derive, no ack, no park (`deferred` stays empty), no hint, and
    // no marshal fetch. Recording σ into the store the actor holds fires the
    // seed-record Notify, and the executor's REAL `seed_notify` select! arm —
    // not a hand-driven call — derives and acks it. That arm is the hold's only
    // exit: no further delivery, no timer.
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
            // `wait_until` panics after 2000 virtual ms, so a regression FAILS
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

    // (a) THE soak7 BUG, reproduced at the executor contract — the headline
    // test. No per-height finalization cert exists ANYWHERE (the executor has no
    // cert lookup), `spec_executed` is EMPTY (a restarted / lagging / following
    // node), and no successor has been delivered ⇒ the height derives with the
    // REAL threshold seed of its OWN round and acks. Pre-B′: permanent park
    // (CertMissing → PARK on every re-poke, forever, network-wide).
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
    // beacon-INACTIVE — derives IMMEDIATELY with the `order.digest()` fallback:
    // no hold, no hint. `None` is the agreed derivation there, not a miss, and
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

    // PREDICATE FIRST, and this is the case that distinguishes it: σ IS in the
    // store for the block's own round, but the agreed epoch map calls that epoch
    // beacon-INACTIVE, so the derive must IGNORE it and use `None` — what the
    // rest of the network derives. Store-first ordering passes every other test
    // in this file and fails exactly here.
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
            // keys on the beacon-ACTIVE epoch: the round wanted here is the one
            // this fixture's epocher names, and it is the inactive one.
            let round = Round::new(Epoch::new(0), View::new(VIEW));
            FIXTURE_SEEDS.with(|seeds| seeds.record(real_witness(round)));
            assert!(
                FIXTURE_SEEDS.with(|seeds| seeds.lookup(round).is_some()),
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

    // The bootstrap edge, which the old field-keyed derive could not serve: at
    // the FIRST block of the first beacon-active epoch `witness_link` keys the
    // wire field on the PARENT's epoch (`mandatory_at(1)` — false), so an honest
    // block there carries no seed at all, while the beacon IS active at its own
    // epoch and σ for `Round(2, 16)` exists. Keyed at the block's OWN round the
    // derive finds it; keyed off the wire it fell through to the digest fallback.
    //
    // h = 17 is the second half of the claim: the successor derives with
    // σ(2, 17), its own round, not with the edge's.
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

    /// A `SpecNotarized` command carrying a real recovered seed (populates
    /// `spec_executed[h].seed_round` — reconciled against the round the
    /// finalized derive resolves).
    fn spec_msg_seeded(order: &OrderBlock, seed: crate::beacon::Seed) -> Message {
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

            // A linked seedless chain ANCHOR+1..=ANCHOR+N whose `result` fields
            // commit the hashes the deriver WILL produce: every height now
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
    // (σ is resolved locally and every delivered block is its own walk element).
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

    // (d) GUARD #2, re-gated to `last_tip >= h + K`: a catching-up node whose
    // committee-attested block at `h + K` disagrees with the hash it derived
    // engages SafetyHalt(ResultDivergence) BEFORE the ack. Green ONLY because
    // this fixture's `FakeDeriver` lands the derived hash at derive time
    // (`land_on_import` off, the default); with reth's FCU-only canonicalisation the guard reads
    // `None` and the halt comes K blocks later (R-006 scenario 1, testbed (3b)).
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
            // The attested root at H+K commits a DIFFERENT hash than the derive
            // produces ⇒ a fork the guard must catch.
            let forged = B256::repeat_byte(0xEE);
            let order_hk = sample_order(Digest(B256::ZERO), H + K, forged);
            fx.marshal.canned.lock().unwrap().insert(H + K, order_hk);

            // The node is BEHIND: the finalized frontier is already past H+K.
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

    // (e) GUARD #2's absent-body arm — the executor's ONLY park: the node is
    // behind (`tip >= h + K`) but the attested body at `h + K` is not
    // backfilled yet ⇒ PARK before the ack and before the `split_off` prune,
    // hint exactly `h + K`, hold the next delivery QUEUED behind the park (the
    // drain is gated on it), and keep speculation suppressed while parked (the
    // `spec_execute` guard is kept, not narrowed). When the body lands, the
    // re-poke re-derives with the RETAINED seed (zero lookups) and the queued
    // child derives right after — nothing is lost.
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

            // The derive ran (import happened) but the ack is withheld — parked
            // on the absent H+K body, with exactly H+K hinted.
            ctx.sleep(Duration::from_millis(50)).await;
            {
                let payloads = fx.beacon.new_payload_calls.lock().unwrap();
                let heights: Vec<u64> = payloads.iter().map(|p| p.number).collect();
                assert!(
                    !heights.is_empty(),
                    "H derived + imported before parking on the absent h+K body"
                );
                // H may import more than once — every later delivery re-pokes the
                // park, and the re-poke re-derives. What must NOT appear is H+1:
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

            // Speculation stays suppressed while parked — the guard is KEPT.
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

            // The H+K body lands (attesting the hash the derive produced) → the
            // tip re-poke re-derives with the RETAINED σ and acks.
            let attested = fx.chain.spec_executed_hash(H).unwrap();
            let order_hk = sample_order(Digest(B256::ZERO), H + K, attested);
            fx.marshal.canned.lock().unwrap().insert(H + K, order_hk);
            // Same-height tip: the re-poke is the event; keeping the tip at
            // H+K leaves guard #2 COLD for the held child (tip < child + K), so
            // the child derives + acks below without needing an H+K+1 body.
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

    // (e′) The guard-#2 park's DELIVERY-INDEPENDENT backstop: a body landing at
    // `height <= tip` fires no `Update::Tip`, so the FCU-heartbeat re-poke is
    // what clears the park ([[dpos-deferred-catchup-invariants]] #3 — reused
    // tick, no new timer). The re-poke re-derives from the RETAINED σ
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
            // Beacon-ACTIVE epoch + an EMPTY store: the only way to hold a block.
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(crate::beacon::testing::SeedStore::new())
                .with_epocher(beacon_active_epocher());
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
    // for its σ re-dispatches `h` (the marshal's `last_processed_height` never
    // advanced) and derives it once σ is there, with NO hole — the hash equals
    // the one a never-stopped node derives. `awaiting_seed` needs no
    // persistence; the withheld ack is the durable record.
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

    // (P2 🟡) A FIRST-SEEN SPIN NOTARIZATION must not speculate with the spin
    // round's seed: §4.1 re-canonicalises the round to the block's own
    // `proposal_view`. Without a `SeedStore` entry for the canonical round the
    // speculation is SKIPPED (never speculate with a known-wrong seed); once σ
    // for that round lands the finalized path derives it exactly once, no reorg.
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

            // First-seen notarization at a SPIN round, store EMPTY → skip (no
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

    // (P2 🟡, the SeedStore arm) A node that HOLDS the canonical round's seed in
    // its `SeedStore` re-canonicalises the spin notarization and speculates with
    // the SAME seed everyone else uses — the finalized reconcile then reuses the
    // speculation (rounds match; no re-derive, no reorg).
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
            // Finalize resolves the SAME round → reconcile REUSES the spec.
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

    // (c′) F4 CROSS-NODE CONVERGENCE — the fork hazard this design kills: two
    // nodes whose local cert state named DIFFERENT spin rounds for the same
    // height both derive it from σ of the block's OWN agreed round ⇒ identical
    // hash. (Under the pre-B′ `lookup_seed` path each derived from its own local
    // cert's round.)
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

    // (a) THE DEATH SPIRAL, in miniature: a
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
            // (100): a gap (103 > 101) ⇒ PARKED (pre-fix: dropped forever).
            mailbox
                .send(spec_msg(&o3))
                .expect("spec 103 (gap → parked)");

            // Finalize 101, 102, 103. 102's derive advances `spec_head` to 102,
            // which is what fires the drain for the parked 103 notarization.
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
            actor.try_drain_parked(&cause).await.expect("drain");

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
            // `result`-consistent: 103 derives on the finalized path too, so its
            // committed result must match what the deriver produces at 100.
            let chain = result_consistent_chain(ANCHOR, fx.anchor_hash, 3);
            let (o1, o2, o3) = (chain[0].clone(), chain[1].clone(), chain[2].clone());
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

    // (a) THE SIBLING-ROLLBACK CRASH, in miniature: a
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
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR).with_epocher(beacon_active_epocher());
            let anchor = fx.anchor_hash;

            // 101 is speculated with a round the AGREED map does not name (a
            // divergent local cert state at the block's own view, which §4.1
            // keeps verbatim) ⇒ the finalized derive re-keys it and rolls back.
            // 102/103 speculate with their own agreed rounds, so ONLY 101 rolls
            // back.
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

            // Each height derives at its own delivery: 101 (round mismatch →
            // rollback + re-derive), then 102 (must RE-DERIVE, not reuse fork-A),
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
            // 103 derives on top of the re-derived 102, so the FINAL head is
            // 103's hash — but it must DESCEND from hash_fin_102, and the head
            // must have visited hash_fin_102 on the way (pre-fix it stayed stuck
            // at 101 and never reached either).
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

    // (b) PARENT-LINKAGE, isolated: a speculated block whose recorded parent no
    // longer matches the block canonical at `height − 1` is REJECTED by
    // `correctly_speculated` and re-derived — even though the seed ROUND and the
    // ordering DIGEST both match. Here the seed VALUE is identical on
    // both paths, so the only difference between the reuse hash and the re-derive
    // hash is the PARENT — proof the parent-linkage clause (not the round clause)
    // forced the re-derive.
    #[test]
    fn stale_parent_speculation_is_rejected_despite_matching_round() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR).with_epocher(beacon_active_epocher());
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let cause = Span::current();

            // 101 speculated with the SAME σ the finalized derive resolves for
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

            // A parent reorg with NO rollback event of its own: the block canonical
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

    // EAGER FINALIZED DERIVE (record-lag closer): a
    // delivered finalized `h` whose OWN agreed round `Round(0, proposal_view)` is
    // in the SeedStore is derived + finalized-recorded AT DELIVERY, before its
    // child `h+1` exists — closing the recorded_tip = delivered_tip − 1 lag that
    // livelocked the finalized-tier result gate (nullify storm / stall).
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

    // The fixture's DEFAULT σ source is a LIVE store, not a negative provider:
    // a witnessed link files σ under the parent's own round, so `h` derives from
    // the store at its OWN delivery with no `with_seed_store` and no child. This
    // is the source the derive re-keys onto, and a default that silently went
    // back to answering `None` would leave every such link deriving from the
    // child body instead — invisible here, a hang once the child stops carrying it.
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

    // MISS: with the round ABSENT from the store — and the epoch beacon-ACTIVE,
    // so `None` is not the agreed answer — the delivered block stays HELD. The
    // only exit is σ arriving; there is no fallback and no deadline.
    #[test]
    fn a_store_miss_on_a_beacon_active_round_holds_the_block() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let h = ANCHOR + 1;
            // Store present but EMPTY — the opt-out from the fixture default, and
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

    // EAGER + REORG: `h` speculated as sibling A, then finalized-DELIVERED as a
    // DIFFERENT sibling B while the round is in the store — the eager derive takes
    // the re-derive path (the v21-shape assertion at the eager site), records B,
    // and NEVER leaves the speculated A behind.
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

    // CHILD-DELIVERY-AFTER-EAGER: once `h` is eager-consumed, delivering its child
    // `h+1` must NOT re-derive `h` (the hold is gone) — `h+1` simply becomes the
    // new held tip (its own round is not in the store ⇒ a miss ⇒ hold).
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

    // EPOCH-BOUNDARY eager derive (the divergence-critical epoch identity): `h` is
    // the LAST block of epoch e, so its child crosses into e+1 and `witness_link`'s
    // boundary adjustment (`ec − 1`) pins the wire field's round epoch to e — which is
    // exactly `epocher.containing(h).epoch()`. With the store populated under
    // `Round(e, view)` the eager derive HITS with the correctly computed epoch-e
    // round and records `h` before the child exists.
    #[test]
    fn eager_derive_hits_at_the_epoch_boundary_with_the_parent_epoch_round() {
        use commonware_consensus::types::{Epocher as _, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            // origin 0, length 8: epoch 2 = heights 16..=23; h = 23 is its LAST
            // block (the child at 24 is the first block of epoch 3). Epoch 2 is
            // `DETERMINISTIC_BOOTSTRAP_EPOCH`, the first beacon-ACTIVE one — a
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
            const ANCHOR: u64 = 22;
            const H: u64 = 23; // last block of epoch 2, the bootstrap epoch
            let store = crate::beacon::testing::SeedStore::new();
            // SAME view, WRONG epoch (e+1 = 3): the only entry in the store.
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

    // SEED-NOTIFY RE-ATTEMPT (the deadlock-breaker; migrated from the deleted
    // `SpecNotarized` Poke — the race in miniature):
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
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let h = ANCHOR + 1;
            // Store starts EMPTY: the delivery-time eager derive must miss.
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

    // SEED-NOTIFY NO-OP (b): a notify re-attempt must NOT spuriously derive when
    // either (i) nothing is held (the arm's `awaiting_seed.is_some()` guard is
    // false), or (ii) a tip is held but the store STILL misses its round (the
    // seed has not landed yet — a later notify will derive
    // it). Neither path may advance the EL or touch the hold.
    #[test]
    fn seed_notify_is_a_noop_without_hold_or_seed() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let h = ANCHOR + 1;
            // EMPTY store: the opt-out from the fixture default, so both arms
            // below are reached with nothing recorded for h's round.
            let store = crate::beacon::testing::SeedStore::new();
            let fx = Fixture::new(ANCHOR)
                .with_seed_store(store)
                .with_epocher(beacon_active_epocher());
            let (mut actor, _mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let cause = Span::current();

            // (i) No hold: the arm's guard (`awaiting_seed.is_some()`) is false,
            // so the body is a no-op even if driven directly (the take() early
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

            // Finalize anchor+1 (sets safe = head = h(anchor+1); finalized
            // clamped at the anchor in the pre-K window). +2 is NOT finalized —
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
    // execution, and the same-round reconcile keeps the speculation
    // (the deriver runs exactly once).
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
            // Finalize the same order: the store answers the SAME round, so the
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

    // steady-state re-jump

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

    /// The catch-up ACK BARRIER: deliver `order` and await its ack. The re-jump
    /// tests park the frontier far
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

    /// A re-jump callback recording each `from` it was invoked with and returning
    /// the scripted [`crate::cold_start_jump::JumpOutcome`].
    type RejumpCalls = Arc<Mutex<Vec<u64>>>;
    fn recording_re_jump(scripted: Scripted) -> (ReJump, RejumpCalls) {
        let (cb, calls, _targets) = recording_re_jump_with_targets(scripted);
        (cb, calls)
    }

    /// As [`recording_re_jump`] but also returns the TARGET heights the executor
    /// handed in — the `(finalization, block)` pair it read out of its own marshal
    /// archive at the tip it triggered on (§5.2).
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

    /// As [`recording_re_jump`] but wires a RECORDING `rotate` escape (the
    /// recording-rotate idiom from `cert_inlet::tests`), returning the rotation-count
    /// atomic so a test can assert Rule-L failover fired the expected number of times.
    /// `scripts` is a SATURATING sequence — call N returns `scripts[min(N, len−1)]` —
    /// so a single-element vec is the single-outcome case and a longer vec scripts a
    /// per-call outcome sequence.
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

    // STALE FINALIZATION BACKLOG PRUNE: deliveries
    // queued while the drain arm is gated off by an IN-FLIGHT jump are stale
    // below-landing blocks; reseed_forward must prune them (ack Ok — canonical
    // post-backfill, never Canceled) so the reopened drain does not re-populate
    // `awaiting_seed` with a jumped-over height — pre-fix, the first genuine
    // post-floor dispatch walked back into the jump-pruned range and hit the
    // missing-artifact fatal (a jump-MANUFACTURED skip-gap misclassified as
    // archive corruption). Post-fix: backlog pruned+acked, the next post-floor
    // dispatch derives and acks cleanly, executor stays up.
    #[test]
    fn reseed_prunes_stale_queued_finalizations_no_missing_artifact_fatal() {
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
            // acks. Pre-fix the stale 101/102 drained first and THIS arrival's
            // gap-walk fetch of the (jump-pruned) prefix returned None — the
            // missing-artifact fatal.
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
                    "the executor stays up (no missing-artifact fatal)"
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

    // (4.2 Б1.1) THE TRIGGER IS THE MARSHAL TIP AND NOTHING ELSE — however loud
    // an unauthenticated source is about how far ahead the chain has run.
    //
    // WHAT THIS TEST USED TO SAY. It was
    // `re_jump_fires_off_upstream_frontier_when_marshal_tip_frozen`, and it
    // asserted the OPPOSITE: that a frozen marshal tip plus a far-ahead
    // `upstream_frontier` atomic MUST fire the jump, because the trigger read
    // `max(tip, upstream_frontier)`. That atomic existed for one reason — under
    // the "committee[E] not committed" defer the inlet stored nothing, so the tip
    // froze exactly when the jump was needed — and it paid for that with an input
    // nobody had authenticated: whoever fed the inlet, or answered the probe's
    // `Latest`, chose the number the deep trigger compared against. §5.2 removes
    // both the atomic and the reason: the frozen-tip probe puts the ladder step
    // `Finalized{last(T+1)}` on the marshal every frozen tick, a served step goes
    // through `verify_delivered`, and the tip moves. A tip that stays frozen is a
    // tip nothing VERIFIED has moved, and that is not a state to jump out of.
    //
    // The remaining unauthenticated input on the path is the probe's `Latest`
    // height, which is exactly what this test shouts: a probe answering with a
    // height 5_010 blocks past the serving window, and a marshal tip five blocks
    // above the anchor.
    //
    // RED under the mutation that hands the trigger that height — one line in
    // `probe_frontier`, `let _ = self.maybe_re_jump(frontier).await;`, which is
    // what `max(tip, upstream_frontier)` amounted to once the probe was the
    // atomic's only writer: `a re-jump was spawned on a height no one
    // authenticated: [100]`. (Run 2026-09-12; the mutation was reverted.)
    //
    // Falsifier: any recorded call (an unverified height reached the trigger); a
    // probe that never ran (then the loud source never spoke and the test is
    // vacuous).
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

            // A LOW (frozen) marshal tip: the gap off the tip is ≤ threshold.
            actor
                .maybe_re_jump(Height::new(ANCHOR + 5))
                .await
                .expect("no fault");
            ctx.sleep(Duration::from_millis(10)).await;
            // The loud source speaks — and is believed only as far as one
            // `hint_finalization`, which is a fetch the marshal verifies itself.
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
            // ...and the next frozen tip still measures the gap off the tip.
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

    // (4.2 Б1.2) AND THE TARGET COMES OUT OF THE NODE'S OWN ARCHIVE, AT THE TIP
    // IT TRIGGERED ON — not from anything a peer answered.
    //
    // Falsifier: a target height that is not the tip; no call at all (then the
    // fixture stopped triggering and the assert above it is vacuous).
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

            // And with NO pair at that tip the trigger declines rather than
            // guesses: a heartbeat re-poke of a tip the floor moved past.
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

    /// A `ReJump` whose probe REPLAYS canned outcomes, one per tick (saturating on
    /// the last), and counts the ticks that actually reached the probe body.
    ///
    /// The probe closure itself is production's (`consensus/dpos.rs`, `stand.rs`)
    /// and is not under test here; what is under test is what
    /// `Actor::probe_frontier` DOES with a named step, which is why the outcomes
    /// are canned. `tracked_epoch` answers a constant: the only thing
    /// `probe_frontier` does with it is pass it to the closure and log it.
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

    /// A named ladder step for `height`, addressed at the dummy committee.
    fn step_at(height: u64) -> Option<(Height, NonEmptyVec<PeerPubkey>)> {
        Some((Height::new(height), dummy_peers().expect("one peer")))
    }

    /// A `CertUpstream` that answers nothing. The probe's `Latest` arm then
    /// returns `None`, so the only thing the follower-shape test below can
    /// observe is the LADDER STEP — which is the point: the step must not need
    /// an answered `Latest` to be taken (review A2-01 removed `servable`).
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
    /// epoch — the two things `dpos::frontier_probe` reads to NAME a rung
    /// (`geometry().last(T+1)` and `committee(T+1).participants`).
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

    // (review B1-01) A FOLLOWER CLIMBS THE SAME LADDER, AND ITS `T` IS ITS OWN.
    //
    // The two production pieces a follower now wires (`consensus/dpos.rs`, the
    // `launch_follower` re-jump: `probe: Some(frontier_probe(up, committee))` and
    // `tracked_epoch: Some(local_tracked_epoch(committee, finalized_cursor))`),
    // composed exactly as that site composes them, over an executor whose tip is
    // frozen. What the run has to show is one thing: the rung `last(T+1)` reaches
    // the marshal.
    //
    // WHY IT MATTERS THAT IT IS THE FOLLOWER. Until this pass the follower wired
    // `probe: None, tracked_epoch: None` — a validator's step and a follower's
    // silence — on the reasoning that its WS inlet is an always-on live producer.
    // The inlet is a SUBSCRIPTION: it replays no intermediate height, and every
    // cert above this node's own two-epoch ceiling is deferred by the committee
    // read window, storing nothing. With `upstream_frontier` deleted (§5.2) the
    // trigger reads the marshal tip alone, so at `fin == tip == ceiling` the gap
    // is 0, the tip is frozen, and nothing local can unfreeze it. The step is what
    // does: the marshal pulls `last(T+1)` BY HEIGHT through this node's own
    // upstream, `verify_delivered` stores it, `Update::Tip` re-arms the trigger.
    //
    // THE FIXTURE IS THAT STATE. `fin = tip = 95 = last(2)`, so ET's rule gives
    // `T = 3` and the rung is `last(4) = 159` — above the tip, which is the only
    // reason putting it does anything.
    //
    // RED before the fix (mutation: `local_tracked_epoch` returning `None`, which
    // IS the follower's pre-fix `tracked_epoch: None`): no step is named, `hints`
    // stays empty.
    //
    // WHAT IT DOES NOT SHOW, said here because the journal says it too: a live
    // follower under a real deep lag. The stand builds no follower, so the
    // acceptance for that class stays open (В§0(5)).
    //
    // Falsifier: an empty `hints` (the follower names no rung, i.e. the pre-fix
    // state); a rung at or below the frozen tip (a fetch the marshal discards);
    // a rung that is not `last(T+1)` for the LOCAL `T`.
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
            // `last_consensus` seeds BOTH `last_tip_height` and `probe_prev_tip`,
            // so the tip is 95 and frozen from tick one.
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

    // (4.2 А) THE LADDER IS THE REPETITION OF THE TICK: a step the marshal can act
    // on is put AGAIN on the next frozen tick, not once.
    //
    // §5.2 has no state machine here — "лестница = повторение того же шага". The
    // repetition is what makes an unserved step harmless (§5.4 "догон вместо
    // прыжка": the node keeps walking contiguously and asks again) and it is what
    // nothing pinned before this pass.
    //
    // Falsifier: an empty `hints` (the step never reached the marshal); a single
    // hint over two frozen ticks (the ladder became a one-shot).
    #[test]
    fn a_ladder_step_is_put_on_the_marshal_again_on_every_frozen_tick() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            const STEP: u64 = 191;
            let (re_jump, ticks) = scripted_probe(vec![ProbeOutcome {
                // A `Latest` answer ABOVE the step: the network has produced
                // `last(T+1)`, so `servable` holds (see `probe_frontier`). A
                // distinct height so the step's hints are told apart from the
                // untargeted frontier hint the same tick also puts.
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

    // (4.2 А, review A2-10) THE STEP IS JUDGED AGAINST THE MARSHAL FLOOR, NOT THE
    // TIP — and the difference is the hole a jumped node carries.
    //
    // The marshal drops `HintFinalized` when `height <= last_processed_height`
    // (`marshal/core/actor.rs:633-635`), so a step at or below the FLOOR is a
    // fetch nobody acts on and is skipped here. A step between the floor and the
    // tip is the opposite case: a node that jumped holds nothing in that range,
    // the marshal WILL fetch and store it, and gating on the tip — which is what
    // this code did — suppressed exactly those.
    //
    // RED before this change: the arm read `height <= self.last_tip_height`, so
    // the in-hole step at 160 was skipped and `hints` stayed empty.
    //
    // Falsifier: the in-hole step missing from `hints` (the tip is still the
    // gate); the at-floor step present (a fetch the marshal discards).
    #[test]
    fn a_ladder_step_is_skipped_at_the_marshal_floor_and_put_inside_the_hole() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 200;
            const FLOOR: u64 = 150;
            // Tip 200, floor 150: heights 151..=200 are the hole the jump left,
            // and the `Latest` witness is above both so `servable` never decides.
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
            // `last_consensus` seeds BOTH `last_tip_height` and `probe_prev_tip`,
            // so the tip is 200 and frozen from tick one.
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

    // (4.2 Б2 fix-1, B2-02) THE STARTUP DRAIN'S `None` ARM IS REACHABLE AND FATAL,
    // and the fatal is the END of #12's deferral, not a pause in it. The drain walks
    // `(reth's last block .. the marshal's acked cursor]`, and that cursor IS the
    // marshal's floor (`.claude/COMMONWARE_INTERNALS.md:190-193`), which the marshal
    // never repairs below — so a marshal that cannot serve a drained height will
    // never be able to, and the jump cannot cover for it either (`maybe_re_jump`
    // refuses to spawn while this drain is non-empty). The executor must therefore
    // DIE here rather than skip; the message it dies with names the floor and the
    // operator's two ways out (this test pins the death, `dispatch_fault`'s log
    // carries the text).
    //
    // NOT a SafetyHalt: nothing here says the NETWORK disagrees with this node —
    // only that this node's own two stores disagree.
    //
    // Falsifier: the actor still running after the drain hit a hole (the deferral
    // would then be silently permanent, with `dpos_sync_degraded{crash_recover}`
    // stuck raised); the fork-safety latch engaging.
    #[test]
    fn a_startup_drain_over_a_hole_the_marshal_cannot_repair_dies_loudly() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100; // reth's last block = the drain's lower bound
            const CURSOR: u64 = ANCHOR + 2; // the marshal's acked cursor = its floor
            let fx = Fixture::new(ANCHOR);
            // The marshal holds NOTHING at ANCHOR+1: `FakeMarshal::canned` is empty,
            // so the very first drained height comes back `None`.
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

    // STARTUP-BACKFILL FAST-FORWARD (the v33 fresh-spare freeze in miniature):
    // a fresh spare's `[last_execution+1 ..= last_consensus]`
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

            actor
                .reseed_forward(landing, landing_hash, floor)
                .await
                .expect("reseed_forward");

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
    // finalized result (the whole-committee divergence, re-entered through
    // restart). Pre-fix (floor = reth head) the assertion
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
            // SIBLING (≠ the re-derive), acked+N−K a distinct spec hash.
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
            actor
                .reseed_forward(landing, landing_hash, floor)
                .await
                .expect("reseed_forward");

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

    // (d''', review B1-04) THE LANDING CONTRADICTS AN AUTHENTICATED CERTIFICATE ⇒
    // `Fault::corruption`, AND NOTHING IS ROTATED.
    //
    // WHAT THIS TEST SAID BEFORE. It was `re_jump_invalid_target_rotates` and it
    // pinned the opposite: `rotations == 1` and a surviving loop, on the reasoning
    // that `InvalidTarget` is "the same NON-fatal bad-upstream treatment as
    // BadTarget". That reasoning belongs to the era when the jump target came from
    // `CertUpstream::get_latest` — an answer a peer chose. Since §5.2 the
    // steady-state target is the `(finalization, block)` pair this node read out of
    // its OWN marshal archive, written only by `store_finalization` after
    // `verify_delivered`, so there is no upstream to rotate away from: rotating
    // moves the fetch seam and leaves the contradiction standing. §5.4 files both
    // routes into this outcome — "Посадка не на заверенную ветку" and "reth
    // Invalid" — as `Fault::corruption`.
    //
    // RED before the fix (mutation: the arm restored to `self.rotate_upstream()`),
    // on `rotations == 0` and on the handle never resolving.
    //
    // Falsifier: a rotation (the node treats a local EL contradiction as somebody
    // else's fault); a surviving loop (it keeps driving reth after the EL
    // contradicted an authenticated certificate); no jump call at all (the fixture
    // stopped triggering and everything above is vacuous); a floor advance.
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
                // Yield so the spawned waiter completes + its `jump_done` arm runs.
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

    // Case (A) no-regression: while the tip is HELD (`awaiting_seed`), a
    // SHALLOW gap (≤ JUMP_THRESHOLD) must NOT start a re-jump — the hold
    // proceeds untouched. Only a deep gap (> JUMP_THRESHOLD) engages the
    // re-jump.
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

            // Beacon-active round with no σ in the store ⇒ the block is HELD.
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

    // THE SEED HOLD MUST NOT GATE `maybe_re_jump` (research B3): that gate is
    // what bounds a σ-less node's stall, so a block held for its σ must not
    // suppress the spawn. Pinned here as a test rather than a comment — adding
    // `awaiting_seed.is_none()` to `maybe_re_jump`'s five gates makes this fail.
    //
    // And (f′, Fix A) `reseed_forward` disposes the held block with
    // `acknowledge()` (Ok, NEVER a drop — a dropped `Exact` is a Canceled ack,
    // fatal to the marshal): the floor moves past the held height, so it is
    // pruned, not skipped.
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

            // The block is HELD (its σ never lands — the node is about to jump
            // far past it).
            let (m1, mut w1) =
                finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO));
            mailbox.send(m1).expect("send held block");
            ctx.sleep(Duration::from_millis(20)).await;
            assert!(
                (&mut w1).now_or_never().is_none(),
                "premise: the block is HELD (unacked) when the deep tip arrives"
            );

            // A DEEP frontier tip (gap > JUMP_THRESHOLD) while holding → re-jump spawns.
            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                .expect("send deep tip");

            // `wait_until` panics after 2000 virtual ms, so gating `maybe_re_jump`
            // on the hold FAILS here instead of hanging on the ack below.
            wait_until(&ctx, "the re-jump spawned despite the held block", || {
                !calls.lock().unwrap().is_empty()
            })
            .await;
            assert_eq!(
                *calls.lock().unwrap(),
                vec![ANCHOR],
                "re-jump SPAWNED once despite the held block (durably-stuck recovery)"
            );
            // The Landed reseed disposes the held block via `acknowledge()` → Ok.
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

    use metrics_util::{
        debugging::{DebugValue, DebuggingRecorder, Snapshotter},
        CompositeKey,
    };

    /// `Snapshotter::snapshot` RESETS every counter it reads, so a test takes
    /// exactly ONE snapshot and queries this drained copy.
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

    /// Total of an UNLABELLED counter — the shape both detector counters use.
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

    // THE DETECTOR, and the four things it must be. It (1) stays SILENT below the
    // threshold, (2) reports once the hold outlives it, (3) reports ONCE rather
    // than once per call, and (4) CHANGES NOTHING — the block is still held
    // afterwards and still derives the moment σ lands, which is the only correct
    // exit.
    //
    // Clause (2) is asserted twice, the second time AFTER a miss re-hold, which
    // is what pins that `HeldForSeed::since` rides through
    // `try_eager_finalized_derive`'s restore. Resetting it there would keep the
    // detector permanently silent under a stream of unrelated seed records — the
    // exact condition it exists to report.
    //
    // The hold is BACK-DATED rather than waited out: the deterministic runtime
    // advances virtual time in 1 ms cycles, so sleeping past a 60 s threshold
    // costs ~60 s of real time and would dominate the suite.
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

                // `drain_counters` DRAINS, as its name says: each read reports the
                // increments since the previous one, not a running total. Every
                // count below is therefore "reports since the last assertion".
                let reports = || counter_total(&drain_counters(&snap), NAME);

                // (1) A fresh hold is the ordinary record-vs-delivery race.
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

                // (2)+(3) Past the threshold: one report, however often it is asked.
                backdate(&mut actor);
                actor.detect_stalled_seed_hold();
                actor.detect_stalled_seed_hold();
                actor.detect_stalled_seed_hold();
                assert_eq!(reports(), 1, "one stall is one event — not one per call");

                // (2, again) A σ for an UNRELATED round fires the same re-attempt
                // the notify arm makes; it MISSES and puts the block straight back.
                // Clearing `reported` isolates the question to `since`: if the
                // restore reset it, the detector below would find a fresh hold.
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

                // (4) Nothing about the hold moved.
                assert!(
                    fx.beacon.new_payload_calls.lock().unwrap().is_empty(),
                    "the detector must not derive the held block"
                );
                assert!(
                    (&mut waiter).now_or_never().is_none(),
                    "the detector must not resolve the held ack"
                );
                assert!(!fx.safety_halt.is_engaged(), "a detector never halts");

                // ...and the only correct exit still works.
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

    // The wiring: the detector runs off the EXISTING FCU heartbeat tick and
    // nothing else. Deleting the call from that arm leaves every assertion above
    // green — this is the only test that fails.
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

                // Hold + back-date BEFORE the actor is spawned, so a couple of
                // ordinary heartbeat ticks are all the timeline this needs.
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

    // The same backfill hole as `backfill_hole_is_fatal_at_the_backfill_site`, seen
    // at the ROUTER: a bare `break` exits the loop WITHOUT reading the halt latch,
    // so an already-halted node would drop every retained marshal `Exact` into
    // Canceled (fatal to the marshal). The fault counter is the only observable
    // that separates "routed as a Corruption" from "the loop merely exited".
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

    // Teardown, not a fault: every sender dropped with the latch CLEAR is the node
    // shutting this executor down. It must exit, count itself as a clean exit
    // (the cause label is what tells an operator a stopped executor was torn down
    // rather than killed by a fault), and raise no fault.
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

    // The ONE case with the latch already engaged at start: `SafetyHalt::restore_marker`
    // re-engages from the datadir marker before any actor spawns, so NO fault ever
    // reaches the router — the router's own is-engaged check cannot fire, and without
    // the top-of-loop gate this actor would drive reth for a chain it has already
    // latched "I refuse".
    #[test]
    fn a_marker_restored_latch_parks_the_executor_before_it_drives_reth() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            fx.safety_halt.engage(SyncReason::ResultDivergence);
            let (actor, mailbox) = fx.build(ctx.clone(), ANCHOR, ANCHOR);
            let handle = actor.start();

            // A block plus the flush child its height needs to derive at all — so a
            // missing gate shows up as real EL traffic, not merely as a held tip.
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

    // The arm's latch check is defence in depth, not a window anyone can point at
    // today: the only production engage sites are the router (which parks and never
    // returns) and the startup marker restore (covered by the top-of-loop pre-class
    // gate). It exists because a latch engaged from outside this actor while the loop
    // sits in `select!` would otherwise be missed. A mailbox close in that state must
    // park HERE: the
    // loop's `break` returns from the task, dropping the held marshal `Exact` into
    // Canceled, which the marshal treats as fatal.
    #[test]
    fn mailbox_close_while_halted_parks_and_retains_acks() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            // Beacon-ACTIVE epoch + an EMPTY store, so the block below is HELD
            // and its ack is un-resolved and in the actor's hands when the latch
            // trips.
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

            // Mid-flight, with the loop parked in `select!` and no timer due to wake
            // it (the fixture's heartbeat is 60s): engage, then close.
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

    // bugs 6/7: while a jump is in flight (`jump_done` armed) the executor is the
    // SINGLE EL writer — NO finalize-derive and NO speculative execute may fire
    // (their FCUs would retarget reth's backfill, starving the jump's `Valid`
    // terminator). A finalize + a spec delivered during the jump must NOT derive;
    // once the jump completes (a no-op `Lagging` landing here) the queued finalize
    // drains.
    #[test]
    fn no_derive_or_spec_while_jump_in_flight_then_drains() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            // A re-jump waiter that HANGS until `release`, then lands as a no-op
            // (`Lagging` — no floor change, so the queued ANCHOR+1 stays derivable).
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

    // REAL-marshal SafetyHalt liveness.
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
                .map(|kp| build_signer(&ns, bimap.clone(), kp, 0, None).expect("member"))
                .collect();
            let verifier = build_verifier(&ns, bimap, 0, None);
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
                        metrics: ExecutorMetrics::default(),
                        sync_metrics: fx.sync_metrics.clone(),
                        safety_halt: fx.safety_halt.clone(),
                        spawn_unblocked: std::sync::Arc::new(tokio::sync::Notify::new()),
                        re_jump: None,
                        // Deliberately the negative provider: every height here
                        // sits in epoch 0 under the epocher below, which is
                        // beacon-INACTIVE, so the derive resolves `None` and can
                        // never hold. This is the only site driving a REAL
                        // marshal, and a σ at epoch 0 would model a state
                        // production cannot reach.
                        randomness: crate::beacon::absent_unregistered(),
                        epocher: crate::epocher::OriginEpocher::new(
                            0,
                            std::num::NonZeroU64::new(1 << 40).expect("nonzero"),
                        ),
                        anchor_advanced: std::sync::Arc::new(|| {}),
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
                // forged, so the executor's cross-check halts at K. Each height
                // derives at its OWN dispatch (this actor's epocher puts every
                // height in epoch 0, which is beacon-INACTIVE, so σ is `None` and
                // nothing is ever held), which is when K's forged cross-check
                // fires. K+1 is dispatched below to prove its ack is RETAINED by
                // the park rather than dropped.
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

                // K's own dispatch engages the halt; K+1 is dispatched into the
                // halted executor, which parks retaining both acks.
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
                    .await
                    .expect("reseed_forward");

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
                    .await
                    .expect("reseed_forward");

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
                    .await
                    .expect("reseed_forward");

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
                    .await
                    .expect("reseed_forward");

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
                    .await
                    .expect("reseed_forward");

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

    // The wake-up arm must not spin on a dead beacon. A `broadcast` receiver whose
    // senders are all gone answers `Closed` IMMEDIATELY and FOREVER, so an arm that
    // treated `Closed` like any other wake-up would re-poll without awaiting for as
    // long as a tip is HELD — a hot loop in exactly the incident where the node is
    // already degraded. The HEAD shape parked on a `Notify` whose sender the actor
    // itself held, so the case could not arise; `Disarm` is what replaces that park.
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
        // The spin premise, asserted rather than assumed: with the sender gone the
        // receiver is ready on every poll, twice in a row and without awaiting.
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

    // The other two classes are wake-ups for OTHER consumers of the same channel:
    // no derive, and the arm stays armed.
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
