//! Fluent Application: bridges commonware consensus ⇄ the deferred-execution
//! pipeline.
//!
//! `propose` assembles an ordering artifact — no EL work on the critical
//! path; `verify` is a pure function of agreed state + the local derived
//! chain (bounded wait on the execution gate); `report` feeds finalized
//! artifacts to [`crate::executor`] for derive + import.
//!
//! Trait implementations:
//!   - [`Application<E>`]: high-level, with `AncestorStream` ancestry.
//!   - [`VerifyingApplication<E>`]: same shape, returns `bool`.
//!   - [`Reporter<Activity = Update<OrderBlock>>`]: fed by `marshal::core::Actor`.
//!
//! NOT implemented: `Relay`. The `marshal::standard::Inline` wrapper
//! provides `Relay` (inline.rs:471); `FluentApp` does not.

use crate::{
    beacon::{
        actor::{
            CommitteeFor, DkgLogIndex, DETERMINISTIC_BOOTSTRAP_EPOCH, DKG_MARGIN_BLOCKS,
            DKG_SETTLE_BLOCKS,
        },
        ceremony::CeremonyOutput,
        certify::SeedStore,
        outcome::{encode_outcome, group_public_key, parse_outcome, validate_share_on_poly},
        seed::{verify_seed, GroupPublic, Seed},
    },
    digest::Digest,
    executor, extra_data,
    fault::TransportError,
    order_block::{
        result_matches, result_target, OrderBlock, ResultTarget, MIN_GAS_LIMIT, TX_BYTE_BUDGET,
        TX_BYTE_BUDGET_AT_BOUNDARY,
    },
};
use alloy_consensus::Transaction as _;
use alloy_primitives::{Address, Bytes, B256};
use alloy_rpc_types_engine::{ForkchoiceState, ForkchoiceUpdated, PayloadStatus};
use commonware_consensus::{
    marshal::{
        ancestry::{AncestorStream, BlockProvider},
        Update,
    },
    simplex::types::Context as SimplexContext,
    types::{Epoch, Round, View},
    Application, Reporter, VerifyingApplication,
};
use commonware_cryptography::{bls12381::primitives::group::Share, ed25519::PublicKey};
use commonware_runtime::{Clock, Metrics, Spawner};
use commonware_utils::ordered::BiMap;
#[cfg(test)]
use commonware_utils::ordered::Set;
/// The signing scheme bound for this Application.
pub use fluentbase_bls::Scheme as BlsScheme;
use fluentbase_bls::{BlsPubkey, PeerPubkey};
use futures::StreamExt as _;
use rand_08::Rng;
use reth_ethereum_primitives::{Block as RethBlock, TransactionSigned};
use reth_primitives_traits::SealedBlock;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};

/// Bounded wait in `verify` for local execution to reach `N − K`: the
/// exec-gate budget = worst-case derive+execute of one block (~500ms today,
/// growth headroom to 1s). Sits inside the certification window: the
/// proposal arrives ≤ `leader` (1750ms) from view entry and
/// `certification = 3200ms` (`ConsensusTimeouts::fluent_1s`) leaves
/// ~1450ms ≥ this budget. Liveness-tuning, not a safety param
/// (timeout ⇒ vote false) — still keep uniform across nodes.
pub const VERIFY_EXEC_BUDGET: Duration = Duration::from_millis(1000);
const VERIFY_EXEC_POLL: Duration = Duration::from_millis(25);

/// Target ordering cadence: one block per second. The proposer holds its
/// proposal until wall clock reaches `parent.timestamp + BLOCK_INTERVAL`,
/// so timestamps advance as consecutive integer seconds ≈ wall clock
/// (Clique-family parent+period pacing). Slow/nullified views self-correct:
/// a late proposer is already past the target and does not sleep.
/// Honest-proposer discipline only — the verify-side future bound
/// ([`TIMESTAMP_FUTURE_TOLERANCE_SECS`]) is the enforcement half.
pub const BLOCK_INTERVAL: Duration = Duration::from_secs(1);

/// Verify-side future bound: reject `block.timestamp > now + tolerance`.
/// 1s covers second-granularity truncation + honest NTP skew. Load-bearing
/// with pacing: without it, ONE far-future timestamp both poisons
/// block.timestamp permanently (strict-monotonicity ratchet) and makes
/// every honest proposer sleep_until(fake_time) — a single-block chain
/// halt. With it, such a proposal fails verify at the honest quorum and
/// the view nullifies. Deliberately == `BLOCK_INTERVAL` (1s): do NOT widen it
/// (e.g. to 2s for sloppy NTP) — that would let chain-time run up to ~1 block
/// ahead of real time and weaken the hard "1 block ≈ 1 real second"
/// requirement. Clock sync is an operator duty, not a reason to loosen this
/// (decision 2026-06-24). Consensus rule — MUST be uniform across nodes.
pub const TIMESTAMP_FUTURE_TOLERANCE_SECS: u64 = 1;

/// Read-side view of the local derived chain, shared by propose/verify and
/// the executor. Implemented in the node crate over reth's provider — hash
/// strictly by NUMBER, never `best_number` (its semantics flip between
/// tree-sync and pipeline backfill).
pub trait ExecutedChain: Clone + Send + Sync + 'static {
    /// Highest derived + canonicalized height.
    fn executed_tip(&self) -> u64;

    /// Tier-S (SPECULATIVE): canonical EVM hash of the derived block at `height`
    /// on the provider HEAD chain — advanced at notarization latency by
    /// `spec_execute`, NOT yet beyond reorg. The head can carry a sibling the
    /// finalization will replace, so this tier is read ONLY by the executor's
    /// own parent-linkage / backward cross-checks — NEVER by the result gate
    /// (that would re-open bundle-20260716T150148Z).
    fn spec_executed_hash(&self, height: u64) -> Option<B256>;

    /// Tier-F (FINALIZED): the finalized-execution result at `height`, or `None`
    /// if this node has not finalized-derived `height` yet. The result gate
    /// (propose + verify) samples THIS tier so a still-speculative sibling A at
    /// h−K can never be committed as an OrderBlock `result` and then re-finalize
    /// as sibling B — the honest-run whole-committee SafetyHalt of
    /// bundle-20260716T150148Z.
    ///
    /// Tier-F IS reth's canonical chain below the monotone finalized-execution
    /// cursor ([`FinalizedCursor`]): past the `try_derive` canonical
    /// postcondition the canonical hash at every advanced height is the
    /// finalized result, durably persisted by reth (content-immutable on
    /// finalization; the executor is reth's sole writer and never rolls the head
    /// below safe), so there is no separate hash store to keep. NO default — a
    /// speculative read wearing a finalized name is the exact bug class the tier
    /// split closes, so every consumer chooses its tier explicitly (an unwired
    /// path is a compile error, not a silent downgrade).
    fn finalized_executed_hash(&self, height: u64) -> Option<B256>;

    /// Advance the monotone finalized-execution cursor to `height` — executor
    /// only, called past the `try_derive` canonical postcondition (the eager
    /// finalized derive) and at the BLS-authenticated re-jump landing. Every
    /// height ≤ the cursor is finalized-without-a-possible-sibling, so
    /// `finalized_executed_hash` there resolves through reth's canonical chain.
    /// Seeded at `Actor::init` from the marshal's durable acked cursor (each
    /// acked height passed the canonical postcondition before its ack persisted,
    /// so a fresh process's provider content at ≤ cursor is beyond reorg —
    /// WITHOUT the seed a coordinated restart of ≥ f+1 propose-skips/false-votes
    /// for K blocks and wedges). Monotone (`fetch_max`); a lower value is a
    /// no-op. Default no-op (readers / non-writers). Replaces the former
    /// `record_finalized_executed` + `raise_finalized_floor` — reth is the store,
    /// so only the cursor moves.
    fn advance_finalized(&self, _height: u64) {}
}

/// The monotone finalized-execution cursor: one `AtomicU64` naming the highest
/// height known finalized-without-a-possible-sibling. Tier-F
/// ([`ExecutedChain::finalized_executed_hash`]) IS reth's canonical chain BELOW
/// this cursor — the cursor stores NO hashes because reth already persists them
/// durably (content-immutable on finalization; the executor is reth's sole
/// writer), collapsing the former bounded map + retain-window + finality-floor
/// into a single generalized cursor. Advanced by the executor past the
/// `try_derive` canonical postcondition (the eager finalized derive) and at a
/// BLS-authenticated re-jump landing; seeded at `Actor::init` from the marshal's
/// durable acked cursor. Shared (the executor advances, the gate reads a clone),
/// so it survives per-epoch engine restarts within the process.
#[derive(Clone, Debug, Default)]
pub struct FinalizedCursor {
    cursor: Arc<AtomicU64>,
}

impl FinalizedCursor {
    /// Tier-F lookup: the provider's canonical hash at `height` IS the finalized
    /// hash iff `height ≤ cursor` (no sibling can exist there); above the cursor
    /// the height is not-yet-finalized-reconciled ⇒ `None`. A provider miss
    /// at-or-below the cursor (deep-pruned history, or a crash lost the reth tail
    /// above the durable ack) also returns `None` — never a wrong hash.
    pub fn resolve(&self, height: u64, canonical: impl Fn(u64) -> Option<B256>) -> Option<B256> {
        (height <= self.cursor.load(Ordering::Acquire))
            .then(|| canonical(height))
            .flatten()
    }

    /// Advance the cursor (monotone; a lower value is a no-op).
    pub fn advance(&self, height: u64) {
        self.cursor.fetch_max(height, Ordering::Release);
    }
}

/// Ordering-assembly: pick txs for height N against executed state plus the
/// in-flight ordered-but-unexecuted suffix overlay. No execution.
pub trait OrderingAssembler: Send + Sync + 'static {
    fn assemble(&self, height: u64, gas_limit: u64, byte_budget: usize) -> Vec<TransactionSigned>;

    /// Every ordering-finalized artifact, in order — keeps the in-flight
    /// suffix (nonces/hashes of ordered-but-unexecuted txs) authoritative so
    /// `assemble` does not re-propose what the pool still thinks is pending
    /// (the pool tracks the EXECUTED head, which lags ordering by ≤ K).
    fn observe_finalized(&self, block: &OrderBlock);
}

/// EIP-1559 header rule: `|limit − parent| < parent/1024` and
/// `limit ≥ MIN_GAS_LIMIT`. The gas limit is agreed data (an [`OrderBlock`]
/// field), so verify bounds it against the parent exactly like Ethereum
/// header validation does.
pub fn gas_limit_within_1_1024(parent: u64, limit: u64) -> bool {
    limit >= MIN_GAS_LIMIT && limit.abs_diff(parent) < (parent / 1024).max(1)
}

/// Why [`FluentApp::expected_leader_index`] refused. Both variants are a VOTE
/// REJECT, never a skip — they are named separately only so the log line says
/// which one, because they mean very different things: the first is a leader
/// outside the committee this node agreed on, the second is a committee that
/// outgrew the 1-byte wire format (which `OuterBuilder::build`'s startup assert
/// exists to make unreachable).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LeaderIndexError {
    #[error("round leader is not a member of this epoch's committee")]
    LeaderNotInCommittee,
    #[error("committee index {index} exceeds the 1-byte record (committee size {committee_size})")]
    IndexExceedsWireFormat { index: usize, committee_size: usize },
}

/// Vote-time rule for the production record in `extra_data`: the block must
/// carry EXACTLY the 2-byte record naming its own leader.
///
/// `expected_leader_index` is `None` only for an instance with no committee map
/// (a follower, a verify-only scheme, a test) — it casts no vote, so the rule is
/// skipped rather than failed. `Some(i)` demands all three of: exact length,
/// known version, and `carried == i`.
///
/// Empty `extra_data` REJECTS under `Some`. That asymmetry is load-bearing: the
/// EXECUTOR must tolerate an empty field (its gate keys on height, and a
/// migration-window block legitimately carries none), while VERIFY must not —
/// and it is verify's refusal that makes the executor's empty arm unreachable
/// through consensus. `Ok(None)` from the decoder is the empty case, hence the
/// match on `Ok(Some(_))` rather than on `is_ok()`.
///
/// Exact length is also what keeps a 4 KiB-tolerant OrderBlock codec from
/// finalizing a block whose `extra_data` no reth header (32-byte cap) can hold.
fn production_record_ok(extra_data: &[u8], expected_leader_index: Option<u8>) -> bool {
    let Some(expected) = expected_leader_index else {
        return true;
    };
    matches!(
        extra_data::decode_production_record(extra_data),
        Ok(Some(record)) if record.leader_index == expected
    )
}

/// Proposer-side step of the agreed gas limit toward the local target,
/// clamped to the bound [`gas_limit_within_1_1024`] enforces.
pub fn step_gas_limit(parent: u64, target: u64) -> u64 {
    let max_delta = (parent / 1024).saturating_sub(1);
    let stepped = if target > parent {
        parent + max_delta.min(target - parent)
    } else {
        parent - max_delta.min(parent - target)
    };
    stepped.max(MIN_GAS_LIMIT)
}

/// Resolves this node's memoized DKG result for an epoch it is a MEMBER of:
/// the agreed `Output` (`PK_E`) + this node's share. `None` ⇒ observer / share
/// not produced (⇒ withhold the qualifying beacon vote). Provided by the launch
/// site over the live-DKG `CeremonyStore`.
pub type BeaconForEpoch = Arc<dyn Fn(u64) -> Option<(CeremonyOutput, Share)> + Send + Sync>;

/// Outcome of a group-key resolution (`PK_epoch`), 3-state. Conflating the
/// last two states IS the P1 bug: `Unknown` = "this node structurally does not
/// hold `PK_epoch`" (a stable fact about this node — re-polling cannot help);
/// `ReadFailed` = "the committee read was transiently unavailable, I could not
/// even decide" (retried, NEVER cached). The 2-state `BeaconResolver` fold
/// (`_ => None`, `dpos.rs`) is fine for its retryable share-gate consumer but
/// unusable on a vote path — do not collapse this enum into it.
// A `GroupPublic` (G2) is ~288 B; the enum is a transient return value that is
// matched immediately and never stored, so the stack copy is cheaper than the
// per-resolve heap allocation boxing would put on the vote path.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyLookup {
    Resolved(GroupPublic),
    Unknown,
    ReadFailed,
}

/// Provenance tier of a [`GroupKeys`] entry. Ordered: attested outranks
/// local — on a CONFLICTING insert an observed value DISPLACES a local one,
/// never vice-versa (see [`insert_group_key`]). The prior untiered
/// first-write-wins policy let a diverged local W1 write beat the network's
/// W4 observed-outcome write by 1.3 s of timing — trust inverted (soak
/// 2026-07-14, v5@epoch77).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum KeySource {
    /// This node's OWN DKG material (writers W1/W3 and the ladder-dkg
    /// memoize) — locally reconstructed, can diverge from the chain.
    LocalDkg,
    /// Agreed chain data: a finalized change-epoch boundary block's
    /// `beacon_outcome` (writer W4) — validated by quorum at vote time.
    ObservedOutcome,
}

/// The CROSS-EPOCH shared per-epoch group-key map (`epoch → (PK_epoch,
/// provenance)`). Created ONCE next to the `SeedStore` singleton (`outer.rs`)
/// and cloned by `Arc` into every [`FluentApp`] clone (hence every per-epoch
/// engine) and into `epoch_manager::Config` — it must OUTLIVE every engine
/// (engines are aborted at the transition) and be the SAME map for `E` and
/// `E+1` (writer W2 = W1 of the previous epoch). Insert-only WITHIN a
/// provenance tier: an epoch's group key is immutable agreed data, so a
/// same-tier differing re-insert is a divergence witness and the first write
/// wins; ACROSS tiers the attested source wins (see [`insert_group_key`]).
/// NEVER build a second map: the writers fill one map and the vote path reads
/// it — two maps silently re-open the boundary forge arm.
pub type GroupKeys = Arc<RwLock<BTreeMap<u64, (GroupPublic, KeySource)>>>;

/// The lazy 3-state group-key resolver (ladder step 1: the node's OWN live-DKG
/// material, carry-forward + committee-equality gated). Built at the launch
/// site (`dpos.rs`) — the only place the `CeremonyStore` exists — and threaded
/// into [`BeaconVerify`]. Reads `.public()` ONLY, never the share.
pub type GroupKeyFor = Arc<dyn Fn(u64) -> KeyLookup + Send + Sync>;

/// The parent-link classification shared by propose and the verify gate: the
/// PARENT's epoch `Ep` and whether the parent-seed witness is REQUIRED — a
/// pure function of the simplex context (agreed data, no local state).
/// `ctx.parent.0 == View::zero()` is commonware's GENESIS_VIEW sentinel: the
/// parent is the previous epoch's boundary block (F3), so `Ep = Ec − 1`; on
/// every other link `Ep = Ec` (F2). `Ec == 0` at the boundary form means the
/// parent is the chain ANCHOR (never proposed, no round) ⇒ `None` ⇒ never
/// beacon-active — this is what keeps the first post-activation block
/// producible. The witness is required iff
/// `Ep >= DETERMINISTIC_BOOTSTRAP_EPOCH` — deterministic, never "do I hold a
/// key" (F8; a two-branch key-dependent gate would re-open the downgrade arm).
fn witness_link(ctx: &SimplexContext<Digest, PublicKey>) -> (Option<u64>, bool) {
    let ec = ctx.round.epoch().get();
    let ep = if ctx.parent.0 == View::zero() {
        ec.checked_sub(1)
    } else {
        Some(ec)
    };
    let required = ep.is_some_and(|e| e >= DETERMINISTIC_BOOTSTRAP_EPOCH);
    (ep, required)
}

/// Tiered, idempotent write into the shared [`GroupKeys`] map — the
/// ATTESTED-SOURCE-WINS conflict policy. An epoch's group key is agreed chain
/// data; on a DIFFERING re-insert the higher-provenance value holds the entry:
/// a [`KeySource::ObservedOutcome`] write (agreed chain data) DISPLACES a
/// differing [`KeySource::LocalDkg`] one, never vice-versa, and WITHIN a tier
/// the first write wins. Same-value re-inserts keep the strongest provenance
/// (an observed confirm upgrades a local entry to attested — the promote
/// value-gate's input, [`attested_group_key`]). Failures are never inserted
/// (the callers only reach here with a resolved key).
pub(crate) fn insert_group_key(map: &GroupKeys, epoch: u64, pk: GroupPublic, source: KeySource) {
    let Ok(mut m) = map.write() else {
        tracing::warn!(epoch, "group-key map poisoned; dropping resolved key");
        return;
    };
    let Some(&(existing, existing_src)) = m.get(&epoch) else {
        m.insert(epoch, (pk, source));
        return;
    };
    if existing == pk {
        if source > existing_src {
            m.insert(epoch, (pk, source));
        }
        return;
    }
    // A DIFFERING value for one epoch is the network-wide key-divergence
    // witness (soak 2026-07-14: v5's own W1 value vs the network's ⇒ a lone
    // reject{bad_signature}) — keep it LOUD + counted whichever side wins.
    let winner = match source.cmp(&existing_src) {
        std::cmp::Ordering::Greater => "observed_displaces_local",
        std::cmp::Ordering::Less => "attested_kept",
        std::cmp::Ordering::Equal => "first_write_kept",
    };
    tracing::warn!(
        epoch,
        existing = %pk_prefix(&existing),
        existing_source = ?existing_src,
        offered = %pk_prefix(&pk),
        offered_source = ?source,
        winner,
        "group-key re-insert with a DIFFERING value"
    );
    metrics::counter!("dpos_group_key_conflict_total", "winner" => winner).increment(1);
    if source > existing_src {
        m.insert(epoch, (pk, source));
    }
    // Two DIFFERING observed-outcome values would mean two finalized boundary
    // blocks disagree on one epoch's mint — fork-grade, never a handled state.
    debug_assert!(
        !(source == KeySource::ObservedOutcome && existing_src == KeySource::ObservedOutcome),
        "two observed agreed group keys differ for epoch {epoch}"
    );
}

/// The NETWORK-ATTESTED `PK_epoch` for `epoch`, if one is known: a
/// [`KeySource::ObservedOutcome`] map entry only. Local-sourced entries are
/// deliberately invisible here — the promote value-gate must never compare a
/// local resolve against another local resolve.
pub(crate) fn attested_group_key(map: &GroupKeys, epoch: u64) -> Option<GroupPublic> {
    map.read().ok().and_then(|m| {
        m.get(&epoch)
            .and_then(|&(pk, src)| (src == KeySource::ObservedOutcome).then_some(pk))
    })
}

/// First 8 serialized bytes of a group public key, hex — a stable, greppable
/// value fingerprint. Enough to byte-diff key VALUES across nodes from logs
/// alone (the reject-triage need); the full G2 hex is 192 chars of log noise.
pub(crate) fn pk_prefix(pk: &GroupPublic) -> String {
    let mut s = pk.to_string();
    s.truncate(16);
    s
}

/// The per-epoch beacon-DKG context threaded into [`FluentApp`]'s verify/propose
/// path: the boundary "C" share-on-polynomial qualification + the proposer's
/// `beacon_outcome` assertion. `None` on `FluentApp` ⇒ no beacon context
/// (cold-start epoch 0 / followers / tests) ⇒ the beacon gate is a no-op.
#[derive(Clone)]
pub struct BeaconVerify {
    beacon_for_epoch: BeaconForEpoch,
    committee_for: CommitteeFor,
    /// Lazy 3-state `PK_epoch` resolver (ladder step 1) — consulted by
    /// [`FluentApp::group_public_for`] on a [`GroupKeys`] map miss; the map
    /// memoizes `Resolved` only. Distinct from `beacon_for_epoch`: that is an
    /// EXACT-epoch lookup (misses on every stable epoch after the last change)
    /// and it exposes the share; this one is carry-forward + committee-gated
    /// and key-only.
    group_key_for: GroupKeyFor,
    /// The chain's beacon seed-signing namespace
    /// (`seed_namespace(fluent_namespace(chain_id))`) — the domain the witness
    /// signature is verified under (`verify_seed`).
    seed_namespace: Vec<u8>,
    dpos_activation: u64,
    epoch_interval: u64,
    /// AMENDMENT 5 determinism core (both `None` ⇒ propose carries no `dkg_logs`,
    /// report accumulates nothing — the finalize stays legacy). `recorded_dkg_logs`
    /// is the DKG actor's published `idx→hash` of logs THIS node holds (read at
    /// propose); `finalized_dkg_logs` is the accumulated finalized set (written at
    /// report, height-gated `≤ H_settle`; read by propose for dedup + by the actor's
    /// deterministic finalize). Shared `Arc`s with the DkgActor.
    recorded_dkg_logs: Option<DkgLogIndex>,
    finalized_dkg_logs: Option<DkgLogIndex>,
    /// DEVNET/TEST-ONLY byzantine behaviour; `None` (and absent without the
    /// feature) on every honest node.
    #[cfg(feature = "dpos-devnet-byzantine")]
    byzantine: Option<crate::byzantine::ByzantineMode>,
}

impl BeaconVerify {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        beacon_for_epoch: BeaconForEpoch,
        committee_for: CommitteeFor,
        group_key_for: GroupKeyFor,
        seed_namespace: Vec<u8>,
        dpos_activation: u64,
        epoch_interval: u64,
    ) -> Self {
        Self {
            beacon_for_epoch,
            committee_for,
            group_key_for,
            seed_namespace,
            dpos_activation,
            epoch_interval,
            recorded_dkg_logs: None,
            finalized_dkg_logs: None,
            #[cfg(feature = "dpos-devnet-byzantine")]
            byzantine: None,
        }
    }

    /// Attach the AMENDMENT 5 shared dealer-log hash indices (the SAME `Arc`s the
    /// DkgActor holds): `recorded_dkg_logs` (propose reads what this node holds) +
    /// `finalized_dkg_logs` (report writes the accumulated finalized set, propose
    /// reads it for dedup). Unset ⇒ propose carries no `dkg_logs`.
    pub fn with_dkg_logs(mut self, recorded: DkgLogIndex, finalized: DkgLogIndex) -> Self {
        self.recorded_dkg_logs = Some(recorded);
        self.finalized_dkg_logs = Some(finalized);
        self
    }

    /// The `H_settle` height for the epoch whose ceremony a block at `height`
    /// carries dealer-log hashes for (`E+1` where `E = epoch_of(height)`):
    /// `epoch_start(E+1) − (DKG_MARGIN_BLOCKS − DKG_SETTLE_BLOCKS)`. The finalized
    /// accumulator is frozen at `≤ H_settle` so the pinned finalize set is a pure
    /// function of the finalized chain prefix (determinism).
    fn h_settle_for_carrier(&self, height: u64) -> u64 {
        let next = self.epoch_of(height) + 1;
        self.epoch_start(next)
            .saturating_sub(DKG_MARGIN_BLOCKS - DKG_SETTLE_BLOCKS)
    }

    /// AMENDMENT 5 propose: the `(idx, hash)` dealer-log entries to ride an
    /// `OrderBlock` at `height` — the E+1 logs this node has RECORDED
    /// (`recorded_dkg_logs`) MINUS those already FINALIZED in ancestors
    /// (`finalized_dkg_logs`, first-finalized-wins dedup). Canonically sorted by
    /// `idx`. Empty when unwired, or once every held log is already finalized.
    fn dkg_logs_to_include(&self, height: u64) -> Vec<(u8, B256)> {
        let (Some(recorded), Some(finalized)) = (
            self.recorded_dkg_logs.as_ref(),
            self.finalized_dkg_logs.as_ref(),
        ) else {
            return Vec::new();
        };
        let next = self.epoch_of(height) + 1;
        let Ok(rec) = recorded.read() else {
            return Vec::new();
        };
        let Some(rec_epoch) = rec.get(&next) else {
            return Vec::new();
        };
        let fin = finalized.read().ok();
        let fin_epoch = fin.as_ref().and_then(|m| m.get(&next));
        let mut out: Vec<(u8, B256)> = rec_epoch
            .iter()
            .filter(|(idx, hash)| {
                // Dedup: skip an idx already finalized in an ancestor with the SAME
                // hash (a differing hash is a NEW body for that idx — still offer it).
                fin_epoch.and_then(|m| m.get(idx)) != Some(*hash)
            })
            .map(|(idx, hash)| (*idx, *hash))
            .collect();
        out.sort_by_key(|(idx, _)| *idx); // BTreeMap iter is already sorted; belt-and-suspenders
        out
    }

    /// AMENDMENT 5 report: accumulate a FINALIZED `OrderBlock`'s `dkg_logs` into the
    /// shared `finalized_dkg_logs`, HEIGHT-GATED at `≤ H_settle` so the pinned
    /// finalize set is frozen (a block > H_settle carrying a late hash cannot enter
    /// the set — determinism). First-write-wins per `idx` (a finalized hash is agreed
    /// consensus data). No-op when unwired / empty / past the gate.
    fn note_finalized_dkg_logs(&self, block: &OrderBlock) {
        if block.dkg_logs.is_empty() {
            return;
        }
        let Some(finalized) = self.finalized_dkg_logs.as_ref() else {
            return;
        };
        if block.height > self.h_settle_for_carrier(block.height) {
            return; // past H_settle — excluded from the frozen pinned set
        }
        let next = self.epoch_of(block.height) + 1;
        if let Ok(mut m) = finalized.write() {
            let entry = m.entry(next).or_default();
            for (idx, hash) in &block.dkg_logs {
                entry.entry(*idx).or_insert(*hash); // first-finalized-wins
            }
        }
    }

    /// DEVNET/TEST-ONLY: attach a byzantine behaviour. No-op when `None`.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub fn with_byzantine(mut self, mode: Option<crate::byzantine::ByzantineMode>) -> Self {
        self.byzantine = mode;
        self
    }

    /// `true` iff this node is flagged to forge the beacon `PK_E` (devnet/test).
    /// Always `false` on a production build (the field does not exist).
    fn forges_beacon_pk(&self) -> bool {
        #[cfg(feature = "dpos-devnet-byzantine")]
        {
            matches!(
                self.byzantine,
                Some(crate::byzantine::ByzantineMode::ForgeBeaconPk)
            )
        }
        #[cfg(not(feature = "dpos-devnet-byzantine"))]
        {
            false
        }
    }

    fn epoch_of(&self, height: u64) -> u64 {
        height.saturating_sub(self.dpos_activation) / self.epoch_interval.max(1)
    }

    fn epoch_start(&self, epoch: u64) -> u64 {
        self.dpos_activation + epoch * self.epoch_interval
    }

    /// A height is a CHANGE-epoch first block iff it is the first block of an
    /// epoch `E ≥ 1` whose committee differs from `E-1`'s, OR the first block of
    /// the deterministic-bootstrap epoch (committee[2] always seeds the beacon
    /// during epoch 1, even on a stable committee — keyed off the same
    /// [`DETERMINISTIC_BOOTSTRAP_EPOCH`] the DKG actor's `maybe_start` uses, so the
    /// two never disagree on which boundaries assert a `beacon_outcome`). Both
    /// committees are read at the current finalized hash (the resolver's contract);
    /// an unresolvable read ⇒ `false` (an honest change block then fails the
    /// epoch-type gate transiently → view-change → retry once the read resolves).
    fn is_change_epoch_first_block(&self, height: u64, epoch: u64) -> bool {
        if epoch == 0 || height != self.epoch_start(epoch) {
            return false;
        }
        if epoch == DETERMINISTIC_BOOTSTRAP_EPOCH {
            return true;
        }
        let cur = (self.committee_for)(epoch);
        let prev = (self.committee_for)(epoch - 1);
        let change = matches!((&cur, &prev), (Some(c), Some(p)) if c != p);
        // Diagnostic (fires only for a first-block-of-epoch — once per boundary per
        // propose/verify): shows whether committee[E]/[E-1] are readable at the
        // finalized hash and the change decision. Pinpoints a boundary block being
        // treated as a normal block because the committee wasn't yet visible.
        tracing::info!(
            height,
            epoch,
            cur_readable = cur.is_some(),
            prev_readable = prev.is_some(),
            change,
            "beacon: is_change_epoch_first_block (boundary eval)"
        );
        change
    }
}

/// The Fluent consensus application.
///
/// Generic over `XC` (local derived-chain view) and `A` (tx assembler).
pub struct FluentApp<XC, A> {
    /// Per-epoch beacon-DKG verify/propose context (see [`BeaconVerify`]).
    /// `None` ⇒ no beacon gating (cold-start epoch 0 / followers / tests).
    beacon: Option<BeaconVerify>,
    /// Shared `round → recovered seed` map (cross-epoch singleton from
    /// `outer.rs`). Read by the propose-side `parent_seed` embed (the witness
    /// rollout); `None` for tests / followers that run no consensus plane.
    seed_store: Option<SeedStore>,
    /// The cross-epoch shared `epoch → PK_epoch` map (see [`GroupKeys`]).
    /// Written OFF the vote path (W1 at engine spawn in `epoch_manager`, W4 in
    /// [`Reporter::report`] below); read + lazily filled (memoize-on-success
    /// only) by [`Self::group_public_for`].
    group_keys: GroupKeys,
    genesis: Arc<OrderBlock>,
    executor: executor::Mailbox,
    /// Observer for `Update::Block` finalizations — NOT a state-advancing
    /// path. Wired to the staking reader's epoch-boundary detection.
    boundary_hook: Arc<dyn Fn(OrderBlock) + Send + Sync>,
    /// Rate-limiter cursor for the result-gate slow-wait INFO line: the last
    /// height for which "verify result-gate waiting" was emitted, so a height
    /// re-verified across views logs at most once. Observability-only; created
    /// internally (not a `new` argument) and shared across clones like a node's
    /// other per-instance atomics.
    verify_gate_last_logged_height: Arc<AtomicU64>,
    executed: XC,
    assembler: Arc<A>,
    /// Proposer-local fields — they shape only this node's OWN proposals
    /// (agreed data once embedded); verify never reads them.
    fee_recipient: Address,
    target_gas_limit: u64,
    /// Chain-wide sequencer→DPoS activation block — origin of the `result_target`
    /// pre-activation window (`height < activation + K` ⇒ `result` is ZERO). A
    /// CHAIN constant, NOT this node's cold-start anchor (`genesis.height`): a
    /// deep-catch-up node seeds its ordering-chain genesis at the live frontier
    /// yet still proposes/verifies the K-below-anchor blocks, which are
    /// post-activation and carry real (non-zero) results. Mirrors the executor's
    /// `dpos_activation_block` so both the BFT and finalized cross-checks key the
    /// window identically.
    dpos_activation_block: u64,
    /// The epoch committee's pubkey→index BiMap, injected by
    /// [`Self::with_committee_index`] from `EpochEngine::new` — the SAME map the
    /// engine builds its scheme from, so the index this app computes and the
    /// committee the engine votes with are one agreed snapshot by construction.
    ///
    /// `None` means this instance holds no committee: a verify-only scheme, a
    /// follower, or a test. Such an instance casts no vote, so the leader index
    /// it cannot compute is not a vote condition — which is the only reason a
    /// `None` here is allowed to be permissive rather than a reject. When the map
    /// IS present and the lookup misses, that is a REJECT; see
    /// [`Self::expected_leader_index`], which keeps the two cases apart on
    /// purpose.
    ///
    /// Deliberately NOT an epoch-keyed registry shared across epochs: a registry
    /// miss would make a vote depend on node-local lookup state at zero quorum
    /// slack, the class this codebase prohibits by name (see the byte-compare
    /// prohibition in `verify_block`).
    committee_index: Option<Arc<BiMap<PeerPubkey, BlsPubkey>>>,
}

impl<XC: Clone, A> Clone for FluentApp<XC, A> {
    fn clone(&self) -> Self {
        Self {
            beacon: self.beacon.clone(),
            committee_index: self.committee_index.clone(),
            seed_store: self.seed_store.clone(),
            group_keys: self.group_keys.clone(),
            genesis: self.genesis.clone(),
            executor: self.executor.clone(),
            boundary_hook: self.boundary_hook.clone(),
            verify_gate_last_logged_height: self.verify_gate_last_logged_height.clone(),
            executed: self.executed.clone(),
            assembler: self.assembler.clone(),
            fee_recipient: self.fee_recipient,
            target_gas_limit: self.target_gas_limit,
            dpos_activation_block: self.dpos_activation_block,
        }
    }
}

impl<XC, A> FluentApp<XC, A>
where
    XC: ExecutedChain,
    A: OrderingAssembler,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        genesis: OrderBlock,
        executor: executor::Mailbox,
        boundary_hook: Arc<dyn Fn(OrderBlock) + Send + Sync>,
        executed: XC,
        assembler: Arc<A>,
        fee_recipient: Address,
        target_gas_limit: u64,
        dpos_activation_block: u64,
        seed_store: Option<SeedStore>,
        // A constructor ARGUMENT, deliberately not an internal default: the
        // same `Arc` must also reach `epoch_manager::Config` (writer W1) — a
        // silently-forgotten second map would split writers from readers.
        group_keys: GroupKeys,
    ) -> Self {
        Self {
            beacon: None,
            committee_index: None,
            seed_store,
            group_keys,
            genesis: Arc::new(genesis),
            executor,
            boundary_hook,
            // u64::MAX sentinel: no height has logged yet (0 is a valid height).
            verify_gate_last_logged_height: Arc::new(AtomicU64::new(u64::MAX)),
            executed,
            assembler,
            fee_recipient,
            target_gas_limit,
            dpos_activation_block,
        }
    }

    /// Attach the per-epoch beacon-DKG verify/propose context (the boundary "C"
    /// gate + the proposer's `beacon_outcome` assertion). Validators supply this;
    /// cold-start / followers / tests leave it `None`.
    pub fn with_beacon(mut self, beacon: BeaconVerify) -> Self {
        self.beacon = Some(beacon);
        self
    }

    /// Attach the epoch committee's pubkey→index map. Called from
    /// `EpochEngine::new`, which holds both the map and this app before moving
    /// the app into `Inline` — that adjacency is what makes the two the same
    /// agreed snapshot. Followers, verify-only schemes and tests leave it unset.
    pub fn with_committee_index(mut self, bimap: Arc<BiMap<PeerPubkey, BlsPubkey>>) -> Self {
        self.committee_index = Some(bimap);
        self
    }

    /// The committee index of `leader`, for the production record carried in
    /// `extra_data`.
    ///
    /// THREE states, and collapsing any two of them is the bug this signature
    /// exists to prevent:
    /// - `Ok(None)` — no committee map. Not a voter; the caller skips the index
    ///   rule and lets the other structural rules decide.
    /// - `Ok(Some(i))` — the index to compare the carried byte against.
    /// - `Err(())` — the map is present and the leader is NOT in it. This is a
    ///   REJECT, never a skip. Writing this as
    ///   `self.committee_index.as_ref().and_then(|m| m.position(k))` would fold
    ///   it into the permissive `None` arm and silently invert the rule.
    ///
    /// An index above `u8::MAX` is also `Err`: the wire format is one byte, and
    /// `OuterBuilder::build`'s startup assert exists to make that unreachable —
    /// if it is ever reached anyway, refusing to vote is the safe direction.
    pub fn expected_leader_index(
        &self,
        leader: &PeerPubkey,
    ) -> Result<Option<u8>, LeaderIndexError> {
        let Some(bimap) = self.committee_index.as_ref() else {
            return Ok(None);
        };
        let idx = bimap
            .position(leader)
            .ok_or(LeaderIndexError::LeaderNotInCommittee)?;
        u8::try_from(idx)
            .map(Some)
            .map_err(|_| LeaderIndexError::IndexExceedsWireFormat {
                index: idx,
                committee_size: bimap.len(),
            })
    }

    /// Resolve `PK_epoch` for the witness-signature arm: the shared
    /// [`GroupKeys`] map first (the common case — no I/O; every voting node
    /// finds its OWN epoch there via W1), then the lazy 3-state resolver
    /// (ladder step 1), memoizing ONLY on `Resolved`. `Unknown` and
    /// `ReadFailed` are NEVER cached — a later call re-runs the resolve and
    /// can succeed (the DKG actor writes the `CeremonyStore` asynchronously,
    /// and W4 can land the key from an observed outcome block at any time).
    /// Synchronous by construction: both inputs are in-memory (the map and the
    /// resolver's `CeremonyStore`/committee snapshot) — no await, no in-flight
    /// state on the vote path.
    pub fn group_public_for(&self, epoch: u64) -> KeyLookup {
        if let Some(pk) = self
            .group_keys
            .read()
            .ok()
            .and_then(|m| m.get(&epoch).map(|&(pk, _)| pk))
        {
            metrics::counter!("dpos_group_public_source_total", "ladder" => "map").increment(1);
            return KeyLookup::Resolved(pk);
        }
        let Some(bv) = self.beacon.as_ref() else {
            // No beacon context (follower / test) ⇒ this node structurally
            // holds no DKG material — a stable fact, not a transient.
            return KeyLookup::Unknown;
        };
        match (bv.group_key_for)(epoch) {
            KeyLookup::Resolved(pk) => {
                metrics::counter!("dpos_group_public_source_total", "ladder" => "dkg").increment(1);
                tracing::debug!(
                    epoch,
                    group_public = %pk_prefix(&pk),
                    "group key resolved from own DKG material (ladder=dkg); memoizing"
                );
                insert_group_key(&self.group_keys, epoch, pk, KeySource::LocalDkg);
                KeyLookup::Resolved(pk)
            }
            miss => miss,
        }
    }

    /// Pure structural validity of `block` against its parent — everything
    /// verify checks WITHOUT touching the local derived chain (`now_secs` is
    /// the verifier's clock, sampled by the caller). Parent linkage +
    /// contiguous height are already enforced by Inline's `validate_block`
    /// before app verify runs — not re-checked here.
    ///
    /// Rule SA (self-attestation): `block.proposal_view` must equal the
    /// verifier's OWN `ctx.round.view()` — a consensus input, not local state.
    /// For every CERTIFIED block, `proposal_view` is therefore the TRUE view it
    /// was proposed in, attested by that epoch's 2f+1 multisig and sealed in
    /// `digest()`; a lying proposer is voted false by every honest voter of
    /// that view and never notarizes. SA is a VOTE-TIME-ONLY obligation: no
    /// ingress path (cert-follower / backfill / cold-start / recovery) may
    /// re-check it — they have no `ctx`, and a re-check against local cert
    /// state would reject the legitimately re-proposed boundary block (F4).
    /// The boundary RE-PROPOSE never reaches here: marshal short-circuits on
    /// `digest == context.parent.1` BEFORE app-verify.
    fn structural_checks(
        block: &OrderBlock,
        parent: &OrderBlock,
        now_secs: u64,
        round: Round,
        expected_leader_index: Option<u8>,
    ) -> bool {
        block.proposal_view == round.view().get()
            && block.timestamp > parent.timestamp
            && block.timestamp <= now_secs + TIMESTAMP_FUTURE_TOLERANCE_SECS
            && gas_limit_within_1_1024(parent.gas_limit, block.gas_limit)
            && production_record_ok(&block.extra_data, expected_leader_index)
            && total_tx_gas(&block.txs).is_some_and(|gas| gas <= block.gas_limit)
    }

    /// Paced proposal body, factored out of `Application::propose` so the
    /// pacing/timestamp behavior is unit-testable (`AncestorStream` has no
    /// public constructor). `context` is the proposer's simplex context: it
    /// supplies `proposal_view = ctx.round.view()` (rule SA) and the
    /// parent-link classification for the `parent_seed` witness.
    async fn build_proposal<E: Clock>(
        &self,
        clock: &E,
        context: &SimplexContext<Digest, PublicKey>,
        parent: OrderBlock,
    ) -> Option<OrderBlock> {
        let height = parent.height + 1;

        // Parent-seed witness (§3 propose side) — never propose a block your
        // own verifier would reject. The ROUND is a pure function of AGREED
        // data: `Round::new(Ep, parent.proposal_view)` — the same value every
        // voter pins (rule PIN). It must NEVER come from a certificate: the
        // local first-wins finalization archive can name a different (spin)
        // round per node at the boundary (F4), and an honest proposer reading
        // it would split honest voters (R4). The signature bytes come from the
        // in-process SeedStore (written at the notarization hook for every
        // round this node's voter processed); a threshold seed is unique per
        // (group key, round), so bytes cannot split voters. A store miss ⇒
        // SKIP THE VIEW (fast view-change, same pattern as the exec-lag and
        // boundary-DKG gates) — never an invalid block, never a fallback for
        // the round. Hoisted above the pace sleep so a doomed view nullifies
        // ~1 s sooner.
        let (parent_epoch, seed_required) = witness_link(context);
        let parent_seed = if !seed_required {
            None
        } else {
            let ep = parent_epoch.expect("required implies Some(ep)");
            let round = Round::new(Epoch::new(ep), View::new(parent.proposal_view));
            let seed = self
                .seed_store
                .as_ref()
                .and_then(|store| store.lookup(round));
            match seed {
                Some(signature) => Some(Seed {
                    target_round: round,
                    signature,
                }),
                None => {
                    metrics::counter!("dpos_parent_seed_lookup_miss_total").increment(1);
                    let boundary = context.parent.0 == View::zero();
                    if boundary {
                        // The R4 liveness cost: only a mid-spin joiner or a
                        // node restarted between V0 and the boundary lands
                        // here (the store is in-memory). Must stay small.
                        metrics::counter!("dpos_parent_seed_boundary_skip_total").increment(1);
                    }
                    tracing::info!(
                        height,
                        ?round,
                        boundary,
                        "parent-seed witness not in SeedStore; skipping propose (view skip)"
                    );
                    return None;
                }
            }
        };

        // Item C (leader liveness, fast view-change): a CHANGE-epoch boundary leader

        // Item C (leader liveness, fast view-change): a CHANGE-epoch boundary leader
        // that does not yet hold the agreed `PK_E` cannot produce a valid boundary
        // proposal (every verifier rejects a boundary block without the asserted
        // outcome). Decline NOW — BEFORE the 1s pace sleep below — so the voter arms
        // `MissingProposal` → immediate Nullify → the next (share-holding) leader
        // proposes ~1s sooner. This is the SAME condition the post-pace
        // `beacon_outcome` gate enforces (see below), hoisted to save the pace on a
        // doomed view. It fires ONLY on a change-epoch first block, so a stable
        // beacon-active epoch (no `CeremonyStore` entry — the DKG runs only on a
        // committee change) is never affected.
        if let Some(bv) = self.beacon.as_ref() {
            let epoch = bv.epoch_of(height);
            if bv.is_change_epoch_first_block(height, epoch)
                && (bv.beacon_for_epoch)(epoch).is_none()
            {
                tracing::info!(
                    height,
                    epoch,
                    "beacon: boundary leader without epoch-E DKG outcome — declining propose \
                     (fast view-change)"
                );
                return None;
            }
        }

        // Pace to 1 blk/s: hold until wall clock reaches parent + 1s.
        // Cancellation-safe: Inline selects this future against
        // tx.closed(), so a moved-on view aborts the sleep.
        //
        // Capped at one interval from NOW: verify tolerates parents up to
        // TIMESTAMP_FUTURE_TOLERANCE_SECS ahead of our clock, and an uncapped
        // sleep on such a parent would overrun the peers' leader deadline
        // (its derivation assumes the pace component ≤ BLOCK_INTERVAL) —
        // a proposer with a lagging clock would be nullified on every view it
        // leads. The produced timestamp stays parent+1 (content, not wall
        // time), so chain-time monotonicity is unaffected.
        let pace_target =
            std::time::UNIX_EPOCH + Duration::from_secs(parent.timestamp) + BLOCK_INTERVAL;
        let pace_cap = clock.current() + BLOCK_INTERVAL;
        clock.sleep_until(pace_target.min(pace_cap)).await;

        // Execution gate (proposer-≤K-behind): the result commitment needs the
        // FINALIZED-tier derived hash at height − K — NOT the speculative head
        // (a still-speculative sibling A at h−K could re-finalize as sibling B,
        // committing a hash the network will diverge from: bundle-20260716T150148Z).
        // K guarantees h−K is finalized-reconciled before h commits its result;
        // a proposer whose local finalize reconcile has not caught up skips the
        // view rather than guessing. Sampled after the pace sleep — the EL gets
        // the full inter-block interval to reach height − K.
        let result = match result_target(height, self.dpos_activation_block) {
            ResultTarget::PreActivation => B256::ZERO,
            ResultTarget::Height(h) => match self.executed.finalized_executed_hash(h) {
                Some(hash) => hash,
                None => {
                    metrics::counter!("dpos_result_gate_finalized_miss_total").increment(1);
                    tracing::debug!(
                        height,
                        result_height = h,
                        executed_tip = self.executed.executed_tip(),
                        "finalized reconcile lags result target; skipping propose"
                    );
                    return None;
                }
            },
        };

        let gas_limit = step_gas_limit(parent.gas_limit, self.target_gas_limit);
        let timestamp = clock
            .current()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before UNIX_EPOCH")
            .as_secs()
            .max(parent.timestamp + 1);
        // Hoist the boundary/beacon_outcome decision ABOVE `assemble` so the tx
        // byte budget can reserve the beacon-outcome framing on a change-epoch
        // boundary block (bug 1 — otherwise txs at the full budget plus
        // beacon_outcome can exceed MAX_ORDER_BLOCK_SIZE / the p2p frame cap), and
        // the "DKG outcome not ready → skip view" early-return fires before any tx
        // assembly work.
        //
        // On a CHANGE-epoch first block this node, as proposer, MUST assert the
        // agreed DKG `Output` (PK_E) in `beacon_outcome`. If our ceremony has not
        // produced it yet, skip the view (like the exec-lag gate) rather than
        // propose a `None` that every verifier would reject on the epoch-type gate.
        let beacon_outcome: Option<Bytes> = match self.beacon.as_ref() {
            Some(bv) => {
                let epoch = bv.epoch_of(height);
                if bv.is_change_epoch_first_block(height, epoch) {
                    match (bv.beacon_for_epoch)(epoch) {
                        Some((out, _share)) => {
                            // Byzantine forge of the asserted PK_E; off the feature
                            // this is just `out`. The honest C-gate + certify hook
                            // Nullify it (§8.11.2).
                            #[cfg(feature = "dpos-devnet-byzantine")]
                            let out = if bv.forges_beacon_pk() {
                                tracing::warn!(
                                    height,
                                    epoch,
                                    "BYZANTINE: proposing forged PK_E at boundary"
                                );
                                crate::byzantine::forge_outcome_same_committee(&out)
                            } else {
                                out
                            };
                            tracing::info!(
                                height,
                                epoch,
                                "beacon: proposing change-epoch boundary with asserted PK_E"
                            );
                            Some(Bytes::from(encode_outcome(&out)))
                        }
                        None => {
                            tracing::info!(
                                height,
                                epoch,
                                "beacon: change-epoch boundary but DKG outcome not ready; skipping propose"
                            );
                            return None;
                        }
                    }
                } else {
                    None
                }
            }
            None => None,
        };

        // A boundary block reserves the beacon-outcome framing so the assembled
        // artifact always fits MAX_ORDER_BLOCK_SIZE and the byte-identical p2p frame
        // cap (bug 1); every other block gets the full budget.
        let tx_budget = if beacon_outcome.is_some() {
            TX_BYTE_BUDGET_AT_BOUNDARY
        } else {
            TX_BYTE_BUDGET
        };
        let txs = self.assembler.assemble(height, gas_limit, tx_budget);

        // Stamp the production record naming THIS node. commonware invokes
        // `propose` only on the elected leader, so `context.leader` is us — and
        // resolving the index through the same call verify uses is what makes the
        // proposer and its voters agree by construction rather than by convention.
        //
        // Both non-`Some` arms decline the view instead of proposing: a block
        // whose record this node cannot compute is a block its own verifier would
        // reject, and skipping the view costs one nullification while proposing it
        // costs a guaranteed one plus a wasted leader deadline. `Ok(None)` (no
        // committee map) is unreachable for a real proposer — `EpochEngine::new`
        // injects the map before the app can be asked to propose.
        let leader_index = match self.expected_leader_index(&context.leader) {
            Ok(Some(idx)) => idx,
            Ok(None) => {
                tracing::warn!(
                    height,
                    "propose: no committee index map — cannot stamp a production record; \
                     skipping propose (view skip)"
                );
                return None;
            }
            Err(e) => {
                tracing::warn!(
                    height,
                    error = %e,
                    "propose: cannot resolve own committee index; skipping propose (view skip)"
                );
                return None;
            }
        };
        let extra_data = Bytes::from(extra_data::encode_production_record(leader_index));

        if parent_seed.is_some() {
            metrics::counter!("dpos_parent_seed_embedded_total").increment(1);
        }
        // AMENDMENT 5: ride the E+1 dealer-log hashes this node has recorded, deduped
        // against those already finalized in ancestors (no reader wired ⇒ empty).
        let dkg_logs = self
            .beacon
            .as_ref()
            .map(|bv| bv.dkg_logs_to_include(height))
            .unwrap_or_default();
        Some(OrderBlock {
            parent: parent.digest(),
            height,
            // Rule SA: self-attest the view this block is proposed in — what
            // makes the witness round agreed data for the CHILD (§2/§3).
            proposal_view: context.round.view().get(),
            timestamp,
            fee_recipient: self.fee_recipient,
            gas_limit,
            extra_data,
            result,
            txs,
            beacon_outcome,
            dkg_logs,
            parent_seed,
        })
    }
}

/// Σ tx.gas_limit with overflow as None — the one stateless tx bound verify
/// enforces: it caps the execution work an agreed artifact can demand.
/// Signature/chain-id/nonce validity are NOT checked here: the deterministic
/// skip rule in derivation handles them identically on every node, and
/// checking them in verify would add per-tx ECDSA work to the vote path
/// without bounding anything the gas cap doesn't already bound.
fn total_tx_gas(txs: &[TransactionSigned]) -> Option<u64> {
    txs.iter()
        .try_fold(0u64, |acc, tx| acc.checked_add(tx.gas_limit()))
}

/// Beacon boundary gate (returns `false` ⇒ vote against the block):
/// - epoch-type gate: `beacon_outcome` is present IFF this is a change-epoch
///   first block (a `Some` anywhere else, or a missing `Some` on a change block,
///   is malformed → reject);
/// - on a change-epoch first block: this node's epoch-E share must lie on the
///   proposer's asserted polynomial ("C", [`validate_share_on_poly`]). An
///   observer / not-yet-ready share withholds the qualifying accept (votes
///   `false`); a quorum of converged share-holders carries the block, a forged
///   poly that misses the honest shares cannot reach quorum.
///
/// `beacon == None` (cold-start epoch 0 / followers / tests) ⇒ no gating. The
/// seed-verify backstop that closes C's high-degree caveat is the always-active
/// deriver path (recovered seed vs the committed `PK_E`), NOT this gate.
fn beacon_gate_decision(beacon: Option<&BeaconVerify>, block: &OrderBlock) -> bool {
    let Some(bv) = beacon else {
        return true; // no beacon context — nothing to gate
    };
    let epoch = bv.epoch_of(block.height);
    // AMENDMENT 5 dkg_logs FORMAT-ONLY verify (a verifier need NOT hold the bodies —
    // off-chain-first). The codec already enforced canonical strictly-ascending idx +
    // the entry COUNT and the idx VALUE against the network-wide MAX_COMMITTEE_SIZE;
    // here bound each `idx < n = committee[E+1].len()` (a hash for a non-committee
    // position is malformed). Committee unreadable ⇒ skip (accept-biased, like the
    // other committee reads).
    //
    // The accept-biased arm is DELIBERATE and must not be "fixed" into a false vote:
    // voting false when the committee is unreadable makes the vote a function of THIS
    // node's EL-sync progress — the prohibited local-state class spelled out at the
    // verify-time comment further down (nullify-storm / verify-gate freeze). Two other
    // layers cover what slips through it: the codec bound above caps the blast radius
    // to `idx ∈ [n, MAX_COMMITTEE_SIZE)`, and `ceremony::scoped_pinned_logs`
    // deterministically SKIPS an unmappable idx instead of wedging on it.
    if !block.dkg_logs.is_empty() {
        if let Some(committee) = (bv.committee_for)(epoch + 1) {
            let n = committee.len();
            if block.dkg_logs.iter().any(|(idx, _)| (*idx as usize) >= n) {
                tracing::warn!(
                    height = block.height,
                    epoch,
                    n,
                    "dkg_logs: an idx is out of committee[E+1] range — voting false"
                );
                metrics::counter!("dpos_marker_reject_total", "reason" => "dkg_logs_idx")
                    .increment(1);
                return false;
            }
        }
    }
    let is_change = bv.is_change_epoch_first_block(block.height, epoch);
    if block.beacon_outcome.is_some() != is_change {
        tracing::warn!(
            height = block.height,
            epoch,
            is_change,
            has_outcome = block.beacon_outcome.is_some(),
            "beacon epoch-type gate: beacon_outcome presence mismatch — voting false"
        );
        return false;
    }
    let Some(bytes) = block.beacon_outcome.as_ref() else {
        return true; // non-change block, correctly absent
    };
    let outcome = match parse_outcome(bytes) {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(
                height = block.height,
                epoch,
                ?e,
                "beacon_outcome failed to parse"
            );
            return false;
        }
    };
    // GRAFT-3 bind RETIRED (AMENDMENT 5): `dkgQual[E]` is presence-only now (no
    // on-chain PK to bind against). `PK_{E+1}` is a pure function of the finalized
    // dealer-log set, so there is no forged-PK attack surface to close here — the
    // "C" share-on-poly gate below + the `certify.rs` seed σ-verify (which Nullifies
    // a boundary block whose recovered seed does not verify under the asserted PK)
    // are the divergence catchers.
    // DEVNET/TEST-ONLY: a byzantine node colluding to notarize a forged boundary
    // votes yes regardless of the "C" gate, so a byzantine quorum can carry the
    // forge to the certify hook (where the seed-verify Nullifies it). HARMLESS to
    // an honest leader's real boundary (its real share passes C anyway). Never
    // reachable in production (the flag does not compile in).
    if bv.forges_beacon_pk() {
        tracing::warn!(
            height = block.height,
            epoch,
            "BYZANTINE: bypassing C gate for change-epoch boundary block"
        );
        return true;
    }
    let Some(committee_e) = (bv.committee_for)(epoch) else {
        tracing::warn!(epoch, "committee[E] unavailable at verify — voting false");
        return false;
    };
    match (bv.beacon_for_epoch)(epoch) {
        Some((_out, share)) => {
            let ok = validate_share_on_poly(&outcome, &committee_e, &share);
            if !ok {
                tracing::warn!(epoch, "C share-on-poly FAILED for asserted outcome");
            }
            ok
        }
        None => {
            tracing::debug!(
                epoch,
                "no epoch-E share — withholding beacon qualifying vote"
            );
            false
        }
    }
}

impl<E, XC, A> Application<E> for FluentApp<XC, A>
where
    E: Rng + Spawner + Metrics + Clock + Send + 'static,
    XC: ExecutedChain,
    A: OrderingAssembler,
{
    type SigningScheme = BlsScheme;
    type Context = SimplexContext<Digest, PublicKey>;
    type Block = OrderBlock;

    async fn genesis(&mut self) -> OrderBlock {
        (*self.genesis).clone()
    }

    async fn propose<P: BlockProvider<Block = OrderBlock>>(
        &mut self,
        ctx: (E, Self::Context),
        mut ancestry: AncestorStream<P, OrderBlock>,
    ) -> Option<OrderBlock> {
        let parent = ancestry.next().await?;
        let block = self.build_proposal(&ctx.0, &ctx.1, parent).await;
        if let Some(b) = &block {
            // commonware invokes `propose` ONLY on the elected leader, so this
            // fires exactly once per block this node proposes — the per-validator
            // proposer signal the weighted-VRF smoke tallies (`log_count`) and the
            // D2 proposer-share monitoring seam consumes (Prometheus counter).
            tracing::info!(height = b.height, "dpos: proposing order block");
            metrics::counter!("dpos_proposed_total").increment(1);
        }
        block
    }
}

impl<E, XC, A> VerifyingApplication<E> for FluentApp<XC, A>
where
    E: Rng + Spawner + Metrics + Clock + Send + 'static,
    XC: ExecutedChain,
    A: OrderingAssembler,
{
    async fn verify<P: BlockProvider<Block = OrderBlock>>(
        &mut self,
        ctx: (E, Self::Context),
        mut ancestry: AncestorStream<P, OrderBlock>,
    ) -> bool {
        // Inline seeds the stream [block, parent] (validation.rs:186) — both
        // next() calls return buffered, no marshal fetch. At the boundary the
        // parent IS the previous epoch's terminal block `L` — every node
        // running the `E+1` engine already holds its body (Inline::genesis).
        let Some(block) = ancestry.next().await else {
            return false;
        };
        let Some(parent) = ancestry.next().await else {
            return false;
        };
        self.verify_block(&ctx.0, &ctx.1, &block, &parent).await
    }
}

impl<XC, A> FluentApp<XC, A>
where
    XC: ExecutedChain,
    A: OrderingAssembler,
{
    /// The whole vote decision over a `(block, parent)` pair, factored out of
    /// the trait `verify` so the gate is unit-testable (`AncestorStream` has
    /// no public constructor).
    async fn verify_block<E: Clock>(
        &self,
        clock: &E,
        ctx: &SimplexContext<Digest, PublicKey>,
        block: &OrderBlock,
        parent: &OrderBlock,
    ) -> bool {
        let now_secs = clock
            .current()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before UNIX_EPOCH")
            .as_secs();
        // The production record names its own producer, so unlike the bitmap it
        // replaced it IS checkable at vote time — against `ctx.leader`, which is
        // consensus-supplied agreed data, never local state. The two-step form is
        // mandatory: `Err` is a REJECT, never a skip, and folding it into the
        // permissive `Ok(None)` arm (`as_ref().and_then(..)`) silently inverts the
        // rule. `Ok(None)` means this instance holds no committee map (a
        // verify-only scheme, a follower, a test) — it casts no vote, so the index
        // it cannot compute is not a vote condition.
        let expected_leader_index = match self.expected_leader_index(&ctx.leader) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    height = block.height,
                    round = ?ctx.round,
                    error = %e,
                    "production record: cannot resolve the round leader's committee index — \
                     voting false"
                );
                metrics::counter!("dpos_marker_reject_total", "reason" => "leader_index")
                    .increment(1);
                return false;
            }
        };
        if !Self::structural_checks(block, parent, now_secs, ctx.round, expected_leader_index) {
            return false;
        }

        // WHY THE RECORD IS CHECKED AGAINST `ctx.leader` AND NOTHING ELSE, carried
        // forward from the retired bitmap's prohibition because the hazard it names
        // is unchanged: a verify-time byte-compare against this node's OWN
        // marshal-archived finalization is UNSOUND. The archive holds each node's
        // first-observed, locally-assembled cert — commonware `assemble` keeps any
        // ≥quorum attestation set and `verify_certificate` accepts any ≥quorum
        // bitmap, so honest nodes legitimately hold byte-DIFFERENT certs for the
        // same round → a byte-exact compare false-rejects honest proposals →
        // nullify storm / liveness stall (same class as the verify-gate
        // non-deterministic-cert-freeze hazard). DO NOT re-introduce a verify-time
        // compare against local cert or lookup state; `ctx.leader` is agreed, a
        // local archive is not.

        // Beacon boundary gate: epoch-type (beacon_outcome present IFF change-epoch
        // first block) + the "C" share-on-polynomial qualification on a change block.
        if !beacon_gate_decision(self.beacon.as_ref(), block) {
            return false;
        }

        // ── Parent-seed witness gate, SYNCHRONOUS PRELUDE (§3) — no budget,
        // no await. Everything here is a pure function of agreed data (the two
        // block bodies + ctx); a `false` returns immediately.
        let (parent_epoch, seed_required) = witness_link(ctx);
        let witness = if !seed_required {
            // Pre-bootstrap / anchor link: a witness MUST be absent (a present
            // one would be unagreeable data smuggled under the digest).
            if block.parent_seed.is_some() {
                metrics::counter!("dpos_parent_seed_reject_total", "reason" => "pre_bootstrap")
                    .increment(1);
                return false;
            }
            None
        } else {
            // Beacon-active link: the witness is required UNCONDITIONALLY —
            // there is no live no-beacon fallback (F8), so absence is the
            // downgrade attack, not a degraded mode.
            let Some(s) = block.parent_seed.as_ref() else {
                metrics::counter!("dpos_parent_seed_reject_total", "reason" => "missing")
                    .increment(1);
                return false;
            };
            // RULE PIN — the witness round is a pure function of agreed data
            // on EVERY link, boundary included: `parent.proposal_view` is
            // truthful by rule SA + the parent-epoch committee multisig over
            // digest(parent). Key-free: the pin holds even when PK_Ep is
            // unresolvable below.
            let ep = parent_epoch.expect("required implies Some(ep)");
            let pinned = Round::new(Epoch::new(ep), View::new(parent.proposal_view));
            if s.target_round != pinned {
                metrics::counter!("dpos_parent_seed_reject_total", "reason" => "pin").increment(1);
                return false;
            }
            Some((ep, s))
        };
        // P4 tripwire (kept alongside PIN, not subsumed by it): on a
        // same-epoch link `parent.proposal_view` and `ctx.parent.0` are
        // provably equal, but they come from DIFFERENT SOURCES (block body vs
        // simplex context) — a mismatch is a block certified at a view it did
        // not claim, exactly the class that would silently un-back PIN.
        if ctx.parent.0 != View::zero() && parent.proposal_view != ctx.parent.0.get() {
            metrics::counter!("dpos_parent_view_mismatch_total").increment(1);
            return false;
        }

        // ── THE ONE INTERLEAVED POLL LOOP (Fix B): the witness-signature arm
        // and the result gate share the SAME 40-tick budget and are evaluated
        // together on every tick — neither can starve the other (the two reads
        // correlate: `committee_for` and `executed_hash` hit the same EL
        // state). Common path: both resolve on tick 0, no sleep. A definitive
        // `false` from either condition returns at once. Deadline verdicts are
        // independent: an unresolved key ⇒ ACCEPT-biased (+metric, residual
        // R-P1 — rejecting on crypto you cannot run is a correlated
        // local-state-dependent false vote ⇒ chain halt); an unresolved result
        // ⇒ vote false (EL backpressure, unchanged).
        // The result gate samples the FINALIZED tier (see
        // `ExecutedChain::finalized_executed_hash`): the honest semantics are
        // "wait for h−K to be finalized-reconciled locally", not "match the
        // speculative head" — the speculative head can carry a sibling that the
        // finalization will replace (bundle-20260716T150148Z).
        let check = |this: &Self| {
            result_matches(
                block.result,
                block.height,
                this.dpos_activation_block,
                |h| this.executed.finalized_executed_hash(h),
            )
        };
        let ec = ctx.round.epoch().get();
        let mut key_done = witness.is_none();
        let mut result_done = false;
        // Result-gate observability: total wall time this verify spends waiting on
        // `executed_hash(h-K)` — 0 on the common tick-0 resolve. A wait past one
        // poll tick means the local EL has fallen >K behind the consensus tip
        // (deferred-executor backlog), the mechanism behind late votes / nullify
        // under heavy blocks. Recorded exactly once per verify at the result arm's
        // terminal (resolve / definitive-false / budget-exhausted).
        let gate_started = std::time::Instant::now();
        let mut gate_slow_logged = false;
        let polls = (VERIFY_EXEC_BUDGET.as_micros() / VERIFY_EXEC_POLL.as_micros()) as u32;
        for tick in 0..=polls {
            if tick > 0 {
                clock.sleep(VERIFY_EXEC_POLL).await;
            }
            if !key_done {
                let (ep, s) = witness.expect("key arm exists only with a witness");
                match self.group_public_for(ep) {
                    KeyLookup::Resolved(pk) => {
                        let ns = self
                            .beacon
                            .as_ref()
                            .map(|bv| bv.seed_namespace.as_slice())
                            .unwrap_or(&[]);
                        if !verify_seed(&pk, ns, s.target_round, &s.signature) {
                            // Loud + byte-diffable: the resolved-key fingerprint is
                            // what lets a lone rejecting node's PK_Ep be compared
                            // against the quorum's from logs alone (a diverged
                            // carried-forward key rejects here with ladder never
                            // having run — soak 2026-07-14 v5@epoch77).
                            tracing::warn!(
                                height = block.height,
                                parent_epoch = ep,
                                round = ?s.target_round,
                                group_public = %pk_prefix(&pk),
                                "parent-seed witness FAILED signature verify under \
                                 resolved PK_Ep; voting false (reject reason=bad_signature)"
                            );
                            metrics::counter!(
                                "dpos_parent_seed_reject_total",
                                "reason" => "bad_signature"
                            )
                            .increment(1);
                            return false;
                        }
                        key_done = true;
                    }
                    KeyLookup::Unknown => {
                        // Structurally cannot know PK_Ep — re-polling cannot
                        // help; resolve NOW, burn zero further budget. Accept:
                        // safety is carried by key availability + quorum
                        // (b ⊆ churn-in), not by reject-bias.
                        tracing::debug!(
                            height = block.height,
                            parent_epoch = ep,
                            "witness key structurally unknown (no PK_Ep on this \
                             node); accepting unverified (reason=unknown)"
                        );
                        metrics::counter!(
                            "dpos_parent_seed_boundary_unverified_total",
                            "reason" => "unknown"
                        )
                        .increment(1);
                        if ep == ec {
                            // W1 makes a same-epoch miss unreachable for any
                            // node that votes at all.
                            debug_assert!(false, "same-epoch PK_E miss (W1 violated)");
                            metrics::counter!("dpos_group_key_invariant_violation_total")
                                .increment(1);
                        }
                        key_done = true;
                    }
                    // Transient — stays pending, retried NEXT tick. Never
                    // cached (§5 b).
                    KeyLookup::ReadFailed => {}
                }
            }
            if !result_done {
                match check(self) {
                    Some(false) => {
                        metrics::histogram!("dpos_verify_result_gate_wait_seconds")
                            .record(gate_started.elapsed().as_secs_f64());
                        return false;
                    }
                    Some(true) => {
                        metrics::histogram!("dpos_verify_result_gate_wait_seconds")
                            .record(gate_started.elapsed().as_secs_f64());
                        result_done = true;
                    }
                    None => {
                        // Slow path: still no finalized `executed_hash(h-K)` after ≥1 tick.
                        // Log once per verify, and — via the shared cursor — at
                        // most once per height (a height is re-verified across
                        // views; the saturation signal is per-height).
                        if tick > 0 && !gate_slow_logged {
                            gate_slow_logged = true;
                            let prev = self
                                .verify_gate_last_logged_height
                                .swap(block.height, Ordering::Relaxed);
                            if prev != block.height {
                                let executed_tip = self.executed.executed_tip();
                                tracing::info!(
                                    height = block.height,
                                    result_height =
                                        block.height.saturating_sub(crate::order_block::K),
                                    waited_ms = gate_started.elapsed().as_millis() as u64,
                                    executed_tip,
                                    executor_lag_blocks = block.height.saturating_sub(executed_tip),
                                    "verify result-gate waiting on finalized executed_hash(h-K); \
                                     local finalize reconcile lags the consensus tip \
                                     (deferred-executor backlog)"
                                );
                            }
                        }
                    }
                }
            }
            if key_done && result_done {
                return true;
            }
        }
        // Budget exhausted: each condition keeps its own, independent verdict.
        if !key_done {
            // The committee read outlasted the shared budget ⇒ accept-biased
            // (residual R-P1). A SUSTAINED rise after the read recovers means
            // a failure got cached — the sticky-`None` bug.
            tracing::debug!(
                height = block.height,
                parent_epoch = witness.map(|(e, _)| e),
                "witness key read outlasted the verify budget; accepting \
                 unverified (reason=read_failed)"
            );
            metrics::counter!(
                "dpos_parent_seed_boundary_unverified_total",
                "reason" => "read_failed"
            )
            .increment(1);
        }
        if !result_done {
            // Budget exhausted with no finalized h−K locally: finalization is
            // lagging into the gate.
            metrics::counter!("dpos_result_gate_finalized_miss_total").increment(1);
            metrics::histogram!("dpos_verify_result_gate_wait_seconds")
                .record(gate_started.elapsed().as_secs_f64());
            tracing::warn!(
                height = block.height,
                executed_tip = self.executed.executed_tip(),
                "verify exec budget exhausted; voting false (finalize-reconcile backpressure)"
            );
            return false;
        }
        true
    }
}

impl<XC, A> Reporter for FluentApp<XC, A>
where
    XC: Clone + Send + Sync + 'static,
    A: OrderingAssembler,
{
    type Activity = Update<OrderBlock>;

    async fn report(&mut self, activity: Update<OrderBlock>) {
        // Boundary hook fires for `Update::Block` only — the epoch-boundary
        // detection integration point. The assembler observes the same block
        // so its in-flight suffix tracks ordered-but-unexecuted txs.
        if let Update::Block(ref block, _) = activity {
            // W4 — the observed-outcome cursor (ladder step 2): memoize the
            // agreed `PK_E` carried by a change-epoch first block into the
            // shared group-key map. Off the vote path; covers non-members that
            // followed the chain since the change epoch (a stable committee
            // publishes no outcome, so this cursor alone is NOT sufficient —
            // ladder step 1 covers the restarted signer).
            if let (Some(bv), Some(bytes)) = (self.beacon.as_ref(), block.beacon_outcome.as_ref()) {
                match parse_outcome(bytes) {
                    Ok(outcome) => {
                        let epoch = bv.epoch_of(block.height);
                        let pk = *group_public_key(&outcome);
                        tracing::debug!(
                            epoch,
                            height = block.height,
                            group_public = %pk_prefix(&pk),
                            "W4: memoizing observed-outcome group key"
                        );
                        insert_group_key(&self.group_keys, epoch, pk, KeySource::ObservedOutcome);
                    }
                    // The outcome was already validated at vote time; a parse
                    // failure here is diagnostic, never state-advancing.
                    Err(e) => tracing::warn!(
                        height = block.height,
                        ?e,
                        "finalized beacon_outcome unparseable; group-key cursor skipped"
                    ),
                }
            }
            // AMENDMENT 5: accumulate this finalized block's dealer-log hashes into
            // the shared finalized set (height-gated ≤ H_settle) — the deterministic
            // finalize input the DkgActor's `select` runs over.
            if let Some(bv) = self.beacon.as_ref() {
                bv.note_finalized_dkg_logs(block);
            }
            self.assembler.observe_finalized(block);
            (self.boundary_hook)(block.clone());
        }
        // Ack flow: the `Exact` ack inside Update::Block travels INSIDE this
        // command and is fired by the executor after derive + import. Marshal
        // awaits the ack via PendingAcks; if the executor task crashes
        // mid-flight, the dropped ack trips marshal's supervisor cascade.
        if let Err(e) = self.executor.send(executor::Message {
            cause: tracing::Span::current(),
            command: executor::Command::Finalize(Box::new(activity)),
        }) {
            tracing::error!(?e, "executor mailbox closed; finalize command dropped");
        }
    }
}

/// Bound for the reth beacon-engine handle used by the executor. No
/// payload-attributes parameter: the deferred path never builds via
/// FCU-with-attrs (blocks are derived, not requested from a builder).
pub trait BeaconEngineLike: Send + Sync + 'static {
    /// Full derivation output accepted by [`Self::import_derived`].
    type ExecutionData: Send + 'static;

    /// Drive the fork-choice. The VERDICT (incl. a semantic
    /// `PayloadStatusEnum::Invalid`) rides in `Ok(..)`; a TRANSPORT failure (a
    /// closed engine channel / RPC-handle blip) is the typed
    /// [`TransportError`] in `Err`. This split is TYPE-LEVEL (family 5): a
    /// verdict can never be folded into the transport error, so the executor's
    /// fork-safety rule ("retry ⇔ transport `Err`; SafetyHalt ⇔ `Ok(Invalid)`")
    /// is a property of the return type, not a comment at each call site.
    fn fork_choice_updated(
        &self,
        state: ForkchoiceState,
    ) -> impl std::future::Future<Output = Result<ForkchoiceUpdated, TransportError>> + Send;

    /// Import one derived block into the EL. Implementations either hand
    /// reth the pre-executed artifacts (`InsertExecutedBlock` — single
    /// execution) or fall back to `new_payload` (reth re-executes; the
    /// conformance/escape-hatch mode). Same transport-vs-verdict split as
    /// [`Self::fork_choice_updated`]: this shared return type is what makes the
    /// two engine entry points get the IDENTICAL transport class
    /// ([`crate::fault::FaultClass::TransientExternal`]`(EngineRetry)`), closing the historic
    /// asymmetry where an import transport error was actor-death while its FCU
    /// sibling retried.
    fn import_derived(
        &self,
        data: Self::ExecutionData,
    ) -> impl std::future::Future<Output = Result<PayloadStatus, TransportError>> + Send;
}

/// The executor-facing view of one derivation's output. Identity (hash,
/// number) is all the consensus crate needs; the concrete type carries the
/// full execution artifacts (receipts, bundle state, trie updates) so the
/// node-side importer can hand reth an already-executed block instead of
/// re-executing via `new_payload`.
pub trait DerivedBlock: Send + Sync + 'static {
    fn evm_hash(&self) -> B256;
    fn number(&self) -> u64;
    /// Beacon observation for this block, surfaced to the executor's
    /// `BeaconMetrics`: `Some(true)` = `prev_randao` was the verified threshold
    /// seed; `Some(false)` = a beacon-active block fell back to `order.digest()`
    /// (seed absent/unverified — the certify hook Nullifies such a boundary, so
    /// this is the local pre-Nullify observation); `None` = pre-beacon / no seed
    /// (not a beacon-active observation). Defaults to `None`.
    fn beacon_active(&self) -> Option<bool> {
        None
    }
}

impl DerivedBlock for SealedBlock<RethBlock> {
    fn evm_hash(&self) -> B256 {
        self.hash()
    }
    fn number(&self) -> u64 {
        self.number
    }
}

/// Typed "parent header not readable yet" derivation failure. reth-2.2
/// canonicalizes imports eagerly on the engine-tree thread, so a block can be
/// "added to canonical chain" milliseconds before provider reads see its
/// header; a recovery path that derives against a parent imported
/// concurrently (devp2p live-sync or its own previous iteration's import)
/// must be able to tell this transient visibility race from a real failure.
#[derive(Debug, thiserror::Error)]
#[error("derive: parent header {0} not found")]
pub struct ParentHeaderMissing(pub B256);

/// Derivation with a bounded retry on the parent-visibility race above. The
/// live executor is immune — it awaits an FCU response after every block —
/// but paths that derive against a parent imported WITHOUT an awaited FCU in
/// between (the crash-recovery walk; the follower's first derive after an
/// EL-sync jump, where devp2p canonicalized the parent) must absorb the race
/// here. Any other derivation error stays immediately fatal.
pub(crate) async fn derive_with_visibility_retry<C, D>(
    ctx: &C,
    deriver: &D,
    order: &OrderBlock,
    parent_hash: B256,
    seed: Option<Seed>,
) -> eyre::Result<D::Derived>
where
    C: commonware_runtime::Clock,
    D: DerivedBlockBuilder,
{
    const RETRY: Duration = Duration::from_millis(100);
    const DEADLINE: Duration = Duration::from_secs(10);
    let deadline = ctx.current() + DEADLINE;
    loop {
        match deriver
            .derive_and_execute(order.clone(), parent_hash, seed.clone())
            .await
        {
            Err(e)
                if e.downcast_ref::<ParentHeaderMissing>().is_some()
                    && ctx.current() < deadline =>
            {
                ctx.sleep(RETRY).await;
            }
            other => return other,
        }
    }
}

/// Deterministic OrderBlock → derived-EVM-block execution: every node must
/// compute a byte-identical derived block for the same `(order, parent)` —
/// this is the function whose output the committee's `result` agreement
/// attests. Implemented in the node crate over reth-evm's `BlockBuilder`
/// (same execution code path as the stock payload builder, so semantics are
/// identical to a built block).
pub trait DerivedBlockBuilder: Send + Sync + 'static {
    /// Full derivation output (block + execution artifacts).
    type Derived: DerivedBlock;

    fn derive_and_execute(
        &self,
        order: OrderBlock,
        parent_evm_hash: B256,
        seed: Option<Seed>,
    ) -> impl std::future::Future<Output = eyre::Result<Self::Derived>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_consensus::types::{Epoch, View};
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
    use commonware_runtime::Runner as _;
    use std::sync::Mutex;

    fn test_group_keys() -> GroupKeys {
        Arc::new(RwLock::new(BTreeMap::new()))
    }

    // The monotone finalized-execution cursor: tier-F resolves reth's canonical
    // hash at-or-below the cursor (no sibling can exist there) and `None` above
    // it (still speculative-tier); `advance` is monotone; a provider miss
    // at-or-below the cursor returns `None`, never a wrong hash; and the cursor
    // is visible across clones (the executor advances, the gate reads a clone).
    // (Migrated from the deleted map's `finalized_results_prunes_below_the_window`
    // + `finalized_results_floor_falls_back_to_canonical_below_it` — the map,
    // its retain window, and the separate floor collapsed into this cursor:
    // reth IS the store, so there is nothing to prune and no hashes to retain.)
    #[test]
    fn finalized_cursor_resolves_canonical_at_or_below_and_none_above() {
        let cursor = FinalizedCursor::default();
        let reader = cursor.clone();
        let canonical = |h: u64| (h <= 20).then(|| B256::repeat_byte(h as u8));
        cursor.advance(10);

        // At and below the cursor: the provider hash IS the finalized hash.
        assert_eq!(reader.resolve(10, canonical), Some(B256::repeat_byte(10)));
        assert_eq!(reader.resolve(3, canonical), Some(B256::repeat_byte(3)));
        // Above the cursor: `None` even though canonical HAS the height (that
        // hash is still speculative-tier there — a sibling can still finalize).
        assert_eq!(reader.resolve(11, canonical), None);
        // Advancing the cursor exposes the next height through the provider.
        cursor.advance(12);
        assert_eq!(reader.resolve(12, canonical), Some(B256::repeat_byte(12)));
        assert_eq!(reader.resolve(13, canonical), None);
        // Monotone: a lower advance is a no-op.
        cursor.advance(5);
        assert_eq!(
            reader.resolve(13, canonical),
            None,
            "cursor did not regress"
        );
        // A provider miss at-or-below the cursor ⇒ None, never a wrong hash
        // (deep-pruned history / a crash that lost the reth tail above the ack).
        assert_eq!(
            reader.resolve(9, |_| None),
            None,
            "provider miss ≤ cursor stays None"
        );
    }

    /// A resolver for tests that exercise no key ladder: structurally-no-key.
    fn no_key_lookup() -> GroupKeyFor {
        Arc::new(|_| KeyLookup::Unknown)
    }

    fn sample_context(view: u64) -> SimplexContext<Digest, PublicKey> {
        SimplexContext {
            round: Round::new(Epoch::new(0), View::new(view)),
            leader: Ed25519PrivateKey::from_seed(7).public_key(),
            parent: (View::new(view.saturating_sub(1)), Digest(B256::ZERO)),
        }
    }

    #[derive(Clone, Default)]
    struct NoChain;
    impl ExecutedChain for NoChain {
        fn executed_tip(&self) -> u64 {
            0
        }
        fn spec_executed_hash(&self, _height: u64) -> Option<B256> {
            None
        }
        // Test double: no chain, so both tiers are empty.
        fn finalized_executed_hash(&self, _height: u64) -> Option<B256> {
            None
        }
    }

    struct NoTxs;
    impl OrderingAssembler for NoTxs {
        fn assemble(&self, _h: u64, _g: u64, _b: usize) -> Vec<TransactionSigned> {
            Vec::new()
        }
        fn observe_finalized(&self, _block: &OrderBlock) {}
    }

    fn sample_order(parent: Digest, height: u64) -> OrderBlock {
        OrderBlock {
            parent,
            height,
            proposal_view: 0,
            timestamp: 1_700_000_000 + height,
            fee_recipient: Address::ZERO,
            gas_limit: 30_000_000,
            extra_data: Bytes::new(),
            result: B256::ZERO,
            txs: Vec::new(),
            beacon_outcome: None,
            dkg_logs: Vec::new(),
            parent_seed: None,
        }
    }

    /// A committee BiMap in the SAME shape production builds
    /// (`epoch_committee_from_snapshot` → `EpochCommittee::from_pairs`), so the
    /// index these tests assert against is commonware's sorted order, not
    /// insertion order.
    fn test_committee(
        n: usize,
        seed: u64,
    ) -> (Vec<Ed25519PrivateKey>, BiMap<PeerPubkey, BlsPubkey>) {
        let keys: Vec<Ed25519PrivateKey> = (0..n)
            .map(|i| Ed25519PrivateKey::from_seed(seed.wrapping_mul(1000) + i as u64))
            .collect();
        let bimap = committee_bimap(&keys, seed);
        (keys, bimap)
    }

    fn committee_bimap(keys: &[Ed25519PrivateKey], seed: u64) -> BiMap<PeerPubkey, BlsPubkey> {
        use commonware_codec::DecodeExt as _;
        use fluentbase_bls::keys::ValidatorBlsKeypair;
        use rand_08::{rngs::StdRng, SeedableRng as _};

        let mut rng = StdRng::seed_from_u64(seed);
        keys.iter()
            .map(|p| {
                let bls = ValidatorBlsKeypair::generate(&mut rng);
                (
                    p.public_key(),
                    BlsPubkey::decode(bls.public_bytes().as_slice()).unwrap(),
                )
            })
            .collect::<Vec<_>>()
            .try_into()
            .expect("unique participants")
    }

    /// A committee containing the leader EVERY propose fixture elects
    /// (`sample_context` / `propose_ctx` both name `from_seed(7)`), so a
    /// proposing app can resolve its own index and stamp a production record.
    /// Without it `build_proposal` declines the view — which is the production
    /// behaviour, not a test artifact: `EpochEngine::new` always injects the map
    /// before an app can be asked to propose.
    fn propose_committee() -> Arc<BiMap<PeerPubkey, BlsPubkey>> {
        let keys: Vec<Ed25519PrivateKey> = [7u64, 8, 9]
            .into_iter()
            .map(Ed25519PrivateKey::from_seed)
            .collect();
        Arc::new(committee_bimap(&keys, 7))
    }

    /// The index `propose_committee()` assigns to the fixture leader.
    fn propose_leader_index() -> u8 {
        propose_committee()
            .position(&Ed25519PrivateKey::from_seed(7).public_key())
            .expect("fixture leader is a member") as u8
    }

    /// The three states must stay distinct. Folding "the leader is not in the
    /// map" into the permissive "there is no map" is a one-token mistake
    /// (`as_ref().and_then(..)`) that silently turns a reject into a skip on the
    /// vote path — which is why the return type is `Result<Option<_>, _>` and
    /// not an `Option`.
    #[test]
    fn expected_leader_index_keeps_its_three_states_apart() {
        let (executor, _rx) = fresh_mailbox();
        let hook: Arc<dyn Fn(OrderBlock) + Send + Sync> = Arc::new(|_| {});
        let (keys, committee) = test_committee(5, 7);
        let leader = keys[0].public_key();

        // No map ⇒ not a voter ⇒ skip, not fail.
        let bare = build_app(executor.clone(), hook.clone());
        assert_eq!(bare.expected_leader_index(&leader), Ok(None));

        // Map present and the leader is in it ⇒ commonware's sorted position.
        let bimap = Arc::new(committee);
        let seated = build_app(executor.clone(), hook.clone()).with_committee_index(bimap.clone());
        let expected = bimap.position(&leader).expect("leader is a member") as u8;
        assert_eq!(seated.expected_leader_index(&leader), Ok(Some(expected)));

        // Map present and the leader is NOT in it ⇒ REJECT, never a skip.
        let (outsiders, _) = test_committee(1, 99);
        let stranger = outsiders[0].public_key();
        assert!(
            bimap.position(&stranger).is_none(),
            "fixture must be disjoint"
        );
        assert_eq!(
            seated.expected_leader_index(&stranger),
            Err(LeaderIndexError::LeaderNotInCommittee)
        );
    }

    /// The `None` arm is permissive by design (a non-voter's value is not a vote
    /// condition), so it is pinned by test rather than by comment — and the
    /// `Some` arm must reject the empty field the EXECUTOR is required to
    /// tolerate.
    #[test]
    fn production_record_rule_arms() {
        let good = extra_data::encode_production_record(3);

        // No expectation ⇒ every input passes, including one that is not a
        // record at all. This arm is why a stale `None` would be dangerous.
        assert!(production_record_ok(&good, None));
        assert!(production_record_ok(&[], None));
        assert!(production_record_ok(&[0xAB; 24], None));

        assert!(production_record_ok(&good, Some(3)));
        assert!(
            !production_record_ok(&good, Some(4)),
            "wrong index must fail"
        );
        assert!(
            !production_record_ok(&[], Some(3)),
            "empty must REJECT at verify even though the executor tolerates it"
        );
        assert!(!production_record_ok(&[1u8], Some(3)), "short must fail");
        assert!(
            !production_record_ok(&[1u8, 3, 0], Some(3)),
            "long must fail — the OrderBlock codec tolerates 4 KiB, the reth header caps at 32"
        );
        assert!(
            !production_record_ok(&[2u8, 3], Some(3)),
            "unknown version must fail closed"
        );
    }

    fn build_app(
        executor: executor::Mailbox,
        hook: Arc<dyn Fn(OrderBlock) + Send + Sync>,
    ) -> FluentApp<NoChain, NoTxs> {
        FluentApp::new(
            sample_order(Digest(B256::ZERO), 0),
            executor,
            hook,
            NoChain,
            Arc::new(NoTxs),
            Address::ZERO,
            30_000_000,
            // Tests anchor at activation (genesis.height == activation == 0),
            // so the pre-activation window is unchanged by the anchor/activation split.
            0,
            None,
            test_group_keys(),
        )
    }

    type DrainRx = Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<executor::Message>>>;

    fn fresh_mailbox() -> (executor::Mailbox, DrainRx) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (
            executor::Mailbox::new_for_test(tx),
            Arc::new(Mutex::new(rx)),
        )
    }

    /// A `FluentApp` over the GIVEN group-key map + 3-state resolver — the
    /// group-key-ladder fixtures (P1).
    fn build_app_with_keys(
        group_keys: GroupKeys,
        group_key_for: GroupKeyFor,
    ) -> FluentApp<NoChain, NoTxs> {
        let (mailbox, _rx) = fresh_mailbox();
        let bv = BeaconVerify::new(
            Arc::new(|_| None),
            Arc::new(|_| None),
            group_key_for,
            Vec::new(),
            0,
            10,
        );
        FluentApp::new(
            sample_order(Digest(B256::ZERO), 0),
            mailbox,
            Arc::new(|_b: OrderBlock| {}),
            NoChain,
            Arc::new(NoTxs),
            Address::ZERO,
            30_000_000,
            0,
            None,
            group_keys,
        )
        .with_beacon(bv)
    }

    /// A real `PK_epoch` value for the group-key fixtures.
    fn sample_group_public() -> GroupPublic {
        use commonware_cryptography::bls12381::dkg::deal_anonymous;
        use commonware_utils::{test_rng, N3f1, NZU32};
        let mut rng = test_rng();
        let (sharing, _shares) = deal_anonymous::<
            commonware_cryptography::bls12381::primitives::variant::MinSig,
            N3f1,
        >(&mut rng, Default::default(), NZU32!(4));
        *sharing.public()
    }

    /// A second, distinct group key (a fresh deal over the same rng stream).
    fn other_group_public() -> GroupPublic {
        use commonware_cryptography::bls12381::dkg::deal_anonymous;
        use commonware_utils::{test_rng, N3f1, NZU32};
        let mut rng = test_rng();
        let (_first, _) = deal_anonymous::<
            commonware_cryptography::bls12381::primitives::variant::MinSig,
            N3f1,
        >(&mut rng, Default::default(), NZU32!(4));
        let (sharing, _shares) = deal_anonymous::<
            commonware_cryptography::bls12381::primitives::variant::MinSig,
            N3f1,
        >(&mut rng, Default::default(), NZU32!(4));
        *sharing.public()
    }

    // ─────────────── group-key conflict policy (attested-source-wins) ───────

    /// The soak-2026-07-14 inversion, fixed: a stale LOCAL W1 write landed
    /// first, then the network's W4 observed-outcome key arrived and was
    /// DROPPED by first-write-wins — the poisoned entry then failed the next
    /// epoch's parent-seed witness. Attested-source-wins: the observed
    /// (agreed-chain-data) write must DISPLACE the differing local one.
    #[test]
    fn observed_outcome_displaces_a_differing_local_write() {
        let (local, network) = (sample_group_public(), other_group_public());
        assert_ne!(local, network);
        let map = test_group_keys();
        insert_group_key(&map, 77, local, KeySource::LocalDkg); // W1 (stale)
        insert_group_key(&map, 77, network, KeySource::ObservedOutcome); // W4
        assert_eq!(
            map.read().unwrap().get(&77),
            Some(&(network, KeySource::ObservedOutcome)),
            "agreed chain data must beat local reconstruction regardless of timing"
        );
        assert_eq!(attested_group_key(&map, 77), Some(network));
    }

    /// The inverse ordering: once an observed key holds the entry, no later
    /// local resolve may displace it (last-write-wins would re-open the
    /// boundary-forge arm the insert-only map exists to close).
    #[test]
    fn local_write_never_displaces_an_attested_entry() {
        let (local, network) = (sample_group_public(), other_group_public());
        let map = test_group_keys();
        insert_group_key(&map, 77, network, KeySource::ObservedOutcome);
        insert_group_key(&map, 77, local, KeySource::LocalDkg);
        assert_eq!(
            map.read().unwrap().get(&77),
            Some(&(network, KeySource::ObservedOutcome))
        );
    }

    /// Same value, stronger provenance: an observed confirm UPGRADES a local
    /// entry to attested (the promote value-gate's input); within a tier the
    /// first write wins and the map stays insert-only.
    #[test]
    fn same_value_reinsert_upgrades_provenance_only() {
        let pk = sample_group_public();
        let map = test_group_keys();
        insert_group_key(&map, 9, pk, KeySource::LocalDkg);
        assert_eq!(
            attested_group_key(&map, 9),
            None,
            "a local-only entry is NOT network-attested"
        );
        insert_group_key(&map, 9, pk, KeySource::ObservedOutcome);
        assert_eq!(attested_group_key(&map, 9), Some(pk));
        // And it never downgrades back.
        insert_group_key(&map, 9, pk, KeySource::LocalDkg);
        assert_eq!(attested_group_key(&map, 9), Some(pk));
    }

    /// The promote value-gate compares ONLY against network-attested entries:
    /// a differing local entry (our own earlier write — possibly the same
    /// stale source) must not masquerade as a network observation.
    #[test]
    fn attested_group_key_is_blind_to_local_entries() {
        let pk = sample_group_public();
        let map = test_group_keys();
        insert_group_key(&map, 5, pk, KeySource::LocalDkg);
        assert_eq!(attested_group_key(&map, 5), None);
        assert_eq!(attested_group_key(&map, 6), None);
    }

    /// (P1-a) — the sticky-`None` regression, at the resolver/map level: a
    /// TRANSIENT `committee_for` outage must produce `ReadFailed`, cache
    /// NOTHING (no entry of any kind — a one-shot-at-spawn resolution would
    /// make it sticky for the whole epoch, and staking reads fail correlated
    /// across validators ⇒ `f+1` in the accept-arm set ⇒ the boundary forge
    /// arm), and RESOLVE on a later call once the outage clears — memoizing
    /// only then.
    #[test]
    fn a_transient_committee_read_failure_does_not_become_a_sticky_epoch_wide_none() {
        use std::sync::atomic::{AtomicBool, AtomicU32};

        let pk = sample_group_public();
        let calls = Arc::new(AtomicU32::new(0));
        let outage = Arc::new(AtomicBool::new(true));
        let (c, o) = (calls.clone(), outage.clone());
        let resolver: GroupKeyFor = Arc::new(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
            if o.load(Ordering::SeqCst) {
                KeyLookup::ReadFailed
            } else {
                KeyLookup::Resolved(pk)
            }
        });
        let group_keys = test_group_keys();
        let app = build_app_with_keys(group_keys.clone(), resolver);

        // During the outage: ReadFailed, and the failure is NOT cached.
        assert_eq!(app.group_public_for(7), KeyLookup::ReadFailed);
        assert!(
            group_keys.read().unwrap().get(&7).is_none(),
            "a failure must never be inserted into the map"
        );

        // Outage clears ⇒ the SAME call path resolves (nothing negative was
        // memoized) and the success is cached.
        outage.store(false, Ordering::SeqCst);
        assert_eq!(app.group_public_for(7), KeyLookup::Resolved(pk));
        assert_eq!(
            group_keys.read().unwrap().get(&7),
            Some(&(pk, KeySource::LocalDkg))
        );

        // Subsequent reads hit the map — the resolver is not consulted again.
        let before = calls.load(Ordering::SeqCst);
        assert_eq!(app.group_public_for(7), KeyLookup::Resolved(pk));
        assert_eq!(calls.load(Ordering::SeqCst), before, "map hit is I/O-free");
    }

    /// `Unknown` is a stable fact and is NEVER cached either — a later call
    /// re-runs the resolve and can succeed (the DKG store is written
    /// asynchronously; W4 can land the key from a block at any time).
    #[test]
    fn unknown_is_not_cached_and_can_become_resolved() {
        use std::sync::atomic::AtomicBool;

        let pk = sample_group_public();
        let has_material = Arc::new(AtomicBool::new(false));
        let h = has_material.clone();
        let resolver: GroupKeyFor = Arc::new(move |_| {
            if h.load(Ordering::SeqCst) {
                KeyLookup::Resolved(pk)
            } else {
                KeyLookup::Unknown
            }
        });
        let group_keys = test_group_keys();
        let app = build_app_with_keys(group_keys.clone(), resolver);

        assert_eq!(app.group_public_for(9), KeyLookup::Unknown);
        assert!(group_keys.read().unwrap().is_empty());

        has_material.store(true, Ordering::SeqCst);
        assert_eq!(app.group_public_for(9), KeyLookup::Resolved(pk));
        assert_eq!(
            group_keys.read().unwrap().get(&9),
            Some(&(pk, KeySource::LocalDkg))
        );
    }

    /// W1/W2 — a map entry written at engine spawn is read with ZERO resolver
    /// calls: a continuing member performs no `committee_for` read at the next
    /// boundary, so a correlated staking-read outage cannot move it into the
    /// accept-arm set.
    #[test]
    fn a_pre_populated_map_entry_never_touches_the_resolver() {
        let pk = sample_group_public();
        let resolver: GroupKeyFor = Arc::new(move |_| {
            panic!("the resolver must not run on a map hit");
        });
        let group_keys = test_group_keys();
        insert_group_key(&group_keys, 4, pk, KeySource::LocalDkg); // as W1 does, before the engine
        let app = build_app_with_keys(group_keys, resolver);

        assert_eq!(app.group_public_for(4), KeyLookup::Resolved(pk));
    }

    // ───────────────────────────── parent-seed witness (§3) ─────────────────

    use commonware_cryptography::bls12381::primitives::sharing::Sharing;
    use commonware_cryptography::bls12381::primitives::variant::MinSig;
    use fluentbase_bls::beacon::{recover_seed, seed_namespace, sign_seed_partial};
    use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};

    /// Sum of a counter's values across matching keys (name + optional label).
    fn counter_at(snap: &Snapshotter, name: &str, label: Option<(&str, &str)>) -> u64 {
        snap.snapshot()
            .into_vec()
            .into_iter()
            .filter(|(k, ..)| {
                let key = k.key();
                key.name() == name
                    && label.is_none_or(|(lk, lv)| {
                        key.labels().any(|l| l.key() == lk && l.value() == lv)
                    })
            })
            .map(|(.., v)| match v {
                DebugValue::Counter(c) => c,
                _ => 0,
            })
            .sum()
    }

    /// A real threshold-beacon key: group key + shares to mint genuine seeds.
    struct WitnessCrypto {
        sharing: Sharing<MinSig>,
        shares: Vec<Share>,
        pk: GroupPublic,
        ns: Vec<u8>,
    }

    fn witness_crypto(seed: u64) -> WitnessCrypto {
        use commonware_cryptography::bls12381::dkg::deal_anonymous;
        use commonware_utils::{N3f1, NZU32};
        use rand_08::rngs::StdRng;
        use rand_core::SeedableRng as _;
        let mut rng = StdRng::seed_from_u64(seed);
        let (sharing, shares) =
            deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(4));
        WitnessCrypto {
            pk: *sharing.public(),
            ns: seed_namespace(b"fluent-test"),
            sharing,
            shares,
        }
    }

    impl WitnessCrypto {
        /// A GENUINE recovered threshold seed of `round` under THIS group key.
        fn seed_at(&self, round: Round) -> Seed {
            use commonware_utils::N3f1;
            let partials: Vec<_> = self
                .shares
                .iter()
                .map(|s| sign_seed_partial(s, &self.ns, round))
                .collect();
            Seed {
                target_round: round,
                signature: recover_seed::<N3f1>(&self.sharing, &partials).expect("recover"),
            }
        }
    }

    /// A verify-side app over the given executed chain + key plumbing, with
    /// the REAL seed namespace so `verify_seed` runs for real.
    fn witness_app<XC: ExecutedChain>(
        executed: XC,
        group_keys: GroupKeys,
        group_key_for: GroupKeyFor,
        ns: Vec<u8>,
    ) -> FluentApp<XC, NoTxs> {
        let (mailbox, _rx) = fresh_mailbox();
        FluentApp::new(
            sample_order(Digest(B256::ZERO), 0),
            mailbox,
            Arc::new(|_b: OrderBlock| {}),
            executed,
            Arc::new(NoTxs),
            Address::ZERO,
            30_000_000,
            0,
            None,
            group_keys,
        )
        .with_beacon(BeaconVerify::new(
            Arc::new(|_| None),
            Arc::new(|_| None),
            group_key_for,
            ns,
            0,
            10,
        ))
    }

    /// Tiny-timestamp `(parent, block)` pair for the gate tests (the
    /// deterministic clock starts at 0; unix-scale sleeps hang it). Heights
    /// 1→2 sit in the pre-activation result window, so the result gate
    /// resolves on tick 0 unless a test injects its own chain.
    fn witness_pair(
        parent_view: u64,
        block_view: u64,
        seed: Option<Seed>,
    ) -> (OrderBlock, OrderBlock) {
        let parent = OrderBlock {
            proposal_view: parent_view,
            height: 1,
            timestamp: 1,
            ..sample_order(Digest(B256::ZERO), 1)
        };
        let block = OrderBlock {
            proposal_view: block_view,
            height: 2,
            timestamp: 2,
            parent_seed: seed,
            ..sample_order(parent.digest(), 2)
        };
        (parent, block)
    }

    fn ctx_same_epoch(
        ec: u64,
        view: u64,
        parent: &OrderBlock,
    ) -> SimplexContext<Digest, PublicKey> {
        SimplexContext {
            round: Round::new(Epoch::new(ec), View::new(view)),
            leader: Ed25519PrivateKey::from_seed(7).public_key(),
            parent: (View::new(parent.proposal_view), parent.digest()),
        }
    }

    fn ctx_boundary(ec: u64, view: u64, parent: &OrderBlock) -> SimplexContext<Digest, PublicKey> {
        SimplexContext {
            round: Round::new(Epoch::new(ec), View::new(view)),
            leader: Ed25519PrivateKey::from_seed(7).public_key(),
            // GENESIS_VIEW sentinel: the parent is the previous epoch's
            // terminal block.
            parent: (View::zero(), parent.digest()),
        }
    }

    /// Drive `verify_block` on the deterministic runtime (clock advanced past
    /// the tiny block timestamps first) and return (verdict, virtual elapsed).
    fn run_gate<XC: ExecutedChain>(
        app: FluentApp<XC, NoTxs>,
        ctx: SimplexContext<Digest, PublicKey>,
        block: OrderBlock,
        parent: OrderBlock,
    ) -> (bool, Duration) {
        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|rt| async move {
            rt.sleep(Duration::from_secs(3)).await;
            let t0 = rt.current();
            let verdict = app.verify_block(&rt, &ctx, &block, &parent).await;
            let elapsed = rt.current().duration_since(t0).expect("monotonic");
            (verdict, elapsed)
        })
    }

    fn resolved(pk: GroupPublic) -> GroupKeyFor {
        Arc::new(move |_| KeyLookup::Resolved(pk))
    }

    // §9 gate test 1: the honest common path — and it must burn ZERO budget
    // (both loop conditions resolve on tick 0, no sleep).
    #[test]
    fn same_epoch_valid_witness_verifies_true_with_zero_budget() {
        let fx = witness_crypto(1);
        let seed = fx.seed_at(Round::new(Epoch::new(5), View::new(4)));
        let (parent, block) = witness_pair(4, 9, Some(seed));
        let ctx = ctx_same_epoch(5, 9, &parent);
        let app = witness_app(NoChain, test_group_keys(), resolved(fx.pk), fx.ns.clone());
        let (verdict, elapsed) = run_gate(app, ctx, block, parent);
        assert!(verdict);
        assert_eq!(elapsed, Duration::ZERO, "common path must not sleep");
    }

    /// A committee map holding the key `ctx_same_epoch`/`ctx_boundary` name as
    /// the round leader, plus that leader's index in it.
    fn armed_committee() -> (Arc<BiMap<PeerPubkey, BlsPubkey>>, u8) {
        let keys: Vec<Ed25519PrivateKey> = [7u64, 101, 102, 103]
            .into_iter()
            .map(Ed25519PrivateKey::from_seed)
            .collect();
        let bimap = committee_bimap(&keys, 42);
        let idx = bimap
            .position(&Ed25519PrivateKey::from_seed(7).public_key())
            .expect("the round leader is seated") as u8;
        (Arc::new(bimap), idx)
    }

    // The production-record rule threaded through the REAL `verify_block`
    // wiring, not the two helpers in isolation. Every other gate test builds an
    // app with NO committee map, so `expected_leader_index` returns `Ok(None)`
    // and the rule short-circuits `true` before it can reject anything — the
    // wiring that carries `ctx.leader` into the rule is what these three pin.
    #[test]
    fn armed_voter_accepts_a_record_naming_the_round_leader() {
        let fx = witness_crypto(1);
        let seed = fx.seed_at(Round::new(Epoch::new(5), View::new(4)));
        let (parent, mut block) = witness_pair(4, 9, Some(seed));
        let (bimap, idx) = armed_committee();
        block.extra_data = extra_data::encode_production_record(idx).into();
        let ctx = ctx_same_epoch(5, 9, &parent);
        let app = witness_app(NoChain, test_group_keys(), resolved(fx.pk), fx.ns.clone())
            .with_committee_index(bimap);
        assert!(run_gate(app, ctx, block, parent).0);
    }

    /// Both halves of the `Some(i)` arm at the gate: a record naming SOMEONE
    /// ELSE, and the empty field the executor is separately required to
    /// tolerate at the activation height. A voter must reject both.
    #[test]
    fn armed_voter_rejects_a_record_that_does_not_name_the_round_leader() {
        let (bimap, idx) = armed_committee();
        for (label, field) in [
            (
                "names another member",
                extra_data::encode_production_record(idx + 1),
            ),
            ("empty", Vec::new()),
        ] {
            let fx = witness_crypto(1);
            let seed = fx.seed_at(Round::new(Epoch::new(5), View::new(4)));
            let (parent, mut block) = witness_pair(4, 9, Some(seed));
            block.extra_data = field.into();
            let ctx = ctx_same_epoch(5, 9, &parent);
            let app = witness_app(NoChain, test_group_keys(), resolved(fx.pk), fx.ns.clone())
                .with_committee_index(bimap.clone());
            assert!(
                !run_gate(app, ctx, block, parent).0,
                "an armed voter must reject a production record that is {label}"
            );
        }
    }

    /// The `Err` arm is a REJECT, never a skip: an armed voter whose map does
    /// not seat the round leader votes false and says why, rather than falling
    /// through to the permissive no-map path.
    #[test]
    fn armed_voter_rejects_a_round_leader_outside_its_committee() {
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        let verdict = metrics::with_local_recorder(&recorder, || {
            let fx = witness_crypto(1);
            let seed = fx.seed_at(Round::new(Epoch::new(5), View::new(4)));
            let (parent, mut block) = witness_pair(4, 9, Some(seed));
            // Seeds 9000.. — disjoint from the seed-7 leader `ctx_same_epoch` names.
            let (_, disjoint) = test_committee(4, 9);
            block.extra_data = extra_data::encode_production_record(0).into();
            let ctx = ctx_same_epoch(5, 9, &parent);
            let app = witness_app(NoChain, test_group_keys(), resolved(fx.pk), fx.ns.clone())
                .with_committee_index(Arc::new(disjoint));
            run_gate(app, ctx, block, parent).0
        });
        assert!(!verdict);
        assert_eq!(
            counter_at(
                &snap,
                "dpos_marker_reject_total",
                Some(("reason", "leader_index"))
            ),
            1
        );
    }

    // §9 gate test 2: the downgrade attack — a beacon-active link with NO
    // witness must be voted false unconditionally (there is no live no-beacon
    // fallback; F8).
    #[test]
    fn missing_witness_on_beacon_active_link_is_rejected() {
        let fx = witness_crypto(1);
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        let verdict = metrics::with_local_recorder(&recorder, || {
            let (parent, block) = witness_pair(4, 9, None);
            let ctx = ctx_same_epoch(5, 9, &parent);
            let app = witness_app(NoChain, test_group_keys(), resolved(fx.pk), fx.ns.clone());
            run_gate(app, ctx, block, parent).0
        });
        assert!(!verdict);
        assert_eq!(
            counter_at(
                &snap,
                "dpos_parent_seed_reject_total",
                Some(("reason", "missing"))
            ),
            1
        );
    }

    // §9 gate test 3 + (N1-b): rule PIN — a witness naming ANY round but
    // `Round::new(Ep, parent.proposal_view)` is rejected, even when the seed
    // is a GENUINE threshold seed of that other round (crypto alone cannot
    // gate this: `verify_seed`'s message domain is the round alone). Exactly
    // one row of the table — the pinned round — is accepted. This is the
    // grinding surface: every pre-V0 round of the epoch has a genuine,
    // publicly-fetchable seed.
    #[test]
    fn witness_naming_any_round_but_the_pinned_one_is_rejected() {
        let fx = witness_crypto(1);
        let v0 = 50u64;
        for (k_epoch, k_view) in [
            (4u64, 45u64), // wrong epoch + wrong view
            (5, 45),       // v0 − 5
            (5, 49),       // v0 − 1
            (5, 50),       // v0 — THE pinned round
            (5, 51),       // v0 + 1
            (5, 57),       // v0 + 7
            (4, 50),       // right view, wrong epoch
        ] {
            let seed = fx.seed_at(Round::new(Epoch::new(k_epoch), View::new(k_view)));
            let (parent, block) = witness_pair(v0, 60, Some(seed));
            let ctx = ctx_same_epoch(5, 60, &parent);
            let app = witness_app(NoChain, test_group_keys(), resolved(fx.pk), fx.ns.clone());
            let (verdict, _) = run_gate(app, ctx, block, parent);
            assert_eq!(
                verdict,
                (k_epoch, k_view) == (5, v0),
                "only the pinned round (5, {v0}) may pass; got true for ({k_epoch}, {k_view})"
            );
        }
    }

    // §9 gate tests 5 + 6: the boundary link — the pin is
    // `Round::new(Ec − 1, L.proposal_view)`; a genuine witness passes, a
    // FORGED seed (valid signature under a DIFFERENT group key, correct
    // pinned round) fails the signature arm.
    #[test]
    fn boundary_witness_genuine_passes_forged_fails() {
        let fx = witness_crypto(1);
        let forger = witness_crypto(0xBAD);
        let pinned = Round::new(Epoch::new(4), View::new(7));

        let (parent, block) = witness_pair(7, 3, Some(fx.seed_at(pinned)));
        let ctx = ctx_boundary(5, 3, &parent);
        let app = witness_app(NoChain, test_group_keys(), resolved(fx.pk), fx.ns.clone());
        assert!(run_gate(app, ctx, block, parent).0);

        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        let verdict = metrics::with_local_recorder(&recorder, || {
            let (parent, block) = witness_pair(7, 3, Some(forger.seed_at(pinned)));
            let ctx = ctx_boundary(5, 3, &parent);
            let app = witness_app(NoChain, test_group_keys(), resolved(fx.pk), fx.ns.clone());
            run_gate(app, ctx, block, parent).0
        });
        assert!(!verdict, "a forged boundary seed must be voted false");
        assert_eq!(
            counter_at(
                &snap,
                "dpos_parent_seed_reject_total",
                Some(("reason", "bad_signature"))
            ),
            1
        );
    }

    // §9 gate test 7: an unresolvable PK_Ep is ACCEPT-biased (+metric) — but
    // the PIN is key-free and still rejects a wrong round even then.
    #[test]
    fn boundary_pk_unresolvable_accepts_but_the_pin_still_rejects() {
        let fx = witness_crypto(1);
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        let (ok, wrong_round) = metrics::with_local_recorder(&recorder, || {
            let pinned = Round::new(Epoch::new(4), View::new(7));
            let (parent, block) = witness_pair(7, 3, Some(fx.seed_at(pinned)));
            let ctx = ctx_boundary(5, 3, &parent);
            let app = witness_app(NoChain, test_group_keys(), no_key_lookup(), fx.ns.clone());
            let ok = run_gate(app, ctx, block, parent).0;

            let spin = Round::new(Epoch::new(4), View::new(37));
            let (parent, block) = witness_pair(7, 3, Some(fx.seed_at(spin)));
            let ctx = ctx_boundary(5, 3, &parent);
            let app = witness_app(NoChain, test_group_keys(), no_key_lookup(), fx.ns.clone());
            let wrong_round = run_gate(app, ctx, block, parent).0;
            (ok, wrong_round)
        });
        assert!(ok, "accept-biased on a structurally-unknown key");
        assert!(
            !wrong_round,
            "the PIN is key-free — wrong round rejects anyway"
        );
        assert_eq!(
            counter_at(
                &snap,
                "dpos_parent_seed_boundary_unverified_total",
                Some(("reason", "unknown"))
            ),
            1,
            "only the honest-round accept counts as unverified; the pin reject never reaches the key arm"
        );
    }

    // §9 gate test 8 / (N1-c): rule SA — a block lying about its own proposal
    // view is rejected (with everything else valid). Without SA, PIN pins
    // nothing.
    #[test]
    fn a_block_lying_about_its_own_proposal_view_is_rejected() {
        let fx = witness_crypto(1);
        let seed = fx.seed_at(Round::new(Epoch::new(5), View::new(4)));
        let (parent, block) = witness_pair(4, 8 /* lies: certified view is 9 */, Some(seed));
        let ctx = ctx_same_epoch(5, 9, &parent);
        let app = witness_app(NoChain, test_group_keys(), resolved(fx.pk), fx.ns.clone());
        assert!(!run_gate(app, ctx, block, parent).0);
    }

    // §9 gate test 9: pre-bootstrap links (Ep < DETERMINISTIC_BOOTSTRAP_EPOCH)
    // MUST carry None — a present witness is unagreeable smuggled data.
    #[test]
    fn pre_bootstrap_link_requires_absent_witness() {
        let fx = witness_crypto(1);
        let seed = fx.seed_at(Round::new(Epoch::new(1), View::new(4)));
        let (parent, block) = witness_pair(4, 9, Some(seed));
        let ctx = ctx_same_epoch(1, 9, &parent);
        let app = witness_app(NoChain, test_group_keys(), resolved(fx.pk), fx.ns.clone());
        assert!(!run_gate(app, ctx, block, parent).0, "Some ⇒ false");

        let (parent, block) = witness_pair(4, 9, None);
        let ctx = ctx_same_epoch(1, 9, &parent);
        let app = witness_app(NoChain, test_group_keys(), resolved(fx.pk), fx.ns.clone());
        assert!(run_gate(app, ctx, block, parent).0, "None ⇒ true");
    }

    // §9 gate test 10: the anchor link — Ec == 0 with the GENESIS_VIEW parent
    // sentinel is the chain anchor (never proposed, proposal_view == 0): the
    // witness is not required and the first post-activation block verifies.
    #[test]
    fn anchor_link_verifies_without_a_witness() {
        let fx = witness_crypto(1);
        let (parent, block) = witness_pair(0, 1, None);
        let ctx = ctx_boundary(0, 1, &parent);
        let app = witness_app(NoChain, test_group_keys(), resolved(fx.pk), fx.ns.clone());
        assert!(run_gate(app, ctx, block, parent).0);
    }

    // (P4) the free second-source tripwire: on a same-epoch link the parent's
    // self-attested view must agree with the simplex context's parent view —
    // a mismatch is a block certified at a view it did not claim. The witness
    // itself is CORRECTLY pinned to parent.proposal_view, so PIN alone would
    // pass this fixture; the test exists to prove the tripwire is wired.
    #[test]
    fn parent_proposal_view_disagreeing_with_ctx_parent_view_is_rejected() {
        let fx = witness_crypto(1);
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        let verdict = metrics::with_local_recorder(&recorder, || {
            let seed = fx.seed_at(Round::new(Epoch::new(5), View::new(4)));
            let (parent, block) = witness_pair(4, 9, Some(seed));
            let mut ctx = ctx_same_epoch(5, 9, &parent);
            ctx.parent.0 = View::new(5); // simplex says the parent certified at 5
            let app = witness_app(NoChain, test_group_keys(), resolved(fx.pk), fx.ns.clone());
            run_gate(app, ctx, block, parent).0
        });
        assert!(!verdict);
        assert_eq!(
            counter_at(&snap, "dpos_parent_view_mismatch_total", None),
            1
        );
    }

    /// A real committee DKG (players → (outcome, shares)) for the ladder-step-1
    /// fixtures: the group key is what `group_key_resolver` resolves and what
    /// the minted seeds verify under.
    struct CommitteeCrypto {
        outcome: CeremonyOutput,
        shares: Vec<Share>,
        ns: Vec<u8>,
    }

    fn committee_crypto() -> CommitteeCrypto {
        use commonware_cryptography::bls12381::{dkg::deal, primitives::sharing::Mode};
        use commonware_cryptography::ed25519::PrivateKey as Ed25519PrivateKey;
        use commonware_math::algebra::Random as _;
        use commonware_utils::N3f1;
        use rand_08::rngs::StdRng;
        use rand_core::SeedableRng as _;
        let mut rng = StdRng::seed_from_u64(0xC0117);
        let players: Set<PeerPubkey> =
            Set::from_iter_dedup((0..4).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()));
        let (outcome, share_map) =
            deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players.clone())
                .expect("deal");
        let shares: Vec<Share> = players
            .iter()
            .map(|p| share_map.get_value(p).expect("share").clone())
            .collect();
        CommitteeCrypto {
            ns: seed_namespace(b"fluent-test"),
            outcome,
            shares,
        }
    }

    impl CommitteeCrypto {
        fn seed_at(&self, round: Round) -> Seed {
            use commonware_utils::N3f1;
            let partials: Vec<_> = self
                .shares
                .iter()
                .map(|s| sign_seed_partial(s, &self.ns, round))
                .collect();
            Seed {
                target_round: round,
                signature: recover_seed::<N3f1>(self.outcome.public(), &partials).expect("recover"),
            }
        }

        /// The node's `CeremonyStore` as a restart reload leaves it: material
        /// keyed at the LAST CHANGE EPOCH, not the current one.
        fn store_keyed_at(&self, change_epoch: u64) -> crate::beacon::actor::CeremonyStore {
            Arc::new(RwLock::new(BTreeMap::from([(
                change_epoch,
                (self.outcome.clone(), self.shares[0].clone()),
            )])))
        }
    }

    /// (R1) — the rolling-restart chain-halt regression, END TO END: a
    /// restarted signer of a STABLE committee (material keyed at `Ec − 9`,
    /// observed-outcome cursor EMPTY, marshal walk exhausted — neither is
    /// modelled because neither is consulted) must vote **true** on an honest
    /// same-epoch block, resolving `PK_Ec` via ladder step 1 (its own DKG
    /// material). Under the pre-R1 spec (cursor + walk only) `group_public_for`
    /// is not `Resolved`, and while the accept-biased arm still votes true, the
    /// signature is never actually verified — the assertion with teeth is the
    /// `ladder="dkg"` source counter plus the forged-seed reject below. A
    /// `false` here is a permanent, self-sustaining halt after a rolling
    /// restart: the only event that repopulates the cursor is an outcome block
    /// the halted chain can never produce.
    #[test]
    fn restarted_signer_on_stable_committee_resolves_pk_and_votes_true() {
        let cc = committee_crypto();
        let ec = 12u64;
        let store = cc.store_keyed_at(ec - 9);
        // The chain minted at the change epoch (bit set) and never re-minted.
        let mint = ec - 9;
        let dkg_qual: crate::beacon::carry::DkgQualFor = Arc::new(move |e| Some(e == mint));
        let resolver = crate::dpos::group_key_resolver(
            store,
            dkg_qual,
            Arc::new(RwLock::new(BTreeMap::new())),
        );

        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        let (honest, forged) = metrics::with_local_recorder(&recorder, || {
            let pinned = Round::new(Epoch::new(ec), View::new(4));
            let (parent, block) = witness_pair(4, 9, Some(cc.seed_at(pinned)));
            let ctx = ctx_same_epoch(ec, 9, &parent);
            let group_keys = test_group_keys(); // the empty post-restart map
            let app = witness_app(NoChain, group_keys, resolver.clone(), cc.ns.clone());
            let honest = run_gate(app, ctx, block, parent).0;

            // The same restarted node must still REJECT a forged seed — i.e.
            // the key truly resolved and the crypto ran (not fall-through).
            let forger = witness_crypto(0xBAD);
            let (parent, block) = witness_pair(4, 9, Some(forger.seed_at(pinned)));
            let ctx = ctx_same_epoch(ec, 9, &parent);
            let app = witness_app(NoChain, test_group_keys(), resolver.clone(), cc.ns.clone());
            let forged = run_gate(app, ctx, block, parent).0;
            (honest, forged)
        });
        assert!(
            honest,
            "R1: a restarted stable-committee signer must vote true"
        );
        assert!(!forged, "the resolved key must actually verify signatures");
        assert!(
            counter_at(
                &snap,
                "dpos_group_public_source_total",
                Some(("ladder", "dkg"))
            ) >= 1,
            "the resolution must come from ladder step 1 (own DKG material)"
        );
        assert_eq!(
            counter_at(&snap, "dpos_parent_seed_boundary_unverified_total", None),
            0,
            "verified for real — not accepted by the unverified fall-through"
        );
    }

    /// (P1-a, closing the Phase-2-deferred clauses) — the transient
    /// committee-read outage at a BOUNDARY, end to end on the vote path:
    /// accept-biased while the outage lasts (`reason="read_failed"`, never
    /// `"unknown"`), nothing cached, and once the outage clears the SAME
    /// fixture verifies for real (no unverified increment; a forged seed now
    /// votes false).
    #[test]
    fn a_committee_read_outage_is_accept_biased_then_verifies_after_recovery() {
        use std::sync::atomic::AtomicBool;

        let cc = committee_crypto();
        let ec = 12u64;
        let store = cc.store_keyed_at(3);
        let outage = Arc::new(AtomicBool::new(true));
        let o = outage.clone();
        // During the outage every dkgQual read fails; after it clears, the
        // frozen history reads: minted at 3, never re-minted.
        let dkg_qual: crate::beacon::carry::DkgQualFor = Arc::new(move |e| {
            if o.load(Ordering::SeqCst) {
                None
            } else {
                Some(e == 3)
            }
        });
        let resolver = crate::dpos::group_key_resolver(
            store,
            dkg_qual,
            Arc::new(RwLock::new(BTreeMap::new())),
        );
        let pinned = Round::new(Epoch::new(ec - 1), View::new(7));
        let group_keys = test_group_keys();

        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            // 1. During the outage: the vote is TRUE (accept-biased), counted
            //    as read_failed — NOT unknown — and nothing is cached.
            let (parent, block) = witness_pair(7, 3, Some(cc.seed_at(pinned)));
            let ctx = ctx_boundary(ec, 3, &parent);
            let app = witness_app(NoChain, group_keys.clone(), resolver.clone(), cc.ns.clone());
            let (verdict, elapsed) = run_gate(app, ctx, block, parent);
            assert!(verdict, "accept-biased during the outage");
            assert_eq!(
                elapsed, VERIFY_EXEC_BUDGET,
                "a ReadFailed re-polls the whole shared budget"
            );
            assert_eq!(
                counter_at(
                    &snap,
                    "dpos_parent_seed_boundary_unverified_total",
                    Some(("reason", "read_failed"))
                ),
                1
            );
            assert_eq!(
                counter_at(
                    &snap,
                    "dpos_parent_seed_boundary_unverified_total",
                    Some(("reason", "unknown"))
                ),
                0,
                "the two non-Resolved states must be distinguishable"
            );
            assert!(
                group_keys.read().unwrap().is_empty(),
                "no negative-cache entry of any kind"
            );

            // 2. Outage clears ⇒ the key RESOLVES (nothing negative was
            //    memoized), the vote is true BY VERIFICATION, the map fills.
            outage.store(false, Ordering::SeqCst);
            let before = counter_at(&snap, "dpos_parent_seed_boundary_unverified_total", None);
            let (parent, block) = witness_pair(7, 3, Some(cc.seed_at(pinned)));
            let ctx = ctx_boundary(ec, 3, &parent);
            let app = witness_app(NoChain, group_keys.clone(), resolver.clone(), cc.ns.clone());
            assert!(run_gate(app, ctx, block, parent).0);
            assert_eq!(
                counter_at(&snap, "dpos_parent_seed_boundary_unverified_total", None),
                before,
                "verified for real — the unverified counter must not move"
            );
            assert!(group_keys.read().unwrap().contains_key(&(ec - 1)));

            // 3. …and a FORGED seed on the recovered fixture votes FALSE —
            //    under a one-shot/sticky resolution it would still be accepted
            //    for the whole epoch.
            let forger = witness_crypto(0xBAD);
            let (parent, block) = witness_pair(7, 3, Some(forger.seed_at(pinned)));
            let ctx = ctx_boundary(ec, 3, &parent);
            let app = witness_app(NoChain, group_keys.clone(), resolver, cc.ns.clone());
            assert!(!run_gate(app, ctx, block, parent).0);
        });
    }

    // (Fix 2) a PK_E read failure that clears WITHIN the verify budget
    // resolves and VERIFIES (no unverified fall-through, map populated,
    // bounded by the shared budget) — and `Unknown` does NOT re-poll (zero
    // budget burned).
    #[test]
    fn a_read_failure_that_clears_within_the_budget_resolves_and_verifies() {
        use std::sync::atomic::AtomicU32;

        let cc = committee_crypto();
        let ec = 12u64;
        let store = cc.store_keyed_at(3);
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        // Fails the first 8 polls (200 ms of the 25 ms-tick budget), then
        // recovers — well inside VERIFY_EXEC_BUDGET.
        let dkg_qual: crate::beacon::carry::DkgQualFor = Arc::new(move |e| {
            if c.fetch_add(1, Ordering::SeqCst) < 8 {
                None
            } else {
                Some(e == 3)
            }
        });
        let resolver = crate::dpos::group_key_resolver(
            store,
            dkg_qual,
            Arc::new(RwLock::new(BTreeMap::new())),
        );
        let pinned = Round::new(Epoch::new(ec - 1), View::new(7));
        let group_keys = test_group_keys();

        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            let (parent, block) = witness_pair(7, 3, Some(cc.seed_at(pinned)));
            let ctx = ctx_boundary(ec, 3, &parent);
            let app = witness_app(NoChain, group_keys.clone(), resolver.clone(), cc.ns.clone());
            let (verdict, elapsed) = run_gate(app, ctx, block, parent);
            assert!(verdict);
            assert_eq!(
                elapsed,
                VERIFY_EXEC_POLL * 8,
                "the re-poll stops the instant the read recovers"
            );
            assert_eq!(
                counter_at(&snap, "dpos_parent_seed_boundary_unverified_total", None),
                0,
                "resolved WITHIN the budget ⇒ verified, not accepted-by-fall-through"
            );
            assert!(group_keys.read().unwrap().contains_key(&(ec - 1)));
        });

        // `Unknown` (a churn-in member with no material at all) resolves on
        // tick 0: zero budget, no sleep — this is why the 3-state lookup
        // exists; collapsing Unknown into ReadFailed would spend the whole
        // verify budget on every boundary block for every churned-in member.
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            let empty_store: crate::beacon::actor::CeremonyStore =
                Arc::new(RwLock::new(BTreeMap::new()));
            let resolver = crate::dpos::group_key_resolver(
                empty_store,
                Arc::new(move |e| Some(e == 3)),
                Arc::new(RwLock::new(BTreeMap::new())),
            );
            let (parent, block) = witness_pair(7, 3, Some(cc.seed_at(pinned)));
            let ctx = ctx_boundary(ec, 3, &parent);
            let app = witness_app(NoChain, test_group_keys(), resolver, cc.ns.clone());
            let (verdict, elapsed) = run_gate(app, ctx, block, parent);
            assert!(verdict);
            assert_eq!(elapsed, Duration::ZERO, "Unknown must burn zero budget");
        });
        assert_eq!(
            counter_at(
                &snap,
                "dpos_parent_seed_boundary_unverified_total",
                Some(("reason", "unknown"))
            ),
            1
        );
    }

    /// An executed chain whose hash becomes available after N `executed_hash`
    /// polls — models "execution reaches h − K mid-verify".
    #[derive(Clone)]
    struct TickChain {
        calls: Arc<std::sync::atomic::AtomicU32>,
        ready_after: u32,
        hash: B256,
    }
    impl ExecutedChain for TickChain {
        fn executed_tip(&self) -> u64 {
            0
        }
        fn spec_executed_hash(&self, _height: u64) -> Option<B256> {
            let served = self.calls.fetch_add(1, Ordering::SeqCst);
            (served >= self.ready_after).then_some(self.hash)
        }
        // Test double for the result-gate poll loop: tier-F resolves on the
        // same N-tick schedule as the spec read (the gate only ever samples
        // this tier; the split is exercised structurally elsewhere).
        fn finalized_executed_hash(&self, height: u64) -> Option<B256> {
            self.spec_executed_hash(height)
        }
    }

    /// A post-activation-window `(parent, block)` pair whose result gate needs
    /// `executed_hash(height − K)` — the fixture for the interleaved loop.
    fn result_gated_pair(seed: Option<Seed>, result: B256) -> (OrderBlock, OrderBlock) {
        let parent = OrderBlock {
            proposal_view: 7, // matches the (ec − 1, 7) pinned round the tests mint
            height: 2,
            timestamp: 1,
            ..sample_order(Digest(B256::ZERO), 2)
        };
        let block = OrderBlock {
            proposal_view: 3, // == the ctx_boundary(_, 3, _) view the tests use (rule SA)
            height: 3,        // == activation(0) + K ⇒ result target = executed_hash(0)
            timestamp: 2,
            parent_seed: seed,
            result,
            ..sample_order(parent.digest(), 3)
        };
        (parent, block)
    }

    /// (Fix B) — the INTERLEAVED-loop regression suite: the key arm and the
    /// result gate share ONE budget and are evaluated together every tick, so
    /// neither can starve the other. Fails under sequential polling (key arm
    /// first, result gate with whatever remains).
    #[test]
    fn a_pk_repoll_that_burns_most_of_the_budget_still_lets_the_result_gate_resolve() {
        use std::sync::atomic::AtomicU32;

        let cc = committee_crypto();
        let ec = 12u64;
        let pinned = Round::new(Epoch::new(ec - 1), View::new(7));
        let exec_hash = B256::repeat_byte(0x5E);

        // Both conditions slow AND correlated (the real-world shape): the
        // committee read fails for 36 of the 40 ticks; execution reaches
        // h − K at tick 28.
        let make_resolver = |fail_polls: u32| {
            let store = cc.store_keyed_at(3);
            let calls = Arc::new(AtomicU32::new(0));
            let dkg_qual: crate::beacon::carry::DkgQualFor = Arc::new(move |e| {
                if calls.fetch_add(1, Ordering::SeqCst) < fail_polls {
                    None
                } else {
                    Some(e == 3)
                }
            });
            crate::dpos::group_key_resolver(store, dkg_qual, Arc::new(RwLock::new(BTreeMap::new())))
        };

        // Variant 1: both recover inside the budget ⇒ true, VERIFIED (no
        // unverified), one shared budget, and the result gate was polled every
        // tick (call count ≈ 29, not the ≤ 2 a sequential key loop leaves it).
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            let chain = TickChain {
                calls: Arc::new(AtomicU32::new(0)),
                ready_after: 28,
                hash: exec_hash,
            };
            let group_keys = test_group_keys();
            let (parent, block) = result_gated_pair(Some(cc.seed_at(pinned)), exec_hash);
            let ctx = ctx_boundary(ec, 3, &parent);
            let app = witness_app(
                chain.clone(),
                group_keys.clone(),
                make_resolver(36),
                cc.ns.clone(),
            );
            let (verdict, elapsed) = run_gate(app, ctx, block, parent);
            assert!(verdict, "both gates must resolve inside ONE budget");
            assert!(
                elapsed <= VERIFY_EXEC_BUDGET,
                "the budget is shared, not additive: {elapsed:?}"
            );
            assert!(
                chain.calls.load(Ordering::SeqCst) >= 25,
                "the result gate must be polled ~every tick, not once: {}",
                chain.calls.load(Ordering::SeqCst)
            );
            assert!(group_keys.read().unwrap().contains_key(&(ec - 1)));
        });
        assert_eq!(
            counter_at(&snap, "dpos_parent_seed_boundary_unverified_total", None),
            0,
            "the key was VERIFIED, not accepted by fall-through"
        );

        // Variant 2 (starvation — the clause that flips the vote): the key
        // read NEVER recovers; execution arrives at tick 28. The vote must
        // still be TRUE (read_failed once, result satisfied). Under
        // sequential polling the key loop consumes all 40 ticks and the
        // result gate's single shot votes false on an honest block.
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            let chain = TickChain {
                calls: Arc::new(AtomicU32::new(0)),
                ready_after: 28,
                hash: exec_hash,
            };
            let (parent, block) = result_gated_pair(Some(cc.seed_at(pinned)), exec_hash);
            let ctx = ctx_boundary(ec, 3, &parent);
            let app = witness_app(
                chain,
                test_group_keys(),
                make_resolver(u32::MAX),
                cc.ns.clone(),
            );
            let (verdict, _) = run_gate(app, ctx, block, parent);
            assert!(
                verdict,
                "an honest block must not be voted false by starvation"
            );
        });
        assert_eq!(
            counter_at(
                &snap,
                "dpos_parent_seed_boundary_unverified_total",
                Some(("reason", "read_failed"))
            ),
            1
        );

        // Variant 3 (the mirror): result-gate exhaustion still votes FALSE —
        // the EL-backpressure rule is unchanged; the interleave must not
        // weaken it. Key resolves on tick 0.
        {
            let chain = TickChain {
                calls: Arc::new(AtomicU32::new(0)),
                ready_after: u32::MAX,
                hash: exec_hash,
            };
            let (parent, block) = result_gated_pair(Some(cc.seed_at(pinned)), exec_hash);
            let ctx = ctx_boundary(ec, 3, &parent);
            let app = witness_app(chain, test_group_keys(), make_resolver(0), cc.ns.clone());
            let (verdict, elapsed) = run_gate(app, ctx, block, parent);
            assert!(!verdict, "execution never reached h − K ⇒ vote false");
            assert_eq!(elapsed, VERIFY_EXEC_BUDGET);
        }

        // Variant 4 (zero-cost common path): both available at tick 0 ⇒ true
        // without the virtual clock advancing at all.
        {
            let chain = TickChain {
                calls: Arc::new(AtomicU32::new(0)),
                ready_after: 0,
                hash: exec_hash,
            };
            let (parent, block) = result_gated_pair(Some(cc.seed_at(pinned)), exec_hash);
            let ctx = ctx_boundary(ec, 3, &parent);
            let app = witness_app(chain, test_group_keys(), make_resolver(0), cc.ns.clone());
            let (verdict, elapsed) = run_gate(app, ctx, block, parent);
            assert!(verdict);
            assert_eq!(elapsed, Duration::ZERO, "no sleep on the common path");
        }
    }

    // ───────────────────────────── propose side (§3) ────────────────────────

    fn propose_app(store: SeedStore) -> FluentApp<NoChain, NoTxs> {
        let (mailbox, _rx) = fresh_mailbox();
        FluentApp::new(
            sample_order(Digest(B256::ZERO), 0),
            mailbox,
            Arc::new(|_b: OrderBlock| {}),
            NoChain,
            Arc::new(NoTxs),
            Address::ZERO,
            30_000_000,
            0,
            Some(store),
            test_group_keys(),
        )
        .with_committee_index(propose_committee())
    }

    fn tiny_parent(proposal_view: u64) -> OrderBlock {
        OrderBlock {
            proposal_view,
            height: 1,
            timestamp: 1,
            ..sample_order(Digest(B256::ZERO), 1)
        }
    }

    fn propose_ctx(
        ec: u64,
        view: u64,
        parent_view: (u64, bool), // (view, boundary?)
        parent: &OrderBlock,
    ) -> SimplexContext<Digest, PublicKey> {
        SimplexContext {
            round: Round::new(Epoch::new(ec), View::new(view)),
            leader: Ed25519PrivateKey::from_seed(7).public_key(),
            parent: (
                if parent_view.1 {
                    View::zero()
                } else {
                    View::new(parent_view.0)
                },
                parent.digest(),
            ),
        }
    }

    /// The wire flip's propose half: a proposal carries EXACTLY the 2-byte record
    /// naming its own proposer, and that record is what its voters recompute from
    /// `ctx.leader`. Asserting the bytes (not just "some extra_data") is the point
    /// — the executor feeds `leader_index` straight to `recordProduction`, so a
    /// silently wrong index mis-credits production with no other symptom.
    #[test]
    fn proposal_stamps_the_production_record_naming_its_proposer() {
        let fx = witness_crypto(1);
        let pinned = Round::new(Epoch::new(5), View::new(4));
        let seed = fx.seed_at(pinned);
        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|rt| async move {
            let store = SeedStore::new();
            store.record(pinned, seed.signature);
            let app = propose_app(store);
            let parent = tiny_parent(4);
            let ctx = propose_ctx(5, 9, (4, false), &parent);
            let block = app
                .build_proposal(&rt, &ctx, parent)
                .await
                .expect("proposed");
            assert_eq!(
                block.extra_data.as_ref(),
                extra_data::encode_production_record(propose_leader_index()).as_slice(),
            );
            // And the proposer's own verifier accepts what it just built.
            assert!(
                production_record_ok(&block.extra_data, Some(propose_leader_index())),
                "a proposer must never build a block its own verify rule rejects"
            );
        });
    }

    /// A leader that cannot name itself in its committee SKIPS the view instead of
    /// proposing a block every honest voter would reject. Cheaper by one wasted
    /// leader deadline, and it keeps "every consensus block carries 2 valid bytes"
    /// true by construction rather than by convention.
    #[test]
    fn a_leader_outside_its_own_committee_declines_to_propose() {
        let fx = witness_crypto(1);
        let pinned = Round::new(Epoch::new(5), View::new(4));
        let seed = fx.seed_at(pinned);
        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|rt| async move {
            let store = SeedStore::new();
            store.record(pinned, seed.signature);
            // A committee that does NOT contain the fixture leader (`from_seed(7)`).
            let (_outsiders, disjoint) = test_committee(3, 99);
            let app = propose_app(store).with_committee_index(Arc::new(disjoint));
            let parent = tiny_parent(4);
            let ctx = propose_ctx(5, 9, (4, false), &parent);
            assert!(app.build_proposal(&rt, &ctx, parent).await.is_none());
        });
    }

    // §9 propose: every proposal self-attests its view (rule SA) and, on a
    // beacon-active same-epoch link, carries EXACTLY the stored seed of
    // `Round::new(Ec, parent.proposal_view)`.
    #[test]
    fn proposal_self_attests_its_view_and_embeds_the_stored_parent_seed() {
        let fx = witness_crypto(1);
        let pinned = Round::new(Epoch::new(5), View::new(4));
        let seed = fx.seed_at(pinned);
        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|rt| async move {
            let store = SeedStore::new();
            store.record(pinned, seed.signature);
            let app = propose_app(store);
            let parent = tiny_parent(4);
            let ctx = propose_ctx(5, 9, (4, false), &parent);
            let block = app
                .build_proposal(&rt, &ctx, parent)
                .await
                .expect("proposed");
            assert_eq!(block.proposal_view, 9, "rule SA: proposal_view == ctx view");
            assert_eq!(block.parent_seed, Some(seed), "the exact stored seed");
        });
    }

    // §9 propose: a SeedStore miss ⇒ the view is SKIPPED (None) — never an
    // invalid block, never a round fallback.
    #[test]
    fn seed_store_miss_skips_the_view_not_an_invalid_block() {
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            let runtime = commonware_runtime::deterministic::Runner::default();
            runtime.start(|rt| async move {
                let app = propose_app(SeedStore::new());
                let parent = tiny_parent(4);
                let ctx = propose_ctx(5, 9, (4, false), &parent);
                assert!(app.build_proposal(&rt, &ctx, parent).await.is_none());
            });
        });
        assert_eq!(
            counter_at(&snap, "dpos_parent_seed_lookup_miss_total", None),
            1
        );
    }

    // §9 propose (the R4 regression): at the boundary the named round comes
    // from the PARENT BLOCK — never from local cert state. The fixture's store
    // also holds a seed for a DIFFERENT (spin) round, as the local first-wins
    // finalization would name it; the marshal is None, so consulting a cert
    // for the round is structurally impossible (it would skip, failing the
    // assertion). Exactly `Round::new(Ec − 1, L.proposal_view)` is embedded.
    #[test]
    fn boundary_round_comes_from_the_parent_block_never_a_cert() {
        let fx = witness_crypto(1);
        let v0 = 7u64;
        let pinned = Round::new(Epoch::new(4), View::new(v0));
        let spin = Round::new(Epoch::new(4), View::new(v0 + 30));
        let genuine = fx.seed_at(pinned);
        let decoy = fx.seed_at(spin);
        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|rt| async move {
            let store = SeedStore::new();
            store.record(pinned, genuine.signature);
            store.record(spin, decoy.signature); // the local first-wins spin round
            let app = propose_app(store);
            let parent = tiny_parent(v0);
            let ctx = propose_ctx(5, 3, (0, true), &parent);
            let block = app
                .build_proposal(&rt, &ctx, parent)
                .await
                .expect("proposed");
            assert_eq!(
                block.parent_seed.as_ref().map(|s| s.target_round),
                Some(pinned),
                "the round must be pinned to the parent block, not the spin cert"
            );
        });
    }

    // §9 propose: boundary + a store that does NOT hold the pinned round ⇒
    // skip the view (+ the boundary-skip liveness metric).
    #[test]
    fn boundary_seed_store_miss_for_the_pinned_round_skips_the_view() {
        let fx = witness_crypto(1);
        let spin = Round::new(Epoch::new(4), View::new(37));
        let decoy = fx.seed_at(spin);
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            let runtime = commonware_runtime::deterministic::Runner::default();
            runtime.start(|rt| async move {
                let store = SeedStore::new();
                store.record(spin, decoy.signature); // only the spin round
                let app = propose_app(store);
                let parent = tiny_parent(7); // pin = (4, 7) — absent
                let ctx = propose_ctx(5, 3, (0, true), &parent);
                assert!(app.build_proposal(&rt, &ctx, parent).await.is_none());
            });
        });
        assert_eq!(
            counter_at(&snap, "dpos_parent_seed_boundary_skip_total", None),
            1
        );
    }

    // §9 propose: a pre-bootstrap link (Ep < 2) embeds NO witness and still
    // proposes — mirrors verify's `!required ⇒ None` arm.
    #[test]
    fn pre_bootstrap_propose_carries_no_witness() {
        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|rt| async move {
            let app = propose_app(SeedStore::new());
            let parent = tiny_parent(4);
            let ctx = propose_ctx(1, 9, (4, false), &parent);
            let block = app
                .build_proposal(&rt, &ctx, parent)
                .await
                .expect("proposed");
            assert_eq!(block.parent_seed, None);
            assert_eq!(block.proposal_view, 9);
        });
    }

    #[test]
    fn beacon_gate_epoch_type_and_share_on_poly() {
        use crate::beacon::dkg_oracle::run_local_dkg;
        use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
        use commonware_math::algebra::Random as _;
        use rand_08::rngs::StdRng;
        use rand_core::SeedableRng as _;

        let mut rng = StdRng::seed_from_u64(42);
        let keys: Vec<Ed25519PrivateKey> = (0..5)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let committee: Set<PeerPubkey> = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let (out, shares) = run_local_dkg(&mut rng, b"ns", 1, &keys, &keys).expect("dkg");
        // A different ceremony over the same committee ⇒ a forged poly for the
        // same PK_E slot whose constant misses our real share.
        let (out_forged, _) = run_local_dkg(&mut rng, b"ns", 2, &keys, &keys).expect("dkg forged");
        let my_share = shares.get(&keys[0].public_key()).expect("share").clone();
        // A different set for E-1 so epoch 1 reads as a CHANGE epoch.
        let prev_committee: Set<PeerPubkey> =
            Set::from_iter_dedup((0..5).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()));

        let make_bv = |share: Option<Share>, change: bool| {
            let cur = committee.clone();
            let prev = if change {
                prev_committee.clone()
            } else {
                committee.clone()
            };
            let committee_for: CommitteeFor = Arc::new(move |e: u64| match e {
                0 => Some(prev.clone()),
                _ => Some(cur.clone()),
            });
            let out_e = out.clone();
            let beacon_for_epoch: BeaconForEpoch = Arc::new(move |e: u64| {
                (e == 1)
                    .then(|| share.clone().map(|s| (out_e.clone(), s)))
                    .flatten()
            });
            BeaconVerify::new(
                beacon_for_epoch,
                committee_for,
                no_key_lookup(),
                Vec::new(),
                0,
                10,
            )
        };

        let block = |height: u64, oc: Option<Bytes>| {
            let mut b = sample_order(Digest(B256::ZERO), height);
            b.beacon_outcome = oc;
            b
        };
        let enc = |o: &CeremonyOutput| Bytes::from(encode_outcome(o));

        // (a) honest change block (height 10 = epoch_start(1)): C passes.
        let bv = make_bv(Some(my_share.clone()), true);
        assert!(beacon_gate_decision(Some(&bv), &block(10, Some(enc(&out)))));
        // (b) forged outcome: C fails for the honest share-holder.
        assert!(!beacon_gate_decision(
            Some(&bv),
            &block(10, Some(enc(&out_forged)))
        ));
        // (c) epoch-type: change block missing the outcome → reject.
        assert!(!beacon_gate_decision(Some(&bv), &block(10, None)));
        // (d) epoch-type: outcome on a non-first block of the epoch → reject.
        assert!(!beacon_gate_decision(
            Some(&bv),
            &block(11, Some(enc(&out)))
        ));
        // (f) observer (no share) on a change block → withhold.
        let bv_obs = make_bv(None, true);
        assert!(!beacon_gate_decision(
            Some(&bv_obs),
            &block(10, Some(enc(&out)))
        ));
        // (e) carry-forward (committee unchanged): no outcome expected.
        let bv_cf = make_bv(Some(my_share), false);
        assert!(beacon_gate_decision(Some(&bv_cf), &block(10, None)));
        assert!(!beacon_gate_decision(
            Some(&bv_cf),
            &block(10, Some(enc(&out)))
        ));
        // (g) no beacon context → no gating.
        assert!(beacon_gate_decision(None, &block(10, Some(enc(&out)))));

        // (h) dkg_logs idx bound, and the accept-bias that must NOT be "fixed".
        // With committee[E+1] readable an idx at n is out of range ⇒ vote false.
        let n = committee.len();
        let mut over = block(10, Some(enc(&out)));
        over.dkg_logs = vec![(n as u8, B256::repeat_byte(0x77))];
        assert!(
            !beacon_gate_decision(Some(&bv), &over),
            "idx == n is out of committee range ⇒ reject"
        );
        // The SAME block is accepted when the committee cannot be read. Voting false
        // there would make the vote depend on this node's EL-sync progress — the
        // prohibited local-state class. The codec bound and the ceremony's
        // deterministic skip are what cover this arm; see `beacon_gate_decision`.
        let unreadable: CommitteeFor = Arc::new(|_| None);
        let bv_unreadable = BeaconVerify::new(
            Arc::new(|_| None),
            unreadable,
            no_key_lookup(),
            Vec::new(),
            0,
            10,
        );
        let mut over_no_outcome = over.clone();
        over_no_outcome.beacon_outcome = None;
        assert!(
            beacon_gate_decision(Some(&bv_unreadable), &over_no_outcome),
            "committee unreadable ⇒ the idx check is skipped (accept-biased, deliberate)"
        );
    }

    /// AMENDMENT 5 (test d): propose includes the RECORDED E+1 dealer-log hashes minus
    /// those already FINALIZED in ancestors (first-finalized-wins dedup), canonically
    /// sorted by idx. `H_settle` height-gating freezes the finalized set.
    #[test]
    fn dkg_logs_propose_includes_recorded_minus_finalized() {
        use std::collections::BTreeMap;
        use std::sync::RwLock;
        let recorded: DkgLogIndex = Arc::new(RwLock::new(BTreeMap::new()));
        let finalized: DkgLogIndex = Arc::new(RwLock::new(BTreeMap::new()));
        let (h0, h1, h2) = (
            B256::repeat_byte(0x10),
            B256::repeat_byte(0x11),
            B256::repeat_byte(0x12),
        );
        // interval=40, activation=0: a block at height 5 is epoch 0 ⇒ carries E+1 = 1.
        recorded
            .write()
            .unwrap()
            .insert(1, BTreeMap::from([(0u8, h0), (1u8, h1), (2u8, h2)]));
        let bv = BeaconVerify::new(
            Arc::new(|_| None),
            Arc::new(|_| None),
            no_key_lookup(),
            Vec::new(),
            0,
            40,
        )
        .with_dkg_logs(recorded, finalized.clone());

        // Nothing finalized yet ⇒ all three recorded entries are included, sorted.
        assert_eq!(
            bv.dkg_logs_to_include(5),
            vec![(0, h0), (1, h1), (2, h2)],
            "all recorded logs ride when nothing is finalized"
        );
        // Finalize idx 0 (same hash) in an ancestor ⇒ deduped out; 1 and 2 remain.
        finalized
            .write()
            .unwrap()
            .insert(1, BTreeMap::from([(0u8, h0)]));
        assert_eq!(
            bv.dkg_logs_to_include(5),
            vec![(1, h1), (2, h2)],
            "a finalized idx (same hash) is deduped from the propose set"
        );
    }

    /// A change-epoch BOUNDARY block reserves the beacon-outcome framing (bug 1):
    /// `assemble` is called with `TX_BYTE_BUDGET_AT_BOUNDARY` at the boundary and
    /// `TX_BYTE_BUDGET` otherwise, so a boundary block carrying `beacon_outcome`
    /// still fits `MAX_ORDER_BLOCK_SIZE` / the p2p frame cap.
    #[test]
    fn boundary_propose_reserves_beacon_framing_in_the_tx_budget() {
        use crate::beacon::dkg_oracle::run_local_dkg;
        use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
        use commonware_math::algebra::Random as _;
        use rand_08::rngs::StdRng;
        use rand_core::SeedableRng as _;

        #[derive(Clone, Default)]
        struct RecordingAssembler(Arc<Mutex<Vec<usize>>>);
        impl OrderingAssembler for RecordingAssembler {
            fn assemble(&self, _h: u64, _g: u64, budget: usize) -> Vec<TransactionSigned> {
                self.0.lock().unwrap().push(budget);
                Vec::new()
            }
            fn observe_finalized(&self, _b: &OrderBlock) {}
        }
        #[derive(Clone)]
        struct ResolvedChain;
        impl ExecutedChain for ResolvedChain {
            fn executed_tip(&self) -> u64 {
                1000
            }
            fn spec_executed_hash(&self, h: u64) -> Option<B256> {
                Some(B256::repeat_byte(h as u8))
            }
            // Test double: every height resolves in both tiers.
            fn finalized_executed_hash(&self, h: u64) -> Option<B256> {
                self.spec_executed_hash(h)
            }
        }

        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let mut rng = StdRng::seed_from_u64(99);
            let keys: Vec<Ed25519PrivateKey> = (0..5)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee: Set<PeerPubkey> =
                Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            let prev: Set<PeerPubkey> = Set::from_iter_dedup(
                (0..5).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()),
            );
            let (out, shares) = run_local_dkg(&mut rng, b"ns", 1, &keys, &keys).expect("dkg");
            let my_share = shares.get(&keys[0].public_key()).expect("share").clone();

            // interval 10, activation 0: epoch_start(1) = 10 is a CHANGE-epoch first
            // block (committee[0] = prev ≠ committee[1] = committee).
            let committee_for: CommitteeFor = Arc::new(move |e: u64| match e {
                0 => Some(prev.clone()),
                _ => Some(committee.clone()),
            });
            let beacon_for_epoch: BeaconForEpoch =
                Arc::new(move |e: u64| (e == 1).then(|| (out.clone(), my_share.clone())));
            let bv = BeaconVerify::new(
                beacon_for_epoch,
                committee_for,
                no_key_lookup(),
                Vec::new(),
                0,
                10,
            );

            let budgets = Arc::new(Mutex::new(Vec::new()));
            let (mailbox, _rx) = fresh_mailbox();
            let app = FluentApp::new(
                sample_order(Digest(B256::ZERO), 0),
                mailbox,
                Arc::new(|_b: OrderBlock| {}),
                ResolvedChain,
                Arc::new(RecordingAssembler(budgets.clone())),
                Address::ZERO,
                30_000_000,
                0,
                None,
                test_group_keys(),
            )
            .with_beacon(bv)
            .with_committee_index(propose_committee());

            // Propose the change-epoch first block (height 10) → reduced budget.
            app.build_proposal(
                &ctx,
                &sample_context(1),
                sample_order(Digest(B256::ZERO), 9),
            )
            .await
            .expect("boundary proposal");
            assert_eq!(
                *budgets.lock().unwrap().last().unwrap(),
                TX_BYTE_BUDGET_AT_BOUNDARY,
                "a boundary block must assemble against the reduced (beacon-reserved) budget"
            );

            // Propose a non-boundary block (height 11) → full budget.
            app.build_proposal(
                &ctx,
                &sample_context(1),
                sample_order(Digest(B256::ZERO), 10),
            )
            .await
            .expect("non-boundary proposal");
            assert_eq!(
                *budgets.lock().unwrap().last().unwrap(),
                TX_BYTE_BUDGET,
                "a non-boundary block uses the full tx budget"
            );
        });
    }

    /// Deterministic epoch-2 bootstrap: committee[2]'s first block is a change
    /// boundary (asserts an outcome + runs the C gate) EVEN ON A STABLE committee,
    /// while epoch 1 stays seedless (no outcome). interval=10, activation=0 ⇒
    /// epoch_start(1)=10, epoch_start(2)=20.
    #[test]
    fn epoch_two_bootstrap_is_change_boundary_on_stable_committee() {
        use crate::beacon::dkg_oracle::run_local_dkg;
        use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
        use commonware_math::algebra::Random as _;
        use rand_08::rngs::StdRng;
        use rand_core::SeedableRng as _;

        let mut rng = StdRng::seed_from_u64(7);
        let keys: Vec<Ed25519PrivateKey> = (0..5)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        // STABLE committee: identical for every epoch (so on-change activation
        // would NEVER fire; only the deterministic epoch-2 bootstrap does).
        let committee: Set<PeerPubkey> = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let (out, shares) = run_local_dkg(&mut rng, b"ns", 2, &keys, &keys).expect("dkg");
        let my_share = shares.get(&keys[0].public_key()).expect("share").clone();

        let committee_for: CommitteeFor = {
            let c = committee.clone();
            Arc::new(move |_e: u64| Some(c.clone()))
        };
        let out_e = out.clone();
        let beacon_for_epoch: BeaconForEpoch = Arc::new(move |e: u64| {
            (e == DETERMINISTIC_BOOTSTRAP_EPOCH).then(|| (out_e.clone(), my_share.clone()))
        });
        let bv = BeaconVerify::new(
            beacon_for_epoch,
            committee_for,
            no_key_lookup(),
            Vec::new(),
            0,
            10,
        );

        let block = |height: u64, oc: Option<Bytes>| {
            let mut b = sample_order(Digest(B256::ZERO), height);
            b.beacon_outcome = oc;
            b
        };
        let enc = |o: &CeremonyOutput| Bytes::from(encode_outcome(o));

        assert!(bv.is_change_epoch_first_block(20, 2));
        // Epoch-2 first block: outcome required + C share-on-poly passes.
        assert!(beacon_gate_decision(Some(&bv), &block(20, Some(enc(&out)))));
        // Epoch-2 first block missing the outcome → reject (epoch-type gate).
        assert!(!beacon_gate_decision(Some(&bv), &block(20, None)));
        // Epoch 1 (seedless) on the same stable committee: NOT a change boundary —
        // no outcome expected; an asserted outcome is rejected.
        assert!(!bv.is_change_epoch_first_block(10, 1));
        assert!(beacon_gate_decision(Some(&bv), &block(10, None)));
        assert!(!beacon_gate_decision(
            Some(&bv),
            &block(10, Some(enc(&out)))
        ));
    }

    #[test]
    fn gas_limit_bound_is_strict_1_1024() {
        let parent = 30_000_000u64;
        let delta = parent / 1024;
        assert!(gas_limit_within_1_1024(parent, parent));
        assert!(gas_limit_within_1_1024(parent, parent + delta - 1));
        assert!(gas_limit_within_1_1024(parent, parent - delta + 1));
        assert!(!gas_limit_within_1_1024(parent, parent + delta));
        assert!(!gas_limit_within_1_1024(parent, parent - delta));
        assert!(!gas_limit_within_1_1024(parent, MIN_GAS_LIMIT - 1));
    }

    #[test]
    fn step_gas_limit_converges_within_bound() {
        let parent = 30_000_000u64;
        // Every step must satisfy the verify bound, in both directions.
        let up = step_gas_limit(parent, 50_000_000);
        assert!(gas_limit_within_1_1024(parent, up) && up > parent);
        let down = step_gas_limit(parent, 10_000_000);
        assert!(gas_limit_within_1_1024(parent, down) && down < parent);
        assert_eq!(step_gas_limit(parent, parent), parent);
        // Converges exactly when the target is within one step.
        assert_eq!(step_gas_limit(parent, parent + 5), parent + 5);
    }

    // Pacing tests use single-digit timestamps: the deterministic runtime
    // advances virtual time in 1ms cycles (deterministic.rs `Config::cycle`),
    // so a sleep to a realistic unix-seconds target never completes.
    fn tiny_ts_parent() -> OrderBlock {
        OrderBlock {
            timestamp: 5,
            ..sample_order(Digest(B256::ZERO), 0)
        }
    }

    #[test]
    fn propose_paces_to_parent_plus_one_second() {
        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (mailbox, _rx) = fresh_mailbox();
            let app = build_app(mailbox, Arc::new(|_b: OrderBlock| {}))
                .with_committee_index(propose_committee());
            let parent = tiny_ts_parent();

            // Clock at the parent's timestamp (synchronized proposer): the
            // pace sleep must carry it to parent+1 and the timestamp lands
            // exactly there.
            ctx.sleep_until(std::time::UNIX_EPOCH + Duration::from_secs(parent.timestamp))
                .await;
            let block = app
                .build_proposal(&ctx, &sample_context(1), parent.clone())
                .await
                .expect("proposed");
            assert_eq!(block.timestamp, parent.timestamp + 1);
            let now = ctx
                .current()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            assert!(now > parent.timestamp, "clock advanced by the pace sleep");
        });
    }

    #[test]
    fn pace_sleep_is_capped_for_a_future_dated_parent() {
        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (mailbox, _rx) = fresh_mailbox();
            let app = build_app(mailbox, Arc::new(|_b: OrderBlock| {}))
                .with_committee_index(propose_committee());
            let parent = tiny_ts_parent();

            // Proposer clock lags the parent's timestamp (skew within the
            // verify tolerance): the sleep must cap at one BLOCK_INTERVAL
            // from now — never parent+1 — or the peers' leader deadline
            // (which budgets pace ≤ BLOCK_INTERVAL) would expire first.
            let start = ctx.current();
            let block = app
                .build_proposal(&ctx, &sample_context(1), parent.clone())
                .await
                .expect("proposed");
            let slept = ctx.current().duration_since(start).unwrap();
            assert!(
                slept <= BLOCK_INTERVAL,
                "pace sleep must be capped at BLOCK_INTERVAL under clock skew, slept {slept:?}"
            );
            // The CONTENT timestamp still extends the parent chain.
            assert_eq!(block.timestamp, parent.timestamp + 1);
        });
    }

    #[test]
    fn propose_does_not_pace_when_past_target() {
        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (mailbox, _rx) = fresh_mailbox();
            let app = build_app(mailbox, Arc::new(|_b: OrderBlock| {}))
                .with_committee_index(propose_committee());
            let parent = tiny_ts_parent();

            // A late proposer (slow/nullified prior views) is already past
            // parent+1: no extra sleep, timestamp = now.
            let late = parent.timestamp + 10;
            ctx.sleep_until(std::time::UNIX_EPOCH + Duration::from_secs(late))
                .await;
            let block = app
                .build_proposal(&ctx, &sample_context(1), parent)
                .await
                .expect("proposed");
            assert_eq!(block.timestamp, late);
        });
    }

    // Item C: a CHANGE-epoch boundary leader with no live-DKG outcome for the epoch
    // declines to propose IMMEDIATELY — before the 1s pace sleep — so the voter
    // fast-Nullifies to the next (share-holding) leader.
    #[test]
    fn boundary_leader_without_outcome_declines_propose() {
        use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
        use commonware_math::algebra::Random as _;
        use rand_08::rngs::StdRng;
        use rand_core::SeedableRng as _;

        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let mut rng = StdRng::seed_from_u64(7);
            let k0: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let k1: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let c0: Set<PeerPubkey> = Set::from_iter_dedup(k0.iter().map(|k| k.public_key()));
            let c1: Set<PeerPubkey> = Set::from_iter_dedup(k1.iter().map(|k| k.public_key()));
            // c0 != c1 ⇒ epoch 1's first block is a CHANGE-epoch boundary.
            let committee_for: CommitteeFor = Arc::new(move |e: u64| match e {
                0 => Some(c0.clone()),
                _ => Some(c1.clone()),
            });
            // This node ran no live DKG ⇒ no CeremonyStore entry for any epoch.
            let beacon_for_epoch: BeaconForEpoch = Arc::new(|_e| None);
            // activation=0, interval=10 ⇒ epoch_start(1)=10, so proposed height 10
            // (parent 9 + 1) is the change-epoch first block.
            let bv = BeaconVerify::new(
                beacon_for_epoch,
                committee_for,
                no_key_lookup(),
                Vec::new(),
                0,
                10,
            );

            let (mailbox, _rx) = fresh_mailbox();
            let app = build_app(mailbox, Arc::new(|_b: OrderBlock| {}))
                .with_beacon(bv)
                .with_committee_index(propose_committee());

            let parent = sample_order(Digest(B256::ZERO), 9);
            let start = ctx.current();
            let decision = app.build_proposal(&ctx, &sample_context(1), parent).await;
            assert!(
                decision.is_none(),
                "boundary leader without epoch-E DKG outcome must decline"
            );
            assert!(
                ctx.current().duration_since(start).unwrap() < BLOCK_INTERVAL,
                "must decline BEFORE the pace sleep (fast view-change)"
            );
        });
    }

    /// Every leg runs with the record rule ARMED (`Some`), which is what a voter
    /// always passes: `expected_leader_index` returns `Ok(None)` only for an
    /// instance that casts no vote.
    #[test]
    fn structural_checks_reject_each_violation() {
        const LEADER: u8 = 3;
        let parent = sample_order(Digest(B256::ZERO), 1);
        let good = OrderBlock {
            parent: parent.digest(),
            extra_data: Bytes::from(extra_data::encode_production_record(LEADER)),
            ..sample_order(parent.digest(), 2)
        };
        let now = good.timestamp;
        let check = |b: &OrderBlock| {
            FluentApp::<NoChain, NoTxs>::structural_checks(
                b,
                &parent,
                now,
                sample_context(0).round,
                Some(LEADER),
            )
        };
        assert!(check(&good));

        assert!(!check(&OrderBlock {
            timestamp: parent.timestamp,
            ..good.clone()
        }));

        assert!(!check(&OrderBlock {
            gas_limit: parent.gas_limit * 2,
            ..good.clone()
        }));

        // Wrong length, and — the leg the bitmap format could never carry — a
        // well-formed record naming SOMEONE ELSE as the producer.
        assert!(!check(&OrderBlock {
            extra_data: Bytes::from(vec![0xFF; 3]),
            ..good.clone()
        }));
        assert!(!check(&OrderBlock {
            extra_data: Bytes::from(extra_data::encode_production_record(LEADER + 1)),
            ..good.clone()
        }));
        assert!(!check(&OrderBlock {
            extra_data: Bytes::new(),
            ..good.clone()
        }));
    }

    #[test]
    fn structural_checks_enforce_future_bound() {
        let parent = sample_order(Digest(B256::ZERO), 1);
        let good = OrderBlock {
            parent: parent.digest(),
            ..sample_order(parent.digest(), 2)
        };

        // At the tolerance boundary: a proposer one second ahead of this
        // verifier's clock is still honest (truncation + NTP skew).
        let now = good.timestamp - TIMESTAMP_FUTURE_TOLERANCE_SECS;
        assert!(FluentApp::<NoChain, NoTxs>::structural_checks(
            &good,
            &parent,
            now,
            sample_context(0).round,
            None
        ));

        // One second past the boundary: rejected.
        assert!(!FluentApp::<NoChain, NoTxs>::structural_checks(
            &good,
            &parent,
            now - 1,
            sample_context(0).round,
            None
        ));
    }

    #[test]
    fn report_block_sends_finalize_fires_hook_and_advances_height() {
        use commonware_utils::{acknowledgement::Exact, Acknowledgement as _};
        use std::sync::atomic::AtomicUsize;

        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (mailbox, rx) = fresh_mailbox();
            let counter = Arc::new(AtomicUsize::new(0));
            let c = counter.clone();
            let mut app = build_app(
                mailbox,
                Arc::new(move |_b: OrderBlock| {
                    c.fetch_add(1, Ordering::SeqCst);
                }),
            );

            let block = sample_order(Digest(B256::ZERO), 42);
            let (ack, _waiter) = Exact::handle();
            <FluentApp<NoChain, NoTxs> as Reporter>::report(
                &mut app,
                Update::Block(block.clone(), ack),
            )
            .await;

            assert_eq!(counter.load(Ordering::SeqCst), 1, "hook fired once");
            let msg = rx.lock().unwrap().try_recv().expect("Finalize sent");
            match msg.command {
                executor::Command::Finalize(update) => match *update {
                    Update::Block(b, _ack) => assert_eq!(b.digest(), block.digest()),
                    _ => panic!("expected Update::Block"),
                },
                executor::Command::SpecNotarized(_) => {
                    panic!("FluentApp never emits SpecNotarized")
                }
            }
        });
    }

    #[test]
    fn report_tip_skips_hook_but_forwards() {
        use commonware_consensus::types::{Epoch, View};
        use std::sync::atomic::AtomicUsize;

        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (mailbox, rx) = fresh_mailbox();
            let counter = Arc::new(AtomicUsize::new(0));
            let c = counter.clone();
            let mut app = build_app(
                mailbox,
                Arc::new(move |_b: OrderBlock| {
                    c.fetch_add(1, Ordering::SeqCst);
                }),
            );

            let round = Round::new(Epoch::new(0), View::new(0));
            <FluentApp<NoChain, NoTxs> as Reporter>::report(
                &mut app,
                Update::Tip(
                    round,
                    commonware_consensus::types::Height::new(0),
                    Digest(B256::ZERO),
                ),
            )
            .await;

            assert_eq!(
                counter.load(Ordering::SeqCst),
                0,
                "hook must NOT fire on Tip"
            );
            let msg = rx.lock().unwrap().try_recv().expect("Finalize sent");
            match msg.command {
                executor::Command::Finalize(update) => {
                    assert!(matches!(*update, Update::Tip(..)));
                }
                executor::Command::SpecNotarized(_) => {
                    panic!("FluentApp never emits SpecNotarized")
                }
            }
        });
    }
}
