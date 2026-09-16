//! BLS12-381 multisig wrapper for Fluent DPoS consensus signing.
//!
//! A thin layer over [`commonware_cryptography::bls12381`] and
//! [`commonware_consensus::simplex::scheme::bls12381_multisig`], pinned to the
//! `MinSig` variant (pubkey in G2 96 B, signature in G1 48 B).
//!
//! # Invariants
//!
//! - **Variant pin**: `MinSig` only, never `MinPk` — we use the low-level
//!   [`commonware_cryptography::bls12381::primitives::group::Private`] with
//!   `ops::*::<MinSig>`, because the high-level
//!   [`commonware_cryptography::bls12381::PrivateKey`] wrapper hardcodes `MinPk`.
//! - **Scheme wrapper**: only the macro-generated [`Scheme`] is exposed. The
//!   underlying `Generic` has a `pub signer: Option<(Participant, Private)>` field
//!   and must never be re-exported.
//! - **Attestation handling**: `Attestation<S>` is *not* exposed by this crate.
//!   The Simplex Engine calls `verify_attestations` (with subgroup check) before
//!   any attestation contributes to a `Certificate`; a consumer that forwards raw
//!   attestations must call `attestation.signature.get()` to force blst decode +
//!   subgroup check before trusting the bytes.

use commonware_consensus::simplex::scheme::bls12381_multisig;
use commonware_cryptography::bls12381::primitives::variant::MinSig;
use commonware_cryptography::ed25519;

pub mod beacon;
pub mod combined_scheme;
pub mod encoding;
pub mod error;
pub mod keys;
pub mod keystore;
pub mod oracle;
pub mod pop;
pub mod scheme;
pub mod secret_store;
pub mod share_seal;

pub use error::Error;
pub use scheme::EpochCommittee;
pub use share_seal::ShareSealKey;

/// BLS variant fixed to MinSig.
pub type Variant = MinSig;

/// Identity (peer) public key used for participant ordering and P2P auth.
pub type PeerPubkey = ed25519::PublicKey;

/// BLS public key (G2 compressed, 96 bytes for MinSig).
pub type BlsPubkey =
    <MinSig as commonware_cryptography::bls12381::primitives::variant::Variant>::Public;

/// BLS signature (G1 compressed, 48 bytes for MinSig).
pub type BlsSignature =
    <MinSig as commonware_cryptography::bls12381::primitives::variant::Variant>::Signature;

/// Inner multisig signing scheme — the attributable vote half of [`Scheme`]. Used
/// directly only by the combined scheme's delegation and by verifier-only
/// consumers that need just the vote half.
///
/// `bls12381_multisig::Scheme<P, V>` is a thin wrapper around the underlying
/// `Generic<P, V, N>` — see the crate doc for why only this is exposed.
pub type VoteScheme = bls12381_multisig::Scheme<PeerPubkey, Variant>;

/// The Fluent DPoS consensus scheme: an attributable multisig vote (for
/// finalization + slashing) fused with a threshold beacon seed partial (for
/// randomness). Every vote carries both; the seed is recovered from the
/// notarization/finalization certificate. See [`combined_scheme`].
pub type Scheme = combined_scheme::CombinedScheme;

/// Key and signature widths on the wire, declared once in `fluentbase-types` and
/// imported by both sides: the staking contract checks incoming keys and proofs of
/// possession against the same four numbers. Compressed: G2 pubkey / G1 signature
/// under MinSig. Uncompressed: the EIP-2537 forms, G2 as 4 × 64 and G1 as 2 × 64.
pub use fluentbase_types::staking_protocol::{
    BLS_PUBKEY_LENGTH as PUBKEY_BYTES, BLS_PUBKEY_UNCOMPRESSED_LENGTH as PUBKEY_EIP2537_BYTES,
    BLS_SIGNATURE_LENGTH as SIGNATURE_BYTES,
    BLS_SIGNATURE_UNCOMPRESSED_LENGTH as SIGNATURE_EIP2537_BYTES,
};

/// Private scalar byte length. Not shared: the contract never sees a secret.
pub const SECRET_BYTES: usize = 32;

/// Build the base BLS namespace for a given chain.
///
/// Layout:
///
/// ```text
/// [b"FLUENT_DPOS_V1_"] || [chain_id.to_be_bytes()]
/// ↑ 15 bytes              ↑ 8 bytes              = 23 bytes total
/// ```
///
/// Per-subject suffixes (`_NOTARIZE`, `_NULLIFY`, `_FINALIZE`, `_SEED`) are
/// appended by commonware internally; this wrapper does not add them. `chain_id`
/// prevents cross-chain replay.
///
/// The literal `"FLUENT_DPOS_V1_"` is immutable for the lifetime of the V1 chain:
/// changing the variant, scheme, curve, or canonical encoding requires a hard fork
/// with a new `chain_id` and the namespace `"FLUENT_DPOS_V2_"`.
pub fn fluent_namespace(chain_id: u64) -> Vec<u8> {
    let mut ns = Vec::with_capacity(15 + 8);
    ns.extend_from_slice(b"FLUENT_DPOS_V1_");
    ns.extend_from_slice(&chain_id.to_be_bytes());
    ns
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fluent_namespace_layout_is_stable() {
        let ns = fluent_namespace(20994);
        assert_eq!(ns.len(), 23);
        assert_eq!(&ns[..15], b"FLUENT_DPOS_V1_");
        assert_eq!(&ns[15..], &20994u64.to_be_bytes());
    }

    #[test]
    fn fluent_namespace_distinguishes_chain_ids() {
        assert_ne!(fluent_namespace(1), fluent_namespace(2));
        assert_ne!(fluent_namespace(0), fluent_namespace(u64::MAX));
    }
}
