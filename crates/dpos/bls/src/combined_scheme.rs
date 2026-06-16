//! `CombinedScheme` — the attributable + threshold consensus scheme.
//!
//! Each consensus vote carries `(vote, seed)`: an attributable multisig share
//! (for finalization + equivocation slashing) AND a threshold seed partial over
//! the round (for the randomness beacon). The notarization/finalization
//! certificate therefore recovers a unique per-round seed as a byproduct of
//! consensus — no separate beacon plane.
//!
//! Composition: this holds the inner multisig [`crate::VoteScheme`] and
//! delegates the vote half to its `certificate::Scheme` methods, repackaging
//! `Attestation`/`Certificate` between the combined and vote-only forms. The
//! seed half uses [`crate::beacon`] (pure threshold ops over `round.encode()`).
//!
//! Dual mode: a scheme built WITH a per-epoch threshold share is beacon-active
//! (a real partial is REQUIRED on `Notarize`/`Finalize` votes — a vote without
//! it is invalid → Nullify → not counted → notarize quorum ⟺ ≥t partials);
//! WITHOUT a share it is fallback (`seed = None` everywhere → the deriver uses
//! the weak `order.digest()` randomness).
//! The signature is `CodecFixed`, so the optional seed is a FIXED slot (a
//! 1-byte present flag + a 48-byte G1 slot): `Nullify` votes and fallback
//! epochs carry `None`.

use bytes::{Buf, BufMut};
use commonware_codec::{EncodeSize, Error as CodecError, FixedSize, Read, ReadExt as _, Write};
use commonware_consensus::{simplex::types::Subject, types::Round};
use commonware_cryptography::{
    bls12381::primitives::{
        group::Share,
        sharing::Sharing,
        variant::{MinSig, PartialSignature},
    },
    certificate::{Attestation, Scheme as CertScheme},
    Digest,
};
use commonware_parallel::Strategy;
use commonware_utils::{ordered::Set, Faults, Participant};
use rand_core::CryptoRngCore;

use crate::{beacon, BlsSignature, PeerPubkey, VoteScheme};

type VoteCertificate = <VoteScheme as CertScheme>::Certificate;

/// Compressed-G1 byte length — the seed slot width.
const SEED_SLOT: usize = crate::SIGNATURE_BYTES;

/// The round a subject is scoped to (used as the seed message domain).
fn subject_round<D: Digest>(subject: &Subject<'_, D>) -> Round {
    match subject {
        Subject::Notarize { proposal } | Subject::Finalize { proposal } => proposal.round,
        Subject::Nullify { round } => *round,
    }
}

/// Whether a subject carries a real seed partial (Notarize/Finalize) vs none
/// (Nullify).
fn seeded_subject<D: Digest>(subject: &Subject<'_, D>) -> bool {
    matches!(subject, Subject::Notarize { .. } | Subject::Finalize { .. })
}

/// Encode an optional seed as a FIXED-size slot: a 1-byte present flag + a
/// 48-byte G1 slot (the signature when present, all-zero when absent). An
/// explicit flag — not a sentinel point — is REQUIRED because the BLS12-381 G1
/// identity is not a decodable point (`G1::read` rejects infinity), so a "no
/// seed" (Nullify / fallback-epoch) vote could not otherwise round-trip while
/// keeping the `CodecFixed` constant size.
fn write_seed_slot(seed: &Option<BlsSignature>, buf: &mut impl BufMut) {
    match seed {
        Some(s) => {
            1u8.write(buf);
            s.write(buf);
        }
        None => {
            0u8.write(buf);
            buf.put_slice(&[0u8; SEED_SLOT]);
        }
    }
}

fn read_seed_slot(buf: &mut impl Buf) -> Result<Option<BlsSignature>, CodecError> {
    let present = u8::read(buf)?;
    let raw = <[u8; SEED_SLOT]>::read(buf)?;
    match present {
        0 => Ok(None),
        1 => Ok(Some(BlsSignature::read(&mut raw.as_slice())?)),
        _ => Err(CodecError::Invalid(
            "CombinedSignature",
            "bad seed present flag",
        )),
    }
}

/// Per-vote signature: the attributable multisig share + the threshold seed
/// partial. FIXED 97 B (vote 48 ‖ flag 1 ‖ seed-slot 48); `seed = None` on a
/// Nullify vote or in a fallback (no-beacon) epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CombinedSignature {
    pub vote: BlsSignature,
    pub seed: Option<BlsSignature>,
}

impl CombinedSignature {
    /// The attributable multisig half — what the slasher extracts (48 B).
    pub fn vote(&self) -> &BlsSignature {
        &self.vote
    }
}

impl FixedSize for CombinedSignature {
    const SIZE: usize = crate::SIGNATURE_BYTES + 1 + SEED_SLOT;
}

impl Write for CombinedSignature {
    fn write(&self, buf: &mut impl BufMut) {
        self.vote.write(buf);
        write_seed_slot(&self.seed, buf);
    }
}

impl Read for CombinedSignature {
    type Cfg = ();
    fn read_cfg(buf: &mut impl Buf, _: &()) -> Result<Self, CodecError> {
        let vote = BlsSignature::read(buf)?;
        let seed = read_seed_slot(buf)?;
        Ok(Self { vote, seed })
    }
}

/// Certificate assembled from a quorum of [`CombinedSignature`]s: the
/// attributable multisig certificate (bitmap + aggregate vote) plus the
/// recovered threshold seed (`None` for a Nullify/fallback cert).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CombinedCertificate {
    pub vote: VoteCertificate,
    pub seed: Option<BlsSignature>,
}

impl CombinedCertificate {
    /// The recovered seed signature, or `None` when absent (Nullify/fallback).
    pub fn seed(&self) -> Option<BlsSignature> {
        self.seed
    }
}

impl Write for CombinedCertificate {
    fn write(&self, buf: &mut impl BufMut) {
        self.vote.write(buf);
        write_seed_slot(&self.seed, buf);
    }
}

impl EncodeSize for CombinedCertificate {
    fn encode_size(&self) -> usize {
        self.vote.encode_size() + 1 + SEED_SLOT
    }
}

impl Read for CombinedCertificate {
    type Cfg = usize;
    fn read_cfg(buf: &mut impl Buf, participants: &usize) -> Result<Self, CodecError> {
        let vote = VoteCertificate::read_cfg(buf, participants)?;
        let seed = read_seed_slot(buf)?;
        Ok(Self { vote, seed })
    }
}

/// The per-epoch threshold material a beacon-active scheme holds.
#[derive(Clone)]
struct BeaconPart {
    sharing: Sharing<MinSig>,
    share: Option<Share>,
    seed_namespace: Vec<u8>,
}

/// Combined attributable + threshold consensus scheme.
#[derive(Clone)]
pub struct CombinedScheme {
    vote: VoteScheme,
    beacon: Option<BeaconPart>,
}

impl core::fmt::Debug for CombinedScheme {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CombinedScheme")
            .field("vote", &self.vote)
            .field("beacon_active", &self.beacon.is_some())
            .finish()
    }
}

impl CombinedScheme {
    /// Build from an already-constructed vote scheme + optional beacon part.
    /// `beacon = None` ⇒ fallback (pure multisig, sentinel seed everywhere).
    /// For a signer, asserts the threshold `share.index` matches the consensus
    /// participant index (both Commonware-sorted) — a mismatch would
    /// mis-attribute a partial.
    pub(crate) fn new(
        vote: VoteScheme,
        beacon: Option<(Sharing<MinSig>, Option<Share>, Vec<u8>)>,
    ) -> Self {
        let beacon = beacon.map(|(sharing, share, seed_namespace)| {
            if let (Some(s), Some(me)) = (&share, vote.me()) {
                assert_eq!(
                    s.index, me,
                    "beacon share index != consensus participant index"
                );
            }
            BeaconPart {
                sharing,
                share,
                seed_namespace,
            }
        });
        Self { vote, beacon }
    }

    fn vote_attestation(att: &Attestation<Self>) -> Option<Attestation<VoteScheme>> {
        let combined = att.signature.get()?;
        Some(Attestation {
            signer: att.signer,
            signature: combined.vote.into(),
        })
    }
}

impl CertScheme for CombinedScheme {
    type Subject<'a, D: Digest> = Subject<'a, D>;
    type PublicKey = PeerPubkey;
    type Signature = CombinedSignature;
    type Certificate = CombinedCertificate;

    fn me(&self) -> Option<Participant> {
        self.vote.me()
    }

    fn participants(&self) -> &Set<Self::PublicKey> {
        self.vote.participants()
    }

    fn sign<D: Digest>(&self, subject: Self::Subject<'_, D>) -> Option<Attestation<Self>> {
        let round = subject_round(&subject);
        let vote_att = self.vote.sign::<D>(subject)?;
        let vote = *vote_att.signature.get()?;
        let seed = match (&self.beacon, seeded_subject(&subject)) {
            (Some(b), true) => {
                let share = b.share.as_ref()?;
                Some(beacon::sign_seed_partial(share, &b.seed_namespace, round).value)
            }
            _ => None,
        };
        Some(Attestation {
            signer: vote_att.signer,
            signature: CombinedSignature { vote, seed }.into(),
        })
    }

    fn verify_attestation<R, D>(
        &self,
        rng: &mut R,
        subject: Self::Subject<'_, D>,
        attestation: &Attestation<Self>,
        strategy: &impl Strategy,
    ) -> bool
    where
        R: CryptoRngCore,
        D: Digest,
    {
        let Some(vote_att) = Self::vote_attestation(attestation) else {
            return false;
        };
        let round = subject_round(&subject);
        if !self
            .vote
            .verify_attestation(rng, subject, &vote_att, strategy)
        {
            return false;
        }
        let Some(combined) = attestation.signature.get() else {
            return false;
        };
        match (&self.beacon, seeded_subject(&subject)) {
            // ACTIVE Notarize/Finalize: a missing or invalid seed partial makes
            // the whole vote invalid (→ Nullify → not counted toward quorum).
            (Some(b), true) => match combined.seed {
                Some(value) => beacon::verify_seed_partial(
                    &b.sharing,
                    &b.seed_namespace,
                    round,
                    &PartialSignature::<MinSig> {
                        index: attestation.signer,
                        value,
                    },
                ),
                None => false,
            },
            // Nullify, or a fallback epoch: the seed MUST be absent.
            _ => combined.seed.is_none(),
        }
    }

    fn assemble<I, M>(&self, attestations: I, strategy: &impl Strategy) -> Option<Self::Certificate>
    where
        I: IntoIterator<Item = Attestation<Self>>,
        I::IntoIter: Send,
        M: Faults,
    {
        let atts: Vec<_> = attestations.into_iter().collect();
        let vote_atts: Vec<Attestation<VoteScheme>> =
            atts.iter().filter_map(Self::vote_attestation).collect();
        let vote = self.vote.assemble::<_, M>(vote_atts, strategy)?;
        let seed = match &self.beacon {
            Some(b)
                if atts
                    .iter()
                    .all(|a| a.signature.get().is_some_and(|c| c.seed.is_some())) =>
            {
                let partials: Vec<PartialSignature<MinSig>> = atts
                    .iter()
                    .filter_map(|a| {
                        a.signature.get().and_then(|c| {
                            c.seed.map(|value| PartialSignature::<MinSig> {
                                index: a.signer,
                                value,
                            })
                        })
                    })
                    .collect();
                Some(beacon::recover_seed(&b.sharing, &partials).ok()?)
            }
            _ => None,
        };
        Some(CombinedCertificate { vote, seed })
    }

    fn verify_certificate<R, D, M>(
        &self,
        rng: &mut R,
        subject: Self::Subject<'_, D>,
        certificate: &Self::Certificate,
        strategy: &impl Strategy,
    ) -> bool
    where
        R: CryptoRngCore,
        D: Digest,
        M: Faults,
    {
        let round = subject_round(&subject);
        if !self
            .vote
            .verify_certificate::<_, _, M>(rng, subject, &certificate.vote, strategy)
        {
            return false;
        }
        match (&self.beacon, seeded_subject(&subject)) {
            (Some(b), true) => match &certificate.seed {
                Some(sig) => {
                    beacon::verify_seed(b.sharing.public(), &b.seed_namespace, round, sig)
                }
                None => false,
            },
            _ => certificate.seed.is_none(),
        }
    }

    fn is_attributable() -> bool {
        true
    }

    fn is_batchable() -> bool {
        true
    }

    fn certificate_codec_config(&self) -> <Self::Certificate as Read>::Cfg {
        self.vote.certificate_codec_config()
    }

    fn certificate_codec_config_unbounded() -> <Self::Certificate as Read>::Cfg {
        VoteScheme::certificate_codec_config_unbounded()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{beacon::seed_namespace, fluent_namespace, keys::ValidatorBlsKeypair};
    use commonware_codec::{DecodeExt as _, Encode as _};
    use commonware_consensus::{
        simplex::types::Proposal,
        types::{Epoch, Round, View},
    };
    use commonware_cryptography::{
        bls12381::dkg::deal_anonymous, ed25519::PrivateKey as Ed25519PrivateKey,
        sha256::Digest as Sha256Digest, Signer as _,
    };
    use commonware_math::algebra::Random as _;
    use commonware_parallel::Sequential;
    use commonware_utils::{ordered::BiMap, N3f1, TryCollect as _};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    const NS_CHAIN: u64 = 20994;

    /// `n` combined-scheme signers over one committee sharing the same public
    /// polynomial — each holds its own multisig key and the matching threshold
    /// share (share index == its committee Participant index).
    fn committee(n: usize) -> (Vec<CombinedScheme>, Vec<u8>) {
        let mut rng = StdRng::seed_from_u64(7);
        let peer_sks: Vec<Ed25519PrivateKey> = (0..n)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let bls_kps: Vec<ValidatorBlsKeypair> = (0..n)
            .map(|_| ValidatorBlsKeypair::generate(&mut rng))
            .collect();
        let bimap: BiMap<PeerPubkey, crate::BlsPubkey> = peer_sks
            .iter()
            .zip(bls_kps.iter())
            .map(|(p, b)| {
                (
                    p.public_key(),
                    crate::BlsPubkey::decode(b.public_bytes().as_slice()).unwrap(),
                )
            })
            .try_collect()
            .unwrap();

        let (sharing, shares) = deal_anonymous::<MinSig, N3f1>(
            &mut rng,
            Default::default(),
            (n as u32).try_into().unwrap(),
        );
        let ns = fluent_namespace(NS_CHAIN);
        let seed_ns = seed_namespace(&ns);

        let schemes = bls_kps
            .iter()
            .map(|kp| {
                let vote =
                    VoteScheme::signer(&ns, bimap.clone(), kp.secret().clone()).expect("member");
                let me = vote.me().expect("signer");
                let share = shares.iter().find(|s| s.index == me).unwrap().clone();
                CombinedScheme::new(vote, Some((sharing.clone(), Some(share), seed_ns.clone())))
            })
            .collect();
        (schemes, seed_ns)
    }

    fn proposal() -> Proposal<Sha256Digest> {
        Proposal::new(
            Round::new(Epoch::new(1), View::new(9)),
            View::new(8),
            Sha256Digest::decode([7u8; 32].as_slice()).unwrap(),
        )
    }

    fn assemble_over<'a>(
        schemes: &[CombinedScheme],
        subject: Subject<'a, Sha256Digest>,
    ) -> CombinedCertificate {
        let mut rng = StdRng::seed_from_u64(1);
        let atts: Vec<_> = schemes
            .iter()
            .map(|s| s.sign(subject).expect("sign"))
            .collect();
        // every signer's attestation must verify
        for a in &atts {
            assert!(schemes[0].verify_attestation(&mut rng, subject, a, &Sequential));
        }
        schemes[0]
            .assemble::<_, N3f1>(atts, &Sequential)
            .expect("assemble")
    }

    #[test]
    fn notarize_and_finalize_recover_byte_identical_seed() {
        let (schemes, _) = committee(4);
        let p = proposal();
        let cert_n = assemble_over(&schemes, Subject::Notarize { proposal: &p });
        let cert_f = assemble_over(&schemes, Subject::Finalize { proposal: &p });

        let seed_n = cert_n.seed().expect("notarization carries a seed");
        let seed_f = cert_f.seed().expect("finalization carries a seed");
        assert_eq!(
            seed_n.encode(),
            seed_f.encode(),
            "seed recovered from the notarization cert must be byte-identical to the finalization cert"
        );

        let mut rng = StdRng::seed_from_u64(2);
        assert!(schemes[0].verify_certificate::<_, Sha256Digest, N3f1>(
            &mut rng,
            Subject::Notarize { proposal: &p },
            &cert_n,
            &Sequential
        ));
    }

    #[test]
    fn nullify_certificate_has_no_seed() {
        let (schemes, _) = committee(4);
        let round = Round::new(Epoch::new(1), View::new(9));
        let cert = assemble_over(&schemes, Subject::Nullify { round });
        assert!(
            cert.seed().is_none(),
            "nullify carries the absent-seed sentinel"
        );
        let mut rng = StdRng::seed_from_u64(3);
        assert!(schemes[0].verify_certificate::<_, Sha256Digest, N3f1>(
            &mut rng,
            Subject::Nullify { round },
            &cert,
            &Sequential
        ));
    }

    #[test]
    fn withheld_seed_partial_makes_notarize_attestation_invalid() {
        let (schemes, _) = committee(4);
        let p = proposal();
        let subject = Subject::Notarize { proposal: &p };
        let mut att = schemes[0].sign(subject).expect("sign");
        let mut combined = *att.signature.get().unwrap();
        combined.seed = None;
        att.signature = combined.into();
        let mut rng = StdRng::seed_from_u64(4);
        assert!(
            !schemes[0].verify_attestation(&mut rng, subject, &att, &Sequential),
            "a Notarize without a valid seed partial must be rejected"
        );
    }

    #[test]
    fn fallback_scheme_is_pure_multisig() {
        let mut rng = StdRng::seed_from_u64(7);
        let peer_sks: Vec<Ed25519PrivateKey> = (0..4)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let bls_kps: Vec<ValidatorBlsKeypair> = (0..4)
            .map(|_| ValidatorBlsKeypair::generate(&mut rng))
            .collect();
        let bimap: BiMap<PeerPubkey, crate::BlsPubkey> = peer_sks
            .iter()
            .zip(bls_kps.iter())
            .map(|(p, b)| {
                (
                    p.public_key(),
                    crate::BlsPubkey::decode(b.public_bytes().as_slice()).unwrap(),
                )
            })
            .try_collect()
            .unwrap();
        let ns = fluent_namespace(NS_CHAIN);
        let schemes: Vec<CombinedScheme> = bls_kps
            .iter()
            .map(|kp| {
                let vote = VoteScheme::signer(&ns, bimap.clone(), kp.secret().clone()).unwrap();
                CombinedScheme::new(vote, None)
            })
            .collect();
        let p = proposal();
        let cert = assemble_over(&schemes, Subject::Notarize { proposal: &p });
        assert!(
            cert.seed().is_none(),
            "a fallback (beacon=None) cert carries no seed"
        );
    }
}
