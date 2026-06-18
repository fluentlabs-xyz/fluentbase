//! Executor: drives the reth EL from ordering-finalized [`OrderBlock`]s —
//! derive → execute (import via `new_payload`) → two-tier FCU.
//!
//! Two-tier forkchoice: `head` follows the locally derived executed tip;
//! `safe`/`finalized` follow RESULT finality = `ordering_finalized − K`
//! (clamped to the cold-start anchor), i.e. the height whose derived hash the
//! committee has attested by agreeing the OrderBlock K heights above it.
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
use commonware_runtime::{spawn_cell, Clock, ContextCell, FutureExt as _, Handle, Pacer, Spawner};
use commonware_utils::{
    acknowledgement::Exact, futures::OptionFuture, vec::NonEmptyVec, Acknowledgement as _,
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
    /// finalization rounds. Best-effort: `forward_finalized` stays the sole
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
    pub seed: Option<crate::beacon::types::Seed>,
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
    finalized_height: Height,
}

impl LastCanonicalized {
    fn update_finalized(mut self, height: Height, hash: B256) -> Self {
        if height > self.finalized_height {
            self.finalized_height = height;
            self.forkchoice.safe_block_hash = hash;
            self.forkchoice.finalized_block_hash = hash;
        }
        if height >= self.head_height {
            self.head_height = height;
            self.forkchoice.head_block_hash = hash;
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
    Seed(crate::beacon::types::Seed),
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
                Some(signature) => SeedLookup::Seed(crate::beacon::types::Seed {
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
    Seed(Option<crate::beacon::types::Seed>),
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
    pub fcu_pace: Duration,
    pub peers_for_finalization: PeersForFinalization,
}

pub struct Actor<E, BE, D, XC, MarshalMailbox> {
    context: ContextCell<E>,
    beacon_engine: BE,
    deriver: D,
    executed: XC,
    marshal: MarshalMailbox,
    mailbox: mpsc::UnboundedReceiver<Message>,

    last_canonicalized: LastCanonicalized,
    /// Highest ordering-finalized height processed; drives the result-final
    /// cursor (`− K`, clamped to the anchor).
    ordering_finalized: u64,
    /// Anchor floor for the finalized cursor: the cold-start finalized point
    /// is result-final by construction (committee-external trust root).
    anchor_finalized: (Height, B256),

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
            last_canonicalized: LastCanonicalized {
                forkchoice: ForkchoiceState {
                    head_block_hash: cfg.initial_head.1,
                    safe_block_hash: cfg.initial_finalized.1,
                    finalized_block_hash: cfg.initial_finalized.1,
                },
                head_height: cfg.initial_head.0,
                finalized_height: cfg.initial_finalized.0,
            },
            ordering_finalized: cfg.last_execution_finalized_height,
            anchor_finalized: cfg.initial_finalized,
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
                    self.handle_message(msg).await;
                }

                _ = (&mut self.fcu_heartbeat_timer).fuse() => {
                    self.send_forkchoice_update_heartbeat().await;
                    self.reset_fcu_heartbeat_timer();
                }
            }
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

    async fn handle_message(&mut self, message: Message) {
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
                Update::Tip(_round, height, _ordering_digest) => {
                    debug!(%height, "ordering tip observed; EL catch-up is backfill+derive");
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
                    // fatal — `forward_finalized` will derive the block at
                    // finalization regardless.
                    warn!(%error, %digest, "speculative execution skipped");
                }
            }
        }
    }

    /// Speculatively derive + import a NOTARIZED block, advancing the EL head
    /// ahead of finalization. Strictly forward-only (`spec_head + 1`); a gap or
    /// an already-covered height is left to `forward_finalized`, which keeps
    /// this path race-free with finalized delivery (both run in this one loop).
    #[instrument(skip_all, parent = &cause, fields(%digest), err(level = Level::DEBUG))]
    async fn spec_execute(
        &mut self,
        cause: Span,
        _round: commonware_consensus::types::Round,
        digest: crate::digest::Digest,
        seed: Option<crate::beacon::types::Seed>,
    ) -> eyre::Result<()> {
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
            !fcu.is_invalid(),
            "EL reported invalid speculative FCU: {:?}",
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
    async fn try_derive(
        &mut self,
        cause: Span,
        order: OrderBlock,
        ack: Exact,
    ) -> eyre::Result<DeriveOutcome> {
        let height = order.height;
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
        // Move the head onto the finalized block only when speculation did not
        // already place the correct block here (else we would roll back the
        // speculative lead). A re-derive/rollback DOES move the head (reorg).
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
            !fcu.is_invalid(),
            "EL reported invalid finalize FCU: {:?}",
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
            let derived = self
                .deriver
                .derive_and_execute(order, parent_hash, seed)
                .await
                .wrap_err_with(|| format!("gap derivation failed at height {h}"))?;
            parent_hash = derived.evm_hash();
            self.submit_finalized_payload(derived).await?;
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
    use crate::digest::Digest;
    use crate::order_block::K;
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

    /// Shared height→hash canonical map: the deriver inserts on derive
    /// (modelling new_payload+FCU canonicalization), the ExecutedChain
    /// reads — mirrors the provider-backed production impl.
    #[derive(Clone, Default)]
    struct FakeChain {
        canonical: Arc<Mutex<BTreeMap<u64, B256>>>,
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

    #[derive(Clone)]
    struct FakeDeriver {
        chain: FakeChain,
    }

    impl DerivedBlockBuilder for FakeDeriver {
        type Derived = RethExecBlock;

        async fn derive_and_execute(
            &self,
            order: OrderBlock,
            parent_evm_hash: B256,
            _seed: Option<crate::beacon::types::Seed>,
        ) -> eyre::Result<RethExecBlock> {
            let sealed = sealed_at(parent_evm_hash, order.height, order.digest().0);
            // Last writer wins, modelling a reth reorg: a finalized sibling
            // derived after a speculative one replaces the canonical hash.
            self.chain
                .canonical
                .lock()
                .unwrap()
                .insert(order.height, sealed.hash());
            Ok(sealed)
        }
    }

    #[derive(Clone, Default)]
    struct FakeBeacon {
        fcu_calls: Arc<Mutex<Vec<ForkchoiceState>>>,
        new_payload_calls: Arc<Mutex<Vec<RethExecBlock>>>,
    }

    impl BeaconEngineLike for FakeBeacon {
        type ExecutionData = RethExecBlock;

        async fn fork_choice_updated(
            &self,
            state: ForkchoiceState,
        ) -> eyre::Result<ForkchoiceUpdated> {
            self.fcu_calls.lock().unwrap().push(state);
            Ok(ForkchoiceUpdated::from_status(PayloadStatusEnum::Valid))
        }

        async fn import_derived(&self, data: RethExecBlock) -> eyre::Result<PayloadStatus> {
            self.new_payload_calls.lock().unwrap().push(data);
            Ok(PayloadStatus::from_status(PayloadStatusEnum::Valid))
        }
    }

    /// Scripts how `lookup_seed` behaves in a test.
    #[derive(Clone, Copy, Default, PartialEq)]
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
    }

    #[derive(Clone, Default)]
    struct FakeMarshal {
        canned: Arc<Mutex<BTreeMap<u64, OrderBlock>>>,
        seed_mode: Arc<Mutex<SeedMode>>,
        /// Heights passed to `hint_finalization`, in call order.
        hints: Arc<Mutex<Vec<u64>>>,
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
            match *self.seed_mode.lock().unwrap() {
                SeedMode::NoBeacon => SeedLookup::NoBeacon,
                SeedMode::AlwaysMissing => SeedLookup::CertMissing,
                SeedMode::MissingUntilHinted => {
                    if self.hints.lock().unwrap().contains(&height.get()) {
                        SeedLookup::NoBeacon
                    } else {
                        SeedLookup::CertMissing
                    }
                }
            }
        }
        async fn hint_finalization(&self, height: Height, _targets: NonEmptyVec<PeerPubkey>) {
            self.hints.lock().unwrap().push(height.get());
        }
    }

    struct Fixture {
        chain: FakeChain,
        beacon: FakeBeacon,
        marshal: FakeMarshal,
        anchor_hash: B256,
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
            Self {
                chain,
                beacon: FakeBeacon::default(),
                marshal: FakeMarshal::default(),
                anchor_hash,
            }
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
            Actor::init(
                ctx,
                Config {
                    beacon_engine: self.beacon.clone(),
                    deriver: FakeDeriver {
                        chain: self.chain.clone(),
                    },
                    executed: self.chain.clone(),
                    marshal: self.marshal.clone(),
                    fcu_heartbeat_interval: Duration::from_secs(60),
                    last_consensus_finalized_height: Height::new(last_consensus),
                    last_execution_finalized_height: anchor_height,
                    initial_finalized: (Height::new(anchor_height), self.anchor_hash),
                    initial_head: (Height::new(anchor_height), self.anchor_hash),
                    fcu_pace: Duration::from_millis(0),
                    peers_for_finalization: std::sync::Arc::new(dummy_peers),
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
                // Heights ANCHOR+1..=ANCHOR+K-1: finalized pinned to the anchor.
                for fcu in &fcus[..(K - 1) as usize] {
                    assert_eq!(fcu.finalized_block_hash, fx.anchor_hash);
                }
                // Height ANCHOR+K: result_final = ANCHOR (still the anchor hash);
                // height ANCHOR+K+1: result_final = ANCHOR+1 = derived hash.
                let derived_anchor_plus_1 = fx.chain.executed_hash(ANCHOR + 1).unwrap();
                let last = fcus.last().unwrap();
                assert_eq!(last.finalized_block_hash, derived_anchor_plus_1);
                assert_eq!(
                    last.head_block_hash,
                    fx.chain.executed_hash(ANCHOR + K + 1).unwrap()
                );
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

    #[test]
    fn backfill_drains_before_live_finalize() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 0;
            let fx = Fixture::new(ANCHOR);
            // Heights 1..=3 canned in the marshal (crash-recovery backfill).
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                let mut parent = Digest(B256::ZERO);
                for h in 1..=3u64 {
                    let order = sample_order(parent, h, B256::ZERO);
                    parent = order.digest();
                    canned.insert(h, order);
                }
            }
            let (actor, mailbox) = fx.build(ctx, ANCHOR, 3);
            let handle = actor.start();

            // Live finalize for height 4 lands BEFORE backfill drains.
            let parent3 = fx.marshal.canned.lock().unwrap().get(&3).unwrap().digest();
            let (msg, waiter) = finalize_msg(sample_order(parent3, 4, B256::ZERO));
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
            // Heights 101..=103 exist ONLY in the marshal (not yet derived).
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                let mut parent = Digest(B256::ZERO);
                for h in (ANCHOR + 1)..=(ANCHOR + 3) {
                    let order = sample_order(parent, h, B256::ZERO);
                    parent = order.digest();
                    canned.insert(h, order);
                }
            }
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // Deliver height 104 directly — its parent 103 is underived.
            let (msg, waiter) =
                finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 4, B256::ZERO));
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

            // Speculatively execute sibling A (notarized at ANCHOR+1).
            let order_a = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::repeat_byte(0xAA));
            fx.marshal
                .canned
                .lock()
                .unwrap()
                .insert(ANCHOR + 1, order_a.clone());
            mailbox.send(spec_msg(&order_a)).expect("send spec A");

            // But a different sibling B finalizes (A was nullified).
            let order_b = sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::repeat_byte(0xBB));
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
}
