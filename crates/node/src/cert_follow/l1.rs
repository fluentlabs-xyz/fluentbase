//! L1 Rollup checkpoint source (D2): the follower's trust root.
//!
//! Reads `getBatch(lastFinalizedBatchIndex()).toBlockHash` — the hash of the
//! last L2 block in the last L1-FINALIZED batch (challenge window passed) —
//! via raw `eth_call` at the L1 `finalized` tag. `BatchRecord` carries no
//! absolute L2 height; the follower resolves the hash against its own synced
//! chain (`provider.block_number(hash)`), so the assert is hash-existence,
//! not height-equality.

use alloy_primitives::{Address, Bytes, B256};
use alloy_sol_types::{sol, SolCall as _};
use eyre::{ensure, eyre, WrapErr as _};
use jsonrpsee::{core::client::ClientT as _, http_client::HttpClientBuilder, rpc_params};
use tracing::info;

sol! {
    /// Mirrors `IRollupTypes.BatchRecord` (solidity-contracts
    /// interfaces/rollup/IRollupTypes.sol) — field ORDER is the ABI tuple
    /// layout and must match exactly.
    struct BatchRecord {
        bytes32 batchRoot;
        uint32 acceptedAtBlock;
        uint8 expectedBlobs;
        uint8 status;
        uint64 sentMessageCursorStart;
        uint24 submitBlobsWindowSnapshot;
        uint24 preconfirmationWindowSnapshot;
        uint24 challengeWindowSnapshot;
        uint24 finalizationDelaySnapshot;
        uint24 numberOfBlocks;
        bytes32 toBlockHash;
    }

    function lastFinalizedBatchIndex() view returns (uint256);
    function getBatch(uint256 batchIndex) view returns (BatchRecord memory);
}

/// L1 checkpoint deployment facts (`--cert-follow.l1-rpc-url` +
/// `--cert-follow.l1-rollup-address`).
#[derive(Clone, Debug)]
pub struct L1CheckpointConfig {
    pub rpc_url: String,
    pub rollup_address: Address,
}

/// Fetch the L1-finalized checkpoint hash. Fail-loud on any transport or
/// decode error — a follower configured with an L1 trust root must not
/// silently degrade to the trust-blind devnet path.
pub async fn fetch_checkpoint_hash(cfg: &L1CheckpointConfig) -> eyre::Result<B256> {
    let client = HttpClientBuilder::default()
        .build(&cfg.rpc_url)
        .wrap_err_with(|| format!("building L1 HTTP client for {}", cfg.rpc_url))?;

    let index_ret = eth_call(
        &client,
        cfg.rollup_address,
        lastFinalizedBatchIndexCall {}.abi_encode(),
    )
    .await
    .wrap_err("Rollup.lastFinalizedBatchIndex() eth_call failed")?;
    let index = lastFinalizedBatchIndexCall::abi_decode_returns(&index_ret)
        .wrap_err("decoding lastFinalizedBatchIndex() return")?;

    let batch_ret = eth_call(
        &client,
        cfg.rollup_address,
        getBatchCall { batchIndex: index }.abi_encode(),
    )
    .await
    .wrap_err("Rollup.getBatch(lastFinalized) eth_call failed")?;
    let batch =
        getBatchCall::abi_decode_returns(&batch_ret).wrap_err("decoding getBatch() return")?;

    ensure!(
        batch.toBlockHash != B256::ZERO,
        "Rollup.getBatch({index}).toBlockHash is ZERO — wrong rollup address or \
         uninitialized contract at {}",
        cfg.rollup_address
    );
    info!(
        batch_index = %index,
        hash = ?batch.toBlockHash,
        "cert-follow: L1 Rollup checkpoint fetched"
    );
    Ok(batch.toBlockHash)
}

/// Raw `eth_call` at the `finalized` tag (the checkpoint must itself be
/// L1-finalized — reading at `latest` would trust an L1 reorg window).
async fn eth_call(
    client: &jsonrpsee::http_client::HttpClient,
    to: Address,
    data: Vec<u8>,
) -> eyre::Result<Bytes> {
    let call = serde_json::json!({ "to": to, "data": Bytes::from(data) });
    let ret: Bytes = client
        .request("eth_call", rpc_params![call, "finalized"])
        .await
        .map_err(|e| eyre!("eth_call: {e}"))?;
    Ok(ret)
}
