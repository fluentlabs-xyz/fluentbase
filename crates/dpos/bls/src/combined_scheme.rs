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
use commonware_consensus::{simplex::types::Subject, types::Round};
use commonware_cryptography::{
    certificate::{Attestation, Scheme as CertScheme},
    Digest,
};
use commonware_parallel::Strategy;
use commonware_utils::{ordered::Set, Faults, Participant};
use core::mem::size_of;
use rand_core::CryptoRngCore;
use std::sync::Arc;

use crate::{
    oracle::{SeedCheck, SeedOracle},
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
/// identity is not a decodable point (`G1::read` rejects infinity), so a
/// fallback-epoch vote, which carries no seed at all, could not otherwise
/// round-trip while keeping the `CodecFixed` constant size.
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
/// partial. FIXED 97 B (vote 48 ‖ flag 1 ‖ seed-slot 48); `seed = None` ONLY in
/// a fallback (no-beacon) epoch.
///
/// NOT on a Nullify — a beacon-active epoch carries a partial on every subject,
/// Nullify included (`sign`, and the module doc's last paragraph for why). A
/// reader who believes otherwise will conclude a nullified view produces no seed,
/// which is exactly the property the fixed-width slot exists to deny.
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
/// recovered threshold seed (`None` for a FALLBACK cert only — a nullification
/// in a beacon-active epoch recovers the same σ a notarization of that view
/// would).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CombinedCertificate {
    pub vote: VoteCertificate,
    pub seed: Option<BlsSignature>,
}

impl CombinedCertificate {
    /// The recovered seed signature, or `None` in a fallback (no-beacon) epoch.
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

/// Combined attributable + threshold consensus scheme.
///
/// Holds NO key material. Everything threshold is delegated to the beacon's
/// [`SeedOracle`], which reads live state under a lock, so a scheme built before
/// its epoch's ceremony finished starts working the moment the material lands
/// instead of carrying a stale copy for the life of the epoch.
#[derive(Clone)]
pub struct CombinedScheme {
    vote: VoteScheme,
    /// The epoch this scheme was issued for. Purely a BINDING: a subject from
    /// another epoch is rejected before the oracle is consulted. This used to be
    /// an accidental side effect of copying the epoch's key in; it is explicit
    /// now, and it rejects rather than silently answering under the wrong epoch.
    epoch: u64,
    /// `None` ⇒ a fallback (pre-beacon) epoch: no partial is produced, and a
    /// vote carrying one is invalid.
    ///
    /// Distinct from an oracle that answers "no material", which is a
    /// beacon-active epoch this node cannot judge in. Both refuse a vote that
    /// carries a partial, but only the first makes a SEEDLESS vote correct — do
    /// not collapse them.
    oracle: Option<Arc<dyn SeedOracle>>,
}

impl core::fmt::Debug for CombinedScheme {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CombinedScheme")
            .field("vote", &self.vote)
            .field("epoch", &self.epoch)
            .field("beacon_active", &self.oracle.is_some())
            .finish()
    }
}

impl CombinedScheme {
    /// Build from an already-constructed vote scheme, the epoch it is issued
    /// for, and the beacon's oracle.
    ///
    /// `oracle = None` ⇒ fallback (pure multisig, sentinel seed everywhere).
    /// The share-index binding the old material-carrying constructor asserted
    /// lives in the oracle now, where the share still exists.
    pub(crate) fn new(vote: VoteScheme, epoch: u64, oracle: Option<Arc<dyn SeedOracle>>) -> Self {
        Self {
            vote,
            epoch,
            oracle,
        }
    }

    /// Whether `subject`'s round belongs to the epoch this scheme was issued
    /// for. A scheme is registered per epoch and its oracle answers for that
    /// epoch alone, so a subject from another one has no meaning here — it is
    /// refused before the oracle is consulted rather than judged under the wrong
    /// epoch's material.
    fn binds<D: Digest>(&self, subject: &Subject<'_, D>) -> bool {
        subject_round(subject).epoch().get() == self.epoch
    }

    /// Whether this scheme judges the seed slot at all.
    ///
    /// `true` ⇔ an oracle is attached ⇔ the epoch is beacon-active, which is
    /// what makes `verify_certificate` refuse a cleared seed slot and
    /// `verify_attestation` refuse a partial-less vote. It is the ONLY
    /// verification strength a scheme carries that neither
    /// [`CertScheme::participants`] nor [`CertScheme::me`] can see, so a registry
    /// that wants to keep re-registration monotone in strength has to ask for it
    /// explicitly — see `EpochSchemeProvider::register`.
    pub fn is_beacon_active(&self) -> bool {
        self.oracle.is_some()
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
        if !self.binds(&subject) {
            return None;
        }
        let round = subject_round(&subject);
        let vote_att = self.vote.sign::<D>(subject)?;
        let vote = *vote_att.signature.get()?;
        // EVERY subject carries a seed partial in a beacon-active epoch, Nullify
        // included: σ signs the round alone, so a nullification and a notarization
        // of the same view recover the byte-identical σ. That is what makes the
        // leader of view v+1 independent of whether view v produced a block, and
        // it is the whole point of the seed slot being fixed-width.
        //
        // A beacon-active epoch whose oracle has no usable material casts NO vote
        // at all rather than a seedless one: peers reject a seedless vote in such
        // an epoch, so emitting it would subtract this node from the quorum while
        // it believed it was signing.
        let seed = match &self.oracle {
            Some(o) => Some(o.sign_partial(round)?),
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
        if !self.binds(&subject) {
            return false;
        }
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
        match &self.oracle {
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
            Some(o) => match combined.seed {
                Some(value) => o.verify_partial(round, attestation.signer, &value),
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
        let seed = match &self.oracle {
            Some(o)
                if atts
                    .iter()
                    .all(|a| a.signature.get().is_some_and(|c| c.seed.is_some())) =>
            {
                let partials: Vec<(Participant, BlsSignature)> = atts
                    .iter()
                    .filter_map(|a| {
                        a.signature
                            .get()
                            .and_then(|c| c.seed.map(|value| (a.signer, value)))
                    })
                    .collect();
                // The seed threshold is computed HERE, from the same `M` the vote
                // half just quorum'd under — that is what keeps the two halves of
                // one certificate in lockstep now that the oracle cannot carry the
                // fault model as a type parameter.
                let threshold = M::quorum(self.vote.participants().len() as u32);
                Some(o.recover(&partials, threshold)?)
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
        // Then the epoch binding — ABOVE the oracle branch, not inside it. Every
        // other entry point applies it unconditionally, and a pre-beacon scheme
        // that skipped it would answer for a subject naming a different epoch
        // whenever the two committees happened to coincide.
        if !self.binds(&subject) {
            return false;
        }
        // Then the recovered seed slot. Self-assembled certs pass by construction
        // (the seed recovered from partials `verify_attestation` already checked
        // verifies against the same `PK_epoch`); a wire-received cert with a
        // tampered or cleared seed slot on an otherwise-valid multisig quorum is
        // rejected here.
        //
        // No oracle ⇒ a fallback no-beacon epoch ⇒ vote-only.
        let Some(o) = &self.oracle else {
            return true;
        };
        match certificate.seed {
            // `NoKey` — the epoch's key is not resolvable here yet — ADMITS on the
            // multisig quorum alone and nobody consumes the σ. Rejecting instead
            // would punish the sender for this node's own missing key.
            Some(sig) => !matches!(
                o.verify_seed(subject_round(&subject), &sig),
                SeedCheck::Invalid
            ),
            // ANY cert in a beacon-active epoch MUST carry a seed — Nullify
            // included. The oracle's presence is what "beacon-active" means here,
            // so this arm is unreachable below `DETERMINISTIC_BOOTSTRAP_EPOCH`,
            // where no oracle is ever attached.
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
    use commonware_cryptography::bls12381::primitives::{
        group::Share,
        sharing::Sharing,
        variant::{MinSig, PartialSignature},
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
    /// The epoch every fixture below is issued for — `proposal()`'s round sits in
    /// it, so the scheme's epoch binding passes.
    const EPOCH: u64 = 1;

    /// A [`SeedOracle`] over one fixed sharing: exactly the material this scheme
    /// used to hold itself. Production supplies
    /// `fluentbase_consensus::beacon::oracle::BeaconOracle` instead, which reads
    /// the live ceremony store.
    #[derive(Debug)]
    struct TestOracle {
        sharing: Sharing<MinSig>,
        share: Option<Share>,
        seed_ns: Vec<u8>,
        /// `false` ⇒ [`SeedCheck::NoKey`]: a beacon-active epoch whose group key
        /// this node cannot resolve yet.
        key_known: bool,
    }

    impl TestOracle {
        fn arc(
            sharing: &Sharing<MinSig>,
            share: Option<Share>,
            seed_ns: &[u8],
        ) -> Arc<dyn SeedOracle> {
            Arc::new(Self {
                sharing: sharing.clone(),
                share,
                seed_ns: seed_ns.to_vec(),
                key_known: true,
            })
        }
    }

    impl SeedOracle for TestOracle {
        fn sign_partial(&self, round: Round) -> Option<BlsSignature> {
            let share = self.share.as_ref()?;
            Some(crate::beacon::sign_seed_partial(share, &self.seed_ns, round).value)
        }

        fn verify_partial(&self, round: Round, index: Participant, value: &BlsSignature) -> bool {
            crate::beacon::verify_seed_partial(
                &self.sharing,
                &self.seed_ns,
                round,
                &PartialSignature::<MinSig> {
                    index,
                    value: *value,
                },
            )
        }

        fn recover(
            &self,
            partials: &[(Participant, BlsSignature)],
            threshold: u32,
        ) -> Option<BlsSignature> {
            let partials: Vec<PartialSignature<MinSig>> = partials
                .iter()
                .map(|&(index, value)| PartialSignature::<MinSig> { index, value })
                .collect();
            crate::beacon::recover_seed_with_threshold(&self.sharing, &partials, threshold).ok()
        }

        fn verify_seed(&self, round: Round, seed: &BlsSignature) -> SeedCheck {
            if !self.key_known {
                return SeedCheck::NoKey;
            }
            if crate::beacon::verify_seed(self.sharing.public(), &self.seed_ns, round, seed) {
                SeedCheck::Valid
            } else {
                SeedCheck::Invalid
            }
        }
    }

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
                    EPOCH,
                    Some(TestOracle::arc(&sharing, Some(share), &seed_ns)),
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
        let verifier = CombinedScheme::new(VoteScheme::verifier(&ns, bimap), EPOCH, None);
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
            cert_n.seed().expect("notarization carries a seed").encode(),
            cert_x
                .seed()
                .expect("nullification carries a seed")
                .encode(),
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
                seed: Some(crate::beacon::sign_seed_partial(foreign, &seed_ns, round).value),
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
        let scheme = CombinedScheme::new(vote, EPOCH, None);

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
                CombinedScheme::new(vote, EPOCH, None)
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
    fn oracle_backed_verify_certificate_rejects_tampered_and_cleared_seed() {
        let (schemes, seed_ns, sharing, bimap) = committee(4);
        let p = proposal();
        let cert = assemble_over(&schemes, Subject::Finalize { proposal: &p });

        let ns = fluent_namespace(NS_CHAIN);
        let verifier = CombinedScheme::new(
            VoteScheme::verifier(&ns, bimap.clone()),
            EPOCH,
            Some(TestOracle::arc(&sharing, None, &seed_ns)),
        );
        let mut rng = StdRng::seed_from_u64(11);

        assert!(
            verifier.verify_certificate::<_, Sha256Digest, N3f1>(
                &mut rng,
                Subject::Finalize { proposal: &p },
                &cert,
                &Sequential
            ),
            "a genuine seeded cert verifies against the epoch key"
        );

        // A seed recovered for a DIFFERENT round is a valid G1 point that does
        // NOT verify against THIS round — a stand-in for a tampered seed slot.
        let other = Proposal::new(
            Round::new(Epoch::new(EPOCH), View::new(42)),
            View::new(41),
            Sha256Digest::decode([5u8; 32].as_slice()).unwrap(),
        );
        let other_cert = assemble_over(&schemes, Subject::Finalize { proposal: &other });
        let tampered = CombinedCertificate {
            vote: cert.vote.clone(),
            seed: other_cert.seed,
        };
        assert!(
            !verifier.verify_certificate::<_, Sha256Digest, N3f1>(
                &mut rng,
                Subject::Finalize { proposal: &p },
                &tampered,
                &Sequential
            ),
            "a foreign-round seed on a valid multisig quorum must be rejected"
        );

        let cleared = CombinedCertificate {
            vote: cert.vote.clone(),
            seed: None,
        };
        assert!(
            !verifier.verify_certificate::<_, Sha256Digest, N3f1>(
                &mut rng,
                Subject::Finalize { proposal: &p },
                &cleared,
                &Sequential
            ),
            "a seeded-subject cert with a cleared seed flag must be rejected"
        );
    }

    /// The keyless window, and the line through it: a beacon-active epoch whose
    /// group key this node cannot resolve ADMITS a seeded cert on its multisig
    /// quorum alone — rejecting would punish the sender for a local miss — but
    /// still refuses a STRIPPED one, because a cert of such an epoch always
    /// carries a seed. Only `oracle: None`, a pre-beacon epoch, makes seedless
    /// legal.
    #[test]
    fn a_keyless_oracle_admits_a_seeded_cert_but_still_refuses_a_stripped_one() {
        let (schemes, seed_ns, sharing, bimap) = committee(4);
        let p = proposal();
        let cert = assemble_over(&schemes, Subject::Finalize { proposal: &p });

        let ns = fluent_namespace(NS_CHAIN);
        let keyless = CombinedScheme::new(
            VoteScheme::verifier(&ns, bimap),
            EPOCH,
            Some(Arc::new(TestOracle {
                sharing,
                share: None,
                seed_ns,
                key_known: false,
            })),
        );
        let mut rng = StdRng::seed_from_u64(12);

        assert!(keyless.verify_certificate::<_, Sha256Digest, N3f1>(
            &mut rng,
            Subject::Finalize { proposal: &p },
            &cert,
            &Sequential
        ));

        let stripped = CombinedCertificate {
            vote: cert.vote.clone(),
            seed: None,
        };
        assert!(!keyless.verify_certificate::<_, Sha256Digest, N3f1>(
            &mut rng,
            Subject::Finalize { proposal: &p },
            &stripped,
            &Sequential
        ));
    }

    /// The epoch binding, which used to be an accident of copying the key in. A
    /// scheme is registered per epoch and its oracle answers for that epoch
    /// alone, so every entry point that RECEIVES A SUBJECT refuses one from
    /// another epoch rather than judging it under the wrong material.
    ///
    /// Three entry points, not four: `assemble` is handed attestations alone —
    /// no subject, no round — so it has nothing to bind against. The name said
    /// "every entry point" while covering three, which is a claim the test could
    /// not keep.
    ///
    /// The `oracle: None` arm is here because that is where the binding was
    /// actually MISSING: `verify_certificate` used to return `true` for a
    /// pre-beacon scheme before it ever reached `binds`.
    #[test]
    fn a_subject_from_another_epoch_is_refused_by_every_subject_bearing_entry_point() {
        let (schemes, _, _, bimap) = committee(4);
        let p = proposal();
        let cert = assemble_over(&schemes, Subject::Notarize { proposal: &p });
        let att = schemes[0]
            .sign::<Sha256Digest>(Subject::Notarize { proposal: &p })
            .expect("the fixture signs inside its own epoch");

        let foreign = Proposal::new(
            Round::new(Epoch::new(EPOCH + 1), View::new(9)),
            View::new(8),
            p.payload,
        );
        let subject = Subject::Notarize { proposal: &foreign };
        let mut rng = StdRng::seed_from_u64(13);

        assert!(schemes[0].sign::<Sha256Digest>(subject).is_none());
        assert!(!schemes[0].verify_attestation(&mut rng, subject, &att, &Sequential));
        assert!(!schemes[0].verify_certificate::<_, Sha256Digest, N3f1>(
            &mut rng,
            subject,
            &cert,
            &Sequential
        ));

        // Same committee, no oracle — a pre-beacon epoch. The multisig quorum
        // verifies (it is the same committee over the same subject bytes), so
        // only the binding can refuse this one.
        let pre_beacon = CombinedScheme::new(
            VoteScheme::verifier(&fluent_namespace(NS_CHAIN), bimap),
            EPOCH,
            None,
        );
        let seedless = CombinedCertificate {
            vote: cert.vote.clone(),
            seed: None,
        };
        assert!(
            !pre_beacon.verify_certificate::<_, Sha256Digest, N3f1>(
                &mut rng,
                subject,
                &seedless,
                &Sequential
            ),
            "a pre-beacon scheme must bind its epoch too — the oracle-less \
             early return used to skip the check entirely"
        );
    }

    /// The strength a registry cannot see through
    /// [`CertScheme::participants`] / [`CertScheme::me`], and therefore has to
    /// ask for by name. Reds if `is_beacon_active` stops tracking the oracle,
    /// which would make `EpochSchemeProvider::register`'s monotonicity guard
    /// pass a downgrade through.
    #[test]
    fn beacon_activeness_is_visible_and_is_the_difference_a_downgrade_would_lose() {
        let (schemes, seed_ns, sharing, bimap) = committee(4);
        let p = proposal();
        let cert = assemble_over(&schemes, Subject::Finalize { proposal: &p });
        let ns = fluent_namespace(NS_CHAIN);
        let mut rng = StdRng::seed_from_u64(14);

        let strong = CombinedScheme::new(
            VoteScheme::verifier(&ns, bimap.clone()),
            EPOCH,
            Some(TestOracle::arc(&sharing, None, &seed_ns)),
        );
        let weak = CombinedScheme::new(VoteScheme::verifier(&ns, bimap), EPOCH, None);
        assert!(strong.is_beacon_active());
        assert!(!weak.is_beacon_active());

        let stripped = CombinedCertificate {
            vote: cert.vote.clone(),
            seed: None,
        };
        let subject = Subject::Finalize { proposal: &p };
        assert!(
            !strong.verify_certificate::<_, Sha256Digest, N3f1>(
                &mut rng,
                subject,
                &stripped,
                &Sequential
            ),
            "beacon-active: a cleared seed slot is refused"
        );
        assert!(
            weak.verify_certificate::<_, Sha256Digest, N3f1>(
                &mut rng,
                subject,
                &stripped,
                &Sequential
            ),
            "no oracle: the same certificate is admitted — the two verify \
             DIFFERENTLY, so a Some → None replacement is a real downgrade"
        );
    }
}
