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
    BuiltPayload, ExecutionPayload, PayloadAttributesBuilder, PayloadKind, PayloadTypes,
};
use reth_primitives_traits::{HeaderTy, NodePrimitives, SealedBlock, SealedHeaderFor};
use reth_storage_api::BlockReader;
use reth_tasks::shutdown::GracefulShutdown;
use std::time::Duration;
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
    Node: FullNodeComponents<Types: DebugNode<Node>>,
{
    info!(target: "reth::cli", "Using RPC consensus client: {}", consensus_url);

    let block_provider =
        RpcBlockProvider::<AnyNetwork, _>::new(consensus_url.as_str(), |block_response| {
            let number = block_response.header.number;
            let hash = block_response.header.hash;
            // The conversion hook of `BlockProvider` is infallible, so a block this node cannot
            // decode -- an unknown transaction envelope, for instance -- can only be reported by
            // unwinding. `new_block_fetcher` supervises the subscription, so the unwind reaches
            // the critical task supervisor and takes the node down. That is deliberate: a node
            // that cannot decode what the sequencer produces must not keep running on a stale
            // head as if nothing happened.
            let rpc_block = serde_json::to_value(&block_response)
                .and_then(serde_json::from_value)
                .unwrap_or_else(|err| {
                    error!(
                        target: "reth::cli",
                        %err,
                        number,
                        %hash,
                        "Consensus block does not match the RPC block type of this node",
                    );
                    panic!("cannot decode consensus block {number} ({hash}): {err}")
                });
            Node::Types::rpc_to_primitive_block(rpc_block)
        })
        .await?;

    let beacon_engine_handle = handle.node.add_ons_handle.beacon_engine_handle.clone();
    handle
        .node
        .task_executor
        .spawn_critical_task("consensus node worker", async move {
            new_block_fetcher(beacon_engine_handle, block_provider).await
        });
    Ok(())
}

async fn new_block_fetcher<
    P: BlockProvider,
    T: PayloadTypes<BuiltPayload: BuiltPayload<Primitives: NodePrimitives<Block = P::Block>>>,
>(
    engine_handle: ConsensusEngineHandle<T>,
    block_provider: P,
) {
    let (tx, mut block_stream) = mpsc::channel::<P::Block>(64);
    // The subscription stays on its own task so it can keep buffering blocks while the engine
    // processes the previous one, but its handle is kept here so that the task is supervised
    // instead of detached.
    let mut subscription = tokio::spawn(async move { block_provider.subscribe_blocks(tx).await });

    loop {
        tokio::select! {
            joined = &mut subscription => subscription_ended(joined),
            block = block_stream.recv() => {
                let Some(block) = block else {
                    // The sender only disappears together with the subscription task, so join it
                    // to report the failure that actually stopped block ingestion.
                    subscription_ended((&mut subscription).await)
                };
                let payload = T::block_to_payload(SealedBlock::new_unhashed(block));
                let block_hash = payload.block_hash();
                // Send new events to execution client
                let _ = engine_handle.new_payload(payload).await;
                let state = ForkchoiceState::same_hash(block_hash);
                let _ = engine_handle.fork_choice_updated(state, None).await;
            }
        }
    }
}

/// Turns the end of the block subscription task into a fatal error.
///
/// `subscribe_blocks` reconnects on its own and only returns once its receiver is gone, so as long
/// as [`new_block_fetcher`] holds that receiver any termination means block ingestion is dead.
/// Unwinding here propagates to the critical task supervisor, which shuts the node down instead of
/// leaving it silently parked on the last ingested block.
fn subscription_ended(joined: Result<(), tokio::task::JoinError>) -> ! {
    match joined {
        Err(err) if err.is_panic() => {
            error!(target: "reth::cli", "Consensus block subscription panicked");
            std::panic::resume_unwind(err.into_panic())
        }
        Err(err) => panic!("consensus block subscription task was cancelled: {err}"),
        Ok(()) => panic!("consensus block subscription ended while the node was following blocks"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reth_engine_primitives::BeaconEngineMessage;
    use reth_ethereum_engine_primitives::EthPayloadTypes;
    use reth_ethereum_primitives::Block;
    use std::future::pending;
    use tokio::sync::mpsc::UnboundedReceiver;

    enum TestBlockProvider {
        /// Unwinds inside the subscription, the way the conversion hook does when the sequencer
        /// sends a block this node cannot decode.
        PanicsOnConversion,
        /// Returns from the subscription without ever sending a block.
        EndsSubscription,
        /// Sends `usize` blocks and then keeps the subscription open forever.
        Sends(usize),
    }

    impl BlockProvider for TestBlockProvider {
        type Block = Block;

        async fn subscribe_blocks(&self, tx: mpsc::Sender<Self::Block>) {
            match *self {
                Self::PanicsOnConversion => panic!("cannot decode consensus block"),
                Self::EndsSubscription => (),
                Self::Sends(count) => {
                    for _ in 0..count {
                        if tx.send(Block::default()).await.is_err() {
                            return;
                        }
                    }
                    pending().await
                }
            }
        }

        async fn get_block(&self, _block_number: u64) -> eyre::Result<Self::Block> {
            eyre::bail!("not used by these tests")
        }
    }

    fn engine_channel() -> (
        ConsensusEngineHandle<EthPayloadTypes>,
        UnboundedReceiver<BeaconEngineMessage<EthPayloadTypes>>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (ConsensusEngineHandle::new(tx), rx)
    }

    #[tokio::test]
    #[should_panic(expected = "cannot decode consensus block")]
    async fn undecodable_block_takes_the_fetcher_down() {
        let (engine_handle, _engine_rx) = engine_channel();
        new_block_fetcher(engine_handle, TestBlockProvider::PanicsOnConversion).await;
    }

    #[tokio::test]
    #[should_panic(expected = "subscription ended")]
    async fn subscription_end_takes_the_fetcher_down() {
        let (engine_handle, _engine_rx) = engine_channel();
        new_block_fetcher(engine_handle, TestBlockProvider::EndsSubscription).await;
    }

    #[tokio::test]
    async fn blocks_keep_reaching_the_engine() {
        let (engine_handle, mut engine_rx) = engine_channel();
        tokio::spawn(new_block_fetcher(
            engine_handle,
            TestBlockProvider::Sends(2),
        ));

        for _ in 0..2 {
            // Both responders are dropped on purpose: the fetcher ignores engine errors and has
            // to keep going to the next block either way.
            assert!(matches!(
                engine_rx.recv().await,
                Some(BeaconEngineMessage::NewPayload { .. })
            ));
            assert!(matches!(
                engine_rx.recv().await,
                Some(BeaconEngineMessage::ForkchoiceUpdated { .. })
            ));
        }
    }
}
