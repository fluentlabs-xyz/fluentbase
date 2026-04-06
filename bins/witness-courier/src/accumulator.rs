//! Batch accumulator: collects per-block execution responses and tracks L1
//! batch lifecycle events until all conditions for `/sign-batch-root` are met.
//!
//! A batch is "ready" when:
//! 1. All blocks in `[from_block, to_block]` have execution responses
//! 2. Blobs have been accepted on L1 (`BatchAccepted` event received)
//!
//! Responses are stored in a flat pool keyed by block number — blocks are
//! produced in realtime and responses typically arrive before `acceptNextBatch`
//! is called on L1, so there is no "matching batch" yet at insertion time.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use tracing::{info, warn};

use crate::db::Db;
use crate::types::{EthExecutionResponse, SubmitBatchResponse};

#[derive(Debug)]
pub struct PendingBatch {
    pub batch_index: u64,
    pub from_block: u64,
    pub to_block: u64,
    pub blobs_accepted: bool,
}

impl PendingBatch {
    pub fn expected_count(&self) -> u64 {
        self.to_block - self.from_block + 1
    }
}

#[derive(Debug, Default)]
pub struct BatchAccumulator {
    batches: BTreeMap<u64, PendingBatch>,
    responses: HashMap<u64, EthExecutionResponse>,
    /// BlobsAccepted events that arrived before BatchHeadersSubmitted.
    /// Applied when the batch is later registered via set_batch.
    pending_blobs_accepted: HashSet<u64>,
    db: Option<Arc<Mutex<Db>>>,
    /// In-memory cache of batch signatures: batch_index → SubmitBatchResponse.
    /// Mirrors the `batch_signatures` DB table. Eliminates sync SQL on the hot path.
    signatures: HashMap<u64, SubmitBatchResponse>,
}

impl BatchAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create accumulator backed by a DB. Loads all state from DB on construction.
    pub fn with_db(db: Arc<Mutex<Db>>) -> Self {
        let guard = db.lock().unwrap();
        let responses: HashMap<u64, EthExecutionResponse> = guard
            .load_responses()
            .into_iter()
            .map(|r| (r.block_number, r))
            .collect();
        let batches: BTreeMap<u64, PendingBatch> = guard
            .load_batches()
            .into_iter()
            .map(|b| (b.batch_index, b))
            .collect();
        let pending_blobs_accepted: HashSet<u64> = guard
            .load_pending_blobs_accepted()
            .into_iter()
            .collect();
        let signatures: HashMap<u64, SubmitBatchResponse> = {
            let mut map = HashMap::new();
            for batch in batches.values() {
                if let Some(sig) = guard.get_batch_signature(batch.batch_index) {
                    map.insert(batch.batch_index, sig);
                }
            }
            map
        };
        drop(guard);

        Self {
            batches,
            responses,
            pending_blobs_accepted,
            db: Some(db),
            signatures,
        }
    }

    /// Register a new batch from `BatchHeadersSubmitted` event.
    pub async fn set_batch(&mut self, batch_index: u64, from_block: u64, to_block: u64) {
        // Consume any buffered BlobsAccepted for this batch
        let blobs_accepted = self.pending_blobs_accepted.remove(&batch_index);
        if blobs_accepted {
            if let Some(db) = &self.db {
                let db = Arc::clone(db);
                let _ = tokio::task::spawn_blocking(move || {
                    db.lock().unwrap().delete_pending_blobs_accepted(batch_index);
                }).await;
            }
        }

        let already = (from_block..=to_block)
            .filter(|b| self.responses.contains_key(b))
            .count();
        info!(
            batch_index,
            from_block,
            to_block,
            already,
            in_flight = self.batches.len(),
            blobs_already_accepted = blobs_accepted,
            "New batch registered"
        );
        let batch = PendingBatch {
            batch_index,
            from_block,
            to_block,
            blobs_accepted,
        };
        if let Some(db) = &self.db {
            let db = Arc::clone(db);
            let bi = batch_index;
            let fb = from_block;
            let tb = to_block;
            let ba = blobs_accepted;
            let _ = tokio::task::spawn_blocking(move || {
                db.lock().unwrap().save_batch(&PendingBatch {
                    batch_index: bi, from_block: fb, to_block: tb, blobs_accepted: ba,
                });
            }).await;
        }
        self.batches.insert(batch_index, batch);
    }

    /// Store a block execution response. O(1).
    pub async fn insert_response(&mut self, resp: EthExecutionResponse) {
        if let Some(db) = &self.db {
            let db = Arc::clone(db);
            let resp_clone = resp.clone();
            let _ = tokio::task::spawn_blocking(move || {
                db.lock().unwrap().save_response(&resp_clone);
            }).await;
        }
        let block = resp.block_number;
        self.responses.insert(block, resp);
    }

    pub async fn mark_blobs_accepted(&mut self, batch_index: u64) {
        if let Some(batch) = self.batches.get_mut(&batch_index) {
            batch.blobs_accepted = true;
            if let Some(db) = &self.db {
                let db = Arc::clone(db);
                let _ = tokio::task::spawn_blocking(move || {
                    db.lock().unwrap().update_blobs_accepted(batch_index);
                }).await;
            }
            info!(batch_index, "Blobs accepted on L1");
        } else {
            // BatchHeadersSubmitted not yet seen — buffer for when set_batch arrives
            self.pending_blobs_accepted.insert(batch_index);
            if let Some(db) = &self.db {
                let db = Arc::clone(db);
                let _ = tokio::task::spawn_blocking(move || {
                    db.lock().unwrap().save_pending_blobs_accepted(batch_index);
                }).await;
            }
            warn!(batch_index, "BlobsAccepted arrived before BatchHeaders — buffered");
        }
    }

    fn is_batch_ready(&self, batch: &PendingBatch) -> bool {
        batch.blobs_accepted
            && (batch.from_block..=batch.to_block)
                .all(|b| self.responses.contains_key(&b))
    }

    pub fn first_ready(&self) -> Option<u64> {
        self.batches
            .values()
            .find(|b| self.is_batch_ready(b))
            .map(|b| b.batch_index)
    }

    /// Returns the first ready batch (in BTreeMap order) that does NOT have
    /// a cached signature. Used by the eager signer.
    pub fn first_ready_unsigned(&self) -> Option<u64> {
        self.batches
            .values()
            .filter(|b| self.is_batch_ready(b))
            .find(|b| !self.signatures.contains_key(&b.batch_index))
            .map(|b| b.batch_index)
    }

    /// Returns the first batch in BTreeMap order that has a cached signature,
    /// along with the signature bytes. Used by the sequential dispatcher.
    ///
    /// Returns `None` if the first pending batch is not yet signed (strict ordering).
    pub fn first_sequential_signed(&self) -> Option<(u64, Vec<u8>)> {
        let first = self.batches.values().next()?;
        let resp = self.signatures.get(&first.batch_index)?;
        Some((first.batch_index, resp.signature.clone()))
    }

    pub fn get(&self, batch_index: u64) -> Option<&PendingBatch> {
        self.batches.get(&batch_index)
    }

    /// Returns the highest `to_block` across all pending batches, or `None` if empty.
    /// Used on restart to recover `next_batch_from_block` without reading a DB key.
    pub fn max_to_block(&self) -> Option<u64> {
        self.batches.values().map(|b| b.to_block).max()
    }

    /// Remove a completed batch and drain its responses from the pool.
    pub async fn take(&mut self, batch_index: u64) -> Option<PendingBatch> {
        let batch = self.batches.get(&batch_index)?;
        let fb = batch.from_block;
        let tb = batch.to_block;

        // DB first — if we crash after this, the batch is gone from DB (safe).
        // If we crash before, batch stays in both DB and memory (consistent).
        if let Some(db) = &self.db {
            let db = Arc::clone(db);
            let _ = tokio::task::spawn_blocking(move || {
                let guard = db.lock().unwrap();
                guard.delete_batch(batch_index);
                guard.delete_responses(fb, tb);
                guard.delete_batch_signature(batch_index);
            }).await;
        }

        // Memory second — only after DB deletion confirmed
        let batch = self.batches.remove(&batch_index).unwrap();
        self.signatures.remove(&batch_index);
        for b in batch.from_block..=batch.to_block {
            self.responses.remove(&b);
        }
        Some(batch)
    }

    /// Purge stale state for reverted blocks.
    ///
    /// Removes responses for the given block numbers and drops any batch
    /// whose range overlaps with reverted blocks (a batch is atomic on L1 —
    /// if any block in it is reverted, the whole batch is invalid).
    pub async fn handle_reorg(&mut self, reverted: &[u64]) {
        for &block in reverted {
            self.responses.remove(&block);
            if let Some(db) = &self.db {
                let db = Arc::clone(db);
                let _ = tokio::task::spawn_blocking(move || {
                    db.lock().unwrap().delete_response(block);
                }).await;
            }
        }

        let affected: Vec<u64> = self
            .batches
            .iter()
            .filter(|(_, b)| {
                reverted
                    .iter()
                    .any(|&r| r >= b.from_block && r <= b.to_block)
            })
            .map(|(&idx, _)| idx)
            .collect();

        for idx in &affected {
            let batch = self.batches.get(idx).unwrap();
            let fb = batch.from_block;
            let tb = batch.to_block;
            if let Some(db) = &self.db {
                let db = Arc::clone(db);
                let bi = *idx;
                let _ = tokio::task::spawn_blocking(move || {
                    let guard = db.lock().unwrap();
                    guard.delete_batch(bi);
                    guard.delete_responses(fb, tb);
                    guard.delete_batch_signature(bi);
                }).await;
            }
            let batch = self.batches.remove(idx).unwrap();
            self.signatures.remove(idx);
            for b in batch.from_block..=batch.to_block {
                self.responses.remove(&b);
            }
        }

        if !reverted.is_empty() {
            warn!(
                reverted_blocks = ?reverted,
                dropped_batches = affected.len(),
                "Reorg: purged stale state"
            );
        }
    }

    /// Purge responses for specific blocks (key rotation recovery).
    /// Unlike `handle_reorg`, this does NOT drop batches — only clears responses
    /// so they can be re-populated with freshly signed ones.
    pub async fn purge_responses(&mut self, blocks: &[u64]) {
        for &block in blocks {
            self.responses.remove(&block);
            if let Some(db) = &self.db {
                let db = Arc::clone(db);
                let _ = tokio::task::spawn_blocking(move || {
                    db.lock().unwrap().delete_response(block);
                }).await;
            }
        }
        info!(count = blocks.len(), "Purged stale responses for key rotation recovery");
    }

    /// Cache a signature in memory (called after successful signing).
    pub fn cache_signature(&mut self, batch_index: u64, resp: SubmitBatchResponse) {
        self.signatures.insert(batch_index, resp);
    }

    /// Delete a cached batch signature (e.g. after key rotation invalidation).
    pub async fn delete_batch_signature(&mut self, batch_index: u64) {
        self.signatures.remove(&batch_index);
        if let Some(db) = &self.db {
            let db = Arc::clone(db);
            let _ = tokio::task::spawn_blocking(move || {
                db.lock().unwrap().delete_batch_signature(batch_index);
            }).await;
        }
    }

    /// Returns cloned responses for blocks in [from, to].
    pub fn get_responses(&self, from: u64, to: u64) -> Vec<EthExecutionResponse> {
        (from..=to)
            .filter_map(|b| self.responses.get(&b).cloned())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.batches.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::B256;

    fn mock_response(block_number: u64) -> EthExecutionResponse {
        EthExecutionResponse {
            block_number,
            leaf: [0u8; 32],
            tx_data_hash: B256::ZERO,
            signature: vec![],
        }
    }

    #[tokio::test]
    async fn not_ready_without_blobs_accepted() {
        let mut acc = BatchAccumulator::new();
        acc.set_batch(1, 10, 12).await;
        acc.insert_response(mock_response(10)).await;
        acc.insert_response(mock_response(11)).await;
        acc.insert_response(mock_response(12)).await;

        assert!(acc.first_ready().is_none());
        acc.mark_blobs_accepted(1).await;
        assert_eq!(acc.first_ready(), Some(1));
    }

    #[tokio::test]
    async fn not_ready_without_all_responses() {
        let mut acc = BatchAccumulator::new();
        acc.set_batch(1, 10, 12).await;
        acc.mark_blobs_accepted(1).await;

        acc.insert_response(mock_response(10)).await;
        acc.insert_response(mock_response(11)).await;
        assert!(acc.first_ready().is_none());

        acc.insert_response(mock_response(12)).await;
        assert_eq!(acc.first_ready(), Some(1));
    }

    #[tokio::test]
    async fn take_removes_batch_and_responses() {
        let mut acc = BatchAccumulator::new();
        acc.set_batch(1, 10, 10).await;
        acc.insert_response(mock_response(10)).await;
        acc.mark_blobs_accepted(1).await;

        let batch = acc.take(1).await.unwrap();
        assert_eq!(batch.batch_index, 1);
        assert!(acc.first_ready().is_none());
        assert!(acc.responses.is_empty());
    }

    #[tokio::test]
    async fn concurrent_batches() {
        let mut acc = BatchAccumulator::new();
        acc.set_batch(1, 10, 11).await;
        acc.set_batch(2, 12, 13).await;

        acc.insert_response(mock_response(10)).await;
        acc.insert_response(mock_response(11)).await;
        acc.mark_blobs_accepted(1).await;
        assert_eq!(acc.first_ready(), Some(1));

        acc.insert_response(mock_response(12)).await;
        acc.insert_response(mock_response(13)).await;
        acc.mark_blobs_accepted(2).await;

        acc.take(1).await;
        assert_eq!(acc.first_ready(), Some(2));
    }

    #[tokio::test]
    async fn responses_before_batch_registration() {
        let mut acc = BatchAccumulator::new();

        // Normal flow: responses arrive before acceptNextBatch
        acc.insert_response(mock_response(10)).await;
        acc.insert_response(mock_response(11)).await;
        acc.insert_response(mock_response(12)).await;

        acc.set_batch(1, 10, 12).await;
        acc.mark_blobs_accepted(1).await;
        assert_eq!(acc.first_ready(), Some(1));
    }

    #[tokio::test]
    async fn reorg_purges_responses_and_batches() {
        let mut acc = BatchAccumulator::new();
        acc.set_batch(1, 10, 12).await;
        acc.set_batch(2, 13, 15).await;
        acc.insert_response(mock_response(10)).await;
        acc.insert_response(mock_response(11)).await;
        acc.insert_response(mock_response(12)).await;
        acc.insert_response(mock_response(13)).await;
        acc.mark_blobs_accepted(1).await;

        // Revert blocks 11-13 — batch 1 and batch 2 both overlap
        acc.handle_reorg(&[11, 12, 13]).await;

        // Both batches dropped
        assert_eq!(acc.len(), 0);
        // All responses for affected batches cleaned (including 10 from batch 1)
        assert!(acc.responses.is_empty());
    }

    #[tokio::test]
    async fn reorg_preserves_unaffected_batch() {
        let mut acc = BatchAccumulator::new();
        acc.set_batch(1, 10, 11).await;
        acc.set_batch(2, 12, 13).await;
        acc.insert_response(mock_response(10)).await;
        acc.insert_response(mock_response(11)).await;
        acc.insert_response(mock_response(12)).await;
        acc.mark_blobs_accepted(1).await;

        // Revert only block 13 — batch 1 unaffected
        acc.handle_reorg(&[13]).await;

        assert_eq!(acc.len(), 1);
        assert_eq!(acc.first_ready(), Some(1));
        // Block 12 was in batch 2, which got dropped — response removed
        assert!(!acc.responses.contains_key(&12));
    }

    #[tokio::test]
    async fn reorg_cleans_orphan_responses() {
        let mut acc = BatchAccumulator::new();
        // Responses arrived before any batch registered (normal flow)
        acc.insert_response(mock_response(50)).await;
        acc.insert_response(mock_response(51)).await;
        acc.insert_response(mock_response(52)).await;

        acc.handle_reorg(&[51, 52]).await;

        assert!(acc.responses.contains_key(&50));
        assert!(!acc.responses.contains_key(&51));
        assert!(!acc.responses.contains_key(&52));
    }

    #[tokio::test]
    async fn take_only_drains_own_blocks() {
        let mut acc = BatchAccumulator::new();
        acc.set_batch(1, 10, 10).await;
        acc.set_batch(2, 11, 11).await;
        acc.insert_response(mock_response(10)).await;
        acc.insert_response(mock_response(11)).await;

        acc.take(1).await;
        assert!(acc.responses.contains_key(&11));
        assert!(!acc.responses.contains_key(&10));
    }

    #[tokio::test]
    async fn purge_responses_preserves_batches() {
        let mut acc = BatchAccumulator::new();
        acc.set_batch(1, 10, 12).await;
        acc.insert_response(mock_response(10)).await;
        acc.insert_response(mock_response(11)).await;
        acc.insert_response(mock_response(12)).await;
        acc.mark_blobs_accepted(1).await;

        // Batch should be ready
        assert_eq!(acc.first_ready(), Some(1));

        // Purge responses for blocks 11 and 12 (key rotation)
        acc.purge_responses(&[11, 12]).await;

        // Batch is no longer ready (missing responses)
        assert!(acc.first_ready().is_none());
        // But the batch itself still exists
        assert!(acc.get(1).is_some());
        // Block 10 response is preserved
        assert!(acc.responses.contains_key(&10));
        assert!(!acc.responses.contains_key(&11));
        assert!(!acc.responses.contains_key(&12));

        // Re-insert responses — batch becomes ready again
        acc.insert_response(mock_response(11)).await;
        acc.insert_response(mock_response(12)).await;
        assert_eq!(acc.first_ready(), Some(1));
    }

    fn temp_db() -> Arc<Mutex<Db>> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("courier_test_{id}_{}.db", std::process::id()));
        let db = Db::open(&path).unwrap();
        Arc::new(Mutex::new(db))
    }

    #[tokio::test]
    async fn first_ready_unsigned_skips_signed() {
        let db = temp_db();
        let mut acc = BatchAccumulator::with_db(Arc::clone(&db));

        acc.set_batch(1, 10, 10).await;
        acc.set_batch(2, 11, 11).await;
        acc.insert_response(mock_response(10)).await;
        acc.insert_response(mock_response(11)).await;
        acc.mark_blobs_accepted(1).await;
        acc.mark_blobs_accepted(2).await;

        // Both ready, neither signed
        assert_eq!(acc.first_ready_unsigned(), Some(1));

        // Sign batch 1
        let sig_resp = crate::types::SubmitBatchResponse {
            batch_root: vec![0u8; 32],
            versioned_hashes: vec![],
            signature: vec![1, 2, 3],
        };
        acc.cache_signature(1, sig_resp.clone());

        // Now first_ready_unsigned should skip batch 1 and return batch 2
        assert_eq!(acc.first_ready_unsigned(), Some(2));

        // Sign batch 2 as well
        acc.cache_signature(2, sig_resp);

        // No unsigned batches left
        assert_eq!(acc.first_ready_unsigned(), None);
    }

    #[tokio::test]
    async fn first_sequential_signed_strict_ordering() {
        let db = temp_db();
        let mut acc = BatchAccumulator::with_db(Arc::clone(&db));

        acc.set_batch(1, 10, 10).await;
        acc.set_batch(2, 11, 11).await;
        acc.insert_response(mock_response(10)).await;
        acc.insert_response(mock_response(11)).await;
        acc.mark_blobs_accepted(1).await;
        acc.mark_blobs_accepted(2).await;

        // No signatures yet — returns None
        assert!(acc.first_sequential_signed().is_none());

        // Sign only batch 2 (not the first)
        let sig_resp = crate::types::SubmitBatchResponse {
            batch_root: vec![0u8; 32],
            versioned_hashes: vec![],
            signature: vec![4, 5, 6],
        };
        acc.cache_signature(2, sig_resp);

        // Batch 1 is first in BTreeMap but unsigned — returns None (strict ordering)
        assert!(acc.first_sequential_signed().is_none());

        // Sign batch 1
        let sig1 = crate::types::SubmitBatchResponse {
            batch_root: vec![0u8; 32],
            versioned_hashes: vec![],
            signature: vec![7, 8, 9],
        };
        acc.cache_signature(1, sig1);

        // Now batch 1 is first and signed — returns it
        let result = acc.first_sequential_signed();
        assert!(result.is_some());
        let (idx, sig) = result.unwrap();
        assert_eq!(idx, 1);
        assert_eq!(sig, vec![7, 8, 9]);
    }
}
