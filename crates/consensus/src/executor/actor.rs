//! Drives reth EL forwarding + forkchoice state from consensus.

use std::{ops::RangeInclusive, pin::Pin, sync::Arc, time::Duration};

use alloy_primitives::{Bytes, B256};
use alloy_rpc_types_engine::{ForkchoiceState, PayloadId};
use commonware_consensus::{marshal::Update, types::Height, Heightable as _};
use commonware_runtime::{spawn_cell, Clock, ContextCell, FutureExt as _, Handle, Pacer, Spawner};
use commonware_utils::{acknowledgement::Exact, Acknowledgement as _};
use dashmap::DashMap;
use eyre::{ensure, WrapErr as _};
use futures::{
    channel::oneshot,
    future::{ready, BoxFuture, Ready},
    stream::FuturesOrdered,
    FutureExt as _, StreamExt as _,
};
use prometheus_client::metrics::gauge::Gauge;
use tokio::{select, sync::mpsc};
use tracing::{debug, error, error_span, info, info_span, instrument, warn, warn_span, Level, Span};

use commonware_utils::futures::OptionFuture;

use crate::{
    application::BeaconEngineLike,
    block::Block,
    digest::Digest,
    executor::ingress::{
        CanonicalizeAndBuild, CanonicalizeError, Command, Mailbox, Message,
    },
};

/// Max ancestors the S5 gap-heal walks back via the marshal before bailing.
/// Kept below reth's `MIN_BLOCKS_FOR_PIPELINE_RUN` (= `EPOCH_SLOTS` = 32) so a
/// heal never grows large enough for reth to trip its own devp2p backfill —
/// which, with `connected_peers = 0` in DPoS, would never complete.
const MAX_GAP_HEAL: u64 = 31;

// LastCanonicalized — monotonic projection of forkchoice state.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LastCanonicalized {
    forkchoice: ForkchoiceState,
    head_height: Height,
    finalized_height: Height,
}

impl LastCanonicalized {
    fn update_finalized(self, height: Height, digest: Digest) -> Self {
        let mut this = self;
        if height > this.finalized_height {
            this.finalized_height = height;
            this.forkchoice.safe_block_hash = digest.0;
            this.forkchoice.finalized_block_hash = digest.0;
        }
        if height >= this.head_height {
            this.head_height = height;
            this.forkchoice.head_block_hash = digest.0;
        }
        this
    }

    fn update_head(self, height: Height, digest: Digest) -> Self {
        let mut this = self;
        // `&& height >= head_height` keeps the head monotonic (mirrors
        // `update_finalized`): a lower-height head with a digest matching the
        // finalized hash must NOT roll the head backward into an FCU.
        if (height > this.finalized_height || digest.0 == this.forkchoice.finalized_block_hash)
            && height >= this.head_height
        {
            this.head_height = height;
            this.forkchoice.head_block_hash = digest.0;
        }
        this
    }
}

// BlockFetcher — minimal trait so we don't depend on the full marshal Mailbox type.

pub trait BlockFetcher: Clone + Send + Sync + 'static {
    fn fetch_block_by_height(
        &self,
        height: Height,
    ) -> impl std::future::Future<Output = Option<Block>> + Send;
}

/// Audit fix #2: explicit impl for the concrete marshal mailbox.
/// Orphan rule OK — BlockFetcher local, Mailbox foreign.
/// `Mailbox::get_block` takes `impl Into<Identifier<Digest>>` and
/// `Height: Into<Identifier<Digest>>` ([marshal/mod.rs:103](file:///home/djadjka/.cargo/git/checkouts/monorepo-9732103c47eb4665/3c4e02c/consensus/src/marshal/mod.rs#L103)).
impl BlockFetcher
    for commonware_consensus::marshal::core::Mailbox<
        fluentbase_bls::Scheme,
        commonware_consensus::marshal::standard::Standard<Block>,
    >
{
    async fn fetch_block_by_height(&self, height: Height) -> Option<Block> {
        self.get_block(height).await
    }
}


pub struct Config<BE, MarshalMailbox> {
    pub beacon_engine: BE,
    pub marshal: MarshalMailbox,
    pub fcu_heartbeat_interval: Duration,
    pub last_consensus_finalized_height: Height,
    pub last_execution_finalized_height: u64,
    pub initial_finalized: (Height, B256),
    pub initial_head: (Height, B256),
    pub fcu_pace: Duration,
    /// Shared with FluentPayloadBuilder (reader). Executor inserts
    /// extra_data atomically between FCU return and the propose
    /// oneshot send, closing the race against reth's
    /// spawn_blocking_task payload-build worker. FluentApp::propose
    /// still owns the post-resolve_kind removal for cleanup.
    pub extra_data_registry: Arc<DashMap<PayloadId, Bytes>>,
    /// Reth's in-memory canonical state. Read by `canonicalize` to
    /// detect when reth's canonical chain has been silently advanced
    /// past `last_canonicalized.head` by `FluentApp::verify`'s direct
    /// `new_payload` calls (the verify-path bypasses the executor).
    /// Per Engine API spec, sending FCU(head=ancestor_of_canonical) is
    /// allowed but must return `{Valid, payload_id: null}` and skip
    /// payload build. Reth (post-v1.1.5) instead enters SYNCING state
    /// (issue paradigmxyz/reth#16629), trapping the validator.
    pub canonical_state:
        reth_chain_state::CanonicalInMemoryState<reth_ethereum_primitives::EthPrimitives>,
}

pub struct Actor<E, BE, Attrs, MarshalMailbox> {
    context: ContextCell<E>,
    beacon_engine: BE,
    marshal: MarshalMailbox,
    mailbox: mpsc::UnboundedReceiver<Message<Attrs>>,

    last_canonicalized: LastCanonicalized,

    fcu_heartbeat_interval: Duration,
    fcu_heartbeat_timer: Pin<Box<dyn std::future::Future<Output = ()> + Send>>,

    fcu_pace: Duration,

    finalized_heights_to_backfill: RangeInclusive<u64>,
    pending_backfill: OptionFuture<BoxFuture<'static, (u64, Option<Block>)>>,
    pending_finalizations: FuturesOrdered<Ready<(Span, Block, Exact)>>,

    extra_data_registry: Arc<DashMap<PayloadId, Bytes>>,

    /// Ops-visibility gauge for `pending_finalizations.len()`. Alert on
    /// sustained values > 4 — indicates EL is falling behind consensus
    /// (`MAX_PENDING_ACKS = 16` is the marshal-side ceiling).
    pending_finalizations_gauge: Gauge<i64>,

    /// Tracks whether consensus has advanced `last_canonicalized` from its
    /// cold-start initial snapshot. Heartbeat FCUs are suppressed while this is
    /// `false` to avoid the Tempo→DPoS migration race where the verify-path's
    /// `new_payload` calls promote `block_{N+1}` to reth's canonical chain
    /// BEFORE consensus produces its first finalization: a heartbeat FCU with
    /// the stale `head=block_N` (= PREV_FIN) would spuriously roll reth's head
    /// back to a prior block.
    ///
    /// NOTE (corrected 2026-05-29 after primary-source review): a stale head
    /// that reth still HAS (a valid ancestor) returns `{VALID, …}`, NOT SYNCING
    /// — reth only returns SYNCING when the head is missing/unknown (or the
    /// backfill pipeline is busy); see `engine/tree/src/tree/mod.rs`. And
    /// paradigmxyz/reth#16629 is the "`VALID` but no `payloadId` post-reorg"
    /// bug, not a SYNCING bug. The real missing-block SYNCING wedge is a
    /// separate condition. Suppression here remains correct hygiene (don't
    /// resend a stale cold-start head), just not a "SYNCING trap" guard.
    has_advanced_since_init: bool,

    /// Reth's in-memory canonical state. See `Config::canonical_state`
    /// for rationale.
    canonical_state:
        reth_chain_state::CanonicalInMemoryState<reth_ethereum_primitives::EthPrimitives>,
}

impl<E, BE, Attrs, MarshalMailbox> Actor<E, BE, Attrs, MarshalMailbox>
where
    E: Clock + commonware_runtime::Metrics + Pacer + Spawner + Send + 'static,
    BE: BeaconEngineLike<
            PayloadAttrs = Attrs,
            ExecutionData = reth_primitives_traits::SealedBlock<reth_ethereum_primitives::Block>,
        > + Send
        + Sync
        + 'static,
    Attrs: Clone + Send + 'static,
    MarshalMailbox: BlockFetcher,
{
    pub fn init(context: E, cfg: Config<BE, MarshalMailbox>) -> (Self, Mailbox<Attrs>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let mailbox = Mailbox::new(tx);

        let fcu_heartbeat_timer = Box::pin(context.sleep(cfg.fcu_heartbeat_interval));

        let finalized_heights_to_backfill =
            (cfg.last_execution_finalized_height + 1)..=cfg.last_consensus_finalized_height.get();

        let pending_finalizations_gauge = Gauge::<i64>::default();
        context.register(
            "pending_finalizations",
            "Count of finalized blocks awaiting FCU+new_payload+ack in the executor queue \
             (MAX_PENDING_ACKS=16 marshal-side ceiling).",
            pending_finalizations_gauge.clone(),
        );

        let actor = Self {
            context: ContextCell::new(context),
            beacon_engine: cfg.beacon_engine,
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
            fcu_heartbeat_interval: cfg.fcu_heartbeat_interval,
            fcu_heartbeat_timer,
            fcu_pace: cfg.fcu_pace,
            finalized_heights_to_backfill,
            pending_backfill: OptionFuture::default(),
            pending_finalizations: FuturesOrdered::new(),
            extra_data_registry: cfg.extra_data_registry,
            pending_finalizations_gauge,
            has_advanced_since_init: false,
            canonical_state: cfg.canonical_state,
        };
        (actor, mailbox)
    }

    pub fn start(mut self) -> Handle<()> {
        spawn_cell!(self.context, self.run().await)
    }

    async fn run(mut self) {
        info_span!("start").in_scope(|| info!("executor starting"));

        loop {
            if self.pending_backfill.is_none() {
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
                            if let Err(error) = self.forward_finalized(span, block, ack).await {
                                error_span!("shutdown").in_scope(|| error!(%error,
                                    "executor fatal error during backfill; shutting down"));
                                break;
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
                if self.pending_backfill.is_none()
                    && self.finalized_heights_to_backfill.is_empty() => {
                    self.pending_finalizations_gauge
                        .set(self.pending_finalizations.len() as i64);
                    if let Err(error) = self.forward_finalized(cause, block, ack).await {
                        error_span!("shutdown").in_scope(|| error!(%error,
                            "executor fatal error during finalize; shutting down"));
                        break;
                    }
                }

                msg = self.mailbox.recv() => {
                    let Some(msg) = msg else { break; };
                    if let Err(error) = self.handle_message(msg).await {
                        error_span!("shutdown").in_scope(|| error!(%error,
                            "executor fatal error during canonicalize; shutting down"));
                        break;
                    }
                }

                _ = (&mut self.fcu_heartbeat_timer).fuse() => {
                    self.send_forkchoice_update_heartbeat().await;
                    self.reset_fcu_heartbeat_timer();
                }
            }
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
                "FCU heartbeat suppressed; no consensus advance since cold-start init \
                 (sending stale head would force reth rollback if verify-path new_payload \
                 already promoted block_{{N+1}} to canonical chain)"
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
            .fork_choice_updated(self.last_canonicalized.forkchoice, None)
            .pace(&self.context, self.fcu_pace)
            .await;
        if let Err(error) = resp {
            warn!(%error, "heartbeat FCU failed");
        }
    }

    async fn handle_message(&mut self, message: Message<Attrs>) -> eyre::Result<()> {
        let cause = message.cause;
        let is_backfilling =
            !self.pending_backfill.is_none() || !self.finalized_heights_to_backfill.is_empty();
        match message.command {
            Command::CanonicalizeAndBuild(CanonicalizeAndBuild { response, .. })
                if is_backfilling =>
            {
                info_span!("handle_message")
                    .in_scope(|| info!("canonicalize_and_build dropped while backfilling"));
                let _ = response.send(Err(CanonicalizeError::BackfillInProgress));
            }
            Command::CanonicalizeAndBuild(CanonicalizeAndBuild {
                height,
                digest,
                attributes,
                extra_data,
                response,
            }) => {
                self.canonicalize(
                    cause,
                    HeadOrFinalized::Head,
                    height,
                    digest,
                    JustCanonicalizeOrAlsoBuild::AlsoBuild {
                        response,
                        attributes,
                        extra_data,
                    },
                )
                .await;
            }
            Command::Finalize(finalized) => match *finalized {
                // `finalized` advances only via the ingested `Update::Block` path
                // below. A `Tip` carries no block, so acting on it would FCU a
                // block reth may not hold — the busy-spin SYNCING wedge. Ignored
                // by design; the marshal back-fills the gap and emits `Block`.
                Update::Tip(..) => {}
                Update::Block(block, ack) => {
                    self.pending_finalizations
                        .push_back(ready((cause, block, ack)));
                    self.pending_finalizations_gauge
                        .set(self.pending_finalizations.len() as i64);
                }
            },
        }
        Ok(())
    }

    #[instrument(
        skip_all, parent = &cause,
        fields(head.height = %height, head.digest = %digest, %head_or_finalized),
    )]
    async fn canonicalize(
        &mut self,
        cause: Span,
        head_or_finalized: HeadOrFinalized,
        height: Height,
        digest: Digest,
        maybe_build: JustCanonicalizeOrAlsoBuild<Attrs>,
    ) {
        let new = match head_or_finalized {
            HeadOrFinalized::Head => self.last_canonicalized.update_head(height, digest),
            HeadOrFinalized::Finalized => self.last_canonicalized.update_finalized(height, digest),
        };

        if new == self.last_canonicalized {
            if let JustCanonicalizeOrAlsoBuild::JustCanonicalize { response } = maybe_build {
                let _ = response.send(Ok(()));
                return;
            }
            // NB: an `AlsoBuild` with `new == last_canonicalized` (a no-op FCU)
            // falls through here and does NOT arm `has_advanced_since_init` —
            // that flag is set at the success tail, guarded on `new != prev`.
        }

        // Spec-compliant ancestor-FCU guard (per Engine API paris.md):
        //
        //   "Client software MAY skip an update of the forkchoice state
        //    and MUST NOT begin a payload build process if
        //    forkchoiceState.headBlockHash references a VALID ancestor
        //    of the head of canonical chain."
        //
        // Detection: if reth's canonical_state.chain_info() reports a
        // strictly higher block number than our staged `new.head_height`,
        // reth has been silently advanced by `FluentApp::verify`'s direct
        // `new_payload` calls (verify path bypasses executor).
        //
        // Per spec, sending FCU(head=ancestor, attrs=Some) MUST return
        // `{Valid, payload_id: null}` and skip payload build (paradigmxyz/reth
        // #16629 tracks reth returning exactly that `{VALID, null payloadId}`
        // post-reorg, which strands a builder with no payload_id; PR #16676's
        // `--engine.always-process-payload-attributes-on-canonical-head` is the
        // opt-in fix). Rather than depend on that flag, we skip the FCU+attrs
        // entirely when reth's canonical head is already ahead of our staged
        // head: return `BackfillInProgress` to the propose path, which causes
        // `FluentApp::propose` to return `None` for this view. Consensus
        // rotates to the next view's leader; by then either the race is
        // gone (no verify-path advance) OR `last_canonicalized` will have
        // advanced via a real finalize event.
        //
        // Guard skipped for `JustCanonicalize` (Update::Finalized path):
        // those FCUs come from genuine consensus finalization events
        // where rollback is the CORRECT semantic (consensus has changed
        // its mind about which fork to canonicalize); reth handles that
        // path differently.
        //
        // Guard skipped when `canonical_state` is uninitialised
        // (`best_hash == B256::ZERO && best_number == 0`) — tests
        // construct with `CanonicalInMemoryState::empty()`; real reth
        // always reports at minimum the genesis header.
        if matches!(maybe_build, JustCanonicalizeOrAlsoBuild::AlsoBuild { .. }) {
            let reth_info = self.canonical_state.chain_info();
            let canonical_initialised =
                reth_info.best_hash != B256::ZERO || reth_info.best_number != 0;
            info!(
                stage = "ancestor_fcu_guard_check",
                staging_head_number = %new.head_height,
                staging_head_hash = %new.forkchoice.head_block_hash,
                reth_head_number = reth_info.best_number,
                reth_head_hash = %reth_info.best_hash,
                canonical_initialised,
                "evaluating ancestor-FCU guard"
            );
            if canonical_initialised && reth_info.best_number > new.head_height.get() {
                info!(
                    staging_head_number = %new.head_height,
                    staging_head_hash = %new.forkchoice.head_block_hash,
                    reth_head_number = reth_info.best_number,
                    reth_head_hash = %reth_info.best_hash,
                    "ANCESTOR_FCU_GUARD: reth canonical advanced past staged head \
                     via verify-path new_payload — skipping FCU+attrs"
                );
                maybe_build.send_error(CanonicalizeError::BackfillInProgress);
                return;
            }
        }

        let attrs = match &maybe_build {
            JustCanonicalizeOrAlsoBuild::AlsoBuild { attributes, .. } => {
                Some((**attributes).clone())
            }
            JustCanonicalizeOrAlsoBuild::JustCanonicalize { .. } => None,
        };

        let fcu = match self
            .beacon_engine
            .fork_choice_updated(new.forkchoice, attrs)
            .pace(&self.context, self.fcu_pace)
            .await
            .wrap_err("FCU failed")
        {
            Err(error) => {
                maybe_build.send_error(CanonicalizeError::EngineError(error));
                return;
            }
            Ok(r) => r,
        };

        if !fcu.is_valid() {
            maybe_build.send_error(CanonicalizeError::EngineError(eyre::eyre!(
                "EL reported invalid FCU: {:?}",
                fcu.payload_status
            )));
            return;
        }

        match maybe_build {
            JustCanonicalizeOrAlsoBuild::JustCanonicalize { response } => {
                let _ = response.send(Ok(()));
            }
            JustCanonicalizeOrAlsoBuild::AlsoBuild {
                response,
                extra_data,
                ..
            } => match fcu.payload_id {
                Some(payload_id) => {
                    if !extra_data.is_empty() {
                        self.extra_data_registry.insert(payload_id, extra_data);
                    }
                    let _ = response.send(Ok(payload_id));
                }
                None => {
                    let _ = response.send(Err(CanonicalizeError::PayloadIdMissing));
                }
            },
        }
        // Arm heartbeat-suppression release ONLY when the canonical state
        // actually advanced (and the FCU was valid — we only reach here on a
        // valid FCU). A no-op `AlsoBuild` FCU (`new == prev`) must not arm it,
        // or a later heartbeat would resend a stale head.
        if new != self.last_canonicalized {
            self.has_advanced_since_init = true;
        }
        self.last_canonicalized = new;
        self.reset_fcu_heartbeat_timer();
    }

    #[instrument(
        skip_all, parent = &cause,
        fields(block.digest = %block.digest(), block.height = %block.height()),
        err(level = Level::WARN), ret,
    )]
    async fn forward_finalized(
        &mut self,
        cause: Span,
        block: Block,
        ack: Exact,
    ) -> eyre::Result<()> {
        // S5: ingest the block (and heal any missing-ancestor gap) into reth
        // BEFORE the finalize FCU references it. The old order (FCU first,
        // new_payload second) trapped reth in missing-block SYNCING whenever
        // this node had never verified the finalized block — and with
        // `connected_peers = 0` (DPoS distributes blocks over commonware p2p,
        // not reth devp2p) reth's recovery download never completes, wedging
        // the node. Live-reproduced 2026-05-28 (val-0).
        self.ensure_block_and_ancestors_present(&block).await?;
        let (response, rx) = oneshot::channel();
        self.canonicalize(
            Span::current(),
            HeadOrFinalized::Finalized,
            block.height(),
            block.digest(),
            JustCanonicalizeOrAlsoBuild::JustCanonicalize { response },
        )
        .await;
        let canon_res = rx.await.wrap_err("canonicalize dropped")?;
        canon_res.map_err(|e| eyre::eyre!("canonicalize failed: {e}"))?;

        ack.acknowledge();
        Ok(())
    }

    /// Ensure `block` is present in reth, healing a missing-ancestor gap via
    /// the marshal first. `new_payload` is idempotent, so for blocks this node
    /// already verified this is a single no-op call; the ancestor-walk only
    /// engages when consensus finalized a block (or chain) this node never
    /// ingested — the live divergence case. Bounded by [`MAX_GAP_HEAL`].
    async fn ensure_block_and_ancestors_present(&mut self, block: &Block) -> eyre::Result<()> {
        let status = self
            .beacon_engine
            .new_payload(block.clone().into_inner())
            .pace(&self.context, self.fcu_pace)
            .await
            .wrap_err("new_payload(finalized) failed")?;
        if status.is_valid() {
            return Ok(());
        }
        // Syncing here == Disconnected: reth buffered the block but its parent
        // is missing. Walk ancestors oldest-ward via the marshal until one
        // reconnects to reth's known chain.
        ensure!(
            status.is_syncing(),
            "EL reported non-valid/non-syncing for finalized block: `{status:?}`"
        );

        let mut missing: Vec<Block> = Vec::new();
        let mut height = block.height();
        let reconnected = loop {
            if missing.len() as u64 >= MAX_GAP_HEAL || height.get() == 0 {
                break false;
            }
            let parent_h = Height::new(height.get() - 1);
            let Some(parent) = self.marshal.fetch_block_by_height(parent_h).await else {
                eyre::bail!(
                    "S5 gap-heal: marshal has no block at height {parent_h}; gap exceeds \
                     marshal retention — operator: re-sync EL disk from a recent snapshot"
                );
            };
            let s = self
                .beacon_engine
                .new_payload(parent.clone().into_inner())
                .pace(&self.context, self.fcu_pace)
                .await
                .wrap_err("new_payload(ancestor) failed")?;
            missing.push(parent);
            if s.is_valid() {
                break true;
            }
            ensure!(
                s.is_syncing(),
                "EL reported invalid for ancestor {parent_h}: `{s:?}`"
            );
            height = parent_h;
        };
        ensure!(
            reconnected,
            "S5 gap-heal exhausted ({MAX_GAP_HEAL}) without reconnecting finalized block {}",
            block.height()
        );

        // Re-feed the collected ancestors oldest-first so reth deterministically
        // connects the buffered chain, then retry the target. Propagate errors
        // (don't swallow) so a replay failure isn't misreported as exhaustion.
        for ancestor in missing.into_iter().rev() {
            let ah = ancestor.height();
            let s = self
                .beacon_engine
                .new_payload(ancestor.into_inner())
                .pace(&self.context, self.fcu_pace)
                .await
                .wrap_err("new_payload(ancestor replay) failed")?;
            ensure!(
                s.is_valid() || s.is_syncing(),
                "EL reported invalid for ancestor replay {ah}: `{s:?}`"
            );
        }
        let final_status = self
            .beacon_engine
            .new_payload(block.clone().into_inner())
            .pace(&self.context, self.fcu_pace)
            .await
            .wrap_err("new_payload(finalized retry) failed")?;
        ensure!(
            final_status.is_valid(),
            "S5 gap-heal: finalized block {} still not valid after reconnect",
            block.height()
        );
        Ok(())
    }
}

enum JustCanonicalizeOrAlsoBuild<Attrs> {
    JustCanonicalize {
        response: oneshot::Sender<Result<(), CanonicalizeError>>,
    },
    AlsoBuild {
        response: oneshot::Sender<Result<PayloadId, CanonicalizeError>>,
        attributes: Box<Attrs>,
        extra_data: Bytes,
    },
}

impl<Attrs> JustCanonicalizeOrAlsoBuild<Attrs> {
    fn send_error(self, error: CanonicalizeError) {
        match self {
            Self::JustCanonicalize { response } => {
                let _ = response.send(Err(error));
            }
            Self::AlsoBuild { response, .. } => {
                let _ = response.send(Err(error));
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeadOrFinalized {
    Head,
    Finalized,
}

impl std::fmt::Display for HeadOrFinalized {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Head => "head",
            Self::Finalized => "finalized",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{Block as AlloyBlock, BlockBody, Header as AlloyHeader};
    use alloy_primitives::U256;
    use alloy_rpc_types_engine::{ForkchoiceUpdated, PayloadStatus, PayloadStatusEnum};
    use commonware_runtime::{deterministic, Runner as _};
    use reth_ethereum_primitives::TransactionSigned;
    use reth_primitives_traits::SealedBlock as RethSealed;
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };

    type RethExecBlock = RethSealed<reth_ethereum_primitives::Block>;

    fn sample_block(parent: B256, number: u64) -> Block {
        let header = AlloyHeader {
            parent_hash: parent,
            number,
            gas_limit: 30_000_000,
            timestamp: 1_700_000_000,
            difficulty: U256::ZERO,
            ..Default::default()
        };
        let body: BlockBody<TransactionSigned> = BlockBody::default();
        let alloy_block: AlloyBlock<TransactionSigned> = AlloyBlock::new(header, body);
        Block::from_execution_block(RethSealed::seal_slow(
            reth_ethereum_primitives::Block::from(alloy_block),
        ))
    }

    #[derive(Clone, Default)]
    struct FakeBeacon {
        fcu_calls: Arc<Mutex<Vec<ForkchoiceState>>>,
        new_payload_calls: Arc<Mutex<Vec<RethExecBlock>>>,
    }

    impl crate::application::BeaconEngineLike for FakeBeacon {
        type PayloadAttrs = ();
        type ExecutionData = RethExecBlock;

        async fn fork_choice_updated(
            &self,
            state: ForkchoiceState,
            _attrs: Option<()>,
        ) -> eyre::Result<ForkchoiceUpdated> {
            self.fcu_calls.lock().unwrap().push(state);
            Ok(ForkchoiceUpdated::from_status(PayloadStatusEnum::Valid))
        }

        async fn new_payload(&self, data: RethExecBlock) -> eyre::Result<PayloadStatus> {
            self.new_payload_calls.lock().unwrap().push(data);
            Ok(PayloadStatus::from_status(PayloadStatusEnum::Valid))
        }
    }

    #[derive(Clone, Default)]
    struct FakeMarshal {
        canned: Arc<Mutex<BTreeMap<u64, Block>>>,
    }

    impl BlockFetcher for FakeMarshal {
        async fn fetch_block_by_height(&self, height: Height) -> Option<Block> {
            self.canned.lock().unwrap().get(&height.get()).cloned()
        }
    }

    /// Beacon fake modelling reth's missing-parent semantics: `new_payload`
    /// returns Valid only when the block's parent is genesis (ZERO) or already
    /// ingested, else Syncing (buffered/Disconnected); FCU returns Valid only
    /// when the head block was ingested, else Syncing. Lets the S5 gap-heal be
    /// exercised the way the live reth trap behaves.
    #[derive(Clone, Default)]
    struct ConnectivityBeacon {
        present: Arc<Mutex<std::collections::HashSet<B256>>>,
        fcu_calls: Arc<Mutex<Vec<ForkchoiceState>>>,
        new_payload_calls: Arc<Mutex<Vec<B256>>>,
    }

    impl crate::application::BeaconEngineLike for ConnectivityBeacon {
        type PayloadAttrs = ();
        type ExecutionData = RethExecBlock;

        async fn fork_choice_updated(
            &self,
            state: ForkchoiceState,
            _attrs: Option<()>,
        ) -> eyre::Result<ForkchoiceUpdated> {
            self.fcu_calls.lock().unwrap().push(state);
            let status = if self.present.lock().unwrap().contains(&state.head_block_hash) {
                PayloadStatusEnum::Valid
            } else {
                PayloadStatusEnum::Syncing
            };
            Ok(ForkchoiceUpdated::from_status(status))
        }

        async fn new_payload(&self, data: RethExecBlock) -> eyre::Result<PayloadStatus> {
            let hash = data.hash();
            let parent = data.header().parent_hash;
            self.new_payload_calls.lock().unwrap().push(hash);
            let mut present = self.present.lock().unwrap();
            if parent == B256::ZERO || present.contains(&parent) {
                present.insert(hash);
                Ok(PayloadStatus::from_status(PayloadStatusEnum::Valid))
            } else {
                Ok(PayloadStatus::from_status(PayloadStatusEnum::Syncing))
            }
        }
    }

    fn build_actor<BE>(
        ctx: deterministic::Context,
        beacon: BE,
        marshal: FakeMarshal,
        last_consensus: u64,
        last_exec: u64,
    ) -> (Actor<deterministic::Context, BE, (), FakeMarshal>, Mailbox<()>)
    where
        BE: crate::application::BeaconEngineLike<PayloadAttrs = (), ExecutionData = RethExecBlock>
            + Send
            + Sync
            + 'static,
    {
        Actor::init(
            ctx,
            Config {
                beacon_engine: beacon,
                marshal,
                fcu_heartbeat_interval: Duration::from_secs(60),
                last_consensus_finalized_height: Height::new(last_consensus),
                last_execution_finalized_height: last_exec,
                initial_finalized: (Height::new(0), B256::ZERO),
                initial_head: (Height::new(0), B256::ZERO),
                fcu_pace: Duration::from_millis(0),
                extra_data_registry: Arc::new(DashMap::new()),
                canonical_state: reth_chain_state::CanonicalInMemoryState::empty(),
            },
        )
    }

    #[test]
    fn forward_finalized_sends_fcu_and_new_payload_and_acks() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let beacon = FakeBeacon::default();
            let marshal = FakeMarshal::default();
            let (actor, mailbox) = build_actor(ctx, beacon.clone(), marshal, 0, 0);
            let handle = actor.start();

            let block = sample_block(B256::ZERO, 1);
            let block_hash = block.block_hash();
            let (ack, waiter) = Exact::handle();

            mailbox
                .send(Message {
                    cause: Span::current(),
                    command: Command::Finalize(Box::new(Update::Block(block, ack))),
                })
                .expect("send finalize");

            waiter.await.expect("ack resolves");

            assert!(
                !beacon.fcu_calls.lock().unwrap().is_empty(),
                "FCU must have been called"
            );
            {
                let payloads = beacon.new_payload_calls.lock().unwrap();
                assert_eq!(payloads.len(), 1, "exactly one new_payload call");
                assert_eq!(payloads[0].hash(), block_hash);
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    #[test]
    fn backfill_drains_before_finalize() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let beacon = FakeBeacon::default();
            // Canned blocks for heights 1, 2, 3 — to be backfilled on startup.
            let canned = [
                (1u64, sample_block(B256::ZERO, 1)),
                (2u64, sample_block(B256::ZERO, 2)),
                (3u64, sample_block(B256::ZERO, 3)),
            ]
            .into_iter()
            .collect::<BTreeMap<_, _>>();
            let marshal = FakeMarshal {
                canned: Arc::new(Mutex::new(canned)),
            };

            // consensus_last=3, exec_last=0 → backfill range 1..=3.
            let (actor, mailbox) = build_actor(ctx, beacon.clone(), marshal, 3, 0);
            let handle = actor.start();

            // Push a new finalize for height 4 BEFORE backfill drains.
            let new_block = sample_block(B256::ZERO, 4);
            let (ack, waiter) = Exact::handle();
            mailbox
                .send(Message {
                    cause: Span::current(),
                    command: Command::Finalize(Box::new(Update::Block(new_block, ack))),
                })
                .expect("send finalize");

            waiter.await.expect("ack for height 4");

            {
                let payloads = beacon.new_payload_calls.lock().unwrap();
                assert_eq!(payloads.len(), 4);
                let heights: Vec<u64> = payloads.iter().map(|p| p.number).collect();
                assert_eq!(heights, vec![1, 2, 3, 4]);
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // S5 regression: reproduces the live val-0 trap — consensus finalizes a
    // block whose parent this node never verified. Pre-S5 the finalize FCU
    // referenced the missing block → reth SYNCING → deadlock (never acked).
    // Post-S5 the gap is healed from the marshal before the FCU, so the
    // finalization completes deterministically.
    #[test]
    fn forward_finalized_heals_missing_ancestor_via_marshal() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            // Chain: block7 (anchor — reth already has it) <- block8 (only in
            // the marshal, never verified here) <- block9 (finalized, delivered
            // directly via the Finalize command).
            let block7 = sample_block(B256::ZERO, 7);
            let h7 = block7.block_hash();
            let block8 = sample_block(h7, 8);
            let h8 = block8.block_hash();
            let block9 = sample_block(h8, 9);
            let h9 = block9.block_hash();

            let beacon = ConnectivityBeacon::default();
            beacon.present.lock().unwrap().insert(h7); // reth has the anchor only

            let marshal = FakeMarshal {
                canned: Arc::new(Mutex::new([(8u64, block8)].into_iter().collect())),
            };

            // consensus_last=9, exec_last=7 → backfill range 8..=9 drains first;
            // block9 is the finalize we assert on.
            let (actor, mailbox) = build_actor(ctx, beacon.clone(), marshal, 7, 7);
            let handle = actor.start();

            let (ack, waiter) = Exact::handle();
            mailbox
                .send(Message {
                    cause: Span::current(),
                    command: Command::Finalize(Box::new(Update::Block(block9, ack))),
                })
                .expect("send finalize");

            // Must resolve — pre-S5 this deadlocked (FCU→SYNCING, never acked).
            waiter
                .await
                .expect("finalize of block9 must ack after marshal gap-heal");

            {
                let present = beacon.present.lock().unwrap();
                assert!(
                    present.contains(&h8),
                    "missing ancestor block8 must be healed from the marshal"
                );
                assert!(present.contains(&h9), "finalized block9 must be ingested");
            }
            {
                // The finalize FCU referenced a block reth now has → head=block9.
                let fcus = beacon.fcu_calls.lock().unwrap();
                assert_eq!(
                    fcus.last().expect("at least one FCU").head_block_hash,
                    h9,
                    "finalize FCU head must be the finalized block"
                );
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    #[test]
    fn finalized_tip_issues_no_fcu() {
        // Regression guard: Update::Tip must be inert — finalized advances only
        // via the ingested Update::Block path. A Tip->FCU path would resurrect
        // the busy-spin SYNCING wedge (a finalized-FCU for a block reth lacks).
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let beacon = FakeBeacon::default();
            let marshal = FakeMarshal::default();
            let (actor, mailbox) = build_actor(ctx, beacon.clone(), marshal, 0, 0);
            let handle = actor.start();

            // A finalized tip for a block the executor never ingested.
            let tip = sample_block(B256::ZERO, 5);
            mailbox
                .send(Message {
                    cause: Span::current(),
                    command: Command::Finalize(Box::new(Update::Tip(
                        Round::new(Epoch::new(0), View::new(5)),
                        Height::new(5),
                        tip.digest(),
                    ))),
                })
                .expect("send tip");

            // Follow with an observable Block finalize (which DOES issue exactly
            // one FCU) so we can be sure the loop has drained the (no-op) Tip
            // before asserting — the single FCU we observe is the Block's,
            // proving the Tip produced none.
            let block = sample_block(B256::ZERO, 1);
            let block_hash = block.block_hash();
            let (ack, waiter) = Exact::handle();
            mailbox
                .send(Message {
                    cause: Span::current(),
                    command: Command::Finalize(Box::new(Update::Block(block, ack))),
                })
                .expect("send finalize");
            waiter.await.expect("ack for height 1");

            // Only the Block FCU — the Tip produced none.
            {
                let fcus = beacon.fcu_calls.lock().unwrap();
                assert_eq!(fcus.len(), 1, "Update::Tip must not issue an FCU");
                assert_eq!(fcus[0].head_block_hash, block_hash);
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }
}
