//! gRPC server implementation for [`WitnessService`].
//!
//! The node embeds this server via [`create_service`] and spawns it on the
//! task executor. Each [`Subscribe`] call creates a per-client stream that
//! first replays cold-tier history, then hot-tier snapshot, then switches to live broadcast.
//!
//! # Example (inside node startup)
//!
//! ```ignore
//! let hub = Arc::new(WitnessHub::new(1024 * 1024 * 1024, None));
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
            // Subscribe FIRST — prevents lost-window between snapshot and live stream.
            let mut broadcast_rx = hub.subscribe();

            let mut last_sent = if from_block > 0 { from_block - 1 } else { 0 };

            // ── Step 1: Snapshot hot tier immediately (pins Arcs in RAM) ──────
            // This must happen before any cold I/O. Even if the ring buffer evicts
            // these entries later, the Vec holds the Arcs alive.
            let replay = hub.snapshot_from(from_block).await;
            let hot_start = replay.first().map(|r| r.block_number).unwrap_or(u64::MAX);

            // ── Step 2: Stream cold blocks [from_block, hot_start) one-at-a-time
            let cold_blocks = hub.cold_blocks_in_range(from_block, hot_start);

            for block_number in cold_blocks {
                let Some(req) = hub.read_cold_block(block_number).await else { continue; };
                let msg = proto::WitnessMessage {
                    content: Some(proto::witness_message::Content::Witness(
                        proto::WitnessData {
                            block_number: req.block_number,
                            data: req.payload.clone(),
                        },
                    )),
                };
                if tx.send(Ok(msg)).await.is_err() { return; }
                last_sent = req.block_number;
            }
            // req dropped here — cold file data freed before reading the next file.

            // ── Step 3: Send pre-captured hot snapshot ────────────────────────
            for req in replay {
                if req.block_number <= last_sent { continue; } // dedup cold/hot overlap
                let msg = proto::WitnessMessage {
                    content: Some(proto::witness_message::Content::Witness(
                        proto::WitnessData {
                            block_number: req.block_number,
                            data: req.payload.clone(),
                        },
                    )),
                };
                if tx.send(Ok(msg)).await.is_err() { return; }
                last_sent = req.block_number;
            }

            // ── Live stream ───────────────────────────────────────────────────
            // Dedup guard (req.block_number <= last_sent) handles duplicates from
            // the overlap window.
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
                        warn!(skipped = n, last_sent, "Courier subscriber lagged — terminating stream to force cold replay");
                        return; // closes the gRPC stream; courier reconnects from checkpoint
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn acknowledge(
        &self,
        request: Request<proto::AcknowledgeRequest>,
    ) -> Result<Response<proto::AcknowledgeResponse>, Status> {
        let up_to_block = request.into_inner().up_to_block;
        self.hub.acknowledge(up_to_block).await;
        Ok(Response::new(proto::AcknowledgeResponse {}))
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
