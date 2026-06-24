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
//! Ack flow: the marshal's `Exact` ack fires only after derive + import, so
//! marshal backpressure (MAX_PENDING_ACKS) IS execution backpressure.

// **** давай схлопним 3 файла в 1

use crate::{
    application::{BeaconEngineLike, DerivedBlock as _, DerivedBlockBuilder, ExecutedChain},
    order_block::OrderBlock,
};
use alloy_primitives::B256;
use alloy_rpc_types_engine::ForkchoiceState;
use commonware_consensus::{marshal::Update, types::Height};
use commonware_runtime::{
    spawn_cell, Clock, ContextCell, FutureExt as _, Handle, Metrics as _, Pacer, Spawner,
};
use commonware_utils::{
    acknowledgement::Exact, channel::oneshot, futures::OptionFuture, vec::NonEmptyVec,
    Acknowledgement as _,
};
use eyre::{ensure, WrapErr as _};
use fluentbase_bls::PeerPubkey;
use futures::{
    future::{ready, BoxFuture, Ready},
    stream::FuturesOrdered,
    FutureExt as _, StreamExt as _,
};
use prometheus_client::metrics::gauge::Gauge;
use std::{collections::BTreeMap, ops::RangeInclusive, pin::Pin, time::Duration};
use tokio::{select, sync::mpsc};
use tracing::{
    debug, error, error_span, info, info_span, instrument, warn, warn_span, Level, Span,
};

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

/// Payload of [`Command::SpecNotarized`]: the ordering digest + round + the
/// seed recovered from the Notarization certificate. The block body is fetched
/// from the marshal by digest at execution time.
pub struct Notarized {
    pub round: commonware_consensus::types::Round,
    pub digest: crate::digest::Digest,
    pub seed: Option<crate::beacon::seed::Seed>,
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

/// Outcome of looking up the beacon seed for a finalized height.
pub enum SeedLookup {
    /// The finalization cert is not local yet — it exists in the network (the
    /// block IS finalized) and must be fetched before deriving; deriving with
    /// the fallback here would fork the chain. The caller defers + re-polls.
    CertMissing,
    /// The cert is present and carries no seed — a genuine no-beacon epoch; the
    /// gated `order.digest()` fallback is correct and agreed across nodes.
    NoBeacon,
    /// The cert is present and carries the round's threshold seed.
    Seed(crate::beacon::seed::Seed),
}

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

    /// 3-way seed lookup for a finalized height (see [`SeedLookup`]).
    /// Distinguishes "cert not local yet" (must fetch — never fall back) from
    /// "no-beacon epoch" (agreed fallback) from a present seed.
    fn lookup_seed(&self, height: Height) -> impl std::future::Future<Output = SeedLookup> + Send;

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

    async fn lookup_seed(&self, height: Height) -> SeedLookup {
        match self.get_finalization(height).await {
            None => SeedLookup::CertMissing,
            Some(fin) => match fin.certificate.seed() {
                Some(signature) => SeedLookup::Seed(crate::beacon::seed::Seed {
                    target_round: fin.proposal.round,
                    signature,
                }),
                None => SeedLookup::NoBeacon,
            },
        }
    }

    async fn hint_finalization(&self, height: Height, targets: NonEmptyVec<PeerPubkey>) {
        self.hint_finalized(height, targets).await;
    }

    async fn set_floor(&self, height: Height) {
        commonware_consensus::marshal::core::Mailbox::set_floor(self, height).await;
    }
}

/// Self-driven re-poll cadence while awaiting a not-yet-local finalization cert.
const SEED_FETCH_POLL: Duration = Duration::from_millis(200);
/// Total budget for a deferred block's cert to arrive before failing loud.
const SEED_FETCH_MAX_WAIT: Duration = Duration::from_secs(30);

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
/// stall (re-evaluated on the next `Update::Tip`); `AuthFailed` ⇒ fail-closed (a
/// forged far-ahead target fails `verify_jump_authenticated`, the executor loop
/// breaks and the node refuses to serve).
pub type ReJump = std::sync::Arc<
    dyn Fn(u64) -> BoxFuture<'static, crate::cold_start_jump::JumpOutcome> + Send + Sync,
>;

/// A finalized block held while its beacon seed (finalization cert) is not local
/// yet. The `pending_finalizations` drain is paused while this is `Some`, which
/// preserves strict derive order and lets the marshal's `MAX_PENDING_ACKS`
/// backpressure bound the queue.
struct Deferred {
    cause: Span,
    order: OrderBlock,
    ack: Exact,
    deadline: std::time::SystemTime,
}

/// Result of attempting to derive a finalized block.
enum DeriveOutcome {
    /// Derived + imported + FCU'd + acked.
    Done,
    /// A required finalization cert (for `height`) is not local yet; the block
    /// (with its ack) is handed back to be deferred + re-polled. A hint has
    /// already been issued for `height`. `order` is boxed to keep this enum
    /// small (the `Done` arm is the hot path).
    NeedSeed {
        cause: Span,
        order: Box<OrderBlock>,
        ack: Exact,
        height: Height,
    },
}

/// Seed resolution for one height: either a usable seed (`Some` = beacon,
/// `None` = agreed no-beacon fallback) or "cert not local — fetch it".
enum SeedOr {
    Seed(Option<crate::beacon::seed::Seed>),
    Need(Height),
}

/// Result of the gap prefix-derive: the hash at `target`, or a missing cert.
enum PrefixResult {
    Hash(B256),
    NeedSeed(Height),
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
    /// Beacon counters (cross-epoch singleton from `dpos.rs::launch`). The
    /// executor increments `seed_active` / `digest_fallback` per derived block.
    pub beacon_metrics: crate::beacon::metrics::BeaconMetrics,
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
    spawn_unblocked: std::sync::Arc<tokio::sync::Notify>,
    /// Steady-state self-healing re-jump callback (see [`ReJump`]). Fired from
    /// the `Update::Tip` arm when the frontier runs > `JUMP_THRESHOLD` ahead of
    /// `ordering_finalized`.
    re_jump: Option<ReJump>,
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

    last_canonicalized: LastCanonicalized,
    /// Highest ordering-finalized height processed; drives the result-final
    /// cursor (`− K`, clamped to the anchor).
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

    finalized_heights_to_backfill: RangeInclusive<u64>,
    pending_backfill: OptionFuture<BoxFuture<'static, (u64, Option<OrderBlock>)>>,
    pending_finalizations: FuturesOrdered<Ready<(Span, OrderBlock, Exact)>>,

    /// Ops-visibility gauge for `pending_finalizations.len()`. Alert on
    /// sustained values > 4 — indicates EL is falling behind consensus
    /// (`MAX_PENDING_ACKS = 16` is the marshal-side ceiling).
    pending_finalizations_gauge: Gauge<i64>,

    /// Heartbeat FCUs are suppressed until consensus advances from the
    /// cold-start snapshot, so a stale initial head is never re-sent over a
    /// canonical chain that moved without us.
    has_advanced_since_init: bool,

    /// Highest height the executor has imported (speculatively OR finalized).
    /// Speculation only fires for `spec_head + 1`, and is tracked here rather
    /// than via `executed_tip()` to avoid reth's `best_number` lag race.
    spec_head: u64,
    /// Heights speculatively executed at notarization but not yet finalized:
    /// height → the notarized ordering digest. On finalized delivery a digest
    /// match means the speculation was correct (skip re-derive, keep the head
    /// lead); a mismatch (notarized-then-nullified, sibling finalized) forces a
    /// re-derive + head reorg back onto the finalized fork.
    spec_executed: BTreeMap<u64, crate::digest::Digest>,

    peers_for_finalization: PeersForFinalization,
    /// A finalized block whose cert (seed) is not local yet; held with its ack
    /// while `deferred_timer` re-polls. The `pending_finalizations` drain is
    /// paused while this is `Some` (preserves order).
    deferred: Option<Deferred>,
    /// Self-driven (NOT delivery-driven) re-poll timer; armed only while
    /// `deferred` is `Some`. Delivery-driven retry would deadlock on the last
    /// catch-up block (see research addendum).
    deferred_timer: OptionFuture<BoxFuture<'static, ()>>,
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

        let finalized_heights_to_backfill =
            (cfg.last_execution_finalized_height + 1)..=cfg.last_consensus_finalized_height.get();

        let pending_finalizations_gauge = Gauge::<i64>::default();
        context.register(
            "pending_finalizations",
            "Count of finalized blocks awaiting derive+import+ack in the executor queue \
             (MAX_PENDING_ACKS=16 marshal-side ceiling).",
            pending_finalizations_gauge.clone(),
        );

        let actor = Self {
            context: ContextCell::new(context),
            beacon_engine: cfg.beacon_engine,
            deriver: cfg.deriver,
            executed: cfg.executed,
            marshal: cfg.marshal,
            mailbox: rx,
            beacon_metrics: cfg.beacon_metrics,
            spawn_unblocked: cfg.spawn_unblocked,
            re_jump: cfg.re_jump,
            jump_done: OptionFuture::default(),
            jump_handle: None,
            // Best estimate of the marshal frontier at startup; refined by every
            // `Update::Tip`. Drives the heartbeat re-poke (see field doc).
            last_tip_height: cfg.last_consensus_finalized_height,
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
            ordering_finalized: cfg.last_execution_finalized_height,
            anchor_finalized: cfg.initial_finalized,
            dpos_activation_block: cfg.dpos_activation_block,
            fcu_heartbeat_interval: cfg.fcu_heartbeat_interval,
            fcu_heartbeat_timer,
            fcu_pace: cfg.fcu_pace,
            finalized_heights_to_backfill,
            pending_backfill: OptionFuture::default(),
            pending_finalizations: FuturesOrdered::new(),
            pending_finalizations_gauge,
            has_advanced_since_init: false,
            spec_head: cfg.initial_head.0.get(),
            spec_executed: BTreeMap::new(),
            peers_for_finalization: cfg.peers_for_finalization,
            deferred: None,
            deferred_timer: OptionFuture::default(),
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

        loop {
            // Do not pull more work while a block is deferred awaiting its cert
            // — the deferred block must derive first (strict order).
            if self.deferred.is_none() && self.pending_backfill.is_none() {
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
                            let (ack, _waiter) = Exact::handle();
                            let span = info_span!("backfill_on_start", %height);
                            match self.try_derive(span, block, ack).await {
                                Ok(outcome) => self.defer_if_needed(outcome, None),
                                Err(error) => {
                                    error_span!("shutdown").in_scope(|| error!(%error,
                                        "executor fatal error during backfill; shutting down"));
                                    break;
                                }
                            }
                        }
                        None => {
                            warn_span!("backfill_on_start", %height).in_scope(||
                                warn!("marshal did not have block in backfill range"));
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
                        }
                        Ok(crate::cold_start_jump::JumpOutcome::Lagging) => {
                            debug!("steady-state re-jump: lagging / stale target — no-op");
                        }
                        Ok(crate::cold_start_jump::JumpOutcome::Stalled(error)) => {
                            // NON-fatal: the gap is re-evaluated on the next
                            // `Update::Tip` (which re-calls `maybe_re_jump` while
                            // no jump is in flight). THIS is the transient-stall
                            // responsiveness fix — a stall no longer crashes the
                            // executor.
                            warn!(%error, "steady-state re-jump stalled (transport); \
                                will re-evaluate the gap on the next frontier tip");
                        }
                        Ok(crate::cold_start_jump::JumpOutcome::AuthFailed(error)) => {
                            // Fail-closed: a forged far-ahead target fails
                            // `verify_jump_authenticated` (exactly like the
                            // cold-start `?`-abort); refuse to serve.
                            error_span!("shutdown").in_scope(|| error!(%error,
                                "steady-state re-jump failed authentication (fail-closed); \
                                 shutting down"));
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
                    && self.finalized_heights_to_backfill.is_empty() => {
                    self.pending_finalizations_gauge
                        .set(self.pending_finalizations.len() as i64);
                    match self.try_derive(cause, block, ack).await {
                        Ok(outcome) => self.defer_if_needed(outcome, None),
                        Err(error) => {
                            error_span!("shutdown").in_scope(|| error!(%error,
                                "executor fatal error during finalize; shutting down"));
                            break;
                        }
                    }
                }

                // Self-driven re-poll of a deferred block's cert. OptionFuture
                // does NOT auto-clear after Poll::Ready (cf. pending_backfill) —
                // clear it here; re-arm only if still deferred. Delivery-driven
                // retry is avoided on purpose (it deadlocks at the last block).
                () = &mut self.deferred_timer => {
                    *self.deferred_timer = None;
                    let d = self.deferred.take().expect("timer armed ⇒ deferred set");
                    if self.context.current() >= d.deadline {
                        error_span!("shutdown").in_scope(|| error!(height = %d.order.height,
                            "finalization cert unavailable within budget; cannot derive beacon \
                             prev_randao without diverging; shutting down"));
                        break;
                    }
                    match self.try_derive(d.cause, d.order, d.ack).await {
                        Ok(outcome) => self.defer_if_needed(outcome, Some(d.deadline)),
                        Err(error) => {
                            error_span!("shutdown").in_scope(|| error!(%error,
                                "executor fatal error during deferred derive; shutting down"));
                            break;
                        }
                    }
                }

                msg = self.mailbox.recv() => {
                    let Some(msg) = msg else { break; };
                    if let Err(error) = self.handle_message(msg).await {
                        error_span!("shutdown").in_scope(|| error!(%error,
                            "executor fatal error handling message; shutting down"));
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
                    // on the gap / in-flight / deferred, so it is a no-op whenever
                    // the node is not actually behind. (Cannot error.)
                    let _ = self.maybe_re_jump(self.last_tip_height).await;
                    self.reset_fcu_heartbeat_timer();
                }
            }
        }

        // Cancel the read-only re-jump waiter on shutdown (mirror of the
        // subsystem aborts in `outer.rs`) so a spawned `sync_to` wait does not
        // outlive the executor task. All `break`s converge here.
        if let Some(handle) = self.jump_handle.take() {
            handle.abort();
        }
    }

    /// Stash a `NeedSeed` outcome into the deferred slot + arm the self-driven
    /// re-poll timer; a `Done` outcome is a no-op. `deadline` is `None` for a
    /// freshly-popped block (start the budget now) or `Some` to PRESERVE the
    /// original budget across a re-poll.
    fn defer_if_needed(&mut self, outcome: DeriveOutcome, deadline: Option<std::time::SystemTime>) {
        if let DeriveOutcome::NeedSeed {
            cause,
            order,
            ack,
            height,
        } = outcome
        {
            debug!(%height, "finalization cert not local yet; deferring derive + hinting peers");
            let deadline = deadline.unwrap_or_else(|| self.context.current() + SEED_FETCH_MAX_WAIT);
            self.deferred = Some(Deferred {
                cause,
                order: *order,
                ack,
                deadline,
            });
            self.arm_deferred_timer();
        }
    }

    fn reset_fcu_heartbeat_timer(&mut self) {
        self.fcu_heartbeat_timer = Box::pin(self.context.sleep(self.fcu_heartbeat_interval));
    }

    #[instrument(skip_all)]
    async fn send_forkchoice_update_heartbeat(&mut self) {
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
        if let Err(error) = resp {
            warn!(%error, "heartbeat FCU failed");
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
                }
                Update::Block(block, ack) => {
                    self.pending_finalizations
                        .push_back(ready((cause, block, ack)));
                    self.pending_finalizations_gauge
                        .set(self.pending_finalizations.len() as i64);
                }
            },
            Command::SpecNotarized(n) => {
                let Notarized {
                    round,
                    digest,
                    seed,
                } = *n;
                if let Err(error) = self.spec_execute(cause, round, digest, seed).await {
                    // Speculation is best-effort: a failure here is logged, never
                    // fatal — `try_derive` (finalized path) will derive the block at
                    // finalization regardless.
                    warn!(%error, %digest, "speculative execution skipped");
                }
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
    /// `Some` — never spawn a second), a gap ≤ `JUMP_THRESHOLD`, or a deferred
    /// block (don't start a jump while a block is paused awaiting its cert)
    /// all early-return without spawning.
    async fn maybe_re_jump(&mut self, height: Height) -> eyre::Result<()> {
        let Some(re_jump) = self.re_jump.clone() else {
            return Ok(());
        };
        // A jump is already in flight — don't spawn a second.
        if self.jump_done.is_some() {
            return Ok(());
        }
        if height.get().saturating_sub(self.ordering_finalized)
            <= crate::cold_start_jump::JUMP_THRESHOLD
            // Don't start a jump while a block is deferred awaiting its cert
            // (the reseed prunes below the floor; a deferred sub-floor block
            // would be reconciled mid-flight). Wait for the deferred drain.
            || self.deferred.is_some()
        {
            return Ok(());
        }
        info!(
            tip = %height,
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
                let _ = tx.send(re_jump(from).await);
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
    async fn reseed_forward(&mut self, landing_h: u64, landing_hash: B256, floor: u64) {
        info!(landing_h, floor, "steady-state re-jump landed; re-seeding executor + marshal floor");
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
        if let Some(floor_hash) = self.executed.executed_hash(floor) {
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
                warn!(%error, "reseed_forward canonicalization FCU failed");
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
        // STALE-SPEC FIX: the speculative tip / map are stale across a deep jump
        // (their heights are far below the landing). Raise `spec_head` to the
        // landing and drop spec entries at/below it so the next notarization
        // re-speculates forward from the landing.
        self.spec_head = self.spec_head.max(landing_h);
        self.spec_executed = self.spec_executed.split_off(&(landing_h + 1));
        self.has_advanced_since_init = true;
        // Advance the RUNNING marshal floor (raises-only; prunes below; resumes
        // contiguous dispatch from `floor + 1`).
        self.marshal.set_floor(Height::new(floor)).await;
        // DEFERRED reconciliation: a block deferred BELOW the new floor would, on
        // its next re-poll, `try_derive` into a now-pruned gap — drop it + disarm
        // its timer. (`pending_finalizations` below-floor entries self-reconcile
        // via the gap-walk on pop; only `deferred` pauses the drain.)
        if let Some(d) = &self.deferred {
            if d.order.height < floor {
                self.deferred = None;
                *self.deferred_timer = None;
            }
        }
    }

    /// Speculatively derive + import a NOTARIZED block, advancing the EL head
    /// ahead of finalization. Strictly forward-only (`spec_head + 1`); a gap or
    /// an already-covered height is left to `try_derive` (finalized path), which keeps
    /// this path race-free with finalized delivery (both run in this one loop).
    #[instrument(skip_all, parent = &cause, fields(%digest), err(level = Level::DEBUG))]
    async fn spec_execute(
        &mut self,
        cause: Span,
        _round: commonware_consensus::types::Round,
        digest: crate::digest::Digest,
        seed: Option<crate::beacon::seed::Seed>,
    ) -> eyre::Result<()> {
        // A finalized block is deferred awaiting its cert (strict-order pause).
        // Speculating past it would advance head/spec_head OVER the deferred
        // height, leaking the strict-order invariant (self-healing, but the
        // finalized path is the sole authority — let it derive first). The
        // mailbox arm is intentionally NOT gated (shutdown + Command::Finalize
        // enqueue must keep flowing); the guard lives here.
        if self.deferred.is_some() {
            return Ok(());
        }
        let Some(order) = self.marshal.fetch_block_by_digest(digest).await else {
            // Body not in the local buffer yet — finalized path will derive it.
            return Ok(());
        };
        let height = order.height;
        // Only speculate the immediate next block. A higher height (gap) or a
        // height at/below the tip (re-notarization, already executed) is the
        // finalized path's job.
        if height != self.spec_head + 1 {
            return Ok(());
        }
        let parent_height = height
            .checked_sub(1)
            .ok_or_else(|| eyre::eyre!("speculative height 0"))?;
        // Parent must be locally present; a transient miss (reth visibility
        // lag) just defers to the finalized path.
        let Some(parent_hash) = self.executed.executed_hash(parent_height) else {
            return Ok(());
        };

        let derived = self
            .deriver
            .derive_and_execute(order, parent_hash, seed)
            .await
            .wrap_err("speculative derive_and_execute failed")?;
        let derived_hash = derived.evm_hash();
        self.submit_finalized_payload(derived).await?;

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
        self.spec_executed.insert(height, digest);
        Ok(())
    }

    /// Resolve the seed for a finalized height. `CertMissing` ⇒ hint the peers
    /// and signal `Need` so the caller defers (never derive with the fallback on
    /// a beacon chain — that forks).
    async fn seed_or_need(&mut self, height: Height) -> SeedOr {
        match self.marshal.lookup_seed(height).await {
            SeedLookup::Seed(s) => SeedOr::Seed(Some(s)),
            SeedLookup::NoBeacon => SeedOr::Seed(None),
            SeedLookup::CertMissing => {
                if let Some(targets) = (self.peers_for_finalization)() {
                    self.marshal.hint_finalization(height, targets).await;
                }
                SeedOr::Need(height)
            }
        }
    }

    fn arm_deferred_timer(&mut self) {
        self.deferred_timer
            .replace(self.context.sleep(SEED_FETCH_POLL).boxed());
    }

    /// Derive + import + FCU + ack a finalized block IF every cert it needs is
    /// local. The moment a required cert is missing it returns `NeedSeed` —
    /// WITHOUT mutating any finalized state or acking — so the caller can defer
    /// and re-poll on the self-driven timer.
    #[instrument(skip_all, parent = &cause, fields(height = order.height), err)]
    async fn try_derive(
        &mut self,
        cause: Span,
        order: OrderBlock,
        ack: Exact,
    ) -> eyre::Result<DeriveOutcome> {
        let height = order.height;
        // Captured before `order` is consumed by `derive_and_execute` below; the
        // attested result commits `executed_hash(height − K)`, cross-checked after
        // the derive lands.
        let attested_result = order.result;
        let parent_height = height
            .checked_sub(1)
            .ok_or_else(|| eyre::eyre!("ordering height 0 cannot be finalized"))?;

        // Reconcile against speculation: if this height was speculatively
        // executed with the SAME ordering block, reth is already canonical here
        // — skip the re-derive and, crucially, do NOT roll the head back (the
        // speculative lead at `height+1..` must survive). Otherwise (first
        // execution, or the speculation was a sibling that got nullified)
        // derive the finalized block and reorg the head onto it.
        let correctly_speculated = self
            .spec_executed
            .get(&height)
            .is_some_and(|d| *d == order.digest())
            && self.executed.executed_hash(height).is_some();

        let derived_hash = if correctly_speculated {
            // Already derived via spec_execute with the Notarization-cert seed —
            // needs NO finalization seed, so it never defers (the live path).
            self.executed
                .executed_hash(height)
                .expect("checked is_some above")
        } else {
            let parent_hash = match self.executed.executed_hash(parent_height) {
                Some(hash) => hash,
                // The marshal can hold finalized artifacts the EL hasn't derived
                // yet (restart with an unflushed reth tail; repair landing ahead
                // of dispatch). Derivation is strictly sequential, so walk the
                // missing prefix out of the marshal and derive it first; a
                // genuinely unfillable gap stays fatal (visible, not wedged).
                None => match self.derive_missing_prefix(parent_height).await? {
                    PrefixResult::Hash(hash) => hash,
                    PrefixResult::NeedSeed(h) => {
                        return Ok(DeriveOutcome::NeedSeed {
                            cause,
                            order: Box::new(order),
                            ack,
                            height: h,
                        })
                    }
                },
            };

            // The seed MUST be the cert's — a missing cert defers (never the
            // fallback) so a beacon-active chain cannot fork on catch-up.
            let seed = match self.seed_or_need(Height::new(height)).await {
                SeedOr::Seed(s) => s,
                SeedOr::Need(h) => {
                    return Ok(DeriveOutcome::NeedSeed {
                        cause,
                        order: Box::new(order),
                        ack,
                        height: h,
                    })
                }
            };
            let derived = self
                .deriver
                .derive_and_execute(order, parent_hash, seed)
                .await
                .wrap_err("derive_and_execute failed")?;
            let derived_hash = derived.evm_hash();
            self.submit_finalized_payload(derived).await?;
            derived_hash
        };

        // The finalized fork is now canonical at `height`. Any speculation
        // above it that built on a now-orphaned sibling is invalid; reset the
        // speculative tip so the next notarization re-speculates forward. A
        // correct speculation keeps its lead.
        if correctly_speculated {
            self.spec_head = self.spec_head.max(height);
        } else {
            self.spec_head = height;
        }
        self.spec_executed = self.spec_executed.split_off(&(height + 1));

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
            |h| self.executed.executed_hash(h),
        ) {
            return Err(eyre::eyre!(
                "result divergence at height {height}: attested result {attested_result:?} != \
                 local executed_hash; refusing to serve a forked chain"
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
            match self.executed.executed_hash(result_final) {
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

        let fcu = self
            .beacon_engine
            .fork_choice_updated(new.forkchoice)
            .pace(&self.context, self.fcu_pace)
            .await
            .wrap_err("finalize FCU failed")?;
        ensure!(
            fcu.is_valid() || fcu.is_syncing(),
            "EL reported non-valid finalize FCU: {:?}",
            fcu.payload_status
        );
        if new != self.last_canonicalized {
            self.has_advanced_since_init = true;
        }
        self.last_canonicalized = new;
        self.reset_fcu_heartbeat_timer();

        ack.acknowledge();
        Ok(DeriveOutcome::Done)
    }

    /// Derive the missing `..=target` prefix from the marshal's archive:
    /// probe backward to the highest executed ancestor, then fetch + derive +
    /// import forward. Returns the derived hash AT `target`, or `NeedSeed(h)`
    /// when a gap height's finalization cert is not local yet (the whole op then
    /// defers; the re-walk on retry is idempotent — already-derived prefix
    /// heights advance `first_missing`).
    async fn derive_missing_prefix(&mut self, target: u64) -> eyre::Result<PrefixResult> {
        let mut first_missing = target;
        let mut parent_hash = loop {
            if first_missing == 0 {
                return Err(eyre::eyre!(
                    "derive gap reaches height 0 — no executed ancestor"
                ));
            }
            if let Some(hash) = self.executed.executed_hash(first_missing - 1) {
                break hash;
            }
            first_missing -= 1;
        };
        info!(
            first_missing,
            target, "deriving missing prefix from marshal before the delivered block"
        );
        // Up-front hint burst over the whole gap range (marshal skips heights it
        // already holds) so the certs land in parallel rather than one per retry.
        // The committee is fixed within a single walk, so resolve the targets once.
        if let Some(targets) = (self.peers_for_finalization)() {
            for h in first_missing..=target {
                self.marshal
                    .hint_finalization(Height::new(h), targets.clone())
                    .await;
            }
        }
        for h in first_missing..=target {
            let seed = match self.seed_or_need(Height::new(h)).await {
                SeedOr::Seed(s) => s,
                SeedOr::Need(h) => return Ok(PrefixResult::NeedSeed(h)),
            };
            let order = self
                .marshal
                .fetch_block_by_height(Height::new(h))
                .await
                .ok_or_else(|| {
                    eyre::eyre!("derive gap: marshal has no ordering artifact at height {h}")
                })?;
            // Captured before `order` is consumed: each gap block carries its OWN
            // committee-attested `result` commitment, which must be cross-checked
            // exactly like the top-level delivered block — otherwise a wrong
            // `result` on a gap-range block (the byzantine-vrf defense) would be
            // imported unchecked.
            let attested_result = order.result;
            let derived = self
                .deriver
                .derive_and_execute(order, parent_hash, seed)
                .await
                .wrap_err_with(|| format!("gap derivation failed at height {h}"))?;
            parent_hash = derived.evm_hash();
            self.submit_finalized_payload(derived).await?;
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
                |q| self.executed.executed_hash(q),
            ) {
                return Err(eyre::eyre!(
                    "result divergence at gap height {h}: attested result {attested_result:?} != \
                     local executed_hash; refusing to serve a forked chain"
                ));
            }
        }
        Ok(PrefixResult::Hash(parent_hash))
    }

    /// Import the derived block into the EL. VALID is the expected steady
    /// state (single-execution insert acks synthetically; the new_payload
    /// fallback re-executes a block whose parent was derived one iteration
    /// ago); SYNCING is tolerated for the cold-start/rejoin window. Only a
    /// genuinely INVALID status is fatal — under the fallback it means local
    /// derivation diverged from reth's re-execution.
    async fn submit_finalized_payload(&mut self, derived: D::Derived) -> eyre::Result<()> {
        // Single chokepoint for all three derive paths (spec / finalized / gap):
        // record this block's beacon outcome before the value is moved into the EL.
        match derived.beacon_active() {
            Some(true) => self.beacon_metrics.seed_active.inc(),
            Some(false) => self.beacon_metrics.digest_fallback.inc(),
            None => 0,
        };
        let status = self
            .beacon_engine
            .import_derived(derived)
            .pace(&self.context, self.fcu_pace)
            .await
            .wrap_err("import_derived failed")?;
        ensure!(
            status.is_valid() || status.is_syncing(),
            "EL rejected derived block (local derivation diverged?): `{status:?}`"
        );
        Ok(())
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
            timestamp: 1_700_000_000 + height,
            fee_recipient: Address::ZERO,
            gas_limit: 30_000_000,
            extra_data: Bytes::new(),
            result,
            txs: Vec::new(),
            beacon_outcome: None,
            beacon_seed: None,
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
        vis: ByHashVisibility,
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
        fn executed_hash(&self, height: u64) -> Option<B256> {
            self.canonical.lock().unwrap().get(&height).copied()
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
            let sealed = sealed_at(parent_evm_hash, order.height, order.digest().0);
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
        /// Override for the `import_derived` status; `None` ⇒ Valid.
        import_status: Arc<Mutex<Option<PayloadStatusEnum>>>,
        /// By-hash visibility shared with `FakeChain`/`FakeDeriver`: an FCU
        /// canonicalizes `[.., head]` by hash (the visibility model). Default-disabled.
        vis: ByHashVisibility,
    }

    impl BeaconEngineLike for FakeBeacon {
        type ExecutionData = RethExecBlock;

        async fn fork_choice_updated(
            &self,
            state: ForkchoiceState,
        ) -> eyre::Result<ForkchoiceUpdated> {
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

        async fn import_derived(&self, data: RethExecBlock) -> eyre::Result<PayloadStatus> {
            self.new_payload_calls.lock().unwrap().push(data);
            let status = self
                .import_status
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(PayloadStatusEnum::Valid);
            Ok(PayloadStatus::from_status(status))
        }
    }

    /// Scripts how `lookup_seed` behaves in a test.
    #[derive(Clone, Default, PartialEq)]
    enum SeedMode {
        /// Every height is a no-beacon epoch → gated `order.digest()` fallback
        /// (the default; matches the pre-beacon behaviour the non-seed tests rely on).
        #[default]
        NoBeacon,
        /// Every height's cert is permanently missing (fail-loud test).
        AlwaysMissing,
        /// A height's cert is missing until it has been `hint_finalization`ed,
        /// then resolves (models a peer serving the re-fetch). Resolves to
        /// `NoBeacon` — the executor control flow (CertMissing → defer → hint →
        /// resolve → derive) is identical to a real seed; the `Some`-vs-`None`
        /// seed VALUE is only observable in `derive.rs::resolve_prev_randao`,
        /// which is tested there + by the smoke.
        MissingUntilHinted,
        /// Every height's cert carries this real beacon seed — `SeedLookup::Seed`
        /// flows through the executor into the deriver.
        Beacon(crate::beacon::seed::Seed),
    }

    #[derive(Clone, Default)]
    struct FakeMarshal {
        canned: Arc<Mutex<BTreeMap<u64, OrderBlock>>>,
        seed_mode: Arc<Mutex<SeedMode>>,
        /// Heights passed to `hint_finalization`, in call order.
        hints: Arc<Mutex<Vec<u64>>>,
        /// Heights passed to `set_floor`, in call order (the re-jump recorder).
        floors: Arc<Mutex<Vec<u64>>>,
    }

    impl BlockFetcher for FakeMarshal {
        async fn fetch_block_by_height(&self, height: Height) -> Option<OrderBlock> {
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
        async fn lookup_seed(&self, height: Height) -> SeedLookup {
            match &*self.seed_mode.lock().unwrap() {
                SeedMode::NoBeacon => SeedLookup::NoBeacon,
                SeedMode::AlwaysMissing => SeedLookup::CertMissing,
                SeedMode::MissingUntilHinted => {
                    if self.hints.lock().unwrap().contains(&height.get()) {
                        SeedLookup::NoBeacon
                    } else {
                        SeedLookup::CertMissing
                    }
                }
                SeedMode::Beacon(seed) => SeedLookup::Seed(seed.clone()),
            }
        }
        async fn hint_finalization(&self, height: Height, _targets: NonEmptyVec<PeerPubkey>) {
            self.hints.lock().unwrap().push(height.get());
        }
        async fn set_floor(&self, height: Height) {
            self.floors.lock().unwrap().push(height.get());
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
            Self {
                deriver: FakeDeriver::new(chain.clone()),
                chain,
                beacon,
                marshal: FakeMarshal::default(),
                anchor_hash,
                re_jump: Arc::new(Mutex::new(None)),
            }
        }

        /// Inject the steady-state re-jump callback the built actor's `Config`
        /// will carry. Set BEFORE `build`.
        fn with_re_jump(self, re_jump: ReJump) -> Self {
            *self.re_jump.lock().unwrap() = Some(re_jump);
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
                .executed_hash(anchor_height)
                .expect("anchor must be canonical");
            Actor::init(
                ctx,
                Config {
                    beacon_engine: self.beacon.clone(),
                    deriver: self.deriver.clone(),
                    executed: self.chain.clone(),
                    marshal: self.marshal.clone(),
                    fcu_heartbeat_interval: Duration::from_secs(60),
                    last_consensus_finalized_height: Height::new(last_consensus),
                    last_execution_finalized_height: anchor_height,
                    initial_finalized: (Height::new(anchor_height), anchor_hash),
                    initial_head: (Height::new(anchor_height), anchor_hash),
                    dpos_activation_block: activation,
                    fcu_pace: Duration::from_millis(0),
                    peers_for_finalization: std::sync::Arc::new(dummy_peers),
                    beacon_metrics: crate::beacon::metrics::BeaconMetrics::default(),
                    spawn_unblocked: std::sync::Arc::new(tokio::sync::Notify::new()),
                    re_jump: self.re_jump.lock().unwrap().clone(),
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

    /// A `SpecNotarized` command for `order` (seedless; the round view is a
    /// stand-in — the executor keys speculation off the fetched block's height,
    /// not the round).
    fn spec_msg(order: &OrderBlock) -> Message {
        use commonware_consensus::types::{Epoch, View};
        Message {
            cause: Span::current(),
            command: Command::SpecNotarized(Box::new(Notarized {
                round: commonware_consensus::types::Round::new(
                    Epoch::new(0),
                    View::new(order.height),
                ),
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
        lc = lc.update_safe(Height::new(13), h13).update_head(Height::new(13), h13);
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
        assert_eq!(after.safe_height, Height::new(13), "height unchanged (lateral)");
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

            let mut parent = Digest(B256::ZERO);
            for i in 1..=(K + 1) {
                let height = ANCHOR + i;
                let result = match height.checked_sub(K) {
                    Some(h) if h >= ANCHOR => fx.chain.executed_hash(h).unwrap(),
                    _ => B256::ZERO,
                };
                let order = sample_order(parent, height, result);
                parent = order.digest();
                let (msg, waiter) = finalize_msg(order);
                mailbox.send(msg).expect("send");
                waiter.await.expect("ack");
            }

            {
                let fcus = fx.beacon.fcu_calls.lock().unwrap();
                // Heights ANCHOR+1..=ANCHOR+K-1: finalized pinned to the anchor
                // while safe (ordering-final) climbs to each just-finalized tip.
                for (i, fcu) in fcus[..(K - 1) as usize].iter().enumerate() {
                    let ordering_tip = ANCHOR + 1 + i as u64;
                    assert_eq!(fcu.finalized_block_hash, fx.anchor_hash);
                    assert_eq!(
                        fcu.safe_block_hash,
                        fx.chain.executed_hash(ordering_tip).unwrap(),
                        "safe rides the ordering-final tip even while finalized is clamped"
                    );
                    assert_eq!(
                        fcu.safe_block_hash, fcu.head_block_hash,
                        "no speculative lead ⇒ safe == head"
                    );
                }
                // Height ANCHOR+K: result_final = ANCHOR (still the anchor hash);
                // height ANCHOR+K+1: result_final = ANCHOR+1 = derived hash.
                let derived_anchor_plus_1 = fx.chain.executed_hash(ANCHOR + 1).unwrap();
                let ordering_tip = fx.chain.executed_hash(ANCHOR + K + 1).unwrap();
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

    // An OrderBlock whose attested `result` disagrees with the locally-derived
    // hash at `height − K` means this node would serve a fork; `try_derive` fails
    // loud and the actor loop shuts down (the same trustless property
    // `FluentApp::verify` enforces on the BFT path).
    #[test]
    fn result_divergence_shuts_down_the_executor() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // Pre-K window: result MUST be ZERO (no cross-check fires).
            let mut parent = Digest(B256::ZERO);
            for i in 1..K {
                let order = sample_order(parent, ANCHOR + i, B256::ZERO);
                parent = order.digest();
                let (msg, waiter) = finalize_msg(order);
                mailbox.send(msg).expect("send");
                waiter.await.expect("pre-K ack");
            }

            // Height ANCHOR+K commits the hash at ANCHOR — but with a forged value.
            let forged = B256::repeat_byte(0xEE);
            assert_ne!(forged, fx.chain.executed_hash(ANCHOR).unwrap());
            let (msg, waiter) = finalize_msg(sample_order(parent, ANCHOR + K, forged));
            mailbox.send(msg).expect("send divergent");

            // The actor shuts down without acking the divergent block; the
            // mailbox is still open, so `handle.await` resolving at all proves the
            // loop broke (it would hang otherwise).
            assert!(waiter.await.is_err(), "divergent block must not ack");
            handle
                .await
                .expect("executor task joins after shutdown break");
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

            let (msg, waiter) = finalize_msg(order);
            mailbox.send(msg).expect("send below-anchor post-activation block");
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

            // Live finalize for height 4 lands BEFORE backfill drains.
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
            // real parent 103 is underived, so the gap-walk fills 101..103 first.
            // The result still commits the derived hash at 101 (cross-check passes).
            let delivered = OrderBlock {
                parent: Digest(B256::ZERO),
                ..chain[3].clone()
            };
            let (msg, waiter) = finalize_msg(delivered);
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

    // A GAP block (filled by `derive_missing_prefix`, not the top-level delivery)
    // carries its OWN attested `result`; a forged value on a gap-range block must
    // fail loud just like the top-level cross-check — otherwise a
    // committee-attested wrong result on a gap block is imported unchecked (the
    // byzantine-vrf defense). Here ANCHOR+K (the first POST-pre-activation gap
    // height) commits a forged hash; the gap-walk derives it then the cross-check
    // shuts the executor down.
    #[test]
    fn gap_block_result_divergence_shuts_down_the_executor() {
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
            assert_ne!(forged, fx.chain.executed_hash(ANCHOR).unwrap());
            let mut forged_chain = chain.clone();
            forged_chain[forged_idx].result = forged;
            // All gap heights ANCHOR+1 ..= ANCHOR+K+1 exist ONLY in the marshal.
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                for order in &forged_chain[..(K + 1) as usize] {
                    canned.insert(order.height, order.clone());
                }
            }
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // Deliver the TOP height (ANCHOR+K+1) with an unresolvable parent so the
            // gap-walk fills ANCHOR+1 ..= ANCHOR+K first — hitting the forged gap
            // block at ANCHOR+K. The delivered block itself has a consistent result.
            let delivered = OrderBlock {
                parent: Digest(B256::ZERO),
                ..forged_chain[(K + 1) as usize].clone()
            };
            let (msg, waiter) = finalize_msg(delivered);
            mailbox.send(msg).expect("send");

            // The forged gap block must abort the walk → no ack, executor shuts down.
            assert!(waiter.await.is_err(), "forged gap block must not ack");
            handle
                .await
                .expect("executor task joins after gap-divergence break");
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
                        Height::new(ANCHOR + 50),
                        tip_digest,
                    ))),
                })
                .expect("send tip");

            // Drain barrier: one real finalize.
            let (msg, waiter) =
                finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO));
            mailbox.send(msg).expect("send");
            waiter.await.expect("ack");

            {
                let fcus = fx.beacon.fcu_calls.lock().unwrap();
                assert_eq!(fcus.len(), 1, "tip produced no FCU; only the finalize did");
                assert_eq!(
                    fcus[0].head_block_hash,
                    fx.chain.executed_hash(ANCHOR + 1).unwrap()
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
                let derived = fx.chain.executed_hash(ANCHOR + 1).unwrap();
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

    // A finalized block whose cert is not local yet is DEFERRED (not derived
    // with the fallback), the peers are hinted, and once the cert lands (here:
    // after the hint) the block derives — in strict order behind any earlier
    // deferred block. Exercises the non-blocking deferred-slot + self-driven
    // timer.
    #[test]
    fn catch_up_defers_then_derives_in_order_when_cert_lands() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            *fx.marshal.seed_mode.lock().unwrap() = SeedMode::MissingUntilHinted;
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            let o1 = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let o2 = sample_order(o1.digest(), ANCHOR + 2, B256::ZERO);
            let (m1, _w1) = finalize_msg(o1);
            let (m2, w2) = finalize_msg(o2);
            mailbox.send(m1).expect("send 1");
            mailbox.send(m2).expect("send 2");
            // The deterministic clock auto-advances past SEED_FETCH_POLL, firing
            // the re-poll; by then the hint is recorded so the cert resolves.
            w2.await
                .expect("ack 2 (both derived after their certs landed)");

            {
                let payloads = fx.beacon.new_payload_calls.lock().unwrap();
                let heights: Vec<u64> = payloads.iter().map(|p| p.number).collect();
                assert_eq!(
                    heights,
                    vec![ANCHOR + 1, ANCHOR + 2],
                    "both derived, strict order (h before h+1) preserved across deferral"
                );
                let hints = fx.marshal.hints.lock().unwrap();
                assert!(
                    hints.contains(&(ANCHOR + 1)) && hints.contains(&(ANCHOR + 2)),
                    "each missing cert was hinted to peers: {hints:?}"
                );
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A genuine no-beacon epoch (cert present, no seed) derives immediately with
    // the agreed fallback — no defer, no hint.
    #[test]
    fn no_beacon_epoch_derives_immediately_without_hinting() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR); // default SeedMode::NoBeacon
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            let (msg, waiter) =
                finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO));
            mailbox.send(msg).expect("send");
            waiter.await.expect("ack");

            assert_eq!(
                fx.beacon.new_payload_calls.lock().unwrap().len(),
                1,
                "derived immediately"
            );
            assert!(
                fx.marshal.hints.lock().unwrap().is_empty(),
                "no-beacon epoch must not hint a re-fetch"
            );

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A finalized beacon block whose cert NEVER arrives must fail loud (shutdown)
    // after the budget — never derive with the fallback (which would fork).
    #[test]
    fn deferred_fails_loud_when_cert_never_arrives() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            *fx.marshal.seed_mode.lock().unwrap() = SeedMode::AlwaysMissing;
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            let (msg, _waiter) =
                finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO));
            mailbox.send(msg).expect("send");
            // The deterministic clock fast-forwards through SEED_FETCH_MAX_WAIT;
            // the executor breaks (shutdown), so the handle completes WITHOUT us
            // dropping the mailbox.
            let _ = handle.await;

            assert!(
                fx.beacon.new_payload_calls.lock().unwrap().is_empty(),
                "must NOT derive with the fallback when the beacon cert is missing"
            );
            assert!(
                !fx.marshal.hints.lock().unwrap().is_empty(),
                "should have hinted the missing cert before giving up"
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
            w1.await.expect("ack 1");

            let o2b = OrderBlock {
                extra_data: Bytes::from_static(b"B"),
                ..sample_order(o1.digest(), ANCHOR + 2, B256::ZERO)
            };
            let (m2b, w2b) = finalize_msg(o2b.clone());
            mailbox.send(m2b).expect("send finalize 2b");
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
                    fx.chain.executed_hash(ANCHOR + 1).unwrap(),
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
            // clamped at the anchor in the pre-K window).
            let o1 = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            let (m1, w1) = finalize_msg(o1.clone());
            mailbox.send(m1).expect("send finalize 1");
            w1.await.expect("ack 1");

            let safe_after_finalize = fx.chain.executed_hash(ANCHOR + 1).unwrap();
            {
                let fcus = fx.beacon.fcu_calls.lock().unwrap();
                let last = fcus.last().unwrap();
                assert_eq!(last.safe_block_hash, safe_after_finalize);
                assert_eq!(last.finalized_block_hash, fx.anchor_hash);
            }

            // Speculate +2, +3 (notarized, not finalized) — each parent is
            // canonical from the prior FCU.
            let o2 = sample_order(o1.digest(), ANCHOR + 2, B256::ZERO);
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
                    fx.chain.executed_hash(ANCHOR + 3).unwrap(),
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

    // An INVALID forkchoice status (import VALID, FCU INVALID) is fatal —
    // the executor shuts down rather than advancing onto a rejected head.
    #[test]
    fn invalid_fcu_status_is_fatal() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            *fx.beacon.fcu_status.lock().unwrap() = Some(PayloadStatusEnum::Invalid {
                validation_error: "test-induced".into(),
            });
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            let (msg, _waiter) =
                finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO));
            mailbox.send(msg).expect("send");
            // The executor breaks on the INVALID FCU, so the handle completes
            // WITHOUT us dropping the mailbox.
            let _ = handle.await;
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

            let (msg, waiter) =
                finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO));
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

    // Finalized path: a real `SeedLookup::Seed(Some)` flows through the
    // executor into the deriver — the deriver receives the cert-recovered seed,
    // not None.
    #[test]
    fn real_seed_flows_through_executor_to_deriver() {
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let seed = real_seed(Round::new(Epoch::new(0), View::new(ANCHOR + 1)));
            let fx = Fixture::new(ANCHOR);
            *fx.marshal.seed_mode.lock().unwrap() = SeedMode::Beacon(seed.clone());
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            let (msg, waiter) =
                finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO));
            mailbox.send(msg).expect("send");
            waiter.await.expect("ack");

            {
                let seen = fx.deriver.seeds_seen.lock().unwrap();
                assert_eq!(
                    seen.as_slice(),
                    &[(ANCHOR + 1, Some(seed))],
                    "the cert-recovered seed reached the deriver verbatim"
                );
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // Speculative path: the seed recovered from the NOTARIZATION cert (the
    // `SpecNotarized` command) reaches the deriver during speculative execution.
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

            let order = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO);
            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(ANCHOR + 1, order.clone());

            // Speculative command carrying the notarization seed.
            mailbox
                .send(Message {
                    cause: Span::current(),
                    command: Command::SpecNotarized(Box::new(Notarized {
                        round,
                        digest: order.digest(),
                        seed: Some(seed.clone()),
                    })),
                })
                .expect("send spec");
            // Drain barrier: finalize the same order (skips re-derive since it was
            // speculated; the spec import already recorded the seed).
            let (m, w) = finalize_msg(order.clone());
            mailbox.send(m).expect("send finalize");
            w.await.expect("ack");

            {
                let seen = fx.deriver.seeds_seen.lock().unwrap();
                assert_eq!(
                    seen.as_slice(),
                    &[(ANCHOR + 1, Some(seed))],
                    "the notarization-cert seed reached the deriver during speculation"
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

    /// Clonable script for a re-jump callback's terminal outcome. `JumpOutcome`
    /// itself is not `Clone` (its `eyre::Report` variants), so the test scripts a
    /// clonable descriptor and the `Fn` (which may be called more than once)
    /// rebuilds a fresh `JumpOutcome` per call.
    #[derive(Clone)]
    enum Scripted {
        Landed { landing: u64, hash: B256, floor: u64 },
        Lagging,
        Stalled(String),
        AuthFailed(String),
    }

    impl Scripted {
        fn build(&self) -> crate::cold_start_jump::JumpOutcome {
            use crate::cold_start_jump::JumpOutcome;
            match self {
                Scripted::Landed { landing, hash, floor } => JumpOutcome::Landed {
                    landing: *landing,
                    hash: *hash,
                    floor: *floor,
                },
                Scripted::Lagging => JumpOutcome::Lagging,
                Scripted::Stalled(s) => JumpOutcome::Stalled(eyre::eyre!(s.clone())),
                Scripted::AuthFailed(s) => JumpOutcome::AuthFailed(eyre::eyre!(s.clone())),
            }
        }
    }

    /// A re-jump callback recording each `from` it was invoked with and returning
    /// the scripted [`crate::cold_start_jump::JumpOutcome`].
    type RejumpCalls = Arc<Mutex<Vec<u64>>>;
    fn recording_re_jump(scripted: Scripted) -> (ReJump, RejumpCalls) {
        let calls: RejumpCalls = Arc::new(Mutex::new(Vec::new()));
        let calls_cl = calls.clone();
        let cb: ReJump = Arc::new(move |from| {
            calls_cl.lock().unwrap().push(from);
            let scripted = scripted.clone();
            Box::pin(async move { scripted.build() })
        });
        (cb, calls)
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
            let (msg, waiter) = finalize_msg(sample_order(
                Digest(B256::ZERO),
                landing_h + 1,
                B256::ZERO,
            ));
            mailbox.send(msg).expect("send finalize after re-jump");
            waiter.await.expect("post-re-jump block acks");

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
            assert_eq!(safe_height, Height::new(landing), "safe raised to the landing");
            assert_eq!(spec_head, landing, "stale-spec: spec_head raised to the landing");
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
                .derive_and_execute(sample_order(Digest(B256::ZERO), floor + 1, B256::ZERO), floor_hash, None)
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
                assert_eq!(reseed_fcu.head_block_hash, landing_hash, "FCU head = landing");
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
                .derive_and_execute(sample_order(Digest(B256::ZERO), floor + 1, B256::ZERO), floor_hash, None)
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
            let (msg, waiter) =
                finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO));
            mailbox.send(msg).expect("send");
            waiter.await.expect("ack");

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

    // (c) An `AuthFailed` outcome (a forged far-ahead target that
    // `verify_jump_authenticated` rejects) shuts the executor loop down — no ack
    // for any later block (the loop has broken), fail-closed.
    #[test]
    fn re_jump_auth_failed_shuts_down_the_executor() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let (cb, _calls) = recording_re_jump(Scripted::AuthFailed(
                "forged far-ahead target rejected".into(),
            ));
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                .expect("send tip");

            // The actor breaks on the fail-closed AuthFailed, so the handle
            // completes WITHOUT us dropping the mailbox (it would hang otherwise).
            handle
                .await
                .expect("executor task joins after fail-closed re-jump break");
            assert!(
                fx.marshal.floors.lock().unwrap().is_empty(),
                "a fail-closed re-jump must NOT advance the marshal floor"
            );
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
            let (msg, waiter) =
                finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO));
            mailbox.send(msg).expect("send");
            waiter
                .await
                .expect("ack after a Stalled re-jump (NON-fatal)");

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
            let (msg, waiter) =
                finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO));
            mailbox.send(msg).expect("send");
            waiter.await.expect("ack after Lagging re-jump");

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

    // (e) The re-jump does NOT start while a block is DEFERRED awaiting its cert:
    // the reseed prunes below the floor, so starting mid-deferral could reconcile
    // a sub-floor deferred block. With a deferred block parked (cert always
    // missing), a far frontier tip must NOT invoke the re-jump callback.
    #[test]
    fn re_jump_does_not_start_while_block_deferred() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let (cb, calls) = recording_re_jump(Scripted::Lagging);
            let fx = Fixture::new(ANCHOR).with_re_jump(cb);
            // The cert never lands → the block stays deferred (the drain pauses).
            *fx.marshal.seed_mode.lock().unwrap() = SeedMode::AlwaysMissing;
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // Finalize a block whose beacon cert is missing ⇒ it defers.
            let (msg, _waiter) =
                finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO));
            mailbox.send(msg).expect("send deferring block");

            // A far frontier tip arrives WHILE the block is deferred → the
            // re-jump must NOT spawn (the deferred-gate). The executor will
            // eventually fail loud when the cert budget expires (AlwaysMissing).
            mailbox
                .send(tip_msg(ANCHOR + JUMP_THRESHOLD + 5_010))
                .expect("send tip");

            // The actor shuts down on the deferred budget timeout, joining the
            // handle (mailbox not dropped).
            let _ = handle.await;
            assert!(
                calls.lock().unwrap().is_empty(),
                "re-jump must NOT be invoked while a block is deferred"
            );
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
            mailbox.send(spec_msg(&order)).expect("send spec");

            // Drain barrier: finalize the SAME order — reconciliation skips the
            // re-derive iff the speculation landed first.
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
}
