use thiserror::Error;

/// Errors produced by the `fluentbase-bls` crate.
///
/// Variants are leaf-level: callers wrap these with their own context
/// (`anyhow`, custom enums, etc.) at the appropriate boundary.
#[derive(Debug, Error)]
pub enum Error {
    #[error("private key bytes are not a valid scalar (zero or out of field)")]
    InvalidSecret,

    #[error("public key bytes are not a valid G2 point (decoding, subgroup, or infinity check failed)")]
    InvalidPubkey,

    #[error("signature bytes are not a valid G1 point (decoding, subgroup, or infinity check failed)")]
    InvalidSignature,

    #[error("proof of possession verification failed")]
    InvalidPoP,
}
