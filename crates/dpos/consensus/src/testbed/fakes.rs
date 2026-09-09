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
use fluentbase_bls::{BlsPubkey, PeerPubkey};
use fluentbase_staking_reader::{
    reader::{ConsensusKeys, ValidatorSetSnapshot, ValidatorWithKeys},
    ReadError, StakingStateRead,
};
use fluentbase_types::staking_protocol::{epoch_at_block, MAX_COMMITTEE_LOOKAHEAD_EPOCHS};
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

/// What the NETWORK executed, height → finalized-executed hash — the stand's
/// devp2p EL peer. Every node publishes a height here the moment its own
/// executor finalizes it (first writer wins, so a divergent deriver cannot
/// overwrite a hash the honest majority already published), and a node that
/// EL-syncs reads bodies out of it exactly as reth backfills them from peers.
///
/// This is the seam that makes a re-jump mean anything: EL sync does not need σ,
/// because the block arrives fully formed with its `prev_randao` already in the
/// header — which is why a node parked for want of an epoch key can still be
/// carried forward by it.
#[derive(Clone, Default)]
pub(super) struct ElNetwork {
    executed: Arc<Mutex<BTreeMap<u64, B256>>>,
}

impl ElNetwork {
    fn publish(&self, height: u64, hash: B256) {
        self.executed.lock().unwrap().entry(height).or_insert(hash);
    }

    fn range(&self, upto: u64) -> Vec<(u64, B256)> {
        self.executed
            .lock()
            .unwrap()
            .range(..=upto)
            .map(|(h, x)| (*h, *x))
            .collect()
    }
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
    /// Reverse index `executed hash -> height`, the read [`FakeStaking`] needs to
    /// answer "what was the contract state at this hash". Every hash this chain
    /// ever sealed stays in it (a reorged-out sibling still HAS a number), and it
    /// is deliberately NOT `canonical`'s inverse: [`Self::note_hash`] also puts
    /// the persisted finalized marker of a RESTARTED node here, a height whose
    /// block this process has not re-derived yet.
    by_hash: Arc<Mutex<BTreeMap<B256, u64>>>,
    /// The devp2p peer this node's EL syncs from. Written on every finalized
    /// height this node executes; read only by a re-jump landing.
    el_network: ElNetwork,
}

impl FakeChain {
    pub(super) fn with_genesis_on(hash: B256, el_network: ElNetwork) -> Self {
        let chain = Self {
            el_network,
            ..Self::default()
        };
        chain.canonical.lock().unwrap().insert(0, hash);
        chain.note_hash(0, hash);
        chain.finalized.advance(0);
        chain
    }

    /// A re-jump landing. `RethElSync::sync_to` FCUs reth toward the
    /// committee-ATTESTED `result` of the upstream tip and waits for reth to
    /// declare it canonical AND EXECUTED (`cold_start_jump.rs:434-455`), which
    /// means reth devp2p-backfilled and executed every body up to it — so the
    /// landing is not a lone hash on top of a hole, it is a whole executed
    /// prefix. Modelled by copying that prefix out of [`ElNetwork`].
    ///
    /// Returns `false` when the peer cannot serve the landing hash — the stand's
    /// form of "the served branch is not the one the network executed", which
    /// production catches with `verify_jump_structural` + the BLS multisig.
    pub(super) fn land_jump(&self, height: u64, hash: B256) -> bool {
        if self.el_network.executed.lock().unwrap().get(&height) != Some(&hash) {
            return false;
        }
        // COMPARE, never overwrite: a height this node already derived is its own
        // evidence, and letting a peer's answer replace it would let a divergent
        // node that reached a height first (`ElNetwork::publish` is
        // first-writer-wins, and nothing orders the honest majority first) rewrite
        // history under a jumper. A mismatch is the served branch disagreeing with
        // what this node executed — production's `verify_jump_structural` refusal.
        for (h, x) in self.el_network.range(height) {
            match self.spec_hash_at(h) {
                Some(mine) if mine != x => return false,
                Some(_) => {}
                None => self.land(h, x),
            }
        }
        self.finalized.advance(height);
        self.finalized_tip.fetch_max(height, Ordering::SeqCst);
        true
    }

    /// Record `hash` as the executed hash of `height` WITHOUT making it canonical.
    /// Used for the genesis anchor and for a replayed node's persisted finalized
    /// marker — the one height a restarted node knows a hash for before it has
    /// re-derived anything (reth's finalized marker survives the process; the
    /// stand's block bodies do not).
    pub(super) fn note_hash(&self, height: u64, hash: B256) {
        self.by_hash.lock().unwrap().insert(hash, height);
    }

    /// Height of an executed hash, or `None` when this chain never sealed it.
    pub(super) fn height_of(&self, hash: B256) -> Option<u64> {
        self.by_hash.lock().unwrap().get(&hash).copied()
    }

    /// The three-valued executed-state probe [`fluentbase_consensus::executed_state_hash`]
    /// is in production (`executed.rs:45-65`), over this chain: `Ok(None)` strictly
    /// above the executed head, `Ok(Some)` at a materialized height, `Err` for a
    /// materialized height with no hash (the header-index fault arm).
    pub(super) fn executed_state_hash(&self, height: u64) -> Result<Option<B256>, ReadError> {
        let best = self.executed_tip();
        if height > best {
            return Ok(None);
        }
        self.spec_hash_at(height).map(Some).ok_or_else(|| {
            ReadError::Backend(format!(
                "testbed: no executed hash at {height} (best={best})"
            ))
        })
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
        self.note_hash(height, hash);
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
        let from = self.finalized_tip.load(Ordering::SeqCst);
        self.finalized.advance(height);
        self.finalized_tip.fetch_max(height, Ordering::SeqCst);
        // Publish what this node just finalized-executed, so a peer that has to
        // EL-sync can be served the bodies it never derived.
        for h in from + 1..=height {
            if let Some(x) = self.spec_hash_at(h) {
                self.el_network.publish(h, x);
            }
        }
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

/// `epoch → member node indices`, or `None` for an epoch the contract never
/// committed a committee for. This is the stand's INPUT: WHO sits in an epoch.
/// WHEN that epoch becomes readable, and at WHICH hash, is [`FakeStaking`]'s
/// answer, not this closure's.
pub(super) type Members = Arc<dyn Fn(u64) -> Option<Vec<usize>> + Send + Sync>;

/// How many staking reads answered "committed" and how many answered "not
/// committed yet", per epoch — the observation the not-yet-committed tests
/// assert on. Counted inside [`FakeStaking`], so it says what the plane and the
/// `EpochTransition` actually ASKED, not what the stand arranged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct StakingReads {
    /// `epoch -> reads that came back with a committee`.
    pub committed: BTreeMap<u64, u64>,
    /// Reads that came back EMPTY because the epoch is not committed at the read
    /// height yet, per epoch. Production answers those with `Ok` and an empty
    /// `validators`, never an error — `reader.rs:633`.
    pub uncommitted: BTreeMap<u64, u64>,
    /// Reads at a hash this chain never sealed (production: a state read at an
    /// unknown block).
    pub unknown_state: u64,
}

/// The staking contract as a STATE MACHINE OVER EXECUTED HEIGHT.
///
/// Every read takes `at: B256` — an executed hash of some height of the node's
/// own [`FakeChain`] — and answers the contract state AS OF that height. The
/// two rules it reproduces:
///
/// * **Commit height.** The node's pre-execution stage drains
///   `commitEpochCommittee()` on EVERY block while
///   `nextEpochToCommit() <= current_epoch + MAX_COMMITTEE_LOOKAHEAD_EPOCHS`
///   (`node/src/evm.rs:895-918`, `:1227-1231`), and the contract reverts a
///   target above that horizon (`contracts/staking/src/consensus.rs:572-578`).
///   So after block `h` every epoch `<= epoch(h) + 2` is committed. Genesis is
///   the one exception: it is not executed by that stage, and the bootstrap
///   issues exactly ONE `commitEpochCommittee`, for epoch 0
///   (`devnet/local-dpos-smoke/genesis-bootstrap/src/bootstrap.rs:399-408`).
/// * **Not committed = `Ok` with an empty committee**, NOT a `ReadError`. The
///   production reader documents and returns exactly that (`reader.rs:633-634`),
///   and `EpochTransition` keys three separate branches on it
///   (`epoch_transition.rs:523`, `:545`, `:631`); an `Err` there would take the
///   boundary down instead of parking it.
///
/// `dkgQual[e] = committee[e] != committee[e-1]`, set inside the same commit
/// (`contracts/staking/src/consensus.rs:598`, `staking-abi/src/lib.rs:110`), so
/// it is readable exactly when the committee is.
///
/// NOT modelled (step 5b): `recordProduction`, penalties, tombstones, registry
/// mutation. The registry is a fixed set.
#[derive(Clone)]
pub(super) struct FakeStaking {
    chain: FakeChain,
    members: Members,
    /// Every stand node as the contract would hold it, indexed by node number.
    validators: Arc<Vec<ValidatorWithKeys>>,
    /// `getRegistryWithKeys()` — height-invariant here (no registry mutation).
    registry: Arc<Vec<PeerPubkey>>,
    epoch_len: u64,
    reads: Arc<Mutex<StakingReads>>,
}

impl FakeStaking {
    pub(super) fn new(
        chain: FakeChain,
        members: Members,
        peers: &[PeerPubkey],
        bls: &[BlsPubkey],
        registry: Vec<PeerPubkey>,
        epoch_len: u64,
    ) -> Self {
        let validators = peers
            .iter()
            .zip(bls)
            .enumerate()
            .map(|(i, (peer, bls))| ValidatorWithKeys {
                address: Address::with_last_byte(i as u8 + 1),
                keys: ConsensusKeys {
                    bls_pubkey: *bls,
                    peer_pubkey: peer.clone(),
                    activation_epoch: 0,
                },
                tombstoned: false,
            })
            .collect();
        Self {
            chain,
            members,
            validators: Arc::new(validators),
            registry: Arc::new(registry),
            epoch_len,
            reads: Arc::new(Mutex::new(StakingReads::default())),
        }
    }

    pub(super) fn reads(&self) -> StakingReads {
        self.reads.lock().unwrap().clone()
    }

    /// Every stand node in one epoch-0 snapshot. NOT a contract read: it is the
    /// fixed anonymous sharing `StaticRandomness` deals from, which by
    /// construction covers every node whatever an epoch's committee is.
    pub(super) fn all_validators_snapshot(&self) -> ValidatorSetSnapshot {
        let validators = self.validators.as_ref().clone();
        ValidatorSetSnapshot {
            block_hash: B256::ZERO,
            block_number: 0,
            epoch: 0,
            weights: Some(vec![1u128; validators.len()]),
            validators,
        }
    }

    /// Whether `epoch`'s committee is committed in the state at `height` — the
    /// contract's own horizon. See the type doc for both anchors.
    fn committed_at(&self, epoch: u64, height: u64) -> bool {
        if height == 0 {
            return epoch == 0;
        }
        match epoch_at_block(height, 0, self.epoch_len) {
            Some(current) => epoch <= current + MAX_COMMITTEE_LOOKAHEAD_EPOCHS,
            None => false,
        }
    }

    fn height_at(&self, at: B256) -> Result<u64, ReadError> {
        self.chain.height_of(at).ok_or_else(|| {
            self.reads.lock().unwrap().unknown_state += 1;
            ReadError::Backend(format!("testbed: no state at {at}"))
        })
    }

    /// The committee the contract would hold for `epoch`, peer-key ASCENDING as
    /// `commitEpochCommittee` sorts it (`contracts/staking/src/consensus.rs:596`).
    fn committee(&self, epoch: u64) -> Option<Vec<ValidatorWithKeys>> {
        let mut members: Vec<ValidatorWithKeys> = (self.members)(epoch)?
            .into_iter()
            .map(|i| self.validators[i].clone())
            .collect();
        if members.is_empty() {
            return None;
        }
        members.sort_unstable_by(|a, b| a.keys.peer_pubkey.cmp(&b.keys.peer_pubkey));
        Some(members)
    }

    /// `getDkgQual(epoch)` paired with "is `epoch`'s committee committed at
    /// `at`" — the two legs `beacon::carry::DkgQualProbe` reads
    /// (`carry.rs:181`). An uncommitted epoch reads its bit as the contract
    /// map's default `false` over an empty committee, which is `(false, false)`.
    pub(super) fn dkg_qual(&self, epoch: u64, at: B256) -> Result<(bool, bool), ReadError> {
        let snap = self.epoch_committee_snapshot(epoch, at)?;
        if snap.validators.is_empty() {
            return Ok((false, false));
        }
        if epoch == 0 {
            // `committee_changed` short-circuits to `false` at genesis
            // (`contracts/staking/src/consensus.rs:598-609`).
            return Ok((false, true));
        }
        let prev = self.epoch_committee_snapshot(epoch - 1, at)?;
        let keys = |s: &ValidatorSetSnapshot| -> Vec<PeerPubkey> {
            s.validators
                .iter()
                .map(|v| v.keys.peer_pubkey.clone())
                .collect()
        };
        Ok((keys(&prev) != keys(&snap), true))
    }
}

impl StakingStateRead for FakeStaking {
    fn epoch_committee_snapshot(
        &self,
        epoch: u64,
        at: B256,
    ) -> Result<ValidatorSetSnapshot, ReadError> {
        let height = self.height_at(at)?;
        let validators = self
            .committed_at(epoch, height)
            .then(|| self.committee(epoch))
            .flatten();
        let mut reads = self.reads.lock().unwrap();
        let counter = if validators.is_some() {
            &mut reads.committed
        } else {
            &mut reads.uncommitted
        };
        *counter.entry(epoch).or_default() += 1;
        drop(reads);
        // An uncommitted / missed-commit epoch is `Ok` with `validators: []`
        // and `weights: Some(vec![])` — the empty `stakes` leg beside an empty
        // `addrs` takes the equal-length arm (`reader.rs:667-680`), NOT the
        // `weights: None` "ring has wrapped" arm.
        let validators = validators.unwrap_or_default();
        let weights = Some(vec![1u128; validators.len()]);
        Ok(ValidatorSetSnapshot {
            block_hash: at,
            block_number: height,
            epoch,
            validators,
            weights,
        })
    }
    fn epoch_block_interval(&self, at: B256) -> Result<u64, ReadError> {
        self.height_at(at)?;
        Ok(self.epoch_len)
    }
    fn dpos_activation_block(&self, at: B256) -> Result<u64, ReadError> {
        self.height_at(at)?;
        Ok(0)
    }
    fn active_registry_peers(&self, at: B256) -> Result<Vec<PeerPubkey>, ReadError> {
        self.height_at(at)?;
        Ok(self.registry.as_ref().clone())
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

/// What one node's byzantine wrappers did — the tamper's own witness.
///
/// Every field is written by a wrapper and read by a test. A wrapper that never
/// fired leaves the counters at zero, which is what makes "the branch I asserted
/// is the branch the run took" checkable instead of assumed.
#[cfg(feature = "dpos-devnet-byzantine")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ByzFacts {
    // ---- R-002, `Role::TwoReveals` ----
    /// `DkgBody::Reveal` broadcasts this node's wrapper intercepted.
    pub reveals_seen: u64,
    /// Of those, the ones it actually split (original to the others, forged to
    /// the victim).
    pub reveals_swapped: u64,
    /// `keccak256(encode(L1))` — the log every honest member but the victim got.
    pub log1_hash: Option<B256>,
    /// `keccak256(encode(L2))` — the log the victim got instead.
    pub log2_hash: Option<B256>,
    /// Both logs `check` against the epoch's `Info` AND name this node as the
    /// dealer — the production predicate a receiver applies
    /// (`ceremony.rs::handle`'s `Reveal` arm).
    pub both_logs_check: bool,
    /// The victim the forged log was addressed to.
    pub victim: Option<PeerPubkey>,
    /// Signer schemes this node's `Randomness` wrapper rebuilt over the
    /// verify-only oracle.
    pub schemes_withheld: u64,
    /// `(the honest scheme signs a probe subject, the withheld one does)` at the
    /// last rebuild — the withholding's own witness. `Some((true, false))` is the
    /// only shape that proves the partial is being withheld rather than absent.
    pub withhold_probe: Option<(bool, bool)>,

    // ---- R-008, `Role::ForgedSeedUpstream` ----
    /// `Finalized{h}` answers this node served inside the window with a σ slot
    /// present.
    pub certs_seen: u64,
    /// Of those, the ones whose σ slot it replaced.
    pub certs_forged: u64,
    /// The heights it forged, in order.
    pub forged_heights: Vec<u64>,
    /// Every forged answer decoded back to `Some(σ')` with `σ' != σ`.
    pub forged_seed_differs: bool,
    /// Every forged answer's multisig half re-encoded byte-identically — the
    /// certificate bitmap and aggregate were NOT touched (and are not compared
    /// as bytes anywhere else: this is the one place the stand knows both
    /// versions of the same certificate).
    pub forged_vote_half_intact: bool,
    /// ARCHIVE-POISONING WITNESS, kept by every node that is NOT forging: the
    /// heights it served whose σ had already been served under a DIFFERENT round.
    /// A σ is unique per `(round, PK)` (`seed.rs`), so an honest archive can never
    /// produce one — a non-empty list means this node relayed a forgery it had
    /// itself accepted.
    pub served_seed_replays: Vec<u64>,
}

/// A node's [`ByzFacts`] behind a handle the stand can clone into its wrappers
/// and read at collection time.
#[cfg(feature = "dpos-devnet-byzantine")]
#[derive(Clone, Debug, Default)]
pub(super) struct ByzReport(Arc<Mutex<ByzFacts>>);

#[cfg(feature = "dpos-devnet-byzantine")]
impl ByzReport {
    pub(super) fn snapshot(&self) -> ByzFacts {
        self.0.lock().expect("byz report").clone()
    }

    pub(super) fn with<R>(&self, f: impl FnOnce(&mut ByzFacts) -> R) -> R {
        f(&mut self.0.lock().expect("byz report"))
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
