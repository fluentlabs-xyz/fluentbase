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
    digest::Digest,
    executor, extra_data,
    order_block::{result_target, OrderBlock, ResultTarget, TX_BYTE_BUDGET},
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
use commonware_cryptography::{certificate::Signers, ed25519::PublicKey};
use commonware_runtime::{Clock, Metrics, Spawner};
/// The signing scheme bound for this Application.
pub use fluentbase_bls::Scheme as BlsScheme;
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
    fn assemble(&self, height: u64, gas_limit: u64, byte_budget: usize)
        -> Vec<TransactionSigned>;

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

/// The Fluent consensus application.
///
/// Generic over `XC` (local derived-chain view) and `A` (tx assembler).
pub struct FluentApp<XC, A> {
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
    /// Ordering-chain genesis height ([`crate::order_block::anchor_order_block`]);
    /// origin of the `result_target` pre-activation window.
    anchor_height: u64,
}

impl<XC: Clone, A> Clone for FluentApp<XC, A> {
    fn clone(&self) -> Self {
        Self {
            genesis: self.genesis.clone(),
            executor: self.executor.clone(),
            boundary_hook: self.boundary_hook.clone(),
            marshal: self.marshal.clone(),
            latest_finalized_height: self.latest_finalized_height.clone(),
            executed: self.executed.clone(),
            assembler: self.assembler.clone(),
            fee_recipient: self.fee_recipient,
            target_gas_limit: self.target_gas_limit,
            anchor_height: self.anchor_height,
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
    ) -> Self {
        let anchor_height = genesis.height;
        Self {
            genesis: Arc::new(genesis),
            executor,
            boundary_hook,
            marshal,
            latest_finalized_height,
            executed,
            assembler,
            fee_recipient,
            target_gas_limit,
            anchor_height,
        }
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
        Some((fin.proposal.round, fin.certificate.signers.clone()))
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
        let result = match result_target(height, self.anchor_height) {
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

        let extra_data = Bytes::from(match self.latest_finalized_cert().await {
            Some((round, signers)) => extra_data::encode_simplex_attestation(round, &signers),
            None => Vec::new(),
        });

        let gas_limit = step_gas_limit(parent.gas_limit, self.target_gas_limit);
        let timestamp = clock
            .current()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before UNIX_EPOCH")
            .as_secs()
            .max(parent.timestamp + 1);
        let txs = self.assembler.assemble(height, gas_limit, TX_BYTE_BUDGET);

        Some(OrderBlock {
            parent: parent.digest(),
            height,
            timestamp,
            fee_recipient: self.fee_recipient,
            gas_limit,
            extra_data,
            result,
            txs,
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

        // Result gate: bounded await for own execution to reach height − K,
        // then EXACT equality against the agreed commitment. Timeout → false
        // (backpressure: consensus slows until execution catches up — the
        // Monad "execution lags by at most K" enforcement semantic).
        match result_target(block.height, self.anchor_height) {
            ResultTarget::PreActivation => block.result == B256::ZERO,
            ResultTarget::Height(h) => {
                let polls = (VERIFY_EXEC_BUDGET.as_micros() / VERIFY_EXEC_POLL.as_micros()) as u32;
                for _ in 0..polls {
                    if let Some(local) = self.executed.executed_hash(h) {
                        return block.result == local;
                    }
                    ctx.0.sleep(VERIFY_EXEC_POLL).await;
                }
                match self.executed.executed_hash(h) {
                    Some(local) => block.result == local,
                    None => {
                        tracing::warn!(
                            height = block.height,
                            result_height = h,
                            executed_tip = self.executed.executed_tip(),
                            "verify exec budget exhausted; voting false (EL backpressure)"
                        );
                        false
                    }
                }
            }
        }
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
) -> eyre::Result<D::Derived>
where
    C: commonware_runtime::Clock,
    D: DerivedBlockBuilder,
{
    const RETRY: Duration = Duration::from_millis(100);
    const DEADLINE: Duration = Duration::from_secs(10);
    let deadline = ctx.current() + DEADLINE;
    loop {
        match deriver.derive_and_execute(order.clone(), parent_hash).await {
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

            assert_eq!(counter.load(Ordering::SeqCst), 0, "hook must NOT fire on Tip");
            let msg = rx.lock().unwrap().try_recv().expect("Finalize sent");
            match msg.command {
                executor::Command::Finalize(update) => {
                    assert!(matches!(*update, Update::Tip(..)));
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
