//! Test-only EIP-2537 → compressed inverse conversions (MinSig). The forward
//! direction ships in `fluentbase_bls::encoding`; the reverse is only needed to
//! check round-trips and to drive the pinned conformance constants through
//! `verify_pop`, so it lives here rather than in the production API.
//!
//! Byte-order facts (mirror of `encoding.rs`): blst `serialize` emits
//! G1 = `X || Y` and G2 = `X.c1 || X.c0 || Y.c1 || Y.c0` (imaginary coefficient
//! first); EIP-2537 left-pads each Fp to 64 B and orders G2 as
//! `x.c0, x.c1, y.c0, y.c1` (real coefficient first).

use fluentbase_bls::{
    PUBKEY_BYTES, PUBKEY_EIP2537_BYTES, SIGNATURE_BYTES, SIGNATURE_EIP2537_BYTES,
};

const FP: usize = 48;
const PAD: usize = 16;

fn get_padded(src: &[u8], slot: usize) -> Option<&[u8]> {
    let base = slot * 64;
    if src[base..base + PAD].iter().any(|&b| b != 0) {
        return None; // malformed EIP-2537: top 16 bytes must be zero
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
    let mut ser = [0u8; 192]; // blst order: X.c1 X.c0 Y.c1 Y.c0
    ser[0..FP].copy_from_slice(xc1);
    ser[FP..2 * FP].copy_from_slice(xc0);
    ser[2 * FP..3 * FP].copy_from_slice(yc1);
    ser[3 * FP..4 * FP].copy_from_slice(yc0);
    let point = blst::min_sig::PublicKey::deserialize(&ser).ok()?;
    point.validate().ok()?;
    Some(point.compress())
}
