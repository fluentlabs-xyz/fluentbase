//! Fluent Application: bridges commonware consensus to the deferred-execution
//! pipeline. `propose` assembles an ordering artifact with no EL work on the
//! critical path; `verify` is a pure function of agreed state and the local
//! derived chain; `report` feeds finalized artifacts to [`crate::executor`]
//! for derive and import.

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

/// Bounded wait in `verify` for local execution to reach `N − K`. Sized to
/// the worst-case derive+execute of one block and to sit inside the
/// certification window. Liveness tuning, not a safety parameter, but keep it
/// uniform across nodes.
pub const VERIFY_EXEC_BUDGET: Duration = Duration::from_millis(1000);
const VERIFY_EXEC_POLL: Duration = Duration::from_millis(25);

/// Target ordering cadence: one block per second. The proposer holds its
/// proposal until wall clock reaches `parent.timestamp + BLOCK_INTERVAL`, so
/// timestamps advance as consecutive integer seconds. Honest-proposer
/// discipline only; [`TIMESTAMP_FUTURE_TOLERANCE_SECS`] is the verify-side
/// enforcement.
pub const BLOCK_INTERVAL: Duration = Duration::from_secs(1);

/// Verify-side future bound: reject `block.timestamp > now + tolerance`. One
/// second covers second-granularity truncation and honest NTP skew. Without it
/// a single far-future timestamp ratchets `block.timestamp` forward and makes
/// every honest proposer sleep until it, halting the chain. Deliberately equal
/// to `BLOCK_INTERVAL`; a consensus rule, so it must be uniform across nodes.
pub const TIMESTAMP_FUTURE_TOLERANCE_SECS: u64 = 1;

/// Read-side view of the local derived chain, shared by propose/verify and
/// the executor. The node crate implements it over reth's provider; hash
/// strictly by number, never `best_number` (its semantics flip between
/// tree-sync and pipeline backfill).
pub trait ExecutedChain: Clone + Send + Sync + 'static {
    /// Highest derived + canonicalized height.
    fn executed_tip(&self) -> u64;

    /// Tier-S (speculative): canonical EVM hash of the derived block at `height`
    /// on the provider head chain, advanced at notarization latency but not yet
    /// beyond reorg. Read only by the executor's parent-linkage and backward
    /// cross-checks, never by the result gate.
    fn spec_executed_hash(&self, height: u64) -> Option<B256>;

    /// Tier-F (finalized): the finalized-execution result at `height`, or `None`
    /// if this node has not finalized-derived `height` yet. The result gate
    /// (propose and verify) samples this tier so a still-speculative sibling at
    /// `h−K` can never be committed as a result and then re-finalize as another
    /// sibling. Tier-F is reth's canonical chain below the monotone finalized
    /// cursor ([`FinalizedCursor`]), so there is no separate hash store. No
    /// default: every consumer chooses its tier explicitly.
    fn finalized_executed_hash(&self, height: u64) -> Option<B256>;

    /// Advance the monotone finalized-execution cursor to `height` — executor
    /// only, called past the `try_derive` canonical postcondition and at the
    /// BLS-authenticated re-jump landing. Seeded at `Actor::init` from the
    /// marshal's durable acked cursor. Monotone (`fetch_max`); a lower value is a
    /// no-op. Default no-op for readers and non-writers.
    fn advance_finalized(&self, _height: u64) {}
}

/// The monotone finalized-execution cursor: the highest height known
/// finalized-without-a-possible-sibling. Tier-F
/// ([`ExecutedChain::finalized_executed_hash`]) is reth's canonical chain
/// below this cursor, which stores no hashes because reth already persists
/// them. Shared, so it survives per-epoch engine restarts within the process.
#[derive(Clone, Debug, Default)]
pub struct FinalizedCursor {
    cursor: Arc<AtomicU64>,
}

impl FinalizedCursor {
    /// Tier-F lookup: the provider's canonical hash at `height` is the finalized
    /// hash iff `height <= cursor`; above the cursor the height is not yet
    /// reconciled, so `None`. A provider miss at or below the cursor also returns
    /// `None`, never a wrong hash.
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
    /// [`crate::committee`] takes this height as its read anchor. It lags the
    /// executor's `ordering_finalized` by at most one derive, which is the safe
    /// direction for an anchor.
    pub fn height(&self) -> u64 {
        self.cursor.load(Ordering::Acquire)
    }
}

/// Ordering assembly: picks txs for a height against executed state plus the
/// in-flight ordered-but-unexecuted suffix overlay, without executing.
pub trait OrderingAssembler: Send + Sync + 'static {
    fn assemble(&self, height: u64, gas_limit: u64, byte_budget: usize) -> Vec<TransactionSigned>;

    /// Record every ordering-finalized artifact, in order, so the in-flight
    /// suffix stays authoritative and `assemble` does not re-propose what the
    /// pool still thinks is pending (the pool tracks the executed head, which
    /// lags ordering by up to `K`).
    fn observe_finalized(&self, block: &OrderBlock);
}

/// EIP-1559 header rule: `|limit − parent| < parent/1024` and
/// `limit ≥ MIN_GAS_LIMIT`. The gas limit is agreed data, so verify bounds it
/// against the parent as Ethereum header validation does.
pub fn gas_limit_within_1_1024(parent: u64, limit: u64) -> bool {
    limit >= MIN_GAS_LIMIT && limit.abs_diff(parent) < (parent / 1024).max(1)
}

/// Why [`FluentApp::expected_leader_index`] refused. Both variants are a vote
/// reject, never a skip; they are named separately so the log says which. The
/// second is made unreachable by `OuterBuilder::build`'s startup assert.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LeaderIndexError {
    #[error("round leader is not a member of this epoch's committee")]
    LeaderNotInCommittee,
    #[error("committee index {index} exceeds the 1-byte record (committee size {committee_size})")]
    IndexExceedsWireFormat { index: usize, committee_size: usize },
}

/// Vote-time rule for the production record in `extra_data`: a voting block
/// must carry exactly the [`extra_data::PRODUCTION_RECORD_LEN`]-byte record
/// naming its own leader. `None` means this instance has no committee map and
/// casts no vote, so the rule is skipped; `Some(i)` demands exact length, a
/// known version, and `carried == i`.
///
/// Empty `extra_data` rejects under `Some` even though the executor must
/// tolerate it (a migration-window block may carry none), and verify's
/// refusal is what keeps the executor's empty arm unreachable through
/// consensus. The exact length also keeps the 4 KiB-tolerant OrderBlock codec
/// from finalizing a block whose `extra_data` no reth header can hold.
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

pub struct FluentApp<XC, A> {
    genesis: Arc<OrderBlock>,
    executor: executor::Mailbox,
    /// Observer for `Update::Block` finalizations; not a state-advancing path.
    /// Wired to the staking reader's epoch-boundary detection.
    boundary_hook: Arc<dyn Fn(OrderBlock) + Send + Sync>,
    /// Rate-limiter cursor for the result-gate slow-wait log line: the last height
    /// for which it was emitted, so a height re-verified across views logs once.
    /// Observability only; created internally, not a `new` argument.
    verify_gate_last_logged_height: Arc<AtomicU64>,
    executed: XC,
    assembler: Arc<A>,
    /// Proposer-local: shapes only this node's own proposals (agreed data once
    /// embedded); verify never reads it.
    target_gas_limit: u64,
    /// Chain-wide sequencer→DPoS activation block — origin of the
    /// `result_target` pre-activation window (`height < activation + K`). A chain
    /// constant, not this node's cold-start anchor (`genesis.height`): a
    /// deep-catch-up node still proposes and verifies the K-below-anchor blocks,
    /// which are post-activation and carry real results.
    dpos_activation_block: u64,
    /// The epoch committee's pubkey→index map, injected by
    /// [`Self::with_committee_index`] from `EpochEngine::new` — the same map the
    /// engine builds its scheme from, so the index this app computes and the
    /// committee the engine votes with are one agreed snapshot.
    ///
    /// `None` means this instance holds no committee (a verify-only scheme, a
    /// follower, a test): it casts no vote, so a missing index is not a vote
    /// condition and is permissive. A present map with a miss is a reject; see
    /// [`Self::expected_leader_index`]. Deliberately not an epoch-keyed registry:
    /// a registry miss would make a vote depend on node-local lookup state.
    committee_index: Option<Arc<BiMap<PeerPubkey, BlsPubkey>>>,
    /// L2 chain id — the domain separator of the vote signatures a block-carried
    /// equivocation charge is verified under. A chain constant, so it lives on the
    /// cross-epoch app rather than the per-epoch committee injection.
    chain_id: u64,
    /// Read handle on the slasher's verified-charge queue, drained at most one
    /// charge per block by [`Self::build_proposal`]. `None` for an instance with
    /// no slasher.
    charges: Option<ChargeStore>,
    /// Committee members observed slashed for equivocation. Read at verify to
    /// refuse their proposals and at propose to drop a charge whose verdict has
    /// landed. Empty is the honest "nothing observed" state, so the default is
    /// permissive.
    tombstones: TombstoneSet,
    /// Ordering half of the clock pair, written in [`Reporter::report`]. The app
    /// is the right writer because it is built once per process and holds no
    /// execution state, so it keeps reporting through a `SafetyHalt` park and an
    /// executor death.
    plane_clock: crate::sync_metrics::PlaneClock,
    /// The marshal's ordering tip — the height of the highest finalization this
    /// node has verified and stored — published from the `Update::Tip` arm of
    /// [`Reporter::report`].
    ///
    /// One writer and one parked consumer per channel: the epoch manager
    /// subscribes through [`Self::ordering_tip`], the beacon's `DkgActor` parks on
    /// [`Self::beacon_tip`], written in the same statement. A watch, so a consumer
    /// can read the tip at a decision point and be woken by it. The separate
    /// channel for the beacon is load-bearing: two receivers parked on one
    /// `tokio::sync::watch` are woken in a process-random order, which reorders a
    /// seeded deterministic run.
    ///
    /// `0` until the first tip, which is the right answer at genesis.
    ordering_tip: Arc<tokio::sync::watch::Sender<u64>>,
    /// The beacon plane's own tip channel, built before the app, which keeps the
    /// receiver for its `DkgActor` and hands the sender in
    /// ([`Self::with_beacon_tip`]). Written from the same `Update::Tip` arm as
    /// [`Self::ordering_tip`] immediately after it. `None` on a node with no
    /// beacon plane.
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
        // The same handle must also reach `slasher::Config`; a second store would be
        // a queue nothing fills.
        charges: Option<ChargeStore>,
        // The writer is the node's tombstone watcher in another crate, so a default
        // here would be a set nothing fills.
        tombstones: TombstoneSet,
    ) -> Self {
        Self {
            committee_index: None,
            chain_id,
            charges,
            tombstones,
            // Observability only, so a setter rather than another constructor argument;
            // an instance that never receives one publishes nothing.
            plane_clock: crate::sync_metrics::PlaneClock::default(),
            // A channel of its own so every clone of one `FluentApp` shares it by
            // construction. A node with a beacon plane adds the plane's channel beside it
            // (`with_beacon_tip`).
            ordering_tip: Arc::new(tokio::sync::watch::Sender::new(0)),
            beacon_tip: None,
            genesis: Arc::new(genesis),
            executor,
            boundary_hook,
            // u64::MAX sentinel: 0 is a valid height, so it cannot mean "not yet logged".
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

    /// Publish the ordering tip on this watch as well as the app's own. The beacon
    /// plane is built before the app and its `DkgActor` holds a receiver of this
    /// sender. Call it before the app is cloned — the builders do — or the reporter
    /// half keeps `None` and the actor's `changed()` never fires.
    ///
    /// A second channel, not a second subscription to the app's own: two receivers
    /// parked on one `tokio::sync::watch` are woken in a process-random order.
    pub fn with_beacon_tip(mut self, beacon_tip: Arc<tokio::sync::watch::Sender<u64>>) -> Self {
        self.beacon_tip = Some(beacon_tip);
        self
    }

    /// Subscribe to the marshal's ordering tip. The receiver holds the last value,
    /// so a consumer built after a tip was already reported reads that tip rather
    /// than the `0` seed.
    pub fn ordering_tip(&self) -> tokio::sync::watch::Receiver<u64> {
        self.ordering_tip.subscribe()
    }

    /// Attach the epoch committee's pubkey→index map. Called from
    /// `EpochEngine::new`, which holds both the map and this app before moving the
    /// app into `Inline`. Followers, verify-only schemes and tests leave it unset.
    pub fn with_committee_index(mut self, bimap: Arc<BiMap<PeerPubkey, BlsPubkey>>) -> Self {
        self.committee_index = Some(bimap);
        self
    }

    /// Install the tombstone view a running node's watcher fills. Test-only: in
    /// production the handle is a constructor argument so an instance cannot be
    /// built without the writer's own.
    #[cfg(test)]
    fn with_tombstones(mut self, tombstones: TombstoneSet) -> Self {
        self.tombstones = tombstones;
        self
    }

    /// The committee index of `leader` for the production record carried in
    /// `extra_data`.
    ///
    /// `Ok(None)` means no committee map (not a voter, so the caller skips the
    /// index rule); `Ok(Some(i))` is the index to compare the carried byte
    /// against; `Err` means the map is present and the leader is not in it, or the
    /// index exceeds `u8::MAX`. `Err` is a reject, never a skip: folding it into
    /// the permissive `None` arm would silently invert the rule.
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

    /// Pure structural validity of `block` against its parent — everything verify
    /// checks without touching the local derived chain. Parent linkage and
    /// contiguous height are already enforced by Inline's `validate_block`.
    ///
    /// Rule SA: `block.proposal_view` must equal the verifier's own
    /// `ctx.round.view()`, a consensus input rather than local state. SA is a
    /// vote-time-only obligation: no ingress path (cert-follower, backfill,
    /// cold-start, recovery) may re-check it against local cert state.
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

    /// Paced proposal body, factored out of `Application::propose` so pacing and
    /// timestamp behavior is unit-testable (`AncestorStream` has no public
    /// constructor). `context` supplies `proposal_view = ctx.round.view()`.
    async fn build_proposal<E: Clock>(
        &self,
        clock: &E,
        context: &SimplexContext<Digest, PublicKey>,
        parent: OrderBlock,
    ) -> Option<OrderBlock> {
        let height = parent.height + 1;

        // Read the pace cap's origin at view entry rather than after the sleep: the
        // leader timeout budgets one block interval, so time spent before pacing must
        // come out of it, not be added to it.
        let view_entered = clock.current();

        // Pace to one block per second: hold until wall clock reaches parent + 1s.
        // Cancellation-safe (Inline selects this future against `tx.closed()`).
        //
        // Capped at one interval from now: an uncapped sleep on a future-dated parent
        // would overrun the peers' leader deadline. The produced timestamp is still
        // parent + 1, so chain-time monotonicity is unaffected.
        let pace_target =
            std::time::UNIX_EPOCH + Duration::from_secs(parent.timestamp) + BLOCK_INTERVAL;
        let pace_cap = view_entered + BLOCK_INTERVAL;
        clock.sleep_until(pace_target.min(pace_cap)).await;

        // Execution gate: the result commitment needs the finalized-tier derived hash
        // at height − K, never the speculative head. A proposer whose local finalize
        // reconcile has not caught up skips the view rather than guessing. Sampled
        // after the pace sleep so the EL gets the full inter-block interval.
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

        // Stamp the production record naming this node: commonware invokes `propose`
        // only on the elected leader, so `context.leader` is us. Both non-`Some` arms
        // decline the view rather than propose a block our own verifier would reject;
        // `Ok(None)` is unreachable for a real proposer because `EpochEngine::new`
        // injects the map first.
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
        // At most one charge per block: each costs every voter two BLS verifies
        // outside `VERIFY_EXEC_BUDGET`. The charge is drawn from this block's epoch
        // (the round's, not `epoch_of(height)`) because only that epoch's committee
        // can verify it; a charge that outlives its epoch leaves by the transaction
        // fallback. If the block fails to gather a quorum the charge stays queued and
        // the next proposer offers it again.
        // A charge names a committee index while a tombstone is recorded against a
        // peer key, so the round's `committee_index` maps between them. Without a map
        // nothing is filtered, the same permissive direction the leader-index rule
        // takes.
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
            // Rule SA: self-attest the view this block is proposed in, making
            // `Round(epoch(h), proposal_view)` — the round this block's σ resolves at —
            // agreed data rather than a per-node guess.
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

/// Σ tx.gas_limit with overflow as `None` — the one stateless tx bound verify
/// enforces. Signature, chain-id and nonce validity are not checked: the
/// deterministic skip rule in derivation handles them identically on every
/// node, and checking them would add per-tx ECDSA work without bounding
/// anything the gas cap does not already bound.
fn total_tx_gas(txs: &[TransactionSigned]) -> Option<u64> {
    txs.iter()
        .try_fold(0u64, |acc, tx| acc.checked_add(tx.gas_limit()))
}

/// Equivocation gate (returns `false` to vote against): the accusation in
/// `extra_data` and the evidence in `OrderBlock::equivocation` must be present
/// together, and when present the evidence must convict exactly the accused
/// member of this block's epoch. Presence is exact in both directions: an
/// accusation without evidence is uncheckable, evidence without an accusation
/// is unagreed payload under the digest.
///
/// `committee_index == None` skips only the cryptographic arm; the presence
/// rule needs no committee and is enforced regardless.
fn equivocation_gate_decision(
    block: &OrderBlock,
    epoch: u64,
    committee_index: Option<&Arc<BiMap<PeerPubkey, BlsPubkey>>>,
    chain_id: u64,
) -> bool {
    // An absent or undecodable record names nobody, so no evidence may ride with
    // it; `structural_checks` already rejected both for any instance that votes.
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
            // commonware invokes `propose` only on the elected leader, so this fires once
            // per block this node proposes.
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
        // Inline seeds the stream `[block, parent]`, so both `next()` calls return
        // buffered. At the boundary the parent is the previous epoch's terminal
        // block, whose body every node running the `E+1` engine already holds.
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
    /// The whole vote decision over a `(block, parent)` pair, factored out of the
    /// trait `verify` so the gate is unit-testable (`AncestorStream` has no public
    /// constructor).
    async fn verify_block<E: Clock>(
        &self,
        clock: &E,
        ctx: &SimplexContext<Digest, PublicKey>,
        block: &OrderBlock,
        parent: &OrderBlock,
    ) -> bool {
        // A slashed member keeps its seat and its leader slots until the committee
        // turns over, and binding its proposal clears the round's leader deadline, so
        // equivocating costs the full certification deadline. Refusing here is the
        // node's one seam before certification: a `false` verify times the view out at
        // once.
        //
        // This vote decision reads a non-hash-invariant field deliberately: a node
        // that has not yet seen the verdict votes as before, so the populations
        // disagree only on nullification, never on which block is final.
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
        // The production record names its own producer, so it is checkable at vote
        // time against `ctx.leader`, which is consensus-supplied agreed data. `Err` is
        // a reject, never a skip; folding it into the permissive `Ok(None)` arm would
        // invert the rule.
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

        // The record is checked against `ctx.leader` and nothing else. A verify-time
        // byte-compare against this node's own marshal-archived finalization would be
        // unsound: `assemble` keeps any ≥quorum attestation set and
        // `verify_certificate` accepts any ≥quorum bitmap, so honest nodes hold
        // byte-different certs for the same round and a byte-exact compare would
        // false-reject honest proposals. `ctx.leader` is agreed; a local archive is
        // not.

        // The charge is verified against this round's committee — the only one whose
        // signer-index to BLS-key mapping a running node can reconstruct.
        if !equivocation_gate_decision(
            block,
            ctx.round.epoch().get(),
            self.committee_index.as_ref(),
            self.chain_id,
        ) {
            return false;
        }

        // On a same-epoch link `parent.proposal_view` and `ctx.parent.0` are provably
        // equal but come from different sources (block body vs simplex context), so a
        // mismatch is a block certified at a view it did not claim. `proposal_view` is
        // still the key the executor resolves σ at.
        if ctx.parent.0 != View::zero() && parent.proposal_view != ctx.parent.0.get() {
            metrics::counter!("dpos_parent_view_mismatch_total").increment(1);
            return false;
        }

        // The result-gate poll loop. The common path resolves on tick 0 with no sleep;
        // a definitive `false` returns at once, and an unresolved result at the
        // deadline votes false. The gate samples the finalized tier — "wait for h−K to
        // be finalized-reconciled locally", not "match the speculative head".
        let check = |this: &Self| {
            result_matches(
                block.result,
                block.height,
                this.dpos_activation_block,
                |h| this.executed.finalized_executed_hash(h),
            )
        };
        let mut result_done = false;
        // Wall time this verify spends waiting on the finalized hash at h−K; zero on
        // the common tick-0 resolve. Recorded once per verify at the result arm's
        // terminal.
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
                        // Log once per verify and, via the shared cursor, at most once per height: a
                        // height is re-verified across views and the saturation signal is per-height.
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
            // Budget exhausted with no finalized h−K: finalization is lagging the gate.
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
        // The boundary hook and the assembler observe `Update::Block` only.
        if let Update::Block(ref block, _) = activity {
            self.assembler.observe_finalized(block);
            (self.boundary_hook)(block.clone());
        }
        // The gauge is observability only, but the watch is not: it is the process's
        // clock and the only one that keeps moving once this node's execution stalls.
        // The tip still travels to the executor untouched below either way.
        if let Update::Tip(_, height, _) = &activity {
            self.plane_clock.record_ordering_tip(height.get());
            // `send_replace` publishes value and wake-up in one call, unconditionally: the
            // marshal reports a tip only when it rises, and the live epoch is a function
            // of the current tip, so a consumer that missed the last publish would hold a
            // stale epoch until the next one.
            self.ordering_tip.send_replace(height.get());
            // The beacon plane's channel, written immediately after in the same statement:
            // one writer, two channels, a fixed order.
            if let Some(beacon_tip) = &self.beacon_tip {
                beacon_tip.send_replace(height.get());
            }
        }
        // The `Exact` ack inside `Update::Block` travels inside this command and is
        // fired by the executor after derive and import; a dropped ack trips marshal's
        // supervisor cascade.
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

    /// Drive the fork choice. The verdict (including a semantic
    /// `PayloadStatusEnum::Invalid`) rides in `Ok`; a failure that produced no
    /// verdict is the typed [`EngineError`] in `Err`. The split is type-level, so
    /// the executor's fork-safety rule is a property of the return type.
    ///
    /// `EngineError` carries its own [`crate::fault::FaultClass`] so an
    /// implementation can distinguish "reth never processed this" (retry) from
    /// "reth rejected the forkchoice state" (permanent local inconsistency).
    fn fork_choice_updated(
        &self,
        state: ForkchoiceState,
    ) -> impl std::future::Future<Output = Result<ForkchoiceUpdated, EngineError>> + Send;

    /// Import one derived block into the EL. Implementations either hand reth the
    /// pre-executed artifacts (`InsertExecutedBlock`, single execution) or fall
    /// back to `new_payload` (reth re-executes). Same no-verdict-vs-verdict split
    /// as [`Self::fork_choice_updated`], so both engine entry points get the same
    /// transport fault class.
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
    /// seed; `Some(false)` = a beacon-active block fell back to `order.digest()`;
    /// `None` = pre-beacon / no seed. Defaults to `None`.
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

/// Typed "parent header not readable yet" derivation failure. reth
/// canonicalizes imports on the engine-tree thread, so a block can be added to
/// the canonical chain milliseconds before provider reads see its header; a
/// recovery path deriving against such a parent must tell this transient race
/// from a real failure.
#[derive(Debug, thiserror::Error)]
#[error("derive: parent header {0} not found")]
pub struct ParentHeaderMissing(pub B256);

/// Typed "a gap-walk prefix element on a beacon-active round has no σ yet"
/// derivation failure. The walk owns neither the cause nor the ack, so it
/// cannot park; its caller classifies this leaf as it does
/// [`ParentHeaderMissing`]. Deriving with the `order.digest()` fallback
/// instead would re-roll `prev_randao` against the round the network used.
#[derive(Debug, thiserror::Error)]
#[error(
    "derive gap: no seed recorded for beacon-active height {height} (round view {proposal_view})"
)]
pub struct PrefixSeedMissing {
    pub height: u64,
    pub proposal_view: u64,
}

/// Derivation with a bounded retry on the parent-visibility race above. Any
/// path that derives against a parent imported without an awaited
/// canonicalization in between must make the parent visible first; both walks
/// do it. This absorbs the narrower race where the parent is imported
/// concurrently by someone else, and is not a substitute for sending the
/// canonicalization FCU. Any other derivation error stays immediately fatal.
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

/// Deterministic `OrderBlock` → derived-EVM-block execution: every node must
/// compute a byte-identical derived block for the same `(order, parent)` — the
/// function whose output the committee's `result` agreement attests. The node
/// crate implements it over reth-evm's `BlockBuilder`.
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

    // Tier-F resolves reth's canonical hash at or below the cursor and `None`
    // above it; `advance` is monotone; a provider miss at or below the cursor
    // returns `None`, never a wrong hash; and the cursor is visible across
    // clones.
    #[test]
    fn finalized_cursor_resolves_canonical_at_or_below_and_none_above() {
        let cursor = FinalizedCursor::default();
        let reader = cursor.clone();
        let canonical = |h: u64| (h <= 20).then(|| B256::repeat_byte(h as u8));
        cursor.advance(10);

        assert_eq!(reader.resolve(10, canonical), Some(B256::repeat_byte(10)));
        assert_eq!(reader.resolve(3, canonical), Some(B256::repeat_byte(3)));
        assert_eq!(reader.resolve(11, canonical), None);
        cursor.advance(12);
        assert_eq!(reader.resolve(12, canonical), Some(B256::repeat_byte(12)));
        assert_eq!(reader.resolve(13, canonical), None);
        cursor.advance(5);
        assert_eq!(
            reader.resolve(13, canonical),
            None,
            "cursor did not regress"
        );
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

    /// A committee BiMap in the same shape production builds, so the index
    /// these tests assert against is commonware's sorted order, not insertion
    /// order.
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

    /// The BLS keypairs [`committee_bimap`] puts in the map, in the same order,
    /// so a test that signs as a committee member holds the secrets behind the
    /// very pubkeys the map was built from.
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

    /// A committee containing the leader every propose fixture elects
    /// (`from_seed(7)`), so a proposing app can resolve its own index and
    /// stamp a production record; without it `build_proposal` declines the
    /// view, as it would in production when no map is injected.
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

    /// The three states must stay distinct: folding "the leader is not in the
    /// map" into "there is no map" silently turns a reject into a skip on the
    /// vote path, which is why the return type is `Result<Option<_>, _>`.
    #[test]
    fn expected_leader_index_keeps_its_three_states_apart() {
        let (executor, _rx) = fresh_mailbox();
        let hook: Arc<dyn Fn(OrderBlock) + Send + Sync> = Arc::new(|_| {});
        let (keys, committee) = test_committee(5, 7);
        let leader = keys[0].public_key();

        let bare = build_app(executor.clone(), hook.clone());
        assert_eq!(bare.expected_leader_index(&leader), Ok(None));

        let bimap = Arc::new(committee);
        let seated = build_app(executor.clone(), hook.clone()).with_committee_index(bimap.clone());
        let expected = bimap.position(&leader).expect("leader is a member") as u8;
        assert_eq!(seated.expected_leader_index(&leader), Ok(Some(expected)));

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

    /// The `None` arm is permissive by design, and the `Some` arm must reject
    /// the empty field the executor is required to tolerate.
    #[test]
    fn production_record_rule_arms() {
        let good = extra_data::encode_production_record(3, None);

        assert!(production_record_ok(&good, None));
        assert!(production_record_ok(&[], None));
        assert!(production_record_ok(&[0xAB; 24], None));

        assert!(production_record_ok(&good, Some(3)));
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
            // Tests anchor at activation, so the pre-activation window is
            // unchanged by the anchor/activation split.
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

    // The honest common path burns no budget: the result gate resolves on
    // tick 0 with no sleep.
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

    // The production-record rule through the real `verify_block` wiring: every
    // other gate test builds an app with no committee map, so the rule
    // short-circuits before it can reject anything, and this pins the wiring
    // that carries `ctx.leader` into it.
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
    /// disarms the leader timer. Refusing is the node's one seam before
    /// certification: a `false` verify times the view out immediately.
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

    /// The refusal is driven from chain state, not from evidence a node
    /// happened to hold, so it survives a restart: everything below is built
    /// fresh and it still arms from the committee snapshot alone.
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

    /// Both halves of the `Some(i)` arm at the gate: a record naming another
    /// member, and the empty field the executor is separately required to
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

    /// The `Err` arm is a reject, never a skip: an armed voter whose map does
    /// not seat the round leader votes false rather than falling through to
    /// the permissive no-map path.
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

    // Rule SA: a block lying about its own proposal view is rejected with
    // everything else valid.
    #[test]
    fn a_block_lying_about_its_own_proposal_view_is_rejected() {
        let (parent, block) = witness_pair(4, 8 /* lies: certified view is 9 */);
        let ctx = ctx_same_epoch(5, 9, &parent);
        let app = witness_app(NoChain);
        assert!(!run_gate(app, ctx, block, parent).0);
    }

    /// An executed chain whose hash becomes available only after N
    /// `finalized_executed_hash` polls — models "execution reaches h − K
    /// mid-verify".
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

    /// The result gate re-reads across ticks: a chain that only answers on the
    /// 5th poll still verifies true, spending real budget to do it.
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
    /// whole budget and votes false rather than accepting an unchecked result.
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

    // The anchor link: `Ec == 0` with the genesis-view parent sentinel is the
    // chain anchor, and the first post-activation block verifies over it.
    #[test]
    fn anchor_link_verifies_over_it() {
        let (parent, block) = witness_pair(0, 1);
        let ctx = ctx_boundary(0, 1, &parent);
        let app = witness_app(NoChain);
        assert!(run_gate(app, ctx, block, parent).0);
    }

    // On a same-epoch link the parent's self-attested view must agree with the
    // simplex context's parent view; a mismatch is a block certified at a view
    // it did not claim.
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

    /// A proposal carries exactly the 3-byte record naming its own proposer,
    /// which its voters recompute from `ctx.leader`. Asserting the bytes
    /// matters because the executor feeds `leader_index` straight to
    /// `recordProduction`, so a wrong index mis-credits production silently.
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
    /// ed25519 seed 8, deliberately not the fixture leader, so the accused and
    /// the producer are different members.
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

    /// A proposer that holds a charge stamps the verdict into `extra_data` and
    /// the evidence into the block, and its own vote-time gate accepts what it
    /// just built; asserting both halves together is the test.
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
            // A charge the proposer does not hold for this epoch is not
            // offered: only the epoch's own committee could verify it.
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
    /// leaving it queued would occupy the one-charge-per-block slot ahead of
    /// every later charge for the same epoch, so the proposer drops it.
    #[test]
    fn a_charge_whose_verdict_already_landed_is_dropped_rather_than_re_offered() {
        let (accused, charge) = sample_charge(5, 9);
        let committee = propose_committee();
        let settled = committee
            .get(accused as usize)
            .expect("the accused is seated")
            .clone();
        // A second charge for the same epoch, seated above the settled one so
        // the walk has to get past it rather than stopping at the first key.
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
        // cryptographic arm — the presence rule needs no committee and still holds.
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
        assert!(!equivocation_gate_decision(
            &charged(Some(accused + 1), Some(evidence)),
            5,
            Some(&committee),
            TEST_CHAIN_ID
        ));
    }

    /// A leader that cannot name itself in its committee skips the view instead
    /// of proposing a block every honest voter would reject.
    #[test]
    fn a_leader_outside_its_own_committee_declines_to_propose() {
        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|rt| async move {
            // A committee that does not contain the fixture leader
            // (`from_seed(7)`).
            let (_outsiders, disjoint) = test_committee(3, 99);
            let app = propose_app(None).with_committee_index(Arc::new(disjoint));
            let parent = tiny_parent(4);
            let ctx = propose_ctx(5, 9, (4, false), &parent);
            assert!(app.build_proposal(&rt, &ctx, parent).await.is_none());
        });
    }

    // Rule SA: every proposal self-attests its view.
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
    // advances virtual time in 1 ms cycles, so a sleep to a realistic
    // unix-seconds target never completes.
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

            // Clock at the parent's timestamp: the pace sleep must carry it to
            // parent+1, and the timestamp lands exactly there.
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

            // Proposer clock lags the parent's timestamp: the sleep must cap at
            // one `BLOCK_INTERVAL` from now, not parent+1, or the peers' leader
            // deadline would expire first.
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
            // The content timestamp still extends the parent chain.
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

            // A late proposer is already past parent+1: no extra sleep,
            // timestamp = now.
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

    /// Every leg runs with the record rule armed (`Some`), which is what a
    /// voter always passes: `expected_leader_index` returns `Ok(None)` only for
    /// an instance that casts no vote.
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

        // A wrong length, and a well-formed record naming another member as
        // the producer.
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

    /// The epoch manager's live-epoch input has one writer, the marshal's
    /// BFT-attested tip: a height reported here has been verified and stored by
    /// the marshal, so no peer can supply a number the committee did not
    /// certify. The test shows the door locally — a delivered block moves
    /// nothing and a `Tip` publishes exactly its own height.
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

            // A finalized block at a height is not a tip: the block stream is
            // ack-gated on the executor and lags the attested frontier.
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

            // A clone of the app publishes into the same channel: a per-clone
            // channel would leave the manager subscribed to a writer nothing
            // drives.
            let mut clone = app.clone();
            <FluentApp<NoChain, NoTxs> as Reporter>::report(
                &mut clone,
                Update::Tip(round, Height::new(4_300), Digest(B256::ZERO)),
            )
            .await;
            assert_eq!(*tip.borrow(), 4_300, "a clone writes the same channel");
        });
    }

    // The ordering clock rides the BFT-attested tip, never block delivery:
    // delivery is ack-gated on the executor, so a gauge fed from
    // `Update::Block` would freeze with the pipeline it exists to expose.
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

    /// The watch handed in by `with_beacon_tip` is written by the same
    /// `Update::Tip` that writes the app's own: the actor holds a receiver
    /// taken before the app existed, the manager subscribes afterwards, and one
    /// tip must reach both. A dropped sender would leave the actor on a channel
    /// nothing writes.
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
