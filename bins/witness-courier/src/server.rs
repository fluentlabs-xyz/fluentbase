//! gRPC server implementation for [`WitnessService`].
//!
//! The node embeds this server via [`create_service`] and spawns it on the
//! task executor. Each [`Subscribe`] call creates a per-client stream that
//! first replays buffered history, then switches to live broadcast.
//!
//! # Example (inside node startup)
//!
//! ```ignore
//! let hub = Arc::new(WitnessHub::new());
//! let svc = witness_courier::server::create_service(Arc::clone(&hub));
//!
//! let server = tonic::transport::Server::builder()
//!     .add_service(svc)
//!     .serve(addr);
//!
//! node.task_executor.spawn(Box::pin(async move {
//!     if let Err(e) = server.await {
//!         tracing::error!(err = %e, "gRPC witness server failed");
//!     }
//! }));
//! ```

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tracing::{info, warn};

use crate::hub::WitnessHub;
use crate::proto::{
    self,
    witness_service_server::{WitnessService, WitnessServiceServer},
};
use crate::types::HubEvent;

/// gRPC service implementation backed by a [`WitnessHub`].
pub struct WitnessGrpcService {
    hub: Arc<WitnessHub>,
}

#[tonic::async_trait]
impl WitnessService for WitnessGrpcService {
    type SubscribeStream = ReceiverStream<Result<proto::WitnessMessage, Status>>;

    async fn subscribe(
        &self,
        request: Request<proto::SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let from_block = request.into_inner().from_block;
        let (tx, rx) = mpsc::channel(64);
        let hub = Arc::clone(&self.hub);

        info!(from_block, "Courier subscribed");

        tokio::spawn(async move {
            let replay = hub.snapshot_from(from_block).await;
            let mut last_sent = if from_block > 0 { from_block - 1 } else { 0 };

            for req in replay {
                let msg = proto::WitnessMessage {
                    content: Some(proto::witness_message::Content::Witness(
                        proto::WitnessData {
                            block_number: req.block_number,
                            data: req.payload.clone(),
                        },
                    )),
                };
                if tx.send(Ok(msg)).await.is_err() {
                    return;
                }
                last_sent = req.block_number;
            }

            let mut broadcast_rx = hub.subscribe();
            loop {
                match broadcast_rx.recv().await {
                    Ok(HubEvent::Witness(req)) => {
                        if req.block_number <= last_sent {
                            continue;
                        }
                        let msg = proto::WitnessMessage {
                            content: Some(proto::witness_message::Content::Witness(
                                proto::WitnessData {
                                    block_number: req.block_number,
                                    data: req.payload.clone(),
                                },
                            )),
                        };
                        if tx.send(Ok(msg)).await.is_err() {
                            return;
                        }
                        last_sent = req.block_number;
                    }
                    Ok(HubEvent::Reorg { reverted_blocks }) => {
                        // Reset dedup cursor so replacement blocks pass through
                        if let Some(&min) = reverted_blocks.iter().min() {
                            if min > 0 {
                                last_sent = min - 1;
                            }
                        }
                        let msg = proto::WitnessMessage {
                            content: Some(proto::witness_message::Content::Reorg(
                                proto::ReorgNotification {
                                    reverted_block_numbers: reverted_blocks,
                                },
                            )),
                        };
                        if tx.send(Ok(msg)).await.is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "Courier subscriber lagged");
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

/// Build a [`WitnessServiceServer`] ready to be added to a tonic server.
///
/// Sets max message sizes to `usize::MAX` because witness payloads can be
/// large (tens of MB for heavy blocks).
pub fn create_service(hub: Arc<WitnessHub>) -> WitnessServiceServer<WitnessGrpcService> {
    WitnessServiceServer::new(WitnessGrpcService { hub })
        .max_encoding_message_size(usize::MAX)
        .max_decoding_message_size(usize::MAX)
}