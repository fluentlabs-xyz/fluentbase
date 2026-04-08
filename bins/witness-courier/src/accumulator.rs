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

use alloy_primitives::B256;
use tracing::{error, info, warn};

use crate::db::Db;
use crate::types::{EthExecutionResponse, SubmitBatchResponse};

#[derive(Debug)]
pub struct PendingBatch {
    pub batch_index: u64,
    pub from_block: u64,
    pub to_block: u64,
    pub blobs_accepted: bool,
}

#[derive(Debug)]
pub struct DispatchedBatch {
    pub batch_index: u64,
    pub from_block: u64,
    pub to_block: u64,
    pub tx_hash: B256,
    pub l1_block: u64,
}

impl PendingBatch {
    pub fn expected_count(&self) -> u64 {
        self.to_block - self.from_block + 1
    }
}

#[derive(Debug)]
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
    /// Batches submitted to L1 awaiting finalization.
    pub(crate) dispatched: BTreeMap<u64, DispatchedBatch>,
}

impl BatchAccumulator {
    pub fn new() -> Self {
        Self {
            batches: BTreeMap::new(),
            responses: HashMap::new(),
            pending_blobs_accepted: HashSet::new(),
            db: None,
            signatures: HashMap::new(),
            dispatched: BTreeMap::new(),
        }
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
        let dispatched: BTreeMap<u64, DispatchedBatch> = guard
            .load_dispatched_batches()
            .into_iter()
            .filter_map(|(bi, fb, tb, tx_hash_bytes, l1b)| {
                let tx_hash = B256::try_from(tx_hash_bytes.as_slice()).ok().or_else(|| {
                    error!(batch_index = bi, len = tx_hash_bytes.len(), "Corrupt tx_hash in dispatched_batches — skipping");
                    None
                })?;
                Some((bi, DispatchedBatch {
                    batch_index: bi,
                    from_block: fb,
                    to_block: tb,
                    tx_hash,
                    l1_block: l1b,
                }))
            })
            .collect();
        drop(guard);

        Self {
            batches,
            responses,
            pending_blobs_accepted,
            db: Some(db),
            signatures,
            dispatched,
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

    /// Returns the highest `to_block` across all pending and dispatched batches, or `None` if empty.
    /// Used on restart to recover `next_batch_from_block` without reading a DB key.
    pub fn max_to_block(&self) -> Option<u64> {
        let pending_max = self.batches.values().map(|b| b.to_block).max();
        let dispatched_max = self.dispatched.values().map(|d| d.to_block).max();
        pending_max.max(dispatched_max)
    }

    /// Purge responses for specific blocks (key rotation recovery).
    /// Clears responses so they can be re-populated with freshly signed ones.
    /// Batches are preserved — only responses are removed.
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

    /// Move a batch from pending to dispatched state.
    /// Removes from `batches` and `signatures`, inserts into `dispatched`.
    pub async fn mark_dispatched(
        &mut self,
        batch_index: u64,
        tx_hash: B256,
        l1_block: u64,
    ) {
        let Some(batch) = self.batches.remove(&batch_index) else { return };
        self.signatures.remove(&batch_index);

        let dispatched = DispatchedBatch {
            batch_index,
            from_block: batch.from_block,
            to_block: batch.to_block,
            tx_hash,
            l1_block,
        };

        if let Some(db) = &self.db {
            let db = Arc::clone(db);
            let fb = batch.from_block;
            let tb = batch.to_block;
            let tx_h = tx_hash.0.to_vec();
            let _ = tokio::task::spawn_blocking(move || {
                db.lock().unwrap().move_to_dispatched(batch_index, fb, tb, &tx_h, l1_block);
            }).await;
        }

        self.dispatched.insert(batch_index, dispatched);
    }

    /// Finalize a dispatched batch: delete all associated data from DB + memory.
    pub async fn finalize_dispatched(&mut self, batch_index: u64) -> Option<DispatchedBatch> {
        let dispatched = self.dispatched.remove(&batch_index)?;
        let fb = dispatched.from_block;
        let tb = dispatched.to_block;

        if let Some(db) = &self.db {
            let db = Arc::clone(db);
            let _ = tokio::task::spawn_blocking(move || {
                db.lock().unwrap().finalize_dispatched_batch(batch_index, fb, tb);
            }).await;
        }

        for b in fb..=tb {
            self.responses.remove(&b);
        }

        Some(dispatched)
    }

    /// Move a dispatched batch back to pending (reorg recovery).
    pub async fn undispatch(&mut self, batch_index: u64) -> bool {
        let Some(dispatched) = self.dispatched.remove(&batch_index) else {
            return false;
        };

        let batch = PendingBatch {
            batch_index,
            from_block: dispatched.from_block,
            to_block: dispatched.to_block,
            blobs_accepted: true,
        };

        if let Some(db) = &self.db {
            let db = Arc::clone(db);
            let fb = dispatched.from_block;
            let tb = dispatched.to_block;
            let _ = tokio::task::spawn_blocking(move || {
                db.lock().unwrap().undispatch_batch(batch_index, fb, tb);
            }).await;
        }

        self.batches.insert(batch_index, batch);
        true
    }

    /// Atomically remove a pending batch that was already preconfirmed on-chain.
    /// Cleans up batches, signatures, responses and persists last_batch_end in one DB transaction.
    pub async fn skip_already_dispatched(&mut self, batch_index: u64) {
        let Some(batch) = self.batches.remove(&batch_index) else { return };
        self.signatures.remove(&batch_index);

        let fb = batch.from_block;
        let tb = batch.to_block;

        if let Some(db) = &self.db {
            let db = Arc::clone(db);
            let _ = tokio::task::spawn_blocking(move || {
                db.lock().unwrap().skip_pending_batch(batch_index, fb, tb);
            }).await;
        }

        for b in fb..=tb {
            self.responses.remove(&b);
        }
    }

    /// Returns dispatched batch indices where l1_block <= finalized.
    pub fn dispatched_finalization_candidates(&self, finalized_block: u64) -> Vec<u64> {
        self.dispatched
            .values()
            .filter(|d| d.l1_block <= finalized_block)
            .map(|d| d.batch_index)
            .collect()
    }

    /// Check if any dispatched batches exist.
    pub fn has_dispatched(&self) -> bool {
        !self.dispatched.is_empty()
    }

    /// Get the tx_hash of a dispatched batch.
    pub fn dispatched_tx_hash(&self, batch_index: u64) -> Option<B256> {
        self.dispatched.get(&batch_index).map(|d| d.tx_hash)
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

        acc.mark_dispatched(1, B256::ZERO, 1).await;
        acc.finalize_dispatched(1).await;
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

    #[tokio::test]
    async fn mark_dispatched_removes_from_batches_adds_to_dispatched() {
        let mut acc = BatchAccumulator::new();
        acc.set_batch(1, 10, 12).await;
        acc.insert_response(mock_response(10)).await;
        acc.insert_response(mock_response(11)).await;
        acc.insert_response(mock_response(12)).await;
        acc.mark_blobs_accepted(1).await;

        let tx_hash = B256::from([0xAA; 32]);
        acc.mark_dispatched(1, tx_hash, 100).await;

        assert!(acc.get(1).is_none());
        assert!(acc.dispatched.contains_key(&1));
        let d = &acc.dispatched[&1];
        assert_eq!(d.from_block, 10);
        assert_eq!(d.to_block, 12);
        assert_eq!(d.tx_hash, tx_hash);
        assert_eq!(d.l1_block, 100);
    }

    #[tokio::test]
    async fn finalize_dispatched_cleans_up_responses() {
        let mut acc = BatchAccumulator::new();
        acc.set_batch(1, 10, 11).await;
        acc.insert_response(mock_response(10)).await;
        acc.insert_response(mock_response(11)).await;
        acc.mark_blobs_accepted(1).await;

        // Also insert a response for a different batch to verify it's not removed
        acc.insert_response(mock_response(12)).await;

        acc.mark_dispatched(1, B256::from([0xBB; 32]), 50).await;

        let dispatched = acc.finalize_dispatched(1).await.unwrap();
        assert_eq!(dispatched.batch_index, 1);
        assert!(acc.dispatched.is_empty());
        assert!(!acc.responses.contains_key(&10));
        assert!(!acc.responses.contains_key(&11));
        // Response for block 12 (different batch) should still exist
        assert!(acc.responses.contains_key(&12));
    }

    #[tokio::test]
    async fn undispatch_moves_back_to_batches() {
        let mut acc = BatchAccumulator::new();
        acc.set_batch(1, 10, 11).await;
        acc.insert_response(mock_response(10)).await;
        acc.insert_response(mock_response(11)).await;
        acc.mark_blobs_accepted(1).await;

        acc.mark_dispatched(1, B256::from([0xCC; 32]), 60).await;
        assert!(acc.get(1).is_none());

        let ok = acc.undispatch(1).await;
        assert!(ok);
        assert!(acc.dispatched.is_empty());
        let batch = acc.get(1).unwrap();
        assert_eq!(batch.from_block, 10);
        assert_eq!(batch.to_block, 11);
        assert!(batch.blobs_accepted);
    }

    #[tokio::test]
    async fn max_to_block_considers_dispatched() {
        let mut acc = BatchAccumulator::new();
        acc.set_batch(1, 10, 20).await;
        acc.insert_response(mock_response(10)).await;
        acc.mark_blobs_accepted(1).await;

        assert_eq!(acc.max_to_block(), Some(20));

        acc.mark_dispatched(1, B256::from([0xDD; 32]), 70).await;

        // After dispatch, pending is empty but dispatched has it
        assert!(acc.batches.is_empty());
        assert_eq!(acc.max_to_block(), Some(20));
    }

    #[tokio::test]
    async fn dispatched_batches_db_round_trip() {
        let db = temp_db();
        let mut acc = BatchAccumulator::with_db(Arc::clone(&db));

        acc.set_batch(1, 10, 12).await;
        acc.insert_response(mock_response(10)).await;
        acc.insert_response(mock_response(11)).await;
        acc.insert_response(mock_response(12)).await;
        acc.mark_blobs_accepted(1).await;

        let tx_hash = B256::from([0xEE; 32]);
        acc.mark_dispatched(1, tx_hash, 80).await;

        // Reload from DB
        let acc2 = BatchAccumulator::with_db(Arc::clone(&db));
        assert!(acc2.dispatched.contains_key(&1));
        let d = &acc2.dispatched[&1];
        assert_eq!(d.from_block, 10);
        assert_eq!(d.to_block, 12);
        assert_eq!(d.tx_hash, tx_hash);
        assert_eq!(d.l1_block, 80);
        // Pending batch should be gone (moved to dispatched)
        assert!(acc2.get(1).is_none());
    }
}
