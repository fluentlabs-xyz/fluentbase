//! This is temporary single-node consensus that is used for block production for Fluent,
//! it will be replaced with DPoS consensus later.
use alloy_consensus::BlockHeader;
use alloy_network::AnyNetwork;
use alloy_primitives::B256;
use alloy_rpc_types_engine::ForkchoiceState;
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
use tracing::{error, info};

/// Re-readable probe for the governance-scheduled Tempo→DPoS activation
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
    /// Tempo→DPoS migration clean-halt: stop producing once the head reaches
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
                    // Tempo→DPoS migration clean-halt: stop producing once the
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
                                "reached DPoS activation block; halting Tempo block \
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
    // (Tempo era / activation not scheduled yet) keeps finalize-on-receipt.
    // Activation is re-probed per block and latched on `Some` so a node
    // launched before `setDposActivationBlock` still picks it up.
    let mut two_tier_activation: Option<u64> = None;
    let mut recent: std::collections::BTreeMap<u64, B256> = std::collections::BTreeMap::new();
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
        let _ = engine_handle.new_payload(payload).await;
        let state = ForkchoiceState {
            head_block_hash: block_hash,
            safe_block_hash: finalized,
            finalized_block_hash: finalized,
        };
        let _ = engine_handle.fork_choice_updated(state, None).await;
    }
}
