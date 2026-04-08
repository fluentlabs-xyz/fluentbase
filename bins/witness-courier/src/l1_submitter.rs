//! L1 transaction submitter for `preconfirmBatch`.
//!
//! After the enclave signs a batch root, the courier submits
//! `preconfirmBatch(nitroVerifier, batchIndex, signature)` to the L1 rollup
//! contract. This transitions the batch from `Accepted` → `Preconfirmed`.

use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use alloy_sol_types::{sol, SolCall, SolError};
use alloy_json_rpc::RpcError;
use eyre::{eyre, Result};
use tracing::info;

/// Metadata from a confirmed L1 transaction, used for finalization tracking.
pub struct SubmitReceipt {
    pub tx_hash: B256,
    pub l1_block: u64,
}

sol! {
    /// Submit enclave signature to L1, proving batch validity.
    function preconfirmBatch(
        address nitroVerifier,
        uint256 batchIndex,
        bytes signature
    ) external;

    /// NitroVerifier view function (auto-generated getter for mapping).
    function verifiedPubkeys(address) external view returns (bool);

    /// Rollup contract error: batch is not in the expected status.
    error InvalidBatchStatus(uint256 batchIndex, uint8 status);
}

/// Result of a preconfirmation submission.
pub enum SubmitOutcome {
    /// TX confirmed on L1.
    Submitted(SubmitReceipt),
    /// Batch already progressed past Accepted (status >= 3).
    AlreadyPreconfirmed { batch_index: u64, status: u8 },
}

/// Check if an enclave address is registered in NitroVerifier.
pub async fn is_key_registered(
    provider: &impl Provider,
    nitro_verifier_addr: Address,
    enclave_address: Address,
) -> Result<bool> {
    let call = verifiedPubkeysCall(enclave_address);
    let tx = TransactionRequest {
        to: Some(nitro_verifier_addr.into()),
        input: Bytes::from(call.abi_encode()).into(),
        ..Default::default()
    };
    let result = provider.call(tx).await
        .map_err(|e| eyre!("verifiedPubkeys call failed: {e}"))?;
    let registered = verifiedPubkeysCall::abi_decode_returns(&result)
        .map_err(|e| eyre!("Failed to decode verifiedPubkeys result: {e}"))?;
    Ok(registered)
}

/// Submit `preconfirmBatch` to L1 and wait for the receipt.
///
/// Returns [`SubmitOutcome::AlreadyPreconfirmed`] if the contract reverts with
/// `InvalidBatchStatus` and the on-chain status is >= Preconfirmed (3).
/// This handles the case where a previous attempt succeeded on-chain despite
/// returning an RPC error.
pub async fn submit_preconfirmation(
    provider: &impl Provider,
    contract_addr: Address,
    nitro_verifier_addr: Address,
    batch_index: u64,
    signature: Vec<u8>,
) -> Result<SubmitOutcome> {
    let call = preconfirmBatchCall {
        nitroVerifier: nitro_verifier_addr,
        batchIndex: U256::from(batch_index),
        signature: Bytes::from(signature),
    };

    let tx = TransactionRequest {
        to: Some(contract_addr.into()),
        input: Bytes::from(call.abi_encode()).into(),
        ..Default::default()
    };

    let pending = match provider.send_transaction(tx).await {
        Ok(p) => p,
        Err(ref e) => {
            if let Some(outcome) = check_already_preconfirmed(e, batch_index) {
                return Ok(outcome);
            }
            return Err(eyre!("preconfirmBatch tx send failed: {e}"));
        }
    };

    let tx_hash = *pending.tx_hash();
    info!(%tx_hash, batch_index, "preconfirmBatch tx sent");

    let receipt = pending
        .get_receipt()
        .await
        .map_err(|e| eyre!("preconfirmBatch receipt failed: {e}"))?;

    if !receipt.status() {
        return Err(eyre!(
            "preconfirmBatch reverted (tx {tx_hash}, batch {batch_index})"
        ));
    }

    let l1_block = receipt.block_number
        .ok_or_else(|| eyre!("receipt missing block_number (tx {tx_hash})"))?;

    info!(
        %tx_hash,
        batch_index,
        l1_block,
        gas_used = receipt.gas_used,
        "preconfirmBatch confirmed"
    );

    Ok(SubmitOutcome::Submitted(SubmitReceipt { tx_hash, l1_block }))
}

/// Check if an RPC error indicates the batch is already preconfirmed on-chain.
/// BatchStatus: None=0, HeadersSubmitted=1, Accepted=2, Preconfirmed=3, Challenged=4, Finalized=5
///
/// Parses revert data directly from the error payload without relying on
/// the message containing "revert" (which `as_decoded_error` requires).
fn check_already_preconfirmed<E>(err: &RpcError<E, Box<serde_json::value::RawValue>>, batch_index: u64) -> Option<SubmitOutcome> {
    let RpcError::ErrorResp(payload) = err else { return None };
    let data = payload.try_data_as::<String>().and_then(Result::ok)?;
    let bytes = data.parse::<Bytes>().ok()?;
    let decoded = InvalidBatchStatus::abi_decode(&bytes).ok()?;
    if decoded.status >= 3 {
        return Some(SubmitOutcome::AlreadyPreconfirmed {
            batch_index,
            status: decoded.status,
        });
    }
    None
}
