//! The fakes below the consensus seams. See the module doc for what each lies about.

use crate::{
    application::{
        BeaconEngineLike, DerivedBlockBuilder, ExecutedChain, FinalizedCursor, OrderingAssembler,
    },
    beacon::seed::Seed,
    cert_follow::{CertUpstream, UpstreamFinalized},
    fault::EngineError,
    order_block::OrderBlock,
    plane_upstream::{FrontierHandler, FrontierKey},
    slasher::actor::{SlasherTxSink, SubmitOutcome},
};
use alloy_consensus::{Block as AlloyBlock, BlockBody, Header as AlloyHeader};
use alloy_primitives::{keccak256, Address, Bytes as AlloyBytes, B256, U256};
use alloy_rpc_types_engine::{
    ForkchoiceState, ForkchoiceUpdated, PayloadStatus, PayloadStatusEnum,
};
use bytes::Bytes;
use commonware_consensus::types::Height;
use commonware_resolver::{p2p::Producer, Consumer};
use commonware_utils::channel::oneshot as cw_oneshot;
use fluentbase_bls::PeerPubkey;
use fluentbase_staking_reader::{reader::ValidatorSetSnapshot, ReadError, StakingStateRead};
use reth_ethereum_primitives::TransactionSigned;
use reth_primitives_traits::SealedBlock;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

pub(super) type ExecBlock = SealedBlock<reth_ethereum_primitives::Block>;

/// The derived-EVM-block stand-in: a sealed header at `number` over `parent`
/// whose hash is pinned by `discriminator` (`extra_data`). `timestamp = number`
/// keeps every virtual timestamp tiny (§13 rule 23: the anchor's timestamp seeds
/// the ordering chain's pace sleep).
pub(super) fn sealed_at(parent: B256, number: u64, discriminator: B256) -> ExecBlock {
    let header = AlloyHeader {
        parent_hash: parent,
        number,
        gas_limit: 30_000_000,
        timestamp: number,
        difficulty: U256::ZERO,
        extra_data: AlloyBytes::from(discriminator.to_vec()),
        ..Default::default()
    };
    let body: BlockBody<TransactionSigned> = BlockBody::default();
    SealedBlock::seal_slow(reth_ethereum_primitives::Block::from(AlloyBlock::new(
        header, body,
    )))
}

pub(super) fn genesis_sealed() -> ExecBlock {
    sealed_at(B256::ZERO, 0, B256::ZERO)
}

/// Height → canonical EVM hash, canonicalized at derive (last writer wins — a
/// finalized derive replaces a speculative sibling, modelling a reth reorg).
#[derive(Clone, Default)]
pub(super) struct FakeChain {
    canonical: Arc<Mutex<BTreeMap<u64, B256>>>,
    finalized: FinalizedCursor,
    /// The highest height the executor advanced the finalized cursor to — the
    /// tier-F tip. The stand compares nodes on THIS tier: tier-S at a height a
    /// node has not finalized yet may hold a notarized-then-nullified sibling.
    finalized_tip: Arc<AtomicU64>,
    /// The σ the executor handed `derive_and_execute` at each height (last
    /// writer wins, like `canonical`): `None` in a beacon-INACTIVE epoch. The
    /// object the live-beacon tests compare across nodes.
    seeds: Arc<Mutex<BTreeMap<u64, Option<Seed>>>>,
}

impl FakeChain {
    pub(super) fn with_genesis(hash: B256) -> Self {
        let chain = Self::default();
        chain.canonical.lock().unwrap().insert(0, hash);
        chain.finalized.advance(0);
        chain
    }

    /// Tier-F tip: the highest finalized-executed height.
    pub(super) fn tip(&self) -> u64 {
        self.finalized_tip.load(Ordering::SeqCst)
    }

    /// Tier-F hash at `height` (`None` above the finalized cursor).
    pub(super) fn hash_at(&self, height: u64) -> Option<B256> {
        self.finalized_executed_hash(height)
    }

    fn spec_hash_at(&self, height: u64) -> Option<B256> {
        self.canonical.lock().unwrap().get(&height).copied()
    }

    /// The seed derive saw at `height` (`None` = derived seedless, or never
    /// derived).
    pub(super) fn seed_at(&self, height: u64) -> Option<Seed> {
        self.seeds.lock().unwrap().get(&height).cloned().flatten()
    }

    fn land(&self, height: u64, hash: B256) {
        self.canonical.lock().unwrap().insert(height, hash);
    }
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
        self.spec_hash_at(height)
    }
    fn finalized_executed_hash(&self, height: u64) -> Option<B256> {
        self.finalized.resolve(height, |h| self.spec_hash_at(h))
    }
    fn advance_finalized(&self, height: u64) {
        self.finalized.advance(height);
        self.finalized_tip.fetch_max(height, Ordering::SeqCst);
    }
}

/// derive = `sealed_at(parent, height, keccak(order digest ‖ prev_randao(seed)))`.
/// With `divergent_at = Some(h)` the block at `h` seals to a DIFFERENT hash on
/// this node only — the `Role::DivergentResult` fault: K blocks later this node
/// commits (and expects) a `result` nobody else derived.
#[derive(Clone)]
pub(super) struct FakeDeriver {
    chain: FakeChain,
    divergent_at: Option<u64>,
}

impl FakeDeriver {
    pub(super) fn new(chain: FakeChain, divergent_at: Option<u64>) -> Self {
        Self {
            chain,
            divergent_at,
        }
    }
}

impl DerivedBlockBuilder for FakeDeriver {
    type Derived = ExecBlock;

    async fn derive_and_execute(
        &self,
        order: OrderBlock,
        parent_evm_hash: B256,
        seed: Option<Seed>,
    ) -> eyre::Result<ExecBlock> {
        let digest = order.digest().0;
        let mut discriminator = match &seed {
            Some(s) => keccak256(
                [
                    digest.as_slice(),
                    crate::beacon::seed::prev_randao_from_seed(s).as_slice(),
                ]
                .concat(),
            ),
            None => digest,
        };
        if self.divergent_at == Some(order.height) {
            discriminator = keccak256([discriminator.as_slice(), b"divergent"].concat());
        }
        let sealed = sealed_at(parent_evm_hash, order.height, discriminator);
        self.chain.land(order.height, sealed.hash());
        self.chain.seeds.lock().unwrap().insert(order.height, seed);
        Ok(sealed)
    }
}

/// Every FCU and import is `Valid`; nothing is recorded.
#[derive(Clone, Default)]
pub(super) struct FakeBeacon;

impl BeaconEngineLike for FakeBeacon {
    type ExecutionData = ExecBlock;

    async fn fork_choice_updated(
        &self,
        _state: ForkchoiceState,
    ) -> Result<ForkchoiceUpdated, EngineError> {
        Ok(ForkchoiceUpdated::from_status(PayloadStatusEnum::Valid))
    }

    async fn import_derived(&self, _data: ExecBlock) -> Result<PayloadStatus, EngineError> {
        Ok(PayloadStatus::from_status(PayloadStatusEnum::Valid))
    }
}

/// Empty blocks only.
pub(super) struct NoTxs;

impl OrderingAssembler for NoTxs {
    fn assemble(
        &self,
        _height: u64,
        _gas_limit: u64,
        _byte_budget: usize,
    ) -> Vec<TransactionSigned> {
        Vec::new()
    }
    fn observe_finalized(&self, _block: &OrderBlock) {}
}

/// `epoch → committee snapshot`, or `None` past the schedule.
pub(super) type Schedule = Arc<dyn Fn(u64) -> Option<ValidatorSetSnapshot> + Send + Sync>;

/// The staking-state read the slasher (and nothing else in this stand) sees:
/// committees from the schedule, the rest constant.
#[derive(Clone)]
pub(super) struct SnapshotReader {
    pub(super) schedule: Schedule,
    pub(super) epoch_len: u64,
}

impl StakingStateRead for SnapshotReader {
    fn epoch_committee_snapshot(
        &self,
        epoch: u64,
        _at: B256,
    ) -> Result<ValidatorSetSnapshot, ReadError> {
        Ok((self.schedule)(epoch).unwrap_or(ValidatorSetSnapshot {
            block_hash: B256::ZERO,
            block_number: 0,
            epoch,
            validators: vec![],
            weights: None,
        }))
    }
    fn epoch_block_interval(&self, _at: B256) -> Result<u64, ReadError> {
        Ok(self.epoch_len)
    }
    fn dpos_activation_block(&self, _at: B256) -> Result<u64, ReadError> {
        Ok(0)
    }
    fn active_registry_peers(&self, _at: B256) -> Result<Vec<PeerPubkey>, ReadError> {
        Ok(vec![])
    }
}

/// A slasher sink that never submits (no staking contract in the stand).
pub(super) struct NoSink;

impl SlasherTxSink for NoSink {
    fn submit<'a>(
        &'a self,
        _target: Address,
        _calldata: AlloyBytes,
    ) -> std::pin::Pin<Box<dyn core::future::Future<Output = SubmitOutcome> + Send + 'a>> {
        Box::pin(async { SubmitOutcome::Failed("testbed NoSink: no staking contract".into()) })
    }
}

/// What one node's upstream plane did, counted at both ends of the production
/// code: the client (`CertUpstream` calls and how many came back `Some`) and the
/// serve side (`Producer::produce` requests from peers, `Consumer::deliver`
/// results). A node that followed the chain with `latest_delivered ==
/// finalized_delivered == 0` did not follow THROUGH the plane.
#[derive(Clone, Default)]
pub(super) struct UpstreamCounters {
    pub latest_calls: Arc<AtomicU64>,
    pub latest_delivered: Arc<AtomicU64>,
    pub finalized_calls: Arc<AtomicU64>,
    pub finalized_delivered: Arc<AtomicU64>,
    /// `produce` calls this node answered for peers (any key).
    pub serve_requests: Arc<AtomicU64>,
    /// `deliver` calls the resolver made on this node that decoded.
    pub deliveries_decoded: Arc<AtomicU64>,
    /// `deliver` calls that did NOT decode (`false` — the R-009 arm).
    pub deliveries_rejected: Arc<AtomicU64>,
    /// `ReJump::call` invocations — the stand's re-jump is a no-op `Lagging`
    /// and its gate is `u64::MAX`, so this stays 0.
    pub rejump_calls: Arc<AtomicU64>,
}

/// A plain-number snapshot of [`UpstreamCounters`] for the outcome.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct UpstreamStats {
    pub latest_calls: u64,
    pub latest_delivered: u64,
    pub finalized_calls: u64,
    pub finalized_delivered: u64,
    pub serve_requests: u64,
    pub deliveries_decoded: u64,
    pub deliveries_rejected: u64,
    pub rejump_calls: u64,
}

impl UpstreamCounters {
    pub(super) fn snapshot(&self) -> UpstreamStats {
        let get = |c: &AtomicU64| c.load(Ordering::SeqCst);
        UpstreamStats {
            latest_calls: get(&self.latest_calls),
            latest_delivered: get(&self.latest_delivered),
            finalized_calls: get(&self.finalized_calls),
            finalized_delivered: get(&self.finalized_delivered),
            serve_requests: get(&self.serve_requests),
            deliveries_decoded: get(&self.deliveries_decoded),
            deliveries_rejected: get(&self.deliveries_rejected),
            rejump_calls: get(&self.rejump_calls),
        }
    }
}

/// The production `PlaneUpstreamHandle` behind a call counter — the `U` the
/// stand hands `OuterEngine::start`.
#[derive(Clone)]
pub(super) struct CountingUpstream<U> {
    inner: U,
    counters: UpstreamCounters,
}

impl<U: CertUpstream> CountingUpstream<U> {
    pub(super) fn new(inner: U, counters: UpstreamCounters) -> Self {
        Self { inner, counters }
    }
}

impl<U: CertUpstream> CertUpstream for CountingUpstream<U> {
    async fn get_finalization(&self, height: Height) -> Option<UpstreamFinalized> {
        self.counters.finalized_calls.fetch_add(1, Ordering::SeqCst);
        let got = self.inner.get_finalization(height).await;
        if got.is_some() {
            self.counters
                .finalized_delivered
                .fetch_add(1, Ordering::SeqCst);
        }
        got
    }
    async fn get_latest(&self) -> Option<UpstreamFinalized> {
        self.counters.latest_calls.fetch_add(1, Ordering::SeqCst);
        let got = self.inner.get_latest().await;
        if got.is_some() {
            self.counters
                .latest_delivered
                .fetch_add(1, Ordering::SeqCst);
        }
        got
    }
    async fn rotate(&self) {
        self.inner.rotate().await
    }
}

/// The production `FrontierHandler` behind a call counter — what the node's
/// frontier resolver engine gets as producer and consumer.
#[derive(Clone)]
pub(super) struct CountingHandler {
    inner: FrontierHandler,
    counters: UpstreamCounters,
}

impl CountingHandler {
    pub(super) fn new(inner: FrontierHandler, counters: UpstreamCounters) -> Self {
        Self { inner, counters }
    }
}

impl Producer for CountingHandler {
    type Key = FrontierKey;

    async fn produce(&mut self, key: FrontierKey) -> cw_oneshot::Receiver<Bytes> {
        self.counters.serve_requests.fetch_add(1, Ordering::SeqCst);
        self.inner.produce(key).await
    }
}

impl Consumer for CountingHandler {
    type Key = FrontierKey;
    type Value = Bytes;
    type Failure = ();

    async fn deliver(&mut self, key: FrontierKey, value: Bytes) -> bool {
        let ok = self.inner.deliver(key, value).await;
        let counter = if ok {
            &self.counters.deliveries_decoded
        } else {
            &self.counters.deliveries_rejected
        };
        counter.fetch_add(1, Ordering::SeqCst);
        ok
    }

    async fn failed(&mut self, key: FrontierKey, failure: ()) {
        self.inner.failed(key, failure).await
    }
}
