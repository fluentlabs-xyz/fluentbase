//! This is temporary single-node consensus that is used for block production for Fluent,
//! it will be replaced with DPoS consensus later.
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
    BuiltPayload, EngineApiMessageVersion, ExecutionPayload, PayloadAttributesBuilder, PayloadKind,
    PayloadTypes,
};
use reth_primitives_traits::{HeaderTy, NodePrimitives, SealedBlock, SealedHeaderFor};
use reth_storage_api::BlockReader;
use std::{sync::Arc, time::Duration};
use tokio::{sync::mpsc, time::Interval};
use tracing::{error, info};

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
        .spawn_critical_task("consensus validator worker", async move {
            block_producer.run(block_time).await;
        });
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

    pub async fn run(mut self, mut block_time: Interval) {
        let mut fcu_interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                // Wait for the interval or the pool to receive a transaction
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
                EngineApiMessageVersion::default(),
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
        if self.last_block_hash == B256::ZERO {
            return Ok(());
        }
        let state = ForkchoiceState::same_hash(self.last_block_hash);
        let res = self
            .to_engine
            .fork_choice_updated(state, None, EngineApiMessageVersion::default())
            .await?;
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
            new_block_fetcher(beacon_engine_handle, Arc::new(block_provider)).await
        });
    Ok(())
}

async fn new_block_fetcher<
    P: BlockProvider + Clone,
    T: PayloadTypes<BuiltPayload: BuiltPayload<Primitives: NodePrimitives<Block = P::Block>>>,
>(
    engine_handle: ConsensusEngineHandle<T>,
    block_provider: P,
) {
    let mut block_stream = {
        let (tx, rx) = mpsc::channel::<P::Block>(64);
        let block_provider = block_provider.clone();
        tokio::spawn(async move {
            block_provider.subscribe_blocks(tx).await;
        });
        rx
    };

    while let Some(block) = block_stream.recv().await {
        let payload = T::block_to_payload(SealedBlock::new_unhashed(block));
        let block_hash = payload.block_hash();
        // Send new events to execution client
        let _ = engine_handle.new_payload(payload).await;
        let state = ForkchoiceState::same_hash(block_hash);
        let _ = engine_handle
            .fork_choice_updated(state, None, EngineApiMessageVersion::V3)
            .await;
    }
}
