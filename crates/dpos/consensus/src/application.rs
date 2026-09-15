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
    beacon::Seed,
    digest::Digest,
    executor, extra_data,
    fault::EngineError,
    order_block::{
        result_matches, result_target, OrderBlock, ResultTarget, MIN_GAS_LIMIT, TX_BYTE_BUDGET,
    },
    slasher::{evidence::verify_block_charge, ChargeStore, TombstoneSet},
};
use alloy_consensus::Transaction as _;
use alloy_primitives::{Bytes, B256};
use alloy_rpc_types_engine::{ForkchoiceState, ForkchoiceUpdated, PayloadStatus};
use commonware_codec::Encode as _;
use commonware_consensus::{
    marshal::{
        ancestry::{AncestorStream, BlockProvider},
        Update,
    },
    simplex::types::Context as SimplexContext,
    types::{Round, View},
    Application, Reporter, VerifyingApplication,
};
use commonware_cryptography::ed25519::PublicKey;
use commonware_runtime::{Clock, Metrics, Spawner};
use commonware_utils::ordered::BiMap;
/// The signing scheme bound for this Application.
pub use fluentbase_bls::Scheme as BlsScheme;
use fluentbase_bls::{BlsPubkey, PeerPubkey};
use futures::StreamExt as _;
use rand_08::Rng;
use reth_ethereum_primitives::{Block as RethBlock, TransactionSigned};
use reth_primitives_traits::SealedBlock;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
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
    /// (that would re-open the whole-committee SafetyHalt divergence below).
    fn spec_executed_hash(&self, height: u64) -> Option<B256>;

    /// Tier-F (FINALIZED): the finalized-execution result at `height`, or `None`
    /// if this node has not finalized-derived `height` yet. The result gate
    /// (propose + verify) samples THIS tier so a still-speculative sibling A at
    /// h−K can never be committed as an OrderBlock `result` and then re-finalize
    /// as sibling B — the honest-run whole-committee SafetyHalt.
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

    /// The cursor itself — the highest height known
    /// finalized-without-a-possible-sibling.
    ///
    /// Added for [`crate::committee`], which takes this height as its read
    /// ANCHOR rather than keeping a second ordering-finalized cursor of its own.
    /// The two are the same number: every site that raises the executor's
    /// `ordering_finalized` raises this cursor with the SAME value in the same
    /// arm — the `init` seed (`executor.rs:1085` / `:1149`, both
    /// `last_consensus_finalized_height`), the re-jump landing (`:2441` /
    /// `:2451`, both `landing_h`) and the finalized derive (`:3291` / `:3566`,
    /// both `height` inside one `try_derive`). Where the two can differ is
    /// strictly inside that last body: `ordering_finalized` is raised at `:3291`
    /// and the cursor only at `:3566`, so between them — and on the fault arms
    /// that return in between, all of which take the executor down — this cursor
    /// LAGS by at most one derive. Lagging is the safe direction for an anchor:
    /// it reads a committee at a strictly older finalized height, never at a
    /// height the node has not finalized.
    pub fn height(&self) -> u64 {
        self.cursor.load(Ordering::Acquire)
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
/// carry EXACTLY the [`extra_data::PRODUCTION_RECORD_LEN`]-byte record naming its
/// own leader. The record's accusation half is gated separately, against the
/// evidence, by [`equivocation_gate_decision`].
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
/// finalizing a block whose `extra_data` no reth header
/// (`FLUENT_MAXIMUM_EXTRA_DATA_SIZE`) can hold.
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

/// The Fluent consensus application.
///
/// Generic over `XC` (local derived-chain view) and `A` (tx assembler).
pub struct FluentApp<XC, A> {
    /// Per-epoch beacon-DKG verify/propose context (see [`BeaconVerify`]).
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
    /// Proposer-local field — it shapes only this node's OWN proposals
    /// (agreed data once embedded); verify never reads it.
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
    /// L2 chain id — the domain separator of the vote signatures a block-carried
    /// equivocation charge is verified under (`fluent_namespace`). A chain
    /// constant, so it belongs on the cross-epoch app rather than on the
    /// per-epoch committee injection.
    chain_id: u64,
    /// Read handle on the slasher's verified-charge queue, drained one charge
    /// per block by [`Self::build_proposal`]. `None` for an instance with no
    /// slasher (a follower, a test) — it proposes nothing, so it charges nobody.
    charges: Option<ChargeStore>,
    /// Committee members observed slashed for equivocation. Read at verify to
    /// refuse binding their proposals, and at propose to drop a charge whose
    /// verdict has already landed. Empty is the honest "nothing observed" state,
    /// which is also what a test and a node whose watcher has not run yet hold —
    /// so the default is permissive, like the absent committee map.
    tombstones: TombstoneSet,
    /// Ordering half of the clock pair, written in [`Reporter::report`]. This
    /// app is the right writer precisely because it is built ONCE per process
    /// and holds no execution state: it keeps reporting through a `SafetyHalt`
    /// park, an executor death and every per-epoch engine abort, so a frozen
    /// `dpos_dkg_clock_height` against a climbing `dpos_ordering_finalized_height`
    /// is readable from outside instead of being two silences.
    plane_clock: crate::sync_metrics::PlaneClock,
    /// The marshal's ordering tip — the height of the highest finalization this
    /// node has VERIFIED and stored — published from the `Update::Tip` arm of
    /// [`Reporter::report`], the one place the tip is delivered to this process.
    ///
    /// THE clock of the process, one writer and one PARKED consumer per channel:
    /// the epoch manager ([`crate::epoch_manager::Actor`], whose live epoch IS
    /// `epoch_of(tip)`) subscribes through [`Self::ordering_tip`] and parks on
    /// this one; the beacon's `DkgActor` (`beacon::ValidatorInputs::clock`)
    /// parks on [`Self::beacon_tip`], a second channel written in the same
    /// statement below. A watch, so a consumer can both READ the tip at a
    /// decision point and be WOKEN by it, and nothing is ever dropped.
    ///
    /// ONE PARKED RECEIVER PER CHANNEL IS LOAD-BEARING, and it is the whole
    /// reason the beacon's channel is separate rather than a second
    /// subscription to this one. `tokio::sync::watch` wakes its receivers
    /// through a `big_notify` of EIGHT `Notify` shards; a receiver picks its
    /// shard on every `changed()` call with the per-process thread-local RNG
    /// (`tokio-1.52.3/src/sync/watch.rs:422` → `runtime/context.rs:125` →
    /// `FastRand::new` ← `RandomState`), while `notify_waiters` walks the shards
    /// in index order. Two receivers parked on ONE channel are therefore woken
    /// in an order drawn from process-random state — which reorders the tasks in
    /// the ready queue of a seeded, otherwise fully deterministic run. Two
    /// channels, each with one parked receiver, are woken in the order this app
    /// writes them, and that order is code.
    ///
    /// `0` until the first tip: the marshal reports one at startup from its
    /// highest stored finalization (CW `marshal/core/actor.rs:397-402`), and a
    /// node with no stored finalization at all is genuinely at genesis, where
    /// `epoch_of(0)` — the pre-activation clamp — is the right answer.
    ///
    /// An `Arc<Sender>` rather than a `Sender` because this app is CLONED (the
    /// marshal's reporter half and the epoch manager's copy are the same app),
    /// and a `watch::Sender` is not `Clone`; every clone must publish into the
    /// one channel the consumers subscribed to.
    ordering_tip: Arc<tokio::sync::watch::Sender<u64>>,
    /// The beacon plane's own tip channel, when this node runs one: the plane
    /// builds it before the app exists, keeps the receiver for its `DkgActor`
    /// and hands the sender in ([`Self::with_beacon_tip`]). Written from the
    /// same `Update::Tip` arm as [`Self::ordering_tip`], immediately after it,
    /// so both consumers see every tip and neither can be woken ahead of the
    /// other by anything but this order. `None` on a node with no beacon plane
    /// (a follower, a test).
    beacon_tip: Option<Arc<tokio::sync::watch::Sender<u64>>>,
}

impl<XC: Clone, A> Clone for FluentApp<XC, A> {
    fn clone(&self) -> Self {
        Self {
            committee_index: self.committee_index.clone(),
            genesis: self.genesis.clone(),
            executor: self.executor.clone(),
            boundary_hook: self.boundary_hook.clone(),
            verify_gate_last_logged_height: self.verify_gate_last_logged_height.clone(),
            executed: self.executed.clone(),
            assembler: self.assembler.clone(),
            target_gas_limit: self.target_gas_limit,
            dpos_activation_block: self.dpos_activation_block,
            chain_id: self.chain_id,
            charges: self.charges.clone(),
            tombstones: self.tombstones.clone(),
            plane_clock: self.plane_clock.clone(),
            ordering_tip: self.ordering_tip.clone(),
            beacon_tip: self.beacon_tip.clone(),
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
        target_gas_limit: u64,
        dpos_activation_block: u64,
        chain_id: u64,
        // Same reasoning as `group_keys`: the same handle must also reach
        // `slasher::Config`, and a second store would be a queue nothing fills.
        charges: Option<ChargeStore>,
        // Same reasoning again: the writer is the node's tombstone watcher, in
        // another crate entirely, so a default here would be a set nothing fills.
        tombstones: TombstoneSet,
    ) -> Self {
        Self {
            committee_index: None,
            chain_id,
            charges,
            tombstones,
            // Observability-only, so a setter rather than a 15th constructor
            // argument: an instance that never receives one publishes nothing,
            // which is what a follower and every test should publish.
            plane_clock: crate::sync_metrics::PlaneClock::default(),
            // A channel of its own, so every app in a process — the marshal's
            // reporter half and the epoch manager's copy are clones of one
            // `FluentApp` — shares one by construction. This one is never
            // replaced: a node with a beacon plane ADDS the plane's channel
            // beside it (`with_beacon_tip`).
            ordering_tip: Arc::new(tokio::sync::watch::Sender::new(0)),
            // Set only on a node that runs a beacon plane.
            beacon_tip: None,
            genesis: Arc::new(genesis),
            executor,
            boundary_hook,
            // u64::MAX sentinel: no height has logged yet (0 is a valid height).
            verify_gate_last_logged_height: Arc::new(AtomicU64::new(u64::MAX)),
            executed,
            assembler,
            target_gas_limit,
            dpos_activation_block,
        }
    }

    /// Attach the registered clock pair this app writes the ordering half of.
    /// Only the launch site has one; everything else leaves the gauges silent.
    pub fn with_plane_clock(mut self, plane_clock: crate::sync_metrics::PlaneClock) -> Self {
        self.plane_clock = plane_clock;
        self
    }

    /// Publish the ordering tip on THIS watch as well as the app's own. The
    /// beacon plane is built before the app (`node/dpos.rs`,
    /// `testbed/stand.rs`) and its `DkgActor` holds a receiver of this very
    /// sender, so handing it in is what makes the actor's clock a channel this
    /// app writes rather than one nothing ever writes. Call it before the app is
    /// cloned — the builders do — or the reporter half keeps a `None` here and
    /// the actor's `changed()` never fires: no deal, no seal.
    ///
    /// A SECOND CHANNEL, not a second subscription to the app's own, and
    /// `Self::beacon_tip` says why: two receivers parked on one
    /// `tokio::sync::watch` are woken in a process-random order.
    pub fn with_beacon_tip(mut self, beacon_tip: Arc<tokio::sync::watch::Sender<u64>>) -> Self {
        self.beacon_tip = Some(beacon_tip);
        self
    }

    /// Subscribe to the marshal's ordering tip as this app publishes it.
    ///
    /// The receiver holds the LAST value, so a consumer built after a tip has
    /// already been reported reads that tip rather than the `0` seed — which is
    /// what makes this safe for a consumer (the epoch manager) that is started
    /// per promotion, long after the marshal's startup tip.
    pub fn ordering_tip(&self) -> tokio::sync::watch::Receiver<u64> {
        self.ordering_tip.subscribe()
    }

    /// Attach the epoch committee's pubkey→index map. Called from
    /// `EpochEngine::new`, which holds both the map and this app before moving
    /// the app into `Inline` — that adjacency is what makes the two the same
    /// agreed snapshot. Followers, verify-only schemes and tests leave it unset.
    pub fn with_committee_index(mut self, bimap: Arc<BiMap<PeerPubkey, BlsPubkey>>) -> Self {
        self.committee_index = Some(bimap);
        self
    }

    /// Install the tombstone view a running node's watcher fills. Test-only: in
    /// production the handle is a constructor argument precisely so an instance
    /// cannot be built without the writer's own, and a second entry point would
    /// re-open that hole.
    #[cfg(test)]
    fn with_tombstones(mut self, tombstones: TombstoneSet) -> Self {
        self.tombstones = tombstones;
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
    /// supplies `proposal_view = ctx.round.view()` (rule SA).
    async fn build_proposal<E: Clock>(
        &self,
        clock: &E,
        context: &SimplexContext<Digest, PublicKey>,
        parent: OrderBlock,
    ) -> Option<OrderBlock> {
        let height = parent.height + 1;

        // The pace cap's origin, read at view entry rather than after the
        // sleep: `leader_timeout` is one block interval plus a 750 ms margin
        // (`timeouts.rs`), so anything this call spends before pacing must come
        // out of the leader's OWN interval, not be added to it.
        let view_entered = clock.current();

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
        let pace_cap = view_entered + BLOCK_INTERVAL;
        clock.sleep_until(pace_target.min(pace_cap)).await;

        // Execution gate (proposer-≤K-behind): the result commitment needs the
        // FINALIZED-tier derived hash at height − K — NOT the speculative head
        // (a still-speculative sibling A at h−K could re-finalize as sibling B,
        // committing a hash the network will diverge from).
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
        let txs = self.assembler.assemble(height, gas_limit, TX_BYTE_BUDGET);

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
        // At most ONE charge per block: each costs every voter two BLS verifies
        // that run OUTSIDE `VERIFY_EXEC_BUDGET`, out of the ~450 ms vote margin.
        //
        // The charge is drawn from THIS block's epoch and no other. Only that
        // epoch's committee can verify it (`committee_index` maps exactly one
        // epoch), so an older charge would produce a block every voter rejects
        // and `BTreeMap` ordering would keep re-offering that same key. A charge
        // that outlives its epoch leaves by the transaction fallback instead.
        //
        // The epoch is the ROUND's, not `epoch_of(height)`: the round is what
        // every voter's `committee_index` was built for, and it is agreed data
        // both sides of the vote read identically.
        //
        // If the block fails to gather a quorum nothing special happens — the
        // charge stays in the queue and the next proposer holding it offers it
        // again. No backoff, no retry counter, no timer.
        // A charge names a committee index; the tombstone is recorded against a
        // peer key. The round's own `committee_index` is the mapping between
        // them, and it is the right one by construction — `next_charge` is asked
        // only for this round's epoch. Without a map nothing is filtered, the
        // same permissive direction the leader-index rule takes.
        let charge = self.charges.as_ref().and_then(|store| {
            store.next_charge(context.round.epoch().get(), |accused| {
                self.committee_index
                    .as_ref()
                    .and_then(|bimap| bimap.get(accused as usize))
                    .is_some_and(|peer| self.tombstones.contains(peer))
            })
        });
        let (accused, equivocation) = match charge {
            Some((accused, evidence)) => {
                (Some(accused), Some(Bytes::from(evidence.encode().to_vec())))
            }
            None => (None, None),
        };
        if let Some(accused) = accused {
            tracing::info!(
                height,
                accused,
                "proposing an equivocation charge with its evidence"
            );
            metrics::counter!("dpos_equivocation_charge_proposed_total").increment(1);
        }
        let extra_data = Bytes::from(extra_data::encode_production_record(leader_index, accused));

        Some(OrderBlock {
            parent: parent.digest(),
            height,
            // Rule SA: self-attest the view this block is proposed in. It is
            // what makes `Round(epoch(h), proposal_view)` — the round every node
            // resolves this block's σ at — agreed data rather than a per-node
            // guess, and it is checked at THIS block's own vote below.
            proposal_view: context.round.view().get(),
            timestamp,
            gas_limit,
            extra_data,
            result,
            txs,
            equivocation,
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

/// Equivocation gate (returns `false` ⇒ vote against the block): the accusation
/// in `extra_data` and the evidence in `OrderBlock::equivocation` must be present
/// together or absent together, and when present the evidence must convict
/// exactly the accused member of exactly this block's epoch.
///
/// Presence is exact in BOTH directions on purpose. An accusation without
/// evidence is a slash nobody could check; evidence without an accusation is
/// unagreed payload riding under the digest, and both would let one Byzantine
/// proposer put something in a block that the committee did not attest to.
///
/// Every voter runs this against the block in front of it, so a charge is valid
/// or not on its own bytes — block validity never depends on whether the evidence
/// gossip reached this node in time. A false charge cannot pass, because honest
/// nodes hold no valid evidence for an innocent validator.
///
/// `committee_index == None` (a follower, a verify-only scheme, a test) skips
/// only the CRYPTOGRAPHIC arm — such an instance casts no vote, so a signature it
/// cannot check is not a vote condition, exactly as for the leader-index rule.
/// The presence rule needs no committee and is enforced regardless.
fn equivocation_gate_decision(
    block: &OrderBlock,
    epoch: u64,
    committee_index: Option<&Arc<BiMap<PeerPubkey, BlsPubkey>>>,
    chain_id: u64,
) -> bool {
    // An absent or undecodable record names nobody, so no evidence may ride with
    // it. `structural_checks` already rejected both for any instance that votes.
    let accused = match extra_data::decode_production_record(&block.extra_data) {
        Ok(Some(record)) => record.accused,
        Ok(None) | Err(_) => None,
    };
    let (accused, evidence) = match (accused, block.equivocation.as_ref()) {
        (None, None) => return true,
        (Some(accused), Some(evidence)) => (accused, evidence),
        (accused, evidence) => {
            tracing::warn!(
                height = block.height,
                epoch,
                has_accusation = accused.is_some(),
                has_evidence = evidence.is_some(),
                "equivocation gate: accusation and evidence must both be present \
                 or both absent — voting false"
            );
            metrics::counter!("dpos_marker_reject_total", "reason" => "equivocation_presence")
                .increment(1);
            return false;
        }
    };
    let Some(bimap) = committee_index else {
        return true;
    };
    if let Err(e) = verify_block_charge(evidence, accused, epoch, (**bimap).clone(), chain_id) {
        tracing::warn!(
            height = block.height,
            epoch,
            accused,
            error = %e,
            "equivocation gate: the block's charge does not verify — voting false"
        );
        metrics::counter!("dpos_marker_reject_total", "reason" => "equivocation_charge")
            .increment(1);
        return false;
    }
    metrics::counter!("dpos_equivocation_charge_verified_total").increment(1);
    true
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
        // A slashed member keeps its seat and its leader slots until the committee
        // turns over, and simplex clears the round's leader deadline the moment
        // its proposal binds (`voter/round.rs` `set_proposal` / `verified`). That
        // is what makes an equivocator's slot cost the full certification
        // deadline instead of the leader timeout: equivocating disarms the timer.
        // Refusing here is the node's one seam before certification — a `false`
        // verify sets both deadlines to now (`voter/state.rs::trigger_timeout` →
        // `set_deadlines(now, now)`), so the view ends at once instead of paying
        // out the certification window.
        //
        // This is a vote decision read from a NON-hash-invariant field, and that
        // is deliberate rather than overlooked: a node that has not yet seen the
        // verdict simply votes as before, so the two populations disagree only on
        // whether this view nullifies — never on which block is final. The flag
        // is monotone, so the disagreement resolves in one direction and within
        // the few blocks it takes the verdict to reach everyone.
        if self.tombstones.contains(&ctx.leader) {
            tracing::warn!(
                height = block.height,
                round = ?ctx.round,
                "refusing to bind a proposal from a validator slashed for equivocation"
            );
            metrics::counter!("dpos_marker_reject_total", "reason" => "tombstoned_leader")
                .increment(1);
            return false;
        }
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

        // Equivocation gate: accusation and evidence bound to each other, and the
        // charge verified against THIS round's committee — the only one whose
        // `signer_idx → BLS key` mapping a running node can reconstruct.
        if !equivocation_gate_decision(
            block,
            ctx.round.epoch().get(),
            self.committee_index.as_ref(),
            self.chain_id,
        ) {
            return false;
        }

        // P4 tripwire: on a same-epoch link `parent.proposal_view` and
        // `ctx.parent.0` are provably equal, but they come from DIFFERENT
        // SOURCES (block body vs simplex context) — a mismatch is a block
        // certified at a view it did not claim. It outlived the witness pin it
        // was a second source for: `proposal_view` is still the key the
        // executor resolves σ at, so a block lying about it still misroutes a
        // derive.
        if ctx.parent.0 != View::zero() && parent.proposal_view != ctx.parent.0.get() {
            metrics::counter!("dpos_parent_view_mismatch_total").increment(1);
            return false;
        }

        // The result-gate poll loop. It used to INTERLEAVE two conditions, the
        // witness-signature arm and this one, over one shared 40-tick budget;
        // with the witness gone the budget has a single claimant and the
        // starvation question it answered no longer exists. Common path:
        // resolves on tick 0, no sleep. A definitive `false` returns at once;
        // an unresolved result at the deadline votes false (EL backpressure).
        // The result gate samples the FINALIZED tier (see
        // `ExecutedChain::finalized_executed_hash`): the honest semantics are
        // "wait for h−K to be finalized-reconciled locally", not "match the
        // speculative head" — the speculative head can carry a sibling that the
        // finalization will replace.
        let check = |this: &Self| {
            result_matches(
                block.result,
                block.height,
                this.dpos_activation_block,
                |h| this.executed.finalized_executed_hash(h),
            )
        };
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
            if result_done {
                return true;
            }
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
            self.assembler.observe_finalized(block);
            (self.boundary_hook)(block.clone());
        }
        // The gauge is observability only, but the watch is not: it is the
        // process's clock — the epoch manager's live-epoch input and the beacon
        // actor's deal/seal clock — and the only one that keeps moving once this
        // node's execution stalls. The tip still travels to the executor
        // untouched below either way.
        if let Update::Tip(_, height, _) = &activity {
            self.plane_clock.record_ordering_tip(height.get());
            // The value and the wake-up in one `send_replace`: unconditional
            // (the marshal reports a tip only when it rises, `store_finalization`
            // guards `height > self.tip`, so there is nothing to filter here) and
            // lossless — the live epoch is a FUNCTION of the current tip, and a
            // consumer that missed the last publish would hold a stale epoch
            // until the next one.
            self.ordering_tip.send_replace(height.get());
            // The beacon plane's channel, written immediately after and from the
            // same statement: one writer, two channels, a fixed order. See
            // `beacon_tip` for why this is not a second subscription above.
            if let Some(beacon_tip) = &self.beacon_tip {
                beacon_tip.send_replace(height.get());
            }
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
    /// `PayloadStatusEnum::Invalid`) rides in `Ok(..)`; a failure that produced
    /// NO verdict is the typed [`EngineError`] in `Err`. This split is
    /// TYPE-LEVEL (family 5): a verdict can never be folded into the error, so
    /// the executor's fork-safety rule ("retry ⇔ transport `Err`; SafetyHalt ⇔
    /// `Ok(Invalid)`") is a property of the return type, not a comment at each
    /// call site.
    ///
    /// The `Err` half is NOT uniformly transient: [`EngineError`] carries its
    /// own [`crate::fault::FaultClass`] so an implementation can distinguish
    /// "reth never processed this" (retry forever) from "reth processed it and
    /// rejected the forkchoice STATE" (a permanent local inconsistency).
    fn fork_choice_updated(
        &self,
        state: ForkchoiceState,
    ) -> impl std::future::Future<Output = Result<ForkchoiceUpdated, EngineError>> + Send;

    /// Import one derived block into the EL. Implementations either hand
    /// reth the pre-executed artifacts (`InsertExecutedBlock` — single
    /// execution) or fall back to `new_payload` (reth re-executes; the
    /// conformance/escape-hatch mode). Same no-verdict-vs-verdict split as
    /// [`Self::fork_choice_updated`]: this shared return type is what makes the
    /// two engine entry points get the IDENTICAL transport class
    /// ([`crate::fault::FaultClass::TransientExternal`]`(EngineRetry)`), closing the historic
    /// asymmetry where an import transport error was actor-death while its FCU
    /// sibling retried.
    fn import_derived(
        &self,
        data: Self::ExecutionData,
    ) -> impl std::future::Future<Output = Result<PayloadStatus, EngineError>> + Send;
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

/// Typed "a gap-walk prefix element on a beacon-active round has no σ yet"
/// derivation failure. The walk owns neither `cause` nor the ack, so it cannot
/// park; its CALLER can, and does — `try_derive` classifies this leaf exactly
/// as it classifies [`ParentHeaderMissing`]. Deriving with the `order.digest()`
/// fallback instead would re-roll `prev_randao` against the round the rest of
/// the network used, so the block waits rather than forks.
#[derive(Debug, thiserror::Error)]
#[error(
    "derive gap: no seed recorded for beacon-active height {height} (round view {proposal_view})"
)]
pub struct PrefixSeedMissing {
    pub height: u64,
    pub proposal_view: u64,
}

/// Derivation with a bounded retry on the parent-visibility race above.
///
/// Any path that derives against a parent imported WITHOUT an awaited
/// canonicalization in between must make the parent visible first — that is a
/// property of the code path, not of the component. Both walks do it: the
/// crash-recovery walk (`dpos.rs`) and the executor's gap-walk
/// (`derive_finalized_with_gap_fill`). Classifying "the executor" as immune
/// wholesale is what left the gap-walk unprotected.
///
/// This retry absorbs the narrower race where the parent is imported
/// CONCURRENTLY by someone else (the follower's first derive after an EL-sync
/// jump, where devp2p canonicalized the parent). It is not a substitute for
/// sending the canonicalization FCU. Any other derivation error stays
/// immediately fatal.
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
    use crate::slasher::Message;
    use alloy_primitives::Address;
    use commonware_consensus::types::{Epoch, View};
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
    use commonware_runtime::Runner as _;
    use fluentbase_bls::keys::ValidatorBlsKeypair;
    use fluentbase_staking_reader::reader::{
        ConsensusKeys, ValidatorSetSnapshot, ValidatorWithKeys,
    };
    use std::sync::Mutex;

    /// The domain separator every test signature is produced and verified under.
    const TEST_CHAIN_ID: u64 = 20_994;

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
            gas_limit: 30_000_000,
            extra_data: Bytes::new(),
            result: B256::ZERO,
            txs: Vec::new(),
            equivocation: None,
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

    /// The BLS keypairs [`committee_bimap`] puts in the map, in the same order —
    /// so a test that must SIGN as a committee member holds the secrets behind
    /// the very pubkeys the map was built from.
    fn committee_bls_keys(n: usize, seed: u64) -> Vec<ValidatorBlsKeypair> {
        use rand_08::{rngs::StdRng, SeedableRng as _};

        let mut rng = StdRng::seed_from_u64(seed);
        (0..n)
            .map(|_| ValidatorBlsKeypair::generate(&mut rng))
            .collect()
    }

    fn committee_bimap(keys: &[Ed25519PrivateKey], seed: u64) -> BiMap<PeerPubkey, BlsPubkey> {
        use commonware_codec::DecodeExt as _;

        keys.iter()
            .zip(committee_bls_keys(keys.len(), seed))
            .map(|(p, bls)| {
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
        let good = extra_data::encode_production_record(3, None);

        // No expectation ⇒ every input passes, including one that is not a
        // record at all. This arm is why a stale `None` would be dangerous.
        assert!(production_record_ok(&good, None));
        assert!(production_record_ok(&[], None));
        assert!(production_record_ok(&[0xAB; 24], None));

        assert!(production_record_ok(&good, Some(3)));
        // Naming an accused member does not change who produced the block.
        assert!(production_record_ok(
            &extra_data::encode_production_record(3, Some(0)),
            Some(3)
        ));
        assert!(
            !production_record_ok(&good, Some(4)),
            "wrong index must fail"
        );
        assert!(
            !production_record_ok(&[], Some(3)),
            "empty must REJECT at verify even though the executor tolerates it"
        );
        // Exact length is what keeps a 4 KiB-tolerant OrderBlock codec from
        // finalizing a block whose extra_data no reth header
        // (FLUENT_MAXIMUM_EXTRA_DATA_SIZE) can hold, so it is pinned on BOTH
        // sides of the record's own width — the near miss below is the 2-byte
        // record this format replaced.
        assert!(!production_record_ok(&[1u8, 3], Some(3)), "short must fail");
        assert!(
            !production_record_ok(&[1u8, 3, extra_data::NO_CHARGE, 0], Some(3)),
            "long must fail"
        );
        assert!(
            !production_record_ok(&[2u8, 3, extra_data::NO_CHARGE], Some(3)),
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
            30_000_000,
            // Tests anchor at activation (genesis.height == activation == 0),
            // so the pre-activation window is unchanged by the anchor/activation split.
            0,
            TEST_CHAIN_ID,
            None,
            TombstoneSet::default(),
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

    /// A verify-side app over the given executed chain.
    ///
    /// It takes no randomness handle any more: `FluentApp` holds none. The seed
    /// the verify path needs rides the certificate the executor resolves, and
    /// the field this app used to carry had no reader at all.
    fn witness_app<XC: ExecutedChain>(executed: XC) -> FluentApp<XC, NoTxs> {
        let (mailbox, _rx) = fresh_mailbox();
        FluentApp::new(
            sample_order(Digest(B256::ZERO), 0),
            mailbox,
            Arc::new(|_b: OrderBlock| {}),
            executed,
            Arc::new(NoTxs),
            30_000_000,
            0,
            TEST_CHAIN_ID,
            None,
            TombstoneSet::default(),
        )
    }

    /// Tiny-timestamp `(parent, block)` pair for the verify-gate tests (the
    /// deterministic clock starts at 0; unix-scale sleeps hang it). Heights
    /// 1→2 sit in the pre-activation result window, so the result gate
    /// resolves on tick 0 unless a test injects its own chain.
    fn witness_pair(parent_view: u64, block_view: u64) -> (OrderBlock, OrderBlock) {
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

    // The honest common path — and it must burn ZERO budget (the result gate
    // resolves on tick 0, no sleep).
    #[test]
    fn the_honest_common_path_verifies_true_with_zero_budget() {
        let (parent, block) = witness_pair(4, 9);
        let ctx = ctx_same_epoch(5, 9, &parent);
        let app = witness_app(NoChain);
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
        let (parent, mut block) = witness_pair(4, 9);
        let (bimap, idx) = armed_committee();
        block.extra_data = extra_data::encode_production_record(idx, None).into();
        let ctx = ctx_same_epoch(5, 9, &parent);
        let app = witness_app(NoChain).with_committee_index(bimap);
        assert!(run_gate(app, ctx, block, parent).0);
    }

    /// The committee snapshot as a node reads it back from chain, with `leader`
    /// carrying the equivocation verdict.
    fn snapshot_tombstoning(
        leader: &PeerPubkey,
        bimap: &BiMap<PeerPubkey, BlsPubkey>,
    ) -> ValidatorSetSnapshot {
        ValidatorSetSnapshot {
            block_hash: B256::ZERO,
            block_number: 1,
            epoch: 5,
            validators: bimap
                .iter_pairs()
                .map(|(peer, bls)| ValidatorWithKeys {
                    address: Address::ZERO,
                    keys: ConsensusKeys {
                        bls_pubkey: *bls,
                        peer_pubkey: peer.clone(),
                        activation_epoch: 0,
                    },
                    tombstoned: peer == leader,
                })
                .collect(),
            weights: None,
        }
    }

    /// A slashed member keeps its seat and its leader slots, and equivocating
    /// disarms the leader timer — binding its proposal clears the round's leader
    /// deadline, so the view then runs to the certification deadline instead.
    /// Refusing is the node's one seam before certification: a `false` verify
    /// makes simplex time the view out immediately.
    ///
    /// NOT expressible here: that the view then ends at the leader timeout rather
    /// than the certification deadline. Both deadlines live in commonware's voter
    /// round, which this crate drives only through the `verify` verdict; the
    /// timing claim belongs to the live byzantine smoke.
    #[test]
    fn a_proposal_from_a_tombstoned_leader_is_refused_and_only_from_that_leader() {
        let leader = Ed25519PrivateKey::from_seed(7).public_key();
        let bystander = Ed25519PrivateKey::from_seed(101).public_key();
        for (label, tombstoned, expected) in [
            ("nobody", None, true),
            ("some other member", Some(&bystander), true),
            ("the round leader", Some(&leader), false),
        ] {
            let (parent, mut block) = witness_pair(4, 9);
            let (bimap, idx) = armed_committee();
            block.extra_data = extra_data::encode_production_record(idx, None).into();
            let ctx = ctx_same_epoch(5, 9, &parent);
            let tombstones = TombstoneSet::default();
            if let Some(peer) = tombstoned {
                tombstones.observe(&snapshot_tombstoning(peer, &bimap));
            }
            let app = witness_app(NoChain)
                .with_committee_index(bimap)
                .with_tombstones(tombstones);
            assert_eq!(
                run_gate(app, ctx, block, parent).0,
                expected,
                "with {label} tombstoned"
            );
        }
    }

    /// The reaction is driven from chain state and not from the evidence a node
    /// happened to hold, and this is the property that makes it survive a
    /// restart: everything below is built fresh — a new tombstone set, a new app,
    /// no charge store, no vote store, no gossip — and the refusal still arms
    /// from the committee snapshot alone.
    #[test]
    fn the_refusal_rearms_from_chain_state_alone_after_a_restart() {
        let leader = Ed25519PrivateKey::from_seed(7).public_key();
        let (bimap, idx) = armed_committee();
        let snapshot = snapshot_tombstoning(&leader, &bimap);

        // Before the read, a restarted process knows nothing and votes as usual.
        let fresh = TombstoneSet::default();
        assert!(!fresh.contains(&leader));

        let (parent, mut block) = witness_pair(4, 9);
        block.extra_data = extra_data::encode_production_record(idx, None).into();
        let ctx = ctx_same_epoch(5, 9, &parent);
        assert!(
            run_gate(
                witness_app(NoChain)
                    .with_committee_index(bimap.clone())
                    .with_tombstones(fresh.clone()),
                ctx,
                block,
                parent
            )
            .0,
            "a process that has read nothing yet must not refuse"
        );

        // The first committee read after the restart is the whole input.
        fresh.observe(&snapshot);

        let (parent, mut block) = witness_pair(4, 9);
        block.extra_data = extra_data::encode_production_record(idx, None).into();
        let ctx = ctx_same_epoch(5, 9, &parent);
        assert!(
            !run_gate(
                witness_app(NoChain)
                    .with_committee_index(bimap)
                    .with_tombstones(fresh),
                ctx,
                block,
                parent
            )
            .0,
            "the snapshot alone re-arms the refusal"
        );
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
                extra_data::encode_production_record(idx + 1, None),
            ),
            ("empty", Vec::new()),
        ] {
            let (parent, mut block) = witness_pair(4, 9);
            block.extra_data = field.into();
            let ctx = ctx_same_epoch(5, 9, &parent);
            let app = witness_app(NoChain).with_committee_index(bimap.clone());
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
            let (parent, mut block) = witness_pair(4, 9);
            // Seeds 9000.. — disjoint from the seed-7 leader `ctx_same_epoch` names.
            let (_, disjoint) = test_committee(4, 9);
            block.extra_data = extra_data::encode_production_record(0, None).into();
            let ctx = ctx_same_epoch(5, 9, &parent);
            let app = witness_app(NoChain).with_committee_index(Arc::new(disjoint));
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

    // §9 gate test 8 / (N1-c): rule SA — a block lying about its own proposal
    // view is rejected (with everything else valid). Without SA, PIN pins
    // nothing.
    #[test]
    fn a_block_lying_about_its_own_proposal_view_is_rejected() {
        let (parent, block) = witness_pair(4, 8 /* lies: certified view is 9 */);
        let ctx = ctx_same_epoch(5, 9, &parent);
        let app = witness_app(NoChain);
        assert!(!run_gate(app, ctx, block, parent).0);
    }

    /// An executed chain whose hash becomes available only after N
    /// `finalized_executed_hash` polls — models "execution reaches h − K
    /// mid-verify". Kept when the witness arm was deleted: the result gate is
    /// still a POLL loop, and nothing else in this module makes it take more
    /// than tick 0.
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
        fn finalized_executed_hash(&self, height: u64) -> Option<B256> {
            self.spec_executed_hash(height)
        }
    }

    /// A post-activation `(parent, block)` pair whose result gate must read
    /// `executed_hash(height − K)` — height 3 with `dpos_activation_block = 0`.
    fn result_gated_pair(result: B256) -> (OrderBlock, OrderBlock) {
        let parent = OrderBlock {
            proposal_view: 7,
            height: 2,
            timestamp: 1,
            ..sample_order(Digest(B256::ZERO), 2)
        };
        let block = OrderBlock {
            proposal_view: 3,
            height: 3,
            timestamp: 2,
            result,
            ..sample_order(parent.digest(), 3)
        };
        (parent, block)
    }

    /// The result gate RE-READS across ticks: a chain that only answers on the
    /// 5th poll still verifies true, and the verify spends real budget doing it.
    /// Reds if the loop resolves the gate once instead of polling.
    #[test]
    fn the_result_gate_polls_until_the_el_catches_up() {
        let exec_hash = B256::repeat_byte(0x5E);
        let (parent, block) = result_gated_pair(exec_hash);
        let chain = TickChain {
            calls: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            ready_after: 5,
            hash: exec_hash,
        };
        let ctx = ctx_boundary(12, 3, &parent);
        let app = witness_app(chain);
        let (verdict, elapsed) = run_gate(app, ctx, block, parent);
        assert!(verdict, "a gate that resolves inside the budget votes true");
        assert_eq!(
            elapsed,
            VERIFY_EXEC_POLL * 5,
            "and it got there by polling, one tick per unavailable read"
        );
    }

    /// The other end of the same loop: an EL that never reaches h − K spends the
    /// whole budget and votes FALSE (backpressure), rather than accepting an
    /// unchecked result.
    #[test]
    fn the_result_gate_votes_false_when_the_budget_runs_out() {
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        let (verdict, elapsed) = metrics::with_local_recorder(&recorder, || {
            let (parent, block) = result_gated_pair(B256::repeat_byte(0x5E));
            let chain = TickChain {
                calls: Arc::new(std::sync::atomic::AtomicU32::new(0)),
                ready_after: u32::MAX,
                hash: B256::ZERO,
            };
            let ctx = ctx_boundary(12, 3, &parent);
            run_gate(witness_app(chain), ctx, block, parent)
        });
        assert!(!verdict);
        assert_eq!(elapsed, VERIFY_EXEC_BUDGET, "the whole budget is spent");
        assert_eq!(
            counter_at(&snap, "dpos_result_gate_finalized_miss_total", None),
            1
        );
    }

    // The anchor link — Ec == 0 with the GENESIS_VIEW parent sentinel is the
    // chain anchor (never proposed, proposal_view == 0), and the first
    // post-activation block verifies over it.
    #[test]
    fn anchor_link_verifies_over_it() {
        let (parent, block) = witness_pair(0, 1);
        let ctx = ctx_boundary(0, 1, &parent);
        let app = witness_app(NoChain);
        assert!(run_gate(app, ctx, block, parent).0);
    }

    // (P4) the second-source tripwire: on a same-epoch link the parent's
    // self-attested view must agree with the simplex context's parent view —
    // a mismatch is a block certified at a view it did not claim, which would
    // misroute the round every node resolves that block's σ at.
    #[test]
    fn parent_proposal_view_disagreeing_with_ctx_parent_view_is_rejected() {
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        let verdict = metrics::with_local_recorder(&recorder, || {
            let (parent, block) = witness_pair(4, 9);
            let mut ctx = ctx_same_epoch(5, 9, &parent);
            ctx.parent.0 = View::new(5); // simplex says the parent certified at 5
            let app = witness_app(NoChain);
            run_gate(app, ctx, block, parent).0
        });
        assert!(!verdict);
        assert_eq!(
            counter_at(&snap, "dpos_parent_view_mismatch_total", None),
            1
        );
    }

    // propose side (§3)

    fn propose_app(charges: Option<ChargeStore>) -> FluentApp<NoChain, NoTxs> {
        let (mailbox, _rx) = fresh_mailbox();
        FluentApp::new(
            sample_order(Digest(B256::ZERO), 0),
            mailbox,
            Arc::new(|_b: OrderBlock| {}),
            NoChain,
            Arc::new(NoTxs),
            30_000_000,
            0,
            TEST_CHAIN_ID,
            charges,
            TombstoneSet::default(),
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

    /// The wire flip's propose half: a proposal carries EXACTLY the 3-byte record
    /// naming its own proposer, and that record is what its voters recompute from
    /// `ctx.leader`. Asserting the bytes (not just "some extra_data") is the point
    /// — the executor feeds `leader_index` straight to `recordProduction`, so a
    /// silently wrong index mis-credits production with no other symptom.
    #[test]
    fn proposal_stamps_the_production_record_naming_its_proposer() {
        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|rt| async move {
            let app = propose_app(None);
            let parent = tiny_parent(4);
            let ctx = propose_ctx(5, 9, (4, false), &parent);
            let block = app
                .build_proposal(&rt, &ctx, parent)
                .await
                .expect("proposed");
            assert_eq!(
                block.extra_data.as_ref(),
                extra_data::encode_production_record(propose_leader_index(), None).as_slice(),
            );
            // And the proposer's own verifier accepts what it just built.
            assert!(
                production_record_ok(&block.extra_data, Some(propose_leader_index())),
                "a proposer must never build a block its own verify rule rejects"
            );
        });
    }

    /// A real charge signed by the member `propose_committee()` seats behind
    /// ed25519 seed 8 — deliberately NOT the fixture leader, so the accused and
    /// the producer are different members and a test cannot pass by conflating
    /// the two record bytes.
    fn sample_charge(epoch: u64, view: u64) -> (u8, Message) {
        use commonware_consensus::simplex::types::{
            Activity, Attributable as _, ConflictingNotarize, Notarize, Proposal,
        };
        use fluentbase_bls::{fluent_namespace, scheme::build_signer};

        let committee = propose_committee();
        let offender = build_signer(
            &fluent_namespace(TEST_CHAIN_ID),
            (*committee).clone(),
            &committee_bls_keys(3, 7)[1],
            epoch,
            None,
        )
        .expect("the offender is a committee member");
        let round = Round::new(Epoch::new(epoch), View::new(view));
        let vote = |tag: u8| {
            Notarize::sign(
                &offender,
                Proposal::new(round, View::new(view - 1), Digest(B256::repeat_byte(tag))),
            )
            .expect("the offender signs")
        };
        let (first, second) = (vote(0xaa), vote(0xbb));
        let accused = first.signer().get() as u8;
        (
            accused,
            Activity::ConflictingNotarize(ConflictingNotarize::new(first, second)),
        )
    }

    /// The block path's whole point: a proposer that holds a charge stamps the
    /// verdict into `extra_data` AND the evidence into the block, and its own
    /// vote-time gate accepts what it just built. Asserting both halves together
    /// is the test — either alone would pass on a block no voter would take.
    #[test]
    fn a_proposer_holding_a_charge_stamps_the_verdict_and_its_evidence() {
        let (accused, charge) = sample_charge(5, 9);
        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|rt| async move {
            let charges = ChargeStore::default();
            assert!(charges.hold(5, accused, charge));

            let app = propose_app(Some(charges));
            let parent = tiny_parent(4);
            let ctx = propose_ctx(5, 9, (4, false), &parent);
            let block = app
                .build_proposal(&rt, &ctx, parent)
                .await
                .expect("proposed");

            assert_eq!(
                block.extra_data.as_ref(),
                extra_data::encode_production_record(propose_leader_index(), Some(accused))
                    .as_slice(),
            );
            assert!(block.equivocation.is_some());
            assert!(
                equivocation_gate_decision(&block, 5, Some(&propose_committee()), TEST_CHAIN_ID),
                "a proposer must never build a block its own verify rule rejects"
            );
            // A charge the proposer does not hold for THIS epoch is not offered:
            // only the epoch's own committee could verify it.
            assert_eq!(
                app.charges
                    .as_ref()
                    .and_then(|c| c.next_charge(6, |_| false))
                    .map(|(idx, _)| idx),
                None
            );
        });
    }

    /// Once the verdict has landed the charge has nothing left to achieve, and
    /// leaving it queued is worse than useless: it is the lowest key for its
    /// epoch, so it would occupy the one-charge-per-block slot ahead of every
    /// later charge for the same epoch, forever. So the proposer drops it and
    /// offers the next one instead.
    #[test]
    fn a_charge_whose_verdict_already_landed_is_dropped_rather_than_re_offered() {
        let (accused, charge) = sample_charge(5, 9);
        let committee = propose_committee();
        let settled = committee
            .get(accused as usize)
            .expect("the accused is seated")
            .clone();
        // A second charge for the same epoch, seated ABOVE the settled one so the
        // walk has to get past it rather than stopping at the first key.
        let later = accused
            .checked_add(1)
            .filter(|idx| (*idx as usize) < committee.len())
            .expect("the fixture committee seats a member above the accused");

        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|rt| async move {
            let charges = ChargeStore::default();
            assert!(charges.hold(5, accused, charge.clone()));
            assert!(charges.hold(5, later, charge));

            let tombstones = TombstoneSet::default();
            tombstones.observe(&snapshot_tombstoning(&settled, &committee));

            let app = propose_app(Some(charges.clone())).with_tombstones(tombstones);
            let parent = tiny_parent(4);
            let ctx = propose_ctx(5, 9, (4, false), &parent);
            let block = app
                .build_proposal(&rt, &ctx, parent)
                .await
                .expect("proposed");

            assert_eq!(
                block.extra_data.as_ref(),
                extra_data::encode_production_record(propose_leader_index(), Some(later))
                    .as_slice(),
                "the settled charge is stepped over, the next one carried"
            );
            assert!(
                !charges.contains(5, accused),
                "and the settled charge is gone, not merely skipped"
            );
        });
    }

    /// Presence is exact in both directions. A verdict without evidence is a
    /// slash nobody could check; evidence without a verdict is unagreed payload
    /// riding under the digest. Both are one Byzantine proposer away.
    #[test]
    fn the_equivocation_gate_binds_the_verdict_to_its_evidence() {
        let (accused, charge) = sample_charge(5, 9);
        let evidence = Bytes::from(charge.encode().to_vec());
        let committee = propose_committee();
        let charged = |accused: Option<u8>, evidence: Option<Bytes>| OrderBlock {
            extra_data: Bytes::from(extra_data::encode_production_record(
                propose_leader_index(),
                accused,
            )),
            equivocation: evidence,
            ..sample_order(Digest(B256::ZERO), 9)
        };

        for (block, expected, why) in [
            (
                charged(None, None),
                true,
                "no charge at all is the common block",
            ),
            (
                charged(Some(accused), Some(evidence.clone())),
                true,
                "a verdict backed by its evidence",
            ),
            (
                charged(Some(accused), None),
                false,
                "a verdict nobody could check",
            ),
            (
                charged(None, Some(evidence.clone())),
                false,
                "evidence under the digest that the record does not claim",
            ),
        ] {
            assert_eq!(
                equivocation_gate_decision(&block, 5, Some(&committee), TEST_CHAIN_ID),
                expected,
                "{why}"
            );
        }

        // An instance with no committee map casts no vote, so it skips only the
        // CRYPTOGRAPHIC arm — the presence rule needs no committee and still holds.
        assert!(equivocation_gate_decision(
            &charged(Some(accused), Some(evidence.clone())),
            5,
            None,
            TEST_CHAIN_ID
        ));
        assert!(!equivocation_gate_decision(
            &charged(Some(accused), None),
            5,
            None,
            TEST_CHAIN_ID
        ));

        // The epoch invariant, at the gate rather than at the decoder: the charge
        // is real, but only epoch 5's committee can check it, so epoch 6 refuses
        // it rather than voting on crypto it cannot run.
        assert!(!equivocation_gate_decision(
            &charged(Some(accused), Some(evidence.clone())),
            6,
            Some(&committee),
            TEST_CHAIN_ID
        ));
        // Real evidence, innocent victim.
        assert!(!equivocation_gate_decision(
            &charged(Some(accused + 1), Some(evidence)),
            5,
            Some(&committee),
            TEST_CHAIN_ID
        ));
    }

    /// A leader that cannot name itself in its committee SKIPS the view instead of
    /// proposing a block every honest voter would reject. Cheaper by one wasted
    /// leader deadline, and it keeps "every consensus block carries a valid record"
    /// true by construction rather than by convention.
    #[test]
    fn a_leader_outside_its_own_committee_declines_to_propose() {
        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|rt| async move {
            // A committee that does NOT contain the fixture leader (`from_seed(7)`).
            let (_outsiders, disjoint) = test_committee(3, 99);
            let app = propose_app(None).with_committee_index(Arc::new(disjoint));
            let parent = tiny_parent(4);
            let ctx = propose_ctx(5, 9, (4, false), &parent);
            assert!(app.build_proposal(&rt, &ctx, parent).await.is_none());
        });
    }

    // §9 propose: every proposal self-attests its view (rule SA), and on a
    // beacon-active same-epoch link it proposes only once
    // `Round::new(Ec, parent.proposal_view)` is held — the seed no longer rides
    // the block, so the store hit is observable as the view PROCEEDING.
    #[test]
    fn proposal_self_attests_its_view_over_a_held_witness_round() {
        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|rt| async move {
            let app = propose_app(None);
            let parent = tiny_parent(4);
            let ctx = propose_ctx(5, 9, (4, false), &parent);
            let block = app
                .build_proposal(&rt, &ctx, parent)
                .await
                .expect("proposed");
            assert_eq!(block.proposal_view, 9, "rule SA: proposal_view == ctx view");
        });
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

    /// Every leg runs with the record rule ARMED (`Some`), which is what a voter
    /// always passes: `expected_leader_index` returns `Ok(None)` only for an
    /// instance that casts no vote.
    #[test]
    fn structural_checks_reject_each_violation() {
        const LEADER: u8 = 3;
        let parent = sample_order(Digest(B256::ZERO), 1);
        let good = OrderBlock {
            parent: parent.digest(),
            extra_data: Bytes::from(extra_data::encode_production_record(LEADER, None)),
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
            extra_data: Bytes::from(extra_data::encode_production_record(LEADER + 1, None)),
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

    /// The epoch manager's live-epoch input has ONE writer and it is the
    /// marshal's BFT-attested tip — R-003 closed by construction.
    ///
    /// The incident was that the live frontier was inferred from epoch tags on
    /// the unauthenticated vote backup channel, so a peer naming `u64::MAX` (or
    /// f+1 of them naming anything) pinned an honest node into permanent
    /// verify-only. The replacement is `epoch_of(marshal tip)`, and this is the
    /// only door the tip comes through: a height reported here has been
    /// BLS-verified and stored by the marshal (CW `marshal/core/actor.rs`
    /// `verify_delivered` → `store_finalization` → `Update::Tip`), and no peer
    /// can put a number in it that the committee did not certify.
    ///
    /// What the test can show locally is the door: a delivered BLOCK — the only
    /// other activity this reporter sees — moves nothing, and a `Tip` publishes
    /// exactly its own height. A second writer anywhere would break the first
    /// half; taking the tip from anything but `Update::Tip` would break the
    /// second.
    #[test]
    fn the_ordering_tip_watch_is_written_only_by_a_verified_tip() {
        use commonware_consensus::types::{Epoch, Height, View};
        use commonware_utils::{acknowledgement::Exact, Acknowledgement as _};

        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (mailbox, _rx) = fresh_mailbox();
            let mut app = build_app(mailbox, Arc::new(|_b: OrderBlock| {}));
            let mut tip = app.ordering_tip();
            assert_eq!(*tip.borrow(), 0, "no tip reported yet");

            // A finalized BLOCK at a height is not a tip: the block stream is
            // ack-gated on the executor and lags the attested frontier, and the
            // live epoch is a function of the frontier.
            let (ack, _waiter) = Exact::handle();
            <FluentApp<NoChain, NoTxs> as Reporter>::report(
                &mut app,
                Update::Block(sample_order(Digest(B256::ZERO), 7), ack),
            )
            .await;
            assert_eq!(
                *tip.borrow(),
                0,
                "a delivered block must not move the live-epoch input"
            );

            let round = Round::new(Epoch::new(0), View::new(0));
            <FluentApp<NoChain, NoTxs> as Reporter>::report(
                &mut app,
                Update::Tip(round, Height::new(4_242), Digest(B256::ZERO)),
            )
            .await;
            assert!(tip
                .has_changed()
                .expect("the sender is the app, alive here"));
            assert_eq!(
                *tip.borrow_and_update(),
                4_242,
                "the tip published is the attested height itself"
            );

            // A CLONE of the app publishes into the SAME channel: the marshal's
            // reporter half and the epoch manager's copy are clones of one
            // `FluentApp`, and a per-clone channel would leave the manager
            // subscribed to a writer nothing drives.
            let mut clone = app.clone();
            <FluentApp<NoChain, NoTxs> as Reporter>::report(
                &mut clone,
                Update::Tip(round, Height::new(4_300), Digest(B256::ZERO)),
            )
            .await;
            assert_eq!(*tip.borrow(), 4_300, "a clone writes the same channel");
        });
    }

    // The ordering clock must ride marshal's BFT-attested TIP, never block
    // DELIVERY: delivery is ack-gated on the executor, so a gauge fed from
    // `Update::Block` would freeze with the very pipeline it exists to expose.
    #[test]
    fn the_ordering_clock_tracks_the_tip_and_not_the_delivered_block() {
        use commonware_consensus::types::{Epoch, View};
        use commonware_utils::{acknowledgement::Exact, Acknowledgement as _};

        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (mailbox, _rx) = fresh_mailbox();
            let clock = crate::sync_metrics::PlaneClock::default();
            let mut app =
                build_app(mailbox, Arc::new(|_b: OrderBlock| {})).with_plane_clock(clock.clone());

            let (ack, _waiter) = Exact::handle();
            <FluentApp<NoChain, NoTxs> as Reporter>::report(
                &mut app,
                Update::Block(sample_order(Digest(B256::ZERO), 900), ack),
            )
            .await;
            assert_eq!(
                clock.snapshot().0,
                0,
                "a delivered block is ack-gated on the executor and must not move the clock"
            );

            <FluentApp<NoChain, NoTxs> as Reporter>::report(
                &mut app,
                Update::Tip(
                    Round::new(Epoch::new(0), View::new(7)),
                    commonware_consensus::types::Height::new(900),
                    Digest(B256::ZERO),
                ),
            )
            .await;
            let (ordering, dkg, lag) = clock.snapshot();
            assert_eq!(ordering, 900);
            assert_eq!(
                (dkg, lag),
                (0, -1),
                "the DKG half has never been written, so the two are not yet comparable"
            );
        });
    }

    /// The watch handed in by `with_beacon_tip` IS written by the same
    /// `Update::Tip` that writes the app's own — the property that makes the
    /// beacon actor's clock and the epoch manager's tip ONE VALUE on two
    /// channels: the actor holds a receiver taken from the sender BEFORE the app
    /// existed, the manager subscribes through `ordering_tip()` afterwards, and
    /// one `Update::Tip` must reach both. A `with_beacon_tip` that dropped the
    /// sender (or a `report` that wrote only one of the two) would leave the
    /// actor's receiver on a channel nothing writes — `changed()` never fires,
    /// no deal, no seal.
    #[test]
    fn the_tip_is_published_on_the_watch_handed_in_and_on_every_later_subscription() {
        use commonware_consensus::types::{Epoch, View};

        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (mailbox, _rx) = fresh_mailbox();
            // The plane's receiver, taken before the app exists.
            let plane_tip = Arc::new(tokio::sync::watch::Sender::new(0u64));
            let mut actor_clock = plane_tip.subscribe();
            let app = build_app(mailbox, Arc::new(|_b: OrderBlock| {}))
                .with_beacon_tip(plane_tip.clone());
            // The manager's subscription, taken after — through the app.
            let mut manager_tip = app.ordering_tip();
            // The clone the marshal reports into.
            let mut reporter = app.clone();

            let tip = |h: u64| {
                Update::Tip(
                    Round::new(Epoch::new(0), View::new(7)),
                    commonware_consensus::types::Height::new(h),
                    Digest(B256::ZERO),
                )
            };
            <FluentApp<NoChain, NoTxs> as Reporter>::report(&mut reporter, tip(900)).await;
            assert!(
                actor_clock.has_changed().expect("the sender is alive"),
                "the receiver taken from the sender handed in was not woken"
            );
            assert_eq!(*actor_clock.borrow_and_update(), 900);
            assert!(manager_tip.has_changed().expect("the sender is alive"));
            assert_eq!(*manager_tip.borrow_and_update(), 900);
            assert_eq!(
                *plane_tip.borrow(),
                900,
                "published on the sender handed in"
            );

            // Nothing is dropped and nothing coalesces away the newest value: two
            // tips back to back leave the receiver at the second.
            <FluentApp<NoChain, NoTxs> as Reporter>::report(&mut reporter, tip(901)).await;
            <FluentApp<NoChain, NoTxs> as Reporter>::report(&mut reporter, tip(902)).await;
            assert!(actor_clock.has_changed().expect("the sender is alive"));
            assert_eq!(*actor_clock.borrow_and_update(), 902);
        });
    }
}
