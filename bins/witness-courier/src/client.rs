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

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, Semaphore};
use tokio::time::MissedTickBehavior;
use tonic::transport::Channel;
use tracing::{error, info, warn};

use crate::accumulator::BatchAccumulator;
use crate::db::Db;
use crate::l1_listener::L1Event;
use crate::l1_submitter;
use crate::proto::witness_service_client::WitnessServiceClient;
use crate::proto::SubscribeRequest;
use crate::types::{EthExecutionResponse, SubmitBatchResponse};

use alloy_network::Ethereum;
use alloy_primitives::Address;
use alloy_provider::Provider;
use rsp_client_executor::{evm::FluentEvmConfig, io::ClientExecutorInput};
use rsp_host_executor::HostExecutor;
use rsp_primitives::genesis::Genesis;
use rsp_provider::create_provider;

const MAX_BACKOFF: Duration = Duration::from_secs(30);
const INITIAL_BACKOFF: Duration = Duration::from_millis(500);

/// Zstd compression level for network transfer to remote proxy.
const ZSTD_LEVEL: i32 = 3;

/// Max compressed payload size for network transfer.
const MAX_COMPRESSED_BYTES: usize = 256 * 1024 * 1024; // 256 MiB

/// Max concurrent `/sign-block-execution` requests.
const MAX_CONCURRENT_REQUESTS: usize = 32;

/// Blocks to wait past a missing block before triggering L3/L4 fallback.
const FALLBACK_GAP_M: u64 = 32;

/// Maximum gaps dispatched to L3/L4 fallback per tick.
/// Prevents O(N) memory and task explosion when historical gap is large.
const FALLBACK_BATCH_SIZE: usize = 128;


/// Configuration for the orchestrator.
#[derive(Clone)]
pub struct OrchestratorConfig<P: Provider + Clone + 'static> {
    pub server_addr: String,
    pub proxy_url: String,
    pub db_path: PathBuf,
    pub http_client: reqwest::Client,
    pub l1_contract_addr: Address,
    pub nitro_verifier_addr: Address,
    pub l1_provider: P,
    pub api_key: String,
    pub fallback_local_rpc: Option<String>,
    pub fallback_remote_rpc: Option<String>,
}

/// Response from a `/sign-block-execution` request.
struct BlockResult {
    block_number: u64,
    response: EthExecutionResponse,
}

/// Run the courier orchestrator loop forever.
///
/// Merges gRPC witness stream, block execution responses, and L1 events.
pub async fn run<P: Provider + Clone + 'static>(config: OrchestratorConfig<P>, mut l1_events: mpsc::Receiver<L1Event>, mut l1_ckpt_rx: mpsc::Receiver<u64>) -> ! {
    let db = Arc::new(Db::open(&config.db_path).expect("Failed to open courier DB"));

    let mut accumulator = BatchAccumulator::with_db(Arc::clone(&db));
    let mut backoff = INITIAL_BACKOFF;
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));

    // Channel for completed block execution responses
    let (result_tx, mut result_rx) = mpsc::channel::<BlockResult>(256);

    // Track sequential batch boundaries — recover rightmost known batch boundary on restart
    let mut next_batch_from_block: Option<u64> =
        accumulator.max_to_block().map(|e| e + 1)
            .or_else(|| db.get_last_batch_end().map(|e| e + 1));

    // Pre-compute the initial confirmed set from DB responses above the checkpoint.
    // This allows the watermark to advance through already-loaded responses immediately
    // on the first run_stream call without re-requesting those blocks.
    let checkpoint = db.get_checkpoint();
    let initial_confirmed: BTreeSet<u64> = db
        .get_all_response_block_numbers()
        .into_iter()
        .filter(|&b| b > checkpoint)
        .collect();

    loop {
        let from_block = db.get_checkpoint() + 1;
        info!(from_block, "Connecting to witness server");

        match run_stream(
            &config,
            &db,
            from_block,
            initial_confirmed.clone(),
            &semaphore,
            &result_tx,
            &mut result_rx,
            &mut l1_events,
            &mut l1_ckpt_rx,
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

/// Per-session orchestration state — lives for one gRPC connection.
///
/// Created fresh on each [`run_stream`] call; dropped on disconnect.
/// Cross-session state (`accumulator`, `next_batch_from_block`) lives in [`run`] and is
/// passed by `&mut` so it survives reconnects.
struct StreamState<P: Provider + Clone + 'static> {
    config: OrchestratorConfig<P>,
    db: Arc<Db>,
    semaphore: Arc<Semaphore>,
    result_tx: mpsc::Sender<BlockResult>,
    ack_client: WitnessServiceClient<Channel>,
    batch_done_tx: mpsc::Sender<(u64, bool)>,
    fallback_done_tx: mpsc::Sender<(u64, bool)>,
    checkpoint: u64,
    confirmed: BTreeSet<u64>,
    submitting_batches: HashSet<u64>,
    pending_requests: BTreeSet<u64>,
    highest_witness_received: u64,
    fallback_active: BTreeSet<u64>,
    fallback_exhausted: BTreeSet<u64>,
}

impl<P: Provider + Clone + 'static> StreamState<P> {
    /// Handle a gRPC message: either a reorg notification or a new witness.
    fn on_grpc_message(&mut self, msg: crate::proto::WitnessMessage, accumulator: &mut BatchAccumulator) {
        use crate::proto::witness_message::Content;
        match msg.content {
            Some(Content::Reorg(reorg)) => {
                let blocks = reorg.reverted_block_numbers;
                warn!(?blocks, "Reorg received — purging stale state");
                accumulator.handle_reorg(&blocks);
                if let Some(&min) = blocks.iter().min() {
                    let new_checkpoint = min.saturating_sub(1);
                    self.db.save_checkpoint(new_checkpoint);
                    self.checkpoint = new_checkpoint;
                    self.confirmed.retain(|&b| b <= new_checkpoint);
                    info!(new_checkpoint, "Checkpoint rolled back due to reorg");
                }
            }
            Some(Content::Witness(witness)) => {
                let block_number = witness.block_number;
                let raw_bytes = witness.data.len();

                let compressed = match compress_witness(&witness.data, block_number) {
                    Ok(data) => data,
                    Err(()) => return,
                };

                if compressed.len() > MAX_COMPRESSED_BYTES {
                    error!(
                        block_number,
                        compressed_bytes = compressed.len(),
                        limit = MAX_COMPRESSED_BYTES,
                        "Compressed witness exceeds limit — skipping"
                    );
                    return;
                }

                let compressed_bytes = compressed.len();
                info!(
                    block_number,
                    raw_bytes,
                    compressed_bytes,
                    ratio = format_args!("{:.1}x", raw_bytes as f64 / compressed_bytes.max(1) as f64),
                    "Dispatching witness"
                );

                self.pending_requests.insert(block_number);
                self.highest_witness_received = self.highest_witness_received.max(block_number);

                spawn_block_request(
                    self.config.http_client.clone(),
                    self.config.proxy_url.clone(),
                    self.config.api_key.clone(),
                    block_number,
                    compressed,
                    Arc::clone(&self.semaphore),
                    self.result_tx.clone(),
                );
            }
            None => {}
        }
    }

    /// Handle a completed block execution response: advance watermark, ACK, try dispatch.
    fn on_block_result(&mut self, result: BlockResult, accumulator: &mut BatchAccumulator) {
        let block_number = result.block_number;
        self.pending_requests.remove(&block_number);
        info!(block_number, "Block execution response received");
        accumulator.insert_response(result.response);

        // Advance contiguous watermark
        self.confirmed.insert(block_number);
        while self.confirmed.contains(&(self.checkpoint + 1)) {
            self.checkpoint += 1;
            self.confirmed.remove(&self.checkpoint);
        }
        self.db.save_checkpoint(self.checkpoint);

        // Cumulative ACK — fire-and-forget; a missed ACK is cleaned up by the next one.
        {
            let mut ack = self.ack_client.clone();
            let cp = self.checkpoint;
            tokio::spawn(async move {
                let _ = ack.acknowledge(crate::proto::AcknowledgeRequest { up_to_block: cp }).await;
            });
        }

        self.dispatch_batch_if_ready(accumulator);
    }

    /// Handle an L1 event: register a new batch or mark blobs accepted.
    fn on_l1_event(
        &mut self,
        event: L1Event,
        accumulator: &mut BatchAccumulator,
        next_batch_from_block: &mut Option<u64>,
        from_block: u64,
    ) {
        match event {
            L1Event::BatchHeaders { batch_index, expected_blobs: _, batch_root: _, num_blocks } => {
                let from = next_batch_from_block.unwrap_or(from_block);
                let to = from + num_blocks.saturating_sub(1);
                info!(batch_index, from, to, num_blocks, "Setting batch from L1 event");
                accumulator.set_batch(batch_index, from, to);
                *next_batch_from_block = Some(to + 1);
            }
            L1Event::BlobsAccepted { batch_index } => {
                accumulator.mark_blobs_accepted(batch_index);
                self.dispatch_batch_if_ready(accumulator);
            }
        }
    }

    /// Handle the result of a background batch submission task.
    fn on_batch_done(&mut self, batch_index: u64, success: bool, accumulator: &mut BatchAccumulator) {
        self.submitting_batches.remove(&batch_index);
        if success {
            if let Some(batch) = accumulator.take(batch_index) {
                self.db.save_last_batch_end(batch.to_block);
                info!(batch_index, to_block = batch.to_block, "Batch preconfirmed on L1");
            }
        } else {
            warn!(batch_index, "Batch submission failed — will retry on next event");
        }
    }

    /// Scan for witness gaps and spawn L3/L4 fallback tasks for any found.
    fn on_fallback_tick(&mut self) {
        if self.highest_witness_received < FALLBACK_GAP_M {
            return;
        }
        let gap_threshold = self.highest_witness_received - FALLBACK_GAP_M;

        let gaps: Vec<u64> = (self.checkpoint + 1..=gap_threshold)
            .filter(|b| {
                !self.confirmed.contains(b)
                    && !self.pending_requests.contains(b)
                    && !self.fallback_active.contains(b)
                    && !self.fallback_exhausted.contains(b)
            })
            .take(FALLBACK_BATCH_SIZE)
            .collect();

        for block_number in gaps {
            warn!(block_number, "Gap detected — spawning L3/L4 fallback");
            self.fallback_active.insert(block_number);

            let local_rpc = self.config.fallback_local_rpc.clone();
            let remote_rpc = self.config.fallback_remote_rpc.clone();
            let http = self.config.http_client.clone();
            let proxy = self.config.proxy_url.clone();
            let key = self.config.api_key.clone();
            let sem = Arc::clone(&self.semaphore);
            let rtx = self.result_tx.clone();
            let fdtx = self.fallback_done_tx.clone();

            tokio::spawn(async move {
                let ok = try_witness_fallback(
                    block_number, local_rpc, remote_rpc,
                    http, proxy, key, sem, rtx,
                ).await;
                let _ = fdtx.send((block_number, ok)).await;
            });
        }
    }

    /// Handle a fallback task completion: clear active set, propagate into pending or exhaust.
    fn on_fallback_done(&mut self, block_number: u64, success: bool) {
        self.fallback_active.remove(&block_number);
        if success {
            self.pending_requests.insert(block_number);
        } else {
            self.fallback_exhausted.insert(block_number);
            error!(
                block_number,
                "Witness fallback exhausted (L3 + L4 failed) — block permanently missing, batch will stall"
            );
        }
    }

    /// Dispatch at most one ready batch as a background task. No-op if one is already in flight.
    fn dispatch_batch_if_ready(&mut self, accumulator: &mut BatchAccumulator) {
        while let Some(batch_index) = accumulator.first_ready() {
            if self.submitting_batches.contains(&batch_index) {
                break; // already in flight — wait for batch_done_tx
            }
            let Some(batch) = accumulator.get(batch_index) else { break; };
            let (from_block, to_block) = (batch.from_block, batch.to_block);

            self.submitting_batches.insert(batch_index);
            let cfg = self.config.clone();
            let dtx = self.batch_done_tx.clone();
            tokio::spawn(async move {
                let ok = submit_batch_io(
                    &cfg.http_client, &cfg.proxy_url, &cfg.api_key,
                    &cfg.l1_provider, cfg.l1_contract_addr, cfg.nitro_verifier_addr,
                    batch_index, from_block, to_block,
                ).await;
                let _ = dtx.send((batch_index, ok)).await;
            });
            break; // one in flight at a time; next event re-checks
        }
    }
}

/// Single stream session: connect, build state, run select! loop.
#[allow(clippy::too_many_arguments)]
async fn run_stream<P: Provider + Clone + 'static>(
    config: &OrchestratorConfig<P>,
    db: &Arc<Db>,
    from_block: u64,
    mut confirmed: BTreeSet<u64>,  // pre-populated from DB on first call
    semaphore: &Arc<Semaphore>,
    result_tx: &mpsc::Sender<BlockResult>,
    result_rx: &mut mpsc::Receiver<BlockResult>,
    l1_events: &mut mpsc::Receiver<L1Event>,
    l1_ckpt_rx: &mut mpsc::Receiver<u64>,
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

    let ack_client = client.clone(); // cheap clone — shares the underlying Channel

    let mut stream = client.subscribe(SubscribeRequest { from_block }).await?.into_inner();
    info!(from_block, "Subscribed to witness stream");

    // Advance watermark through any already-loaded DB responses
    let mut checkpoint: u64 = from_block.saturating_sub(1);
    while confirmed.contains(&(checkpoint + 1)) {
        checkpoint += 1;
        confirmed.remove(&checkpoint);
    }

    let (batch_done_tx, mut batch_done_rx) = mpsc::channel::<(u64, bool)>(8);
    let (fallback_done_tx, mut fallback_done_rx) = mpsc::channel::<(u64, bool)>(32);
    let mut fallback_ticker = tokio::time::interval(Duration::from_secs(5));
    fallback_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut state = StreamState {
        config: config.clone(),
        db: Arc::clone(db),
        semaphore: Arc::clone(semaphore),
        result_tx: result_tx.clone(),
        ack_client,
        batch_done_tx,
        fallback_done_tx,
        checkpoint,
        confirmed,
        submitting_batches: HashSet::new(),
        pending_requests: BTreeSet::new(),
        highest_witness_received: from_block.saturating_sub(1),
        fallback_active: BTreeSet::new(),
        fallback_exhausted: BTreeSet::new(),
    };

    loop {
        tokio::select! {
            // ── Stream A: gRPC witness / reorg messages ────────────────
            msg = stream.message() => match msg? {
                None => { info!("Witness stream ended"); return Ok(()); }
                Some(msg) => state.on_grpc_message(msg, accumulator),
            },

            // ── Stream B: block execution responses ──────────────────
            Some(result) = result_rx.recv() =>
                state.on_block_result(result, accumulator),

            // ── Stream C: L1 events ──────────────────────────────────
            Some(event) = l1_events.recv() =>
                state.on_l1_event(event, accumulator, next_batch_from_block, from_block),

            // ── Stream D: batch submission completions ────────────────
            Some((batch_index, success)) = batch_done_rx.recv() =>
                state.on_batch_done(batch_index, success, accumulator),

            // ── Stream E: fallback gap checker ────────────────────────
            _ = fallback_ticker.tick() => state.on_fallback_tick(),

            // ── Stream F: fallback task completions ───────────────────
            Some((block_number, success)) = fallback_done_rx.recv() =>
                state.on_fallback_done(block_number, success),

            // ── Stream G: L1 listener checkpoint persistence ─────────────────
            Some(l1_block) = l1_ckpt_rx.recv() => {
                db.save_l1_checkpoint(l1_block);
            },
        }
    }
}

/// Execute network I/O for batch submission — no accumulator access.
/// Returns true if sign + preconfirm both succeeded.
async fn submit_batch_io<P: Provider + Clone + 'static>(
    http_client: &reqwest::Client,
    proxy_url: &str,
    api_key: &str,
    l1_provider: &P,
    l1_contract_addr: Address,
    nitro_verifier_addr: Address,
    batch_index: u64,
    from_block: u64,
    to_block: u64,
) -> bool {
    info!(batch_index, from_block, to_block, "Batch ready — triggering /sign-batch-root");

    let sign_result = call_sign_batch_root(
        http_client, proxy_url, api_key, from_block, to_block, batch_index,
    ).await;

    let batch_resp = match sign_result {
        Ok(resp) => resp,
        Err(e) => {
            error!(batch_index, err = %e, "Failed to sign batch root — will retry");
            return false;
        }
    };

    info!(batch_index, "Batch root signed — submitting preconfirmBatch");

    if let Err(e) = l1_submitter::submit_preconfirmation(
        l1_provider, l1_contract_addr, nitro_verifier_addr, batch_index, batch_resp.signature,
    ).await {
        error!(batch_index, err = %e, "preconfirmBatch failed — will retry");
        return false;
    }

    true
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
            Err(_) => return, // semaphore closed → shutdown
        };

        let mut backoff = Duration::from_millis(500);
        let mut attempt = 0u32;

        loop {
            attempt += 1;
            match send_block_request(&http_client, &proxy_url, &api_key, block_number, &compressed).await {
                Ok(response) => {
                    let _ = result_tx.send(BlockResult { block_number, response }).await;
                    return;
                }
                Err(e) => {
                    warn!(
                        block_number,
                        attempt,
                        err = %e,
                        backoff_ms = backoff.as_millis(),
                        "Block request failed — retrying"
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
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

type FallbackInput = ClientExecutorInput<<FluentEvmConfig as reth_evm::ConfigureEvm>::Primitives>;

/// Attempt to recover a missing witness via local RPC (L3) then remote archive (L4).
///
/// On success: serializes + compresses the witness and calls `spawn_block_request`,
/// which handles Nitro HTTP retries. Returns true if a request was dispatched.
/// On failure: returns false — caller logs and marks block exhausted.
async fn try_witness_fallback(
    block_number: u64,
    local_rpc: Option<String>,
    remote_rpc: Option<String>,
    http_client: reqwest::Client,
    proxy_url: String,
    api_key: String,
    semaphore: Arc<Semaphore>,
    result_tx: mpsc::Sender<BlockResult>,
) -> bool {
    let genesis = Genesis::Fluent;
    let chain_spec = match reth_chainspec::ChainSpec::try_from(&genesis) {
        Ok(cs) => std::sync::Arc::new(cs),
        Err(e) => {
            error!(block_number, err = %e, "L3/L4: failed to build ChainSpec");
            return false;
        }
    };
    let evm_config = rsp_host_executor::create_eth_block_execution_strategy_factory(&genesis, None);
    let executor = HostExecutor::new(evm_config, chain_spec);

    // L3 — local Reth JSON-RPC
    if let Some(url_str) = &local_rpc {
        match url::Url::parse(url_str) {
            Ok(url) => {
                let provider = create_provider::<Ethereum>(url);
                match executor.execute(block_number, &provider, genesis.clone(), None, false).await {
                    Ok(input) => {
                        info!(block_number, "L3 fallback succeeded");
                        return dispatch_fallback_witness(
                            block_number, &input,
                            http_client, proxy_url, api_key, semaphore, result_tx,
                        );
                    }
                    Err(e) => warn!(block_number, err = %e, "L3 fallback failed — trying L4"),
                }
            }
            Err(e) => warn!(block_number, err = %e, "L3: invalid fallback_local_rpc URL"),
        }
    }

    // L4 — remote archive RPC
    if let Some(url_str) = &remote_rpc {
        match url::Url::parse(url_str) {
            Ok(url) => {
                let provider = create_provider::<Ethereum>(url);
                match executor.execute(block_number, &provider, genesis.clone(), None, false).await {
                    Ok(input) => {
                        info!(block_number, "L4 fallback succeeded");
                        return dispatch_fallback_witness(
                            block_number, &input,
                            http_client, proxy_url, api_key, semaphore, result_tx,
                        );
                    }
                    Err(e) => warn!(block_number, err = %e, "L4 fallback failed"),
                }
            }
            Err(e) => warn!(block_number, err = %e, "L4: invalid fallback_remote_rpc URL"),
        }
    }

    false
}

/// Serialize, compress, and dispatch a recovered witness into the normal signing pipeline.
fn dispatch_fallback_witness(
    block_number: u64,
    input: &FallbackInput,
    http_client: reqwest::Client,
    proxy_url: String,
    api_key: String,
    semaphore: Arc<Semaphore>,
    result_tx: mpsc::Sender<BlockResult>,
) -> bool {
    let raw = match bincode::serialize(input) {
        Ok(b) => b,
        Err(e) => {
            error!(block_number, err = %e, "Fallback: bincode serialization failed");
            return false;
        }
    };
    let compressed = match compress_witness(&raw, block_number) {
        Ok(c) => c,
        Err(()) => return false,
    };
    spawn_block_request(http_client, proxy_url, api_key, block_number, compressed, semaphore, result_tx);
    true
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


