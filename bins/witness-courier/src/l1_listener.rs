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

    /// L2 block header committed in `acceptNextBatch` calldata.
    struct L2BlockHeader {
        bytes32 previousBlockHash;
        bytes32 blockHash;
        bytes32 withdrawalRoot;
        bytes32 depositRoot;
        uint256 depositCount;
    }

    /// Function ABI for calldata decoding.
    function acceptNextBatch(L2BlockHeader[] calldata blockHeaders, uint256 expectedBlobsCount) external;
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

const PAGE_SIZE: u64 = 2_000;

/// Single poll iteration: fetch logs from `from_block` to latest, paginated.
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
        return Ok(from_block.saturating_sub(1));
    }

    let mut current = from_block;

    while current <= latest_block {
        let page_end = (current + PAGE_SIZE - 1).min(latest_block);

        // Query BatchHeadersSubmitted events
        let headers_filter = Filter::new()
            .address(contract_addr)
            .event_signature(BatchHeadersSubmitted::SIGNATURE_HASH)
            .from_block(current)
            .to_block(page_end);

        let headers_logs = provider
            .get_logs(&headers_filter)
            .await
            .map_err(|e| eyre!("BatchHeadersSubmitted log query failed [{current}..{page_end}]: {e}"))?;

        for log in &headers_logs {
            match BatchHeadersSubmitted::decode_log_data(&log.inner.data) {
                Ok(event) => {
                    let batch_index: u64 = event.batchIndex.try_into().unwrap_or(u64::MAX);
                    let expected_blobs: u64 =
                        event.expectedBlobsCount.try_into().unwrap_or(u64::MAX);

                    let num_blocks = match fetch_block_count_from_tx(provider, log.transaction_hash).await {
                        Ok(n) => n,
                        Err(e) => {
                            return Err(eyre!(
                                "Failed to fetch block count for batch {batch_index}: {e}"
                            ));
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
            .from_block(current)
            .to_block(page_end);

        let accepted_logs = provider
            .get_logs(&accepted_filter)
            .await
            .map_err(|e| eyre!("BatchAccepted log query failed [{current}..{page_end}]: {e}"))?;

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

        current = page_end + 1;
    }

    Ok(latest_block)
}

/// Decode `acceptNextBatch` calldata from a transaction hash.
///
/// Returns the decoded call struct containing all block headers.
/// The caller decides whether to retry or abort on error.
pub async fn decode_accept_next_batch(
    provider: &RootProvider,
    tx_hash: Option<B256>,
) -> eyre::Result<acceptNextBatchCall> {
    use alloy_sol_types::SolCall;

    let hash = tx_hash.ok_or_else(|| eyre::eyre!("log has no transaction hash"))?;

    let tx = provider
        .get_transaction_by_hash(hash)
        .await
        .map_err(|e| eyre::eyre!("get_transaction_by_hash failed: {e}"))?
        .ok_or_else(|| eyre::eyre!("transaction {hash} not found"))?;

    let input = tx.input();

    acceptNextBatchCall::abi_decode(input)
        .map_err(|e| eyre::eyre!("Failed to decode acceptNextBatch calldata: {e}"))
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
    Ok(decode_accept_next_batch(provider, tx_hash).await?.blockHeaders.len() as u64)
}

// ---------------------------------------------------------------------------
// Startup: resolve L2 checkpoint from batch ID
// ---------------------------------------------------------------------------

const STARTUP_PAGE_SIZE: u64 = 5_000;

/// Resolve the L2 starting block for a given `batch_id` by looking up the
/// `acceptNextBatch` calldata on L1 and then querying the L2 node for the
/// block number corresponding to `blockHeaders[0].previousBlockHash`.
///
/// Returns `(l2_from_block, l1_event_block)`:
/// - `l2_from_block`: first L2 block in the batch
/// - `l1_event_block`: L1 block containing the `BatchHeadersSubmitted` event
pub async fn resolve_l2_start_checkpoint(
    l1_provider: &RootProvider,
    l2_provider: &RootProvider,
    contract_addr: Address,
    batch_id: u64,
    l1_deploy_block: u64,
) -> eyre::Result<(u64, u64)> {
    use alloy_primitives::U256;

    let latest = l1_provider
        .get_block_number()
        .await
        .map_err(|e| eyre!("Failed to get latest L1 block: {e}"))?;

    let batch_topic = B256::from(U256::from(batch_id));

    // Paginated scan for the BatchHeadersSubmitted event with the target batch index.
    let mut current = l1_deploy_block;
    while current <= latest {
        let to = (current + STARTUP_PAGE_SIZE - 1).min(latest);

        let filter = Filter::new()
            .address(contract_addr)
            .event_signature(BatchHeadersSubmitted::SIGNATURE_HASH)
            .topic1(batch_topic)
            .from_block(current)
            .to_block(to);

        let logs = l1_provider
            .get_logs(&filter)
            .await
            .map_err(|e| eyre!("eth_getLogs [{current}..{to}] failed: {e}"))?;

        if let Some(log) = logs.first() {
            let l1_block = log
                .block_number
                .ok_or_else(|| eyre!("BatchHeadersSubmitted log missing block_number"))?;

            let decoded = decode_accept_next_batch(l1_provider, log.transaction_hash)
                .await
                .map_err(|e| eyre!("Failed to decode calldata for batch {batch_id}: {e}"))?;

            let num_blocks = decoded.blockHeaders.len() as u64;

            // Batch 0 always starts at L2 block 1 (genesis).
            let l2_from_block = if batch_id == 0 {
                1
            } else {
                let prev_hash = decoded.blockHeaders[0].previousBlockHash;
                let block = l2_provider
                    .get_block_by_hash(prev_hash)
                    .await
                    .map_err(|e| eyre!("L2 eth_getBlockByHash({prev_hash}) failed: {e}"))?
                    .ok_or_else(|| eyre!(
                        "L2 block with hash {prev_hash} not found — \
                         is the L2 RPC (FLUENT_FALLBACK_LOCAL_RPC) synced?"
                    ))?;
                block.header.number + 1
            };

            info!(
                batch_id,
                l2_from_block,
                num_blocks,
                l1_block,
                "Resolved L2 start checkpoint from L1 calldata + L2 block lookup"
            );

            return Ok((l2_from_block, l1_block));
        }

        current = to + 1;
    }

    eyre::bail!(
        "BatchHeadersSubmitted for batch {batch_id} not found in L1 blocks \
         [{l1_deploy_block}..{latest}]. \
         Ensure FLUENT_L1_DEPLOY_BLOCK is ≤ the block where batch 0 was submitted, \
         and that L1_RPC_URL is correct."
    )
}
