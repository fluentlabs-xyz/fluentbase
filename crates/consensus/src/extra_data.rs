//! Canonical byte format for the simplex-attestation embedded in
//! `block.header.extra_data`.
//!
//! Wire format:
//! `[epoch: u64 BE][view: u64 BE][committee_size: u8][bitmap: ceil(N/8) bytes]`
//! = 17 + ceil(N/8) bytes. Bit `i` of byte `i/8` (LSB-first within each
//! byte) = signer `i` present.
//!
//! Cross-language parity:
//! - Solidity decoder: `LivenessSlashing.processBitmap` (same LSB-first layout).
//! - `committee_size` is `u8` here AND in Solidity. Both Rust
//!   `fluentbase_p2p::constants::MAX_PEER_SET_SIZE` and Solidity
//!   `ChainConfig.MAX_ACTIVE_VALIDATORS` cap at 51. Bumping either past 255
//!   requires widening this wire format to u16 BE; the panic in
//!   `encode_simplex_attestation` is intentionally fail-loud, and the
//!   startup assert in `OuterBuilder::build` catches the config mistake
//!   before any block is proposed.
//! - Pinned by `crates/bls/tests/equivocation_evidence_conformance.rs` style
//!   hex fixtures (this module's own unit tests).

use commonware_consensus::types::{Epoch, Round, View};
use commonware_cryptography::certificate::Signers;

/// Encode `(round, signers)` into the canonical attestation byte format.
///
/// Panics if the committee size exceeds `u8::MAX` (255). The V1 target is
/// V=51, so this is impossible in practice; the panic exists so a future
/// governance change that widens the committee fails loudly at the encoder
/// rather than silently truncating to `committeeSize == 0` in Solidity
/// (which would early-return `processBitmap` and turn off liveness for the
/// affected blocks).
pub fn encode_simplex_attestation(round: Round, signers: &Signers) -> Vec<u8> {
    let n = signers.len();
    let committee_size: u8 = n
        .try_into()
        .expect("committee_size exceeds u8::MAX; widen wire format to u16 BE if this fires");
    let bitmap_bytes = n.div_ceil(8);
    // **** есть смысл вынести magic numbers, особенно если он преиспользуются
    let mut out = Vec::with_capacity(8 + 8 + 1 + bitmap_bytes);
    out.extend_from_slice(&round.epoch().get().to_be_bytes()); // 8 B
    out.extend_from_slice(&round.view().get().to_be_bytes()); // 8 B
    out.push(committee_size); // 1 B
    let mut bitmap = vec![0u8; bitmap_bytes];
    for p in signers.iter() {
        let idx = p.get() as usize;
        bitmap[idx >> 3] |= 1 << (idx & 7);
    }
    out.extend_from_slice(&bitmap);
    out
}

/// Decoded attestation. `committee_size` is encoded explicitly because it
/// cannot be uniquely derived from the bitmap byte length alone
/// (committee sizes 49..=56 all yield 7-byte bitmaps).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedAttestation {
    pub round: Round,
    pub committee_size: u8,
    pub bitmap: Vec<u8>,
}

/// Decode the canonical attestation byte format.
///
/// - Empty input → `Ok(None)` (cold-start / no-previous-cert).
/// - Otherwise validates the length matches the declared `committee_size`
///   and returns `Ok(Some(_))`.
pub fn decode_simplex_attestation(buf: &[u8]) -> Result<Option<DecodedAttestation>, DecodeError> {
    if buf.is_empty() {
        return Ok(None);
    }
    const HDR: usize = 8 + 8 + 1;
    if buf.len() < HDR {
        return Err(DecodeError::TooShort);
    }
    let epoch_u64 = u64::from_be_bytes(buf[0..8].try_into().unwrap());
    let view_u64 = u64::from_be_bytes(buf[8..16].try_into().unwrap());
    let committee_size = buf[16];
    if committee_size == 0 {
        // The encoder never emits a zero committee (it derives the size from a
        // real `signers.len()`); a zero-size attestation is only craftable by a
        // forger. Reject it rather than decode an empty bitmap (defense-in-depth;
        // `application.rs` byte-equal check + `evm.rs` decode are the other backstops).
        return Err(DecodeError::ZeroCommittee);
    }
    let expected_bitmap = (committee_size as usize).div_ceil(8);
    if buf.len() != HDR + expected_bitmap {
        return Err(DecodeError::LengthMismatch {
            expected: HDR + expected_bitmap,
            got: buf.len(),
        });
    }
    Ok(Some(DecodedAttestation {
        round: Round::new(Epoch::new(epoch_u64), View::new(view_u64)),
        committee_size,
        bitmap: buf[HDR..].to_vec(),
    }))
}

/// Bitmap-only encoder used by the validator for the byte-equal comparison
/// against the bitmap segment of `extra_data`. Mirrors the bitmap layout
/// in `encode_simplex_attestation` exactly.
pub fn encode_bitmap_only(signers: &Signers) -> Vec<u8> {
    let n = signers.len();
    debug_assert!(
        n <= u8::MAX as usize,
        "committee_size exceeds u8::MAX; widen wire format to u16 BE if this fires"
    );
    let bitmap_bytes = n.div_ceil(8);
    let mut bitmap = vec![0u8; bitmap_bytes];
    for p in signers.iter() {
        let idx = p.get() as usize;
        bitmap[idx >> 3] |= 1 << (idx & 7);
    }
    bitmap
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("extra_data too short for attestation header (need ≥ 17 bytes)")]
    TooShort,
    #[error("extra_data length mismatch: expected {expected}, got {got}")]
    LengthMismatch { expected: usize, got: usize },
    #[error("extra_data committee_size is zero (only craftable by a forger)")]
    ZeroCommittee,
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_utils::Participant;

    fn round(epoch: u64, view: u64) -> Round {
        Round::new(Epoch::new(epoch), View::new(view))
    }

    fn all_signers(n: usize) -> Signers {
        Signers::from(n, (0..n as u32).map(Participant::new))
    }

    #[test]
    fn empty_buf_decodes_to_none() {
        assert_eq!(decode_simplex_attestation(&[]).unwrap(), None);
    }

    #[test]
    fn header_only_too_short_errors() {
        let buf = vec![0u8; 16]; // < 17
        assert_eq!(
            decode_simplex_attestation(&buf).unwrap_err(),
            DecodeError::TooShort
        );
    }

    #[test]
    fn length_mismatch_errors() {
        // committee_size = 8 → expects 1 bitmap byte; supply 2.
        let mut buf = vec![0u8; 17];
        buf[16] = 8;
        buf.extend_from_slice(&[0xFF, 0xAA]);
        let err = decode_simplex_attestation(&buf).unwrap_err();
        assert!(matches!(err, DecodeError::LengthMismatch { .. }));
    }

    #[test]
    fn roundtrip_committee_one() {
        let signers = all_signers(1);
        let r = round(7, 42);
        let buf = encode_simplex_attestation(r, &signers);
        // Length = 17 + 1 = 18; bitmap byte 0 == 0b0000_0001.
        assert_eq!(buf.len(), 18);
        assert_eq!(buf[17], 0x01);
        let d = decode_simplex_attestation(&buf).unwrap().unwrap();
        assert_eq!(d.round, r);
        assert_eq!(d.committee_size, 1);
        assert_eq!(d.bitmap, vec![0x01]);
    }

    #[test]
    fn roundtrip_committee_eight() {
        let signers = all_signers(8);
        let r = round(1, 1);
        let buf = encode_simplex_attestation(r, &signers);
        assert_eq!(buf.len(), 17 + 1);
        assert_eq!(buf[17], 0xFF);
        let d = decode_simplex_attestation(&buf).unwrap().unwrap();
        assert_eq!(d.committee_size, 8);
        assert_eq!(d.bitmap, vec![0xFF]);
    }

    #[test]
    fn roundtrip_committee_fifty_one() {
        let signers = all_signers(51);
        let r = round(123, 456);
        let buf = encode_simplex_attestation(r, &signers);
        // 17 + ceil(51/8) = 17 + 7 = 24
        assert_eq!(buf.len(), 24);
        let d = decode_simplex_attestation(&buf).unwrap().unwrap();
        assert_eq!(d.committee_size, 51);
        assert_eq!(d.bitmap.len(), 7);
        // First 6 bytes are 0xFF (signers 0..47); last byte has bits 48,49,50 set.
        for byte in &d.bitmap[..6] {
            assert_eq!(*byte, 0xFF);
        }
        assert_eq!(d.bitmap[6], 0b0000_0111);
    }

    #[test]
    fn roundtrip_committee_sixty_four() {
        let signers = all_signers(64);
        let buf = encode_simplex_attestation(round(0, 0), &signers);
        assert_eq!(buf.len(), 17 + 8);
        let d = decode_simplex_attestation(&buf).unwrap().unwrap();
        assert_eq!(d.committee_size, 64);
        assert_eq!(d.bitmap, vec![0xFF; 8]);
    }

    #[test]
    fn lsb_first_bitmap_layout() {
        // Committee size = 9, only signers 0 and 8 present.
        let signers = Signers::from(9, [Participant::new(0), Participant::new(8)]);
        let buf = encode_simplex_attestation(round(1, 2), &signers);
        // Bitmap bytes (positions 17, 18): LSB of byte 0 (signer 0) +
        // LSB of byte 1 (signer 8).
        assert_eq!(buf[17], 0x01);
        assert_eq!(buf[18], 0x01);
    }

    #[test]
    fn encode_bitmap_only_matches_attestation_suffix() {
        let signers = Signers::from(
            8,
            [
                Participant::new(0),
                Participant::new(3),
                Participant::new(7),
            ],
        );
        let buf = encode_simplex_attestation(round(1, 2), &signers);
        let bitmap = encode_bitmap_only(&signers);
        assert_eq!(&buf[17..], bitmap.as_slice());
    }

    /// Hex-pinned fixture cross-checked with the Solidity decoder. If this
    /// fails after a deliberate format change, regenerate Solidity test
    /// inputs.
    #[test]
    fn hex_pinned_fixture() {
        // Committee size = 8, signers 0 and 3 present, epoch 7 view 42.
        let signers = Signers::from(8, [Participant::new(0), Participant::new(3)]);
        let buf = encode_simplex_attestation(round(7, 42), &signers);
        // epoch (8 B BE) = 0x00..07; view (8 B BE) = 0x00..2A; size = 0x08;
        // bitmap = 0b0000_1001 = 0x09.
        let expected = hex::decode("0000000000000007000000000000002a0809").unwrap();
        assert_eq!(buf, expected);
    }

    #[test]
    fn decode_zero_committee_size_is_rejected() {
        // 17 header bytes, committee_size = 0. The encoder never emits this;
        // a forged zero-committee attestation must be rejected, not decoded.
        let mut buf = vec![0u8; 16];
        buf.push(0); // committee_size byte = 0
        assert_eq!(
            decode_simplex_attestation(&buf),
            Err(DecodeError::ZeroCommittee)
        );
    }
}
