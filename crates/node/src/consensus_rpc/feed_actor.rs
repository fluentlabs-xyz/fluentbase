//! [`FeedActor`] — drains finalized heights from the consensus `FeedSink` channel
//! and populates the [`FeedStateHandle`] that backs the `consensus` RPC.
//!
//! On each finalized height it fetches `(cert, block)` from the marshal archive,
//! builds a [`CertifiedBlock`], updates `latest_finalized`, and broadcasts an
//! `Event::Finalized`. Finalized-only (F2=b).

use std::time::{SystemTime, UNIX_EPOCH};

use commonware_consensus::types::Height;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::consensus_rpc::{
    state::{FeedError, FeedStateHandle},
    types::Query,
};

pub struct FeedActor {
    rx: mpsc::UnboundedReceiver<Height>,
    handle: FeedStateHandle,
    last_height: Option<u64>,
}

impl FeedActor {
    pub fn new(rx: mpsc::UnboundedReceiver<Height>, handle: FeedStateHandle) -> Self {
        Self {
            rx,
            handle,
            last_height: None,
        }
    }

    /// Run until the `FeedSink` is dropped (node shutdown).
    pub async fn run(mut self) {
        while let Some(height) = self.rx.recv().await {
            let h = height.get();
            // `FeedSink` forwards both `Update::Block` and `Update::Tip` heights;
            // emit once per advancing height.
            if self.last_height.is_some_and(|last| h <= last) {
                continue;
            }
            match self.handle.get_finalization(Query::Height(h)).await {
                Ok(block) => {
                    self.last_height = Some(h);
                    self.handle.record_finalized(block, now_ms());
                }
                // Delivery can briefly race archival; a later height re-advances
                // the feed, so a transient miss is not fatal.
                Err(FeedError::Missing) => {
                    debug!(
                        height = h,
                        "feed: (cert,block) not yet in archive, skipping"
                    )
                }
                Err(FeedError::NotReady) => {
                    warn!(
                        height = h,
                        "feed: marshal mailbox unset; dropping feed event"
                    )
                }
            }
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
