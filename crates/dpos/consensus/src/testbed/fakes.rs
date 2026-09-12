//! The fakes below the consensus seams. See the module doc for what each lies about.

use crate::{
    application::{
        BeaconEngineLike, DerivedBlockBuilder, ExecutedChain, FinalizedCursor, OrderingAssembler,
    },
    beacon::Seed,
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
use commonware_p2p::{utils::mux, Message, Receiver};
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
/// ONE binding, because it is read in four places that must agree — the contract
/// read ([`FakeStaking::dpos_activation_block`]), `OuterBuilder`, and the jump's
/// two clamps ([`JumpElSync`]'s and `jump_to_target`'s own).
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
/// differently (`InvalidTarget` is `Fault::corruption` since review B1-04 — the
/// EL contradicted an attested pair this node read from its own archive —
/// while `StalledWithPeers` is deliberately non-fatal and does not even rotate).
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
    /// `best − ordering_finalized` (canonical tip minus the tier-F tip) right
    /// after every FCU that moved the canonical chain, as a histogram `gap →
    /// count`. Event-driven rather than sampled: the speculative lead lives for
    /// one notarize→finalize round trip, shorter than any driver tick.
    head_gap_on_fcu: Arc<Mutex<BTreeMap<u64, u64>>>,
    /// The same gap right after every jump landing — the EL was carried to the
    /// landing while the tier-F tip is still the pre-jump one, until
    /// `reseed_forward` advances it. Kept apart from the FCU histogram because it
    /// is the jump window, not speculation.
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
        self.note_head_gap(&self.head_gap_on_landing);
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

/// Which branch of the READING node's own execution layer a state hash sits on,
/// at the moment of the read.
///
/// The distinction the epoch-pure `Members` closure cannot express: the contract
/// is a state machine over a CHAIN, and two chains that share a height can hold
/// two different committees there. `Canonical` is `provider.block_hash(h) == at`
/// (tier-S — the `canonical` map of [`FakeChain`]);
/// `Speculative` is every other hash the node's tree knows at `h` — a
/// derived-but-not-yet-canonical block, a reorged-out sibling, or the
/// tree-only marker a replayed node is handed
/// ([`FakeChain::note_hash`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Branch {
    Canonical,
    Speculative,
}

/// `(epoch, state hash, height of that hash, which branch it is on) → member
/// node indices`, or `None` for an epoch with no committee on that branch.
///
/// The BRANCHING form of [`Members`]: where that closure is a pure function of
/// the epoch — so "two nodes at different heights read the same committee" is
/// true of any caller, correct or not — this one lets the stand hand a
/// DIFFERENT committee to a read taken off the canonical chain. That is what
/// makes the cross-node equality of a committee record a property of the
/// READING code instead of a tautology of the fake.
pub(super) type BranchCommittees =
    Arc<dyn Fn(u64, &B256, u64, Branch) -> Option<Vec<usize>> + Send + Sync>;

/// One node (by index) tombstoned from `height` on, as the contract's LIVE
/// equivocation flag: `getEpochCommitteeWithStakes` reads `tombstoned` at the
/// call's own block rather than at the epoch commit
/// (`crates/staking-abi/src/lib.rs:204-214`,
/// `crates/dpos/staking-reader/src/reader.rs` — the `tombstoned` leg), which is
/// what lets a mid-epoch verdict reach the committee it names.
pub(super) type Tombstones = Arc<Vec<(usize, u64)>>;

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
    /// `epoch -> reads taken at a hash this node's canonical chain did NOT hold
    /// at that height` ([`Branch::Speculative`]).
    ///
    /// Counted whatever the committee schedule is, because it is a property of
    /// the CALLER and not of the fake: a consumer that resolves its own block
    /// hash off a speculative cursor shows up here even when every branch
    /// answers the same committee.
    ///
    /// One stand artifact belongs here rather than in a test: a REPLAYED node
    /// (`StandConfig::resume_from`) is handed its persisted finalized marker
    /// through [`FakeChain::note_hash`], which is deliberately TREE-ONLY, so
    /// the cold-start read at that marker is counted `Speculative`.
    pub speculative: BTreeMap<u64, u64>,
    /// `epoch -> reads that answered a non-empty committee with NO frozen
    /// weights` — the contract's "my weight ring has wrapped past this epoch"
    /// answer (`stakes` empty beside a non-empty `addrs`).
    pub weights_none: BTreeMap<u64, u64>,
    /// `epoch -> reads that answered at least one TOMBSTONED member`.
    pub tombstoned_seen: BTreeMap<u64, u64>,
    /// `epoch -> snapshot calls made through the COMMITTEE MODULE'S port`
    /// ([`crate::committee::EpochReads`]) alone.
    ///
    /// Separate from [`Self::committed`] because that counter cannot answer
    /// "one snapshot per epoch": it also counts the `EpochTransition`'s own
    /// reads and the two extra snapshots this fake's [`FakeStaking::dkg_qual`]
    /// issues to compute the bit. This one counts exactly what the module
    /// asked.
    pub module_snapshot: BTreeMap<u64, u64>,
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
/// * **Frozen weights come out of a RING.** The contract keeps them in
///   [`WEIGHT_RING_EPOCHS`] frames and answers an empty `stakes` leg beside a
///   non-empty `addrs` once a frame has been reused
///   (`crates/staking-abi/src/lib.rs:204-214`), which the reader decodes as
///   `weights: None`. A frame is reused at `E + WEIGHT_RING_EPOCHS`, and the
///   newest epoch committed at height `h` is `epoch(h) +
///   MAX_COMMITTEE_LOOKAHEAD_EPOCHS`, so an epoch answers `None` exactly while
///   `epoch + WEIGHT_RING_EPOCHS <= epoch(h) + MAX_COMMITTEE_LOOKAHEAD_EPOCHS`
///   — 14 epochs below the reading height, far outside the committee module's
///   own window (`committee/mod.rs::WINDOW_FITS_THE_WEIGHT_RING`). A stand can
///   therefore never reach that arm by running long enough, which is why
///   [`FakeStaking::weights_none_for`] exists as a separate switch.
///
/// NOT modelled (step 5b): `recordProduction`, penalties, registry mutation.
/// The registry is a fixed set.
#[derive(Clone)]
pub(super) struct FakeStaking {
    chain: FakeChain,
    members: Members,
    /// The branching committee schedule, if the stand set one — see
    /// [`BranchCommittees`]. `None` keeps [`Self::members`] as the whole answer,
    /// which is what every test written before it assumes.
    by_branch: Option<BranchCommittees>,
    /// The one epoch for which this contract answers `weights: None` DESPITE the
    /// ring still holding its frame — "the contract answered something no
    /// committed epoch can answer". A switch and not a schedule: the module
    /// refuses such an epoch permanently, so a second one would only be a second
    /// copy of the same refusal.
    weights_none_for: Option<u64>,
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
            tombstoned: Arc::new(Vec::new()),
            validators: Arc::new(validators),
            registry: Arc::new(registry),
            epoch_len,
            reads: Arc::new(Mutex::new(StakingReads::default())),
        }
    }

    /// The three step-5b knobs, applied after [`Self::new`] rather than passed
    /// through it: the constructor already takes six arguments, and three more
    /// positional ones would be three more things a caller can transpose.
    /// Every one of them defaults to the pre-step behaviour.
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

    /// Which branch of THIS node's execution layer `at` sits on at `height` —
    /// see [`Branch`]. The comparison is against the canonical map, which is the
    /// only tier `provider.block_hash(n)` can see.
    fn branch_of(&self, at: B256, height: u64) -> Branch {
        match self.chain.spec_hash_at(height) {
            Some(canonical) if canonical == at => Branch::Canonical,
            _ => Branch::Speculative,
        }
    }

    /// The committee the contract would hold for `epoch` IN THE STATE AT `at`,
    /// peer-key ASCENDING as `commitEpochCommittee` sorts it
    /// (`contracts/staking/src/consensus.rs:564`).
    ///
    /// `tombstoned` is applied LAST and from the read HEIGHT, not from the
    /// epoch: the contract reads that flag live at the call's own block while
    /// the membership beside it is frozen, so a member tombstoned at height `h`
    /// is flagged in every read at or above `h` of every epoch it sits in.
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

    /// The frozen leader weights the contract answers for `epoch` at `height`,
    /// or `None` for its "the ring has wrapped past this epoch" answer — see the
    /// ring paragraph on [`FakeStaking`]. An EMPTY committee takes neither arm:
    /// the reader's equal-length branch answers `Some(vec![])` there
    /// (`reader.rs:667-680`), which is what an uncommitted epoch looks like.
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

    /// `getDkgQual(epoch)` paired with "is `epoch`'s committee committed at
    /// `at`" — the two legs `beacon::CommitteeReads::dkg_qual` answers with, and
    /// the beacon's own frozen reader is built over them (`carry.rs:185`). An
    /// uncommitted epoch reads its bit as the contract
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

/// The two staticcalls the committee module makes, over the same fake.
///
/// A separate impl from [`StakingStateRead`] because the module's port is
/// deliberately narrower — the two reads it issues and nothing else — and
/// because `dkg_qual` is not on `StakingStateRead` at all. The inherent
/// `FakeStaking::dkg_qual` answers the `(bit, committed)` pair the beacon's
/// trait wants; the module derives "committed" from having a record at all, so
/// only the bit crosses.
impl crate::committee::EpochReads for FakeStaking {
    fn epoch_committee_snapshot(
        &self,
        epoch: u64,
        at: B256,
    ) -> Result<ValidatorSetSnapshot, ReadError> {
        // Counted HERE and not inside the shared body: this port is the
        // module's alone, so `StakingReads::module_snapshot` says how many
        // snapshot calls the MODULE made, which is the countable form of "one
        // snapshot per epoch, whatever asked".
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
        let branch = self.branch_of(at, height);
        let validators = self
            .committed_at(epoch, height)
            .then(|| self.committee(epoch, at, height, branch))
            .flatten();
        // An uncommitted / missed-commit epoch is `Ok` with `validators: []`
        // and `weights: Some(vec![])` — the empty `stakes` leg beside an empty
        // `addrs` takes the equal-length arm (`reader.rs:667-680`), NOT the
        // `weights: None` "ring has wrapped" arm.
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

/// Every `committee[E]` read the jump made, in call order: `(epoch, executed
/// hash it was read AT)`. The observation behind "the committee was read at the
/// LANDING hash" — recorded by [`JumpCommittees`] itself, so a test asserts what
/// production asked for instead of inferring it from the outcome.
pub(super) type JumpCommitteeReads = Arc<Mutex<Vec<(u64, B256)>>>;

/// One `ReJump::call` — one run of the production
/// [`jump_to_target`](crate::cold_start_jump::jump_to_target).
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
    /// `(height, result)` of the TARGET certificate this call consumed — since
    /// §5.2 the `(finalization, block)` pair the EXECUTOR read out of this node's
    /// own marshal archive at the tip it triggered on and handed to `ReJumpFn`,
    /// recorded in the stand's callback before the jump consumes it. So the
    /// landing can be checked against the cert it came from rather than against
    /// the chain the landing just wrote. (It used to be teed off the jump's own
    /// `get_latest`, which is the call §5.2 removed.)
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

/// Every ladder step a node's frontier probe NAMED, as `(T, last(T+1))` in call
/// order — the stand's window into §5.2's "ступень". Recorded where the step is
/// NAMED (the probe closure), so a test reads what was asked for rather than
/// inferring it from what arrived.
///
/// No tip beside it: the node's own marshal tip is what the executor judges the
/// step against, and reading it from this closure is an extra marshal message that
/// changes the run (review A2-02, and the stand-side note in `stand.rs`). The
/// step-vs-tip comparison is pinned in `executor::tests` instead.
pub(super) type FrontierSteps = Arc<Mutex<Vec<(u64, u64)>>>;

/// The jump's committee read over [`FakeStaking`], BY EXECUTED HASH — the stand's
/// [`RethCommitteeSource`](crate::cert_inlet::RethCommitteeSource). Built exactly
/// as the node's steady-state re-jump builds it (`consensus/src/dpos.rs`): a
/// state reader plus the chain namespace. `verify_jump_authenticated` calls
/// `scheme_at(epoch, landing_hash, None)`, so the committee comes out of the
/// CONTRACT STATE MACHINE at the landing — never out of the stand's schedule.
///
/// The finalized-tip hash closure it used to carry is gone with
/// `CommitteeSource::scheme_at_finalized_tip`: every committee read that is NOT
/// at an arbitrary jump hash goes through the committee module now.
pub(super) struct JumpCommittees {
    staking: FakeStaking,
    namespace: Vec<u8>,
    reads: JumpCommitteeReads,
}

impl JumpCommittees {
    pub(super) fn new(staking: FakeStaking, namespace: Vec<u8>, reads: JumpCommitteeReads) -> Self {
        Self {
            staking,
            namespace,
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
    /// (`l1_checkpoint` is `None`, so `jump_to_target` skips the L1 re-assert),
    /// but the tier has to be right or the first fixture that wires a checkpoint
    /// inherits a fake oracle. The §5.2 LANDING check calls `holds` on this same
    /// seam with the attested `block.result`, and that one IS reached.
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
    /// `deliver` calls that returned `false` — a SIGNAL OF A LIE on any of the
    /// five arms (`plane_upstream.rs`: undecodable bytes, a foreign height under
    /// `Finalized{h}`, a payload that is not the served body's digest, a height
    /// outside the certificate's epoch, a multisig that fails under a READABLE
    /// committee). Each one costs the sender this channel for the life of the
    /// resolver engine.
    ///
    /// Since 4.2-А this is the load-bearing observable of the three role tests
    /// (R-001/R-004/R-009): the victim REFUSED the forgery. It is a count only —
    /// the `reason` split lives in `metrics::counter!`, which goes to the
    /// process-global recorder the stand does not read (journal §8.4), so no
    /// stand assert can say WHICH arm fired.
    pub deliveries_rejected: Arc<AtomicU64>,
    /// Every BY-HEIGHT pull this node's upstream client made, in order.
    ///
    /// `finalized_calls`/`finalized_delivered` count the same events but cannot
    /// say WHICH heights, which is what a ladder assertion needs (review B1-03):
    /// "the rung `last(T+1)` was asked for by height and served" is a different
    /// statement from "74 by-height pulls happened", and only the first one
    /// separates a served rung from the ordinary contiguous repair traffic
    /// running beside it.
    ///
    /// The ladder step and the marshal's ordinary gap repair are the SAME verb on
    /// this seam (`get_finalization`), so a reader cannot tell them apart by the
    /// call — only by the height, which is what the ladder test matches on.
    pub served_heights: Arc<Mutex<Vec<Pull>>>,
    /// `ReJump::call` invocations — each one runs the PRODUCTION
    /// [`crate::cold_start_jump::jump_to_target`] over
    /// [`JumpCommittees`] + [`JumpElSync`]. Stays 0 while
    /// `StandConfig::re_jump_threshold` is `None` (the gate is then `u64::MAX`,
    /// so `Executor::maybe_re_jump` never arms the waiter). A COUNT ONLY: what
    /// each call did is [`JumpCall`], and asserting on this number alone cannot
    /// tell a landing from a failed authentication.
    pub rejump_calls: Arc<AtomicU64>,
}

/// One by-height pull the upstream client made: the height asked for and whether
/// the answer came back. `finalized_calls`/`finalized_delivered` count the same
/// events without the height, which is the one thing a ladder assertion needs.
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
    /// Signer schemes this node's `Beacon` wrapper rebuilt over the
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
    /// The PRODUCTION handler, over the stand's one runtime context — nothing
    /// about `deliver`'s five checks is re-implemented here.
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
/// `(mux sub-channel, sender)` — what the buffered body engine's per-sender
/// deque would have to hold to keep every body of one sub-channel resident. The
/// DKG agreement instances take the sub-channels at and above
/// `DKG_SUBCHANNEL_BASE`; the per-epoch `OrderBlock` bodies ride the epoch's own
/// number. Payloads are keyed by their keccak, so a re-broadcast of the same
/// body (a `Plan::Forward`) counts once, as it does in the engine's deque.
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
/// [`BodyTap`], records the frame's sub-channel and payload first. Every one of
/// a node's five plane channels is wrapped in it (the mux brokers share one
/// receiver type), and only the broadcast channel's carries a tap.
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
