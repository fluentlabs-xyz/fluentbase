//! The fakes below the consensus seams.

use crate::{
    application::{
        BeaconEngineLike, DerivedBlockBuilder, ExecutedChain, FinalizedCursor, OrderingAssembler,
    },
    beacon::Seed,
    cert_follow::{CertUpstream, UpstreamFinalized},
    cold_start_jump::{ElSync, SyncFailure, EL_SYNC_STALL_ESCAPE},
    fault::EngineError,
    order_block::{OrderBlock, K},
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
use commonware_p2p::{utils::mux, Message, Receiver};
use commonware_resolver::{p2p::Producer, Consumer};
use commonware_runtime::{deterministic, Clock as _};
use commonware_utils::channel::oneshot as cw_oneshot;
use eyre::eyre;
use fluentbase_bls::{BlsPubkey, PeerPubkey};
use fluentbase_staking_reader::{
    reader::{ConsensusKeys, ValidatorSetSnapshot, ValidatorWithKeys},
    ReadError, StakingStateRead,
};
use fluentbase_types::staking_protocol::{
    epoch_at_block, MAX_COMMITTEE_LOOKAHEAD_EPOCHS, WEIGHT_RING_EPOCHS,
};
use reth_ethereum_primitives::TransactionSigned;
use reth_primitives_traits::SealedBlock;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

pub(super) type ExecBlock = SealedBlock<reth_ethereum_primitives::Block>;

/// `dposActivationBlock` of every stand: the ordering chain starts at genesis.
pub(super) const DPOS_ACTIVATION_BLOCK: u64 = 0;

/// The derived-EVM-block stand-in: a sealed header at `number` over `parent`,
/// its hash pinned by `discriminator` (`extra_data`). `timestamp = number` keeps
/// virtual timestamps tiny.
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

/// One executed block as the devp2p peer network holds it: its height and the
/// hash of its parent, so a jumping node can walk a served tip's branch down to a
/// fork point it already holds.
#[derive(Clone, Copy, Debug)]
pub(super) struct ElBlock {
    pub height: u64,
    pub parent: B256,
}

/// What the network executed, keyed by hash to `(height, parent)` — the stand's
/// devp2p EL peer. Every node publishes a block the moment its executor finalizes
/// it, and a node that EL-syncs walks a served tip's parent chain out of it.
///
/// Keyed by hash, not height, so a divergent branch coexists with the honest chain
/// and a lying upstream can actually serve its branch; fork safety is
/// [`FakeChain::land_jump`]'s job. EL sync needs no σ because the block arrives
/// with its `prev_randao` already in the header.
#[derive(Clone, Default)]
pub(super) struct ElNetwork {
    blocks: Arc<Mutex<std::collections::HashMap<B256, ElBlock>>>,
}

impl ElNetwork {
    /// Publish `hash` as an executed block at `height` with `parent`. A hash is
    /// unique to a branch, so a re-publish of the same hash is a no-op.
    fn publish(&self, hash: B256, height: u64, parent: B256) {
        self.blocks
            .lock()
            .unwrap()
            .entry(hash)
            .or_insert(ElBlock { height, parent });
    }

    fn get(&self, hash: B256) -> Option<ElBlock> {
        self.blocks.lock().unwrap().get(&hash).copied()
    }

    /// Publish a whole branch of `(hash, height, parent)` triples at once.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub(super) fn publish_branch(&self, branch: &[(B256, u64, B256)]) {
        for &(hash, height, parent) in branch {
            self.publish(hash, height, parent);
        }
    }
}

/// What [`FakeChain::land_jump`] did. The two refusals map to different production
/// `SyncFailure`s, which the executor treats differently: `InvalidTarget` is
/// corruption, while `StalledWithPeers` is non-fatal and does not rotate.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum JumpLanding {
    /// The served branch's prefix was copied into the canonical chain. The finalized
    /// cursor is not advanced here — that is the executor's `reseed_forward` on a
    /// `Landed` outcome.
    Landed,
    /// The devp2p peer holds no such hash, so the walk falls off the served tip
    /// immediately.
    Unservable,
    /// The peer serves the branch, but at `height` a walked block sits where this
    /// node's canonical chain holds a different hash.
    ConflictingPrefix {
        height: u64,
        mine: B256,
        served: B256,
    },
}

/// One executed block as reth's engine tree holds it: `InsertExecutedBlock` lands
/// it in `TreeState.blocks_by_hash`, which no provider method reads.
#[derive(Clone, Copy, Debug)]
struct TreeBlock {
    height: u64,
    parent: B256,
}

/// One EL-tier transition of a node's [`FakeChain`], in the order it happened.
///
/// The two tiers are only meaningful relative to each other, so one interleaved
/// log is the observable: a test compares the index of a [`Self::Derived`] against
/// the index of the [`Self::Canonicalized`] for the same height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ElEvent {
    /// `derive_and_execute` sealed this block and put it in the tree; nothing is
    /// canonical yet.
    Derived(u64, B256),
    /// A `fork_choice_updated` (or a jump landing) made this block canonical at this
    /// height.
    Canonicalized(u64, B256),
}

/// reth's two tiers over the same blocks, as the executor's guards read them.
///
/// * `tree` — executed but not canonical: what an `InsertExecutedBlock` leaves
///   behind, keyed by hash so a same-height sibling coexists with the canonical
///   block.
/// * `canonical` — the chain by number that `provider.block_hash(n)` answers, and
///   the only thing
///   [`ExecutedChain::spec_executed_hash`](crate::application::ExecutedChain::spec_executed_hash)
///   can see. A block enters it only when an FCU names it (or a descendant) as
///   head, or when the devp2p peer backfills it under a jump landing.
///
/// Divergences from reth: only an FCU (or a landing) canonicalizes; an
/// already-known `(height, hash)` import is a silent no-op; a new-head FCU drops
/// the old canonical suffix above the head; an FCU whose head cannot be linked to
/// the canonical chain canonicalizes nothing and answers `SYNCING`; safe and
/// finalized tags are ignored; `INVALID` and an engine-fatal `Err` are never
/// produced; there is no persistence lag and no pruning.
#[derive(Clone, Default)]
pub(super) struct FakeChain {
    /// Tier-T: every executed block by hash, canonical or not.
    tree: Arc<Mutex<BTreeMap<B256, TreeBlock>>>,
    /// Tier-S: the canonical chain by number — `provider.block_hash(n)`.
    canonical: Arc<Mutex<BTreeMap<u64, B256>>>,
    finalized: FinalizedCursor,
    /// The highest height the executor advanced the finalized cursor to — the tier-F
    /// tip. Nodes are compared on this tier, because tier-S at an unfinalized height
    /// may hold a notarized-then-nullified sibling.
    finalized_tip: Arc<AtomicU64>,
    /// The σ the executor handed `derive_and_execute` at each height, last writer
    /// wins; `None` in a beacon-inactive epoch.
    seeds: Arc<Mutex<BTreeMap<u64, Option<Seed>>>>,
    /// The devp2p peer this node's EL syncs from, written on every finalized height
    /// this node executes and read only by a re-jump landing.
    el_network: ElNetwork,
    /// Every tier transition in order — see [`ElEvent`].
    el_events: Arc<Mutex<Vec<ElEvent>>>,
    /// `best − ordering_finalized` right after every FCU that moved the canonical
    /// chain, as a histogram `gap -> count`. Event-driven because the speculative
    /// lead lives shorter than a driver tick.
    head_gap_on_fcu: Arc<Mutex<BTreeMap<u64, u64>>>,
    /// The same gap right after every jump landing, when the EL has moved but the
    /// tier-F tip has not.
    head_gap_on_landing: Arc<Mutex<BTreeMap<u64, u64>>>,
}

impl FakeChain {
    pub(super) fn with_genesis_on(hash: B256, el_network: ElNetwork) -> Self {
        let chain = Self {
            el_network,
            ..Self::default()
        };
        chain.land_canonical(0, hash);
        chain.finalized.advance(0);
        chain
    }

    /// `InsertExecutedBlock`: the block is executed and reachable by hash, and the
    /// canonical chain is untouched. An already-known hash is a silent no-op, with one
    /// repair: a stored parent of `B256::ZERO` is a placeholder and a later insert that
    /// knows the real parent replaces it.
    ///
    /// The placeholder exists because [`Self::note_hash`] registers a replayed node's
    /// persisted marker before any body has been re-derived, so it cannot know the
    /// parent, and without the repair the honest re-derive could never link it.
    fn insert_tree(&self, height: u64, hash: B256, parent: B256) {
        self.tree
            .lock()
            .unwrap()
            .entry(hash)
            .and_modify(|known| {
                if known.parent == B256::ZERO && parent != B256::ZERO {
                    known.parent = parent;
                }
            })
            .or_insert(TreeBlock { height, parent });
    }

    /// An FCU naming `head`: reth commits `[fork point ..= head]` and drops the old
    /// suffix above `head`. A head already canonical at its own height is a no-op.
    /// Returns `false` when the head cannot be linked to the current canonical chain —
    /// unknown to the tree, or its ancestor walk falls off before meeting a block the
    /// canonical chain holds — which is reth's missing-block branch that answers
    /// `SYNCING`.
    ///
    /// Fail-closed on purpose: committing a segment whose walk did not reach a fork
    /// point writes a hole or an unlinked chain, and every consumer reads this map as
    /// a chain.
    fn canonicalize(&self, head: B256) -> bool {
        let tree = self.tree.lock().unwrap();
        let Some(&TreeBlock { height: head_h, .. }) = tree.get(&head) else {
            return false;
        };
        let mut canonical = self.canonical.lock().unwrap();
        if canonical.get(&head_h) == Some(&head) {
            return true;
        }
        // Walk back to the fork point: the first ancestor the canonical chain
        // already holds at that height.
        let mut segment: Vec<(u64, B256)> = Vec::new();
        let mut cursor = head;
        let linked = loop {
            let Some(&TreeBlock { height, parent }) = tree.get(&cursor) else {
                break false;
            };
            if canonical.get(&height) == Some(&cursor) {
                break true;
            }
            segment.push((height, cursor));
            if height == 0 {
                break false;
            }
            cursor = parent;
        };
        if !linked {
            return false;
        }
        canonical.retain(|h, _| *h <= head_h);
        canonical.extend(segment.iter().copied());
        drop(canonical);
        drop(tree);
        // Ascending, so the log reads as the chain grew and not as the walk
        // unwound.
        self.el_events.lock().unwrap().extend(
            segment
                .into_iter()
                .rev()
                .map(|(h, x)| ElEvent::Canonicalized(h, x)),
        );
        self.note_head_gap(&self.head_gap_on_fcu);
        true
    }

    fn note_head_gap(&self, hist: &Mutex<BTreeMap<u64, u64>>) {
        let gap = self.executed_tip().saturating_sub(self.tip());
        *hist.lock().unwrap().entry(gap).or_default() += 1;
    }

    /// `gap → how many FCUs left the canonical tip that far above the tier-F tip`.
    pub(super) fn head_gap_on_fcu(&self) -> BTreeMap<u64, u64> {
        self.head_gap_on_fcu.lock().unwrap().clone()
    }

    /// The same, per jump landing.
    pub(super) fn head_gap_on_landing(&self) -> BTreeMap<u64, u64> {
        self.head_gap_on_landing.lock().unwrap().clone()
    }

    /// Make `hash` canonical at `height` outright: the genesis anchor, and the
    /// devp2p-backfilled prefix a jump landing copies in.
    ///
    /// The parent is `canonical[height - 1]`; its absence panics rather than falling
    /// back, because the only callers are genesis and [`Self::land_jump`], which lands
    /// ascending and has therefore already landed `height - 1`.
    fn land_canonical(&self, height: u64, hash: B256) {
        let parent = match height.checked_sub(1) {
            None => B256::ZERO,
            Some(below) => *self
                .canonical
                .lock()
                .unwrap()
                .get(&below)
                .unwrap_or_else(|| {
                    panic!(
                        "testbed: landing {hash} at {height} on a hole — the EL peer's executed \
                         range has no canonical block at {below}, so the landing is not a whole \
                         executed prefix"
                    )
                }),
        };
        self.insert_tree(height, hash, parent);
        self.canonical.lock().unwrap().insert(height, hash);
        self.el_events
            .lock()
            .unwrap()
            .push(ElEvent::Canonicalized(height, hash));
    }

    /// Whether `hash` is on the canonical chain — production's
    /// `provider.block_number(hash)` — as distinct from [`Self::height_of`], which
    /// reads the tree.
    fn canonical_holds(&self, hash: B256) -> bool {
        self.canonical.lock().unwrap().values().any(|x| *x == hash)
    }

    /// Every tier transition this node's EL made, in order.
    pub(super) fn el_events(&self) -> Vec<ElEvent> {
        self.el_events.lock().unwrap().clone()
    }

    /// The devp2p backfill a re-jump's `sync_to` drives: walk the served tip's parent
    /// chain out of [`ElNetwork`] down to a fork point the canonical chain already
    /// holds, and commit the walked segment. This is the reth side effect only; the
    /// finalized cursor is the executor's `reseed_forward` on a `Landed` outcome.
    ///
    /// `Unservable` means the peer holds no such hash; `ConflictingPrefix` means a
    /// walked block contradicts this node's executed history.
    pub(super) fn land_jump(&self, hash: B256) -> JumpLanding {
        let mut segment: Vec<(u64, B256)> = Vec::new();
        let mut cursor = hash;
        loop {
            // Reached a block this node already holds canonically — the fork point.
            if self.canonical_holds(cursor) {
                break;
            }
            let Some(ElBlock { height, parent }) = self.el_network.get(cursor) else {
                // The peer holds no such hash: nothing to walk, nothing to serve.
                return JumpLanding::Unservable;
            };
            // The canonical chain holds a different hash at this height.
            if let Some(mine) = self.spec_hash_at(height) {
                return JumpLanding::ConflictingPrefix {
                    height,
                    mine,
                    served: cursor,
                };
            }
            segment.push((height, cursor));
            if height == 0 {
                break;
            }
            cursor = parent;
        }
        // Land ascending, so each `land_canonical` finds the parent it needs.
        for (h, x) in segment.into_iter().rev() {
            self.land_canonical(h, x);
        }
        self.note_head_gap(&self.head_gap_on_landing);
        JumpLanding::Landed
    }

    /// Record `hash` as an executed block at `height` without making it canonical —
    /// the persisted finalized marker a restarted node knows before it has re-derived
    /// anything. Landing it canonically would leave a hole the replay does not model.
    pub(super) fn note_hash(&self, height: u64, hash: B256) {
        self.insert_tree(height, hash, B256::ZERO);
    }

    /// Height of an executed hash, or `None` when this chain never sealed it. Reads
    /// the tree, not the canonical chain: a reorged-out sibling still has a number,
    /// and this is what [`FakeStaking`] reads contract state by.
    pub(super) fn height_of(&self, hash: B256) -> Option<u64> {
        self.tree.lock().unwrap().get(&hash).map(|b| b.height)
    }

    /// The three-valued executed-state probe production uses, over this chain:
    /// `Ok(None)` strictly above the executed head, `Ok(Some)` at a materialized
    /// height, `Err` for a materialized height with no hash.
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
}

impl ExecutedChain for FakeChain {
    /// Production is `provider.last_block_number()` (the DB tier); here it is the
    /// canonical tip.
    fn executed_tip(&self) -> u64 {
        self.canonical
            .lock()
            .unwrap()
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0)
    }
    /// Production is `provider.block_hash(height)`; here it is the canonical map,
    /// which only an FCU or a jump landing writes.
    fn spec_executed_hash(&self, height: u64) -> Option<B256> {
        self.spec_hash_at(height)
    }
    /// Production is the same canonical read gated by the [`FinalizedCursor`];
    /// identical here, since tier-F is tier-S below the cursor.
    fn finalized_executed_hash(&self, height: u64) -> Option<B256> {
        self.finalized.resolve(height, |h| self.spec_hash_at(h))
    }
    /// Production is the cursor only; here it also records the tier-F tip and
    /// publishes the executed block to [`ElNetwork`].
    fn advance_finalized(&self, height: u64) {
        let from = self.finalized_tip.load(Ordering::SeqCst);
        self.finalized.advance(height);
        self.finalized_tip.fetch_max(height, Ordering::SeqCst);
        // Publish what this node just finalized-executed, so a peer that has to
        // EL-sync can be served the bodies it never derived.
        for h in from + 1..=height {
            match self.spec_hash_at(h) {
                // Publish by hash with the parent hash, so a jumping peer can walk
                // this branch down to a fork point (`land_jump`). The parent is the
                // canonical hash below `h` (genesis's parent is `ZERO`).
                Some(x) => {
                    let parent = h
                        .checked_sub(1)
                        .and_then(|below| self.spec_hash_at(below))
                        .unwrap_or(B256::ZERO);
                    self.el_network.publish(x, h, parent);
                }
                // A height this node derived must be canonical before the cursor
                // passes it, or the tier split is broken and every later result-gate
                // read samples a chain that does not exist.
                None if self.seeds.lock().unwrap().contains_key(&h) => panic!(
                    "testbed: finalized cursor advanced to {height} past {h}, which this node \
                     DERIVED but never canonicalized — the FCU that should have committed it \
                     did not, or a later reorg dropped it"
                ),
                // A height this node never derived: the one legitimate case is a
                // replay, whose fresh [`FakeChain`] has no bodies below the cursor.
                None => {}
            }
        }
    }
}

/// derive = `sealed_at(parent, height, keccak(order digest ‖ prev_randao(seed)))`.
/// With `divergent_at = Some(h)` the block at `h` seals to a different hash on this
/// node only — the [`Role::DivergentResult`](super::stand::Role) fault.
///
/// The derived block lands in the [`FakeChain`] tree and nowhere else: only an FCU
/// canonicalizes.
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
                    crate::beacon::prev_randao_from_seed(s).as_slice(),
                ]
                .concat(),
            ),
            None => digest,
        };
        if self.divergent_at == Some(order.height) {
            discriminator = keccak256([discriminator.as_slice(), b"divergent"].concat());
        }
        let sealed = sealed_at(parent_evm_hash, order.height, discriminator);
        self.chain
            .insert_tree(order.height, sealed.hash(), parent_evm_hash);
        self.chain
            .el_events
            .lock()
            .unwrap()
            .push(ElEvent::Derived(order.height, sealed.hash()));
        self.chain.seeds.lock().unwrap().insert(order.height, seed);
        Ok(sealed)
    }
}

/// The engine boundary over the same [`FakeChain`] the [`ExecutedChain`] reads, so
/// "what the executor imported" and "what the executor can read back" cannot drift
/// apart.
///
/// `import_derived` inserts into the tree and always answers `Valid`;
/// `fork_choice_updated` canonicalizes `[fork point ..= head]` and answers `Valid`,
/// or canonicalizes nothing and answers `SYNCING` when the head cannot be linked.
/// What is not modelled is listed on [`FakeChain`].
#[derive(Clone)]
pub(super) struct FakeBeacon {
    chain: FakeChain,
}

impl FakeBeacon {
    pub(super) fn new(chain: FakeChain) -> Self {
        Self { chain }
    }
}

impl BeaconEngineLike for FakeBeacon {
    type ExecutionData = ExecBlock;

    async fn fork_choice_updated(
        &self,
        state: ForkchoiceState,
    ) -> Result<ForkchoiceUpdated, EngineError> {
        // `safe_block_hash` / `finalized_block_hash` are deliberately unread —
        // see the divergence list on `FakeChain`.
        let status = match self.chain.canonicalize(state.head_block_hash) {
            true => PayloadStatusEnum::Valid,
            false => PayloadStatusEnum::Syncing,
        };
        Ok(ForkchoiceUpdated::from_status(status))
    }

    async fn import_derived(&self, data: ExecBlock) -> Result<PayloadStatus, EngineError> {
        let header = data.header();
        self.chain
            .insert_tree(header.number, data.hash(), header.parent_hash);
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
/// committed a committee for. When that epoch becomes readable, and at which hash,
/// is [`FakeStaking`]'s answer.
pub(super) type Members = Arc<dyn Fn(u64) -> Option<Vec<usize>> + Send + Sync>;

/// Which branch of the reading node's own execution layer a state hash sits on at
/// the moment of the read.
///
/// `Canonical` is `provider.block_hash(h) == at`; `Speculative` is every other hash
/// the node's tree knows at `h` — a derived-but-not-canonical block, a reorged-out
/// sibling, or the tree-only marker a replayed node is handed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Branch {
    Canonical,
    Speculative,
}

/// `(epoch, state hash, height, branch) → member node indices`, or `None` for an
/// epoch with no committee on that branch.
///
/// The branching form of [`Members`]: it lets the stand hand a different committee
/// to a read taken off the canonical chain, so cross-node equality of a committee
/// record becomes a property of the reading code rather than a tautology.
pub(super) type BranchCommittees =
    Arc<dyn Fn(u64, &B256, u64, Branch) -> Option<Vec<usize>> + Send + Sync>;

/// A node (by index) tombstoned from a height on, as the contract's live
/// equivocation flag: the flag is read at the call's own block while the
/// membership beside it is frozen.
pub(super) type Tombstones = Arc<Vec<(usize, u64)>>;

/// How many staking reads answered "committed" and how many "not committed yet",
/// per epoch, plus the other refusal classes. Counted inside [`FakeStaking`], so it
/// says what the plane and the `EpochTransition` actually asked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct StakingReads {
    /// `epoch -> reads that came back with a committee`.
    pub committed: BTreeMap<u64, u64>,
    /// Reads that came back empty because the epoch is not committed at the read
    /// height yet. Production answers those with `Ok` and an empty `validators`.
    pub uncommitted: BTreeMap<u64, u64>,
    /// Reads at a hash this chain never sealed (production: a state read at an
    /// unknown block).
    pub unknown_state: u64,
    /// `epoch -> reads taken at a hash this node's canonical chain did not hold at
    /// that height`. Counted whatever the schedule is: it is a property of the
    /// caller, so a consumer resolving its block hash off a speculative cursor
    /// shows up here even when every branch answers the same committee.
    pub speculative: BTreeMap<u64, u64>,
    /// `epoch -> reads that answered a non-empty committee with no frozen weights`
    /// — the contract's "weight ring has wrapped past this epoch" answer.
    pub weights_none: BTreeMap<u64, u64>,
    /// `epoch -> how many committee reads reverted`. A counter of its own, because
    /// a revert produces no snapshot and lands in neither `committed` nor
    /// `uncommitted`.
    pub reverted: BTreeMap<u64, u64>,
    /// `epoch -> reads that answered at least one tombstoned member`.
    pub tombstoned_seen: BTreeMap<u64, u64>,
    /// `epoch -> snapshot calls made through the committee module's port`
    /// ([`crate::committee::EpochReads`]) alone.
    ///
    /// Separate from [`Self::committed`], which also counts the `EpochTransition`'s
    /// reads and the snapshots this fake's [`FakeStaking::dkg_qual`] issues.
    pub module_snapshot: BTreeMap<u64, u64>,
}

/// The staking contract as a state machine over executed height.
///
/// Every read takes `at: B256`, an executed hash of the node's own [`FakeChain`],
/// and answers the contract state as of that height. After block `h` every epoch
/// `<= epoch(h) + MAX_COMMITTEE_LOOKAHEAD_EPOCHS` is committed, except that
/// genesis commits epoch 0 only. A not-yet-committed epoch answers `Ok` with an
/// empty committee, never an error, and
/// `dkgQual[e] = committee[e] != committee[e-1]` is readable exactly when the
/// committee is.
///
/// Frozen weights live in a [`WEIGHT_RING_EPOCHS`] ring and read `None` once a
/// frame is reused, an epoch 14 below the reading height — far outside the
/// committee module's window, so [`FakeStaking::weights_none_for`] exists as a
/// separate switch.
///
/// Not modelled: `recordProduction`, penalties, registry mutation.
#[derive(Clone)]
pub(super) struct FakeStaking {
    chain: FakeChain,
    members: Members,
    /// The branching committee schedule, if the stand set one — see
    /// [`BranchCommittees`]. `None` keeps [`Self::members`] as the whole answer.
    by_branch: Option<BranchCommittees>,
    /// The one epoch for which this contract answers `weights: None` despite the
    /// ring still holding its frame; a switch, because a committed epoch can never
    /// produce it.
    weights_none_for: Option<u64>,
    /// The one epoch whose committee read reverts — the contract refusing to
    /// answer rather than a statement about committed state, so the module must
    /// refuse the epoch loudly and keep the node running.
    reverts_for: Option<u64>,
    /// Nodes tombstoned from a height on — see [`Tombstones`].
    tombstoned: Tombstones,
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
            by_branch: None,
            weights_none_for: None,
            reverts_for: None,
            tombstoned: Arc::new(Vec::new()),
            validators: Arc::new(validators),
            registry: Arc::new(registry),
            epoch_len,
            reads: Arc::new(Mutex::new(StakingReads::default())),
        }
    }

    /// The optional schedule and schedule switches, applied after [`Self::new`] so
    /// the constructor keeps six positional arguments.
    pub(super) fn with_schedule(
        mut self,
        by_branch: Option<BranchCommittees>,
        weights_none_for: Option<u64>,
        tombstoned: Tombstones,
    ) -> Self {
        self.by_branch = by_branch;
        self.weights_none_for = weights_none_for;
        self.tombstoned = tombstoned;
        self
    }

    /// The epoch whose committee read reverts — see [`Self::reverts_for`].
    pub(super) fn with_revert(mut self, reverts_for: Option<u64>) -> Self {
        self.reverts_for = reverts_for;
        self
    }

    pub(super) fn reads(&self) -> StakingReads {
        self.reads.lock().unwrap().clone()
    }

    /// Every stand node in one epoch-0 snapshot. Not a contract read: it is the
    /// fixed anonymous sharing `StaticRandomness` deals from.
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

    /// Whether `epoch`'s committee is committed in the state at `height`.
    fn committed_at(&self, epoch: u64, height: u64) -> bool {
        if height == 0 {
            return epoch == 0;
        }
        match epoch_at_block(height, DPOS_ACTIVATION_BLOCK, self.epoch_len) {
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

    /// Which branch of this node's execution layer `at` sits on at `height` — see
    /// [`Branch`]. Compared against the canonical map, the only tier
    /// `provider.block_hash(n)` sees.
    fn branch_of(&self, at: B256, height: u64) -> Branch {
        match self.chain.spec_hash_at(height) {
            Some(canonical) if canonical == at => Branch::Canonical,
            _ => Branch::Speculative,
        }
    }

    /// The committee the contract would hold for `epoch` in the state at `at`,
    /// peer-key ascending as `commitEpochCommittee` sorts it.
    ///
    /// `tombstoned` is applied last and from the read height, not the epoch: the
    /// contract reads that flag live at the call's own block while the membership
    /// beside it is frozen, so a member tombstoned at height `h` is flagged in
    /// every read at or above `h`.
    fn committee(
        &self,
        epoch: u64,
        at: B256,
        height: u64,
        branch: Branch,
    ) -> Option<Vec<ValidatorWithKeys>> {
        let indices = match &self.by_branch {
            Some(by_branch) => by_branch(epoch, &at, height, branch)?,
            None => (self.members)(epoch)?,
        };
        let mut members: Vec<ValidatorWithKeys> = indices
            .into_iter()
            .map(|i| {
                let mut validator = self.validators[i].clone();
                validator.tombstoned = self
                    .tombstoned
                    .iter()
                    .any(|(node, from)| *node == i && height >= *from);
                validator
            })
            .collect();
        if members.is_empty() {
            return None;
        }
        members.sort_unstable_by(|a, b| a.keys.peer_pubkey.cmp(&b.keys.peer_pubkey));
        Some(members)
    }

    /// The frozen leader weights the contract answers for `epoch` at `height`, or
    /// `None` for the ring-wrapped answer. An empty committee takes neither arm.
    fn weights_at(&self, epoch: u64, height: u64, members: usize) -> Option<Vec<u128>> {
        if members == 0 {
            return Some(Vec::new());
        }
        if self.weights_none_for == Some(epoch) {
            return None;
        }
        let current = epoch_at_block(height, DPOS_ACTIVATION_BLOCK, self.epoch_len)?;
        let newest_committed = current.saturating_add(MAX_COMMITTEE_LOOKAHEAD_EPOCHS);
        if epoch.saturating_add(WEIGHT_RING_EPOCHS) <= newest_committed {
            return None;
        }
        Some(vec![1u128; members])
    }

    /// `getDkgQual(epoch)` paired with "is `epoch`'s committee committed at `at`" —
    /// the two legs `beacon::CommitteeReads::dkg_qual` answers with. An uncommitted
    /// epoch reads `(false, false)`.
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

/// The two staticcalls the committee module makes, over the same fake.
///
/// A separate impl from [`StakingStateRead`] because the module's port is narrower
/// and because `dkg_qual` is not on `StakingStateRead` at all.
impl crate::committee::EpochReads for FakeStaking {
    fn epoch_committee_snapshot(
        &self,
        epoch: u64,
        at: B256,
    ) -> Result<ValidatorSetSnapshot, ReadError> {
        // Counted here and not inside the shared body: this port is the module's
        // alone, so the count says how many snapshot calls the module made.
        *self
            .reads
            .lock()
            .unwrap()
            .module_snapshot
            .entry(epoch)
            .or_default() += 1;
        StakingStateRead::epoch_committee_snapshot(self, epoch, at)
    }

    fn dkg_qual(&self, epoch: u64, at: B256) -> Result<bool, ReadError> {
        FakeStaking::dkg_qual(self, epoch, at).map(|(bit, _committed)| bit)
    }
}

impl StakingStateRead for FakeStaking {
    fn epoch_committee_snapshot(
        &self,
        epoch: u64,
        at: B256,
    ) -> Result<ValidatorSetSnapshot, ReadError> {
        let height = self.height_at(at)?;
        // Count the revert before any of the counters below: a reverting call
        // produced no snapshot, so counting it as committed or uncommitted would be
        // counting an answer that was never given.
        if self.reverts_for == Some(epoch) {
            *self
                .reads
                .lock()
                .unwrap()
                .reverted
                .entry(epoch)
                .or_default() += 1;
            return Err(ReadError::CallReverted(format!(
                "testbed: getEpochCommitteeWithStakes({epoch}) reverted"
            )));
        }
        let branch = self.branch_of(at, height);
        let validators = self
            .committed_at(epoch, height)
            .then(|| self.committee(epoch, at, height, branch))
            .flatten();
        // An uncommitted epoch is `Ok` with `validators: []` and
        // `weights: Some(vec![])` — the equal-length arm, not the ring-wrapped one.
        let validators = validators.unwrap_or_default();
        let weights = self.weights_at(epoch, height, validators.len());
        let mut reads = self.reads.lock().unwrap();
        let counter = if validators.is_empty() {
            &mut reads.uncommitted
        } else {
            &mut reads.committed
        };
        *counter.entry(epoch).or_default() += 1;
        if branch == Branch::Speculative {
            *reads.speculative.entry(epoch).or_default() += 1;
        }
        if weights.is_none() {
            *reads.weights_none.entry(epoch).or_default() += 1;
        }
        if validators.iter().any(|v| v.tombstoned) {
            *reads.tombstoned_seen.entry(epoch).or_default() += 1;
        }
        drop(reads);
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
        Ok(DPOS_ACTIVATION_BLOCK)
    }
    fn active_registry_peers(&self, at: B256) -> Result<Vec<PeerPubkey>, ReadError> {
        self.height_at(at)?;
        Ok(self.registry.as_ref().clone())
    }
}

/// One `ReJump::call` — one run of the production
/// [`jump_to_target`](crate::cold_start_jump::jump_to_target).
///
/// The `outcome` tag exists because every non-`Landed` variant is otherwise
/// invisible: the stand wires `ReJump::rotate = None`, so a refusal leaves nothing
/// behind but a log line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct JumpCall {
    /// The anchor the executor passed (its `ordering_finalized` cursor).
    pub from: u64,
    /// The `JumpOutcome` variant name, verbatim.
    pub outcome: &'static str,
    /// `(height, result)` of the target certificate this call consumed — the pair
    /// the executor read out of this node's own marshal archive, recorded before
    /// the jump consumes it so the landing can be checked against its certificate.
    pub consumed: Option<(u64, B256)>,
    /// `(landing, hash)` on [`JumpOutcome::Landed`](crate::cold_start_jump::JumpOutcome::Landed).
    pub landed: Option<(u64, B256)>,
    /// The `Display` text of the `eyre::Report` a refusing outcome carries — `None`
    /// for `Landed`/`Lagging`. Only the message distinguishes the landing check
    /// from a reth `Invalid` verdict, both of which produce `InvalidTarget`.
    pub outcome_detail: Option<String>,
}

/// Every jump call one node made, in call order.
pub(super) type JumpCalls = Arc<Mutex<Vec<JumpCall>>>;

/// Every ladder step a node's frontier probe named, as `(T, last(T+1))` in call
/// order. Recorded where the step is named, so a test reads what was asked for.
///
/// No tip beside it: reading the marshal tip from this closure is an extra message
/// that changes the run.
pub(super) type FrontierSteps = Arc<Mutex<Vec<(u64, u64)>>>;

/// The jump's EL seam over [`FakeChain`] and [`ElNetwork`] — the stand's
/// [`RethElSync`](crate::cold_start_jump::RethElSync). A pre-K (`result == 0`) tip
/// and an already-executed target answer the local landing; the real case drives
/// the executed prefix out of the devp2p peer before answering.
///
/// Non-`Landed` outcomes are fixture-dependent: on honest schedules the peer always
/// serves the attested landing, so the refusal paths are written but not exercised.
/// The `EL_SYNC_STALL_ESCAPE` sleep on an unservable target freezes the node's jump
/// for 300 virtual seconds, which a fixture that reaches it must budget.
pub(super) struct JumpElSync {
    chain: FakeChain,
    ctx: deterministic::Context,
    activation: u64,
}

impl JumpElSync {
    pub(super) fn new(chain: FakeChain, ctx: deterministic::Context, activation: u64) -> Self {
        Self {
            chain,
            ctx,
            activation,
        }
    }

    /// Production's `RethElSync::local_landing`: the executed head clamped to
    /// `>= activation`, with the hash of that same height.
    fn local_landing(&self) -> Result<(u64, B256), SyncFailure> {
        let tip = self.chain.executed_tip().max(self.activation);
        let hash = self.chain.spec_hash_at(tip).ok_or_else(|| {
            SyncFailure::Stalled(eyre!("testbed EL holds no hash at its own tip {tip}"))
        })?;
        Ok((tip, hash))
    }
}

impl ElSync for JumpElSync {
    /// The stand never builds a fresh-datadir follower, so the operator-checkpoint
    /// entry has no caller here. Refuse loudly rather than model it.
    async fn sync_to_checkpoint(&self, checkpoint: B256) -> Result<(u64, B256), SyncFailure> {
        panic!(
            "the testbed does not model the fresh-datadir operator-checkpoint entry \
             (sync_to_checkpoint({checkpoint})); only `launch`/`launch_follower` reach it"
        )
    }

    async fn sync_to(&self, latest: &UpstreamFinalized) -> Result<(u64, B256), SyncFailure> {
        let tip_hash = latest.block.result;
        let tip_height = latest.block.height.saturating_sub(K);
        if tip_hash == B256::ZERO {
            // Pre-K window: nothing to EL-sync (`cold_start_jump.rs:448-454`).
            return self.local_landing();
        }
        if self.chain.executed_tip() >= tip_height {
            // Already executed past the target (`cold_start_jump.rs:455-465`).
            return self.local_landing();
        }
        // Production order: FCU `head=safe=finalized=tip_hash` first and let reth
        // backfill the branch whose tip is that hash; only after it reports `Valid`
        // resolve the landing height via `provider.block_hash(landing)`.
        match self.chain.land_jump(tip_hash) {
            JumpLanding::Landed => {}
            JumpLanding::Unservable => {
                self.ctx.sleep(EL_SYNC_STALL_ESCAPE).await;
                return Err(SyncFailure::StalledWithPeers(eyre!(
                    "testbed EL peer cannot serve the attested tip hash {tip_hash} (claimed \
                     height {})",
                    latest.block.height
                )));
            }
            JumpLanding::ConflictingPrefix {
                height,
                mine,
                served,
            } => {
                // Whether reth would answer `Invalid` rather than unwind for a
                // served branch conflicting with executed history is not verified
                // here; the point is that the stand stops folding two conditions
                // into one outcome.
                return Err(SyncFailure::Invalid(eyre!(
                    "testbed EL peer served a branch that contradicts executed history at \
                     {height}: mine {mine}, served {served} (claimed tip height {})",
                    latest.block.height
                )));
            }
        }
        // Clamp before resolving, so the returned pair is always the height and hash
        // of the same block.
        let landing = tip_height.max(self.activation);
        let hash = self.chain.spec_hash_at(landing).ok_or_else(|| {
            SyncFailure::Stalled(eyre!(
                "testbed EL-sync backfilled the served tip {tip_hash} but no block resolves at \
                 the claimed landing height {landing}"
            ))
        })?;
        Ok((landing, hash))
    }

    /// Production's `RethElSync::holds` reads the canonical chain, not the executed
    /// tree; the trait's contract says "holds `hash` canonically". Unreachable when
    /// `l1_checkpoint` is `None`, but the tier must be right for a fixture that
    /// wires one.
    fn holds(&self, hash: B256) -> eyre::Result<bool> {
        Ok(self.chain.canonical_holds(hash))
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
/// serve side (`Producer::produce` requests, `Consumer::deliver` results).
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
    /// `deliver` calls that returned `false` — a signal of a lie on any of the five
    /// arms. Each one costs the sender this channel for the life of the resolver
    /// engine.
    ///
    /// A count only: the `reason` split lives in `metrics::counter!`, which goes to
    /// a recorder the stand does not read, so no assertion can say which arm fired.
    pub deliveries_rejected: Arc<AtomicU64>,
    /// Every by-height pull this node's upstream client made, in order.
    ///
    /// `finalized_calls`/`finalized_delivered` count the same events without the
    /// heights, which is what a ladder assertion needs: the ladder step and the
    /// marshal's ordinary gap repair are the same verb on this seam, so only the
    /// height tells them apart.
    pub served_heights: Arc<Mutex<Vec<Pull>>>,
    /// `ReJump::call` invocations, each running the production
    /// [`crate::cold_start_jump::jump_to_target`]. Stays 0 while
    /// `StandConfig::re_jump_threshold` is `None`, because the gate is then
    /// `u64::MAX`. A count only: [`JumpCall`] says what each call did.
    pub rejump_calls: Arc<AtomicU64>,
}

/// One by-height pull the upstream client made: the height asked for and whether
/// the answer came back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Pull {
    pub height: u64,
    pub delivered: bool,
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

    fn note_served(&self, height: Height, delivered: bool) {
        if delivered {
            self.counters
                .finalized_delivered
                .fetch_add(1, Ordering::SeqCst);
        }
        self.counters.served_heights.lock().unwrap().push(Pull {
            height: height.get(),
            delivered,
        });
    }
}

impl<U: CertUpstream> CertUpstream for CountingUpstream<U> {
    async fn get_finalization(&self, height: Height) -> Option<UpstreamFinalized> {
        self.counters.finalized_calls.fetch_add(1, Ordering::SeqCst);
        let got = self.inner.get_finalization(height).await;
        self.note_served(height, got.is_some());
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

/// One broadcast `ShareConfirm` as the wire saw it: the ceremony epoch, the
/// confirming member's seat, and the `(seat, log hash)` set it claims to hold.
#[cfg(feature = "dpos-devnet-byzantine")]
pub(super) type SentConfirm = (u64, u8, Vec<(u8, B256)>);

/// What one node's byzantine wrappers did — the tamper's own witness. A wrapper
/// that never fired leaves the counters at zero, which is what makes "the branch I
/// asserted is the branch the run took" checkable instead of assumed.
#[cfg(feature = "dpos-devnet-byzantine")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ByzFacts {
    /// `DkgBody::Reveal` broadcasts this node's wrapper intercepted.
    pub reveals_seen: u64,
    /// Of those, the ones it actually split (original to the others, forged to the
    /// victim).
    pub reveals_swapped: u64,
    /// `keccak256(encode(L1))` — the log every honest member but the victim got.
    pub log1_hash: Option<B256>,
    /// `keccak256(encode(L2))` — the log the victim got instead.
    pub log2_hash: Option<B256>,
    /// Both logs `check` against the epoch's `Info` and name this node as the
    /// dealer.
    pub both_logs_check: bool,
    /// The victim the forged log was addressed to.
    pub victim: Option<PeerPubkey>,
    /// Every `ShareConfirm` this node broadcast: ceremony epoch, its own seat, and
    /// the `(seat, log hash)` set it claims. Recorded on every node, because a
    /// victim's confirm naming the forged hash at the dealer's seat is a direct
    /// observation that the second log was recorded.
    pub confirms_sent: Vec<SentConfirm>,
    /// Signer schemes this node's `Beacon` wrapper rebuilt over the
    /// verify-only oracle.
    pub schemes_withheld: u64,
    /// `(the honest scheme signs a probe subject, the withheld one does)` at the last
    /// rebuild — `Some((true, false))` is the only shape that proves the partial is
    /// being withheld rather than absent.
    pub withhold_probe: Option<(bool, bool)>,

    /// `Finalized{h}` answers this node served inside the window with a σ slot
    /// present.
    pub certs_seen: u64,
    /// Of those, the ones whose σ slot it replaced.
    pub certs_forged: u64,
    /// The heights it forged, in order.
    pub forged_heights: Vec<u64>,
    /// Every forged answer decoded back to `Some(σ')` with `σ' != σ`.
    pub forged_seed_differs: bool,
    /// Every forged answer's multisig half re-encoded byte-identically: the bitmap
    /// and aggregate were not touched.
    pub forged_vote_half_intact: bool,
    /// The heights this non-forging node served whose σ had already been served under
    /// a different round. A σ is unique per `(round, PK)`, so an honest archive can
    /// never produce one.
    pub served_seed_replays: Vec<u64>,

    /// `Latest` answers this serve side inflated by `LATEST_INFLATION`.
    pub latest_inflated: u64,
    /// The last real / forged tip height; `inflate_to == inflate_from +
    /// LATEST_INFLATION` is the delta witness.
    pub inflate_from: Option<u64>,
    pub inflate_to: Option<u64>,
    /// Every inflation added exactly `LATEST_INFLATION`.
    pub inflate_delta_ok: bool,
    /// Every inflated answer still passed `verify_jump_structural` (payload
    /// re-pointed to the new digest).
    pub inflate_structural_ok: bool,

    /// `Latest` answers this serve side forged the `result` of.
    pub result_forged: u64,
    /// The last real / forged tip `result`.
    pub forged_result_from: Option<B256>,
    pub forged_result_to: Option<B256>,
    /// Every forge changed the result (`result' != result`).
    pub result_differs: bool,
    /// Every forged answer still passed `verify_jump_structural`.
    pub result_structural_ok: bool,

    /// `Finalized{h}` by-height pulls this serve side answered with the `h − 1` pair.
    pub wrong_height_served: u64,
    /// `(requested h, served height)` for each substitution.
    pub wrong_height_pairs: Vec<(u64, u64)>,
    /// Every substitution served a wholly real, self-consistent finalization of
    /// height `requested − 1`: nothing was mutated, only the wrong height was sent.
    pub wrong_height_valid: bool,
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
    /// The production handler, over the stand's one runtime context — nothing about
    /// `deliver`'s five checks is re-implemented here.
    inner: FrontierHandler<deterministic::Context>,
    counters: UpstreamCounters,
}

impl CountingHandler {
    pub(super) fn new(
        inner: FrontierHandler<deterministic::Context>,
        counters: UpstreamCounters,
    ) -> Self {
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

/// Distinct payloads one node's `BROADCAST_CHANNEL` receiver saw, keyed by
/// `(mux sub-channel, sender)` — what the buffered body engine's per-sender deque
/// would have to hold. Payloads are keyed by their keccak, so a re-broadcast of the
/// same body counts once, as it does in the engine's deque.
#[derive(Clone, Debug, Default)]
pub(super) struct BodyTap(Arc<Mutex<BodiesBySender>>);

/// `(sub-channel, sender) → the keccaks of the distinct payloads seen`.
type BodiesBySender = BTreeMap<(u64, PeerPubkey), BTreeSet<B256>>;

impl BodyTap {
    fn record(&self, subchannel: u64, sender: PeerPubkey, payload: &[u8]) {
        self.0
            .lock()
            .unwrap()
            .entry((subchannel, sender))
            .or_default()
            .insert(keccak256(payload));
    }

    /// `(sub-channel, sender) → distinct bodies`.
    pub(super) fn snapshot(&self) -> BTreeMap<(u64, PeerPubkey), usize> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.len()))
            .collect()
    }
}

/// A `Receiver` that hands every frame through unchanged and, when it carries a
/// [`BodyTap`], records the frame's sub-channel and payload first. Every plane
/// channel is wrapped in it; only the broadcast channel's carries a tap.
#[derive(Debug)]
pub(super) struct TapReceiver<R> {
    inner: R,
    tap: Option<BodyTap>,
}

impl<R> TapReceiver<R> {
    pub(super) fn new(inner: R, tap: Option<BodyTap>) -> Self {
        Self { inner, tap }
    }
}

impl<R: Receiver<PublicKey = PeerPubkey>> Receiver for TapReceiver<R> {
    type Error = R::Error;
    type PublicKey = PeerPubkey;

    async fn recv(&mut self) -> Result<Message<PeerPubkey>, R::Error> {
        let (sender, buf) = self.inner.recv().await?;
        if let Some(tap) = &self.tap {
            if let Ok((subchannel, payload)) = mux::parse(buf.clone()) {
                tap.record(subchannel, sender.clone(), payload.as_ref());
            }
        }
        Ok((sender, buf))
    }
}
