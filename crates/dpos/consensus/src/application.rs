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
        actor::DETERMINISTIC_BOOTSTRAP_EPOCH,
        ceremony::CeremonyOutput,
        outcome::{encode_outcome, parse_outcome, validate_share_on_poly},
        seed::Seed,
    },
    digest::Digest,
    executor, extra_data,
    order_block::{result_matches, result_target, OrderBlock, ResultTarget, TX_BYTE_BUDGET},
};
use alloy_consensus::Transaction as _;
use alloy_primitives::{Address, Bytes, B256};
use alloy_rpc_types_engine::{ForkchoiceState, ForkchoiceUpdated, PayloadStatus};
use commonware_consensus::{
    marshal::{
        ancestry::{AncestorStream, BlockProvider},
        core::Mailbox as MarshalMailbox,
        standard::Standard,
        Update,
    },
    simplex::types::Context as SimplexContext,
    types::{Height, Round},
    Application, Reporter, VerifyingApplication,
};
use commonware_cryptography::{
    bls12381::primitives::group::Share, certificate::Signers, ed25519::PublicKey,
};
use commonware_runtime::{Clock, Metrics, Spawner};
use commonware_utils::ordered::Set;
/// The signing scheme bound for this Application.
pub use fluentbase_bls::Scheme as BlsScheme;
use fluentbase_bls::PeerPubkey;
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
/// the view nullifies. Consensus rule — MUST be uniform across nodes.
pub const TIMESTAMP_FUTURE_TOLERANCE_SECS: u64 = 1;

/// EIP-1559 hard floor for a header gas limit.
pub const MIN_GAS_LIMIT: u64 = 5_000;

/// Read-side view of the local derived chain, shared by propose/verify and
/// the executor. Implemented in the node crate over reth's provider — hash
/// strictly by NUMBER, never `best_number` (its semantics flip between
/// tree-sync and pipeline backfill).
pub trait ExecutedChain: Clone + Send + Sync + 'static {
    /// Highest derived + canonicalized height.
    fn executed_tip(&self) -> u64;
    /// Canonical EVM hash of the derived block at `height`.
    fn executed_hash(&self, height: u64) -> Option<B256>;
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

/// Resolves committee[epoch] (the Commonware-ordered peer set) at the current
/// finalized state — provided by the launch site over the staking reader.
pub type CommitteeForEpoch = Arc<dyn Fn(u64) -> Option<Set<PeerPubkey>> + Send + Sync>;

/// Resolves this node's memoized DKG result for an epoch it is a MEMBER of:
/// the agreed `Output` (`PK_E`) + this node's share. `None` ⇒ observer / share
/// not produced (⇒ withhold the qualifying beacon vote). Provided by the launch
/// site over the live-DKG `CeremonyStore`.
pub type BeaconForEpoch = Arc<dyn Fn(u64) -> Option<(CeremonyOutput, Share)> + Send + Sync>;

/// The per-epoch beacon-DKG context threaded into [`FluentApp`]'s verify/propose
/// path: the boundary "C" share-on-polynomial qualification + the proposer's
/// `beacon_outcome` assertion. `None` on `FluentApp` ⇒ no beacon context
/// (cold-start epoch 0 / followers / tests) ⇒ the beacon gate is a no-op.
#[derive(Clone)]
pub struct BeaconVerify {
    beacon_for_epoch: BeaconForEpoch,
    committee_for: CommitteeForEpoch,
    dpos_activation: u64,
    epoch_interval: u64,
    /// DEVNET/TEST-ONLY byzantine behaviour; `None` (and absent without the
    /// feature) on every honest node.
    #[cfg(feature = "dpos-devnet-byzantine")]
    byzantine: Option<crate::byzantine::ByzantineMode>,
}

impl BeaconVerify {
    pub fn new(
        beacon_for_epoch: BeaconForEpoch,
        committee_for: CommitteeForEpoch,
        dpos_activation: u64,
        epoch_interval: u64,
    ) -> Self {
        Self {
            beacon_for_epoch,
            committee_for,
            dpos_activation,
            epoch_interval,
            #[cfg(feature = "dpos-devnet-byzantine")]
            byzantine: None,
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
            matches!(self.byzantine, Some(crate::byzantine::ByzantineMode::ForgeBeaconPk))
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
    genesis: Arc<OrderBlock>,
    executor: executor::Mailbox,
    /// Observer for `Update::Block` finalizations — NOT a state-advancing
    /// path. Wired to the staking reader's epoch-boundary detection.
    boundary_hook: Arc<dyn Fn(OrderBlock) + Send + Sync>,
    /// Marshal handle for querying finalization certs (cross-epoch
    /// singleton owned by `OuterEngine`). `None` is acceptable for tests /
    /// followers that don't run the liveness pipeline.
    marshal: Option<MarshalMailbox<BlsScheme, Standard<OrderBlock>>>,
    /// Highest finalized block height observed via `Reporter::report`,
    /// stored as h+1 (0 = none yet). Read by `latest_finalized_cert`.
    latest_finalized_height: Arc<AtomicU64>,
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
}

impl<XC: Clone, A> Clone for FluentApp<XC, A> {
    fn clone(&self) -> Self {
        Self {
            beacon: self.beacon.clone(),
            genesis: self.genesis.clone(),
            executor: self.executor.clone(),
            boundary_hook: self.boundary_hook.clone(),
            marshal: self.marshal.clone(),
            latest_finalized_height: self.latest_finalized_height.clone(),
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
        marshal: Option<MarshalMailbox<BlsScheme, Standard<OrderBlock>>>,
        latest_finalized_height: Arc<AtomicU64>,
        executed: XC,
        assembler: Arc<A>,
        fee_recipient: Address,
        target_gas_limit: u64,
        dpos_activation_block: u64,
    ) -> Self {
        Self {
            beacon: None,
            genesis: Arc::new(genesis),
            executor,
            boundary_hook,
            marshal,
            latest_finalized_height,
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


    /// Returns the latest finalized cert's `(round, signers)`, if any.
    pub async fn latest_finalized_cert(&self) -> Option<(Round, Signers)> {
        let marshal = self.marshal.as_ref()?;
        let stored = self.latest_finalized_height.load(Ordering::Acquire);
        if stored == 0 {
            return None;
        }
        let h = stored - 1;
        let fin = marshal.get_finalization(Height::new(h)).await?;
        Some((fin.proposal.round, fin.certificate.vote.signers.clone()))
    }

    /// Pure structural validity of `block` against its parent — everything
    /// verify checks WITHOUT touching the local derived chain (`now_secs` is
    /// the verifier's clock, sampled by the caller). Parent linkage +
    /// contiguous height are already enforced by Inline's `validate_block`
    /// before app verify runs — not re-checked here.
    fn structural_checks(block: &OrderBlock, parent: &OrderBlock, now_secs: u64) -> bool {
        block.timestamp > parent.timestamp
            && block.timestamp <= now_secs + TIMESTAMP_FUTURE_TOLERANCE_SECS
            && gas_limit_within_1_1024(parent.gas_limit, block.gas_limit)
            && extra_data::decode_simplex_attestation(&block.extra_data).is_ok()
            && total_tx_gas(&block.txs).is_some_and(|gas| gas <= block.gas_limit)
    }

    /// Paced proposal body, factored out of `Application::propose` so the
    /// pacing/timestamp behavior is unit-testable (`AncestorStream` has no
    /// public constructor).
    async fn build_proposal<E: Clock>(&self, clock: &E, parent: OrderBlock) -> Option<OrderBlock> {
        let height = parent.height + 1;

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
            if bv.is_change_epoch_first_block(height, epoch) && (bv.beacon_for_epoch)(epoch).is_none()
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

        // Execution gate (proposer-≤K-behind): the result commitment needs
        // the local derived hash at height − K; a lagging proposer skips the
        // view rather than guessing. Sampled after the pace sleep — the EL
        // gets the full inter-block interval to reach height − K.
        let result = match result_target(height, self.dpos_activation_block) {
            ResultTarget::PreActivation => B256::ZERO,
            ResultTarget::Height(h) => match self.executed.executed_hash(h) {
                Some(hash) => hash,
                None => {
                    tracing::debug!(
                        height,
                        result_height = h,
                        executed_tip = self.executed.executed_tip(),
                        "execution lags result target; skipping propose"
                    );
                    return None;
                }
            },
        };

        let cert = self.latest_finalized_cert().await;

        let gas_limit = step_gas_limit(parent.gas_limit, self.target_gas_limit);
        let timestamp = clock
            .current()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before UNIX_EPOCH")
            .as_secs()
            .max(parent.timestamp + 1);
        let txs = self.assembler.assemble(height, gas_limit, TX_BYTE_BUDGET);

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

        let extra_data = Bytes::from(match cert {
            Some((round, signers)) => extra_data::encode_simplex_attestation(round, &signers),
            None => Vec::new(),
        });

        Some(OrderBlock {
            parent: parent.digest(),
            height,
            timestamp,
            fee_recipient: self.fee_recipient,
            gas_limit,
            extra_data,
            result,
            txs,
            beacon_outcome,
            beacon_seed: None,
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
            tracing::warn!(height = block.height, epoch, ?e, "beacon_outcome failed to parse");
            return false;
        }
    };
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
            tracing::debug!(epoch, "no epoch-E share — withholding beacon qualifying vote");
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
        self.build_proposal(&ctx.0, parent).await
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
        // next() calls return buffered, no marshal fetch.
        let Some(block) = ancestry.next().await else {
            return false;
        };
        let Some(parent) = ancestry.next().await else {
            return false;
        };

        let now_secs = ctx
            .0
            .current()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before UNIX_EPOCH")
            .as_secs();
        if !Self::structural_checks(&block, &parent, now_secs) {
            return false;
        }

        // The forge-able liveness bitmap is an OPEN DESIGN item — NOT defended
        // here. extra_data carries only the bitmap, not the signatures, so the
        // verifier cannot soundly re-verify the embedded quorum at verify-time;
        // and a byte-compare against this node's own marshal-archived
        // finalization is UNSOUND (the archive holds each node's first-observed,
        // locally-assembled cert — commonware `assemble` keeps any ≥quorum
        // attestation set and `verify_certificate` accepts any ≥quorum bitmap, so
        // honest nodes legitimately hold byte-DIFFERENT bitmaps for the same
        // round → byte-exact compare false-rejects honest proposals → nullify
        // storm / liveness stall; same class as the verify-gate
        // non-deterministic-cert-freeze hazard). The real defense must live
        // where the signatures/cert actually exist (the STF/zkVM that already
        // re-verifies the finalization cert), or be a deterministic committed
        // liveness bitmap, or liveness-slashing tolerant of per-block bitmap
        // variance via sustained-absence thresholds. Verify does structural
        // decode only (above). DO NOT re-introduce a verify-time byte-compare.

        // Beacon boundary gate: epoch-type (beacon_outcome present IFF change-epoch
        // first block) + the "C" share-on-polynomial qualification on a change block.
        if !beacon_gate_decision(self.beacon.as_ref(), &block) {
            return false;
        }

        // Result gate: bounded await for own execution to reach height − K,
        // then EXACT equality against the agreed commitment via the shared
        // `result_matches` cross-check. Timeout → false (backpressure: consensus
        // slows until execution catches up — the Monad "execution lags by at most
        // K" enforcement semantic).
        let check = |this: &Self| {
            result_matches(block.result, block.height, this.dpos_activation_block, |h| {
                this.executed.executed_hash(h)
            })
        };
        let polls = (VERIFY_EXEC_BUDGET.as_micros() / VERIFY_EXEC_POLL.as_micros()) as u32;
        for _ in 0..polls {
            if let Some(matches) = check(self) {
                return matches;
            }
            ctx.0.sleep(VERIFY_EXEC_POLL).await;
        }
        check(self).unwrap_or_else(|| {
            tracing::warn!(
                height = block.height,
                executed_tip = self.executed.executed_tip(),
                "verify exec budget exhausted; voting false (EL backpressure)"
            );
            false
        })
    }
}

impl<XC, A> Reporter for FluentApp<XC, A>
where
    XC: Clone + Send + Sync + 'static,
    A: OrderingAssembler,
{
    type Activity = Update<OrderBlock>;

    async fn report(&mut self, activity: Update<OrderBlock>) {
        match &activity {
            Update::Block(block, _) => {
                let h = block.height;
                // h+1 encoding: sentinel 0 = "no finalization yet";
                // fetch_max guards out-of-order delivery.
                self.latest_finalized_height
                    .fetch_max(h.saturating_add(1), Ordering::Release);
            }
            Update::Tip(..) => {}
        }

        // Boundary hook fires for `Update::Block` only — the epoch-boundary
        // detection integration point. The assembler observes the same block
        // so its in-flight suffix tracks ordered-but-unexecuted txs.
        if let Update::Block(ref block, _) = activity {
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

    fn fork_choice_updated(
        &self,
        state: ForkchoiceState,
    ) -> impl std::future::Future<Output = eyre::Result<ForkchoiceUpdated>> + Send;

    /// Import one derived block into the EL. Implementations either hand
    /// reth the pre-executed artifacts (`InsertExecutedBlock` — single
    /// execution) or fall back to `new_payload` (reth re-executes; the
    /// conformance/escape-hatch mode).
    fn import_derived(
        &self,
        data: Self::ExecutionData,
    ) -> impl std::future::Future<Output = eyre::Result<PayloadStatus>> + Send;
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
    use commonware_runtime::Runner as _;
    use std::sync::Mutex;

    #[derive(Clone, Default)]
    struct NoChain;
    impl ExecutedChain for NoChain {
        fn executed_tip(&self) -> u64 {
            0
        }
        fn executed_hash(&self, _height: u64) -> Option<B256> {
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
            timestamp: 1_700_000_000 + height,
            fee_recipient: Address::ZERO,
            gas_limit: 30_000_000,
            extra_data: Bytes::new(),
            result: B256::ZERO,
            txs: Vec::new(),
            beacon_outcome: None,
            beacon_seed: None,
        }
    }

    fn build_app(
        executor: executor::Mailbox,
        hook: Arc<dyn Fn(OrderBlock) + Send + Sync>,
    ) -> FluentApp<NoChain, NoTxs> {
        FluentApp::new(
            sample_order(Digest(B256::ZERO), 0),
            executor,
            hook,
            None,
            Arc::new(AtomicU64::new(0)),
            NoChain,
            Arc::new(NoTxs),
            Address::ZERO,
            30_000_000,
            // Tests anchor at activation (genesis.height == activation == 0),
            // so the pre-activation window is unchanged by the anchor/activation split.
            0,
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

    #[test]
    fn beacon_gate_epoch_type_and_share_on_poly() {
        use crate::beacon::dkg_oracle::run_local_dkg;
        use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
        use commonware_math::algebra::Random as _;
        use rand_08::rngs::StdRng;
        use rand_core::SeedableRng as _;

        let mut rng = StdRng::seed_from_u64(42);
        let keys: Vec<Ed25519PrivateKey> =
            (0..5).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
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
            let committee_for: CommitteeForEpoch = Arc::new(move |e: u64| match e {
                0 => Some(prev.clone()),
                _ => Some(cur.clone()),
            });
            let out_e = out.clone();
            let beacon_for_epoch: BeaconForEpoch = Arc::new(move |e: u64| {
                (e == 1)
                    .then(|| share.clone().map(|s| (out_e.clone(), s)))
                    .flatten()
            });
            BeaconVerify::new(beacon_for_epoch, committee_for, 0, 10)
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
        assert!(!beacon_gate_decision(Some(&bv), &block(10, Some(enc(&out_forged)))));
        // (c) epoch-type: change block missing the outcome → reject.
        assert!(!beacon_gate_decision(Some(&bv), &block(10, None)));
        // (d) epoch-type: outcome on a non-first block of the epoch → reject.
        assert!(!beacon_gate_decision(Some(&bv), &block(11, Some(enc(&out)))));
        // (f) observer (no share) on a change block → withhold.
        let bv_obs = make_bv(None, true);
        assert!(!beacon_gate_decision(Some(&bv_obs), &block(10, Some(enc(&out)))));
        // (e) carry-forward (committee unchanged): no outcome expected.
        let bv_cf = make_bv(Some(my_share), false);
        assert!(beacon_gate_decision(Some(&bv_cf), &block(10, None)));
        assert!(!beacon_gate_decision(Some(&bv_cf), &block(10, Some(enc(&out)))));
        // (g) no beacon context → no gating.
        assert!(beacon_gate_decision(None, &block(10, Some(enc(&out)))));
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
        let keys: Vec<Ed25519PrivateKey> =
            (0..5).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
        // STABLE committee: identical for every epoch (so on-change activation
        // would NEVER fire; only the deterministic epoch-2 bootstrap does).
        let committee: Set<PeerPubkey> = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let (out, shares) = run_local_dkg(&mut rng, b"ns", 2, &keys, &keys).expect("dkg");
        let my_share = shares.get(&keys[0].public_key()).expect("share").clone();

        let committee_for: CommitteeForEpoch = {
            let c = committee.clone();
            Arc::new(move |_e: u64| Some(c.clone()))
        };
        let out_e = out.clone();
        let beacon_for_epoch: BeaconForEpoch = Arc::new(move |e: u64| {
            (e == DETERMINISTIC_BOOTSTRAP_EPOCH).then(|| (out_e.clone(), my_share.clone()))
        });
        let bv = BeaconVerify::new(beacon_for_epoch, committee_for, 0, 10);

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
        assert!(!beacon_gate_decision(Some(&bv), &block(10, Some(enc(&out)))));
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
            let app = build_app(mailbox, Arc::new(|_b: OrderBlock| {}));
            let parent = tiny_ts_parent();

            // Clock at the parent's timestamp (synchronized proposer): the
            // pace sleep must carry it to parent+1 and the timestamp lands
            // exactly there.
            ctx.sleep_until(std::time::UNIX_EPOCH + Duration::from_secs(parent.timestamp))
                .await;
            let block = app
                .build_proposal(&ctx, parent.clone())
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
            let app = build_app(mailbox, Arc::new(|_b: OrderBlock| {}));
            let parent = tiny_ts_parent();

            // Proposer clock lags the parent's timestamp (skew within the
            // verify tolerance): the sleep must cap at one BLOCK_INTERVAL
            // from now — never parent+1 — or the peers' leader deadline
            // (which budgets pace ≤ BLOCK_INTERVAL) would expire first.
            let start = ctx.current();
            let block = app
                .build_proposal(&ctx, parent.clone())
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
            let app = build_app(mailbox, Arc::new(|_b: OrderBlock| {}));
            let parent = tiny_ts_parent();

            // A late proposer (slow/nullified prior views) is already past
            // parent+1: no extra sleep, timestamp = now.
            let late = parent.timestamp + 10;
            ctx.sleep_until(std::time::UNIX_EPOCH + Duration::from_secs(late))
                .await;
            let block = app.build_proposal(&ctx, parent).await.expect("proposed");
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
            let k0: Vec<Ed25519PrivateKey> =
                (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
            let k1: Vec<Ed25519PrivateKey> =
                (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
            let c0: Set<PeerPubkey> = Set::from_iter_dedup(k0.iter().map(|k| k.public_key()));
            let c1: Set<PeerPubkey> = Set::from_iter_dedup(k1.iter().map(|k| k.public_key()));
            // c0 != c1 ⇒ epoch 1's first block is a CHANGE-epoch boundary.
            let committee_for: CommitteeForEpoch = Arc::new(move |e: u64| match e {
                0 => Some(c0.clone()),
                _ => Some(c1.clone()),
            });
            // This node ran no live DKG ⇒ no CeremonyStore entry for any epoch.
            let beacon_for_epoch: BeaconForEpoch = Arc::new(|_e| None);
            // activation=0, interval=10 ⇒ epoch_start(1)=10, so proposed height 10
            // (parent 9 + 1) is the change-epoch first block.
            let bv = BeaconVerify::new(beacon_for_epoch, committee_for, 0, 10);

            let (mailbox, _rx) = fresh_mailbox();
            let app = build_app(mailbox, Arc::new(|_b: OrderBlock| {})).with_beacon(bv);

            let parent = sample_order(Digest(B256::ZERO), 9);
            let start = ctx.current();
            let decision = app.build_proposal(&ctx, parent).await;
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

    #[test]
    fn structural_checks_reject_each_violation() {
        let parent = sample_order(Digest(B256::ZERO), 1);
        let good = OrderBlock {
            parent: parent.digest(),
            ..sample_order(parent.digest(), 2)
        };
        let now = good.timestamp;
        assert!(FluentApp::<NoChain, NoTxs>::structural_checks(
            &good, &parent, now
        ));

        let stale_ts = OrderBlock {
            timestamp: parent.timestamp,
            ..good.clone()
        };
        assert!(!FluentApp::<NoChain, NoTxs>::structural_checks(
            &stale_ts, &parent, now
        ));

        let wild_gas = OrderBlock {
            gas_limit: parent.gas_limit * 2,
            ..good.clone()
        };
        assert!(!FluentApp::<NoChain, NoTxs>::structural_checks(
            &wild_gas, &parent, now
        ));

        let bad_extra = OrderBlock {
            extra_data: Bytes::from(vec![0xFF; 3]),
            ..good.clone()
        };
        assert!(!FluentApp::<NoChain, NoTxs>::structural_checks(
            &bad_extra, &parent, now
        ));
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
            &good, &parent, now
        ));

        // One second past the boundary: rejected.
        assert!(!FluentApp::<NoChain, NoTxs>::structural_checks(
            &good,
            &parent,
            now - 1
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
            // h+1 encoding: height 42 stores as 43.
            assert_eq!(app.latest_finalized_height.load(Ordering::Acquire), 43);
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
                Update::Tip(round, Height::new(0), Digest(B256::ZERO)),
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

    #[test]
    fn latest_finalized_cert_returns_none_when_marshal_unwired() {
        let runtime = commonware_runtime::deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (mailbox, _rx) = fresh_mailbox();
            let app = build_app(mailbox, Arc::new(|_b: OrderBlock| {}));
            app.latest_finalized_height.store(10, Ordering::Release);
            assert_eq!(app.latest_finalized_cert().await, None);
        });
    }
}
