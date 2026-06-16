//! Slashing-evidence bridge: convert Simplex `Activity::Conflicting*`
//! events into the four `slashEquivocation*` calldata arguments expected
//! by `Staking.sol`.
//!
//! This is a **stateless** module — every helper takes the evidence
//! struct + a per-epoch committee `BiMap` and returns the four bytes
//! the on-chain `Staking.slashEquivocation*` entry points expect.
//!
//! # Pipeline overview
//!
//! ```text
//!   commonware Activity::Conflicting* (from simplex batcher)
//!     │  evidence = ev.encode()                      (Commonware codec)
//!     │  signer_idx = ev.signer() (via Attributable trait)
//!     │  re-decode inner Notarize/Finalize/Nullify for sig access
//!     │    sig_g1 = .attestation.signature.get()?     (force lazy decode)
//!     │    sig_compressed = sig_g1.encode() as [u8; 48]
//!     │  pk_compressed = committee.value(signer_idx)?.encode() as [u8; 96]
//!     │  EIP-2537 conversions (signature_compressed_to_eip2537 +
//!     │                        pubkey_compressed_to_eip2537)
//!     ▼
//!   SlashCallArgs { kind, evidence (raw), pk_uncompressed (256B),
//!                   sig1_uncompressed (128B), sig2_uncompressed (128B) }
//!     │
//!     ▼
//!   ABI-encode + submit `slashEquivocation*(evidence, pkUnc, sig1Unc, sig2Unc)`
//!   (caller responsibility — slasher worker)
//! ```
//!
//! ## Why re-decode instead of accessor methods
//!
//! Commonware's `ConflictingNotarize<S, D>`, `ConflictingFinalize<S, D>`,
//! and `NullifyFinalize<S, D>` have **private** inner fields and provide
//! no accessor methods (only `Attributable::signer` / `Epochable::epoch`
//! / `Viewable::view` from traits — none expose the inner `Notarize` /
//! `Finalize` / `Nullify` values). The wire format `Write`/`Read` impls
//! ARE public, so we round-trip through `.encode()` →
//! `Notarize::read_cfg` to access the inner sig material. One extra
//! in-memory codec pass per evidence (~168 B) — equivocation events are
//! rare, so cost is irrelevant.

use commonware_codec::{Encode, Read as _};
use commonware_consensus::{
    simplex::types::{
        Activity, Attributable, ConflictingFinalize, ConflictingNotarize, Finalize, Notarize,
        Nullify, NullifyFinalize,
    },
    Epochable,
};
use commonware_cryptography::{certificate, Digest as DigestTrait};
use commonware_parallel::Sequential;
use commonware_utils::ordered::BiMap;
use fluentbase_bls::{
    encoding::{pubkey_compressed_to_eip2537, signature_compressed_to_eip2537},
    BlsPubkey, EpochCommittee, Error, PeerPubkey, Scheme, PUBKEY_BYTES, PUBKEY_EIP2537_BYTES,
    SIGNATURE_BYTES, SIGNATURE_EIP2537_BYTES,
};
use rand_core::CryptoRngCore;

/// Discriminator for the three `slashEquivocation*` entry points.
///
/// Pairs 1:1 with the `Staking.sol` functions: `ConflictingNotarize` →
/// `slashEquivocationNotarize`, `ConflictingFinalize` →
/// `slashEquivocationFinalize`, `NullifyFinalize` →
/// `slashEquivocationNullifyFinalize`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlashKind {
    ConflictingNotarize,
    ConflictingFinalize,
    NullifyFinalize,
}

impl SlashKind {
    /// Filter an `Activity` event to its `SlashKind`, or `None` if not a
    /// slashable variant.
    ///
    /// Useful for matching out evidence from a generic Activity stream:
    ///
    /// ```ignore
    /// if let Some(kind) = SlashKind::from_activity(&activity) {
    ///     // dispatch to extract_from_* + ABI-encode + submit
    /// }
    /// ```
    pub fn from_activity<S, D>(activity: &Activity<S, D>) -> Option<Self>
    where
        S: certificate::Scheme,
        D: DigestTrait,
    {
        match activity {
            Activity::ConflictingNotarize(_) => Some(Self::ConflictingNotarize),
            Activity::ConflictingFinalize(_) => Some(Self::ConflictingFinalize),
            Activity::NullifyFinalize(_) => Some(Self::NullifyFinalize),
            _ => None,
        }
    }
}

/// The four byte-shaped arguments expected by the on-chain
/// `slashEquivocation*` entry points.
///
/// - `evidence`: raw Commonware-encoded `ConflictingNotarize` /
///   `ConflictingFinalize` / `NullifyFinalize` bytes (variant-length;
///   ~135–168 B for the standard 32-B digest case).
/// - `pk_uncompressed`: offender's BLS12-381 G2 public key in EIP-2537
///   uncompressed format (256 B).
/// - `sig1_uncompressed` / `sig2_uncompressed`: the two BLS12-381 G1
///   signatures in EIP-2537 uncompressed format (128 B each).
#[derive(Clone, Debug)]
pub struct SlashCallArgs {
    pub kind: SlashKind,
    pub evidence: Vec<u8>,
    pub pk_uncompressed: [u8; PUBKEY_EIP2537_BYTES],
    pub sig1_uncompressed: [u8; SIGNATURE_EIP2537_BYTES],
    pub sig2_uncompressed: [u8; SIGNATURE_EIP2537_BYTES],
}

/// Encode an `S::Signature` (= G1 compressed for MinSig) into the fixed
/// 48-byte buffer the contract expects.
fn sig_compressed<S: certificate::Scheme>(
    sig: &S::Signature,
) -> Result<[u8; SIGNATURE_BYTES], Error> {
    sig.encode()
        .as_ref()
        .try_into()
        .map_err(|_| Error::InvalidSignature)
}

/// Look up the offender's `BlsPubkey` (96 B G2 compressed) by
/// `signer_idx`. The index must be within `committee.len()`.
fn pk_compressed(
    signer_idx: u32,
    committee: &BiMap<PeerPubkey, BlsPubkey>,
) -> Result<[u8; PUBKEY_BYTES], Error> {
    let bls_pk: &BlsPubkey =
        committee
            .value(signer_idx as usize)
            .ok_or(Error::SignerIndexOutOfRange {
                signer_idx,
                committee_len: committee.len(),
            })?;
    bls_pk
        .encode()
        .as_ref()
        .try_into()
        .map_err(|_| Error::InvalidPubkey)
}

/// Assert the activity's claimed epoch matches the committee's epoch
/// before extracting — protects against passing a mis-aligned committee
/// to the per-variant extractors.
#[inline]
fn check_epoch_match(ev: &impl Epochable, committee: &EpochCommittee) -> Result<(), Error> {
    let evidence_epoch = ev.epoch().get();
    if evidence_epoch != committee.epoch {
        return Err(Error::EpochMismatch {
            evidence_epoch,
            committee_epoch: committee.epoch,
        });
    }
    Ok(())
}

/// Build `slashEquivocationNotarize` calldata args from a
/// `ConflictingNotarize` event.
pub fn extract_from_conflicting_notarize<D>(
    ev: &ConflictingNotarize<Scheme, D>,
    committee: &EpochCommittee,
) -> Result<SlashCallArgs, Error>
where
    D: DigestTrait,
{
    check_epoch_match(ev, committee)?;
    let evidence = ev.encode().to_vec();

    // Re-decode the two inner Notarize structs (Conflicting* fields are
    // private; Write/Read traits are public — round-trip is the only
    // path to the inner sig material).
    let mut buf: &[u8] = &evidence;
    let n1 = Notarize::<Scheme, D>::read_cfg(&mut buf, &()).map_err(|_| Error::InvalidSignature)?;
    let n2 = Notarize::<Scheme, D>::read_cfg(&mut buf, &()).map_err(|_| Error::InvalidSignature)?;

    // Re-establish the equivocation structural invariant before paying gas
    // (mirrors commonware `ConflictingNotarize::new`/`read_cfg`, which `verify`
    // does NOT re-check). Defends against a future non-`read_cfg` ingress (e.g.
    // `Arbitrary`/direct construction) slashing an honest validator. Compare
    // `round` (epoch+view), not just view.
    if n1.signer() != n2.signer() || n1.round() != n2.round() || n1.proposal == n2.proposal {
        return Err(Error::NonConflictingEvidence);
    }

    let sig1_g1 = n1
        .attestation
        .signature
        .get()
        .ok_or(Error::InvalidSignature)?;
    let sig2_g1 = n2
        .attestation
        .signature
        .get()
        .ok_or(Error::InvalidSignature)?;

    let sig1 = sig_compressed::<Scheme>(sig1_g1)?;
    let sig2 = sig_compressed::<Scheme>(sig2_g1)?;
    let pk96 = pk_compressed(ev.signer().get(), &committee.bimap)?;

    Ok(SlashCallArgs {
        kind: SlashKind::ConflictingNotarize,
        evidence,
        pk_uncompressed: pubkey_compressed_to_eip2537(&pk96)?,
        sig1_uncompressed: signature_compressed_to_eip2537(&sig1)?,
        sig2_uncompressed: signature_compressed_to_eip2537(&sig2)?,
    })
}

/// Build `slashEquivocationFinalize` calldata args from a
/// `ConflictingFinalize` event.
pub fn extract_from_conflicting_finalize<D>(
    ev: &ConflictingFinalize<Scheme, D>,
    committee: &EpochCommittee,
) -> Result<SlashCallArgs, Error>
where
    D: DigestTrait,
{
    check_epoch_match(ev, committee)?;
    let evidence = ev.encode().to_vec();

    let mut buf: &[u8] = &evidence;
    let f1 = Finalize::<Scheme, D>::read_cfg(&mut buf, &()).map_err(|_| Error::InvalidSignature)?;
    let f2 = Finalize::<Scheme, D>::read_cfg(&mut buf, &()).map_err(|_| Error::InvalidSignature)?;

    // Structural invariant (mirrors `ConflictingFinalize::new`/`read_cfg`).
    if f1.signer() != f2.signer() || f1.round() != f2.round() || f1.proposal == f2.proposal {
        return Err(Error::NonConflictingEvidence);
    }

    let sig1_g1 = f1
        .attestation
        .signature
        .get()
        .ok_or(Error::InvalidSignature)?;
    let sig2_g1 = f2
        .attestation
        .signature
        .get()
        .ok_or(Error::InvalidSignature)?;

    let sig1 = sig_compressed::<Scheme>(sig1_g1)?;
    let sig2 = sig_compressed::<Scheme>(sig2_g1)?;
    let pk96 = pk_compressed(ev.signer().get(), &committee.bimap)?;

    Ok(SlashCallArgs {
        kind: SlashKind::ConflictingFinalize,
        evidence,
        pk_uncompressed: pubkey_compressed_to_eip2537(&pk96)?,
        sig1_uncompressed: signature_compressed_to_eip2537(&sig1)?,
        sig2_uncompressed: signature_compressed_to_eip2537(&sig2)?,
    })
}

/// Build `slashEquivocationNullifyFinalize` calldata args from a
/// `NullifyFinalize` event.
pub fn extract_from_nullify_finalize<D>(
    ev: &NullifyFinalize<Scheme, D>,
    committee: &EpochCommittee,
) -> Result<SlashCallArgs, Error>
where
    D: DigestTrait,
{
    check_epoch_match(ev, committee)?;
    let evidence = ev.encode().to_vec();

    let mut buf: &[u8] = &evidence;
    let nullify =
        Nullify::<Scheme>::read_cfg(&mut buf, &()).map_err(|_| Error::InvalidSignature)?;
    let finalize =
        Finalize::<Scheme, D>::read_cfg(&mut buf, &()).map_err(|_| Error::InvalidSignature)?;

    // Structural invariant (mirrors `NullifyFinalize::new`/`read_cfg`): same
    // signer + same round. NO proposals-differ check — a Nullify has no
    // proposal; adding one would over-reject valid evidence.
    if nullify.signer() != finalize.signer() || nullify.round != finalize.round() {
        return Err(Error::NonConflictingEvidence);
    }

    let sig1_g1 = nullify
        .attestation
        .signature
        .get()
        .ok_or(Error::InvalidSignature)?;
    let sig2_g1 = finalize
        .attestation
        .signature
        .get()
        .ok_or(Error::InvalidSignature)?;

    let sig1 = sig_compressed::<Scheme>(sig1_g1)?;
    let sig2 = sig_compressed::<Scheme>(sig2_g1)?;
    let pk96 = pk_compressed(ev.signer().get(), &committee.bimap)?;

    Ok(SlashCallArgs {
        kind: SlashKind::NullifyFinalize,
        evidence,
        pk_uncompressed: pubkey_compressed_to_eip2537(&pk96)?,
        sig1_uncompressed: signature_compressed_to_eip2537(&sig1)?,
        sig2_uncompressed: signature_compressed_to_eip2537(&sig2)?,
    })
}

/// Local cryptographic verify of an `Activity` event before submitting it
/// on-chain.
///
/// Wraps `Activity::verify` with `Sequential` strategy (one Activity =
/// two pairings; fork-join overhead > benefit). Defends against local
/// memory corruption / bit-flip before paying gas on-chain.
///
/// Does NOT verify equivocation invariants (`round_1 == round_2 &&
/// signer_1 == signer_2 && proposal_1 != proposal_2` — already enforced
/// by `Conflicting*::new` and `Read::read_cfg`) and does NOT verify
/// on-chain state (validator registration, tombstone) — those are
/// Solidity concerns.
pub fn verify_pre_submit<D, R>(
    activity: &Activity<Scheme, D>,
    scheme: &Scheme,
    rng: &mut R,
) -> Result<(), Error>
where
    D: DigestTrait,
    R: CryptoRngCore,
{
    if activity.verify(rng, scheme, &Sequential) {
        Ok(())
    } else {
        Err(Error::InvalidSignature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_codec::DecodeExt as _;
    use commonware_consensus::{
        simplex::types::Proposal,
        types::{Epoch, Round, View},
    };
    use commonware_cryptography::{
        ed25519::PrivateKey as Ed25519PrivateKey, sha256::Digest as Sha256Digest, Signer,
    };
    use commonware_math::algebra::Random;
    use commonware_utils::TryCollect;
    use fluentbase_bls::{fluent_namespace, keys::ValidatorBlsKeypair, scheme::build_signer};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;

    const TEST_CHAIN_ID: u64 = 20_994;

    fn small_committee(
        seed: u64,
        n: usize,
    ) -> (Vec<ValidatorBlsKeypair>, BiMap<PeerPubkey, BlsPubkey>) {
        let mut rng = StdRng::seed_from_u64(seed);
        let peer_sks: Vec<_> = (0..n)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let bls_kps: Vec<_> = (0..n)
            .map(|_| ValidatorBlsKeypair::generate(&mut rng))
            .collect();
        let bimap: BiMap<_, _> = peer_sks
            .iter()
            .zip(bls_kps.iter())
            .map(|(p, b)| {
                (
                    p.public_key(),
                    BlsPubkey::decode(b.public_bytes().as_slice()).unwrap(),
                )
            })
            .try_collect()
            .unwrap();
        (bls_kps, bimap)
    }

    fn digest(tag: u8) -> Sha256Digest {
        let mut d = [0u8; 32];
        d[0] = tag;
        d[31] = tag;
        Sha256Digest::from(d)
    }

    fn round() -> Round {
        Round::new(Epoch::new(7), View::new(42))
    }

    fn build_ev_conflicting_notarize(
        seed: u64,
    ) -> (
        ConflictingNotarize<Scheme, Sha256Digest>,
        BiMap<PeerPubkey, BlsPubkey>,
    ) {
        let (kps, bimap) = small_committee(seed, 4);
        let s = build_signer(
            &fluent_namespace(TEST_CHAIN_ID),
            bimap.clone(),
            &kps[0],
            None,
        )
        .expect("offender must be member");
        let p1 = Proposal::new(round(), View::new(41), digest(0xaa));
        let p2 = Proposal::new(round(), View::new(41), digest(0xbb));
        let n1 = Notarize::sign(&s, p1).expect("sign n1");
        let n2 = Notarize::sign(&s, p2).expect("sign n2");
        (ConflictingNotarize::new(n1, n2), bimap)
    }

    #[test]
    fn from_activity_returns_none_for_non_conflicting_variants() {
        let (ev, _) = build_ev_conflicting_notarize(1);
        // Conflicting variant — must produce Some
        let conflicting = Activity::<Scheme, Sha256Digest>::ConflictingNotarize(ev);
        assert_eq!(
            SlashKind::from_activity(&conflicting),
            Some(SlashKind::ConflictingNotarize)
        );

        // Non-conflicting Notarize — must produce None
        let (kps, bimap) = small_committee(2, 4);
        let s = build_signer(&fluent_namespace(TEST_CHAIN_ID), bimap, &kps[0], None).unwrap();
        let n = Notarize::sign(&s, Proposal::new(round(), View::new(41), digest(0xcc))).unwrap();
        let plain = Activity::<Scheme, Sha256Digest>::Notarize(n);
        assert_eq!(SlashKind::from_activity(&plain), None);
    }

    #[test]
    fn extract_returns_signer_index_out_of_range_for_empty_committee() {
        let (ev, _) = build_ev_conflicting_notarize(1);
        // Empty BiMap wrapped in an EpochCommittee with the same epoch
        // as the event (7) so the SignerIndexOutOfRange path is reached.
        let empty_committee = EpochCommittee::from_unverified(7, BiMap::default());
        let err = extract_from_conflicting_notarize(&ev, &empty_committee)
            .expect_err("must reject signer_idx >= empty bimap");
        assert!(
            matches!(
                err,
                Error::SignerIndexOutOfRange {
                    committee_len: 0,
                    ..
                }
            ),
            "got: {err:?}"
        );
    }

    #[test]
    fn extract_rejects_epoch_mismatch() {
        // Event has epoch=7 (per `round()` helper); pass an EpochCommittee
        // with epoch=8 → must short-circuit with EpochMismatch BEFORE the
        // signer-index lookup is even attempted.
        let (ev, bimap) = build_ev_conflicting_notarize(1);
        let wrong_epoch_committee = EpochCommittee::from_unverified(8, bimap);
        let err = extract_from_conflicting_notarize(&ev, &wrong_epoch_committee)
            .expect_err("must reject committee from wrong epoch");
        assert!(
            matches!(
                err,
                Error::EpochMismatch {
                    evidence_epoch: 7,
                    committee_epoch: 8
                }
            ),
            "got: {err:?}"
        );
    }

    #[test]
    fn verify_pre_submit_rejects_tampered_signature() {
        let (ev, bimap) = build_ev_conflicting_notarize(1);
        let activity = Activity::<Scheme, Sha256Digest>::ConflictingNotarize(ev);

        // Build a verifier scheme over the same committee so the activity
        // is verifiable in principle.
        let scheme =
            fluentbase_bls::scheme::build_verifier(&fluent_namespace(TEST_CHAIN_ID), bimap, None);
        let mut rng = StdRng::seed_from_u64(0xdeadbeef);

        // Sanity: clean activity must verify.
        verify_pre_submit(&activity, &scheme, &mut rng).expect("clean activity must verify");

        // Tamper: re-encode, flip a byte deep inside sig1's 48-byte body,
        // re-decode. Flipping a body byte (offset 50 = bit 14 of x-coord
        // in sig1) keeps Read invariants (signer/round unchanged) intact
        // but fails verification at .get() blst decode or pairing check.
        let bytes = match &activity {
            Activity::ConflictingNotarize(ev) => ev.encode().to_vec(),
            _ => unreachable!(),
        };
        let mut tampered = bytes.clone();
        tampered[50] ^= 0x01;
        let tampered_ev: ConflictingNotarize<Scheme, Sha256Digest> =
            ConflictingNotarize::decode(tampered.as_slice())
                .expect("tampering a sig body byte must keep Read invariants intact");
        let tampered_activity = Activity::<Scheme, Sha256Digest>::ConflictingNotarize(tampered_ev);

        let err = verify_pre_submit(&tampered_activity, &scheme, &mut rng)
            .expect_err("tampered activity must fail verify");
        assert!(matches!(err, Error::InvalidSignature), "got: {err:?}");
    }

    #[test]
    fn extract_returns_signer_index_out_of_range_for_short_committee() {
        // `build_ev_conflicting_notarize(1)` (= committee seed 1, OFFENDER=0)
        // is the same recipe as `equivocation_evidence_conformance.rs`'s
        // `committee(1)` + `OFFENDER=0`. Its EXPECTED corpus pins
        // `signer_idx = 3` (BiMap-sorted, see conformance pin EXPECTED).
        let (ev, _) = build_ev_conflicting_notarize(1);

        // 2-validator bimap → signer_idx=3 is unconditionally out of range.
        // Peer-key identity doesn't matter for the lookup; only `signer_idx
        // < bimap.len()`. Different seed (99) keeps the test independent
        // of the offender's actual public key. Epoch is set to 7 to match
        // the event so the SignerIndexOutOfRange path is reached (not the
        // EpochMismatch short-circuit).
        let (_, short_bimap) = small_committee(99, 2);
        let short_committee = EpochCommittee::from_unverified(7, short_bimap);

        let err = extract_from_conflicting_notarize(&ev, &short_committee)
            .expect_err("must reject signer_idx=3 >= short_bimap.len()=2");
        assert!(
            matches!(
                err,
                Error::SignerIndexOutOfRange {
                    signer_idx: 3,
                    committee_len: 2,
                }
            ),
            "got: {err:?}"
        );
    }
}
