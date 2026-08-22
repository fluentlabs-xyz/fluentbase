//! [`FeedStateHandle`] — shared state behind the `consensus` RPC.
//!
//! Mirrors tempo `feed/state.rs`: a snapshot (`latest_finalized`) updated by the
//! feed actor, a `broadcast` channel for `consensus_subscribe`, and a SWAPPABLE
//! by-height source for `getFinalization` (marshal mailbox in signer mode, the
//! bounded window in follower mode — set once at cold-start per node mode).
//! Cloneable + `Send + Sync` so the jsonrpsee server handler and the feed actor
//! share it.

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

/// Where `consensus_getEpochArtifact` reads from: a narrow READ CLOSURE over the
/// beacon's artifact store, never the store itself. `beacon::build`'s contract is
/// that the store does not cross back out to the node, so the beacon hands out
/// this one capability instead — see [`fluentbase_consensus::beacon::ArtifactSource`].
///
/// A validator's comes off its beacon plane; a follower's off the store its own
/// key-delivery rung fills, which is what lets a follower serve a tier-2 follower
/// exactly as it already serves `getFinalization` out of its cert window.
pub use fluentbase_consensus::beacon::ArtifactSource;

#[derive(Clone)]
pub struct FeedStateHandle {
    state: Arc<RwLock<FeedState>>,
    source: Arc<RwLock<Option<ByHeightSource>>>,
    artifacts: Arc<RwLock<Option<ArtifactSource>>>,
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
            artifacts: Arc::new(RwLock::new(None)),
            events_tx,
        }
    }

    /// Wire the epoch-artifact read closure (validator: its beacon plane's store;
    /// follower: the store its own key-delivery rung fills). Until this runs,
    /// `consensus_getEpochArtifact` answers `ServiceUnavailable` — the same
    /// not-ready shape the by-height source uses before `set_marshal`.
    pub fn set_artifact_source(&self, artifacts: ArtifactSource) {
        *self.artifacts.write().expect("artifact source poisoned") = Some(artifacts);
    }

    /// Wire the marshal mailbox (node-side, once `DposLayer::launch` returns it).
    /// A validator serves the by-height feed from the full marshal archive.
    pub fn set_marshal(&self, marshal: MarshalMailbox) {
        *self.source.write().expect("feed source poisoned") =
            Some(ByHeightSource::Marshal(marshal));
    }

    /// Wire a follower's bounded serving window (cert-follow mode). A follower
    /// serves the by-height feed from the inlet-fed window instead of an archive.
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

    /// `consensus_getEpochArtifact`: the wire bytes of the artifact minted at
    /// `epoch`, or `Missing` where this node holds none — which is the normal
    /// answer for most of an epoch, not a fault.
    ///
    /// Snapshot the closure under the lock and call it OUTSIDE, the same
    /// discipline `get_finalization` follows: the read is a store lookup plus a
    /// re-encode, and holding a std `RwLock` across it would block the swap and
    /// every other reader.
    pub async fn get_epoch_artifact(&self, epoch: u64) -> Result<Vec<u8>, FeedError> {
        let source = {
            let guard = self.artifacts.read().expect("artifact source poisoned");
            guard.as_ref().ok_or(FeedError::NotReady)?.clone()
        };
        source(epoch).ok_or(FeedError::Missing)
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
