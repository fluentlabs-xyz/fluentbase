//! Simplex equivocation-evidence parsing.
//!
//! Ported from the `SimplexEvidenceDecoder` Solidity contract, which used to sit
//! behind a governance-rotatable address in the chain config. The wire format is
//! whatever the node's `commonware` codec emits and is not ours to change.
//!
//! The port accepts and rejects exactly what the original did, but three
//! sanctioned divergences separate the two texts. The original's single
//! `InvalidEvidence()` is split into one structural error plus three semantic
//! ones, so a caller can tell a malformed blob from a well-formed non-conflict.
//! The 32-byte payload comparison is a direct array compare where the original
//! hashed both sides and compared digests — same verdict, no keccak. The third,
//! the inline shift-63 rejection, is documented at `Cursor::uvarint`.
//!
//! Bare concatenation — no envelope, no tag byte, no length prefix:
//!
//! ```text
//! Round       = uvarint(epoch) ‖ uvarint(view)
//! Proposal    = Round ‖ uvarint(parent) ‖ payload[32]
//! Attestation = uvarint(signerIdx) ‖ sig[48]
//!
//! ConflictingNotarize = ConflictingFinalize = (Proposal ‖ Attestation) × 2
//! NullifyFinalize                           = Round ‖ Attestation ‖ Proposal ‖ Attestation
//! ```
//!
//! Nothing in the bytes says which shape they are: the caller's entry point is
//! the only discriminator, and the two conflicting shapes are byte-identical in
//! structure. The two halves of a conflicting blob arrive in the order the node
//! observed the votes, so no ordering may be assumed.

use crate::{
    consts::*,
    util::{revert, revert_with},
};
use core::ops::Range;
use fluentbase_sdk::{Bytes, ExitCode, SharedAPI};

/// Parsed evidence, ready for signature verification.
///
/// `msg1`/`msg2` are the exact byte spans that were signed, sliced out of the
/// input rather than rebuilt: the verifier hashes them verbatim, so a
/// re-encoding differing by one byte would reject honest evidence.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct DecodedEvidence {
    pub epoch: u64,
    pub kind1: u8,
    pub msg1: Bytes,
    pub sig1: Bytes,
    pub kind2: u8,
    pub msg2: Bytes,
    pub sig2: Bytes,
}

/// Which conflict the caller is asserting. Not derivable from the bytes.
#[derive(Clone, Copy)]
pub(crate) enum EvidenceShape {
    ConflictingNotarize,
    ConflictingFinalize,
    NullifyFinalize,
}

pub(crate) fn decode<SDK: SharedAPI>(
    sdk: &mut SDK,
    evidence: &Bytes,
    shape: EvidenceShape,
) -> Result<DecodedEvidence, ExitCode> {
    match shape {
        EvidenceShape::ConflictingNotarize => {
            decode_conflicting(sdk, evidence, EVIDENCE_MESSAGE_KIND_NOTARIZE)
        }
        EvidenceShape::ConflictingFinalize => {
            decode_conflicting(sdk, evidence, EVIDENCE_MESSAGE_KIND_FINALIZE)
        }
        EvidenceShape::NullifyFinalize => decode_nullify_finalize(sdk, evidence),
    }
}

fn decode_conflicting<SDK: SharedAPI>(
    sdk: &mut SDK,
    evidence: &Bytes,
    kind: u8,
) -> Result<DecodedEvidence, ExitCode> {
    let Ok((first, att1, second, att2)) = parse_conflicting(evidence) else {
        return revert(sdk, ERR_INVALID_EVIDENCE_ENCODING);
    };
    if att1.signer != att2.signer {
        return revert_with(
            sdk,
            ERR_EVIDENCE_SIGNER_MISMATCH,
            &(att1.signer, att2.signer),
        );
    }
    if first.epoch != second.epoch || first.view != second.view {
        return revert_with(
            sdk,
            ERR_EVIDENCE_ROUND_MISMATCH,
            &(first.epoch, first.view, second.epoch, second.view),
        );
    }
    if first.parent == second.parent && first.payload == second.payload {
        return revert_with(
            sdk,
            ERR_EVIDENCE_PROPOSALS_IDENTICAL,
            &(first.epoch, first.view),
        );
    }
    Ok(DecodedEvidence {
        epoch: first.epoch,
        kind1: kind,
        msg1: evidence.slice(first.span),
        sig1: evidence.slice(att1.signature),
        kind2: kind,
        msg2: evidence.slice(second.span),
        sig2: evidence.slice(att2.signature),
    })
}

fn decode_nullify_finalize<SDK: SharedAPI>(
    sdk: &mut SDK,
    evidence: &Bytes,
) -> Result<DecodedEvidence, ExitCode> {
    let Ok((round, att1, proposal, att2)) = parse_nullify_finalize(evidence) else {
        return revert(sdk, ERR_INVALID_EVIDENCE_ENCODING);
    };
    if att1.signer != att2.signer {
        return revert_with(
            sdk,
            ERR_EVIDENCE_SIGNER_MISMATCH,
            &(att1.signer, att2.signer),
        );
    }
    if round.epoch != proposal.epoch || round.view != proposal.view {
        return revert_with(
            sdk,
            ERR_EVIDENCE_ROUND_MISMATCH,
            &(round.epoch, round.view, proposal.epoch, proposal.view),
        );
    }
    // No differing-proposal check here, and none is possible: a nullify carries
    // no proposal. Nullifying a round and finalizing it already contradict.
    // The finalize vote's parent and payload are parsed for their length only.
    Ok(DecodedEvidence {
        epoch: round.epoch,
        kind1: EVIDENCE_MESSAGE_KIND_NULLIFY,
        msg1: evidence.slice(round.span),
        sig1: evidence.slice(att1.signature),
        kind2: EVIDENCE_MESSAGE_KIND_FINALIZE,
        msg2: evidence.slice(proposal.span),
        sig2: evidence.slice(att2.signature),
    })
}

struct ParsedRound {
    epoch: u64,
    view: u64,
    span: Range<usize>,
}

struct ParsedProposal {
    epoch: u64,
    view: u64,
    parent: u64,
    payload: [u8; PROPOSAL_PAYLOAD_LENGTH],
    span: Range<usize>,
}

struct ParsedAttestation {
    signer: u32,
    signature: Range<usize>,
}

type Conflicting = (
    ParsedProposal,
    ParsedAttestation,
    ParsedProposal,
    ParsedAttestation,
);
type NullifyFinalize = (
    ParsedRound,
    ParsedAttestation,
    ParsedProposal,
    ParsedAttestation,
);

fn parse_conflicting(evidence: &[u8]) -> Result<Conflicting, ()> {
    let mut cursor = Cursor::new(evidence);
    let first = cursor.proposal()?;
    let att1 = cursor.attestation()?;
    let second = cursor.proposal()?;
    let att2 = cursor.attestation()?;
    cursor.finish()?;
    Ok((first, att1, second, att2))
}

fn parse_nullify_finalize(evidence: &[u8]) -> Result<NullifyFinalize, ()> {
    let mut cursor = Cursor::new(evidence);
    let round = cursor.round()?;
    let att1 = cursor.attestation()?;
    let proposal = cursor.proposal()?;
    let att2 = cursor.attestation()?;
    cursor.finish()?;
    Ok((round, att1, proposal, att2))
}

/// Cursor over the blob. Every read bounds-checks before advancing, so
/// `offset <= bytes.len()` holds throughout and `finish` reduces to an
/// exact-length check.
struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    /// Protobuf-style LEB128: low seven bits first, `0x80` marks continuation.
    ///
    /// Padded encodings are accepted, matching the Solidity decoder this
    /// replaces. The node's codec refuses to emit them
    /// (`commonware codec/src/varint.rs:125-127`), so the laxness is
    /// unreachable from an honest producer.
    fn uvarint(&mut self) -> Result<u64, ()> {
        let mut value: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = *self.bytes.get(self.offset).ok_or(())?;
            self.offset += 1;
            if shift >= 64 {
                return Err(());
            }
            let chunk = u64::from(byte & 0x7f);
            // The tenth byte contributes bits 63..69. Only bit 63 fits; the
            // Solidity original accumulates in a uint256 and rejects the
            // result above 2^64-1, which admits exactly the same inputs.
            if shift == 63 && chunk > 1 {
                return Err(());
            }
            value |= chunk << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
        }
    }

    fn take(&mut self, len: usize) -> Result<Range<usize>, ()> {
        let end = self.offset.checked_add(len).ok_or(())?;
        if end > self.bytes.len() {
            return Err(());
        }
        let span = self.offset..end;
        self.offset = end;
        Ok(span)
    }

    fn round(&mut self) -> Result<ParsedRound, ()> {
        let start = self.offset;
        let epoch = self.uvarint()?;
        let view = self.uvarint()?;
        Ok(ParsedRound {
            epoch,
            view,
            span: start..self.offset,
        })
    }

    fn proposal(&mut self) -> Result<ParsedProposal, ()> {
        let start = self.offset;
        let epoch = self.uvarint()?;
        let view = self.uvarint()?;
        let parent = self.uvarint()?;
        let payload_span = self.take(PROPOSAL_PAYLOAD_LENGTH)?;
        let mut payload = [0u8; PROPOSAL_PAYLOAD_LENGTH];
        payload.copy_from_slice(&self.bytes[payload_span]);
        Ok(ParsedProposal {
            epoch,
            view,
            parent,
            payload,
            span: start..self.offset,
        })
    }

    fn attestation(&mut self) -> Result<ParsedAttestation, ()> {
        let signer = self.uvarint()?;
        if signer > u64::from(u32::MAX) {
            return Err(());
        }
        let signature = self.take(BLS_SIGNATURE_LENGTH)?;
        Ok(ParsedAttestation {
            signer: signer as u32,
            signature,
        })
    }

    fn finish(&self) -> Result<(), ()> {
        if self.offset != self.bytes.len() {
            return Err(());
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use alloc::{vec, vec::Vec};
    use fluentbase_sdk::hex;
    use fluentbase_testing::TestingContextImpl;

    /// Golden corpus, copied verbatim from the node-side conformance fixture
    /// `fluentbase:crates/dpos/consensus/tests/equivocation_evidence_conformance.rs`,
    /// which pins these bytes against the production `commonware` encoder.
    pub(crate) const CONFLICTING_NOTARIZE: [u8; 168] = hex!(
        "072a29aa000000000000000000000000000000000000000000000000000000000000aa
         038aa1d24f195fc333878b14744f62a363acf0051249c949c4cc473850991aa708
         41eea2171a333b13de2e61fed4936305
         072a29bb000000000000000000000000000000000000000000000000000000000000bb
         03923c9abd2f0abe63eed5a2d9ac175032b2b48685c61f9e6a7c8b7419d7807782
         1d82a3bfd41a5f10bcfcd8434444f820"
    );
    pub(crate) const CONFLICTING_FINALIZE: [u8; 168] = hex!(
        "072a29cc000000000000000000000000000000000000000000000000000000000000cc
         039936ff0962301d36721c6d9e7947ec8a340bb9b5b7fcfa74ba2582918c9b3358
         b31c15c2a8ae372f3340e8c7706d32a6
         072a29dd000000000000000000000000000000000000000000000000000000000000dd
         03877570329a653f6cf0916cd5332247cd29a73d60a867dc1d710d5fe1bb4449b1
         e9393d5f9aed23bb08a2f9aed0e65af2"
    );
    pub(crate) const NULLIFY_FINALIZE: [u8; 135] = hex!(
        "072a
         03b9d1ed34ffda9193ce95eee9ab8db558f4e923a1b58a6f80ca0bf221f7567f72
         d65132b103190fd5c687f7f7a6cdc3db
         072a29ee000000000000000000000000000000000000000000000000000000000000ee
         0389eae226e709054f09892935d13772a1e73b62a8bfbd24e5f1f63617c5242714
         166f49b52ca5d97475b81820f75a6161"
    );

    /// `0xff × 9, 0x01` — ten bytes, top byte contributing bit 63: `u64::MAX`.
    const U64_MAX_VARINT: [u8; 10] = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01];
    /// `0x80 × 9, 0x01` — the same ten-byte width carrying only bit 63.
    const TWO_POW_63_VARINT: [u8; 10] =
        [0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01];
    /// `0xff, 0xff, 0xff, 0xff, 0x0f` — the widest signer index that still fits
    /// the `u32` committee index.
    const U32_MAX_VARINT: [u8; 5] = [0xff, 0xff, 0xff, 0xff, 0x0f];

    /// The parser never looks inside a payload or a signature, so synthetic
    /// blobs fill both with a repeated tag byte and stay legal.
    fn proposal(epoch: &[u8], view: &[u8], parent: &[u8], payload_fill: u8) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(epoch);
        out.extend_from_slice(view);
        out.extend_from_slice(parent);
        out.extend_from_slice(&[payload_fill; PROPOSAL_PAYLOAD_LENGTH]);
        out
    }

    fn attestation(signer: &[u8], sig_fill: u8) -> Vec<u8> {
        let mut out = signer.to_vec();
        out.extend_from_slice(&[sig_fill; BLS_SIGNATURE_LENGTH]);
        out
    }

    fn run(blob: &[u8], shape: EvidenceShape) -> Result<DecodedEvidence, (ExitCode, Vec<u8>)> {
        let mut sdk = TestingContextImpl::default();
        decode(&mut sdk, &Bytes::copy_from_slice(blob), shape)
            .map_err(|code| (code, sdk.take_output()))
    }

    fn assert_reverts(blob: &[u8], shape: EvidenceShape, selector: u32) {
        let (code, output) = run(blob, shape).unwrap_err();
        assert_eq!(code, ExitCode::Panic);
        assert!(output.len() >= SIG_LEN_BYTES, "revert payload {output:?}");
        assert_eq!(&output[..SIG_LEN_BYTES], &selector.to_be_bytes());
    }

    #[test]
    fn conflicting_notarize_corpus_decodes_to_pinned_fields() {
        let got = run(&CONFLICTING_NOTARIZE, EvidenceShape::ConflictingNotarize).unwrap();
        assert_eq!(got.epoch, 7);
        assert_eq!(got.kind1, EVIDENCE_MESSAGE_KIND_NOTARIZE);
        assert_eq!(got.kind2, EVIDENCE_MESSAGE_KIND_NOTARIZE);
        assert_eq!(
            got.msg1[..],
            hex!("072a29aa000000000000000000000000000000000000000000000000000000000000aa")[..]
        );
        assert_eq!(
            got.sig1[..],
            hex!(
                "8aa1d24f195fc333878b14744f62a363acf0051249c949c4cc473850991aa708
                 41eea2171a333b13de2e61fed4936305"
            )[..]
        );
        assert_eq!(
            got.msg2[..],
            hex!("072a29bb000000000000000000000000000000000000000000000000000000000000bb")[..]
        );
        assert_eq!(
            got.sig2[..],
            hex!(
                "923c9abd2f0abe63eed5a2d9ac175032b2b48685c61f9e6a7c8b7419d7807782
                 1d82a3bfd41a5f10bcfcd8434444f820"
            )[..]
        );
    }

    #[test]
    fn conflicting_finalize_corpus_decodes_to_pinned_fields() {
        let got = run(&CONFLICTING_FINALIZE, EvidenceShape::ConflictingFinalize).unwrap();
        assert_eq!(got.epoch, 7);
        assert_eq!(got.kind1, EVIDENCE_MESSAGE_KIND_FINALIZE);
        assert_eq!(got.kind2, EVIDENCE_MESSAGE_KIND_FINALIZE);
        assert_eq!(
            got.msg1[..],
            hex!("072a29cc000000000000000000000000000000000000000000000000000000000000cc")[..]
        );
        assert_eq!(
            got.sig1[..],
            hex!(
                "9936ff0962301d36721c6d9e7947ec8a340bb9b5b7fcfa74ba2582918c9b3358
                 b31c15c2a8ae372f3340e8c7706d32a6"
            )[..]
        );
        assert_eq!(
            got.msg2[..],
            hex!("072a29dd000000000000000000000000000000000000000000000000000000000000dd")[..]
        );
        assert_eq!(
            got.sig2[..],
            hex!(
                "877570329a653f6cf0916cd5332247cd29a73d60a867dc1d710d5fe1bb4449b1
                 e9393d5f9aed23bb08a2f9aed0e65af2"
            )[..]
        );
    }

    #[test]
    fn nullify_finalize_corpus_decodes_to_pinned_fields() {
        let got = run(&NULLIFY_FINALIZE, EvidenceShape::NullifyFinalize).unwrap();
        assert_eq!(got.epoch, 7);
        assert_eq!(got.kind1, EVIDENCE_MESSAGE_KIND_NULLIFY);
        assert_eq!(got.kind2, EVIDENCE_MESSAGE_KIND_FINALIZE);
        // A nullify signs the bare round, so `msg1` is two bytes and carries no
        // proposal at all — the property the whole shape hangs on.
        assert_eq!(got.msg1[..], hex!("072a")[..]);
        assert_eq!(
            got.sig1[..],
            hex!(
                "b9d1ed34ffda9193ce95eee9ab8db558f4e923a1b58a6f80ca0bf221f7567f72
                 d65132b103190fd5c687f7f7a6cdc3db"
            )[..]
        );
        assert_eq!(
            got.msg2[..],
            hex!("072a29ee000000000000000000000000000000000000000000000000000000000000ee")[..]
        );
        assert_eq!(
            got.sig2[..],
            hex!(
                "89eae226e709054f09892935d13772a1e73b62a8bfbd24e5f1f63617c5242714
                 166f49b52ca5d97475b81820f75a6161"
            )[..]
        );
    }

    #[test]
    fn identical_proposals_revert() {
        let mut blob = CONFLICTING_NOTARIZE;
        blob.copy_within(0..35, 84);
        assert_reverts(
            &blob,
            EvidenceShape::ConflictingNotarize,
            ERR_EVIDENCE_PROPOSALS_IDENTICAL,
        );
    }

    #[test]
    fn conflicting_signer_mismatch_reverts() {
        for (blob, shape) in [
            (CONFLICTING_NOTARIZE, EvidenceShape::ConflictingNotarize),
            (CONFLICTING_FINALIZE, EvidenceShape::ConflictingFinalize),
        ] {
            let mut blob = blob;
            blob[119] += 1;
            assert_reverts(&blob, shape, ERR_EVIDENCE_SIGNER_MISMATCH);
        }
    }

    #[test]
    fn nullify_finalize_signer_mismatch_reverts() {
        let mut blob = NULLIFY_FINALIZE;
        blob[86] += 1;
        assert_reverts(
            &blob,
            EvidenceShape::NullifyFinalize,
            ERR_EVIDENCE_SIGNER_MISMATCH,
        );
    }

    #[test]
    fn conflicting_round_mismatch_reverts() {
        for (blob, shape) in [
            (CONFLICTING_NOTARIZE, EvidenceShape::ConflictingNotarize),
            (CONFLICTING_FINALIZE, EvidenceShape::ConflictingFinalize),
        ] {
            let mut blob = blob;
            blob[84] += 1;
            assert_reverts(&blob, shape, ERR_EVIDENCE_ROUND_MISMATCH);
        }
    }

    #[test]
    fn nullify_finalize_round_mismatch_reverts() {
        // Offset 0 is the nullified round's epoch. Offset 2 — which the Solidity
        // suite edits — is the first signer index and reverts for the wrong
        // reason.
        let mut blob = NULLIFY_FINALIZE;
        blob[0] += 1;
        assert_reverts(
            &blob,
            EvidenceShape::NullifyFinalize,
            ERR_EVIDENCE_ROUND_MISMATCH,
        );
    }

    #[test]
    fn every_proper_prefix_reverts() {
        for (blob, shape) in [
            (
                &CONFLICTING_NOTARIZE[..],
                EvidenceShape::ConflictingNotarize,
            ),
            (
                &CONFLICTING_FINALIZE[..],
                EvidenceShape::ConflictingFinalize,
            ),
            (&NULLIFY_FINALIZE[..], EvidenceShape::NullifyFinalize),
        ] {
            for len in 0..blob.len() {
                let (code, output) = run(&blob[..len], shape).unwrap_err();
                assert_eq!(code, ExitCode::Panic, "prefix of {len}");
                assert_eq!(
                    &output[..SIG_LEN_BYTES],
                    &ERR_INVALID_EVIDENCE_ENCODING.to_be_bytes(),
                    "prefix of {len}"
                );
            }
        }
    }

    #[test]
    fn trailing_byte_reverts() {
        for (blob, shape) in [
            (
                &CONFLICTING_NOTARIZE[..],
                EvidenceShape::ConflictingNotarize,
            ),
            (
                &CONFLICTING_FINALIZE[..],
                EvidenceShape::ConflictingFinalize,
            ),
            (&NULLIFY_FINALIZE[..], EvidenceShape::NullifyFinalize),
        ] {
            let mut extended = blob.to_vec();
            extended.push(0);
            assert_reverts(&extended, shape, ERR_INVALID_EVIDENCE_ENCODING);
        }
    }

    #[test]
    fn eleven_byte_varint_reverts() {
        let mut blob = vec![0x80u8; 10];
        blob.push(0x00);
        blob.extend_from_slice(&CONFLICTING_NOTARIZE[1..]);
        assert_reverts(
            &blob,
            EvidenceShape::ConflictingNotarize,
            ERR_INVALID_EVIDENCE_ENCODING,
        );
    }

    #[test]
    fn varint_above_u64_max_reverts() {
        let mut blob = vec![0x80u8; 9];
        blob.push(0x02);
        blob.extend_from_slice(&CONFLICTING_NOTARIZE[1..]);
        assert_reverts(
            &blob,
            EvidenceShape::ConflictingNotarize,
            ERR_INVALID_EVIDENCE_ENCODING,
        );
    }

    #[test]
    fn signer_index_above_u32_max_reverts() {
        let mut blob = CONFLICTING_NOTARIZE[..35].to_vec();
        // 2^32, one past the widest signer index the committee array can hold.
        blob.extend_from_slice(&[0x80, 0x80, 0x80, 0x80, 0x10]);
        blob.extend_from_slice(&CONFLICTING_NOTARIZE[36..]);
        assert_reverts(
            &blob,
            EvidenceShape::ConflictingNotarize,
            ERR_INVALID_EVIDENCE_ENCODING,
        );
    }

    #[test]
    fn padded_varint_is_accepted() {
        // `0x87 0x00` is a non-minimal encoding of 7. The node's codec never
        // emits one; the decoder this replaces took it, and a future tightening
        // has to change this test on purpose.
        let mut blob = vec![0x87u8, 0x00];
        blob.extend_from_slice(&CONFLICTING_NOTARIZE[1..]);
        let got = run(&blob, EvidenceShape::ConflictingNotarize).unwrap();
        assert_eq!(got.epoch, 7);
        assert_eq!(
            got.msg1[..],
            hex!("87002a29aa000000000000000000000000000000000000000000000000000000000000aa")[..]
        );
    }

    #[test]
    fn multi_byte_varint_fields_decode_and_widen_the_signed_span() {
        // Every corpus field is one byte and the padded case's second byte is
        // zero, so no existing test can see what a continuation byte adds.
        // `0xc1 0x02` = 321 and `0x81 0x03` = 385 both put non-zero bits in the
        // second seven-bit group: a wrong shift or mask lands on another number.
        let first = proposal(&[0xc1, 0x02], &[0x81, 0x03], &[0x29], 0xaa);
        let second = proposal(&[0xc1, 0x02], &[0x81, 0x03], &[0x29], 0xbb);
        let blob = [
            first.clone(),
            attestation(&[0x03], 0x11),
            second.clone(),
            attestation(&[0x03], 0x22),
        ]
        .concat();

        let got = run(&blob, EvidenceShape::ConflictingNotarize).unwrap();
        assert_eq!(got.epoch, 321);
        assert_eq!(got.msg1.len(), 37);
        assert_eq!(got.msg1[..], first[..]);
        assert_eq!(got.msg2[..], second[..]);
        assert_eq!(got.sig1[..], [0x11u8; BLS_SIGNATURE_LENGTH][..]);
        assert_eq!(got.sig2[..], [0x22u8; BLS_SIGNATURE_LENGTH][..]);
    }

    #[test]
    fn conflicting_same_epoch_different_view_reverts() {
        // Notarizing proposal A in view 42 and proposal B in view 43 after a
        // view change is what an honest validator does. Only the view separates
        // it from a double vote, so the round check has to compare both halves.
        for shape in [
            EvidenceShape::ConflictingNotarize,
            EvidenceShape::ConflictingFinalize,
        ] {
            let blob = [
                proposal(&[0x07], &[0x2a], &[0x29], 0xaa),
                attestation(&[0x03], 0x11),
                proposal(&[0x07], &[0x2b], &[0x29], 0xbb),
                attestation(&[0x03], 0x22),
            ]
            .concat();
            assert_reverts(&blob, shape, ERR_EVIDENCE_ROUND_MISMATCH);
        }
    }

    #[test]
    fn nullify_finalize_same_epoch_different_view_reverts() {
        // Nullifying view 42 and finalizing view 43 in the same epoch is two
        // honest votes on two different rounds.
        let blob = [
            vec![0x07, 0x2a],
            attestation(&[0x03], 0x11),
            proposal(&[0x07], &[0x2b], &[0x29], 0xee),
            attestation(&[0x03], 0x22),
        ]
        .concat();
        assert_reverts(
            &blob,
            EvidenceShape::NullifyFinalize,
            ERR_EVIDENCE_ROUND_MISMATCH,
        );
    }

    #[test]
    fn widest_u64_varint_is_accepted() {
        let blob = [
            proposal(&U64_MAX_VARINT, &[0x2a], &[0x29], 0xaa),
            attestation(&[0x03], 0x11),
            proposal(&U64_MAX_VARINT, &[0x2a], &[0x29], 0xbb),
            attestation(&[0x03], 0x22),
        ]
        .concat();
        let got = run(&blob, EvidenceShape::ConflictingNotarize).unwrap();
        assert_eq!(got.epoch, u64::MAX);
    }

    #[test]
    fn ten_byte_varint_carrying_only_bit_63_is_accepted() {
        let blob = [
            proposal(&TWO_POW_63_VARINT, &[0x2a], &[0x29], 0xaa),
            attestation(&[0x03], 0x11),
            proposal(&TWO_POW_63_VARINT, &[0x2a], &[0x29], 0xbb),
            attestation(&[0x03], 0x22),
        ]
        .concat();
        let got = run(&blob, EvidenceShape::ConflictingNotarize).unwrap();
        assert_eq!(got.epoch, 1u64 << 63);
    }

    #[test]
    fn widest_signer_index_is_accepted() {
        // The decoded evidence no longer carries the signer index — it is
        // outside both signed spans, so it proves nothing the two verifies do
        // not already prove. The parser still reads it to advance the cursor
        // and still bounds it to `u32`, so what this pins is that the widest
        // in-range index parses rather than tripping that bound.
        let blob = [
            proposal(&[0x07], &[0x2a], &[0x29], 0xaa),
            attestation(&U32_MAX_VARINT, 0x11),
            proposal(&[0x07], &[0x2a], &[0x29], 0xbb),
            attestation(&U32_MAX_VARINT, 0x22),
        ]
        .concat();
        let got = run(&blob, EvidenceShape::ConflictingNotarize).unwrap();
        assert_eq!(got.epoch, 7);
        assert_eq!(got.sig1[..], [0x11u8; BLS_SIGNATURE_LENGTH][..]);
        assert_eq!(got.sig2[..], [0x22u8; BLS_SIGNATURE_LENGTH][..]);
    }

    #[test]
    fn message_kinds_match_the_node_wire_numbers() {
        // These numbers pick the BLS domain separator the verifier hashes under,
        // and the node fixture pins them as literals. Renumbering compiles, keeps
        // every decode test green, and makes every honest slash fail signature
        // verification instead — so pin the numbers, not the constants.
        assert_eq!(EVIDENCE_MESSAGE_KIND_NOTARIZE, 0);
        assert_eq!(EVIDENCE_MESSAGE_KIND_NULLIFY, 1);
        assert_eq!(EVIDENCE_MESSAGE_KIND_FINALIZE, 2);
    }
}
