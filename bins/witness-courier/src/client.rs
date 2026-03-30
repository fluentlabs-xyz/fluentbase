//! gRPC client with automatic reconnect, parallel block dispatch, and batch
//! orchestration.
//!
//! The courier binary calls [`run`] which loops forever: connect → stream →
//! on disconnect, read checkpoint, reconnect with exponential backoff.
//!
//! # Compression
//!
//! Witnesses arrive from the co-located gRPC server as raw bincode (no
//! compression on localhost). Before forwarding to the remote proxy via HTTP,
//! the courier compresses each payload with zstd. This is the single point
//! where compression happens in the pipeline.
//!
//! # Checkpoint file
//!
//! After each successfully processed witness the courier writes the block
//! number to a file (default `/tmp/witness_courier_checkpoint`). On restart
//! it reads this file and sends `Subscribe(from_block = checkpoint + 1)`.
//!
//! # Orchestration
//!
//! The orchestrator merges three event streams:
//! 1. gRPC `WitnessMessage` → spawn parallel `/sign-block-execution` tasks
//! 2. `/sign-block-execution` responses → collect in `BatchAccumulator`
//! 3. L1 events → set batch boundaries and mark blobs accepted
//!
//! When all conditions are met, triggers `/sign-batch-root` → `preconfirmBatch`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, Semaphore};
use tonic::transport::Channel;
use tracing::{error, info, warn};

use crate::accumulator::BatchAccumulator;
use crate::l1_listener::L1Event;
use crate::l1_submitter;
use crate::proto::witness_service_client::WitnessServiceClient;
use crate::proto::SubscribeRequest;
use crate::types::{EthExecutionResponse, SubmitBatchResponse};

use alloy_primitives::Address;
use alloy_provider::Provider;

const MAX_BACKOFF: Duration = Duration::from_secs(30);
const INITIAL_BACKOFF: Duration = Duration::from_millis(500);

/// Zstd compression level for network transfer to remote proxy.
const ZSTD_LEVEL: i32 = 3;

/// Max compressed payload size for network transfer.
const MAX_COMPRESSED_BYTES: usize = 256 * 1024 * 1024; // 256 MiB

/// Max concurrent `/sign-block-execution` requests.
const MAX_CONCURRENT_REQUESTS: usize = 32;

/// Max retries per block request.
const MAX_RETRIES: u32 = 5;

/// Configuration for the orchestrator.
pub struct OrchestratorConfig<P: Provider + Clone + 'static> {
    pub server_addr: String,
    pub proxy_url: String,
    pub checkpoint_path: PathBuf,
    pub http_client: reqwest::Client,
    pub l1_contract_addr: Address,
    pub nitro_verifier_addr: Address,
    pub l1_provider: P,
    pub api_key: String,
}

/// Response from a `/sign-block-execution` request.
struct BlockResult {
    block_number: u64,
    response: EthExecutionResponse,
}

/// Run the courier orchestrator loop forever.
///
/// Merges gRPC witness stream, block execution responses, and L1 events.
pub async fn run<P: Provider + Clone + 'static>(config: OrchestratorConfig<P>, mut l1_events: mpsc::Receiver<L1Event>) -> ! {
    let mut backoff = INITIAL_BACKOFF;
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));
    let mut accumulator = BatchAccumulator::new();

    // Channel for completed block execution responses
    let (result_tx, mut result_rx) = mpsc::channel::<BlockResult>(256);

    // Track sequential batch boundaries
    let mut next_batch_from_block: Option<u64> = None;

    loop {
        let from_block = load_checkpoint(&config.checkpoint_path);
        info!(from_block, "Connecting to witness server");

        match run_stream(
            &config,
            from_block,
            &semaphore,
            &result_tx,
            &mut result_rx,
            &mut l1_events,
            &mut accumulator,
            &mut next_batch_from_block,
        )
        .await
        {
            Ok(()) => {
                info!("Stream ended gracefully");
                backoff = INITIAL_BACKOFF;
            }
            Err(e) => {
                warn!(
                    err = %e,
                    backoff_ms = backoff.as_millis(),
                    "Stream interrupted — reconnecting"
                );
            }
        }

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Single stream session with full orchestration.
#[allow(clippy::too_many_arguments)]
async fn run_stream<P: Provider + Clone + 'static>(
    config: &OrchestratorConfig<P>,
    from_block: u64,
    semaphore: &Arc<Semaphore>,
    result_tx: &mpsc::Sender<BlockResult>,
    result_rx: &mut mpsc::Receiver<BlockResult>,
    l1_events: &mut mpsc::Receiver<L1Event>,
    accumulator: &mut BatchAccumulator,
    next_batch_from_block: &mut Option<u64>,
) -> eyre::Result<()> {
    let channel = Channel::from_shared(config.server_addr.clone())?
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(600))
        .connect()
        .await?;

    let mut client = WitnessServiceClient::new(channel)
        .max_encoding_message_size(usize::MAX)
        .max_decoding_message_size(usize::MAX);

    info!(from_block, "Subscribed to witness stream");

    let mut stream = client
        .subscribe(SubscribeRequest { from_block })
        .await?
        .into_inner();

    loop {
        tokio::select! {
            // ── Stream A: gRPC witness / reorg messages ────────────────
            msg = stream.message() => {
                use crate::proto::witness_message::Content;

                let Some(msg) = msg? else {
                    info!("Witness stream ended");
                    return Ok(());
                };

                match msg.content {
                    Some(Content::Reorg(reorg)) => {
                        let blocks = reorg.reverted_block_numbers;
                        warn!(?blocks, "Reorg received — purging stale state");
                        accumulator.handle_reorg(&blocks);
                        rollback_checkpoint(&config.checkpoint_path, &blocks);
                        continue;
                    }
                    Some(Content::Witness(witness)) => {
                        let block_number = witness.block_number;
                        let raw_bytes = witness.data.len();

                        let compressed = match compress_witness(&witness.data, block_number) {
                            Ok(data) => data,
                            Err(()) => continue,
                        };

                        if compressed.len() > MAX_COMPRESSED_BYTES {
                            error!(
                                block_number,
                                compressed_bytes = compressed.len(),
                                limit = MAX_COMPRESSED_BYTES,
                                "Compressed witness exceeds limit — skipping"
                            );
                            continue;
                        }

                        let compressed_bytes = compressed.len();
                        info!(
                            block_number,
                            raw_bytes,
                            compressed_bytes,
                            ratio = format_args!("{:.1}x", raw_bytes as f64 / compressed_bytes.max(1) as f64),
                            "Dispatching witness"
                        );

                        spawn_block_request(
                            config.http_client.clone(),
                            config.proxy_url.clone(),
                            config.api_key.clone(),
                            block_number,
                            compressed,
                            Arc::clone(semaphore),
                            result_tx.clone(),
                        );

                        save_checkpoint(&config.checkpoint_path, block_number);
                    }
                    None => continue,
                }
            }

            // ── Stream B: block execution responses ──────────────────
            Some(result) = result_rx.recv() => {
                info!(
                    block_number = result.block_number,
                    "Block execution response received"
                );
                accumulator.insert_response(result.response);

                // Drain all ready batches (process in order)
                while let Some(batch_index) = accumulator.first_ready() {
                    handle_batch_ready(config, accumulator, batch_index).await;
                }
            }

            // ── Stream C: L1 events ──────────────────────────────────
            Some(event) = l1_events.recv() => {
                match event {
                    L1Event::BatchHeaders {
                        batch_index,
                        expected_blobs: _,
                        batch_root: _,
                        num_blocks,
                    } => {
                        let from = next_batch_from_block.unwrap_or(from_block);
                        let to = from + num_blocks.saturating_sub(1);

                        info!(
                            batch_index,
                            from,
                            to,
                            num_blocks,
                            "Setting batch from L1 event"
                        );

                        accumulator.set_batch(batch_index, from, to);
                        *next_batch_from_block = Some(to + 1);
                    }

                    L1Event::BlobsAccepted { batch_index } => {
                        accumulator.mark_blobs_accepted(batch_index);

                        while let Some(batch_index) = accumulator.first_ready() {
                            handle_batch_ready(config, accumulator, batch_index).await;
                        }
                    }
                }
            }
        }
    }
}

/// When all conditions are met: call `/sign-batch-root`, then `preconfirmBatch`.
///
/// Always removes the batch from the accumulator (whether success or failure)
/// to prevent infinite retry loops. On transient failures the batch is lost —
/// the L1 state machine will time out and the batch can be retried externally.
async fn handle_batch_ready<P: Provider + Clone + 'static>(
    config: &OrchestratorConfig<P>,
    accumulator: &mut BatchAccumulator,
    batch_index: u64,
) {
    // Take the batch out immediately — even on failure we don't want the
    // `while first_ready()` loop to spin on it.
    let batch = match accumulator.take(batch_index) {
        Some(b) => b,
        None => return,
    };

    let from_block = batch.from_block;
    let to_block = batch.to_block;

    info!(
        batch_index,
        from_block,
        to_block,
        "Batch ready — triggering /sign-batch-root"
    );

    // POST /sign-batch-root { from_block, to_block, batch_index }
    let sign_result = call_sign_batch_root(
        &config.http_client,
        &config.proxy_url,
        &config.api_key,
        from_block,
        to_block,
        batch_index,
    )
    .await;

    let batch_resp = match sign_result {
        Ok(resp) => resp,
        Err(e) => {
            error!(batch_index, err = %e, "Failed to sign batch root — batch dropped");
            return;
        }
    };

    info!(
        batch_index,
        signature_len = batch_resp.signature.len(),
        "Batch root signed — submitting preconfirmBatch"
    );

    // Submit preconfirmBatch to L1
    if let Err(e) = l1_submitter::submit_preconfirmation(
        &config.l1_provider,
        config.l1_contract_addr,
        config.nitro_verifier_addr,
        batch_index,
        batch_resp.signature,
    )
    .await
    {
        error!(batch_index, err = %e, "preconfirmBatch failed — batch dropped");
        return;
    }

    info!(batch_index, "Batch preconfirmed on L1");
}

/// Call the proxy's `/sign-batch-root` endpoint.
async fn call_sign_batch_root(
    http_client: &reqwest::Client,
    proxy_url: &str,
    api_key: &str,
    from_block: u64,
    to_block: u64,
    batch_index: u64,
) -> eyre::Result<SubmitBatchResponse> {
    // Derive base URL from proxy_url (which points to /sign-block-execution)
    let base = proxy_url
        .rfind('/')
        .map(|i| &proxy_url[..i])
        .unwrap_or(proxy_url);
    let url = format!("{base}/sign-batch-root");

    let body = serde_json::json!({
        "from_block": from_block,
        "to_block": to_block,
        "batch_index": batch_index,
    });

    let resp = http_client
        .post(&url)
        .header("x-api-key", api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| eyre::eyre!("sign-batch-root request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(eyre::eyre!("sign-batch-root returned {status}: {text}"));
    }

    resp.json::<SubmitBatchResponse>()
        .await
        .map_err(|e| eyre::eyre!("Failed to parse SubmitBatchResponse: {e}"))
}

/// Spawn a parallel `/sign-block-execution` request with retry logic.
fn spawn_block_request(
    http_client: reqwest::Client,
    proxy_url: String,
    api_key: String,
    block_number: u64,
    compressed: Vec<u8>,
    semaphore: Arc<Semaphore>,
    result_tx: mpsc::Sender<BlockResult>,
) {
    tokio::spawn(async move {
        let _permit = match semaphore.acquire().await {
            Ok(p) => p,
            Err(_) => return,
        };

        let mut backoff = Duration::from_millis(500);

        for attempt in 1..=MAX_RETRIES {
            match send_block_request(&http_client, &proxy_url, &api_key, block_number, &compressed).await {
                Ok(response) => {
                    let _ = result_tx
                        .send(BlockResult {
                            block_number,
                            response,
                        })
                        .await;
                    return;
                }
                Err(e) => {
                    if attempt < MAX_RETRIES {
                        warn!(
                            block_number,
                            attempt,
                            err = %e,
                            backoff_ms = backoff.as_millis(),
                            "Block request failed — retrying"
                        );
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    } else {
                        error!(
                            block_number,
                            err = %e,
                            "Block request failed after {MAX_RETRIES} attempts"
                        );
                    }
                }
            }
        }
    });
}

/// Send a single `/sign-block-execution` request.
async fn send_block_request(
    http_client: &reqwest::Client,
    proxy_url: &str,
    api_key: &str,
    block_number: u64,
    compressed: &[u8],
) -> eyre::Result<EthExecutionResponse> {
    let resp = http_client
        .post(proxy_url)
        .header("content-type", "application/octet-stream")
        .header("content-encoding", "zstd")
        .header("x-block-number", block_number.to_string())
        .header("x-api-key", api_key)
        .body(compressed.to_vec())
        .send()
        .await
        .map_err(|e| eyre::eyre!("HTTP POST failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_else(|_| "<unreadable>".into());
        return Err(eyre::eyre!("proxy returned {status}: {body}"));
    }

    resp.json::<EthExecutionResponse>()
        .await
        .map_err(|e| eyre::eyre!("Failed to parse EthExecutionResponse: {e}"))
}

/// Compresses a raw witness payload with zstd.
fn compress_witness(raw: &[u8], block_number: u64) -> Result<Vec<u8>, ()> {
    zstd::encode_all(raw, ZSTD_LEVEL).map_err(|e| {
        error!(
            block_number,
            err = %e,
            raw_bytes = raw.len(),
            "Zstd compression failed"
        );
    })
}

/// Read the last checkpointed block number from disk.
///
/// Returns `checkpoint + 1` (the next block to request).
/// Returns `0` if the file is missing or unreadable.
pub fn load_checkpoint(path: &Path) -> u64 {
    match std::fs::read_to_string(path) {
        Ok(s) => s.trim().parse::<u64>().map(|n| n + 1).unwrap_or(0),
        Err(_) => 0,
    }
}

/// Persist the block number to the checkpoint file.
pub fn save_checkpoint(path: &Path, block_number: u64) {
    if let Err(e) = std::fs::write(path, block_number.to_string()) {
        error!(err = %e, "Failed to save checkpoint");
    }
}

/// Roll checkpoint back to just before the earliest reverted block.
///
/// On next reconnect the courier will re-request from `min(reverted) - 1`,
/// ensuring canonical replacement witnesses are fetched.
fn rollback_checkpoint(path: &Path, reverted: &[u64]) {
    if let Some(&min) = reverted.iter().min() {
        let new_checkpoint = min.saturating_sub(1);
        save_checkpoint(path, new_checkpoint);
        info!(new_checkpoint, "Checkpoint rolled back due to reorg");
    }
}
