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

/// Events sent through the hub broadcast channel.
#[derive(Debug, Clone)]
pub enum HubEvent {
    /// A new witness is available.
    Witness(SharedProveRequest),
    /// Blocks were reverted due to a chain reorg.
    Reorg { reverted_blocks: Vec<u64> },
}

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
    pub parent_hash: B256,
    pub block_hash: B256,
    pub withdrawal_hash: B256,
    pub deposit_hash: B256,
    pub tx_data_hash: B256,
    pub result_hash: Vec<u8>,
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