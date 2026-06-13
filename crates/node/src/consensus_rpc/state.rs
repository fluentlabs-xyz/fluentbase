//! [`FeedStateHandle`] — shared state behind the `consensus` RPC.
//!
//! Mirrors tempo `feed/state.rs`: a snapshot (`latest_finalized`) updated by the
//! feed actor, a `broadcast` channel for `consensus_subscribe`, and a SWAPPABLE
//! by-height source for `getFinalization` (marshal mailbox in signer mode, the
//! bounded window in follower mode — the unified supervisor re-wires it on every
//! promotion/demotion). Cloneable + `Send + Sync` so the jsonrpsee server
//! handler and the feed actor share it.

use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

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
    latest_finalized: Option<Arc<CertifiedBlock>>,
    latest_result_finalized: Option<u64>,
}

/// Shared by-height window a follower serves from (bounded to
/// `JUMP_THRESHOLD` entries by the window feed task). `Arc` values: the block
/// payload is a multi-MB hex string worst-case, and serving must not deep-copy
/// it under the read lock on every request.
pub type CertWindow = Arc<RwLock<BTreeMap<u64, Arc<CertifiedBlock>>>>;

/// By-height source behind `consensus_getFinalization`: the validator serves
/// from the marshal archive (full history); a follower serves from a bounded
/// in-memory window (deeper gaps are crossed by the downstream node's EL-sync
/// jump, never by cert backfill).
enum ByHeightSource {
    Marshal(MarshalMailbox),
    Window(CertWindow),
}

#[derive(Clone)]
pub struct FeedStateHandle {
    state: Arc<RwLock<FeedState>>,
    source: Arc<RwLock<Option<ByHeightSource>>>,
    events_tx: broadcast::Sender<Event>,
}

impl FeedStateHandle {
    /// `event_capacity` bounds the `subscribe` broadcast buffer (slow consumers
    /// lag, not block).
    pub fn new(event_capacity: usize) -> Self {
        let (events_tx, _) = broadcast::channel(event_capacity);
        Self {
            state: Arc::new(RwLock::new(FeedState::default())),
            source: Arc::new(RwLock::new(None)),
            events_tx,
        }
    }

    /// Wire the marshal mailbox (node-side, once `DposLayer::launch` returns it).
    /// Replaces a previously wired window on in-process promotion.
    pub fn set_marshal(&self, marshal: MarshalMailbox) {
        *self.source.write().expect("feed source poisoned") =
            Some(ByHeightSource::Marshal(marshal));
    }

    /// Wire a follower's bounded serving window (cert-follow mode). Replaces a
    /// previously wired marshal on in-process demotion.
    pub fn set_window(&self, window: CertWindow) {
        *self.source.write().expect("feed source poisoned") = Some(ByHeightSource::Window(window));
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
    pub fn record_finalized(&self, block: Arc<CertifiedBlock>, seen: u64) {
        let result_tier = block.into_parts().ok().and_then(|(_, order)| {
            (order.result != alloy_primitives::B256::ZERO).then(|| {
                (
                    order.height.saturating_sub(fluentbase_consensus::K),
                    order.result,
                )
            })
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
    pub async fn get_finalization(&self, query: Query) -> Result<Arc<CertifiedBlock>, FeedError> {
        match query {
            Query::Latest => self
                .state
                .read()
                .expect("feed state poisoned")
                .latest_finalized
                .clone()
                .ok_or(FeedError::Missing),
            Query::Height(h) => {
                // Snapshot the source under the lock, then await OUTSIDE it
                // (MarshalMailbox is a cheap clone; holding a std RwLock
                // across an await would block the swap and other readers).
                let source = {
                    let guard = self.source.read().expect("feed source poisoned");
                    match guard.as_ref().ok_or(FeedError::NotReady)? {
                        ByHeightSource::Marshal(m) => ByHeightSource::Marshal(m.clone()),
                        ByHeightSource::Window(w) => ByHeightSource::Window(w.clone()),
                    }
                };
                match source {
                    ByHeightSource::Marshal(marshal) => {
                        let height = Height::new(h);
                        let fin = marshal
                            .get_finalization(height)
                            .await
                            .ok_or(FeedError::Missing)?;
                        // `Height: Into<Identifier>` (marshal/mod.rs:103) — fetch the block by height.
                        let block = marshal.get_block(height).await.ok_or(FeedError::Missing)?;
                        Ok(Arc::new(CertifiedBlock::from_parts(&fin, &block)))
                    }
                    ByHeightSource::Window(window) => window
                        .read()
                        .expect("cert window poisoned")
                        .get(&h)
                        .cloned()
                        .ok_or(FeedError::Missing),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The by-height source is SWAPPABLE (promotion/demotion re-wires it) —
    /// a second `set_*` must replace the first, not be silently ignored.
    #[tokio::test]
    async fn second_set_window_replaces_the_first() {
        let handle = FeedStateHandle::new(8);
        assert_eq!(
            handle.get_finalization(Query::Height(7)).await.unwrap_err(),
            FeedError::NotReady
        );

        let w1: CertWindow = Default::default();
        handle.set_window(w1);
        assert_eq!(
            handle.get_finalization(Query::Height(7)).await.unwrap_err(),
            FeedError::Missing
        );

        let w2: CertWindow = Default::default();
        let cb = Arc::new(CertifiedBlock {
            height: 7,
            epoch: 0,
            view: 7,
            digest: alloy_primitives::B256::ZERO,
            certificate: String::new(),
            block: String::new(),
        });
        w2.write().unwrap().insert(7, cb);
        handle.set_window(w2);
        assert_eq!(
            handle
                .get_finalization(Query::Height(7))
                .await
                .unwrap()
                .height,
            7
        );
    }
}
