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
//! WITHOUT a share it is fallback (the seed slot is the [`absent_seed`]
//! sentinel everywhere → the deriver uses the weak `order.digest()` randomness).
//! The seed slot is ALWAYS present (fixed 96 B) because `Scheme::Signature` is
//! `CodecFixed` — `Nullify` votes and fallback epochs carry the sentinel.

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
use commonware_math::algebra::Additive as _;
use commonware_parallel::Strategy;
use commonware_utils::{ordered::Set, Faults, Participant};
use rand_core::CryptoRngCore;

use crate::{beacon, BlsSignature, PeerPubkey, VoteScheme};

type VoteCertificate = <VoteScheme as CertScheme>::Certificate;

/// The canonical "no seed this vote" sentinel — the BLS12-381 G1 identity
/// (point at infinity), a never-valid threshold partial that can never collide
/// with a real recovered seed. Used on `Nullify` votes and in fallback epochs.
pub fn absent_seed() -> BlsSignature {
    BlsSignature::zero()
}

fn is_absent(sig: &BlsSignature) -> bool {
    *sig == absent_seed()
}

/// The round a subject is scoped to (used as the seed message domain).
fn subject_round<D: Digest>(subject: &Subject<'_, D>) -> Round {
    match subject {
        Subject::Notarize { proposal } | Subject::Finalize { proposal } => proposal.round,
        Subject::Nullify { round } => *round,
    }
}

/// Whether a subject carries a real seed partial (Notarize/Finalize) vs the
/// sentinel (Nullify).
fn seeded_subject<D: Digest>(subject: &Subject<'_, D>) -> bool {
    matches!(subject, Subject::Notarize { .. } | Subject::Finalize { .. })
}

/// Per-vote signature: attributable multisig share + threshold seed partial.
/// FIXED 96 B (two G1 points) — `CodecFixed` forbids a variable-size `Option`,
/// so `seed` is always present; it is [`absent_seed`] on a Nullify or fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CombinedSignature {
    pub vote: BlsSignature,
    pub seed: BlsSignature,
}

impl CombinedSignature {
    /// The attributable multisig half — what the slasher extracts (48 B).
    pub fn vote(&self) -> &BlsSignature {
        &self.vote
    }
}

impl FixedSize for CombinedSignature {
    const SIZE: usize = BlsSignature::SIZE * 2;
}

impl Write for CombinedSignature {
    fn write(&self, buf: &mut impl BufMut) {
        self.vote.write(buf);
        self.seed.write(buf);
    }
}

impl Read for CombinedSignature {
    type Cfg = ();
    fn read_cfg(buf: &mut impl Buf, _: &()) -> Result<Self, CodecError> {
        let vote = BlsSignature::read(buf)?;
        let seed = BlsSignature::read(buf)?;
        Ok(Self { vote, seed })
    }
}

/// Certificate assembled from a quorum of [`CombinedSignature`]s: the
/// attributable multisig certificate (bitmap + aggregate vote) plus the
/// recovered threshold seed (or [`absent_seed`] for a Nullify/fallback cert).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CombinedCertificate {
    pub vote: VoteCertificate,
    pub seed: BlsSignature,
}

impl CombinedCertificate {
    /// The recovered seed signature, or `None` when absent (Nullify/fallback).
    pub fn seed(&self) -> Option<BlsSignature> {
        (!is_absent(&self.seed)).then_some(self.seed)
    }
}

impl Write for CombinedCertificate {
    fn write(&self, buf: &mut impl BufMut) {
        self.vote.write(buf);
        self.seed.write(buf);
    }
}

impl EncodeSize for CombinedCertificate {
    fn encode_size(&self) -> usize {
        self.vote.encode_size() + BlsSignature::SIZE
    }
}

impl Read for CombinedCertificate {
    type Cfg = usize;
    fn read_cfg(buf: &mut impl Buf, participants: &usize) -> Result<Self, CodecError> {
        let vote = VoteCertificate::read_cfg(buf, participants)?;
        let seed = BlsSignature::read(buf)?;
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
                beacon::sign_seed_partial(share, &b.seed_namespace, round).value
            }
            _ => absent_seed(),
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
            (Some(b), true) => {
                let partial = PartialSignature::<MinSig> {
                    index: attestation.signer,
                    value: combined.seed,
                };
                !is_absent(&combined.seed)
                    && beacon::verify_seed_partial(&b.sharing, &b.seed_namespace, round, &partial)
            }
            _ => is_absent(&combined.seed),
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
                    .all(|a| a.signature.get().is_some_and(|c| !is_absent(&c.seed))) =>
            {
                let partials: Vec<PartialSignature<MinSig>> = atts
                    .iter()
                    .filter_map(|a| {
                        a.signature.get().map(|c| PartialSignature::<MinSig> {
                            index: a.signer,
                            value: c.seed,
                        })
                    })
                    .collect();
                beacon::recover_seed(&b.sharing, &partials).ok()?
            }
            _ => absent_seed(),
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
            (Some(b), true) => {
                !is_absent(&certificate.seed)
                    && beacon::verify_seed(
                        b.sharing.public(),
                        &b.seed_namespace,
                        round,
                        &certificate.seed,
                    )
            }
            _ => is_absent(&certificate.seed),
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
