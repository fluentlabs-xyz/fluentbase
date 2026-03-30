//! L1 event listener for batch lifecycle events.
//!
//! Polls the rollup contract on L1 for:
//! - `BatchHeadersSubmitted(batchIndex, batchRoot, expectedBlobsCount)` — new batch declared
//! - `BatchAccepted(batchIndex)` — all blobs submitted for the batch
//!
//! Events are sent to the orchestrator via an mpsc channel.

use alloy_primitives::{Address, B256};
use alloy_provider::{Provider, RootProvider};
use alloy_rpc_types::{Filter, TransactionTrait};
use alloy_sol_types::{sol, SolEvent};
use eyre::{eyre, Result};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// L1 contract ABI (minimal)
// ---------------------------------------------------------------------------

sol! {
    /// Emitted by `acceptNextBatch` — declares a new batch with its block range.
    event BatchHeadersSubmitted(
        uint256 indexed batchIndex,
        bytes32 batchRoot,
        uint256 expectedBlobsCount
    );

    /// Emitted when all blobs for a batch have been submitted via `submitBlobs`.
    event BatchAccepted(uint256 indexed batchIndex);
}

// ---------------------------------------------------------------------------
// Event types sent to orchestrator
// ---------------------------------------------------------------------------

/// Events the L1 listener sends to the orchestrator.
#[derive(Debug)]
pub enum L1Event {
    /// A new batch has been declared on L1.
    BatchHeaders {
        batch_index: u64,
        batch_root: B256,
        expected_blobs: u64,
        /// Number of block headers in the `acceptNextBatch` calldata.
        /// Used with sequential tracking to determine from_block/to_block.
        num_blocks: u64,
    },
    /// All blobs for the batch have been accepted.
    BlobsAccepted { batch_index: u64 },
}

// ---------------------------------------------------------------------------
// Listener loop
// ---------------------------------------------------------------------------

const POLL_INTERVAL_SECS: u64 = 6;

/// Run the L1 event listener loop.
///
/// Polls L1 logs starting from `from_block` and sends parsed events to `tx`.
/// This function runs forever.
pub async fn run(
    l1_provider: RootProvider,
    contract_addr: Address,
    mut from_block: u64,
    tx: mpsc::Sender<L1Event>,
) -> ! {
    info!(
        %contract_addr,
        from_block,
        "L1 listener started"
    );

    loop {
        match poll_once(&l1_provider, contract_addr, from_block, &tx).await {
            Ok(latest) => {
                if latest > from_block {
                    from_block = latest + 1;
                }
            }
            Err(e) => {
                warn!(err = %e, "L1 poll failed — retrying");
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS)).await;
    }
}

/// Single poll iteration: fetch logs from `from_block` to latest.
async fn poll_once(
    provider: &RootProvider,
    contract_addr: Address,
    from_block: u64,
    tx: &mpsc::Sender<L1Event>,
) -> Result<u64> {
    let latest_block = provider
        .get_block_number()
        .await
        .map_err(|e| eyre!("Failed to get latest block: {e}"))?;

    if from_block > latest_block {
        return Ok(from_block);
    }

    // Query BatchHeadersSubmitted events
    let headers_filter = Filter::new()
        .address(contract_addr)
        .event_signature(BatchHeadersSubmitted::SIGNATURE_HASH)
        .from_block(from_block)
        .to_block(latest_block);

    let headers_logs = provider
        .get_logs(&headers_filter)
        .await
        .map_err(|e| eyre!("BatchHeadersSubmitted log query failed: {e}"))?;

    for log in &headers_logs {
        match BatchHeadersSubmitted::decode_log_data(&log.inner.data) {
            Ok(event) => {
                let batch_index: u64 = event.batchIndex.try_into().unwrap_or(u64::MAX);
                let expected_blobs: u64 =
                    event.expectedBlobsCount.try_into().unwrap_or(u64::MAX);

                // Fetch the transaction to determine block count from calldata length.
                let num_blocks =
                    fetch_block_count_from_tx(provider, log.transaction_hash).await;

                info!(
                    batch_index,
                    expected_blobs,
                    num_blocks,
                    "BatchHeadersSubmitted event"
                );

                let _ = tx
                    .send(L1Event::BatchHeaders {
                        batch_index,
                        batch_root: event.batchRoot,
                        expected_blobs,
                        num_blocks,
                    })
                    .await;
            }
            Err(e) => {
                error!(err = %e, "Failed to decode BatchHeadersSubmitted");
            }
        }
    }

    // Query BatchAccepted events
    let accepted_filter = Filter::new()
        .address(contract_addr)
        .event_signature(BatchAccepted::SIGNATURE_HASH)
        .from_block(from_block)
        .to_block(latest_block);

    let accepted_logs = provider
        .get_logs(&accepted_filter)
        .await
        .map_err(|e| eyre!("BatchAccepted log query failed: {e}"))?;

    for log in &accepted_logs {
        match BatchAccepted::decode_log_data(&log.inner.data) {
            Ok(event) => {
                let batch_index: u64 = event.batchIndex.try_into().unwrap_or(u64::MAX);
                info!(batch_index, "BatchAccepted event");

                let _ = tx
                    .send(L1Event::BlobsAccepted { batch_index })
                    .await;
            }
            Err(e) => {
                error!(err = %e, "Failed to decode BatchAccepted");
            }
        }
    }

    Ok(latest_block)
}

/// Attempt to determine the number of block headers from the `acceptNextBatch`
/// transaction calldata.
///
/// Falls back to 0 if the transaction can't be fetched or decoded.
async fn fetch_block_count_from_tx(
    provider: &RootProvider,
    tx_hash: Option<B256>,
) -> u64 {
    let Some(hash) = tx_hash else {
        return 0;
    };

    let tx = match provider.get_transaction_by_hash(hash).await {
        Ok(Some(tx)) => tx,
        _ => return 0,
    };

    let input = tx.input();

    // acceptNextBatch(BlockHeader[] calldata, uint256)
    // Selector: 4 bytes
    // Offset to dynamic array: 32 bytes
    // expectedBlobs: 32 bytes
    // Array length: 32 bytes (at the offset position)
    //
    // We read the array length from the calldata.
    // The offset is at bytes [4..36], then the length is at [offset+4 .. offset+4+32].
    if input.len() < 68 {
        return 0;
    }

    // Read offset (first param is dynamic array → its value is the offset)
    let offset_bytes: [u8; 32] = match input[4..36].try_into() {
        Ok(b) => b,
        Err(_) => return 0,
    };
    let offset = u256_to_u64(offset_bytes);

    // Array length is at (4 + offset) .. (4 + offset + 32)
    let len_start = 4 + offset as usize;
    let len_end = len_start + 32;
    if input.len() < len_end {
        return 0;
    }

    let len_bytes: [u8; 32] = match input[len_start..len_end].try_into() {
        Ok(b) => b,
        Err(_) => return 0,
    };
    u256_to_u64(len_bytes)
}

/// Convert a big-endian 32-byte uint256 to u64 (saturating).
fn u256_to_u64(bytes: [u8; 32]) -> u64 {
    // Check if any of the upper 24 bytes are non-zero → saturate
    if bytes[..24].iter().any(|&b| b != 0) {
        return u64::MAX;
    }
    u64::from_be_bytes(bytes[24..32].try_into().unwrap())
}
