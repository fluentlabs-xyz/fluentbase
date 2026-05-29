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

pub mod encoding;
pub mod error;
pub mod evidence;
pub mod keys;
pub mod keystore;
pub mod pop;
pub mod scheme;

mod namespace;

pub use error::Error;
pub use namespace::fluent_namespace;
pub use scheme::EpochCommittee;

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

/// Macro-generated multisig signing scheme for the Simplex engine.
///
/// `bls12381_multisig::Scheme<P, V>` is a thin wrapper around the underlying
/// `Generic<P, V, N>` — see [crate doc](crate) for why we only expose this.
pub type Scheme = bls12381_multisig::Scheme<PeerPubkey, Variant>;

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
