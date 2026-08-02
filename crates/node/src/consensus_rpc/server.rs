//! jsonrpsee `consensus` namespace — serves finality certs to cert-followers.
//!
//! Mirrors tempo `rpc/consensus/mod.rs`, scoped to v1: `getFinalization`,
//! `getLatest`, `subscribe`. Registered on a node via reth `extend_rpc_modules`.

use jsonrpsee::{
    core::{RpcResult, SubscriptionResult},
    proc_macros::rpc,
    types::ErrorObject,
    PendingSubscriptionSink, SubscriptionMessage,
};
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;

use crate::{
    certified_block::CertifiedBlock,
    consensus_rpc::{
        state::{FeedError, FeedStateHandle},
        types::{ConsensusState, Event, Query},
    },
};

/// Custom JSON-RPC error codes (tempo parity).
const NO_CONTENT: i32 = 204;
const SERVICE_UNAVAILABLE: i32 = 503;

fn to_rpc_error(err: FeedError) -> ErrorObject<'static> {
    match err {
        FeedError::Missing => ErrorObject::owned(
            NO_CONTENT,
            "requested finalization not available",
            None::<()>,
        ),
        FeedError::NotReady => ErrorObject::owned(
            SERVICE_UNAVAILABLE,
            "consensus feed not ready yet; retry shortly",
            None::<()>,
        ),
    }
}

#[rpc(server, client, namespace = "consensus")]
pub trait ConsensusApi {
    /// Finalization (block + 2f+1 cert) for `Latest` or a specific `Height`.
    /// `Arc` is serialization-transparent (serde `rc`): the multi-MB payload
    /// is not deep-copied between the serving window and the serializer.
    #[method(name = "getFinalization")]
    async fn get_finalization(&self, query: Query) -> RpcResult<Arc<CertifiedBlock>>;

    /// Latest finalized snapshot.
    #[method(name = "getLatest")]
    async fn get_latest(&self) -> RpcResult<ConsensusState>;

    /// Stream of consensus events (v1: `Finalized` only).
    #[subscription(name = "subscribe" => "event", unsubscribe = "unsubscribe", item = Event)]
    async fn subscribe_events(&self) -> SubscriptionResult;
}

/// RPC handler backed by the shared [`FeedStateHandle`].
#[derive(Clone)]
pub struct ConsensusRpc {
    feed: FeedStateHandle,
}

impl ConsensusRpc {
    pub fn new(feed: FeedStateHandle) -> Self {
        Self { feed }
    }
}

#[jsonrpsee::core::async_trait]
impl ConsensusApiServer for ConsensusRpc {
    async fn get_finalization(&self, query: Query) -> RpcResult<Arc<CertifiedBlock>> {
        self.feed
            .get_finalization(query)
            .await
            .map_err(to_rpc_error)
    }

    async fn get_latest(&self) -> RpcResult<ConsensusState> {
        Ok(self.feed.latest())
    }

    async fn subscribe_events(&self, pending: PendingSubscriptionSink) -> SubscriptionResult {
        let sink = pending.accept().await?;
        let mut rx = self.feed.subscribe();
        tokio::spawn(async move {
            loop {
                // Reap promptly on client disconnect: without this, a dropped
                // subscriber's task lingers until the next broadcast event, which
                // during a finalization stall can be a long time (audit P2-21).
                let event = tokio::select! {
                    _ = sink.closed() => break,
                    recv = rx.recv() => match recv {
                        Ok(event) => event,
                        Err(RecvError::Closed) => break,
                        // Slow consumer fell behind; keep streaming from the latest.
                        Err(RecvError::Lagged(_)) => continue,
                    },
                };
                let Ok(msg) =
                    SubscriptionMessage::new(sink.method_name(), sink.subscription_id(), &event)
                else {
                    break;
                };
                if sink.send(msg).await.is_err() {
                    break;
                }
            }
        });
        Ok(())
    }
}
