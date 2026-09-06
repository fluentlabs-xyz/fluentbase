//! This is temporary single-node consensus that is used for block production for Fluent,
//! it will be replaced with DPoS consensus later.
use alloy_network::Ethereum;
use alloy_primitives::B256;
use alloy_rpc_types_engine::ForkchoiceState;
use eyre::{OptionExt, WrapErr};
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
use reth_tasks::{shutdown::GracefulShutdown, TaskExecutor};
use std::{future::Future, time::Duration};
use tokio::{sync::mpsc, task::JoinHandle, time::Interval};
use tracing::{error, info};

#[cfg(test)]
mod tests;

pub async fn launch_consensus_validator<N, AddOns: RethRpcAddOns<N>, B>(
    handle: &NodeHandle<N, AddOns>,
    block_time: Duration,
    payload_attributes_builder: B,
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
    );

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

#[derive(Debug)]
pub struct BlockProducer<T: PayloadTypes, B> {
    to_engine: ConsensusEngineHandle<T>,
    payload_attributes_builder: B,
    payload_builder: PayloadBuilderHandle<T>,
    last_header: SealedHeaderFor<<T::BuiltPayload as BuiltPayload>::Primitives>,
    last_block_hash: B256,
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
    ) -> Self {
        let last_header = provider
            .sealed_header(provider.best_block_number().unwrap())
            .unwrap()
            .unwrap();
        let last_block_hash = last_header.hash();
        Self {
            to_engine,
            payload_attributes_builder,
            payload_builder,
            last_header,
            last_block_hash,
        }
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
) -> eyre::Result<()>
where
    Node: FullNodeComponents<Types: DebugNode<Node, RpcBlock = alloy_rpc_types_eth::Block>>,
{
    info!(target: "reth::cli", "Using RPC consensus client: {}", consensus_url);

    // Decode directly into the supported RPC block type. Unsupported transaction types now
    // become provider errors, which the subscription logs before continuing, not panics in
    // an infallible AnyNetwork-to-Ethereum conversion callback.
    let block_provider = RpcBlockProvider::<Ethereum, _>::new(
        consensus_url.as_str(),
        Node::Types::rpc_to_primitive_block,
    )
    .await?;

    let beacon_engine_handle = handle.node.add_ons_handle.beacon_engine_handle.clone();
    spawn_consensus_follower(
        &handle.node.task_executor,
        beacon_engine_handle,
        block_provider,
    );
    Ok(())
}

fn spawn_consensus_follower<
    P: BlockProvider,
    T: PayloadTypes<BuiltPayload: BuiltPayload<Primitives: NodePrimitives<Block = P::Block>>>,
>(
    task_executor: &TaskExecutor,
    engine_handle: ConsensusEngineHandle<T>,
    block_provider: P,
) -> JoinHandle<()> {
    let executor = task_executor.clone();
    task_executor.spawn_critical_with_graceful_shutdown_signal(
        "consensus node worker",
        move |shutdown| async move {
            if let Err(err) =
                new_block_fetcher(engine_handle, block_provider, shutdown.ignore_guard()).await
            {
                error!(target: "engine::local", %err, "Consensus follower failed; shutting down node");
                if let Err(err) = executor.initiate_graceful_shutdown() {
                    error!(target: "engine::local", %err, "Failed to request node shutdown");
                }
            }
        },
    )
}

async fn new_block_fetcher<
    P: BlockProvider,
    T: PayloadTypes<BuiltPayload: BuiltPayload<Primitives: NodePrimitives<Block = P::Block>>>,
>(
    engine_handle: ConsensusEngineHandle<T>,
    block_provider: P,
    shutdown: impl Future<Output = ()>,
) -> eyre::Result<()> {
    let (tx, mut block_stream) = mpsc::channel::<P::Block>(64);
    // Poll the subscription in this critical worker so a panic cannot be swallowed by a
    // detached task. Dropping the worker also cancels the subscription.
    let subscription = block_provider.subscribe_blocks(tx);
    tokio::pin!(subscription, shutdown);

    loop {
        tokio::select! {
            biased;

            _ = &mut shutdown => {
                info!(target: "engine::local", "Shutting down consensus node worker");
                return Ok(());
            }
            _ = &mut subscription => {
                eyre::bail!("Consensus block subscription ended unexpectedly");
            }
            block = block_stream.recv() => {
                let block = block.ok_or_eyre("Consensus block stream closed unexpectedly")?;
                let payload = T::block_to_payload(SealedBlock::new_unhashed(block));
                let block_hash = payload.block_hash();
                // Once processing starts, finish the payload/FCU pair before observing shutdown.
                // SYNCING/ACCEPTED are allowed: the engine may need to fetch missing ancestors.
                let status = engine_handle
                    .new_payload(payload)
                    .await
                    .wrap_err("Failed to submit consensus payload")?;
                if status.is_invalid() {
                    eyre::bail!("Invalid consensus payload {block_hash}: {status:?}");
                }
                let state = ForkchoiceState::same_hash(block_hash);
                let result = engine_handle
                    .fork_choice_updated(state, None)
                    .await
                    .wrap_err("Failed to update consensus fork choice")?;
                if result.is_invalid() {
                    eyre::bail!("Invalid consensus fork choice {block_hash}: {result:?}");
                }
            }
        }
    }
}
