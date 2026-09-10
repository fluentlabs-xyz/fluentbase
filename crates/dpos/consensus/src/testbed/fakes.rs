//! The fakes below the consensus seams. See the module doc for what each lies about.

use crate::{
    application::{
        BeaconEngineLike, DerivedBlockBuilder, ExecutedChain, FinalizedCursor, OrderingAssembler,
    },
    beacon::seed::Seed,
    cert_follow::{CertUpstream, UpstreamFinalized},
    cert_inlet::CommitteeSource,
    cold_start_jump::{ElSync, SyncFailure, EL_SYNC_STALL_ESCAPE},
    fault::EngineError,
    order_block::{OrderBlock, K},
    plane_upstream::{FrontierHandler, FrontierKey},
    scheme::epoch_committee_from_snapshot,
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
use commonware_runtime::{deterministic, Clock as _};
use commonware_utils::channel::oneshot as cw_oneshot;
use eyre::{ensure, eyre};
use fluentbase_bls::{
    oracle::SeedOracle, scheme::build_verifier, BlsPubkey, PeerPubkey, Scheme as BlsScheme,
};
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

/// `dposActivationBlock` of every stand: the ordering chain starts at genesis.
/// ONE binding, because it is read in four places that must agree — the contract
/// read ([`FakeStaking::dpos_activation_block`]), `OuterBuilder`, and the jump's
/// two clamps ([`JumpElSync`]'s and `cold_start_jump_with_threshold`'s own).
pub(super) const DPOS_ACTIVATION_BLOCK: u64 = 0;

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

/// One executed block as the devp2p peer network holds it: its height and the
/// hash of its parent, so a jumping node can WALK a served tip's branch down to a
/// fork point it already holds — exactly as reth's backfill downloads bodies by
/// parent hash.
#[derive(Clone, Copy, Debug)]
pub(super) struct ElBlock {
    pub height: u64,
    pub parent: B256,
}

/// What the NETWORK executed, keyed BY HASH → `(height, parent)` — the stand's
/// devp2p EL peer. Every node publishes a block here the moment its own executor
/// finalizes it (`advance_finalized`), and a node that EL-syncs walks a served
/// tip's parent chain out of it exactly as reth backfills bodies from peers.
///
/// Keyed by HASH, not by height, on purpose: a divergent branch coexists with the
/// honest chain under its own hashes (an honest node published height `h` under
/// one hash, a divergent node under another), so a lying upstream that publishes a
/// second branch (R-001 var Б) can actually be SERVED it — which a
/// first-writer-wins-by-height map made structurally impossible. The fork-safety
/// that map provided is now `land_jump`'s job: it lands a served block only where
/// the CANONICAL chain has no block at that height yet, and refuses
/// (`ConflictingPrefix`) where the two disagree.
///
/// This is the seam that makes a re-jump mean anything: EL sync does not need σ,
/// because the block arrives fully formed with its `prev_randao` already in the
/// header — which is why a node parked for want of an epoch key can still be
/// carried forward by it.
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

    /// Publish a whole branch of `(hash, height, parent)` triples at once — the
    /// seam a lying-upstream role uses to seed a divergent prefix into the peer
    /// network before serving a tip that points at it.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub(super) fn publish_branch(&self, branch: &[(B256, u64, B256)]) {
        for &(hash, height, parent) in branch {
            self.publish(hash, height, parent);
        }
    }
}

/// What [`FakeChain::land_jump`] did. The two refusals are NOT the same event and
/// map to different production `SyncFailure`s, which the executor treats
/// differently (`InvalidTarget` rotates the upstream at once, `StalledWithPeers`
/// deliberately does NOT — `executor.rs:1297-1353`).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum JumpLanding {
    /// The served branch's prefix was copied into the CANONICAL chain (the reth
    /// backfill side effect). The finalized cursor is NOT advanced here — that is
    /// the executor's `reseed_forward` on a `Landed` OUTCOME (`executor.rs:2386`).
    Landed,
    /// The devp2p peer holds no such hash — the walk falls off the served tip
    /// immediately, nothing to serve.
    Unservable,
    /// The peer serves the branch, but at `h` a walked block sits where this node's
    /// CANONICAL chain already holds a DIFFERENT hash — the served branch
    /// contradicts executed history. A rendered verdict on the branch, not a
    /// timeout.
    ConflictingPrefix {
        height: u64,
        mine: B256,
        served: B256,
    },
}

/// One executed block as reth's engine tree holds it: `InsertExecutedBlock`
/// lands it in the tree-private `TreeState.blocks_by_hash`, which NO provider
/// method reads (`.claude/RETH_INTERNALS.md` engine-tree verdict (c), and the
/// chain-state resolution-order table: `block_hash(n)` / `header(&hash)` see
/// "canonical in-memory (unpersisted)" but NOT "TreeState-only, pre-FCU").
#[derive(Clone, Copy, Debug)]
struct TreeBlock {
    height: u64,
    parent: B256,
}

/// One EL-tier transition of a node's [`FakeChain`], in the order it happened.
///
/// The two tiers are only meaningful RELATIVE to each other — "the guard read
/// height `h` while `h` was still tree-only" is an ORDERING claim, and an
/// absent log line cannot prove it. One interleaved log is therefore the
/// observable, not two per-tier lists: a test compares the INDEX of a
/// [`Self::Derived`] against the index of the [`Self::Canonicalized`] for the
/// same height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ElEvent {
    /// `derive_and_execute` sealed this block and put it in the TREE. Nothing is
    /// canonical yet.
    Derived(u64, B256),
    /// A `fork_choice_updated` (or a jump landing) made this block CANONICAL at
    /// this height — the point from which `spec_executed_hash` answers it.
    Canonicalized(u64, B256),
}

/// reth's two tiers over the same blocks, as the executor's guards read them.
///
/// * `tree` — EXECUTED but not canonical: what an `InsertExecutedBlock` leaves
///   behind, keyed by hash so a same-height sibling coexists with the canonical
///   block (fork delta §11, `f6fb181`: the already-known gate is by HASH, not by
///   height, so a sibling is inserted rather than dropped).
/// * `canonical` — the chain by NUMBER that `provider.block_hash(n)` answers,
///   and therefore the ONLY thing
///   [`ExecutedChain::spec_executed_hash`](crate::application::ExecutedChain::spec_executed_hash)
///   can see. A block enters it only when a `fork_choice_updated` names it, or
///   a DESCENDANT of it, as head ([`FakeBeacon::fork_choice_updated`] →
///   [`FakeChain::canonicalize`], which walks the head's parent chain down to the
///   fork point) — or when the devp2p peer backfills it under a jump landing
///   ([`FakeChain::land_jump`]). Naming a head that is ALREADY canonical at its
///   height — an ancestor included — is a no-op (reth skips a backward FCU to an
///   ancestor); the suffix above `head_h` is dropped (`retain(h <= head_h)`) only
///   when the head is NOT canonical there, i.e. on the walk back to the fork
///   point — the reorg half of `NewCanonicalChain`.
///
/// # Divergences from reth
///
/// What is MODELLED here, with the anchor it is modelled from:
///
/// * derive/import do not canonicalize; only an FCU (or a backfill landing)
///   does — verdict (c), `.claude/RETH_INTERNALS.md` "engine tree" §Verdict (c)
///   (`engine/tree/src/tree/mod.rs:1551-1576`). This is the R-006 tier: guard
///   #2 in `executor.rs:3140-3180` runs BEFORE the delivered block's own FCU
///   (`executor.rs:3308`), so it reads `None` at that height.
/// * an already-known `(height, hash)` import is a silent no-op — fork delta
///   §11 (`f6fb181`; fire-and-forget, no response channel). Structural here:
///   the tree insert is an `or_insert`, so a re-import of the same hash changes
///   nothing and answers `Valid` exactly as reth's skip does.
/// * a new-head FCU commits `[fork point ..= head]` and DROPS the old canonical
///   suffix above the head — reth's `NewCanonicalChain::Commit|Reorg` →
///   `on_canonical_chain_update`, §"forkchoice_updated paths" branch 4.
/// * an FCU whose head is already canonical at its own height is a NO-OP ON THE
///   CANONICAL CHAIN — branches 2 and 3 of the same list. Only the chain: reth's
///   branch 2 (head == tip) still re-runs safe/finalized while branch 3 (head is
///   an ancestor) skips them (RETH_INTERNALS §Gotchas 1), and this fake tracks
///   no safe/finalized tags at all, so it cannot tell the two apart.
/// * an FCU whose head the tree cannot LINK to the current canonical chain
///   canonicalizes NOTHING and answers `SYNCING` — reth's branch 5
///   (`handle_missing_block`: SYNCING + `DownloadRequest::single_block`, verdict
///   (d)). The walk is fail-closed on purpose: committing a segment whose
///   ancestor walk fell off the tree would leave a GAPPED or UNLINKED canonical
///   map — `{0, 5}` from a head at 5 with an absent parent, or a `3'` that is
///   not a child of `2` — a chain shape reth cannot produce. Every executor FCU
///   site tolerates SYNCING (`fcu.is_valid() || fcu.is_syncing()` on the
///   finalize path `executor.rs:3309` and the speculative path `:2717`; the
///   gap-walk does not inspect the response at all), so the answer costs no
///   liveness where the head IS linked.
///
/// What is NOT modelled, and why:
///
/// * **`PayloadStatusEnum::ACCEPTED` is never produced.** True of reth too —
///   §"new_payload outcome map" ends with "ACCEPTED never produced" — so this
///   is a divergence only against the engine API, not against reth. Nothing to
///   model.
/// * **`SYNCING` is produced ONLY for an unlinkable FCU head (above); the other
///   two reth SYNCING sources are absent, and the IMPORT leg never answers it.**
///   reth's FCU also answers SYNCING while a backfill holds the engine
///   (§"forkchoice_updated paths" step 1) — the stand has no backfill, so that
///   window does not exist. On the import leg there is nothing to diverge from:
///   production's `import_derived` is `InsertExecutedBlock`, which is
///   fire-and-forget and synthesises `Valid` unconditionally
///   (`node/src/importer.rs:112-133`) — it is not `new_payload`, so the
///   parent-state-missing SYNCING of the `new_payload` outcome map never
///   applies. The import leg's only production-side gap is the transport `Err`,
///   covered by the engine-fatal bullet below.
/// * **`PayloadStatusEnum::Invalid` is never produced, by either call.** reth
///   answers INVALID for a head descending from its `InvalidHeaderCache`
///   (populated from blocks IT downloaded and rejected) and for a payload that
///   fails validation (§"new_payload outcome map", §"Invalid handling"). The
///   executor keys THREE distinct behaviours on it — #15 `SafetyHalt(ElInvalid)`
///   on the finalize FCU (`executor.rs:3309-3320`), a plain
///   `Defer(SpecFcuRejected)` on the speculative FCU (`:2717-2731`, the
///   deliberate asymmetry), and a halt on an `Invalid` IMPORT from either path —
///   and none of the three is reachable here. Not modelled because the stand has
///   no second source of blocks: every block it judges is one its own deriver
///   produced, so there is nothing for an invalid-ancestor verdict to be ABOUT.
///   Note also that an INVALID verdict is not a permanent oracle in reth (a
///   cache entry evicts after 128 hits, and a transient consensus error is not
///   cached at all — RETH_INTERNALS §"Invalid handling"), which is a second
///   reason to leave it to `executor.rs`'s own fixture (`fcu_status` /
///   `import_status`) rather than approximate it here.
/// * **`safe_block_hash` and `finalized_block_hash` are IGNORED.**
///   `fork_choice_updated` reads only `head_block_hash`. reth validates both on
///   branches 2 and 4 and rejects the WHOLE forkchoice as `invalid_state` when
///   either names a hash it cannot resolve (§"forkchoice_updated paths" step 2,
///   §"Finalized/safe tracking"; the zero hash is a legal no-op) — the condition
///   `executor.rs`'s `fcu_anchor_inconsistent` fixture exists for. The executor
///   reasons about the tags (`update_safe`'s "safe is ALWAYS canonical-findable
///   at this FCU" argument, `executor.rs:3283-3296`), and this fake can neither
///   confirm nor refute that argument: it neither validates the tags nor tracks
///   them, so `finalized ⊆ safe ⊆ head` is unchecked here.
/// * **The ancestor-FCU short-circuit is modelled as "no chain change" but NOT
///   as "safe/finalized are skipped too."** reth's branch 3 returns VALID
///   without touching safe/finalized (verdict (b)); this fake tracks no
///   safe/finalized TAGS at all — the finalized tier here is the executor's own
///   [`FinalizedCursor`], which is advanced by `advance_finalized` and never by
///   an FCU. A test about the DPoS finalized-freeze (verdict (a)+(b)) therefore
///   cannot be written against this fake.
/// * **The sync-target-gated eager `MakeCanonical` is absent.** reth
///   canonicalizes a new_payload / buffered / downloaded block with no FCU when
///   its hash equals the head of the last SYNCING FCU (verdict (a)). Reaching
///   it needs a SYNCING FCU first, which this fake never answers, so the whole
///   branch is unreachable rather than omitted.
/// * **The cert-follow finalized-freeze is absent** for the same reason: it is
///   verdict (a) + verdict (b) composed, and both legs are missing above.
/// * **An engine-fatal `Err` is never produced.** reth's tree can answer
///   `Internal(InsertBlockFatalError)` and then the tree thread EXITS, so every
///   later call is `EngineUnavailable` (§"Service topology", Handle Err
///   semantics; §Gotchas 6). Both calls here are infallible. The executor's
///   transport-vs-verdict split is tested in `executor.rs`'s own fixture
///   (`fcu_transport_errs` / `import_transport_errs` /
///   `fcu_anchor_inconsistent`), which is where that behaviour belongs; the
///   stand asserts on multi-node agreement, not on engine-boundary taxonomy.
/// * **The by-HASH header-index lag is absent.** reth resolves `header(hash)`
///   only once an FCU canonicalized the segment (verdict (c)); a block landed
///   by devp2p backfill is by-NUMBER visible first. `FakeDeriver` never reads
///   the parent by hash, so the parent-visibility park (`executor.rs:3063`) is
///   unreachable here. `executor.rs`'s `ByHashVisibility` models it.
/// * **`executed_tip()` conflates two different reth reads.**
///   `ProviderExecutedChain::executed_tip` is `last_block_number()`, which the
///   resolution-order table marks DB-only — it does NOT see canonical
///   in-memory blocks and lags the head by the persistence threshold. Here it
///   is the canonical tip, i.e. no persistence lag. Consequence: the fake's
///   [`Self::executed_state_hash`] (production's `executed.rs:45-65`, which
///   gates on `best_block_number()` — a THIRD read, the canonical head) uses
///   that same tip, so the stand cannot express the `best` vs `last` skew that
///   `executed.rs`'s doc is about.
/// * **No persistence, no pruning, no backfill `clear_state()`.** Nothing here
///   ever leaves the canonical map except at a reorg, so a deep-history provider
///   miss below the finalized cursor — the `None` arm `FinalizedCursor::resolve`
///   exists for — cannot happen in the stand.
#[derive(Clone, Default)]
pub(super) struct FakeChain {
    /// Tier-T: every executed block by hash, canonical or not.
    tree: Arc<Mutex<BTreeMap<B256, TreeBlock>>>,
    /// Tier-S: the canonical chain by number — `provider.block_hash(n)`.
    canonical: Arc<Mutex<BTreeMap<u64, B256>>>,
    finalized: FinalizedCursor,
    /// The highest height the executor advanced the finalized cursor to — the
    /// tier-F tip. The stand compares nodes on THIS tier: tier-S at a height a
    /// node has not finalized yet may hold a notarized-then-nullified sibling.
    finalized_tip: Arc<AtomicU64>,
    /// The σ the executor handed `derive_and_execute` at each height (LAST
    /// writer wins — a re-derive of the same height overwrites, which is what
    /// the tier split above deliberately does NOT do to `canonical`): `None` in
    /// a beacon-INACTIVE epoch. The object the live-beacon tests compare across
    /// nodes.
    seeds: Arc<Mutex<BTreeMap<u64, Option<Seed>>>>,
    /// The devp2p peer this node's EL syncs from. Written on every finalized
    /// height this node executes; read only by a re-jump landing.
    el_network: ElNetwork,
    /// Every tier transition in order — see [`ElEvent`]. The observable behind
    /// "the guard read `h` BEFORE `h` was canonical": that is an ordering claim,
    /// and no absent log line can carry it.
    el_events: Arc<Mutex<Vec<ElEvent>>>,
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

    /// `InsertExecutedBlock`: the block is EXECUTED and reachable by hash, and
    /// nothing about the canonical chain changes. An already-known hash is a
    /// silent no-op on the block itself (fork delta §11) — with ONE repair: a
    /// stored parent of `B256::ZERO` is a PLACEHOLDER, not a claim, and a later
    /// insert that knows the real parent replaces it.
    ///
    /// The placeholder exists because [`Self::note_hash`] registers a replayed
    /// node's persisted finalized marker before any body has been re-derived, so
    /// it cannot know the parent. Without the repair, the honest re-derive of
    /// that same hash could never link it — an `or_insert` would be a no-op —
    /// and [`Self::canonicalize`]'s ancestor walk would fall off the tree at
    /// exactly the one height a replay has to cross.
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

    /// An FCU naming `head`: reth commits `[fork point ..= head]` into the
    /// canonical chain and drops the old suffix above `head`
    /// (`NewCanonicalChain::Commit|Reorg` → `on_canonical_chain_update`,
    /// RETH_INTERNALS "forkchoice_updated paths" branch 4). A head already
    /// canonical at its own height is branch 2/3: no change to the chain.
    ///
    /// Returns `false` when the head cannot be LINKED to the current canonical
    /// chain — unknown to the tree, or its ancestor walk falls off the tree
    /// before meeting a block the canonical chain already holds. That is reth's
    /// branch 5 (`handle_missing_block`), and the caller answers `SYNCING`.
    ///
    /// FAIL-CLOSED IS THE POINT. Committing a segment whose walk did not reach a
    /// fork point writes a chain shape reth cannot produce: head 5 with an absent
    /// parent over `{0}` gives `{0, 5}` (a hole), and head 3 with an absent
    /// parent over `{0..4}` gives `{0,1,2,3'}` with `3'` not a child of `2` (an
    /// unlinked chain). Every consumer of `spec_executed_hash` — the result
    /// gates, the gap-walk's backward probe, [`Self::land_jump`]'s prefix
    /// comparison — reads that map as a CHAIN, so a fake that can emit a
    /// non-chain is a fake oracle again, in the same shape as the one this tier
    /// split removed.
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
        // already holds at that height. Reaching height 0 without meeting one
        // means even genesis disagrees — not a fork point, a broken chain.
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
        true
    }

    /// Make `hash` canonical at `height` outright: the genesis anchor, and the
    /// devp2p-backfilled prefix a jump landing copies in (reth declares that
    /// segment canonical AND executed before `sync_to` returns —
    /// `cold_start_jump.rs:442-613`, so a landing is a whole executed prefix and
    /// never a hash on a hole).
    ///
    /// The parent is `canonical[height - 1]`, and its ABSENCE is a stand
    /// invariant violation rather than a case to fall back on: the only two
    /// callers are genesis (`height == 0`, whose parent IS `B256::ZERO` —
    /// `genesis_sealed`) and [`Self::land_jump`], which walks the peer's
    /// executed range in ASCENDING order and has therefore already landed
    /// `height - 1`. Silently storing `B256::ZERO` here would plant exactly the
    /// unlinked tree entry [`Self::canonicalize`] now refuses to commit, one
    /// call too early to see it.
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

    /// Whether `hash` is on the CANONICAL chain — production's
    /// `provider.block_number(hash)`, which the resolution-order table marks
    /// memory→DB and blind to TreeState-only blocks. Deliberately distinct from
    /// [`Self::height_of`], which reads the tree.
    fn canonical_holds(&self, hash: B256) -> bool {
        self.canonical.lock().unwrap().values().any(|x| *x == hash)
    }

    /// Every tier transition this node's EL made, in order.
    pub(super) fn el_events(&self) -> Vec<ElEvent> {
        self.el_events.lock().unwrap().clone()
    }

    /// The devp2p backfill a re-jump's `sync_to` FCU drives: WALK the served tip
    /// `hash`'s parent chain out of [`ElNetwork`] down to a fork point the
    /// CANONICAL chain already holds, and commit the walked segment canonically
    /// (`cold_start_jump.rs:442-613` — reth declares the branch canonical AND
    /// executed before `sync_to` returns, so a landing is a whole executed prefix
    /// and never a hash on a hole). This is the reth SIDE EFFECT only: it does NOT
    /// advance the finalized cursor — that is the executor's job in
    /// `reseed_forward` (`executor.rs:2386`), and only on a `Landed` OUTCOME.
    ///
    /// The two refusals are DIFFERENT conditions and the caller must not fold them
    /// together — see [`JumpLanding`]:
    ///   * `Unservable` — the peer holds no such hash (an unknown / inflated
    ///     target), the walk falls off the tip immediately;
    ///   * `ConflictingPrefix` — a walked block sits at a height where THIS node's
    ///     canonical chain already holds a DIFFERENT hash (the served branch
    ///     contradicts executed history).
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
            // The canonical chain holds a DIFFERENT hash at this height: the served
            // branch contradicts our own executed history.
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
        JumpLanding::Landed
    }

    /// Record `hash` as an EXECUTED block at `height` without making it
    /// canonical — a tree-only entry ([`Self::insert_tree`]). Used for a
    /// replayed node's persisted finalized marker: the one height a restarted
    /// node knows a hash for before it has re-derived anything (reth's
    /// finalized marker survives the process; the stand's block bodies do not).
    ///
    /// DIVERGENCE, deliberate: after a real restart that block IS canonical in
    /// reth's DB. Landing it canonically here would give the replayed node a
    /// canonical hash at a height whose body this process never derived, i.e. a
    /// canonical chain with a hole under it — which is not what the replay
    /// tests are modelling (they re-derive `1..=height` into a fresh
    /// [`FakeChain`]).
    pub(super) fn note_hash(&self, height: u64, hash: B256) {
        self.insert_tree(height, hash, B256::ZERO);
    }

    /// Height of an executed hash, or `None` when this chain never sealed it.
    /// Reads the TREE, not the canonical chain: a reorged-out sibling still has
    /// a number, and this is what [`FakeStaking`] answers "what was the contract
    /// state at this hash" from.
    pub(super) fn height_of(&self, hash: B256) -> Option<u64> {
        self.tree.lock().unwrap().get(&hash).map(|b| b.height)
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
}

impl ExecutedChain for FakeChain {
    /// Production: `provider.last_block_number()` (`node/src/ordering.rs:42`) —
    /// the DB tier. Here: the CANONICAL tip (see the type's divergence list,
    /// "`executed_tip()` conflates two different reth reads").
    fn executed_tip(&self) -> u64 {
        self.canonical
            .lock()
            .unwrap()
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0)
    }
    /// Production: `provider.block_hash(height)` (`node/src/ordering.rs:46`) —
    /// reth's canonical chain by number. Here: the `canonical` map, which only
    /// an FCU or a jump landing writes. Tier-S.
    fn spec_executed_hash(&self, height: u64) -> Option<B256> {
        self.spec_hash_at(height)
    }
    /// Production: the same canonical read gated by the [`FinalizedCursor`]
    /// (`node/src/ordering.rs:57-59`). Identical here — tier-F IS tier-S below
    /// the cursor, in both.
    fn finalized_executed_hash(&self, height: u64) -> Option<B256> {
        self.finalized.resolve(height, |h| self.spec_hash_at(h))
    }
    /// Production: the cursor only (`node/src/ordering.rs:62`). Here the cursor
    /// plus two stand-only bookkeeping writes — the tier-F tip the driver samples
    /// and the [`ElNetwork`] publish that makes this node a devp2p peer.
    fn advance_finalized(&self, height: u64) {
        let from = self.finalized_tip.load(Ordering::SeqCst);
        self.finalized.advance(height);
        self.finalized_tip.fetch_max(height, Ordering::SeqCst);
        // Publish what this node just finalized-executed, so a peer that has to
        // EL-sync can be served the bodies it never derived. [`ElNetwork`]'s doc
        // says "every node publishes a height the moment its executor finalizes
        // it", and this loop is the only writer, so a SILENT skip here would
        // make that doc false and thin the devp2p peer without a trace.
        for h in from + 1..=height {
            match self.spec_hash_at(h) {
                // Publish by HASH with the parent hash, so a jumping peer can walk
                // this branch down to a fork point (`land_jump`). The parent is the
                // canonical hash below `h` (genesis's parent is `ZERO`).
                Some(x) => {
                    let parent = h
                        .checked_sub(1)
                        .and_then(|below| self.spec_hash_at(below))
                        .unwrap_or(B256::ZERO);
                    self.el_network.publish(x, h, parent);
                }
                // A height THIS node derived must be canonical before the cursor
                // passes it — that is the executor's own canonical postcondition
                // (`executor.rs:3389`, the re-apply loop, which cannot exit until
                // `spec_executed_hash(h) == derived_hash`). If it is not, the tier
                // split above is broken and every later result-gate read is
                // sampling a chain that does not exist. Fail loudly rather than
                // publish a thinner history.
                None if self.seeds.lock().unwrap().contains_key(&h) => panic!(
                    "testbed: finalized cursor advanced to {height} past {h}, which this node \
                     DERIVED but never canonicalized — the FCU that should have committed it \
                     did not, or a later reorg dropped it"
                ),
                // A height this node never derived: the ONE legitimate case is a
                // replay, whose `Actor::init` seeds the cursor from the marshal's
                // durable acked height (`executor.rs:1045`) while the fresh
                // `FakeChain` has no bodies below it — the stand does not persist
                // block bodies (see `note_hash`). Nothing to publish; the nodes
                // that DID derive those heights published them already.
                None => {}
            }
        }
    }
}

/// derive = `sealed_at(parent, height, keccak(order digest ‖ prev_randao(seed)))`.
/// With `divergent_at = Some(h)` the block at `h` seals to a DIFFERENT hash on
/// this node only — the `Role::DivergentResult` fault: K blocks later this node
/// commits (and expects) a `result` nobody else derived.
///
/// The derived block lands in the [`FakeChain`] TREE and nowhere else:
/// production's `derive_and_execute` (`node/src/derive.rs:98`) executes against
/// the parent's state and hands back a sealed block — it canonicalizes nothing,
/// and `import_derived` after it canonicalizes nothing either. Only the FCU
/// does.
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

/// The engine boundary over the SAME [`FakeChain`] the [`ExecutedChain`] reads —
/// one struct of truth, so "what the executor imported" and "what the executor
/// can read back" cannot drift apart the way they did while the beacon recorded
/// nothing.
///
/// `import_derived` inserts into the tree (canonical chain untouched) and always
/// answers `Valid`, exactly as production's `InsertExecutedBlock` seam does
/// (`node/src/importer.rs:112-133` — fire-and-forget, the status is synthesised).
/// `fork_choice_updated` canonicalizes `[fork point ..= head]` and answers
/// `Valid`, or — when the head cannot be linked to the canonical chain —
/// canonicalizes nothing and answers `SYNCING`, reth's missing-block branch 5.
/// The statuses and errors that are NOT modelled (INVALID on either call, the
/// backfill-window SYNCING, a transport `Err`, and the ignored safe/finalized
/// tags) are listed on [`FakeChain`] with the reason each is absent.
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
        // see the divergence block on `FakeChain`.
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
        Ok(DPOS_ACTIVATION_BLOCK)
    }
    fn active_registry_peers(&self, at: B256) -> Result<Vec<PeerPubkey>, ReadError> {
        self.height_at(at)?;
        Ok(self.registry.as_ref().clone())
    }
}

/// Every `committee[E]` read the jump made, in call order: `(epoch, executed
/// hash it was read AT)`. The observation behind "the committee was read at the
/// LANDING hash" — recorded by [`JumpCommittees`] itself, so a test asserts what
/// production asked for instead of inferring it from the outcome.
pub(super) type JumpCommitteeReads = Arc<Mutex<Vec<(u64, B256)>>>;

/// One `ReJump::call` — one run of the production
/// [`cold_start_jump_with_threshold`](crate::cold_start_jump::cold_start_jump_with_threshold).
///
/// The `outcome` tag exists because EVERY non-`Landed` variant is otherwise
/// INVISIBLE to a test: the stand wires `ReJump::rotate = None`, so
/// `Executor::rotate_upstream` is a silent no-op, and an `AuthFailed` or
/// `BadTarget` leaves nothing behind but a WARN line. Without the tag a test
/// asserting `rejump_calls >= 1` passes just as happily on a chain where every
/// jump authenticated and FAILED.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct JumpCall {
    /// The anchor the executor passed (its `ordering_finalized` cursor).
    pub from: u64,
    /// The `JumpOutcome` variant name, verbatim.
    pub outcome: &'static str,
    /// `(height, result)` of the upstream certificate this call CONSUMED —
    /// recorded by [`TeeingUpstream`] at the jump's own `get_latest`, so the
    /// landing can be checked against the cert it came from rather than against
    /// the chain the landing just wrote.
    pub consumed: Option<(u64, B256)>,
    /// `(landing, hash)` on [`JumpOutcome::Landed`](crate::cold_start_jump::JumpOutcome::Landed).
    pub landed: Option<(u64, B256)>,
    /// The `Display` text of the `eyre::Report` a NON-terminal outcome carries
    /// (`AuthFailed`, `Stalled`, `BadTarget`, …) — `None` for `Landed`/`Lagging`.
    /// Lets a test tell WHICH refusal arm fired: `verify_jump_authenticated`
    /// returns an error on the committee-BLS `ensure!` AND on an unreadable
    /// committee, and only the message distinguishes them.
    pub outcome_detail: Option<String>,
}

/// Every jump call one node made, in call order.
pub(super) type JumpCalls = Arc<Mutex<Vec<JumpCall>>>;

/// A one-shot tee over the upstream the jump reads: records the
/// `(height, result)` of the `UpstreamFinalized` the jump actually consumed.
/// Built fresh per `ReJump::call`, so its slot holds THAT call's certificate and
/// not whatever the executor's frontier probe asked for in between.
#[derive(Clone)]
pub(super) struct TeeingUpstream<U> {
    inner: U,
    consumed: Arc<Mutex<Option<(u64, B256)>>>,
}

impl<U: CertUpstream> TeeingUpstream<U> {
    pub(super) fn new(inner: U) -> Self {
        Self {
            inner,
            consumed: Arc::new(Mutex::new(None)),
        }
    }

    pub(super) fn consumed(&self) -> Option<(u64, B256)> {
        *self.consumed.lock().unwrap()
    }
}

impl<U: CertUpstream> CertUpstream for TeeingUpstream<U> {
    async fn get_finalization(&self, height: Height) -> Option<UpstreamFinalized> {
        self.inner.get_finalization(height).await
    }
    async fn get_latest(&self) -> Option<UpstreamFinalized> {
        let got = self.inner.get_latest().await;
        if let Some(uf) = &got {
            *self.consumed.lock().unwrap() = Some((uf.block.height, uf.block.result));
        }
        got
    }
    async fn rotate(&self) {
        self.inner.rotate().await
    }
}

/// The jump's committee read over [`FakeStaking`], BY EXECUTED HASH — the stand's
/// [`RethCommitteeSource`](crate::cert_inlet::RethCommitteeSource). Built exactly
/// as the node's steady-state re-jump builds it (`consensus/src/dpos.rs:2503-2513`):
/// a state reader plus the chain namespace plus a finalized-tip hash closure.
/// `verify_jump_authenticated` calls `scheme_at(epoch, landing_hash, None)`, so
/// the committee comes out of the CONTRACT STATE MACHINE at the landing — never
/// out of the stand's schedule.
pub(super) struct JumpCommittees {
    staking: FakeStaking,
    namespace: Vec<u8>,
    finalized_hash: Arc<dyn Fn() -> Option<B256> + Send + Sync>,
    reads: JumpCommitteeReads,
}

impl JumpCommittees {
    pub(super) fn new(
        staking: FakeStaking,
        namespace: Vec<u8>,
        finalized_hash: Arc<dyn Fn() -> Option<B256> + Send + Sync>,
        reads: JumpCommitteeReads,
    ) -> Self {
        Self {
            staking,
            namespace,
            finalized_hash,
            reads,
        }
    }

    fn build_at(
        &self,
        epoch: u64,
        at_hash: B256,
        oracle: Option<Arc<dyn SeedOracle>>,
    ) -> eyre::Result<BlsScheme> {
        let snap = self.staking.epoch_committee_snapshot(epoch, at_hash)?;
        ensure!(
            !snap.validators.is_empty(),
            "epoch {epoch} has no committed committee at {at_hash}"
        );
        let committee = epoch_committee_from_snapshot(&snap)
            .map_err(|e| eyre!("epoch {epoch} committee has non-unique participants: {e:?}"))?;
        Ok(build_verifier(
            &self.namespace,
            committee.bimap,
            epoch,
            oracle,
        ))
    }
}

impl CommitteeSource for JumpCommittees {
    fn scheme_at(
        &self,
        epoch: u64,
        at_hash: B256,
        oracle: Option<Arc<dyn SeedOracle>>,
    ) -> eyre::Result<BlsScheme> {
        self.reads.lock().unwrap().push((epoch, at_hash));
        self.build_at(epoch, at_hash, oracle)
    }

    fn scheme_at_finalized_tip(
        &self,
        epoch: u64,
        oracle: Option<Arc<dyn SeedOracle>>,
    ) -> eyre::Result<Option<BlsScheme>> {
        let Some(hash) = (self.finalized_hash)() else {
            return Ok(None);
        };
        let snap = self.staking.epoch_committee_snapshot(epoch, hash)?;
        if snap.validators.is_empty() {
            return Ok(None);
        }
        Ok(Some(self.build_at(epoch, hash, oracle)?))
    }
}

/// The jump's EL seam over [`FakeChain`] + [`ElNetwork`] — the stand's
/// [`RethElSync`](crate::cold_start_jump::RethElSync). Same three branches as
/// production's `sync_to`: a pre-K (`result == 0`) tip and an already-executed
/// target both answer the LOCAL landing without touching the peer, and the real
/// case drives the whole executed prefix out of the devp2p peer
/// ([`FakeChain::land_jump`]) before answering `(landing, hash)`.
///
/// What lives HERE rather than in the production function, and is therefore NOT
/// covered by driving the production jump:
///
/// * The `tip − K` landing arithmetic and the `result == 0` pre-K test are
///   INSIDE `sync_to` in production too (`cold_start_jump.rs:446-454`), so they
///   are re-stated here by the shape of the trait seam, not by choice. A change
///   to that arithmetic in production does NOT fail a stand test.
/// * Every non-`Landed` `JumpOutcome` is FIXTURE-DEPENDENT: on the current
///   honest schedules the peer always serves the attested landing, so
///   `Unservable` / `ConflictingPrefix` — and with them `StalledWithPeers` /
///   `Invalid`, and every executor branch keyed on them — are unreached. Their
///   code paths are written, not exercised.
/// * The [`EL_SYNC_STALL_ESCAPE`] sleep on an unservable target freezes the
///   node's jump for 300 VIRTUAL seconds. Nothing bounds a test's virtual clock
///   against that today; a fixture that reaches it must budget for it.
/// * The need-gate band is production's, but the stand only ever exercises the
///   `threshold = min(JUMP_THRESHOLD, epoch_len)` = 32 setting and the closed
///   `u64::MAX` one — no test lands in between.
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

    /// Production's `RethElSync::local_landing`: the EXECUTED head clamped to
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
        // PRODUCTION ORDER (`cold_start_jump.rs:478-613`): FCU
        // `head=safe=finalized=tip_hash` FIRST and let reth backfill/execute the
        // branch whose tip is that HASH; only AFTER it reports `Valid` resolve the
        // landing HEIGHT via `provider.block_hash(landing)`. The two steps are
        // separable and their failures are different:
        //   * the HASH is unservable — the peer holds no such branch, reth sits in
        //     its FCU poll loop until the connected-but-no-progress net trips
        //     (`StalledWithPeers`, which the executor deliberately does NOT rotate,
        //     `executor.rs:1331-1353`);
        //   * the served branch contradicts our executed history — reth renders an
        //     `Invalid` verdict (`executor.rs:1297-1310`, rotate-at-once);
        //   * the hash IS servable but the claimed landing HEIGHT resolves to no
        //     block (a real hash paired with an inflated height, R-004) — reth
        //     backfilled the real branch, but `block_hash(inflated) == None` →
        //     `eyre!` → `SyncFailure::Stalled` (`cold_start_jump.rs:603-611`,
        //     `From<eyre::Report>` at `:354-358`), which the executor counts toward
        //     the fault streak and rotates at `MAX_UPSTREAM_FAULTS` (a no-op with
        //     `rotate: None`).
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
                // [ГИПОТЕЗА] Whether reth would in fact answer
                // `PayloadStatusEnum::Invalid` (rather than unwinding and answering
                // `SYNCING` forever, the soak-v43 shape) for a served branch
                // conflicting with executed history is NOT verified here; the point
                // of the split is that the stand stops folding two different
                // conditions into one outcome.
                return Err(SyncFailure::Invalid(eyre!(
                    "testbed EL peer served a branch that contradicts executed history at \
                     {height}: mine {mine}, served {served} (claimed tip height {})",
                    latest.block.height
                )));
            }
        }
        // The hash landed; now resolve the CLAIMED landing height. Clamp BEFORE
        // resolving so the returned pair is always height-and-hash of the SAME
        // block. A landing the backfill did not reach (an inflated height whose
        // real block the servable hash was NOT) resolves to `None` → `Stalled`,
        // NEVER `StalledWithPeers`.
        let landing = tip_height.max(self.activation);
        let hash = self.chain.spec_hash_at(landing).ok_or_else(|| {
            SyncFailure::Stalled(eyre!(
                "testbed EL-sync backfilled the served tip {tip_hash} but no block resolves at \
                 the claimed landing height {landing}"
            ))
        })?;
        Ok((landing, hash))
    }

    /// Production's `RethElSync::holds` is `provider.block_number(hash)`
    /// (`cold_start_jump.rs:614-620`), which the RETH_INTERNALS
    /// resolution-order table marks memory→DB and BLIND to TreeState-only
    /// blocks — so it answers the CANONICAL chain, not the executed tree. The
    /// trait's own doc says as much ("holds `hash` canonically",
    /// `cold_start_jump.rs:371-374`). Unreachable on the stand's path
    /// (`l1_checkpoint` is `None`, so `cold_start_jump_with_threshold` skips the
    /// L1 re-assert), but the tier has to be right or the first fixture that
    /// wires a checkpoint inherits a fake oracle.
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
    /// `deliver` calls that did NOT decode (`false` — undecodable payload). The
    /// wrong-height R-009 pairs DECODE and land in `deliveries_decoded`; R-009
    /// asserts this stays 0 before П-4 binds the height (PLAN 4.2).
    pub deliveries_rejected: Arc<AtomicU64>,
    /// `ReJump::call` invocations — each one runs the PRODUCTION
    /// [`crate::cold_start_jump::cold_start_jump_with_threshold`] over
    /// [`JumpCommittees`] + [`JumpElSync`]. Stays 0 while
    /// `StandConfig::re_jump_threshold` is `None` (the gate is then `u64::MAX`,
    /// so `Executor::maybe_re_jump` never arms the waiter). A COUNT ONLY: what
    /// each call did is [`JumpCall`], and asserting on this number alone cannot
    /// tell a landing from a failed authentication.
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
/// One broadcast `ShareConfirm` as the wire saw it: the ceremony epoch it was
/// framed under, the confirming member's committee seat, and the `(seat, log
/// hash)` set it claims to hold.
#[cfg(feature = "dpos-devnet-byzantine")]
pub(super) type SentConfirm = (u64, u8, Vec<(u8, B256)>);

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
    /// Every `ShareConfirm` this node BROADCAST: the ceremony epoch it was framed
    /// under, its own committee seat, and the `(seat, log hash)` set it claims to
    /// hold. The EPOCH is carried because a reader must be able to scope the claim
    /// — a longer run mints a confirmation per target epoch, and "the last one" is
    /// then a property of the run's length. Recorded on EVERY node, the
    /// honest ones included, because it is the only place the stand can read a
    /// node's `recorded_dkg_logs` index: the confirmation is minted from that index
    /// alone (`beacon/confirmations.rs::mint`), which is written only by the
    /// ceremony's `record_checked_log`. A victim whose confirm names the FORGED
    /// hash at the dealer's seat is a direct observation that the second log was
    /// recorded — not an inference from the share it ends up without.
    pub confirms_sent: Vec<SentConfirm>,
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

    // ---- R-004, `Role::InflatedProbe` ----
    /// `Latest` answers this serve side inflated (height += `LATEST_INFLATION`).
    pub latest_inflated: u64,
    /// The last real / forged tip height — `inflate_to == inflate_from +
    /// LATEST_INFLATION` is the delta witness.
    pub inflate_from: Option<u64>,
    pub inflate_to: Option<u64>,
    /// Every inflation added exactly `LATEST_INFLATION`.
    pub inflate_delta_ok: bool,
    /// Every inflated answer still passed `verify_jump_structural` (payload
    /// re-pointed to the new digest).
    pub inflate_structural_ok: bool,

    // ---- R-001 var Б, `Role::LyingUpstream` ----
    /// `Latest` answers this serve side forged the `result` of.
    pub result_forged: u64,
    /// The last real / forged tip `result`.
    pub forged_result_from: Option<B256>,
    pub forged_result_to: Option<B256>,
    /// Every forge changed the result (`result' != result`).
    pub result_differs: bool,
    /// Every forged answer still passed `verify_jump_structural`.
    pub result_structural_ok: bool,

    // ---- R-009, `Role::WrongHeightFinalized` ----
    /// `Finalized{h}` by-height pulls this serve side answered with the `h − 1` pair.
    pub wrong_height_served: u64,
    /// `(requested h, served height)` for each substitution — the witness that the
    /// served height is EXACTLY `requested − 1` (asserted per serve, kept for the log).
    pub wrong_height_pairs: Vec<(u64, u64)>,
    /// Every substitution served a wholly real, self-consistent finalization
    /// (`payload == block.digest()`) of height `requested − 1` — NOTHING was mutated,
    /// only the wrong height was sent. The one property that separates R-009 from the
    /// mutating forgers.
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
