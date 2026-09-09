//! The fakes below the consensus seams. See the module doc for what each lies about.

use crate::{
    application::{
        BeaconEngineLike, DerivedBlockBuilder, ExecutedChain, FinalizedCursor, OrderingAssembler,
    },
    beacon::seed::Seed,
    cert_follow::{CertUpstream, UpstreamFinalized},
    fault::EngineError,
    order_block::OrderBlock,
    slasher::actor::{SlasherTxSink, SubmitOutcome},
};
use alloy_consensus::{Block as AlloyBlock, BlockBody, Header as AlloyHeader};
use alloy_primitives::{keccak256, Address, Bytes, B256, U256};
use alloy_rpc_types_engine::{
    ForkchoiceState, ForkchoiceUpdated, PayloadStatus, PayloadStatusEnum,
};
use commonware_consensus::types::Height;
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
        extra_data: Bytes::from(discriminator.to_vec()),
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
        _calldata: Bytes,
    ) -> std::pin::Pin<Box<dyn core::future::Future<Output = SubmitOutcome> + Send + 'a>> {
        Box::pin(async { SubmitOutcome::Failed("testbed NoSink: no staking contract".into()) })
    }
}

/// The `Option<U>::None` upstream: the stand has no cert upstream (step 3).
#[derive(Clone, Default)]
pub(super) struct NoUpstream;

impl CertUpstream for NoUpstream {
    async fn get_finalization(&self, _height: Height) -> Option<UpstreamFinalized> {
        None
    }
    async fn get_latest(&self) -> Option<UpstreamFinalized> {
        None
    }
    async fn rotate(&self) {}
}
