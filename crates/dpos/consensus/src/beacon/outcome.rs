//! Decode + inspect the per-epoch DKG outcome embedded in a boundary
//! `OrderBlock` (`beacon_outcome`). The aggregated commonware [`Output`]
//! (group key `PK_epoch` + public polynomial + dealer/player sets) is stored as
//! opaque bytes at the block-codec layer because `Output`'s decode needs the
//! committee-size config; this module supplies that config and extracts the
//! group public key the seed sub-protocol verifies against and the system call
//! publishes to L2.

use commonware_codec::{Encode as _, Read as _};
use commonware_cryptography::bls12381::{
    dkg::Output,
    primitives::{group::Share, sharing::ModeVersion, variant::MinSig},
};
use commonware_utils::ordered::Set;
use core::num::NonZeroU32;
use fluentbase_bls::PeerPubkey;
use fluentbase_p2p::constants::MAX_COMMITTEE_SIZE;

use crate::beacon::seed::GroupPublic;

/// The DKG outcome for our committee: MinSig keys, participants identified by
/// their ed25519 peer pubkey (the commonware participant-ordering key).
pub type DkgOutcome =
    Output<commonware_cryptography::bls12381::primitives::variant::MinSig, PeerPubkey>;

/// Errors decoding an embedded outcome — any of these means the boundary block
/// does not carry a well-formed agreed beacon key.
#[derive(Debug)]
pub enum OutcomeError {
    /// Bytes are not a valid encoded `Output` for a committee ≤ MAX_COMMITTEE_SIZE.
    Decode(commonware_codec::Error),
    /// Trailing bytes after the outcome (a well-formed `Output` consumes all).
    TrailingBytes,
}

/// Decode the embedded `beacon_outcome` bytes into the typed DKG [`DkgOutcome`],
/// bounding the committee to `MAX_COMMITTEE_SIZE` (NonZeroCounter mode, v0).
pub fn parse_outcome(bytes: &[u8]) -> Result<DkgOutcome, OutcomeError> {
    let max = NonZeroU32::new(MAX_COMMITTEE_SIZE as u32).expect("MAX_COMMITTEE_SIZE > 0");
    let mut buf = bytes;
    let outcome =
        DkgOutcome::read_cfg(&mut buf, &(max, ModeVersion::v0())).map_err(OutcomeError::Decode)?;
    if !buf.is_empty() {
        return Err(OutcomeError::TrailingBytes);
    }
    Ok(outcome)
}

/// Encode a DKG outcome to the opaque bytes carried in `OrderBlock.beacon_outcome`.
pub fn encode_outcome(outcome: &DkgOutcome) -> Vec<u8> {
    outcome.encode().to_vec()
}

/// The group public key `PK_epoch` — what seeds verify against and what the
/// system call commits to L2.
pub fn group_public_key(outcome: &DkgOutcome) -> &GroupPublic {
    outcome.public().public()
}

/// The boundary qualification gate ("C", share-on-polynomial): a SHARE-HOLDER
/// accepts a proposer-asserted DKG `outcome` for epoch E iff (a) its players are
/// exactly `committee` (committee[E]) AND the sharing's participant `total` is the
/// committee size (so the polynomial index domain is exactly the committee), and
/// (b) this node's OWN secret share lies on the asserted aggregate polynomial at
/// its index (`g^{sk_j} == outcome.public().partial_public(j)`).
///
/// What C alone guarantees: a forged polynomial that does NOT pass through the
/// honest shares is rejected — to be accepted by a quorum it must agree with the
/// real aggregate at ≥ `quorum` player points. For a polynomial of degree
/// `quorum−1` (the honest aggregate's degree) that pins it to the real aggregate
/// (a degree-`d` poly is determined by `d+1` points) ⟹ the real `PK_E`.
///
/// IMPORTANT CAVEAT — C is NOT standalone-sufficient: commonware `Sharing` does
/// not expose, nor does its decoder pin, the polynomial degree (`Sharing::Read`
/// bounds the coefficient count only to `≤ MAX_COMMITTEE_SIZE`), and the quorum
/// `q = n−f ≈ 2n/3 < n`. So a HIGH-degree forged poly (degree up to `n−1`) can be
/// fitted through the `q` honest public share points while carrying an ARBITRARY
/// constant term (a forged `PK_E`). C cannot detect that here. It is closed by the
/// per-round SEED-VERIFY that the always-active verify path runs ALONGSIDE C at the
/// boundary: the seed `σ₀` recovered from the committee's REAL shares is the unique
/// threshold signature under the real secret and will NOT verify against a forged
/// `PK_E = poly.constant()` (BLS uniqueness). So the COMBINED gate (C ∧
/// `verify_seed(σ₀, PK_E)`) is sound: C cheaply rejects polys that miss the honest
/// shares; seed-verify rejects a high-degree poly with a forged constant. Callers
/// MUST run both — C is necessary, not sufficient. A pure `verify_seed` gate alone
/// would be forgeable (a proposer mints its own keypair `(x, g^x)`); C binds the
/// poly to the validators' own (uncontrolled) shares. `outcome.dealers()` is the
/// quorum-sized qualified set `Q` (a SUBSET of the committee), so it is
/// intentionally NOT required to equal `committee`.
///
/// Observers (no share) cannot run this — the caller must WITHHOLD the qualifying
/// vote for them, never accept on shape alone.
pub fn validate_share_on_poly(
    outcome: &DkgOutcome,
    committee: &Set<PeerPubkey>,
    my_share: &Share,
) -> bool {
    if outcome.players() != committee
        || outcome.public().total().get() as usize != committee.len()
    {
        return false;
    }
    match outcome.public().partial_public(my_share.index) {
        Ok(pub_share) => pub_share == my_share.public::<MinSig>(),
        Err(_) => false,
    }
}

/// DEVNET/TEST-ONLY forge of a different per-epoch DKG outcome over the SAME
/// committee. Deals a fresh anonymous DKG to `real.players()` (the proposer holds
/// the committee's PUBLIC peer set, never the other validators' shares) with a
/// fixed devnet RNG, yielding an `Output` whose `players()`/`total()` match the
/// real committee — so it passes the epoch-type/shape gate — but whose `PK_E`
/// differs. The forged polynomial does NOT thread the honest shares, so each
/// honest share-holder's "C" gate ([`validate_share_on_poly`]) rejects it at
/// verify; under the realistic `f=1` bound the forge cannot reach a notarization
/// quorum, so it never finalizes (the consensus-level SAFETY observable). The
/// certify hook ([`crate::beacon::certify`]) is the closure that would Nullify it
/// IF a colluding byzantine quorum did notarize it — exercised by the gated
/// certify tests where the collusion is constructible.
///
/// Gated behind `dpos-devnet-byzantine` (or `test`): the forge can never be reached
/// in a production build.
#[cfg(any(test, feature = "dpos-devnet-byzantine"))]
pub fn forge_outcome_same_committee(real: &DkgOutcome) -> DkgOutcome {
    use commonware_cryptography::bls12381::{dkg::deal, primitives::sharing::Mode};
    use commonware_utils::N3f1;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    // Fixed devnet seed → deterministic forge (every byzantine node forges the
    // identical PK_E for a given committee, so they collude on one value).
    let mut rng = StdRng::seed_from_u64(0xB17E_FACE);
    let players = real.players().clone();
    let (forged, _shares) = deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
        .expect("forge: deal over the real committee's public players");
    debug_assert_ne!(
        forged.public().public(),
        real.public().public(),
        "a forged outcome must carry a different PK_E than the real one"
    );
    forged
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_cryptography::{
        bls12381::{
            dkg::deal,
            primitives::{sharing::Mode, variant::MinSig},
        },
        ed25519::PrivateKey as Ed25519PrivateKey,
        Signer as _,
    };
    use commonware_math::algebra::Random as _;
    use commonware_utils::{ordered::Set, N3f1};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    fn deal_outcome(n: u32) -> DkgOutcome {
        let mut rng = StdRng::seed_from_u64(7);
        let players: Set<PeerPubkey> =
            Set::from_iter_dedup((0..n).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()));
        let (outcome, _shares) =
            deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
                .expect("deal");
        outcome
    }

    #[test]
    fn outcome_encode_parse_roundtrip_and_group_key() {
        let outcome = deal_outcome(5);
        let bytes = encode_outcome(&outcome);
        let parsed = parse_outcome(&bytes).expect("parse");
        assert_eq!(parsed, outcome, "embedded outcome must round-trip exactly");
        assert_eq!(
            group_public_key(&parsed),
            group_public_key(&outcome),
            "every node derives the same PK_epoch from the embedded outcome"
        );
    }

    #[test]
    fn truncated_outcome_is_rejected() {
        let outcome = deal_outcome(5);
        let bytes = encode_outcome(&outcome);
        assert!(parse_outcome(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let outcome = deal_outcome(5);
        let mut bytes = encode_outcome(&outcome);
        bytes.push(0xFF);
        assert!(matches!(
            parse_outcome(&bytes),
            Err(OutcomeError::TrailingBytes)
        ));
    }

    #[test]
    fn share_on_poly_accepts_own_rejects_forged_and_wrong_committee() {
        use crate::beacon::dkg::run_local_dkg;
        let mut rng = StdRng::seed_from_u64(13);
        let keys: Vec<Ed25519PrivateKey> =
            (0..5).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
        let committee: Set<PeerPubkey> =
            Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));

        let (out_a, shares_a) = run_local_dkg(&mut rng, b"ns", 0, &keys, &keys).expect("dkg a");
        // A DIFFERENT ceremony over the SAME committee -> a different aggregate poly.
        let (_out_b, shares_b) = run_local_dkg(&mut rng, b"ns", 1, &keys, &keys).expect("dkg b");

        for pk in committee.iter() {
            let mine = shares_a.get(pk).expect("share a");
            assert!(
                validate_share_on_poly(&out_a, &committee, mine),
                "own share must lie on the asserted poly"
            );
            let forged = shares_b.get(pk).expect("share b");
            assert!(
                !validate_share_on_poly(&out_a, &committee, forged),
                "a share from a different ceremony must NOT lie on this poly"
            );
        }

        // Outcome asserted for a DIFFERENT committee -> reject (players mismatch).
        let other: Set<PeerPubkey> = Set::from_iter_dedup(
            (0..5).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()),
        );
        let any = shares_a.values().next().expect("a share");
        assert!(!validate_share_on_poly(&out_a, &other, any));
    }

    /// The devnet byzantine forge: a DIFFERENT `PK_E` over the SAME committee that
    /// is NOT trivially shape-rejected (players + total match the committee) yet
    /// FAILS every honest share-holder's "C" gate — exactly the Track-1 SAFETY
    /// mechanism (the forge cannot pass C → cannot reach a quorum at `f=1`).
    #[test]
    fn forge_differs_in_pk_keeps_committee_shape_and_fails_honest_c() {
        use crate::beacon::dkg::run_local_dkg;
        let mut rng = StdRng::seed_from_u64(21);
        let keys: Vec<Ed25519PrivateKey> =
            (0..5).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
        let committee: Set<PeerPubkey> = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let (real, real_shares) = run_local_dkg(&mut rng, b"ns", 0, &keys, &keys).expect("dkg");

        let forged = forge_outcome_same_committee(&real);

        assert_ne!(
            group_public_key(&forged),
            group_public_key(&real),
            "the forge must assert a DIFFERENT PK_E"
        );
        assert_eq!(
            forged.players(),
            real.players(),
            "the forge keeps players == committee (passes the shape/epoch-type gate)"
        );
        assert_eq!(
            forged.public().total(),
            real.public().total(),
            "the forge keeps total == committee size (passes the shape gate)"
        );
        // It is decodable through the wire path verify uses, so it is NOT a
        // trivially-rejected malformed outcome — it reaches the C check.
        let bytes = encode_outcome(&forged);
        assert!(parse_outcome(&bytes).is_ok(), "forge round-trips the codec");
        // Yet EVERY honest share-holder's C gate rejects it (their real share does
        // not lie on the forged poly).
        for pk in committee.iter() {
            let honest_share = real_shares.get(pk).expect("real share");
            assert!(
                !validate_share_on_poly(&forged, &committee, honest_share),
                "an honest share must NOT lie on the forged polynomial"
            );
        }
    }
}
