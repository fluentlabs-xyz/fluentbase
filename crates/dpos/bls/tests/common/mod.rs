//! Test-only EIP-2537 → compressed inverses (MinSig). `fluentbase_bls::encoding`
//! ships the forward direction; the reverse exists only for round-trip tests and
//! for driving the pinned conformance constants through `verify_pop`, so it stays
//! out of the production API.
//!
//! Byte-order contract: blst `serialize` emits G2 as `X.c1 || X.c0 || Y.c1 ||
//! Y.c0` (imaginary coefficient first), while EIP-2537 stores
//! `x.c0, x.c1, y.c0, y.c1` left-padded to 64 B per Fp.

use fluentbase_bls::{
    PUBKEY_BYTES, PUBKEY_EIP2537_BYTES, SIGNATURE_BYTES, SIGNATURE_EIP2537_BYTES,
};

const FP: usize = 48;
const PAD: usize = 16;

fn get_padded(src: &[u8], slot: usize) -> Option<&[u8]> {
    let base = slot * 64;
    if src[base..base + PAD].iter().any(|&b| b != 0) {
        return None; // EIP-2537 requires the top 16 bytes of each 64-byte slot to be zero
    }
    Some(&src[base + PAD..base + 64])
}

/// EIP-2537 uncompressed (128 B) → G1 compressed (48 B, z-cash).
pub fn signature_eip2537_to_compressed(
    eip: &[u8; SIGNATURE_EIP2537_BYTES],
) -> Option<[u8; SIGNATURE_BYTES]> {
    let x = get_padded(eip, 0)?;
    let y = get_padded(eip, 1)?;
    let mut ser = [0u8; 96];
    ser[0..FP].copy_from_slice(x);
    ser[FP..2 * FP].copy_from_slice(y);
    let point = blst::min_sig::Signature::deserialize(&ser).ok()?;
    point.validate(true).ok()?;
    Some(point.compress())
}

/// EIP-2537 uncompressed (256 B) → G2 compressed (96 B, z-cash).
pub fn pubkey_eip2537_to_compressed(
    eip: &[u8; PUBKEY_EIP2537_BYTES],
) -> Option<[u8; PUBKEY_BYTES]> {
    let xc0 = get_padded(eip, 0)?;
    let xc1 = get_padded(eip, 1)?;
    let yc0 = get_padded(eip, 2)?;
    let yc1 = get_padded(eip, 3)?;
    let mut ser = [0u8; 192];
    ser[0..FP].copy_from_slice(xc1);
    ser[FP..2 * FP].copy_from_slice(xc0);
    ser[2 * FP..3 * FP].copy_from_slice(yc1);
    ser[3 * FP..4 * FP].copy_from_slice(yc0);
    let point = blst::min_sig::PublicKey::deserialize(&ser).ok()?;
    point.validate().ok()?;
    Some(point.compress())
}
