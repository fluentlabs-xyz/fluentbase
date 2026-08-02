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
//! (a real partial is REQUIRED on EVERY vote, `Nullify` included — a vote
//! without it is invalid → not counted → quorum ⟺ ≥t partials); WITHOUT a share
//! it is fallback (`seed = None` everywhere → the deriver uses the weak
//! `order.digest()` randomness).
//! The signature is `CodecFixed`, so the optional seed is a FIXED slot (a
//! 1-byte present flag + a 48-byte G1 slot): only fallback (pre-bootstrap)
//! epochs carry `None`.
//!
//! Seeding `Nullify` too is what makes the leader of view v+1 independent of
//! whether view v produced a block: σ signs the round alone, so a nullification
//! and a notarization of the same view recover the byte-identical σ, and an
//! adversary able to force a view empty gains no choice of draw.

use bytes::{Buf, BufMut};
use commonware_codec::{EncodeSize, Error as CodecError, FixedSize, Read, ReadExt as _, Write};
use core::mem::size_of;
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

use crate::{
    beacon::{self, GroupPublic},
    BlsSignature, PeerPubkey, VoteScheme,
};

type VoteCertificate = <VoteScheme as CertScheme>::Certificate;

/// Compressed-G1 byte length — the seed slot width.
const SEED_SLOT: usize = crate::SIGNATURE_BYTES;
/// The seed-present flag byte preceding the [`SEED_SLOT`] (1 = Some, 0 = None).
const SEED_FLAG: usize = size_of::<u8>();

/// The round a subject is scoped to (used as the seed message domain).
fn subject_round<D: Digest>(subject: &Subject<'_, D>) -> Round {
    match subject {
        Subject::Notarize { proposal } | Subject::Finalize { proposal } => proposal.round,
        Subject::Nullify { round } => *round,
    }
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
    const SIZE: usize = crate::SIGNATURE_BYTES + SEED_FLAG + SEED_SLOT;
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
        self.vote.encode_size() + SEED_FLAG + SEED_SLOT
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
    /// Full public polynomial. REQUIRED to verify individual seed partials
    /// (`verify_attestation`) and to recover the seed (`assemble`). A no-share
    /// node (verify-only / fallback) has no `BeaconPart` at all (`beacon: None`),
    /// so once a `BeaconPart` exists the polynomial is always present.
    sharing: Sharing<MinSig>,
    share: Option<Share>,
    seed_namespace: Vec<u8>,
}

/// Combined attributable + threshold consensus scheme.
#[derive(Clone)]
pub struct CombinedScheme {
    vote: VoteScheme,
    beacon: Option<BeaconPart>,
    /// Cert-time seed pin: the epoch's beacon group key `PK_epoch` + seed
    /// namespace. `verify_certificate` uses it to reject a wire-received cert
    /// whose recovered seed slot fails `verify_seed` against `PK_epoch` (a
    /// tampered/cleared seed on an otherwise-valid multisig quorum). A
    /// beacon-active scheme derives it from its own `BeaconPart`; a
    /// verifier-flavored scheme (`beacon = None`) may receive it externally
    /// from the finalized change-boundary block's `beacon_outcome` (agreed,
    /// multisig-bound data — NOT on-chain storage, which stays deleted).
    /// `None` ⇒ vote-only cert verify (pre-beacon fallback epoch, or the key
    /// cursor is unresolved — e.g. right after a deep cold-start jump landing).
    cert_seed_pin: Option<(GroupPublic, Vec<u8>)>,
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
    ///
    /// `external_pin` supplies the cert-time seed pin for a verifier-flavored
    /// scheme (`beacon = None`) whose key comes from an agreed boundary-block
    /// `beacon_outcome`; when `beacon` is present the pin is derived from it and
    /// `external_pin` is ignored (a beacon-active scheme already holds `PK_epoch`).
    pub(crate) fn new(
        vote: VoteScheme,
        beacon: Option<(Sharing<MinSig>, Option<Share>, Vec<u8>)>,
        external_pin: Option<(GroupPublic, Vec<u8>)>,
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
        // INVARIANT: a pin must never be attached for an epoch below
        // `DETERMINISTIC_BOOTSTRAP_EPOCH`. `verify_certificate` reads the pin's
        // presence as "this epoch is beacon-active" and rejects any seedless cert
        // under it — so a pin on a pre-beacon epoch would reject every legal cert
        // there. Holds today because both sources are unreachable that early: a
        // `BeaconPart` needs a dealt key, and `external_pin` comes from a boundary
        // block's `beacon_outcome`, the first of which is epoch 2's.
        let cert_seed_pin = match &beacon {
            Some(b) => Some((*b.sharing.public(), b.seed_namespace.clone())),
            None => external_pin,
        };
        Self {
            vote,
            beacon,
            cert_seed_pin,
        }
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
        // EVERY subject carries a seed partial in a beacon-active epoch, Nullify
        // included: σ signs the round alone, so a nullification and a notarization
        // of the same view recover the byte-identical σ. That is what makes the
        // leader of view v+1 independent of whether view v produced a block, and
        // it is the whole point of the seed slot being fixed-width.
        let seed = match &self.beacon {
            Some(b) => {
                let share = b.share.as_ref()?;
                Some(beacon::sign_seed_partial(share, &b.seed_namespace, round).value)
            }
            None => None,
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
        match &self.beacon {
            // Beacon-active, ANY subject: a missing or invalid seed partial makes
            // the whole vote invalid (→ not counted toward quorum). A
            // group-key-only verifier (no polynomial) cannot check an
            // individual partial — it only ever verifies assembled certs, so
            // reject here rather than accept unchecked.
            //
            // A consequence that is NOT free: a member whose share does not lie
            // on the sharing can no longer help NULLIFY either. While blocks flow
            // the notarize path exposes such a share; in a sustained stall it does
            // not, and at t == quorum one impaired member then makes the nullify
            // quorum unreachable. The promote-time self-check in `epoch_manager`
            // is what keeps that member off the plane — do not remove one without
            // the other.
            //
            // TODO(perf): per-partial verification is O(n) pairing checks per
            // round (~one BLS verify per incoming vote, ~35–51 at n=51) vs O(1)
            // for verifying the recovered aggregate once against the group key.
            // It's load-bearing because t == consensus quorum (no slack: every
            // counted partial must be valid to recover the seed) and it gives
            // per-vote attribution of a bad partial. Affordable at n=51 / 1 blk/s
            // (a few % of a core, and parallelizable), but REVISIT if seed verify
            // becomes a bottleneck at larger n or higher block rates — options:
            // batch-verify the partials (random-linear-combination, but loses
            // per-vote attribution on failure) or aggregate-verify with t < quorum
            // slack. Measure before changing — don't trade away attribution blind.
            Some(b) => match combined.seed {
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
            // A fallback (pre-bootstrap) epoch has no key material at all: the seed
            // MUST be absent, for Nullify and Notarize alike. This arm is what keeps
            // epochs below `DETERMINISTIC_BOOTSTRAP_EPOCH` legal.
            None => combined.seed.is_none(),
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
            Some(BeaconPart { sharing, .. })
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
                Some(beacon::recover_seed::<M>(sharing, &partials).ok()?)
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
        // First the attributable multisig quorum. A false here is always a reject.
        if !self
            .vote
            .verify_certificate::<_, _, M>(rng, subject, &certificate.vote, strategy)
        {
            return false;
        }
        // Then the recovered seed slot, whenever the epoch's beacon key is
        // resolvable locally. Self-assembled certs pass by construction (the seed
        // recovered from partials `verify_attestation` already checked verifies
        // against the same `PK_epoch`); a wire-received cert with a tampered or
        // cleared seed slot on an otherwise-valid multisig quorum is rejected here
        // — the ingress check bug 2 was missing. `None` ⇒ no key material (a
        // fallback no-beacon epoch, or the cursor is unresolved right after a deep
        // jump landing) ⇒ vote-only, exactly the prior behaviour.
        let Some((group_public, seed_namespace)) = &self.cert_seed_pin else {
            return true;
        };
        match certificate.seed {
            Some(sig) => {
                beacon::verify_seed(group_public, seed_namespace, subject_round(&subject), &sig)
            }
            // ANY cert in a beacon-active epoch MUST carry a seed — Nullify included.
            // The pin's presence is what "beacon-active" means here, so this arm is
            // unreachable below `DETERMINISTIC_BOOTSTRAP_EPOCH`: a sub-bootstrap
            // epoch has no `BeaconPart` and no boundary `beacon_outcome`, hence no
            // pin, hence the early return above. Keep that invariant intact — if a
            // pin ever became attachable to a pre-beacon epoch, this arm would
            // reject every legal seedless cert on it.
            None => false,
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
    fn committee(
        n: usize,
    ) -> (
        Vec<CombinedScheme>,
        Vec<u8>,
        Sharing<MinSig>,
        BiMap<PeerPubkey, crate::BlsPubkey>,
    ) {
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
                CombinedScheme::new(
                    vote,
                    Some((sharing.clone(), Some(share), seed_ns.clone())),
                    None,
                )
            })
            .collect();
        (schemes, seed_ns, sharing, bimap)
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
        let (schemes, _, _, _) = committee(4);
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
    fn vote_only_verifier_accepts_seeded_cert_and_rejects_wrong_multisig() {
        // After the on-chain PK_E removal every verifier (cert-follower /
        // marshal / non-signer) checks an assembled cert MULTISIG-ONLY: a
        // beacon-active (seeded) cert is accepted — the seed is bound by the
        // quorum — and a cert whose multisig does not match the verified subject
        // is rejected. (Pre-removal a `beacon: None` scheme wrongly rejected ANY
        // seeded cert via the `_ => seed.is_none()` arm.)
        let (schemes, _, _, bimap) = committee(4);
        let p = proposal();
        let cert = assemble_over(&schemes, Subject::Notarize { proposal: &p });

        let ns = fluent_namespace(NS_CHAIN);
        let verifier = CombinedScheme::new(VoteScheme::verifier(&ns, bimap), None, None);
        let mut rng = StdRng::seed_from_u64(5);

        assert!(
            verifier.verify_certificate::<_, Sha256Digest, N3f1>(
                &mut rng,
                Subject::Notarize { proposal: &p },
                &cert,
                &Sequential
            ),
            "vote-only verifier must accept a seeded cert whose multisig matches the subject"
        );

        // The same cert checked against a DIFFERENT proposal (foreign payload):
        // the multisig is bound to `p`, so the quorum check fails.
        let other = Proposal::new(
            Round::new(Epoch::new(1), View::new(9)),
            View::new(8),
            Sha256Digest::decode([9u8; 32].as_slice()).unwrap(),
        );
        assert!(
            !verifier.verify_certificate::<_, Sha256Digest, N3f1>(
                &mut rng,
                Subject::Notarize { proposal: &other },
                &cert,
                &Sequential
            ),
            "vote-only verifier must reject a cert whose multisig does not match the subject"
        );
    }

    #[test]
    fn nullify_certificate_carries_the_view_seed() {
        let (schemes, _, _, _) = committee(4);
        let round = Round::new(Epoch::new(1), View::new(9));
        let cert = assemble_over(&schemes, Subject::Nullify { round });
        assert!(cert.seed().is_some());
        let mut rng = StdRng::seed_from_u64(3);
        assert!(schemes[0].verify_certificate::<_, Sha256Digest, N3f1>(
            &mut rng,
            Subject::Nullify { round },
            &cert,
            &Sequential
        ));
    }

    /// The security argument of the whole seed-continuity change: because σ signs
    /// the ROUND alone, the leader elected for view v+1 is the same value whether
    /// view v notarized or nullified — so an adversary able to force a view empty
    /// gains no choice of draw.
    #[test]
    fn notarize_and_nullify_of_same_round_recover_byte_identical_seed() {
        let (schemes, _, _, _) = committee(4);
        let p = proposal();
        let round = p.round;
        let cert_n = assemble_over(&schemes, Subject::Notarize { proposal: &p });
        let cert_x = assemble_over(&schemes, Subject::Nullify { round });

        assert_eq!(
            cert_n
                .seed()
                .expect("notarization carries a seed")
                .encode(),
            cert_x.seed().expect("nullification carries a seed").encode(),
        );
    }

    /// The liveness hazard the promote-time share self-check exists to prevent: a
    /// member holding a share that does not lie on the committee's sharing produces
    /// a NULLIFY vote no honest node counts. While blocks flow the notarize path
    /// exposes such a share; in a stall it does not, and at `t == quorum` one such
    /// member makes the nullify quorum unreachable.
    #[test]
    fn a_partial_from_a_foreign_sharing_invalidates_a_nullify_vote() {
        let (schemes, seed_ns, _, _) = committee(4);
        let round = Round::new(Epoch::new(1), View::new(9));
        let subject = Subject::Nullify { round };

        let honest = schemes[0].sign::<Sha256Digest>(subject).expect("sign");
        let mut rng = StdRng::seed_from_u64(17);
        assert!(schemes[1].verify_attestation(&mut rng, subject, &honest, &Sequential));

        let (_, foreign_shares) = deal_anonymous::<MinSig, N3f1>(
            &mut StdRng::seed_from_u64(99),
            Default::default(),
            4u32.try_into().unwrap(),
        );
        let foreign = foreign_shares
            .iter()
            .find(|s| s.index == honest.signer)
            .expect("same index on the foreign sharing");
        let impaired = Attestation::<CombinedScheme> {
            signer: honest.signer,
            signature: CombinedSignature {
                vote: honest.signature.get().expect("decodes").vote,
                seed: Some(beacon::sign_seed_partial(foreign, &seed_ns, round).value),
            }
            .into(),
        };
        assert!(!schemes[1].verify_attestation(&mut rng, subject, &impaired, &Sequential));
    }

    /// Below `DETERMINISTIC_BOOTSTRAP_EPOCH` there is no key material at all, and
    /// BOTH subject kinds must stay legal seedless — this is the sequencer→DPoS
    /// transition window, not a degraded mode.
    #[test]
    fn fallback_epoch_accepts_seedless_nullify_and_notarize() {
        let mut rng = StdRng::seed_from_u64(7);
        let peer_sk = Ed25519PrivateKey::random(&mut rng);
        let bls_kp = ValidatorBlsKeypair::generate(&mut rng);
        let bimap: BiMap<PeerPubkey, crate::BlsPubkey> = std::iter::once((
            peer_sk.public_key(),
            crate::BlsPubkey::decode(bls_kp.public_bytes().as_slice()).unwrap(),
        ))
        .try_collect()
        .unwrap();
        let ns = fluent_namespace(NS_CHAIN);
        let vote = VoteScheme::signer(&ns, bimap, bls_kp.secret().clone()).expect("member");
        let scheme = CombinedScheme::new(vote, None, None);

        let p = proposal();
        let round = p.round;
        for subject in [
            Subject::Notarize { proposal: &p },
            Subject::Nullify { round },
        ] {
            let att = scheme.sign::<Sha256Digest>(subject).expect("sign");
            assert!(att.signature.get().expect("decodes").seed.is_none());
            assert!(scheme.verify_attestation(&mut rng, subject, &att, &Sequential));
        }
    }

    #[test]
    fn withheld_seed_partial_makes_notarize_attestation_invalid() {
        let (schemes, _, _, _) = committee(4);
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
                CombinedScheme::new(vote, None, None)
            })
            .collect();
        let p = proposal();
        let cert = assemble_over(&schemes, Subject::Notarize { proposal: &p });
        assert!(
            cert.seed().is_none(),
            "a fallback (beacon=None) cert carries no seed"
        );
        // A no-pin scheme stays vote-only: a genuine seedless fallback cert must
        // still verify (the residual/degrade path must never reject honest data).
        let mut rng = StdRng::seed_from_u64(8);
        assert!(
            schemes[0].verify_certificate::<_, Sha256Digest, N3f1>(
                &mut rng,
                Subject::Notarize { proposal: &p },
                &cert,
                &Sequential
            ),
            "a fallback (no-pin) seedless cert must still verify vote-only"
        );
    }

    #[test]
    fn pinned_verify_certificate_rejects_tampered_and_cleared_seed() {
        let (schemes, seed_ns, sharing, bimap) = committee(4);
        let p = proposal();
        let cert = assemble_over(&schemes, Subject::Finalize { proposal: &p });

        // A verifier holding the epoch group key (the boundary-block cursor
        // source): its `verify_certificate` pins the recovered seed to `PK_epoch`.
        let ns = fluent_namespace(NS_CHAIN);
        let pinned = CombinedScheme::new(
            VoteScheme::verifier(&ns, bimap.clone()),
            None,
            Some((*sharing.public(), seed_ns.clone())),
        );
        let mut rng = StdRng::seed_from_u64(11);

        assert!(
            pinned.verify_certificate::<_, Sha256Digest, N3f1>(
                &mut rng,
                Subject::Finalize { proposal: &p },
                &cert,
                &Sequential
            ),
            "a genuine seeded cert verifies under a pin"
        );

        // A seed recovered for a DIFFERENT round is a valid G1 point that does
        // NOT verify against THIS round — a stand-in for a tampered seed slot.
        let other = Proposal::new(
            Round::new(Epoch::new(1), View::new(42)),
            View::new(41),
            Sha256Digest::decode([5u8; 32].as_slice()).unwrap(),
        );
        let other_cert = assemble_over(&schemes, Subject::Finalize { proposal: &other });
        let tampered = CombinedCertificate {
            vote: cert.vote.clone(),
            seed: other_cert.seed,
        };
        assert!(
            !pinned.verify_certificate::<_, Sha256Digest, N3f1>(
                &mut rng,
                Subject::Finalize { proposal: &p },
                &tampered,
                &Sequential
            ),
            "a foreign-round seed on a valid multisig quorum must be rejected under a pin"
        );

        let cleared = CombinedCertificate {
            vote: cert.vote.clone(),
            seed: None,
        };
        assert!(
            !pinned.verify_certificate::<_, Sha256Digest, N3f1>(
                &mut rng,
                Subject::Finalize { proposal: &p },
                &cleared,
                &Sequential
            ),
            "a seeded-subject cert with a cleared seed flag must be rejected under a pin"
        );

        // Without a pin the same cleared cert still verifies (the accepted
        // residual window — a node that cannot resolve the key stays vote-only).
        let pinless = CombinedScheme::new(VoteScheme::verifier(&ns, bimap), None, None);
        assert!(
            pinless.verify_certificate::<_, Sha256Digest, N3f1>(
                &mut rng,
                Subject::Finalize { proposal: &p },
                &cleared,
                &Sequential
            ),
            "a pin-less verifier stays vote-only (documents the accepted residual window)"
        );
    }
}
