//! This is temporary single-node consensus that is used for block production for Fluent,
//! it will be replaced with DPoS consensus later.
use alloy_consensus::BlockHeader;
use alloy_network::AnyNetwork;
use alloy_primitives::B256;
use alloy_rpc_types_engine::{ForkchoiceState, PayloadStatusEnum};
use eyre::OptionExt;
use reth_consensus_debug_client::{BlockProvider, RpcBlockProvider};
use reth_engine_primitives::ConsensusEngineHandle;
use reth_node_api::FullNodeComponents;
use reth_node_builder::{rpc::RethRpcAddOns, DebugNode, NodeHandle};
use reth_node_types::PayloadAttrTy;
use reth_payload_builder::PayloadBuilderHandle;
use reth_payload_primitives::{
    BuiltPayload, ExecutionPayload, PayloadAttributesBuilder, PayloadKind, PayloadTypes,
};
use reth_primitives_traits::{HeaderTy, NodePrimitives, SealedBlock, SealedHeaderFor};
use reth_storage_api::BlockReader;
use reth_tasks::shutdown::GracefulShutdown;
use std::{sync::Arc, time::Duration};
use tokio::{sync::mpsc, time::Interval};
use tracing::{debug, error, info, warn};

/// How often a persistent non-`VALID` forkchoice verdict is re-emitted while it
/// does not change. Long enough that a multi-hour backfill does not spam the
/// log, short enough that a parked node is never silent for a whole shift.
const VERDICT_REEMIT_INTERVAL: Duration = Duration::from_secs(60);

/// Re-readable probe for the governance-scheduled sequencer→DPoS activation
/// height. `None` = staking cluster not deployed / activation not scheduled.
/// Re-invoked per producer tick / received block: `setDposActivationBlock`
/// may re-schedule while pending, so a launch-time snapshot goes stale.
/// Callers latch the last `Some` — an on-chain `Some → None` transition is
/// impossible (the setter cannot store 0 on a live chain), so `None` after a
/// `Some` only ever means a transient read failure and must not un-gate.
pub type ActivationProbe = Arc<dyn Fn() -> Option<u64> + Send + Sync>;

pub async fn launch_consensus_validator<N, AddOns: RethRpcAddOns<N>, B>(
    handle: &NodeHandle<N, AddOns>,
    block_time: Duration,
    payload_attributes_builder: B,
    activation_probe: Option<ActivationProbe>,
) -> eyre::Result<()>
where
    N: FullNodeComponents<Types: DebugNode<N>>,
    B: PayloadAttributesBuilder<PayloadAttrTy<N::Types>, reth_node_types::HeaderTy<N::Types>>,
{
    let block_time = tokio::time::interval_at(tokio::time::Instant::now() + block_time, block_time);

    let blockchain_db = handle.node.provider.clone();
    let beacon_engine_handle = handle.node.add_ons_handle.beacon_engine_handle.clone();
    let payload_builder_handle = handle.node.payload_builder_handle.clone();

    let block_producer = BlockProducer::new(
        blockchain_db,
        payload_attributes_builder,
        payload_builder_handle,
        beacon_engine_handle,
        activation_probe,
    )?;

    handle
        .node
        .task_executor
        .spawn_critical_with_graceful_shutdown_signal(
            "consensus validator worker",
            move |shutdown| async move {
                block_producer.run(block_time, shutdown).await;
            },
        );
    Ok(())
}

// No `derive(Debug)`: the `ActivationProbe` closure is not `Debug`, and the
// producer is only ever moved into its worker task, never formatted.
pub struct BlockProducer<T: PayloadTypes, B> {
    to_engine: ConsensusEngineHandle<T>,
    payload_attributes_builder: B,
    payload_builder: PayloadBuilderHandle<T>,
    last_header: SealedHeaderFor<<T::BuiltPayload as BuiltPayload>::Primitives>,
    last_block_hash: B256,
    /// sequencer→DPoS migration clean-halt: stop producing once the head reaches
    /// the on-chain `dposActivationBlock` (DPoS consensus produces from
    /// activation+1). Re-probed each tick, latched on `Some` into
    /// [`Self::activation_gate`]. `None` probe ⇒ pure sequencer, never gates.
    activation_probe: Option<ActivationProbe>,
    activation_gate: Option<u64>,
}

impl<T: PayloadTypes, B> BlockProducer<T, B>
where
    B: PayloadAttributesBuilder<
        T::PayloadAttributes,
        HeaderTy<<T::BuiltPayload as BuiltPayload>::Primitives>,
    >,
{
    fn new(
        provider: impl BlockReader<Header = HeaderTy<<T::BuiltPayload as BuiltPayload>::Primitives>>,
        payload_attributes_builder: B,
        payload_builder: PayloadBuilderHandle<T>,
        to_engine: ConsensusEngineHandle<T>,
        activation_probe: Option<ActivationProbe>,
    ) -> eyre::Result<Self> {
        let best = provider.best_block_number().map_err(|e| {
            eyre::eyre!("BlockProducer: provider has no best block number (empty datadir?): {e}")
        })?;
        let last_header = provider
            .sealed_header(best)
            .map_err(|e| eyre::eyre!("BlockProducer: sealed_header(best) read failed: {e}"))?
            .ok_or_eyre("BlockProducer: no sealed header at best block — chain not initialized")?;
        let last_block_hash = last_header.hash();
        Ok(Self {
            to_engine,
            payload_attributes_builder,
            payload_builder,
            last_header,
            last_block_hash,
            activation_probe,
            activation_gate: None,
        })
    }

    pub async fn run(mut self, mut block_time: Interval, shutdown: GracefulShutdown) {
        let mut fcu_interval = tokio::time::interval(Duration::from_secs(1));
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                biased;

                guard = &mut shutdown => {
                    info!(target: "engine::local", "Shutting down consensus validator worker");
                    drop(guard);
                    break;
                }
                // Wait for the interval or the pool to receive a transaction.
                // If shutdown arrives while this future is in progress, shutdown will wait
                // until `advance_forkchoice_state()` finishes and only then exit the loop.
                _ = block_time.tick() => {
                    // sequencer→DPoS migration clean-halt: stop producing once the
                    // head reaches the on-chain activation block. DPoS
                    // consensus produces from activation+1.
                    if let Some(probe) = &self.activation_probe {
                        if let Some(act) = probe() {
                            self.activation_gate = Some(act);
                        }
                    }
                    if let Some(act) = self.activation_gate {
                        if self.last_header.number() >= act {
                            info!(
                                target: "engine::local",
                                activation = act,
                                "reached DPoS activation block; halting sequencer block \
                                 production (DPoS consensus produces from activation+1)"
                            );
                            break;
                        }
                    }
                    if let Err(e) = self.advance_forkchoice_state().await {
                        error!(target: "engine::local", "Error advancing the chain: {:?}", e);
                    }
                }
                // send FCU once in a while
                _ = fcu_interval.tick() => {
                    if let Err(e) = self.update_forkchoice_state().await {
                        error!(target: "engine::local", "Error updating fork choice: {:?}", e);
                    }
                }
            }
        }
    }

    async fn advance_forkchoice_state(&mut self) -> eyre::Result<()> {
        let res = self
            .to_engine
            .fork_choice_updated(
                ForkchoiceState::same_hash(self.last_block_hash),
                Some(self.payload_attributes_builder.build(&self.last_header)),
            )
            .await?;

        if !res.is_valid() {
            eyre::bail!("Invalid payload status")
        }

        let payload_id = res.payload_id.ok_or_eyre("No payload id")?;

        let Some(Ok(payload)) = self
            .payload_builder
            .resolve_kind(payload_id, PayloadKind::WaitForPending)
            .await
        else {
            eyre::bail!("No payload")
        };

        let header = payload.block().sealed_header().clone();
        let payload = T::block_to_payload(payload.block().clone());
        let res = self.to_engine.new_payload(payload).await?;

        if !res.is_valid() {
            eyre::bail!("Invalid payload")
        }

        self.last_block_hash = header.hash();
        self.last_header = header;

        Ok(())
    }

    async fn update_forkchoice_state(&mut self) -> eyre::Result<()> {
        let state = ForkchoiceState::same_hash(self.last_block_hash);
        let res = self.to_engine.fork_choice_updated(state, None).await?;
        if !res.is_valid() {
            eyre::bail!("Invalid fork choice update {state:?}: {res:?}")
        }
        Ok(())
    }
}

/// Launches the plain / trust-relay full node's block relay: subscribe to an
/// upstream RPC, hand every block it produces to this node's engine.
pub async fn launch_consensus_node<Node, AddOns: RethRpcAddOns<Node>>(
    handle: &NodeHandle<Node, AddOns>,
    consensus_url: String,
    activation_probe: Option<ActivationProbe>,
) -> eyre::Result<()>
where
    Node: FullNodeComponents<Types: DebugNode<Node>>,
{
    info!(target: "reth::cli", "Using RPC consensus client: {}", consensus_url);

    let block_provider =
        RpcBlockProvider::<AnyNetwork, _>::new(consensus_url.as_str(), |block_response| {
            let json =
                serde_json::to_value(block_response).expect("Block serialization cannot fail");
            let rpc_block =
                serde_json::from_value(json).expect("Block deserialization cannot fail");
            Node::Types::rpc_to_primitive_block(rpc_block)
        })
        .await?;

    let beacon_engine_handle = handle.node.add_ons_handle.beacon_engine_handle.clone();
    handle
        .node
        .task_executor
        .spawn_critical_task("consensus node worker", async move {
            new_block_fetcher(
                beacon_engine_handle,
                Arc::new(block_provider),
                activation_probe,
            )
            .await
        });
    Ok(())
}

const VERDICT_VALID: &str = "VALID";

/// Coarse name of a forkchoice verdict, used as the log latch key.
const fn verdict_kind(status: &PayloadStatusEnum) -> &'static str {
    match status {
        PayloadStatusEnum::Valid => VERDICT_VALID,
        PayloadStatusEnum::Accepted => "ACCEPTED",
        PayloadStatusEnum::Syncing => "SYNCING",
        PayloadStatusEnum::Invalid { .. } => "INVALID",
    }
}

/// Latch for the relay's forkchoice-verdict logging.
///
/// Keyed on the verdict kind AND the `validation_error` text: two `INVALID`s
/// with different reasons are different news and must both be reported, which a
/// discriminant-only latch would swallow.
struct VerdictLog {
    kind: &'static str,
    validation_error: Option<String>,
    /// Relay head at which this verdict was FIRST seen — the re-emit reports it
    /// so the operator can see how far the engine has fallen behind.
    since_number: u64,
    last_emit: std::time::Instant,
}

impl VerdictLog {
    fn new(kind: &'static str, validation_error: Option<&str>, number: u64) -> Self {
        Self {
            kind,
            validation_error: validation_error.map(str::to_owned),
            since_number: number,
            last_emit: std::time::Instant::now(),
        }
    }

    fn same_verdict(&self, kind: &'static str, validation_error: Option<&str>) -> bool {
        self.kind == kind && self.validation_error.as_deref() == validation_error
    }
}

async fn new_block_fetcher<
    P: BlockProvider + Clone,
    T: PayloadTypes<BuiltPayload: BuiltPayload<Primitives: NodePrimitives<Block = P::Block>>>,
>(
    engine_handle: ConsensusEngineHandle<T>,
    block_provider: P,
    activation_probe: Option<ActivationProbe>,
) {
    let mut block_stream = {
        let (tx, rx) = mpsc::channel::<P::Block>(64);
        let block_provider = block_provider.clone();
        tokio::spawn(async move {
            block_provider.subscribe_blocks(tx).await;
        });
        rx
    };

    // Two-tier finality mirror (DPoS era only): an upstream block at height
    // N > activation is INCLUSION-level — its execution result becomes
    // committee-attested K blocks later (deferred execution). Finalizing on
    // receipt would overclaim by K and permanently desync this node's
    // `finalized` tag from the validators'. Lag finalized by K, clamped to
    // the activation anchor (the validators' own floor); pre-activation
    // (sequencer-era / activation not scheduled yet) keeps finalize-on-receipt.
    // The engine-API `safe` tag rides the latest landed ordering-final tip
    // (`block_hash` — there is no speculative lead in the importer, each landed
    // block is ordering-final on arrival), matching the validators' executor:
    // `safe = head = block_hash`, `finalized` K behind. Ancestry `finalized ⊆
    // safe ⊆ head` holds trivially.
    // Activation is re-probed per block and latched on `Some` so a node
    // launched before `setDposActivationBlock` still picks it up.
    let mut two_tier_activation: Option<u64> = None;
    let mut recent: std::collections::BTreeMap<u64, B256> = std::collections::BTreeMap::new();
    // Forkchoice verdict log state. The FCU is the only channel through which a
    // rejection can reach an operator, so it is EDGE-triggered (a running
    // backfill answers `SYNCING` for every block for as long as it lasts, and
    // one line per block would bury the transition that matters) but NOT
    // one-shot: a node parked on `SYNCING` or `INVALID` re-emits on
    // [`VERDICT_REEMIT_INTERVAL`], with the relay head so the gap is visible.
    //
    // That re-emit is the only signal for a real dead end: the relay never
    // re-sends a height it has passed, so a block the engine could not attach
    // is only ever recovered by reth's own buffering (`try_buffer_payload`) or
    // by re-downloading it over devp2p — and a relay with no reachable peer can
    // stay behind indefinitely.
    let mut verdict_log: Option<VerdictLog> = None;
    while let Some(block) = block_stream.recv().await {
        if let Some(probe) = &activation_probe {
            if let Some(act) = probe() {
                two_tier_activation = Some(act);
            }
        }
        let payload = T::block_to_payload(SealedBlock::new_unhashed(block));
        let block_hash = payload.block_hash();
        let number = payload.block_number();
        recent.insert(number, block_hash);
        recent.retain(|n, _| n.saturating_add(64) > number);
        let finalized = match two_tier_activation {
            Some(activation) if number > activation => {
                let result_final = fluentbase_consensus::result_final_height(number, activation);
                recent
                    .range(..=result_final)
                    .next_back()
                    .map(|(_, h)| *h)
                    .unwrap_or(B256::ZERO)
            }
            _ => block_hash,
        };
        // Send new events to execution client
        match engine_handle.new_payload(payload).await {
            Ok(status) => match &status.status {
                PayloadStatusEnum::Valid => {}
                PayloadStatusEnum::Invalid { validation_error } => error!(
                    target: "reth::cli",
                    number,
                    %block_hash,
                    %validation_error,
                    "consensus relay: engine rejected the relayed block as INVALID"
                ),
                // SYNCING / ACCEPTED while catching up is expected; the
                // forkchoice verdict below is the load-bearing signal.
                other => debug!(
                    target: "reth::cli",
                    number,
                    %block_hash,
                    status = ?other,
                    "consensus relay: newPayload did not yet accept the relayed block"
                ),
            },
            Err(e) => {
                verdict_log = None;
                error!(
                    target: "reth::cli",
                    number,
                    %block_hash,
                    error = %e,
                    "consensus relay: newPayload never reached the engine"
                );
            }
        }
        let state = ForkchoiceState {
            head_block_hash: block_hash,
            safe_block_hash: block_hash,
            finalized_block_hash: finalized,
        };
        match engine_handle.fork_choice_updated(state, None).await {
            Ok(res) => {
                let verdict = verdict_kind(&res.payload_status.status);
                let validation_error = match &res.payload_status.status {
                    PayloadStatusEnum::Invalid { validation_error } => {
                        Some(validation_error.as_str())
                    }
                    _ => None,
                };
                if let Some(state) = verdict_log
                    .as_mut()
                    .filter(|s| s.same_verdict(verdict, validation_error))
                {
                    // Unchanged verdict. Stay quiet while healthy; while stuck,
                    // re-emit on the interval with how far back the relay first
                    // saw this verdict, so the gap is visible.
                    if verdict != VERDICT_VALID
                        && state.last_emit.elapsed() >= VERDICT_REEMIT_INTERVAL
                    {
                        state.last_emit = std::time::Instant::now();
                        warn!(
                            target: "reth::cli",
                            relay_head = number,
                            %block_hash,
                            verdict,
                            validation_error,
                            since_block = state.since_number,
                            blocks_stuck = number.saturating_sub(state.since_number),
                            "consensus relay: engine STILL not following the relayed head \
                             — the relay never re-sends a height it has passed, so this \
                             only clears once reth attaches the buffered block or \
                             re-downloads it over devp2p"
                        );
                    }
                } else {
                    verdict_log = Some(VerdictLog::new(verdict, validation_error, number));
                    match &res.payload_status.status {
                        PayloadStatusEnum::Valid => info!(
                            target: "reth::cli",
                            relay_head = number,
                            %block_hash,
                            "consensus relay: engine is following the relayed head"
                        ),
                        PayloadStatusEnum::Invalid { validation_error } => error!(
                            target: "reth::cli",
                            relay_head = number,
                            %block_hash,
                            %validation_error,
                            "consensus relay: engine REJECTED the relayed head — this node \
                             will not advance until this is resolved"
                        ),
                        _ => warn!(
                            target: "reth::cli",
                            relay_head = number,
                            %block_hash,
                            verdict,
                            "consensus relay: engine is not following the relayed head \
                             (backfill running, or the block is still buffered)"
                        ),
                    }
                }
            }
            Err(e) => {
                verdict_log = None;
                error!(
                    target: "reth::cli",
                    number,
                    %block_hash,
                    error = %e,
                    "consensus relay: forkchoice update never reached the engine"
                );
            }
        }
    }
}
