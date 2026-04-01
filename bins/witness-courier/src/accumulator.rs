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
use std::sync::Arc;

use tracing::{info, warn};

use crate::db::Db;
use crate::types::EthExecutionResponse;

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
    db: Option<Arc<Db>>,
}

impl BatchAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create accumulator backed by a DB. Loads all state from DB on construction.
    pub fn with_db(db: Arc<Db>) -> Self {
        let responses: HashMap<u64, EthExecutionResponse> = db
            .load_responses()
            .into_iter()
            .map(|r| (r.block_number, r))
            .collect();
        let batches: BTreeMap<u64, PendingBatch> = db
            .load_batches()
            .into_iter()
            .map(|b| (b.batch_index, b))
            .collect();
        let pending_blobs_accepted: HashSet<u64> = db
            .load_pending_blobs_accepted()
            .into_iter()
            .collect();

        Self {
            batches,
            responses,
            pending_blobs_accepted,
            db: Some(db),
        }
    }

    /// Register a new batch from `BatchHeadersSubmitted` event.
    pub fn set_batch(&mut self, batch_index: u64, from_block: u64, to_block: u64) {
        // Consume any buffered BlobsAccepted for this batch
        let blobs_accepted = self.pending_blobs_accepted.remove(&batch_index);
        if blobs_accepted {
            if let Some(db) = &self.db { db.delete_pending_blobs_accepted(batch_index); }
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
        if let Some(db) = &self.db { db.save_batch(&batch); }
        self.batches.insert(batch_index, batch);
    }

    /// Store a block execution response. O(1).
    pub fn insert_response(&mut self, resp: EthExecutionResponse) {
        if let Some(db) = &self.db { db.save_response(&resp); }
        let block = resp.block_number;
        self.responses.insert(block, resp);
    }

    pub fn mark_blobs_accepted(&mut self, batch_index: u64) {
        if let Some(batch) = self.batches.get_mut(&batch_index) {
            batch.blobs_accepted = true;
            if let Some(db) = &self.db { db.update_blobs_accepted(batch_index); }
            info!(batch_index, "Blobs accepted on L1");
        } else {
            // BatchHeadersSubmitted not yet seen — buffer for when set_batch arrives
            self.pending_blobs_accepted.insert(batch_index);
            if let Some(db) = &self.db { db.save_pending_blobs_accepted(batch_index); }
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

    pub fn get(&self, batch_index: u64) -> Option<&PendingBatch> {
        self.batches.get(&batch_index)
    }

    /// Remove a completed batch and drain its responses from the pool.
    pub fn take(&mut self, batch_index: u64) -> Option<PendingBatch> {
        let batch = self.batches.remove(&batch_index)?;
        if let Some(db) = &self.db {
            db.delete_batch(batch_index);
            db.delete_responses(batch.from_block, batch.to_block);
        }
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
    pub fn handle_reorg(&mut self, reverted: &[u64]) {
        for &block in reverted {
            self.responses.remove(&block);
            if let Some(db) = &self.db { db.delete_response(block); }
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
            let batch = self.batches.remove(idx).unwrap();
            if let Some(db) = &self.db {
                db.delete_batch(*idx);
                db.delete_responses(batch.from_block, batch.to_block);
            }
            // Also clean responses for non-reverted blocks in affected batches —
            // the entire batch is invalid, canonical replacements will re-populate.
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
            parent_hash: B256::ZERO,
            block_hash: B256::ZERO,
            withdrawal_hash: B256::ZERO,
            deposit_hash: B256::ZERO,
            tx_data_hash: B256::ZERO,
            result_hash: vec![],
            signature: vec![],
        }
    }

    #[test]
    fn not_ready_without_blobs_accepted() {
        let mut acc = BatchAccumulator::new();
        acc.set_batch(1, 10, 12);
        acc.insert_response(mock_response(10));
        acc.insert_response(mock_response(11));
        acc.insert_response(mock_response(12));

        assert!(acc.first_ready().is_none());
        acc.mark_blobs_accepted(1);
        assert_eq!(acc.first_ready(), Some(1));
    }

    #[test]
    fn not_ready_without_all_responses() {
        let mut acc = BatchAccumulator::new();
        acc.set_batch(1, 10, 12);
        acc.mark_blobs_accepted(1);

        acc.insert_response(mock_response(10));
        acc.insert_response(mock_response(11));
        assert!(acc.first_ready().is_none());

        acc.insert_response(mock_response(12));
        assert_eq!(acc.first_ready(), Some(1));
    }

    #[test]
    fn take_removes_batch_and_responses() {
        let mut acc = BatchAccumulator::new();
        acc.set_batch(1, 10, 10);
        acc.insert_response(mock_response(10));
        acc.mark_blobs_accepted(1);

        let batch = acc.take(1).unwrap();
        assert_eq!(batch.batch_index, 1);
        assert!(acc.first_ready().is_none());
        assert!(acc.responses.is_empty());
    }

    #[test]
    fn concurrent_batches() {
        let mut acc = BatchAccumulator::new();
        acc.set_batch(1, 10, 11);
        acc.set_batch(2, 12, 13);

        acc.insert_response(mock_response(10));
        acc.insert_response(mock_response(11));
        acc.mark_blobs_accepted(1);
        assert_eq!(acc.first_ready(), Some(1));

        acc.insert_response(mock_response(12));
        acc.insert_response(mock_response(13));
        acc.mark_blobs_accepted(2);

        acc.take(1);
        assert_eq!(acc.first_ready(), Some(2));
    }

    #[test]
    fn responses_before_batch_registration() {
        let mut acc = BatchAccumulator::new();

        // Normal flow: responses arrive before acceptNextBatch
        acc.insert_response(mock_response(10));
        acc.insert_response(mock_response(11));
        acc.insert_response(mock_response(12));

        acc.set_batch(1, 10, 12);
        acc.mark_blobs_accepted(1);
        assert_eq!(acc.first_ready(), Some(1));
    }

    #[test]
    fn reorg_purges_responses_and_batches() {
        let mut acc = BatchAccumulator::new();
        acc.set_batch(1, 10, 12);
        acc.set_batch(2, 13, 15);
        acc.insert_response(mock_response(10));
        acc.insert_response(mock_response(11));
        acc.insert_response(mock_response(12));
        acc.insert_response(mock_response(13));
        acc.mark_blobs_accepted(1);

        // Revert blocks 11-13 — batch 1 and batch 2 both overlap
        acc.handle_reorg(&[11, 12, 13]);

        // Both batches dropped
        assert_eq!(acc.len(), 0);
        // All responses for affected batches cleaned (including 10 from batch 1)
        assert!(acc.responses.is_empty());
    }

    #[test]
    fn reorg_preserves_unaffected_batch() {
        let mut acc = BatchAccumulator::new();
        acc.set_batch(1, 10, 11);
        acc.set_batch(2, 12, 13);
        acc.insert_response(mock_response(10));
        acc.insert_response(mock_response(11));
        acc.insert_response(mock_response(12));
        acc.mark_blobs_accepted(1);

        // Revert only block 13 — batch 1 unaffected
        acc.handle_reorg(&[13]);

        assert_eq!(acc.len(), 1);
        assert_eq!(acc.first_ready(), Some(1));
        // Block 12 was in batch 2, which got dropped — response removed
        assert!(!acc.responses.contains_key(&12));
    }

    #[test]
    fn reorg_cleans_orphan_responses() {
        let mut acc = BatchAccumulator::new();
        // Responses arrived before any batch registered (normal flow)
        acc.insert_response(mock_response(50));
        acc.insert_response(mock_response(51));
        acc.insert_response(mock_response(52));

        acc.handle_reorg(&[51, 52]);

        assert!(acc.responses.contains_key(&50));
        assert!(!acc.responses.contains_key(&51));
        assert!(!acc.responses.contains_key(&52));
    }

    #[test]
    fn take_only_drains_own_blocks() {
        let mut acc = BatchAccumulator::new();
        acc.set_batch(1, 10, 10);
        acc.set_batch(2, 11, 11);
        acc.insert_response(mock_response(10));
        acc.insert_response(mock_response(11));

        acc.take(1);
        assert!(acc.responses.contains_key(&11));
        assert!(!acc.responses.contains_key(&10));
    }
}
