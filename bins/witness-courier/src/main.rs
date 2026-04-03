//! Witness courier sidecar binary.
//!
//! Connects to the Fluent node's gRPC witness server (localhost), receives
//! block witnesses, and forwards raw bincode to the remote proxy via HTTP POST.
//! Orchestrates batch signing and L1 preconfirmation.
//!
//! ## Data flow
//!
//! ```text
//! gRPC server ──raw bincode──▶ courier ──HTTP POST──▶ proxy ──▶ Nitro
//!   (localhost)                  (this binary)          (remote)
//! ```
//!
//! ## Orchestration
//!
//! ```text
//! gRPC witnesses ──▶ parallel /sign-block-execution ──▶ BatchAccumulator
//! L1 events ──────▶ BatchHeadersSubmitted / BatchAccepted ──▶ BatchAccumulator
//!                              │
//!                    ALL conditions met
//!                              ▼
//!               /sign-batch-root ──▶ preconfirmBatch (L1)
//! ```
//!
//! # Configuration (environment variables)
//!
//! | Variable | Default | Description |
//! |----------|---------|-------------|
//! | `FLUENT_WITNESS_ADDR` | `http://127.0.0.1:10000` | gRPC server address (local) |
//! | `FLUENT_PROXY_URL` | `http://127.0.0.1:11000/sign-block-execution` | Remote proxy endpoint |
//! | `FLUENT_DB_PATH` | `./witness_courier.db` | SQLite DB for crash recovery |
//! | `FLUENT_HTTP_TIMEOUT_SECS` | `120` | HTTP POST timeout (seconds) |
//! | `L1_RPC_URL` | — | L1 Ethereum RPC URL (required for batch orchestration) |
//! | `L1_CONTRACT_ADDR` | — | Rollup contract address on L1 |
//! | `L1_SUBMITTER_KEY` | — | Private key for signing `preconfirmBatch` txs |
//! | `NITRO_VERIFIER_ADDR` | — | NitroVerifier contract address on L1 |
//! | `L1_START_BLOCK` | `0` | L1 block to start listening for events |
//! | `FLUENT_START_BATCH_ID`   | —  | If set (and no checkpoint in DB), scan L1 to derive L2 start checkpoint |
//! | `FLUENT_L1_DEPLOY_BLOCK`  | `0` | L1 block where Rollup contract was deployed — lower bound for startup scan |

use std::path::PathBuf;
use std::time::Duration;

use alloy_primitives::Address;
use alloy_provider::{ProviderBuilder, RootProvider};
use alloy_network::EthereumWallet;
use alloy_signer_local::PrivateKeySigner;
use tracing::info;

use witness_courier::client::{self, OrchestratorConfig};
use witness_courier::l1_listener;

const DEFAULT_SERVER_ADDR: &str = "http://127.0.0.1:10000";
const DEFAULT_PROXY_URL: &str = "http://127.0.0.1:8080/sign-block-execution";
const DEFAULT_DB_PATH: &str = "./witness_courier.db";
const DEFAULT_HTTP_TIMEOUT_SECS: u64 = 120;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let server_addr =
        std::env::var("FLUENT_WITNESS_ADDR").unwrap_or_else(|_| DEFAULT_SERVER_ADDR.into());
    let proxy_url =
        std::env::var("FLUENT_PROXY_URL").unwrap_or_else(|_| DEFAULT_PROXY_URL.into());
    let db_path = PathBuf::from(
        std::env::var("FLUENT_DB_PATH").unwrap_or_else(|_| DEFAULT_DB_PATH.into()),
    );
    let http_timeout_secs: u64 = std::env::var("FLUENT_HTTP_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_HTTP_TIMEOUT_SECS);

    // L1 configuration
    let l1_rpc_url = std::env::var("L1_RPC_URL").expect("L1_RPC_URL is required");
    let l1_contract_addr: Address = std::env::var("L1_CONTRACT_ADDR")
        .expect("L1_CONTRACT_ADDR is required")
        .parse()
        .expect("Invalid L1_CONTRACT_ADDR");
    let l1_submitter_key = std::env::var("L1_SUBMITTER_KEY").expect("L1_SUBMITTER_KEY is required");
    let nitro_verifier_addr: Address = std::env::var("NITRO_VERIFIER_ADDR")
        .expect("NITRO_VERIFIER_ADDR is required")
        .parse()
        .expect("Invalid NITRO_VERIFIER_ADDR");
    let l1_start_block: u64 = std::env::var("L1_START_BLOCK")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let start_batch_id: Option<u64> = std::env::var("FLUENT_START_BATCH_ID")
        .ok()
        .and_then(|s| s.parse().ok());
    let l1_deploy_block: u64 = std::env::var("FLUENT_L1_DEPLOY_BLOCK")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let api_key = std::env::var("FLUENT_API_KEY").unwrap_or_default();
    let fallback_local_rpc = std::env::var("FLUENT_FALLBACK_LOCAL_RPC").ok();
    let fallback_remote_rpc = std::env::var("FLUENT_FALLBACK_REMOTE_RPC").ok();
    let max_concurrent_fallbacks: usize = std::env::var("FLUENT_MAX_CONCURRENT_FALLBACKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);

    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(http_timeout_secs))
        .pool_max_idle_per_host(2)
        .build()
        .expect("failed to build HTTP client");

    // Build L1 provider for reading (events) — no fillers needed
    let l1_rpc_url_parsed: url::Url = l1_rpc_url.parse().expect("Invalid L1_RPC_URL");
    let l1_read_provider = RootProvider::new_http(l1_rpc_url_parsed.clone());

    // ── Startup: resolve L2 checkpoint from START_BATCH_ID ───────────────────────
    let listener_from_block: u64 = {
        let db_startup = witness_courier::db::Db::open(&db_path)
            .expect("Failed to open DB for startup");

        if let Some(batch_id) = start_batch_id {
            if db_startup.get_checkpoint() == 0 {
                let l2_rpc_url: url::Url = fallback_local_rpc
                    .as_deref()
                    .expect("FLUENT_FALLBACK_LOCAL_RPC is required when FLUENT_START_BATCH_ID is set")
                    .parse()
                    .expect("Invalid FLUENT_FALLBACK_LOCAL_RPC URL");
                let l2_provider = RootProvider::new_http(l2_rpc_url);

                info!(batch_id, "FLUENT_START_BATCH_ID set — resolving L2 start checkpoint from L1");
                let (l2_from_block, l1_event_block) = l1_listener::resolve_l2_start_checkpoint(
                    &l1_read_provider,
                    &l2_provider,
                    l1_contract_addr,
                    batch_id,
                    l1_deploy_block,
                )
                .await
                .expect("Fatal: failed to resolve L2 start checkpoint from L1");

                let l2_checkpoint = l2_from_block.saturating_sub(1);
                db_startup.save_checkpoint(l2_checkpoint);
                // Save (l1_event_block - 1) so listener resumes FROM l1_event_block
                db_startup.save_l1_checkpoint(l1_event_block.saturating_sub(1));

                info!(
                    batch_id,
                    l2_from_block,
                    l2_checkpoint,
                    l1_event_block,
                    "L2 start checkpoint resolved and saved to DB"
                );
            } else {
                info!(
                    batch_id,
                    checkpoint = db_startup.get_checkpoint(),
                    "L2 checkpoint already in DB — skipping startup scan"
                );
            }
        }

        // Compute L1 listener start from (possibly just updated) checkpoint.
        let lfb = if let Some(ckpt) = db_startup.get_l1_checkpoint() {
            (ckpt + 1).max(l1_start_block)
        } else {
            l1_start_block
        };

        drop(db_startup);
        lfb
    };

    info!(
        %server_addr,
        %proxy_url,
        ?db_path,
        http_timeout_secs,
        %l1_contract_addr,
        %nitro_verifier_addr,
        l1_start_block,
        listener_from_block,
        start_batch_id,
        l1_deploy_block,
        fallback_local_rpc = ?fallback_local_rpc,
        fallback_remote_rpc = ?fallback_remote_rpc,
        "Starting witness courier"
    );

    // Build L1 provider for writing (preconfirmBatch)
    let signer: PrivateKeySigner = l1_submitter_key
        .parse()
        .expect("Invalid L1_SUBMITTER_KEY");
    let wallet = EthereumWallet::from(signer);
    let l1_write_provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(l1_rpc_url_parsed);

    // Start L1 event listener
    let (l1_tx, l1_rx) = tokio::sync::mpsc::channel(64);
    let (l1_ckpt_tx, l1_ckpt_rx) = tokio::sync::mpsc::channel::<u64>(32);
    tokio::spawn(l1_listener::run(
        l1_read_provider,
        l1_contract_addr,
        listener_from_block,
        l1_tx,
        l1_ckpt_tx,
    ));

    // Run orchestrator
    let config = OrchestratorConfig {
        server_addr,
        proxy_url,
        db_path,
        http_client,
        l1_contract_addr,
        nitro_verifier_addr,
        l1_provider: l1_write_provider,
        api_key,
        fallback_local_rpc,
        fallback_remote_rpc,
        max_concurrent_fallbacks,
    };

    client::run(config, l1_rx, l1_ckpt_rx).await;
}

