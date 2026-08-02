//! Read errors.
//!
//! The variants distinguish *empty result* from *revert* from *malformed
//! data* because the contract surface mixes these: `getConsensusKeys`
//! returns a zeroed struct for an unknown validator (→ `Ok(None)`, not an
//! error) and `getEpochCommittee` returns `[]` for an uncommitted epoch (→
//! `Ok(vec![])`), whereas a malformed key blob is a hard `BlsKey`/`PeerKey`
//! error and a genuine EVM revert surfaces as `CallReverted`.

use alloy_primitives::B256;

/// Stable display of reth's `DatabaseError::Decode` (`storage/errors/src/db.rs`)
/// — the type the reth-fork torn-changeset guard converts the former
/// `StorageBeforeTx::from_compact` panic into. The display fallback for boundary
/// wrappers that stringify the source chain rather than preserving the typed error.
///
/// These four `*_DISPLAY` constants are the SINGLE source of truth for the
/// torn-static-file-read display shapes, shared between this crate's read-boundary
/// classification (`reader::map_state_provider_err` / the EVM-call err mapper) and
/// the node-side whole-derive retry belt (`crates/node/src/derive.rs`
/// `is_transient_torn_static_file_read`, which re-imports them). Family-5 invariant:
/// string matching lives HERE, in the error-owning layer — never in consensus code.
pub const DECODE_ERROR_DISPLAY: &str = "failed to decode a key from a table";

/// Stable display prefix of `NippyJarError::InconsistentData` (`nippy-jar/src/error.rs`:
/// "attempted to read an inconsistent data range {range:?}, data size: {len}") — a
/// segment being concurrently appended surfaces as `ProviderError::Other(AnyError)`
/// carrying this display (`AnyError::source()` skips the wrapped error as a chain
/// link, so it is matched by display, not typed downcast).
pub const TORN_RANGE_DISPLAY: &str = "attempted to read an inconsistent data range";

/// Stable `Display` of a `std::io::Error` with `ErrorKind::UnexpectedEof`
/// ("failed to fill whole buffer") — a static-file sidecar read (`read_exact_at`)
/// that landed mid-append of a still-growing offsets/data file.
pub const SHORT_READ_DISPLAY: &str = "failed to fill whole buffer";

/// Stable display prefix of `NippyJarError::InconsistentSnapshot` ("inconsistent
/// static-file snapshot: {reason}") — a committed-snapshot validation refusal
/// (active-writer self-load / `LoadedJar` reload-once failure), transient across
/// the mark→publish / mid-heal window.
pub const SNAPSHOT_DISPLAY: &str = "inconsistent static-file snapshot";

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("state/header for block {0} not found")]
    BlockNotFound(B256),

    /// The block's header exists but its EXECUTED state is not materialized at
    /// `hash` — reth's pipeline backfill writes headers ahead of state and, during
    /// a re-execution unwind, transiently removes materialized state even at/below
    /// the finalized hash. This is a TRANSIENT condition (the block re-materializes
    /// on its own), distinct from `BlockNotFound` (no header at all) and from a
    /// genuine `Backend` fault: a consumer DEFERS + retries rather than failing
    /// closed. Classified from reth's typed `ProviderError::StateForHashNotFound`
    /// at the read boundary (`reader.rs`) so consumers match a variant, never a
    /// backend error string.
    #[error("state for block {hash} is not materialized yet (reth pipeline backfill)")]
    StateNotMaterialized { hash: B256 },

    /// A TRANSIENT reth storage read that is neither a clean state-miss nor genuine
    /// on-disk corruption: a torn STATIC-FILE read observed while the persistence
    /// thread appends/freezes a segment concurrently with the read (a torn changeset
    /// row `DatabaseError::Decode`, a `NippyJar` inconsistent-range / inconsistent-
    /// snapshot read, or a short `read_exact_at`). Classified from the typed reth
    /// error at the read boundary (`reader.rs`) — the four shapes mirror
    /// `is_transient_torn_static_file_read` (see the `*_DISPLAY` constants above).
    /// Like [`Self::StateNotMaterialized`] it returns NO committee, so a consumer
    /// DEFERS + retries rather than failing closed (the append settles sub-second;
    /// genuine corruption re-raises every attempt and exhausts the caller's bounded
    /// retry).
    #[error("transient reth static-file read (torn/mid-append): {0}")]
    TransientStorage(String),

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
