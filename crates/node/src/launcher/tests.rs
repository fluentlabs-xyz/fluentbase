use super::*;
use alloy_network::AnyRpcBlock;
use alloy_rpc_types_engine::{PayloadStatus, PayloadStatusEnum};
use jsonrpsee::{server::ServerBuilder, RpcModule};
use reth_engine_primitives::{BeaconEngineMessage, OnForkChoiceUpdated};
use reth_ethereum_engine_primitives::EthEngineTypes;
use reth_ethereum_primitives::Block;
use reth_tasks::shutdown;
use serde_json::json;
use std::{
    future::pending,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::time::timeout;

const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy)]
enum Subscription {
    Pending,
    End,
    CloseChannel,
    Panic,
    Block,
}

struct MockProvider {
    subscription: Subscription,
    dropped: Arc<AtomicBool>,
}

struct DropGuard(Arc<AtomicBool>);

impl Drop for DropGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl MockProvider {
    fn new(subscription: Subscription) -> Self {
        Self {
            subscription,
            dropped: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl BlockProvider for MockProvider {
    type Block = Block;

    async fn subscribe_blocks(&self, tx: mpsc::Sender<Block>) {
        let _guard = DropGuard(self.dropped.clone());
        match self.subscription {
            Subscription::End => return,
            Subscription::CloseChannel => drop(tx),
            Subscription::Panic => panic!("mock subscription panic"),
            Subscription::Block => {
                tx.send(Block::default()).await.unwrap();
                pending::<()>().await;
            }
            Subscription::Pending => {
                pending::<()>().await;
            }
        }
        pending::<()>().await;
    }

    async fn get_block(&self, _: u64) -> eyre::Result<Block> {
        eyre::bail!("not used by the follower")
    }
}

fn engine() -> (
    ConsensusEngineHandle<EthEngineTypes>,
    mpsc::UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
) {
    let (tx, rx) = mpsc::unbounded_channel();
    (ConsensusEngineHandle::new(tx), rx)
}

#[tokio::test]
async fn unknown_rpc_transaction_returns_error_and_next_block_is_decodable() {
    let mut block = serde_json::to_value(<alloy_rpc_types_eth::Block>::default()).unwrap();
    block["transactions"] = json!([{
        "type": "0x7e",
        "hash": B256::ZERO,
        "from": alloy_primitives::Address::ZERO,
        "gas": "0x5208",
        "nonce": "0x0",
        "value": "0x0",
        "input": "0x"
    }]);
    // This shape passed the old AnyNetwork decoder, then panicked in the conversion callback.
    assert!(serde_json::from_value::<AnyRpcBlock>(block.clone()).is_ok());

    let server = ServerBuilder::default().build("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", server.local_addr().unwrap());
    let mut module = RpcModule::new(block);
    module
        .register_method("eth_getBlockByNumber", |params, bad_block, _| {
            let (number, full): (String, bool) = params.parse().unwrap();
            assert!(full);
            if number == "0x1" {
                bad_block.clone()
            } else {
                serde_json::to_value(<alloy_rpc_types_eth::Block>::default()).unwrap()
            }
        })
        .unwrap();
    let server = server.start(module);
    let provider = RpcBlockProvider::<Ethereum, _>::new(&url, |block| -> Block {
        block.into_consensus().convert_transactions()
    })
    .await
    .unwrap();

    let result = timeout(TIMEOUT, provider.get_block(1)).await.unwrap();
    assert!(
        result.is_err(),
        "unknown transaction type must be a provider error"
    );
    assert!(timeout(TIMEOUT, provider.get_block(2))
        .await
        .unwrap()
        .is_ok());
    server.stop().unwrap();
    server.stopped().await;
}

#[tokio::test]
async fn subscription_end_or_closed_channel_requests_node_shutdown() {
    for subscription in [Subscription::End, Subscription::CloseChannel] {
        let executor = TaskExecutor::test();
        let manager = executor.take_task_manager_handle().unwrap();
        let (engine, _rx) = engine();
        let provider = MockProvider::new(subscription);
        let dropped = provider.dropped.clone();
        let worker = spawn_consensus_follower(&executor, engine, provider);

        timeout(TIMEOUT, executor.on_shutdown_signal().clone())
            .await
            .unwrap();
        worker.await.unwrap();
        assert!(manager.await.unwrap().is_ok());
        assert!(dropped.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn subscription_panic_reaches_critical_task_manager() {
    let executor = TaskExecutor::test();
    let manager = executor.take_task_manager_handle().unwrap();
    let (engine, _rx) = engine();
    let worker =
        spawn_consensus_follower(&executor, engine, MockProvider::new(Subscription::Panic));

    let error = timeout(TIMEOUT, manager)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("consensus node worker"));
    worker.await.unwrap();
}

#[tokio::test]
async fn unavailable_engine_returns_error() {
    let (engine, rx) = engine();
    drop(rx);
    let error = timeout(
        TIMEOUT,
        new_block_fetcher(engine, MockProvider::new(Subscription::Block), pending()),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("Failed to submit consensus payload"));
}

#[tokio::test]
async fn invalid_payload_never_advances_fork_choice() {
    let (engine, mut rx) = engine();
    let worker = tokio::spawn(new_block_fetcher(
        engine,
        MockProvider::new(Subscription::Block),
        pending(),
    ));
    let BeaconEngineMessage::NewPayload { tx, .. } = rx.recv().await.unwrap() else {
        panic!("expected new payload");
    };
    tx.send(Ok(PayloadStatus::from_status(PayloadStatusEnum::Invalid {
        validation_error: "invalid test block".into(),
    })))
    .unwrap();

    let error = timeout(TIMEOUT, worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("Invalid consensus payload"));
    assert!(
        rx.recv().await.is_none(),
        "invalid payload must not be followed by FCU"
    );
}

#[tokio::test]
async fn fork_choice_errors_are_propagated() {
    for invalid_status in [false, true] {
        let (engine, mut rx) = engine();
        let worker = tokio::spawn(new_block_fetcher(
            engine,
            MockProvider::new(Subscription::Block),
            pending(),
        ));
        let BeaconEngineMessage::NewPayload { tx, .. } = rx.recv().await.unwrap() else {
            panic!("expected new payload");
        };
        tx.send(Ok(PayloadStatus::from_status(PayloadStatusEnum::Valid)))
            .unwrap();
        let BeaconEngineMessage::ForkchoiceUpdated { tx, .. } = rx.recv().await.unwrap() else {
            panic!("expected fork choice update");
        };
        if invalid_status {
            tx.send(Ok(OnForkChoiceUpdated::with_invalid(
                PayloadStatus::from_status(PayloadStatusEnum::Invalid {
                    validation_error: "invalid test fork".into(),
                }),
            )))
            .unwrap();
        } else {
            drop(tx);
        }
        let error = timeout(TIMEOUT, worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        let expected = if invalid_status {
            "Invalid consensus fork choice"
        } else {
            "Failed to update consensus fork choice"
        };
        assert!(error.to_string().contains(expected), "{error:?}");
    }
}

#[tokio::test]
async fn shutdown_finishes_in_flight_payload_and_allows_syncing() {
    for status in [
        PayloadStatusEnum::Valid,
        PayloadStatusEnum::Syncing,
        PayloadStatusEnum::Accepted,
    ] {
        let (engine, mut rx) = engine();
        let (signal, shutdown) = shutdown::signal();
        let provider = MockProvider::new(Subscription::Block);
        let dropped = provider.dropped.clone();
        let worker = tokio::spawn(new_block_fetcher(engine, provider, shutdown));

        let BeaconEngineMessage::NewPayload { payload, tx } = rx.recv().await.unwrap() else {
            panic!("expected new payload");
        };
        signal.fire();
        assert!(!worker.is_finished());
        tx.send(Ok(PayloadStatus::from_status(status))).unwrap();
        let BeaconEngineMessage::ForkchoiceUpdated { state, tx, .. } = rx.recv().await.unwrap()
        else {
            panic!("expected fork choice update even after shutdown");
        };
        assert_eq!(state, ForkchoiceState::same_hash(payload.block_hash()));
        tx.send(Ok(OnForkChoiceUpdated::syncing())).unwrap();

        timeout(TIMEOUT, worker).await.unwrap().unwrap().unwrap();
        assert!(dropped.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn normal_shutdown_is_successful() {
    let executor = TaskExecutor::test();
    let manager = executor.take_task_manager_handle().unwrap();
    let (engine, _rx) = engine();
    let worker =
        spawn_consensus_follower(&executor, engine, MockProvider::new(Subscription::Pending));
    let shutdown = executor.initiate_graceful_shutdown().unwrap();

    drop(timeout(TIMEOUT, shutdown).await.unwrap());
    worker.await.unwrap();
    assert!(manager.await.unwrap().is_ok());
}
