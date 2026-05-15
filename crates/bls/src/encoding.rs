//! Conversion between blst compressed (z-cash) and EIP-2537 uncompressed
//! BLS12-381 point encodings, for the MinSig variant.
//!
//! - Pubkey ∈ G2: 96 B compressed (Commonware `.encode()`) ↔ 256 B EIP-2537.
//! - Signature ∈ G1: 48 B compressed ↔ 128 B EIP-2537.
//!
//! # Byte-order facts (see `research.md`)
//!
//! blst `serialize` emits **G1 = `X || Y`** and
//! **G2 = `X.c1 || X.c0 || Y.c1 || Y.c0`** (imaginary coefficient first,
//! z-cash convention), each Fp 48 B big-endian.
//!
//! EIP-2537 expects each Fp left-padded with 16 zero bytes to 64 B, and G2
//! ordered **`x.c0, x.c1, y.c0, y.c1`** (real coefficient first). Therefore
//! G2 conversion swaps the two halves of each Fp2 coordinate in addition to
//! padding; G1 conversion only pads.
//!
//! Infinity is rejected (`Err`): a validator key/signature is never the
//! identity point (mirrors Commonware `G1/G2::Read`).

use blst::min_sig::{PublicKey, Signature};

use crate::{
    error::Error, PUBKEY_BYTES, PUBKEY_EIP2537_BYTES, SIGNATURE_BYTES, SIGNATURE_EIP2537_BYTES,
};

const FP: usize = 48; // bytes per Fp (big-endian)
const PAD: usize = 16; // EIP-2537 left-pad: 48 → 64

/// Copy a 48-byte Fp into an EIP-2537 64-byte slot (16 zero bytes then the Fp).
#[inline]
fn put_padded(dst: &mut [u8], slot: usize, fp: &[u8]) {
    // The leading 16 bytes stay zero from the zero-initialised output buffer.
    dst[slot * 64 + PAD..slot * 64 + 64].copy_from_slice(fp);
}

/// Read a 48-byte Fp out of an EIP-2537 64-byte slot, rejecting non-zero pad.
#[inline]
fn get_padded(src: &[u8], slot: usize) -> Result<&[u8], ()> {
    let base = slot * 64;
    if src[base..base + PAD].iter().any(|&b| b != 0) {
        return Err(()); // malformed EIP-2537: top 16 bytes must be zero
    }
    Ok(&src[base + PAD..base + 64])
}

// ---------- Signature (G1) ----------

/// G1 compressed (48 B, z-cash) → EIP-2537 uncompressed (128 B).
pub fn signature_compressed_to_eip2537(
    sig: &[u8; SIGNATURE_BYTES],
) -> Result<[u8; SIGNATURE_EIP2537_BYTES], Error> {
    let point = Signature::uncompress(sig).map_err(|_| Error::InvalidSignature)?;
    // Reject infinity (sig_infcheck = true) + subgroup-implied validity.
    point.validate(true).map_err(|_| Error::InvalidSignature)?;
    let ser = point.serialize(); // [u8;96] = X(48) || Y(48)
    let mut out = [0u8; SIGNATURE_EIP2537_BYTES];
    put_padded(&mut out, 0, &ser[0..FP]); // x
    put_padded(&mut out, 1, &ser[FP..2 * FP]); // y
    Ok(out)
}

/// EIP-2537 uncompressed (128 B) → G1 compressed (48 B, z-cash).
pub fn signature_eip2537_to_compressed(
    eip: &[u8; SIGNATURE_EIP2537_BYTES],
) -> Result<[u8; SIGNATURE_BYTES], Error> {
    let x = get_padded(eip, 0).map_err(|_| Error::InvalidSignature)?;
    let y = get_padded(eip, 1).map_err(|_| Error::InvalidSignature)?;
    let mut ser = [0u8; 96];
    ser[0..FP].copy_from_slice(x);
    ser[FP..2 * FP].copy_from_slice(y);
    let point = Signature::deserialize(&ser).map_err(|_| Error::InvalidSignature)?;
    point.validate(true).map_err(|_| Error::InvalidSignature)?;
    Ok(point.compress())
}

// ---------- PublicKey (G2) ----------

/// G2 compressed (96 B, z-cash) → EIP-2537 uncompressed (256 B).
pub fn pubkey_compressed_to_eip2537(
    pk: &[u8; PUBKEY_BYTES],
) -> Result<[u8; PUBKEY_EIP2537_BYTES], Error> {
    let point = PublicKey::uncompress(pk).map_err(|_| Error::InvalidPubkey)?;
    point.validate().map_err(|_| Error::InvalidPubkey)?; // rejects infinity
    let ser = point.serialize(); // [u8;192] = X.c1 X.c0 Y.c1 Y.c0
    let mut out = [0u8; PUBKEY_EIP2537_BYTES];
    // EIP-2537 slot order: 0=x.c0 1=x.c1 2=y.c0 3=y.c1
    put_padded(&mut out, 0, &ser[FP..2 * FP]); // x.c0  ← blst[48..96]
    put_padded(&mut out, 1, &ser[0..FP]); // x.c1  ← blst[0..48]
    put_padded(&mut out, 2, &ser[3 * FP..4 * FP]); // y.c0  ← blst[144..192]
    put_padded(&mut out, 3, &ser[2 * FP..3 * FP]); // y.c1  ← blst[96..144]
    Ok(out)
}

/// EIP-2537 uncompressed (256 B) → G2 compressed (96 B, z-cash).
pub fn pubkey_eip2537_to_compressed(
    eip: &[u8; PUBKEY_EIP2537_BYTES],
) -> Result<[u8; PUBKEY_BYTES], Error> {
    let xc0 = get_padded(eip, 0).map_err(|_| Error::InvalidPubkey)?;
    let xc1 = get_padded(eip, 1).map_err(|_| Error::InvalidPubkey)?;
    let yc0 = get_padded(eip, 2).map_err(|_| Error::InvalidPubkey)?;
    let yc1 = get_padded(eip, 3).map_err(|_| Error::InvalidPubkey)?;
    let mut ser = [0u8; 192]; // blst order: X.c1 X.c0 Y.c1 Y.c0
    ser[0..FP].copy_from_slice(xc1);
    ser[FP..2 * FP].copy_from_slice(xc0);
    ser[2 * FP..3 * FP].copy_from_slice(yc1);
    ser[3 * FP..4 * FP].copy_from_slice(yc0);
    let point = PublicKey::deserialize(&ser).map_err(|_| Error::InvalidPubkey)?;
    point.validate().map_err(|_| Error::InvalidPubkey)?;
    Ok(point.compress())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::ValidatorBlsKeypair;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;

    fn kp(seed: u64) -> ValidatorBlsKeypair {
        ValidatorBlsKeypair::generate(&mut StdRng::seed_from_u64(seed))
    }

    // Independent reference: blst raw serialize is the authoritative
    // coordinate-value oracle; this transform is written separately from the
    // production fn (different code path) so a shared bug can't hide.
    fn reference_g2_to_eip2537(blst_ser_192: &[u8; 192]) -> [u8; 256] {
        let mut out = [0u8; 256];
        // blst: X.c1 X.c0 Y.c1 Y.c0 ; EIP: x.c0 x.c1 y.c0 y.c1, 16-pad
        for (eip_slot, blst_off) in [(0usize, 48usize), (1, 0), (2, 144), (3, 96)] {
            out[eip_slot * 64 + 16..eip_slot * 64 + 64]
                .copy_from_slice(&blst_ser_192[blst_off..blst_off + 48]);
        }
        out
    }

    #[test]
    fn pubkey_forward_matches_independent_reference() {
        for seed in 0..16u64 {
            let k = kp(seed);
            let comp = k.public_bytes();
            // blst raw serialize as oracle
            let pt = blst::min_sig::PublicKey::uncompress(&comp).unwrap();
            let expected = reference_g2_to_eip2537(&pt.serialize());
            let got = pubkey_compressed_to_eip2537(&comp).unwrap();
            assert_eq!(got, expected, "seed {seed}");
        }
    }

    #[test]
    fn pubkey_roundtrip() {
        for seed in 0..64u64 {
            let comp = kp(seed).public_bytes();
            let eip = pubkey_compressed_to_eip2537(&comp).unwrap();
            let back = pubkey_eip2537_to_compressed(&eip).unwrap();
            assert_eq!(comp, back, "seed {seed}");
        }
    }

    #[test]
    fn signature_roundtrip() {
        use crate::{namespace::fluent_namespace, pop::sign_pop};
        for seed in 0..64u64 {
            let k = kp(seed);
            let sig = sign_pop(&k, &fluent_namespace(20994));
            let eip = signature_compressed_to_eip2537(&sig).unwrap();
            let back = signature_eip2537_to_compressed(&eip).unwrap();
            assert_eq!(sig, back, "seed {seed}");
        }
    }

    #[test]
    fn rejects_garbage_and_bad_pad() {
        assert!(matches!(
            pubkey_compressed_to_eip2537(&[0xFFu8; PUBKEY_BYTES]),
            Err(Error::InvalidPubkey)
        ));
        // non-zero pad in an otherwise-valid EIP-2537 buffer is rejected
        let k = kp(1);
        let mut eip = pubkey_compressed_to_eip2537(&k.public_bytes()).unwrap();
        eip[0] = 0x01; // dirty the leading pad of slot 0
        assert!(matches!(
            pubkey_eip2537_to_compressed(&eip),
            Err(Error::InvalidPubkey)
        ));
    }

    #[test]
    fn infinity_compressed_is_rejected() {
        // z-cash compressed infinity: byte0 = 0xC0 (compressed|infinity), rest 0.
        let mut inf_g2 = [0u8; PUBKEY_BYTES];
        inf_g2[0] = 0xC0;
        assert!(pubkey_compressed_to_eip2537(&inf_g2).is_err());
        let mut inf_g1 = [0u8; SIGNATURE_BYTES];
        inf_g1[0] = 0xC0;
        assert!(signature_compressed_to_eip2537(&inf_g1).is_err());
    }
}

#[cfg(test)]
mod prop {
    use super::*;
    use crate::keys::ValidatorBlsKeypair;
    use proptest::prelude::*;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;

    proptest! {
        #[test]
        fn pubkey_roundtrip_prop(seed in any::<u64>()) {
            let comp = ValidatorBlsKeypair::generate(
                &mut StdRng::seed_from_u64(seed)).public_bytes();
            let eip = pubkey_compressed_to_eip2537(&comp).unwrap();
            prop_assert_eq!(comp, pubkey_eip2537_to_compressed(&eip).unwrap());
        }
    }
}
