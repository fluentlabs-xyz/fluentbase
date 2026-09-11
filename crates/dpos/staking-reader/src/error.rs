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

    /// The committee the contract returned is not strictly ascending on the raw
    /// peer-pubkey bytes. That order IS the consensus index space: the node writes a
    /// commonware `Participant` index into the block and the contract resolves the
    /// accused positionally from its own array, so a divergence slashes the wrong
    /// validator in silence. Fail the read instead.
    #[error("epoch {epoch} committee is not ascending on peer pubkey at position {position} (member {validator}) — contract/consensus index-space divergence")]
    CommitteeOutOfOrder {
        epoch: u64,
        position: usize,
        validator: alloy_primitives::Address,
    },

    /// Two committee members share a peer pubkey. The contract enforces uniqueness
    /// (`ERR_PEER_PUBKEY_ALREADY_IN_USE`) and relies on it for its unstable sort to be
    /// deterministic; a duplicate here means that enforcement broke, and the
    /// positional index the node writes would be ambiguous.
    #[error("epoch {epoch} committee has a duplicate peer pubkey at position {position} (member {validator}) — on-chain uniqueness invariant violated")]
    CommitteeDuplicatePeerKey {
        epoch: u64,
        position: usize,
        validator: alloy_primitives::Address,
    },

    /// A committed committee below the contract's own floor. `commitEpochCommittee`
    /// reverts under it, so a non-empty short committee cannot be a legal on-chain
    /// state (an *empty* one can — an uncommitted epoch — and is not an error).
    #[error("epoch {epoch} committee size {size} is below MIN_COMMITTEE_LENGTH {min}")]
    CommitteeTooSmall { epoch: u64, size: usize, min: usize },

    #[error("provider/evm backend error: {0}")]
    Backend(String),

    #[error("ChainConfig.getEpochBlockInterval() returned 0 — epoch division undefined")]
    ZeroEpochInterval,
}

impl ReadError {
    /// Whether re-issuing the SAME read against the SAME block can legitimately
    /// answer differently later.
    ///
    /// Exactly three variants say yes, and all three are "reth has not put the
    /// bytes where this read looks *yet*", not "the chain says so":
    ///
    /// * [`Self::StateNotMaterialized`] — the header exists, the executed state
    ///   does not; a pipeline backfill or a re-execution unwind re-materializes
    ///   it on its own.
    /// * [`Self::TransientStorage`] — a torn static-file read observed while the
    ///   persistence thread appends a segment; the append settles sub-second.
    /// * [`Self::BlockNotFound`] — no header at that hash on this node yet; the
    ///   block can still arrive.
    ///
    /// Everything else is permanent BY CLASS, not by exhaustion: a revert, a
    /// decode failure, a committee that is out of order / duplicated / below the
    /// floor, a keyless member, an oversized peer set, a zero epoch interval and
    /// a [`Self::Backend`] fault are all statements the chain (or this node's
    /// storage) will repeat verbatim on every retry. A consumer that treats one
    /// of them as transient turns a fail-loud contract disagreement into an
    /// invisible retry loop, which is why this is a predicate on the error type
    /// rather than a judgement each call site re-derives.
    ///
    /// Deliberately written as an exhaustive `match` rather than a `matches!`
    /// with a `_` arm: a new variant must be classified here, not silently
    /// inherit "permanent".
    pub fn is_transient(&self) -> bool {
        match self {
            Self::StateNotMaterialized { .. }
            | Self::TransientStorage(_)
            | Self::BlockNotFound(_) => true,
            Self::CallReverted(_)
            | Self::AbiDecode(_)
            | Self::BlsKey(_)
            | Self::PeerKey
            | Self::CommitteeMemberKeyless { .. }
            | Self::PeerSetTooLarge { .. }
            | Self::CommitteeOutOfOrder { .. }
            | Self::CommitteeDuplicatePeerKey { .. }
            | Self::CommitteeTooSmall { .. }
            | Self::Backend(_)
            | Self::ZeroEpochInterval => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Address;

    #[test]
    fn only_the_three_not_here_yet_reads_are_transient() {
        // One variant per class, both ways round: the three "reth has not put
        // the bytes there yet" shapes retry, and one representative of every
        // permanent class (revert, decode, key, on-chain invariant, config,
        // backend) does not.
        for transient in [
            ReadError::StateNotMaterialized { hash: B256::ZERO },
            ReadError::TransientStorage(TORN_RANGE_DISPLAY.into()),
            ReadError::BlockNotFound(B256::ZERO),
        ] {
            assert!(transient.is_transient(), "{transient} must be retryable");
        }

        for permanent in [
            ReadError::CallReverted("execution reverted".into()),
            ReadError::AbiDecode("committee/stakes length mismatch".into()),
            ReadError::BlsKey("subgroup check".into()),
            ReadError::PeerKey,
            ReadError::CommitteeMemberKeyless {
                epoch: 7,
                validator: Address::ZERO,
            },
            ReadError::PeerSetTooLarge {
                epoch: 7,
                size: 99,
                max: 51,
            },
            ReadError::CommitteeOutOfOrder {
                epoch: 7,
                position: 2,
                validator: Address::ZERO,
            },
            ReadError::CommitteeDuplicatePeerKey {
                epoch: 7,
                position: 2,
                validator: Address::ZERO,
            },
            ReadError::CommitteeTooSmall {
                epoch: 7,
                size: 3,
                min: 4,
            },
            ReadError::Backend("no state found".into()),
            ReadError::ZeroEpochInterval,
        ] {
            assert!(
                !permanent.is_transient(),
                "{permanent} must NOT be retryable"
            );
        }
    }
}
