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
    l1_ckpt_tx: mpsc::Sender<u64>,
) -> ! {
    info!(
        %contract_addr,
        from_block,
        "L1 listener started"
    );

    loop {
        match poll_once(&l1_provider, contract_addr, from_block, &tx).await {
            Ok(latest) => {
                // Always advance — poll_once now guarantees latest >= from_block - 1,
                // so latest + 1 is always ≥ from_block (no regression).
                let _ = l1_ckpt_tx.send(latest).await;
                from_block = latest + 1;
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
        // No new blocks — return (from_block - 1) so caller's `latest + 1` keeps
        // from_block unchanged on the next tick without re-emitting old events.
        return Ok(from_block.saturating_sub(1));
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
                let num_blocks = match fetch_block_count_from_tx(provider, log.transaction_hash).await {
                    Ok(n) => n,
                    Err(e) => {
                        error!(
                            batch_index,
                            err = %e,
                            "Skipping BatchHeadersSubmitted: could not decode block count from calldata"
                        );
                        continue;
                    }
                };

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
/// Returns `Err` on network failure or malformed calldata.
/// The caller decides whether to retry or abort.
pub async fn fetch_block_count_from_tx(
    provider: &RootProvider,
    tx_hash: Option<B256>,
) -> eyre::Result<u64> {
    let hash = tx_hash.ok_or_else(|| eyre::eyre!("log has no transaction hash"))?;

    let tx = provider
        .get_transaction_by_hash(hash)
        .await
        .map_err(|e| eyre::eyre!("get_transaction_by_hash failed: {e}"))?
        .ok_or_else(|| eyre::eyre!("transaction {hash} not found"))?;

    let input = tx.input();

    if input.len() < 68 {
        eyre::bail!(
            "calldata too short ({} bytes) — expected ≥68 for acceptNextBatch",
            input.len()
        );
    }

    let offset_bytes: [u8; 32] = input[4..36]
        .try_into()
        .map_err(|_| eyre::eyre!("failed to read array offset from calldata"))?;
    let offset = u256_to_u64(offset_bytes);

    let len_start = 4 + offset as usize;
    let len_end = len_start + 32;
    if input.len() < len_end {
        eyre::bail!(
            "calldata too short to read array length at offset {offset} (need {len_end}, have {})",
            input.len()
        );
    }

    let len_bytes: [u8; 32] = input[len_start..len_end]
        .try_into()
        .map_err(|_| eyre::eyre!("failed to read array length from calldata"))?;

    Ok(u256_to_u64(len_bytes))
}

/// Convert a big-endian 32-byte uint256 to u64 (saturating).
fn u256_to_u64(bytes: [u8; 32]) -> u64 {
    // Check if any of the upper 24 bytes are non-zero → saturate
    if bytes[..24].iter().any(|&b| b != 0) {
        return u64::MAX;
    }
    u64::from_be_bytes(bytes[24..32].try_into().unwrap())
}
