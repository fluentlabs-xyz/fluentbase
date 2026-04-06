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
//! L2 with 1s block time and 30-80 MB witnesses, the default 1 GiB limit
//! holds ~12-30 blocks — more than enough for a co-located courier restart
//! (typically 1-3 seconds).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::collections::BTreeSet;
use std::sync::{Arc as StdArc, RwLock as StdRwLock};

use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{error, info, warn};

use crate::types::{HubEvent, ProveRequest, SharedProveRequest};

/// Broadcast channel capacity for live subscribers.
/// This only holds `Arc` clones (~8 bytes each), not data copies.
/// 1024 gives a slow subscriber ~17 minutes of runway at 1 block/sec
/// before it gets `Lagged` and must reconnect.
const BROADCAST_CAPACITY: usize = 1024;

struct ColdTier {
    dir: PathBuf,
    index: StdArc<StdRwLock<BTreeSet<u64>>>,
    tx: mpsc::Sender<SharedProveRequest>,
    max_dir_bytes: u64,
    current_dir_bytes: StdArc<std::sync::atomic::AtomicU64>,
}

async fn run_cold_writer(
    mut rx: mpsc::Receiver<SharedProveRequest>,
    dir: PathBuf,
    index: StdArc<StdRwLock<BTreeSet<u64>>>,
    max_dir_bytes: u64,
    current_dir_bytes: StdArc<std::sync::atomic::AtomicU64>,
) {
    use std::sync::atomic::Ordering::Relaxed;

    while let Some(req) = rx.recv().await {
        let block_number = req.block_number;
        let payload = req.payload.clone();

        let compressed = match tokio::task::spawn_blocking(move || {
            zstd::encode_all(payload.as_slice(), 3)
        }).await {
            Ok(Ok(c)) => c,
            Ok(Err(e)) => {
                error!(block_number, err = %e, "Cold tier: Zstd compression failed");
                continue;
            }
            Err(_) => continue,
        };

        let compressed_len = compressed.len() as u64;
        let tmp_path = dir.join(format!("{block_number}.bin.tmp"));
        let final_path = dir.join(format!("{block_number}.bin"));

        match tokio::fs::write(&tmp_path, &compressed).await {
            Ok(()) => {
                match tokio::fs::rename(&tmp_path, &final_path).await {
                    Ok(()) => {
                        index.write().unwrap().insert(block_number);
                        current_dir_bytes.fetch_add(compressed_len, Relaxed);
                        info!(block_number, compressed_bytes = compressed_len, "Cold: written");

                        // FIFO eviction
                        while current_dir_bytes.load(Relaxed) > max_dir_bytes {
                            let oldest = {
                                let idx = index.read().unwrap();
                                idx.iter().next().copied()
                            };
                            let Some(oldest_block) = oldest else { break };
                            let path = dir.join(format!("{oldest_block}.bin"));
                            match tokio::fs::metadata(&path).await {
                                Ok(m) => {
                                    let file_size = m.len();
                                    if let Err(e) = tokio::fs::remove_file(&path).await {
                                        warn!(oldest_block, err = %e, "Cold eviction: remove failed");
                                        break;
                                    }
                                    index.write().unwrap().remove(&oldest_block);
                                    current_dir_bytes.fetch_sub(file_size, Relaxed);
                                    info!(oldest_block, freed_bytes = file_size, "Cold: evicted oldest");
                                }
                                Err(_) => {
                                    index.write().unwrap().remove(&oldest_block);
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!(block_number, err = %e, "Cold rename failed");
                        let _ = tokio::fs::remove_file(&tmp_path).await;
                    }
                }
            }
            Err(e) => {
                error!(block_number, err = %e, "Cold write failed");
            }
        }
    }
}

/// Central witness distribution point.
///
/// Shared via `Arc` between the ExEx (writer) and gRPC server (reader).
/// Thread-safe: the ring buffer is behind a [`RwLock`], the broadcast
/// channel is lock-free for subscribers.\
pub struct WitnessHub {
    tx: broadcast::Sender<HubEvent>,
    buffer: RwLock<RingBuffer>,
    cold: Option<ColdTier>,
}

/// Byte-bounded ring buffer for witness replay.
struct RingBuffer {
    entries: VecDeque<SharedProveRequest>,
    total_bytes: usize,
    max_bytes: usize,
}

impl RingBuffer {
    fn new(max_bytes: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            total_bytes: 0,
            max_bytes,
        }
    }

    /// Push a new entry, evicting oldest entries until the byte limit is satisfied.
    /// Returns any evicted entries.
    fn push(&mut self, req: SharedProveRequest) -> Vec<SharedProveRequest> {
        let entry_bytes = req.payload.len();
        let mut evicted = Vec::new();

        while self.total_bytes + entry_bytes > self.max_bytes {
            match self.entries.pop_front() {
                Some(e) => {
                    self.total_bytes -= e.payload.len();
                    evicted.push(e);
                }
                None => break,
            }
        }

        self.total_bytes += entry_bytes;
        self.entries.push_back(req);
        evicted
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

    /// Returns the block number of the oldest entry, if any.
    fn oldest_block(&self) -> Option<u64> {
        self.entries.front().map(|r| r.block_number)
    }

    /// Current stats for diagnostics.
    fn stats(&self) -> (usize, usize) {
        (self.entries.len(), self.total_bytes)
    }
}

impl WitnessHub {
    /// Create a new hub with a byte-bounded hot ring buffer and optional cold tier.
    pub fn new(max_bytes: usize, cold_dir: Option<PathBuf>, max_cold_bytes: u64) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);

        let cold = cold_dir.map(|dir| {
            std::fs::create_dir_all(&dir)
                .expect("failed to create cold witness directory");

            // Clean up leftover .tmp files from previous crash.
            // Populate index from complete .bin files only (skip empty/corrupt).
            let mut existing = BTreeSet::new();
            let mut initial_bytes: u64 = 0;
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for entry in rd.flatten() {
                    let fname = entry.file_name();
                    let name = fname.to_string_lossy();
                    if name.ends_with(".tmp") {
                        let _ = std::fs::remove_file(entry.path());
                        continue;
                    }
                    if let Some(n) = name
                        .strip_suffix(".bin")
                        .and_then(|s| s.parse::<u64>().ok())
                    {
                        match entry.metadata() {
                            Ok(m) if m.len() > 0 => {
                                initial_bytes += m.len();
                                existing.insert(n);
                            }
                            _ => { let _ = std::fs::remove_file(entry.path()); }
                        }
                    }
                }
            }

            let current_dir_bytes = StdArc::new(std::sync::atomic::AtomicU64::new(initial_bytes));
            let index = StdArc::new(StdRwLock::new(existing));
            let (cold_tx, cold_rx) = mpsc::channel(1000);
            tokio::spawn(run_cold_writer(
                cold_rx,
                dir.clone(),
                StdArc::clone(&index),
                max_cold_bytes,
                StdArc::clone(&current_dir_bytes),
            ));

            ColdTier { dir, index, tx: cold_tx, max_dir_bytes: max_cold_bytes, current_dir_bytes }
        });

        Self {
            tx,
            buffer: RwLock::new(RingBuffer::new(max_bytes)),
            cold,
        }
    }

    /// Push a new witness into the buffer and broadcast it to live subscribers.
    ///
    /// If the buffer exceeds its byte limit, the oldest entries are evicted
    /// and forwarded to the cold tier (if configured).
    /// If there are no active subscribers the broadcast send is silently dropped.
    pub async fn push(&self, req: Arc<ProveRequest>) {
        let payload_bytes = req.payload.len();
        let evicted = {
            let mut buf = self.buffer.write().await;
            let evicted = buf.push(Arc::clone(&req));
            let (entries, total) = buf.stats();
            info!(
                block_number = req.block_number,
                payload_bytes,
                buffer_entries = entries,
                buffer_bytes = total,
                "Witness buffered"
            );
            evicted
        };

        if let Some(cold) = &self.cold {
            for entry in evicted {
                if cold.tx.try_send(entry).is_err() {
                    warn!("Cold channel full — dropping evicted witness");
                }
            }
        }

        // Broadcast to live subscribers. Err means no active receivers — fine.
        let _ = self.tx.send(HubEvent::Witness(req));
    }

    /// Remove reverted blocks from the buffer and notify live subscribers.
    ///
    /// Called on chain reverts. Cleans the ring buffer, removes cold-tier files,
    /// and broadcasts a [`HubEvent::Reorg`] so live subscribers can purge stale state.
    pub async fn remove_and_notify(&self, reverted_blocks: Vec<u64>) {
        {
            let mut buf = self.buffer.write().await;
            for &block_number in &reverted_blocks {
                buf.remove(block_number);
            }
        }

        if let Some(cold) = &self.cold {
            for &block_number in &reverted_blocks {
                // Remove .bin file
                let bin_path = cold.dir.join(format!("{block_number}.bin"));
                match tokio::fs::remove_file(&bin_path).await {
                    Ok(()) => { cold.index.write().unwrap().remove(&block_number); }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => warn!(block_number, err = %e, "Failed to remove cold witness on reorg"),
                }
                // Remove .bin.tmp file (race with run_cold_writer)
                let tmp_path = cold.dir.join(format!("{block_number}.bin.tmp"));
                let _ = tokio::fs::remove_file(&tmp_path).await;
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

    /// Returns the block number of the oldest entry in the hot buffer, if any.
    pub async fn oldest_buffered_block(&self) -> Option<u64> {
        self.buffer.read().await.oldest_block()
    }

    /// Returns sorted block numbers in [from, to) that have cold-tier files.
    /// Does NOT load payloads — callers read one file at a time via `read_cold_block`.
    pub fn cold_blocks_in_range(&self, from: u64, to: u64) -> Vec<u64> {
        let Some(cold) = &self.cold else { return vec![]; };
        let idx = cold.index.read().unwrap();
        idx.range(from..to).copied().collect()
    }

    /// Read one cold-tier file. Returns None if not found (already deleted or never written).
    pub async fn read_cold_block(&self, block_number: u64) -> Option<SharedProveRequest> {
        let cold = self.cold.as_ref()?;
        let path = cold.dir.join(format!("{block_number}.bin"));
        match tokio::fs::read(&path).await {
            Ok(compressed) => {
                match tokio::task::spawn_blocking(move || {
                    zstd::decode_all(compressed.as_slice())
                }).await {
                    Ok(Ok(payload)) => Some(Arc::new(ProveRequest { block_number, payload })),
                    Ok(Err(e)) => {
                        warn!(block_number, err = %e, "Cold: Zstd decompression failed");
                        None
                    }
                    Err(_) => None,
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                cold.index.write().unwrap().remove(&block_number);
                None
            }
            Err(e) => {
                warn!(block_number, err = %e, "Failed to read cold witness");
                None
            }
        }
    }

    /// Get a single block witness from hot buffer by block number.
    pub async fn get_block(&self, block_number: u64) -> Option<SharedProveRequest> {
        let buf = self.buffer.read().await;
        buf.entries
            .iter()
            .find(|r| r.block_number == block_number)
            .cloned()
    }

    /// Get a single witness by block number: tries hot buffer first, then cold tier.
    pub async fn get_witness(&self, block_number: u64) -> Option<SharedProveRequest> {
        if let Some(req) = self.get_block(block_number).await {
            return Some(req);
        }
        self.read_cold_block(block_number).await
    }

    /// Deletes cold-tier files for blocks in [from_block, to_block].
    pub async fn acknowledge_range(&self, from_block: u64, to_block: u64) {
        let Some(cold) = &self.cold else { return; };
        use std::sync::atomic::Ordering::Relaxed;

        let to_delete: Vec<u64> = {
            let idx = cold.index.read().unwrap();
            idx.range(from_block..=to_block).copied().collect()
        };

        for block_number in to_delete {
            let path = cold.dir.join(format!("{block_number}.bin"));
            match tokio::fs::metadata(&path).await {
                Ok(m) => {
                    let file_size = m.len();
                    match tokio::fs::remove_file(&path).await {
                        Ok(()) => {
                            cold.index.write().unwrap().remove(&block_number);
                            cold.current_dir_bytes.fetch_sub(file_size, Relaxed);
                        }
                        Err(e) => warn!(block_number, err = %e, "Failed to acknowledge cold witness"),
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    cold.index.write().unwrap().remove(&block_number);
                }
                Err(e) => warn!(block_number, err = %e, "Failed to stat cold witness"),
            }
        }
        info!(from_block, to_block, "Cold witnesses acknowledged (range)");
    }

    /// Called by the courier (via gRPC) after checkpoint advances.
    /// Deletes all cold-tier files with block_number <= up_to_block.
    pub async fn acknowledge(&self, up_to_block: u64) {
        let Some(cold) = &self.cold else { return; };
        use std::sync::atomic::Ordering::Relaxed;

        let to_delete: Vec<u64> = {
            let idx = cold.index.read().unwrap();
            idx.range(..=up_to_block).copied().collect()
        };

        for block_number in to_delete {
            let path = cold.dir.join(format!("{block_number}.bin"));
            match tokio::fs::metadata(&path).await {
                Ok(m) => {
                    let file_size = m.len();
                    match tokio::fs::remove_file(&path).await {
                        Ok(()) => {
                            cold.index.write().unwrap().remove(&block_number);
                            cold.current_dir_bytes.fetch_sub(file_size, Relaxed);
                        }
                        Err(e) => warn!(block_number, err = %e, "Failed to acknowledge cold witness"),
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    cold.index.write().unwrap().remove(&block_number);
                }
                Err(e) => warn!(block_number, err = %e, "Failed to stat cold witness for acknowledge"),
            }
        }
        info!(up_to_block, "Cold witnesses acknowledged");
    }
}

impl Default for WitnessHub {
    fn default() -> Self {
        Self::new(1024 * 1024 * 1024, None, 10 * 1024 * 1024 * 1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_MAX_BYTES: usize = 1024 * 1024 * 1024;

    fn make_req(block_number: u64, size: usize) -> Arc<ProveRequest> {
        Arc::new(ProveRequest {
            block_number,
            payload: vec![0u8; size],
        })
    }

    #[test]
    fn evicts_by_bytes() {
        let mut buf = RingBuffer::new(TEST_MAX_BYTES);

        // Fill with entries that together exceed TEST_MAX_BYTES.
        let chunk = TEST_MAX_BYTES / 4;
        for i in 0..4 {
            let _ = buf.push(make_req(i, chunk));
        }
        assert_eq!(buf.entries.len(), 4);
        assert_eq!(buf.total_bytes, chunk * 4);

        // Next push should evict oldest to make room.
        let _ = buf.push(make_req(4, chunk));
        assert!(buf.total_bytes <= TEST_MAX_BYTES);
        // Block 0 should be evicted.
        assert!(buf.entries.iter().all(|r| r.block_number != 0));
    }

    #[test]
    fn remove_updates_bytes() {
        let mut buf = RingBuffer::new(TEST_MAX_BYTES);
        let _ = buf.push(make_req(1, 1000));
        let _ = buf.push(make_req(2, 2000));
        let _ = buf.push(make_req(3, 3000));
        assert_eq!(buf.total_bytes, 6000);

        buf.remove(2);
        assert_eq!(buf.total_bytes, 4000);
        assert_eq!(buf.entries.len(), 2);
    }

    #[test]
    fn snapshot_from_filters() {
        let mut buf = RingBuffer::new(TEST_MAX_BYTES);
        for i in 10..15 {
            let _ = buf.push(make_req(i, 100));
        }

        let snap = buf.snapshot_from(12);
        let numbers: Vec<u64> = snap.iter().map(|r| r.block_number).collect();
        assert_eq!(numbers, vec![12, 13, 14]);
    }
}
