//! L1 transaction submitter for `preconfirmBatch`.
//!
//! After the enclave signs a batch root, the courier submits
//! `preconfirmBatch(nitroVerifier, batchIndex, signature)` to the L1 rollup
//! contract. This transitions the batch from `Accepted` → `Preconfirmed`.

use alloy_primitives::{Address, Bytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use alloy_sol_types::{sol, SolCall};
use eyre::{eyre, Result};
use tracing::info;

sol! {
    /// Submit enclave signature to L1, proving batch validity.
    function preconfirmBatch(
        address nitroVerifier,
        uint256 batchIndex,
        bytes signature
    ) external;

    /// NitroVerifier view function (auto-generated getter for mapping).
    function verifiedPubkeys(address) external view returns (bool);
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
pub async fn submit_preconfirmation(
    provider: &impl Provider,
    contract_addr: Address,
    nitro_verifier_addr: Address,
    batch_index: u64,
    signature: Vec<u8>,
) -> Result<()> {
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

    let pending = provider
        .send_transaction(tx)
        .await
        .map_err(|e| eyre!("preconfirmBatch tx send failed: {e}"))?;

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

    info!(
        %tx_hash,
        batch_index,
        gas_used = receipt.gas_used,
        "preconfirmBatch confirmed"
    );

    Ok(())
}
