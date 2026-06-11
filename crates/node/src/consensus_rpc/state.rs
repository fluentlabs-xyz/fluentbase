//! [`FeedStateHandle`] — shared state behind the `consensus` RPC.
//!
//! Mirrors tempo `feed/state.rs`: a snapshot (`latest_finalized`) updated by the
//! feed actor, a `broadcast` channel for `consensus_subscribe`, and an
//! `OnceLock<MarshalMailbox>` for by-height `getFinalization` (set node-side once
//! `DposLayer::launch` returns the mailbox). Cloneable + `Send + Sync` so the
//! jsonrpsee server handler and the feed actor share it.

use std::sync::{Arc, OnceLock, RwLock};

use commonware_consensus::types::Height;
use fluentbase_consensus::MarshalMailbox;
use tokio::sync::broadcast;

use crate::{
    certified_block::CertifiedBlock,
    consensus_rpc::types::{ConsensusState, Event, Query},
};

/// Why a `getFinalization` could not be served.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedError {
    /// The marshal mailbox is not yet wired (node still starting).
    NotReady,
    /// No finalization/block at the requested point.
    Missing,
}

#[derive(Default)]
struct FeedState {
    latest_finalized: Option<CertifiedBlock>,
    latest_result_finalized: Option<u64>,
}

#[derive(Clone)]
pub struct FeedStateHandle {
    state: Arc<RwLock<FeedState>>,
    marshal: Arc<OnceLock<MarshalMailbox>>,
    events_tx: broadcast::Sender<Event>,
}

impl FeedStateHandle {
    /// `event_capacity` bounds the `subscribe` broadcast buffer (slow consumers
    /// lag, not block).
    pub fn new(event_capacity: usize) -> Self {
        let (events_tx, _) = broadcast::channel(event_capacity);
        Self {
            state: Arc::new(RwLock::new(FeedState::default())),
            marshal: Arc::new(OnceLock::new()),
            events_tx,
        }
    }

    /// Wire the marshal mailbox (node-side, once `DposLayer::launch` returns it).
    pub fn set_marshal(&self, marshal: MarshalMailbox) {
        let _ = self.marshal.set(marshal);
    }

    /// New `consensus_subscribe` receiver.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events_tx.subscribe()
    }

    /// `consensus_getLatest` snapshot.
    pub fn latest(&self) -> ConsensusState {
        let state = self.state.read().expect("feed state poisoned");
        ConsensusState {
            latest_finalized: state.latest_finalized.clone(),
            latest_result_finalized: state.latest_result_finalized,
        }
    }

    /// Called by the feed actor on each finalized artifact: update both
    /// finality tiers and fan the events out to `subscribe` listeners
    /// (best-effort — no listeners is fine). The result tier is derived from
    /// the artifact's `result` commitment: inclusion-finalizing height N
    /// attests the derived hash of N − K.
    pub fn record_finalized(&self, block: CertifiedBlock, seen: u64) {
        let result_tier = block.into_parts().ok().and_then(|(_, order)| {
            (order.result != alloy_primitives::B256::ZERO)
                .then(|| (order.height.saturating_sub(fluentbase_consensus::K), order.result))
        });
        {
            let mut state = self.state.write().expect("feed state poisoned");
            state.latest_finalized = Some(block.clone());
            if let Some((h, _)) = result_tier {
                state.latest_result_finalized =
                    Some(state.latest_result_finalized.unwrap_or(0).max(h));
            }
        }
        let _ = self.events_tx.send(Event::Finalized { block, seen });
        if let Some((height, executed_hash)) = result_tier {
            let _ = self.events_tx.send(Event::ResultFinalized {
                height,
                executed_hash,
                seen,
            });
        }
    }

    /// `consensus_getFinalization`: `Latest` from the snapshot; `Height(h)` from
    /// the marshal archive (`get_finalization` + `get_block` → [`CertifiedBlock`]).
    pub async fn get_finalization(&self, query: Query) -> Result<CertifiedBlock, FeedError> {
        match query {
            Query::Latest => self
                .state
                .read()
                .expect("feed state poisoned")
                .latest_finalized
                .clone()
                .ok_or(FeedError::Missing),
            Query::Height(h) => {
                let marshal = self.marshal.get().ok_or(FeedError::NotReady)?;
                let height = Height::new(h);
                let fin = marshal
                    .get_finalization(height)
                    .await
                    .ok_or(FeedError::Missing)?;
                // `Height: Into<Identifier>` (marshal/mod.rs:103) — fetch the block by height.
                let block = marshal.get_block(height).await.ok_or(FeedError::Missing)?;
                Ok(CertifiedBlock::from_parts(&fin, &block))
            }
        }
    }
}
