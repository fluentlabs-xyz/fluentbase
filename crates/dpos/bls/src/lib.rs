//! BLS12-381 multisig wrapper for Fluent DPoS consensus signing.
//!
//! This crate is a thin layer over [`commonware_cryptography::bls12381`] and
//! [`commonware_consensus::simplex::scheme::bls12381_multisig`]. It pins the
//! `MinSig` variant (pubkey in G2 96 B, signature in G1 48 B) and re-exports
//! the macro-generated `Scheme` rather than the public-field `Generic`.
//!
//! # Invariants
//!
//! - **Variant pin**: `MinSig` only. Never instantiated with `MinPk`.
//! - **Scheme wrapper**: Only the macro-generated [`Scheme`] is exposed.
//!   The underlying `Generic` has a `pub signer: Option<(Participant, Private)>`
//!   field and must never be re-exported.
//! - **Stock PoP**: [`pop::sign_pop`] delegates straight to
//!   `ops::sign_proof_of_possession::<MinSig>`. Address binding was considered
//!   and explicitly rejected.
//! - **Attestation handling**: `Attestation<S>` is *not* exposed by this crate.
//!   The Simplex Engine pipeline calls `verify_attestations` (with subgroup
//!   check) before any attestation contributes to a `Certificate`. If a future
//!   consumer needs to forward raw attestations (gossip, DA proofs, observers),
//!   it MUST call `attestation.signature.get()` to force blst decode + subgroup
//!   check before trusting the bytes.
//!
//! # Why not [`commonware_cryptography::bls12381::PrivateKey`]?
//!
//! That high-level wrapper hardcodes the `MinPk` variant. We use the low-level
//! [`commonware_cryptography::bls12381::primitives::group::Private`] together
//! with `ops::*::<MinSig>` to stay on MinSig.

use commonware_consensus::simplex::scheme::bls12381_multisig;
use commonware_cryptography::bls12381::primitives::variant::MinSig;
use commonware_cryptography::ed25519;

pub mod beacon;
pub mod combined_scheme;
pub mod encoding;
pub mod error;
pub mod keys;
pub mod keystore;
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

/// Inner multisig signing scheme — the attributable VOTE half of the
/// [`Scheme`] (`CombinedScheme`). Used directly only by the combined scheme's
/// delegation and by verifier-only consumers that need just the vote half.
///
/// `bls12381_multisig::Scheme<P, V>` is a thin wrapper around the underlying
/// `Generic<P, V, N>` — see [crate doc](crate) for why we only expose this.
pub type VoteScheme = bls12381_multisig::Scheme<PeerPubkey, Variant>;

/// The Fluent DPoS consensus scheme: an attributable multisig vote (for
/// finalization + slashing) fused with a threshold beacon seed partial (for
/// randomness). Every vote carries both; the seed is recovered from the
/// notarization/finalization certificate. See [`combined_scheme`].
pub type Scheme = combined_scheme::CombinedScheme;

/// Compressed pubkey byte length.
pub const PUBKEY_BYTES: usize = 96;

/// Compressed signature byte length.
pub const SIGNATURE_BYTES: usize = 48;

/// Private scalar byte length.
pub const SECRET_BYTES: usize = 32;

/// EIP-2537 uncompressed pubkey byte length (G2: 4 × 64).
pub const PUBKEY_EIP2537_BYTES: usize = 256;

/// EIP-2537 uncompressed signature byte length (G1: 2 × 64).
pub const SIGNATURE_EIP2537_BYTES: usize = 128;

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
/// appended by Commonware internally — our wrapper does NOT add them.
///
/// `chain_id` is included to prevent cross-chain replay (a signature valid on
/// testnet must not verify on mainnet).
///
/// The literal `"FLUENT_DPOS_V1_"` is immutable for the lifetime of the V1
/// chain. Any change to the variant, scheme, curve, or canonical encoding
/// requires a hard fork with a new chain_id and namespace `"FLUENT_DPOS_V2_"`.
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
