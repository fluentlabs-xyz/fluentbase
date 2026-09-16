//! Read errors.
//!
//! An uncommitted epoch is not an error: it reads back as an empty validator set.

use alloy_primitives::B256;

/// reth's `DatabaseError::Decode` display, matched as text where a boundary
/// wrapper stringified the source chain; must stay verbatim. It is re-imported by
/// `crates/node/src/derive.rs`'s `is_transient_torn_static_file_read`.
pub const DECODE_ERROR_DISPLAY: &str = "failed to decode a key from a table";

/// `NippyJarError::InconsistentData` display prefix of a read against a segment
/// being appended; matched as text because `AnyError` hides the typed source.
pub const TORN_RANGE_DISPLAY: &str = "attempted to read an inconsistent data range";

/// `std::io::ErrorKind::UnexpectedEof` display; matched as text for a static-file
/// sidecar read that landed mid-append.
pub const SHORT_READ_DISPLAY: &str = "failed to fill whole buffer";

/// `NippyJarError::InconsistentSnapshot` display prefix of a committed-snapshot
/// validation refusal; transient across the mark→publish window.
pub const SNAPSHOT_DISPLAY: &str = "inconsistent static-file snapshot";

/// Which of the three ways a read failed; every [`ReadError`] has exactly one class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadClass {
    /// The same read at the same block can legitimately answer differently later —
    /// reth has not written the bytes where this read looks yet.
    Transient,
    /// Retrying cannot help, but the answer says nothing about the committed state:
    /// a revert or this node's own storage fault, so the node keeps running.
    Permanent,
    /// The chain contradicted its own invariants: every validator reads the same
    /// impossible thing, so the consumer that must act on the epoch fails closed.
    Impossible,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum ReadError {
    #[error("state/header for block {0} not found")]
    BlockNotFound(B256),

    /// The header exists but its executed state is not materialized at `hash` — reth
    /// backfills headers ahead of state, and an unwind can remove it at or below finalized.
    #[error("state for block {hash} is not materialized yet (reth pipeline backfill)")]
    StateNotMaterialized { hash: B256 },

    /// A torn static-file read concurrent with a segment append (the four
    /// `*_DISPLAY` shapes above); genuine corruption is indistinguishable from it.
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

    /// `size` counts the primary tier only — `committee[E−1] ∪ committee[E] ∪
    /// committee[E+1]` — never the Active registry.
    #[error("epoch {epoch} tracker primary peer-set size {size} (committee[E−1] ∪ committee[E] ∪ committee[E+1]) exceeds configured max_peer_set_size {max} (misconfig / governance drift)")]
    PeerSetTooLarge { epoch: u64, size: usize, max: usize },

    /// Not strictly ascending on the raw peer-pubkey bytes — that order is the
    /// consensus index space, so a divergence would slash the wrong validator.
    #[error("epoch {epoch} committee is not ascending on peer pubkey at position {position} (member {validator}) — contract/consensus index-space divergence")]
    CommitteeOutOfOrder {
        epoch: u64,
        position: usize,
        validator: alloy_primitives::Address,
    },

    /// Two members share a peer pubkey, leaving the positional index ambiguous.
    #[error("epoch {epoch} committee has a duplicate peer pubkey at position {position} (member {validator}) — on-chain uniqueness invariant violated")]
    CommitteeDuplicatePeerKey {
        epoch: u64,
        position: usize,
        validator: alloy_primitives::Address,
    },

    /// A non-empty committee below the contract's floor: `commitEpochCommittee`
    /// reverts under it, so no legal chain state has this shape.
    #[error("epoch {epoch} committee size {size} is below MIN_COMMITTEE_LENGTH {min}")]
    CommitteeTooSmall { epoch: u64, size: usize, min: usize },

    #[error("provider/evm backend error: {0}")]
    Backend(String),

    #[error("ChainConfig.getEpochBlockInterval() returned 0 — epoch division undefined")]
    ZeroEpochInterval,
}

impl ReadError {
    /// The variant's [`ReadClass`]; no `_` arm, deliberately, so a new variant
    /// cannot inherit a class silently.
    pub fn class(&self) -> ReadClass {
        match self {
            Self::StateNotMaterialized { .. }
            | Self::TransientStorage(_)
            | Self::BlockNotFound(_) => ReadClass::Transient,
            Self::CallReverted(_) | Self::Backend(_) => ReadClass::Permanent,
            Self::AbiDecode(_)
            | Self::BlsKey(_)
            | Self::PeerKey
            | Self::CommitteeMemberKeyless { .. }
            | Self::PeerSetTooLarge { .. }
            | Self::CommitteeOutOfOrder { .. }
            | Self::CommitteeDuplicatePeerKey { .. }
            | Self::CommitteeTooSmall { .. }
            | Self::ZeroEpochInterval => ReadClass::Impossible,
        }
    }

    /// Whether the same read can legitimately answer differently later; `false`
    /// covers both the permanent refusal and the impossible chain.
    pub fn is_transient(&self) -> bool {
        matches!(self.class(), ReadClass::Transient)
    }

    /// Whether the failure is the chain contradicting itself rather than this
    /// node failing to read.
    pub fn is_contract_impossible(&self) -> bool {
        matches!(self.class(), ReadClass::Impossible)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Address;

    #[test]
    fn only_the_three_not_here_yet_reads_are_transient() {
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

    /// Every variant's class, read back through both predicates. A new variant is
    /// caught by `class`'s missing `_` arm, not by this hand-written list.
    #[test]
    fn every_variant_falls_into_exactly_one_of_the_three_classes() {
        let cases = [
            (
                ReadError::StateNotMaterialized { hash: B256::ZERO },
                ReadClass::Transient,
            ),
            (
                ReadError::TransientStorage(TORN_RANGE_DISPLAY.into()),
                ReadClass::Transient,
            ),
            (ReadError::BlockNotFound(B256::ZERO), ReadClass::Transient),
            (
                ReadError::CallReverted("execution reverted".into()),
                ReadClass::Permanent,
            ),
            (
                ReadError::Backend("no state found".into()),
                ReadClass::Permanent,
            ),
            (
                ReadError::AbiDecode("committee/stakes length mismatch".into()),
                ReadClass::Impossible,
            ),
            (
                ReadError::BlsKey("subgroup check".into()),
                ReadClass::Impossible,
            ),
            (ReadError::PeerKey, ReadClass::Impossible),
            (
                ReadError::CommitteeMemberKeyless {
                    epoch: 7,
                    validator: Address::ZERO,
                },
                ReadClass::Impossible,
            ),
            (
                ReadError::PeerSetTooLarge {
                    epoch: 7,
                    size: 99,
                    max: 51,
                },
                ReadClass::Impossible,
            ),
            (
                ReadError::CommitteeOutOfOrder {
                    epoch: 7,
                    position: 2,
                    validator: Address::ZERO,
                },
                ReadClass::Impossible,
            ),
            (
                ReadError::CommitteeDuplicatePeerKey {
                    epoch: 7,
                    position: 2,
                    validator: Address::ZERO,
                },
                ReadClass::Impossible,
            ),
            (
                ReadError::CommitteeTooSmall {
                    epoch: 7,
                    size: 3,
                    min: 4,
                },
                ReadClass::Impossible,
            ),
            (ReadError::ZeroEpochInterval, ReadClass::Impossible),
        ];

        for (error, class) in cases {
            assert_eq!(error.class(), class, "{error} is classified elsewhere");
            assert_eq!(
                error.is_transient(),
                class == ReadClass::Transient,
                "{error}: is_transient disagrees with its class"
            );
            assert_eq!(
                error.is_contract_impossible(),
                class == ReadClass::Impossible,
                "{error}: is_contract_impossible disagrees with its class"
            );
        }
    }
}
