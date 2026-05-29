//! Fluent Application: bridges commonware consensus ⇄ reth EL.
//!
//! `propose` and `report` route through [`crate::executor::Actor`] via
//! its [`crate::executor::Mailbox`]; `verify` keeps the direct
//! `BeaconEngineLike::new_payload` call (idempotent — doesn't advance
//! state).
//!
//! Trait implementations:
//!   - [`Application<E>`]: high-level, with `AncestorStream` ancestry.
//!   - [`VerifyingApplication<E>`]: same shape, returns `bool`.
//!   - [`Reporter<Activity = Update<Block>>`]: fed by `marshal::core::Actor`.
//!
//! NOT implemented: `Relay`. The `marshal::standard::Inline` wrapper
//! provides `Relay` (inline.rs:471); `FluentApp` does not.

use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use alloy_primitives::Bytes;
use alloy_rpc_types_engine::{ForkchoiceState, ForkchoiceUpdated, PayloadId, PayloadStatus};
use commonware_consensus::{
    marshal::{
        ancestry::{AncestorStream, BlockProvider},
        core::Mailbox as MarshalMailbox,
        standard::Standard,
        Update,
    },
    simplex::types::Context as SimplexContext,
    types::{Height, Round},
    Application, Heightable as _, Reporter, VerifyingApplication,
};
use commonware_cryptography::{certificate::Signers, ed25519::PublicKey};
use commonware_runtime::{Clock, Metrics, Spawner};
use dashmap::DashMap;
use futures::StreamExt as _;
use rand_08::Rng;
use reth_ethereum_primitives::Block as RethBlock;
use reth_payload_primitives::PayloadKind;
use reth_primitives_traits::SealedBlock;

use crate::{block::Block, digest::Digest, executor, extra_data};

/// The signing scheme bound for this Application.
pub use fluentbase_bls::Scheme as BlsScheme;

/// Retention window for `round_index` entries, in rounds. Liveness only
/// needs cert metadata for rounds embedded in blocks currently in flight
/// (last finalized + a small lookback); the bound is generous and bounds
/// memory at ~40 KiB at ~40 B/entry × 1024 entries.
pub const ROUND_INDEX_RETENTION: u64 = 1024;

/// The Fluent consensus application.
///
/// Generic over `PB` (payload builder), `BE` (beacon-engine handle), `AB`
/// (payload attributes builder) and `Attrs` (concrete reth-fluent
/// `PayloadAttributes`; `Attrs = AB::Attrs` always — explicit generic is
/// needed because `executor::Mailbox<Attrs>` doesn't know about `AB`).
pub struct FluentApp<PB, BE, AB, Attrs> {
    genesis: Arc<Block>,
    payload_builder: PB,
    beacon_engine: BE,
    payload_attrs_builder: AB,
    executor: executor::Mailbox<Attrs>,
    /// Observer for `Update::Block` finalizations — NOT a state-advancing
    /// path. Wired to [`fluentbase_staking_reader::EpochTransition::on_finalized`]
    /// so the staking reader sees finalized blocks and detects epoch
    /// boundaries (fires
    /// `boundary_tx` for `EpochManager::enter`). Required at the type
    /// level — tests pass `Arc::new(|_| {})` when they don't exercise
    /// the boundary path.
    boundary_hook: Arc<dyn Fn(Block) + Send + Sync>,
    /// Bound on how long the payload builder is given to settle before
    /// `propose` calls `resolve_kind(_, WaitForPending)`. Derived from
    /// `leader_timeout - reth_settle`.
    payload_resolve_time: Duration,
    /// Marshal handle for querying finalization certs (cross-epoch
    /// singleton owned by `OuterEngine`). Used by `latest_finalized_cert`
    /// and `cert_for_round`. `None` is acceptable for tests / followers
    /// that don't run the liveness pipeline; production validators MUST
    /// set this via `OuterBuilder` or the cert byte-equal check in
    /// `verify` will return `None` and bypass.
    marshal: Option<MarshalMailbox<BlsScheme, Standard<Block>>>,
    /// Highest finalized block height observed via `Reporter::report`.
    /// `0` = no finalization seen yet (cold-start). Read by
    /// `latest_finalized_cert`.
    latest_finalized_height: Arc<AtomicU64>,
    /// `round → height` index built from observed `Update::Tip` events.
    /// Trimmed in-place to [`ROUND_INDEX_RETENTION`] entries per epoch.
    /// Read by `cert_for_round` (height → marshal cert query).
    round_index: Arc<Mutex<BTreeMap<Round, Height>>>,
    /// Shared with `FluentPayloadBuilder`: FluentApp::propose pre-registers
    /// `extra_data` keyed by engine-returned `PayloadId`; builder reads on
    /// `try_build`; app removes on cleanup after `resolve_kind`.
    extra_data_registry: Arc<DashMap<PayloadId, Bytes>>,
}

impl<PB, BE, AB, Attrs> Clone for FluentApp<PB, BE, AB, Attrs>
where
    PB: Clone,
    BE: Clone,
    AB: Clone,
{
    fn clone(&self) -> Self {
        Self {
            genesis: self.genesis.clone(),
            payload_builder: self.payload_builder.clone(),
            beacon_engine: self.beacon_engine.clone(),
            payload_attrs_builder: self.payload_attrs_builder.clone(),
            executor: self.executor.clone(),
            boundary_hook: self.boundary_hook.clone(),
            payload_resolve_time: self.payload_resolve_time,
            marshal: self.marshal.clone(),
            latest_finalized_height: self.latest_finalized_height.clone(),
            round_index: self.round_index.clone(),
            extra_data_registry: self.extra_data_registry.clone(),
        }
    }
}

impl<PB, BE, AB, Attrs> FluentApp<PB, BE, AB, Attrs> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        genesis: Block,
        payload_builder: PB,
        beacon_engine: BE,
        payload_attrs_builder: AB,
        executor: executor::Mailbox<Attrs>,
        boundary_hook: Arc<dyn Fn(Block) + Send + Sync>,
        payload_resolve_time: Duration,
        marshal: Option<MarshalMailbox<BlsScheme, Standard<Block>>>,
        latest_finalized_height: Arc<AtomicU64>,
        round_index: Arc<Mutex<BTreeMap<Round, Height>>>,
        extra_data_registry: Arc<DashMap<PayloadId, Bytes>>,
    ) -> Self {
        Self {
            genesis: Arc::new(genesis),
            payload_builder,
            beacon_engine,
            payload_attrs_builder,
            executor,
            boundary_hook,
            payload_resolve_time,
            marshal,
            latest_finalized_height,
            round_index,
            extra_data_registry,
        }
    }

    /// Returns the latest finalized cert's `(round, signers)`, if any.
    /// `None` when marshal isn't wired (test / follower mode) or no
    /// finalization has been observed yet (cold-start).
    pub async fn latest_finalized_cert(&self) -> Option<(Round, Signers)> {
        let marshal = self.marshal.as_ref()?;
        // latest_finalized_height is stored as h+1 so sentinel 0 means
        // "no finalization yet" and genesis (h=0) becomes a non-sentinel 1.
        let stored = self.latest_finalized_height.load(Ordering::Acquire);
        if stored == 0 {
            return None;
        }
        let h = stored - 1;
        let fin = marshal.get_finalization(Height::new(h)).await?;
        Some((fin.proposal.round, fin.certificate.signers.clone()))
    }

    /// Returns local cert's signers for `round`, if marshal still has the
    /// corresponding finalization. `None` when marshal isn't wired.
    pub async fn cert_for_round(&self, round: Round) -> Option<Signers> {
        let marshal = self.marshal.as_ref()?;
        let h = *self
            .round_index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&round)?;
        let fin = marshal.get_finalization(h).await?;
        Some(fin.certificate.signers.clone())
    }
}

impl<E, PB, BE, AB, Attrs> Application<E> for FluentApp<PB, BE, AB, Attrs>
where
    E: Rng + Spawner + Metrics + Clock + Send + 'static,
    PB: PayloadBuilderLike<BuiltSealed = SealedBlock<RethBlock>> + Clone + Send + Sync + 'static,
    BE: BeaconEngineLike<PayloadAttrs = Attrs, ExecutionData = SealedBlock<RethBlock>>
        + Clone
        + Send
        + Sync
        + 'static,
    AB: PayloadAttrsBuilderLike<Attrs = Attrs, Header = alloy_consensus::Header>
        + Clone
        + Send
        + Sync
        + 'static,
    Attrs: Clone + Send + Sync + 'static,
{
    type SigningScheme = BlsScheme;
    type Context = SimplexContext<Digest, PublicKey>;
    type Block = Block;

    async fn genesis(&mut self) -> Block {
        (*self.genesis).clone()
    }

    async fn propose<A: BlockProvider<Block = Block>>(
        &mut self,
        _ctx: (E, Self::Context),
        mut ancestry: AncestorStream<A, Block>,
    ) -> Option<Block> {
        let parent = ancestry.next().await?;

        // Encode the previous-finalized cert into extra_data
        // (cold-start: empty → executor / validator no-op decode to None).
        let extra_data_bytes: Vec<u8> = match self.latest_finalized_cert().await {
            Some((round, signers)) => extra_data::encode_simplex_attestation(round, &signers),
            None => Vec::new(),
        };

        let attrs = self.payload_attrs_builder.build(parent.header());

        // The executor inserts extra_data into the registry between
        // FCU return and the oneshot send (closing the race window
        // — see executor/ingress.rs CanonicalizeAndBuild::extra_data
        // for the threading-model analysis). FluentApp still owns the
        // post-`resolve_kind` cleanup removal below.
        let (tx, rx) = futures::channel::oneshot::channel();
        self.executor
            .send(executor::Message {
                cause: tracing::Span::current(),
                command: executor::Command::CanonicalizeAndBuild(executor::CanonicalizeAndBuild {
                    height: <Block as commonware_consensus::Heightable>::height(&parent),
                    digest: parent.digest(),
                    attributes: Box::new(attrs),
                    extra_data: Bytes::from(extra_data_bytes),
                    response: tx,
                }),
            })
            .ok()?;
        let payload_id = match rx.await {
            Ok(Ok(id)) => id,
            Ok(Err(executor::CanonicalizeError::BackfillInProgress)) => {
                // Expected when EL is catching up; skip this view's propose.
                return None;
            }
            Ok(Err(e)) => {
                tracing::warn!(?e, "canonicalize failed; skipping propose");
                return None;
            }
            Err(_) => return None,
        };

        let resolved = self
            .payload_builder
            .resolve_kind(payload_id, PayloadKind::WaitForPending)
            .await;

        // Reclaim the registry entry on EVERY exit once the build is resolved.
        // The executor inserted it keyed by payload_id; reth has already consumed
        // it by the time resolve_kind(WaitForPending) returns (the build job is
        // removed on resolve, and the consumed read happens-before). Running
        // remove here, before the `?`/`.ok()?` early returns, prevents a leak
        // when resolve_kind yields None/Err.
        self.extra_data_registry.remove(&payload_id);

        let sealed = resolved?.ok()?;

        Some(Block::from_execution_block(sealed))
    }
}

impl<E, PB, BE, AB, Attrs> VerifyingApplication<E> for FluentApp<PB, BE, AB, Attrs>
where
    E: Rng + Spawner + Metrics + Clock + Send + 'static,
    PB: PayloadBuilderLike<BuiltSealed = SealedBlock<RethBlock>> + Clone + Send + Sync + 'static,
    BE: BeaconEngineLike<PayloadAttrs = Attrs, ExecutionData = SealedBlock<RethBlock>>
        + Clone
        + Send
        + Sync
        + 'static,
    AB: PayloadAttrsBuilderLike<Attrs = Attrs, Header = alloy_consensus::Header>
        + Clone
        + Send
        + Sync
        + 'static,
    Attrs: Clone + Send + Sync + 'static,
{
    async fn verify<A: BlockProvider<Block = Block>>(
        &mut self,
        _ctx: (E, Self::Context),
        mut ancestry: AncestorStream<A, Block>,
    ) -> bool {
        // Verify keeps direct EL call: it's idempotent (same block hash returns
        // same status) and doesn't advance forkchoice state, so routing through
        // executor would add latency without benefit.
        let Some(block) = ancestry.next().await else {
            return false;
        };

        // Defensive cert byte-equal
        // check against local state. Reject ⇒ no notarize vote ⇒ graceful
        // view skip via Nullify. Empty extra_data is the cold-start /
        // no-prev-cert case and passes through to the payload validity
        // check unchanged.
        //
        // Immediate-only cert lookup: if marshal doesn't yet
        // have the cert, return false → Nullify → view skip; resolver
        // catches up before the next view. A wall-clock wait inside
        // verify fights Simplex's view-skip recovery model.
        let extra = &block.header().extra_data;
        match extra_data::decode_simplex_attestation(extra) {
            Ok(None) => { /* cold-start / no-prev-cert: skip cert check */ }
            Ok(Some(d)) => {
                let Some(local) = self.cert_for_round(d.round).await else {
                    return false;
                };
                if extra_data::encode_bitmap_only(&local) != d.bitmap {
                    return false;
                }
            }
            Err(_) => return false,
        }

        let sealed = block.into_inner();
        match self.beacon_engine.new_payload(sealed).await {
            Ok(status) => status.is_valid(),
            Err(_) => false,
        }
    }
}

impl<PB, BE, AB, Attrs> Reporter for FluentApp<PB, BE, AB, Attrs>
where
    PB: Clone + Send + Sync + 'static,
    BE: Clone + Send + Sync + 'static,
    AB: Clone + Send + Sync + 'static,
    Attrs: Send + Sync + 'static,
{
    type Activity = Update<Block>;

    async fn report(&mut self, activity: Update<Block>) {
        // Sidecar update on each reported activity:
        // - `Update::Block` advances `latest_finalized_height` (height
        //   comes from the block).
        // - `Update::Tip` populates `round_index[round] = height` with
        //   in-place retention trimming so memory stays bounded
        //   (~40 KiB worst-case at `ROUND_INDEX_RETENTION = 1024`).
        match &activity {
            Update::Block(block, _) => {
                let h = block.height().get();
                // Store h+1 so sentinel 0 = "no finalization yet" and
                // genesis (h=0) becomes a non-sentinel 1. saturating_add
                // guards the theoretical h=u64::MAX overflow.
                self.latest_finalized_height
                    .fetch_max(h.saturating_add(1), Ordering::Release);
            }
            Update::Tip(round, height, _digest) => {
                let mut idx = self.round_index.lock().unwrap_or_else(|e| e.into_inner());
                idx.insert(*round, *height);
                let view_cutoff = round.view().get().saturating_sub(ROUND_INDEX_RETENTION);
                let current_epoch = round.epoch().get();
                // EPOCH_RETENTION_WINDOW = N means N concurrent epochs are
                // alive (current + N-1 prior). Retain entries from those N
                // epochs; within current epoch, drop views older than
                // ROUND_INDEX_RETENTION. Prior-epoch entries are kept until
                // the retention window slides past them — verifier may still
                // see blocks referencing the prior epoch's cert during the
                // two-epochs-alive overlap (epoch_manager.rs).
                let cutoff_epoch =
                    current_epoch.saturating_sub(crate::epoch_manager::EPOCH_RETENTION_WINDOW - 1);
                idx.retain(|r, _| {
                    let re = r.epoch().get();
                    re >= cutoff_epoch && (re < current_epoch || r.view().get() >= view_cutoff)
                });
            }
        }

        // Boundary hook fires for `Update::Block` only — it's the 03
        // `EpochTransition::on_finalized` integration point for epoch-boundary
        // detection. Cloned before forwarding to executor so the executor
        // still owns the original `(Block, Exact)` for FCU + ack.
        if let Update::Block(ref block, _) = activity {
            (self.boundary_hook)(block.clone());
        }
        // Both `Update::Block` and `Update::Tip` are forwarded to the executor;
        // it differentiates internally (Block → push_back; Tip → store-and-canon).
        //
        // Ack flow: the `Exact` ack inside Update::Block
        // travels INSIDE this command and is fired by the executor's
        // `forward_finalized` after EL `new_payload` succeeds. Marshal
        // awaits the ack_waiter via PendingAcks; if the executor task
        // crashes mid-flight, FuturesOrdered drops the Exact ack →
        // marshal's waiter resolves to Err(Canceled) → marshal logs
        // fatal + returns → consensus_handle resolves → host adapter's
        // supervisor cancels shutdown_token → graceful node exit. No
        // silent-wait pathology: any drop trips the supervisor cascade.
        if let Err(e) = self.executor.send(executor::Message {
            cause: tracing::Span::current(),
            command: executor::Command::Finalize(Box::new(activity)),
        }) {
            // The executor mailbox is closed only if its task already exited.
            // The dropped Update::Block carries an Exact ack whose Drop trips
            // marshal's PendingAcks supervisor cascade (the recovery path) — but
            // log here so the send-side failure isn't silent.
            tracing::error!(?e, "executor mailbox closed; finalize command dropped");
        }
    }
}

/// Bound for the reth payload-builder handle used in 06.
///
/// Matches `launcher.rs:143` verbatim:
/// `self.payload_builder.resolve_kind(payload_id, PayloadKind::WaitForPending)`.
pub trait PayloadBuilderLike: Send + Sync + 'static {
    /// `SealedBlock<reth_ethereum_primitives::Block>` in 06.
    type BuiltSealed;

    fn resolve_kind(
        &self,
        id: PayloadId,
        kind: PayloadKind,
    ) -> impl std::future::Future<Output = Option<eyre::Result<Self::BuiltSealed>>> + Send;
}

/// Bound for the reth beacon-engine handle used in 06.
pub trait BeaconEngineLike: Send + Sync + 'static {
    /// Concrete `PayloadAttributes` type the engine expects.
    type PayloadAttrs: Send + 'static;
    /// Concrete `ExecutionData` type (typically a `SealedBlock`).
    type ExecutionData: Send + 'static;

    fn fork_choice_updated(
        &self,
        state: ForkchoiceState,
        attrs: Option<Self::PayloadAttrs>,
    ) -> impl std::future::Future<Output = eyre::Result<ForkchoiceUpdated>> + Send;

    fn new_payload(
        &self,
        data: Self::ExecutionData,
    ) -> impl std::future::Future<Output = eyre::Result<PayloadStatus>> + Send;
}

/// Bound for building reth's payload-attributes from a parent header.
pub trait PayloadAttrsBuilderLike: Send + Sync + 'static {
    type Attrs: Send + 'static;
    type Header;

    fn build(&self, parent_header: &Self::Header) -> Self::Attrs;
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{Block as AlloyBlock, BlockBody, Header};
    use alloy_primitives::{B256, U256};
    use commonware_runtime::Runner as _;
    use reth_ethereum_primitives::TransactionSigned;
    use std::sync::Mutex;

    type RethExecBlock = SealedBlock<reth_ethereum_primitives::Block>;

    #[derive(Clone, Debug, Default)]
    struct FakeAttrs;

    #[derive(Clone)]
    struct FakeAttrsBuilder;

    impl PayloadAttrsBuilderLike for FakeAttrsBuilder {
        type Attrs = FakeAttrs;
        type Header = Header;
        fn build(&self, _h: &Header) -> FakeAttrs {
            FakeAttrs
        }
    }

    #[derive(Clone)]
    struct FakeBeaconEngine;

    impl BeaconEngineLike for FakeBeaconEngine {
        type PayloadAttrs = FakeAttrs;
        type ExecutionData = RethExecBlock;

        async fn fork_choice_updated(
            &self,
            _state: ForkchoiceState,
            _attrs: Option<FakeAttrs>,
        ) -> eyre::Result<ForkchoiceUpdated> {
            Err(eyre::eyre!("fake; not exercised in these tests"))
        }

        async fn new_payload(&self, _data: RethExecBlock) -> eyre::Result<PayloadStatus> {
            Err(eyre::eyre!("fake; not exercised in these tests"))
        }
    }

    #[derive(Clone)]
    struct FakePayloadBuilder;

    impl PayloadBuilderLike for FakePayloadBuilder {
        type BuiltSealed = RethExecBlock;

        async fn resolve_kind(
            &self,
            _id: PayloadId,
            _kind: PayloadKind,
        ) -> Option<eyre::Result<RethExecBlock>> {
            None
        }
    }

    fn sample_sealed(parent: B256, number: u64) -> RethExecBlock {
        let header = Header {
            parent_hash: parent,
            number,
            gas_limit: 30_000_000,
            timestamp: 1_700_000_000,
            difficulty: U256::ZERO,
            ..Default::default()
        };
        let body: BlockBody<TransactionSigned> = BlockBody::default();
        let alloy_block: AlloyBlock<TransactionSigned> = AlloyBlock::new(header, body);
        SealedBlock::seal_slow(reth_ethereum_primitives::Block::from(alloy_block))
    }

    fn sample_block(parent: B256, number: u64) -> Block {
        Block::from_execution_block(sample_sealed(parent, number))
    }

    type FreshSidecar = (
        Arc<AtomicU64>,
        Arc<Mutex<BTreeMap<Round, Height>>>,
        Arc<DashMap<PayloadId, Bytes>>,
    );

    fn fresh_sidecar() -> FreshSidecar {
        (
            Arc::new(AtomicU64::new(0)),
            Arc::new(Mutex::new(BTreeMap::new())),
            Arc::new(DashMap::new()),
        )
    }

    fn build_app(
        executor: executor::Mailbox<FakeAttrs>,
    ) -> FluentApp<FakePayloadBuilder, FakeBeaconEngine, FakeAttrsBuilder, FakeAttrs> {
        build_app_with_hook(executor, Arc::new(|_b: Block| {}))
    }

    fn build_app_with_hook(
        executor: executor::Mailbox<FakeAttrs>,
        hook: Arc<dyn Fn(Block) + Send + Sync>,
    ) -> FluentApp<FakePayloadBuilder, FakeBeaconEngine, FakeAttrsBuilder, FakeAttrs> {
        let (h, ri, reg) = fresh_sidecar();
        FluentApp::new(
            sample_block(B256::ZERO, 0),
            FakePayloadBuilder,
            FakeBeaconEngine,
            FakeAttrsBuilder,
            executor,
            hook,
            Duration::from_millis(300),
            None,
            h,
            ri,
            reg,
        )
    }

    type DrainRx = Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<executor::Message<FakeAttrs>>>>;

    /// Drain helper: builds a mailbox + collector pair. The collector receives
    /// every command sent through the mailbox.
    fn fresh_mailbox() -> (executor::Mailbox<FakeAttrs>, DrainRx) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        // SAFETY: Mailbox::new is pub(super); test is in the same crate.
        let mb = executor::Mailbox::<FakeAttrs>::new_for_test(tx);
        (mb, Arc::new(Mutex::new(rx)))
    }

    #[test]
    fn report_block_sends_finalize_command() {
        use commonware_utils::{acknowledgement::Exact, Acknowledgement as _};

        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (mailbox, rx) = fresh_mailbox();
            let mut app = build_app(mailbox);

            let block = sample_block(B256::ZERO, 1);
            let (ack, _waiter) = Exact::handle();
            <FluentApp<FakePayloadBuilder, FakeBeaconEngine, FakeAttrsBuilder, FakeAttrs> as Reporter>::report(
                &mut app,
                Update::Block(block.clone(), ack),
            )
            .await;

            let msg = rx.lock().unwrap().try_recv().expect("Finalize command sent");
            match msg.command {
                executor::Command::Finalize(update) => match *update {
                    Update::Block(b, _ack) => assert_eq!(b.block_hash(), block.block_hash()),
                    _ => panic!("expected Update::Block"),
                },
                _ => panic!("expected Command::Finalize"),
            }
        });
    }

    #[test]
    fn report_block_fires_boundary_hook_and_forwards_to_executor() {
        use commonware_utils::{acknowledgement::Exact, Acknowledgement as _};
        use std::sync::atomic::{AtomicUsize, Ordering};

        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (mailbox, rx) = fresh_mailbox();
            let counter = Arc::new(AtomicUsize::new(0));
            let counter_clone = counter.clone();
            let hook: Arc<dyn Fn(Block) + Send + Sync> =
                Arc::new(move |_b: Block| { counter_clone.fetch_add(1, Ordering::SeqCst); });
            let mut app = build_app_with_hook(mailbox, hook);

            let block = sample_block(B256::ZERO, 1);
            let (ack, _waiter) = Exact::handle();
            <FluentApp<FakePayloadBuilder, FakeBeaconEngine, FakeAttrsBuilder, FakeAttrs> as Reporter>::report(
                &mut app,
                Update::Block(block.clone(), ack),
            )
            .await;

            assert_eq!(counter.load(Ordering::SeqCst), 1, "hook fired exactly once");
            let msg = rx.lock().unwrap().try_recv().expect("Finalize command also sent");
            match msg.command {
                executor::Command::Finalize(_) => (),
                _ => panic!("expected Command::Finalize alongside hook"),
            }
        });
    }

    #[test]
    fn report_tip_does_not_fire_boundary_hook() {
        use commonware_consensus::types::{Epoch, Height, Round, View};
        use std::sync::atomic::{AtomicUsize, Ordering};

        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (mailbox, _rx) = fresh_mailbox();
            let counter = Arc::new(AtomicUsize::new(0));
            let counter_clone = counter.clone();
            let hook: Arc<dyn Fn(Block) + Send + Sync> =
                Arc::new(move |_b: Block| { counter_clone.fetch_add(1, Ordering::SeqCst); });
            let mut app = build_app_with_hook(mailbox, hook);

            let round = Round::new(Epoch::new(0), View::new(0));
            <FluentApp<FakePayloadBuilder, FakeBeaconEngine, FakeAttrsBuilder, FakeAttrs> as Reporter>::report(
                &mut app,
                Update::Tip(round, Height::new(0), Digest(B256::ZERO)),
            )
            .await;

            assert_eq!(counter.load(Ordering::SeqCst), 0, "hook must NOT fire on Tip");
        });
    }

    #[test]
    fn report_tip_sends_finalize_command() {
        use commonware_consensus::types::{Epoch, Height, Round, View};

        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (mailbox, rx) = fresh_mailbox();
            let mut app = build_app(mailbox);

            let round = Round::new(Epoch::new(0), View::new(0));
            <FluentApp<FakePayloadBuilder, FakeBeaconEngine, FakeAttrsBuilder, FakeAttrs> as Reporter>::report(
                &mut app,
                Update::Tip(round, Height::new(0), Digest(B256::ZERO)),
            )
            .await;

            let msg = rx.lock().unwrap().try_recv().expect("Finalize command sent");
            match msg.command {
                executor::Command::Finalize(update) => match *update {
                    Update::Tip(..) => (),
                    _ => panic!("expected Update::Tip"),
                },
                _ => panic!("expected Command::Finalize"),
            }
        });
    }

    #[test]
    fn report_block_advances_latest_finalized_height() {
        use commonware_utils::{acknowledgement::Exact, Acknowledgement as _};

        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (mailbox, _rx) = fresh_mailbox();
            let mut app = build_app(mailbox);
            assert_eq!(app.latest_finalized_height.load(Ordering::Acquire), 0);

            let block = sample_block(B256::ZERO, 42);
            let (ack, _waiter) = Exact::handle();
            <FluentApp<FakePayloadBuilder, FakeBeaconEngine, FakeAttrsBuilder, FakeAttrs> as Reporter>::report(
                &mut app,
                Update::Block(block, ack),
            )
            .await;
            // h+1 encoding: height 42 stores as 43.
            assert_eq!(app.latest_finalized_height.load(Ordering::Acquire), 43);
        });
    }

    #[test]
    fn report_block_genesis_height_zero_clears_sentinel() {
        use commonware_utils::{acknowledgement::Exact, Acknowledgement as _};

        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (mailbox, _rx) = fresh_mailbox();
            let mut app = build_app(mailbox);
            assert_eq!(app.latest_finalized_height.load(Ordering::Acquire), 0);

            let block = sample_block(B256::ZERO, 0);
            let (ack, _waiter) = Exact::handle();
            <FluentApp<FakePayloadBuilder, FakeBeaconEngine, FakeAttrsBuilder, FakeAttrs> as Reporter>::report(
                &mut app,
                Update::Block(block, ack),
            )
            .await;
            // Sentinel was 0 (no finalization); after reporting genesis
            // h=0, h+1 encoding stores 1 — sentinel cleared.
            assert_eq!(app.latest_finalized_height.load(Ordering::Acquire), 1);
        });
    }

    #[test]
    fn report_block_uses_fetch_max_not_overwrite() {
        use commonware_utils::{acknowledgement::Exact, Acknowledgement as _};

        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (mailbox, _rx) = fresh_mailbox();
            let mut app = build_app(mailbox);

            // Out-of-order: report height 100 first, then height 50 — must stay at 100.
            let (ack, _w) = Exact::handle();
            <FluentApp<FakePayloadBuilder, FakeBeaconEngine, FakeAttrsBuilder, FakeAttrs> as Reporter>::report(
                &mut app,
                Update::Block(sample_block(B256::ZERO, 100), ack),
            )
            .await;
            let (ack, _w) = Exact::handle();
            <FluentApp<FakePayloadBuilder, FakeBeaconEngine, FakeAttrsBuilder, FakeAttrs> as Reporter>::report(
                &mut app,
                Update::Block(sample_block(B256::ZERO, 50), ack),
            )
            .await;
            // h+1 encoding: 100 stores as 101; out-of-order 50 → 51 < 101.
            assert_eq!(app.latest_finalized_height.load(Ordering::Acquire), 101);
        });
    }

    #[test]
    fn report_tip_populates_round_index() {
        use commonware_consensus::types::{Epoch, Height, Round, View};

        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (mailbox, _rx) = fresh_mailbox();
            let mut app = build_app(mailbox);

            let round = Round::new(Epoch::new(7), View::new(42));
            <FluentApp<FakePayloadBuilder, FakeBeaconEngine, FakeAttrsBuilder, FakeAttrs> as Reporter>::report(
                &mut app,
                Update::Tip(round, Height::new(123), Digest(B256::ZERO)),
            )
            .await;

            let idx = app.round_index.lock().unwrap();
            assert_eq!(idx.get(&round), Some(&Height::new(123)));
        });
    }

    #[test]
    fn report_tip_trims_round_index_to_retention_window() {
        use commonware_consensus::types::{Epoch, Height, Round, View};

        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (mailbox, _rx) = fresh_mailbox();
            let mut app = build_app(mailbox);

            // Insert epoch 7 view 0 — should be pruned once view advances
            // past ROUND_INDEX_RETENTION (= 1024).
            let old = Round::new(Epoch::new(7), View::new(0));
            <FluentApp<FakePayloadBuilder, FakeBeaconEngine, FakeAttrsBuilder, FakeAttrs> as Reporter>::report(
                &mut app,
                Update::Tip(old, Height::new(1), Digest(B256::ZERO)),
            )
            .await;
            assert!(app.round_index.lock().unwrap().contains_key(&old));

            // Now advance well past the retention window.
            let new = Round::new(Epoch::new(7), View::new(2000));
            <FluentApp<FakePayloadBuilder, FakeBeaconEngine, FakeAttrsBuilder, FakeAttrs> as Reporter>::report(
                &mut app,
                Update::Tip(new, Height::new(2001), Digest(B256::ZERO)),
            )
            .await;
            let idx = app.round_index.lock().unwrap();
            assert!(!idx.contains_key(&old), "old view should be pruned");
            assert!(idx.contains_key(&new), "new view should remain");
        });
    }

    #[test]
    fn report_tip_retains_entries_within_epoch_retention_window() {
        use commonware_consensus::types::{Epoch, Height, Round, View};
        // EPOCH_RETENTION_WINDOW = 2 (epoch_manager). Prior-epoch
        // entries must be preserved during the two-epochs-alive overlap;
        // a verifier may see a block in epoch N+1 whose extra_data
        // references a round from epoch N. Wiping on first new-epoch Tip
        // caused liveness loss at every boundary.

        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (mailbox, _rx) = fresh_mailbox();
            let mut app = build_app(mailbox);

            let epoch_6 = Round::new(Epoch::new(6), View::new(100));
            <FluentApp<FakePayloadBuilder, FakeBeaconEngine, FakeAttrsBuilder, FakeAttrs> as Reporter>::report(
                &mut app,
                Update::Tip(epoch_6, Height::new(100), Digest(B256::ZERO)),
            )
            .await;
            assert!(app.round_index.lock().unwrap().contains_key(&epoch_6));

            // Epoch 7 (current - 1 still inside window=2) — keep epoch 6.
            let epoch_7 = Round::new(Epoch::new(7), View::new(0));
            <FluentApp<FakePayloadBuilder, FakeBeaconEngine, FakeAttrsBuilder, FakeAttrs> as Reporter>::report(
                &mut app,
                Update::Tip(epoch_7, Height::new(101), Digest(B256::ZERO)),
            )
            .await;
            {
                let idx = app.round_index.lock().unwrap();
                assert!(idx.contains_key(&epoch_6), "prior-epoch entry retained within EPOCH_RETENTION_WINDOW");
                assert!(idx.contains_key(&epoch_7));
            }

            // Epoch 8 — window=2 slides past epoch 6, now evicted; epoch 7
            // still inside window.
            let epoch_8 = Round::new(Epoch::new(8), View::new(0));
            <FluentApp<FakePayloadBuilder, FakeBeaconEngine, FakeAttrsBuilder, FakeAttrs> as Reporter>::report(
                &mut app,
                Update::Tip(epoch_8, Height::new(102), Digest(B256::ZERO)),
            )
            .await;
            let idx = app.round_index.lock().unwrap();
            assert!(!idx.contains_key(&epoch_6), "epoch 6 evicted after window slid past");
            assert!(idx.contains_key(&epoch_7));
            assert!(idx.contains_key(&epoch_8));
        });
    }

    #[test]
    fn latest_finalized_cert_returns_none_when_marshal_unwired() {
        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (mailbox, _rx) = fresh_mailbox();
            let app = build_app(mailbox);
            // marshal = None in build_app; even after the height advances
            // the method must return None instead of touching marshal.
            app.latest_finalized_height.store(10, Ordering::Release);
            assert_eq!(app.latest_finalized_cert().await, None);
        });
    }

    #[test]
    fn cert_for_round_returns_none_when_marshal_unwired() {
        use commonware_consensus::types::{Epoch, Height, Round, View};

        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (mailbox, _rx) = fresh_mailbox();
            let app = build_app(mailbox);
            let round = Round::new(Epoch::new(7), View::new(42));
            app.round_index
                .lock()
                .unwrap()
                .insert(round, Height::new(1));
            assert_eq!(app.cert_for_round(round).await, None);
        });
    }
}
