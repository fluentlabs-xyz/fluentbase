//! In-process witness hub: byte-bounded ring buffer + broadcast fan-out.
//!
//! The ExEx pushes witnesses via [`WitnessHub::push`]. The gRPC server reads
//! them via [`WitnessHub::subscribe`] (live stream) and
//! [`WitnessHub::snapshot_from`] (replay on reconnect).
//!
//! ## Memory model
//!
//! Each `ProveRequest` is wrapped in `Arc`. The ring buffer and broadcast
//! channel hold `Arc` clones pointing to the same allocation — witness data
//! is never duplicated.
//!
//! The buffer is bounded by **total payload bytes**, not entry count. On an
//! L2 with 1s block time and 30-80 MB witnesses, the default 512 MiB limit
//! holds ~6-15 blocks — more than enough for a co-located courier restart
//! (typically 1-3 seconds).

use std::collections::VecDeque;
use std::sync::Arc;

use tokio::sync::{broadcast, RwLock};
use tracing::info;

use crate::types::{HubEvent, ProveRequest, SharedProveRequest};

/// Max total payload bytes in the ring buffer.
/// Eviction triggers when either this OR [`RING_BUFFER_MAX_ENTRIES`] is hit,
/// whichever comes first.
const RING_BUFFER_MAX_BYTES: usize = 1024 * 1024 * 1024; // 1 GiB

/// Hard cap on entry count as a secondary limit.
/// At 1 block/sec this covers ~17 minutes of courier downtime.
const RING_BUFFER_MAX_ENTRIES: usize = 1024;

/// Broadcast channel capacity for live subscribers.
/// This only holds `Arc` clones (~8 bytes each), not data copies.
/// 1024 gives a slow subscriber ~17 minutes of runway at 1 block/sec
/// before it gets `Lagged` and must reconnect.
const BROADCAST_CAPACITY: usize = 1024;

/// Central witness distribution point.
///
/// Shared via `Arc` between the ExEx (writer) and gRPC server (reader).
/// Thread-safe: the ring buffer is behind a [`RwLock`], the broadcast
/// channel is lock-free for subscribers.\
pub struct WitnessHub {
    tx: broadcast::Sender<HubEvent>,
    buffer: RwLock<RingBuffer>,
}

/// Byte-bounded ring buffer for witness replay.
struct RingBuffer {
    entries: VecDeque<SharedProveRequest>,
    total_bytes: usize,
}

impl RingBuffer {
    fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(RING_BUFFER_MAX_ENTRIES),
            total_bytes: 0,
        }
    }

    /// Push a new entry, evicting oldest entries until both byte and
    /// count limits are satisfied.
    fn push(&mut self, req: SharedProveRequest) {
        let entry_bytes = req.payload.len();

        while self.total_bytes + entry_bytes > RING_BUFFER_MAX_BYTES
            || self.entries.len() >= RING_BUFFER_MAX_ENTRIES
        {
            match self.entries.pop_front() {
                Some(evicted) => self.total_bytes -= evicted.payload.len(),
                None => break,
            }
        }

        self.total_bytes += entry_bytes;
        self.entries.push_back(req);
    }

    /// Remove all entries for a given block number (revert cleanup).
    fn remove(&mut self, block_number: u64) {
        let before = self.entries.len();
        self.entries.retain(|req| {
            if req.block_number == block_number {
                self.total_bytes -= req.payload.len();
                false
            } else {
                true
            }
        });
        let removed = before - self.entries.len();
        if removed > 0 {
            info!(
                block_number,
                removed,
                remaining = self.entries.len(),
                total_bytes = self.total_bytes,
                "Evicted witness from ring buffer"
            );
        }
    }

    /// Return all entries with `block_number >= from_block`.
    fn snapshot_from(&self, from_block: u64) -> Vec<SharedProveRequest> {
        self.entries
            .iter()
            .filter(|r| r.block_number >= from_block)
            .cloned()
            .collect()
    }

    /// Current stats for diagnostics.
    fn stats(&self) -> (usize, usize) {
        (self.entries.len(), self.total_bytes)
    }
}

impl WitnessHub {
    /// Create a new empty hub.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            tx,
            buffer: RwLock::new(RingBuffer::new()),
        }
    }

    /// Push a new witness into the buffer and broadcast it to live subscribers.
    ///
    /// If the buffer exceeds its byte or entry limit, the oldest entries are
    /// evicted. If there are no active subscribers the broadcast send is
    /// silently dropped.
    pub async fn push(&self, req: Arc<ProveRequest>) {
        let payload_bytes = req.payload.len();
        {
            let mut buf = self.buffer.write().await;
            buf.push(Arc::clone(&req));
            let (entries, total) = buf.stats();
            info!(
                block_number = req.block_number,
                payload_bytes,
                buffer_entries = entries,
                buffer_bytes = total,
                "Witness buffered"
            );
        }
        // Broadcast to live subscribers. Err means no active receivers — fine.
        let _ = self.tx.send(HubEvent::Witness(req));
    }

    /// Remove reverted blocks from the buffer and notify live subscribers.
    ///
    /// Called on chain reverts. Cleans the ring buffer and broadcasts a
    /// [`HubEvent::Reorg`] so live subscribers can purge stale state.
    pub async fn remove_and_notify(&self, reverted_blocks: Vec<u64>) {
        {
            let mut buf = self.buffer.write().await;
            for &block_number in &reverted_blocks {
                buf.remove(block_number);
            }
        }
        let _ = self.tx.send(HubEvent::Reorg { reverted_blocks });
    }

    /// Subscribe to live witness broadcasts.
    ///
    /// Returns a [`broadcast::Receiver`] that yields every witness pushed
    /// after this call. Use [`snapshot_from`](Self::snapshot_from) first
    /// to replay buffered history.
    pub fn subscribe(&self) -> broadcast::Receiver<HubEvent> {
        self.tx.subscribe()
    }

    /// Return all buffered witnesses with `block_number >= from_block`.
    ///
    /// Used by the gRPC server to replay missed witnesses when a courier
    /// reconnects with a `from_block` offset.
    pub async fn snapshot_from(&self, from_block: u64) -> Vec<SharedProveRequest> {
        let buf = self.buffer.read().await;
        buf.snapshot_from(from_block)
    }
}

impl Default for WitnessHub {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_req(block_number: u64, size: usize) -> Arc<ProveRequest> {
        Arc::new(ProveRequest {
            block_number,
            payload: vec![0u8; size],
        })
    }

    #[test]
    fn evicts_by_bytes() {
        let mut buf = RingBuffer::new();

        // Fill with entries that together exceed RING_BUFFER_MAX_BYTES.
        let chunk = RING_BUFFER_MAX_BYTES / 4;
        for i in 0..4 {
            buf.push(make_req(i, chunk));
        }
        assert_eq!(buf.entries.len(), 4);
        assert_eq!(buf.total_bytes, chunk * 4);

        // Next push should evict oldest to make room.
        buf.push(make_req(4, chunk));
        assert!(buf.total_bytes <= RING_BUFFER_MAX_BYTES);
        // Block 0 should be evicted.
        assert!(buf.entries.iter().all(|r| r.block_number != 0));
    }

    #[test]
    fn evicts_by_count() {
        let mut buf = RingBuffer::new();

        // Push RING_BUFFER_MAX_ENTRIES small entries.
        for i in 0..RING_BUFFER_MAX_ENTRIES as u64 {
            buf.push(make_req(i, 1));
        }
        assert_eq!(buf.entries.len(), RING_BUFFER_MAX_ENTRIES);

        // Next push should evict oldest.
        buf.push(make_req(999, 1));
        assert_eq!(buf.entries.len(), RING_BUFFER_MAX_ENTRIES);
        assert!(buf.entries.iter().all(|r| r.block_number != 0));
        assert!(buf.entries.iter().any(|r| r.block_number == 999));
    }

    #[test]
    fn remove_updates_bytes() {
        let mut buf = RingBuffer::new();
        buf.push(make_req(1, 1000));
        buf.push(make_req(2, 2000));
        buf.push(make_req(3, 3000));
        assert_eq!(buf.total_bytes, 6000);

        buf.remove(2);
        assert_eq!(buf.total_bytes, 4000);
        assert_eq!(buf.entries.len(), 2);
    }

    #[test]
    fn snapshot_from_filters() {
        let mut buf = RingBuffer::new();
        for i in 10..15 {
            buf.push(make_req(i, 100));
        }

        let snap = buf.snapshot_from(12);
        let numbers: Vec<u64> = snap.iter().map(|r| r.block_number).collect();
        assert_eq!(numbers, vec![12, 13, 14]);
    }
}