use thiserror::Error;

/// Errors produced by the `fluentbase-bls` crate.
///
/// Variants are leaf-level: callers wrap these with their own context
/// (`anyhow`, custom enums, etc.) at the appropriate boundary.
#[derive(Debug, Error)]
pub enum Error {
    #[error("private key bytes are not a valid scalar (zero or out of field)")]
    InvalidSecret,

    #[error(
        "public key bytes are not a valid G2 point (decoding, subgroup, or infinity check failed)"
    )]
    InvalidPubkey,

    #[error(
        "signature bytes are not a valid G1 point (decoding, subgroup, or infinity check failed)"
    )]
    InvalidSignature,

    #[error("proof of possession verification failed")]
    InvalidPoP,

    #[error("failed reading key file from disk: {0}")]
    IoRead(#[from] std::io::Error),

    #[error("file contents not valid hex")]
    InvalidHex,

    #[error("input wrong length (expected 32 bytes)")]
    InvalidLength,

    #[error("signer_idx {signer_idx} is out of range for committee of {committee_len} validators")]
    SignerIndexOutOfRange {
        signer_idx: u32,
        committee_len: usize,
    },

    #[error("evidence epoch {evidence_epoch} does not match committee epoch {committee_epoch}")]
    EpochMismatch {
        evidence_epoch: u64,
        committee_epoch: u64,
    },

    #[error("evidence is not structurally conflicting (mismatched signer/round, or identical proposals)")]
    NonConflictingEvidence,

    #[error("EIP-2335 keystore: malformed JSON or unsupported version")]
    InvalidKeystore,

    #[error("EIP-2335 keystore: KDF derivation failed")]
    KeystoreKdf,

    #[error("EIP-2335 keystore: checksum mismatch (wrong password or tampered file)")]
    KeystoreChecksum,
}
