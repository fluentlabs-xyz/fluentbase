//! Read errors.
//!
//! The variants distinguish *empty result* from *revert* from *malformed
//! data* because the contract surface mixes these: `getConsensusKeys`
//! returns a zeroed struct for an unknown validator (→ `Ok(None)`, not an
//! error) and `getEpochCommittee` returns `[]` for an uncommitted epoch (→
//! `Ok(vec![])`), whereas a malformed key blob is a hard `BlsKey`/`PeerKey`
//! error and a genuine EVM revert surfaces as `CallReverted`.

use alloy_primitives::B256;

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("state/header for block {0} not found")]
    BlockNotFound(B256),

    #[error("evm read call reverted: {0}")]
    CallReverted(String),

    #[error("abi decode failed: {0}")]
    AbiDecode(String),

    #[error("bls pubkey decode / subgroup check failed: {0}")]
    BlsKey(String),

    #[error("peer pubkey is not a valid 32-byte ed25519 key")]
    PeerKey,

    #[error("epoch {epoch} committee member {validator} has no consensus keys (on-chain invariant violated)")]
    CommitteeMemberKeyless {
        epoch: u64,
        validator: alloy_primitives::Address,
    },

    #[error("epoch {epoch} tracker peer-set size {size} (registry ∪ committee) exceeds configured max_peer_set_size {max} (misconfig / governance drift)")]
    PeerSetTooLarge { epoch: u64, size: usize, max: usize },

    #[error("provider/evm backend error: {0}")]
    Backend(String),

    #[error("ChainConfig.getEpochBlockInterval() returned 0 — epoch division undefined")]
    ZeroEpochInterval,
}
