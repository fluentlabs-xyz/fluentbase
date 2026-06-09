//! Drives reth EL forwarding + forkchoice state from consensus.

use crate::{
    application::BeaconEngineLike,
    block::Block,
    digest::Digest,
    executor::ingress::{CanonicalizeAndBuild, CanonicalizeError, Command, Mailbox, Message},
};
use alloy_primitives::B256;
use alloy_rpc_types_engine::{ForkchoiceState, PayloadId};
use commonware_consensus::{marshal::Update, types::Height, Heightable as _};
use commonware_runtime::{spawn_cell, Clock, ContextCell, FutureExt as _, Handle, Pacer, Spawner};
use commonware_utils::{acknowledgement::Exact, futures::OptionFuture, Acknowledgement as _};
use eyre::{ensure, WrapErr as _};
use futures::{
    channel::oneshot,
    future::{ready, BoxFuture, Ready},
    stream::FuturesOrdered,
    FutureExt as _, StreamExt as _,
};
use prometheus_client::metrics::gauge::Gauge;
use std::{ops::RangeInclusive, pin::Pin, time::Duration};
use tokio::{select, sync::mpsc};
use tracing::{
    debug, error, error_span, info, info_span, instrument, warn, warn_span, Level, Span,
};

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
        // Reached ONLY from the propose path (`CanonicalizeAndBuild`), which
        // carries the consensus-chosen parent — the block consensus decided to
        // build on. That choice is authoritative, so a lower-height parent on
        // the finalized fork (a legitimate reorg of an unfinalized tail — e.g.
        // the Tempo→DPoS migration cold-start where reth's head sits on an
        // orphaned tail) MUST be allowed to roll the head back. There is no
        // spurious-rollback path to guard against: FCU heartbeats bypass
        // `update_head` (see `send_forkchoice_update_heartbeat`), and
        // finalized-monotonicity is held by `update_finalized` + the EL
        // refusing sub-finalized reorgs.
        if height > this.finalized_height || digest.0 == this.forkchoice.finalized_block_hash {
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

/// Explicit impl for the concrete marshal mailbox.
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
    /// A stale head
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
                    },
                )
                .await;
            }
            Command::Finalize(finalized) => match *finalized {
                // FCU-drive reth's head toward the consensus tip so reth's
                // missing-block handler bulk-downloads the gap over devp2p (needs
                // peering). head only — safe/finalized stay at last_canonicalized,
                // and last_canonicalized is NOT mutated: the tip isn't
                // finalized/executed yet; forward_finalized advances state when the
                // block actually finalizes. Guard is height-only, deliberately NOT
                // has_advanced_since_init (unlike the heartbeat): the tip-FCU only
                // moves head FORWARD so there is no stale-rollback to guard, and
                // gating on has_advanced_since_init would deadlock rejoin —
                // canonicalize can't fire until reth holds the blocks, which needs
                // this very FCU to start the devp2p backfill.
                Update::Tip(_round, height, digest) => {
                    if height > self.last_canonicalized.head_height {
                        let mut fc = self.last_canonicalized.forkchoice;
                        fc.head_block_hash = digest.0;
                        info!(%height, head = %digest,
                            "FCU-drive toward consensus tip (reth devp2p catch-up)");
                        if let Err(error) = self
                            .beacon_engine
                            .fork_choice_updated(fc, None)
                            .pace(&self.context, self.fcu_pace)
                            .await
                        {
                            warn!(%error, "tip-drive FCU failed");
                        }
                    }
                }
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

            // The proposer pre-registered extra_data under
            // payload_id(digest, attrs); if update_head retained the old head
            // (proposing on/below finalized, or the no-op AlsoBuild fall-through
            // above) reth builds under a divergent head and never reads that
            // entry — the prev-finalized bitmap is silently dropped this block.
            // Fail-safe (== cold-start empty), but warn so the miss is
            // observable instead of indistinguishable from a genuine cold start.
            if new.forkchoice.head_block_hash != digest.0 {
                warn!(
                    proposed_parent = %digest,
                    fcu_head = %new.forkchoice.head_block_hash,
                    "liveness extra_data registry miss: FCU head != proposed parent"
                );
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

        // Only a genuinely INVALID FCU is fatal. A SYNCING FCU is accepted: on
        // rejoin the finalize FCU references a block reth is still backfilling
        // over devp2p (head ahead of reth's executed tip). The Finalized path
        // proceeds (last_canonicalized advances; the heartbeat re-sends head
        // until reth reports VALID); the AlsoBuild/propose path still needs a
        // payload_id, which SYNCING won't carry → PayloadIdMissing → the
        // proposer skips that view (a backfilling node should not be proposing).
        if fcu.is_invalid() {
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
            JustCanonicalizeOrAlsoBuild::AlsoBuild { response, .. } => match fcu.payload_id {
                Some(payload_id) => {
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
        // Submit the block to reth BEFORE the finalize FCU references it (S5
        // ordering: new_payload first avoids the old FCU-first missing-block
        // trap). A SYNCING result is accepted, not healed: reth devp2p (peering
        // restored under --dpos) bulk-downloads the gap and the finalize FCU +
        // heartbeat reconcile once backfill lands. Walking the marshal to heal
        // here would mis-read the EXPECTED backfill SYNCING as a fatal gap.
        self.submit_finalized_payload(&block).await?;
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

    /// Submit a finalized `block` to reth via `new_payload`. VALID means reth
    /// holds the parent and executed it; SYNCING means reth buffered it while
    /// devp2p backfills the missing-ancestor gap (the expected steady state on
    /// rejoin once peering is restored). Both are accepted — reth + the finalize
    /// FCU heartbeat reconcile when the backfill lands. Only a genuinely INVALID
    /// payload is fatal.
    async fn submit_finalized_payload(&mut self, block: &Block) -> eyre::Result<()> {
        let status = self
            .beacon_engine
            .new_payload(block.clone().into_inner())
            .pace(&self.context, self.fcu_pace)
            .await
            .wrap_err("new_payload(finalized) failed")?;
        ensure!(
            status.is_valid() || status.is_syncing(),
            "EL reported non-valid/non-syncing for finalized block: `{status:?}`"
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
    /// when the head block was ingested, else Syncing. Lets the gap-heal be
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
            let status = if self
                .present
                .lock()
                .unwrap()
                .contains(&state.head_block_hash)
            {
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
    ) -> (
        Actor<deterministic::Context, BE, (), FakeMarshal>,
        Mailbox<()>,
    )
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
                canonical_state: reth_chain_state::CanonicalInMemoryState::empty(),
            },
        )
    }

    #[test]
    fn update_head_rolls_back_to_finalized_fork() {
        // Regression guard: a consensus-directed canonicalize on the finalized
        // fork at a height BELOW the staged head (cold-start migration: head
        // seeded on an unfinalized tail) must roll the head back to the anchor.
        // The removed `&& height >= head_height` guard pinned it to the tail.
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

        let rolled = lc.update_head(Height::new(10), Digest(anchor));
        assert_eq!(
            rolled.head_height,
            Height::new(10),
            "head rolls back to anchor"
        );
        assert_eq!(rolled.forkchoice.head_block_hash, anchor);

        let other = B256::repeat_byte(0x09);
        let unchanged = lc.update_head(Height::new(9), Digest(other));
        assert_eq!(
            unchanged.head_height,
            Height::new(12),
            "no move off-fork below finalized"
        );
        assert_eq!(unchanged.forkchoice.head_block_hash, tail);
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

    // Rejoin: consensus finalizes a block whose parent reth does not yet hold
    // (reth is backfilling the gap over devp2p). Both new_payload(finalized) and
    // the finalize FCU return SYNCING. The executor must ACCEPT SYNCING — ack the
    // block (no deadlock, no crash) and still issue the finalize FCU(head=block9)
    // so reth keeps the target — rather than healing via the marshal or bailing.
    // Healing is devp2p's job now (out of unit scope); peering is the premise.
    #[test]
    fn forward_finalized_accepts_syncing_during_backfill() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            // block7 (anchor reth has) <- block8 (MISSING — being devp2p-synced)
            // <- block9 (finalized, delivered directly). block8 is in NO marshal:
            // the direct-delivery path does not consult the marshal to heal.
            let block7 = sample_block(B256::ZERO, 7);
            let h7 = block7.block_hash();
            let block8 = sample_block(h7, 8);
            let h8 = block8.block_hash();
            let block9 = sample_block(h8, 9);
            let h9 = block9.block_hash();

            let beacon = ConnectivityBeacon::default();
            beacon.present.lock().unwrap().insert(h7); // reth has the anchor only

            let marshal = FakeMarshal::default(); // empty — heal must NOT be used

            // exec_last == consensus_last == 7 → no startup backfill; block9 is
            // delivered live and is the finalize we assert on.
            let (actor, mailbox) = build_actor(ctx, beacon.clone(), marshal, 7, 7);
            let handle = actor.start();

            let (ack, waiter) = Exact::handle();
            mailbox
                .send(Message {
                    cause: Span::current(),
                    command: Command::Finalize(Box::new(Update::Block(block9, ack))),
                })
                .expect("send finalize");

            // Must resolve under SYNCING — the whole point of the SYNCING gap-heal
            // (old walk-and-bail would have crashed the executor; the ack never fires
            // on crash and this await would hang then fail on dropped mailbox).
            waiter
                .await
                .expect("finalized block must ack under SYNCING (accept, not heal)");

            {
                // No marshal heal: reth never got block8/block9 marked present.
                let present = beacon.present.lock().unwrap();
                assert!(!present.contains(&h8), "block8 must NOT be marshal-healed");
                assert!(!present.contains(&h9), "block9 stays buffered (SYNCING)");
            }
            {
                // The finalize FCU was still issued with head=block9 despite
                // SYNCING, so reth keeps backfilling toward the finalized tip.
                let fcus = beacon.fcu_calls.lock().unwrap();
                assert_eq!(
                    fcus.last().expect("at least one FCU").head_block_hash,
                    h9,
                    "finalize FCU head must be the finalized block even under SYNCING"
                );
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    #[test]
    fn finalized_tip_drives_fcu_toward_tip() {
        // Update::Tip(height > head) FCU-drives reth's head toward the consensus
        // tip (so reth bulk-downloads the gap over devp2p), WITHOUT mutating
        // last_canonicalized; a Tip at height <= head stays inert (no spurious
        // rollback FCU). Drains are observed via a following Block finalize.
        use commonware_consensus::types::{Epoch, Round, View};
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let beacon = FakeBeacon::default();
            let marshal = FakeMarshal::default();
            let (actor, mailbox) = build_actor(ctx, beacon.clone(), marshal, 0, 0);
            let handle = actor.start();

            // Tip at height 5 (> initial head 0) must FCU-drive head = tip digest.
            let tip = sample_block(B256::ZERO, 5);
            let tip_hash = tip.block_hash();
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

            // Drain barrier: a Block(height 1) finalize we await, ensuring the Tip
            // was processed first. The finalize itself issues one FCU(head=block1).
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

            {
                let fcus = beacon.fcu_calls.lock().unwrap();
                assert_eq!(fcus.len(), 2, "tip-drive FCU + block-finalize FCU");
                assert_eq!(
                    fcus[0].head_block_hash, tip_hash,
                    "tip-drive FCU head = tip digest"
                );
                assert_eq!(
                    fcus[1].head_block_hash, block_hash,
                    "finalize FCU head = finalized block"
                );
            }

            // A Tip at height <= current head (now 1) must stay inert.
            let stale_tip = sample_block(B256::ZERO, 1);
            mailbox
                .send(Message {
                    cause: Span::current(),
                    command: Command::Finalize(Box::new(Update::Tip(
                        Round::new(Epoch::new(0), View::new(6)),
                        Height::new(1),
                        stale_tip.digest(),
                    ))),
                })
                .expect("send stale tip");

            // Drain barrier: Block(height 2). Only this adds an FCU — the stale
            // Tip (height == head) produced none.
            let block2 = sample_block(block_hash, 2);
            let block2_hash = block2.block_hash();
            let (ack2, waiter2) = Exact::handle();
            mailbox
                .send(Message {
                    cause: Span::current(),
                    command: Command::Finalize(Box::new(Update::Block(block2, ack2))),
                })
                .expect("send finalize 2");
            waiter2.await.expect("ack for height 2");

            {
                let fcus = beacon.fcu_calls.lock().unwrap();
                assert_eq!(
                    fcus.len(),
                    3,
                    "stale Tip (height <= head) issues no FCU; only block2 adds one"
                );
                assert_eq!(fcus[2].head_block_hash, block2_hash);
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }
}
