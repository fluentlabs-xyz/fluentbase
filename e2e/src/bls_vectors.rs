//! Real BLS12-381 proof-of-possession vectors for the staking fixtures.
//!
//! The staking contract verifies every proof of possession itself now, against
//! the EIP-2537 precompiles the harness really runs. A filler key no longer gets
//! past `initialize`, so the fixtures need keys a pairing accepts.
//!
//! These are not synthesised here. `bls_pop_vectors.bin` was produced by the
//! NODE's signer — `fluentbase-bls` (`blst`) driving
//! `keys::derive("test test … junk", 400, chain_id = 1)` and `pop::sign_pop`,
//! through `devnet/local-dpos-smoke/genesis-bootstrap`'s own key derivation, and
//! re-encoded to EIP-2537 by `fluentbase_bls::encoding`. That is what makes a
//! passing fixture evidence: the signature comes from one BLS implementation
//! (blst, in the node) and the pairing that accepts it comes from another
//! (arkworks, in the precompile), with the contract's hash-to-curve in between.
//!
//! `CHAIN_ID` is load-bearing. The message a proof of possession signs is
//! `"FLUENT_DPOS_V1_" ‖ chain_id` as a big-endian u64, so a fixture that runs
//! under a different chain id verifies nothing — it fails closed, with
//! `InvalidProofOfPossession`.

/// The chain id these vectors were signed under, and therefore the one a fixture
/// using them must run under.
pub const CHAIN_ID: u64 = 1;

/// `pubkey_uncompressed(256) ‖ pop(128) ‖ pubkey_compressed(96)` per validator,
/// in derivation-index order.
const VECTORS: &[u8] = include_bytes!("bls_pop_vectors.bin");
const PUBKEY_LENGTH: usize = 256;
const POP_LENGTH: usize = 128;
const COMPRESSED_LENGTH: usize = 96;
const STRIDE: usize = PUBKEY_LENGTH + POP_LENGTH + COMPRESSED_LENGTH;

/// How many validators the fixtures can seat before they run out of keys.
pub fn count() -> usize {
    VECTORS.len() / STRIDE
}

fn entry(index: usize) -> &'static [u8] {
    assert!(
        index < count(),
        "only {} PoP vectors are pinned; regenerate bls_pop_vectors.bin to seat more",
        count()
    );
    &VECTORS[index * STRIDE..(index + 1) * STRIDE]
}

/// 256-byte EIP-2537 G2 public key of validator `index`.
pub fn pubkey(index: usize) -> &'static [u8] {
    &entry(index)[..PUBKEY_LENGTH]
}

/// 128-byte EIP-2537 G1 proof of possession of validator `index`.
pub fn pop(index: usize) -> &'static [u8] {
    &entry(index)[PUBKEY_LENGTH..PUBKEY_LENGTH + POP_LENGTH]
}

/// The 96-byte zcash form of the same key, as `blst` compressed it inside the
/// node (`ValidatorBlsKeypair::public_bytes`).
///
/// This is the identity the node registers under and the one the contract's own
/// `compress_g2_unchecked` has to reproduce from the 256-byte form. Asserting
/// the two match is a cross-implementation check on the half-swap and the y-sign
/// rule, which no amount of in-crate testing can give.
pub fn pubkey_compressed(index: usize) -> &'static [u8] {
    &entry(index)[PUBKEY_LENGTH + POP_LENGTH..]
}
