//! Shared domain types used by both the node (server) and the courier (client).

use std::sync::Arc;

/// A witness payload ready to be sent to the proving backend.
///
/// `payload` contains a bincode-serialized `ClientExecutorInput<FluentPrimitives>`.
/// The courier forwards it as-is — no deserialization needed on the transport layer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProveRequest {
    /// L2 block number this witness corresponds to.
    pub block_number: u64,
    /// Bincode-serialized witness data.
    pub payload: Vec<u8>,
}

/// Arc-wrapped prove request for cheap cloning across broadcast subscribers.
pub type SharedProveRequest = Arc<ProveRequest>;

// ---------------------------------------------------------------------------
// Mirror types from `fluent-nitro-types` (different workspace)
// ---------------------------------------------------------------------------

use alloy_primitives::B256;

/// Per-block execution response from the Nitro enclave.
///
/// Mirror of `nitro_types::EthExecutionResponse` — kept in sync manually.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EthExecutionResponse {
    pub block_number: u64,
    pub leaf: [u8; 32],
    pub tx_data_hash: B256,
    pub signature: Vec<u8>,
}

/// Batch signing response from the Nitro enclave.
///
/// Mirror of `nitro_types::SubmitBatchResponse` — kept in sync manually.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubmitBatchResponse {
    pub batch_root: Vec<u8>,
    pub versioned_hashes: Vec<B256>,
    pub signature: Vec<u8>,
}

/// Response from proxy when batch signing fails due to enclave key rotation.
///
/// Mirror of proxy's `InvalidSignaturesResponse`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InvalidSignaturesResponse {
    pub invalid_blocks: Vec<u64>,
    pub enclave_address: Address,
}

use alloy_primitives::Address;