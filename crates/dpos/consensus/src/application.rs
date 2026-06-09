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

use crate::{block::Block, digest::Digest, executor, extra_data};
use alloy_primitives::{Bytes, B256};
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
/// The signing scheme bound for this Application.
pub use fluentbase_bls::Scheme as BlsScheme;
use futures::StreamExt as _;
use rand_08::Rng;
use reth_ethereum_primitives::Block as RethBlock;
use reth_payload_primitives::PayloadKind;
use reth_primitives_traits::SealedBlock;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

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

        // Encode the previous-finalized cert into extra_data (cold-start:
        // empty → no registry entry → builder ships empty extra_data →
        // on-chain decode no-ops to None).
        let extra_data = Bytes::from(match self.latest_finalized_cert().await {
            Some((round, signers)) => extra_data::encode_simplex_attestation(round, &signers),
            None => Vec::new(),
        });

        let attrs = self.payload_attrs_builder.build(parent.header());

        // Pre-register extra_data under the PayloadId reth derives for this
        // (parent, attrs), BEFORE the FCU. reth spawns the seeding build
        // synchronously inside fork_choice_updated and only that build can
        // establish best_payload from Missing; later fee-neutral rebuilds are
        // dropped as Aborted. Inserting after the FCU (the prior design) always
        // lost. The proposer key matches the engine's whenever the FCU head is
        // this parent (the normal case); a mismatch degrades to empty
        // extra_data (fail-safe — the executor warns on it).
        let pid = self
            .payload_attrs_builder
            .payload_id(parent.digest().0, &attrs);
        if !extra_data.is_empty() {
            self.extra_data_registry.insert(pid, extra_data);
        }

        let (tx, rx) = futures::channel::oneshot::channel();
        self.executor
            .send(executor::Message {
                cause: tracing::Span::current(),
                command: executor::Command::CanonicalizeAndBuild(executor::CanonicalizeAndBuild {
                    height: <Block as commonware_consensus::Heightable>::height(&parent),
                    digest: parent.digest(),
                    attributes: Box::new(attrs),
                    response: tx,
                }),
            })
            .ok()?;
        let payload_id = match rx.await {
            Ok(Ok(id)) => id,
            Ok(Err(executor::CanonicalizeError::BackfillInProgress)) => {
                // Expected when EL is catching up; skip this view's propose.
                self.extra_data_registry.remove(&pid);
                return None;
            }
            Ok(Err(e)) => {
                self.extra_data_registry.remove(&pid);
                tracing::warn!(?e, "canonicalize failed; skipping propose");
                return None;
            }
            Err(_) => {
                self.extra_data_registry.remove(&pid);
                return None;
            }
        };

        let resolved = self
            .payload_builder
            .resolve_kind(payload_id, PayloadKind::WaitForPending)
            .await;

        // Reclaim the pre-registered entry on every exit, keyed by the
        // proposer-derived pid (== the engine's in the normal case). The
        // engine-returned payload_id is used only to resolve the build.
        self.extra_data_registry.remove(&pid);

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

        // Structural-only guard on the embedded prev-finalized attestation:
        // reject MALFORMED extra_data (a pure, deterministic function of the
        // block's bytes). We deliberately do NOT compare the signer bitmap
        // against this node's local finalization cert: that cert's signer set is
        // non-deterministic across honest nodes (commonware aggregates whatever
        // quorum-or-more finalize votes it saw first), so a byte-equal gate makes
        // honest nodes disagree on the same block → Nullify → crash-fault
        // liveness loss. The bitmap is liveness-slashing accounting, consumed
        // deterministically on-chain from the agreed block (processBitmap); its
        // signer-set integrity is not a consensus-safety property.
        if extra_data::decode_simplex_attestation(&block.header().extra_data).is_err() {
            return false;
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
        // `Update::Block` advances `latest_finalized_height`. `Update::Tip`
        // carries no sidecar state any more (the verify cert-index was removed)
        // and is only forwarded to the executor below.
        match &activity {
            Update::Block(block, _) => {
                let h = block.height().get();
                // Store h+1 so sentinel 0 = "no finalization yet" and
                // genesis (h=0) becomes a non-sentinel 1. saturating_add
                // guards the theoretical h=u64::MAX overflow.
                self.latest_finalized_height
                    .fetch_max(h.saturating_add(1), Ordering::Release);
            }
            Update::Tip(..) => {}
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

    /// Reproduce the engine's `PayloadId` derivation for `attrs` built on
    /// `parent_hash`, so the proposer can pre-register `extra_data` under the
    /// key reth's build job reads. Concrete impls call reth's `payload_id`.
    fn payload_id(&self, parent_hash: B256, attrs: &Self::Attrs) -> PayloadId;
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
        fn payload_id(&self, _parent_hash: B256, _attrs: &FakeAttrs) -> PayloadId {
            PayloadId::new([0u8; 8])
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

    type FreshSidecar = (Arc<AtomicU64>, Arc<DashMap<PayloadId, Bytes>>);

    fn fresh_sidecar() -> FreshSidecar {
        (Arc::new(AtomicU64::new(0)), Arc::new(DashMap::new()))
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
        let (h, reg) = fresh_sidecar();
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
}
