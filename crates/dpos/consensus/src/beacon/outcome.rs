//! Decode + inspect the per-epoch DKG outcome. The aggregated commonware
//! [`Output`] (group key `PK_epoch` + public polynomial + dealer/player sets) is
//! stored as opaque bytes wherever it travels, because `Output`'s decode needs
//! the committee-size config; this module supplies that config and extracts the
//! group public key the seed sub-protocol verifies against and the system call
//! publishes to L2.
//!
//! Its one carrier is the epoch-key agreement plane's artifact
//! ([`crate::beacon::artifact`]).

use commonware_codec::{Encode as _, Read as _};
use commonware_cryptography::bls12381::{
    dkg::Output,
    primitives::{group::Share, sharing::ModeVersion, variant::MinSig},
};
use commonware_utils::ordered::Set;
use core::num::NonZeroU32;
use fluentbase_bls::{beacon::GroupPublic, PeerPubkey};
use fluentbase_p2p::constants::MAX_COMMITTEE_SIZE;

/// Decode cap for an embedded DKG outcome (the encoded commonware `Output`
/// for a committee ≤ `MAX_COMMITTEE_SIZE`: a MinSig public polynomial of
/// degree `quorum-1` in G2 plus the dealer/player/revealed sets). 64 KiB is
/// generous headroom over the ~5 KiB worst case at n=51.
pub(crate) const MAX_BEACON_OUTCOME_SIZE: usize = 64 * 1024;

/// The DKG outcome for our committee: MinSig keys, participants identified by
/// their ed25519 peer pubkey (the commonware participant-ordering key).
pub(crate) type DkgOutcome =
    Output<commonware_cryptography::bls12381::primitives::variant::MinSig, PeerPubkey>;

/// Errors decoding an embedded outcome — any of these means the carrier does not
/// hold a well-formed agreed beacon key.
#[derive(Debug)]
pub(crate) enum OutcomeError {
    /// Bytes are not a valid encoded `Output` for a committee ≤ MAX_COMMITTEE_SIZE.
    Decode(commonware_codec::Error),
    /// Trailing bytes after the outcome (a well-formed `Output` consumes all).
    TrailingBytes,
}

/// Decode the opaque outcome bytes into the typed DKG [`DkgOutcome`],
/// bounding the committee to `MAX_COMMITTEE_SIZE` (NonZeroCounter mode, v0).
pub(crate) fn parse_outcome(bytes: &[u8]) -> Result<DkgOutcome, OutcomeError> {
    let max = NonZeroU32::new(MAX_COMMITTEE_SIZE as u32).expect("MAX_COMMITTEE_SIZE > 0");
    let mut buf = bytes;
    let outcome =
        DkgOutcome::read_cfg(&mut buf, &(max, ModeVersion::v0())).map_err(OutcomeError::Decode)?;
    if !buf.is_empty() {
        return Err(OutcomeError::TrailingBytes);
    }
    Ok(outcome)
}

/// Encode a DKG outcome to the opaque bytes the agreement artifact carries.
pub(crate) fn encode_outcome(outcome: &DkgOutcome) -> Vec<u8> {
    outcome.encode().to_vec()
}

/// The group public key `PK_epoch` — what seeds verify against and what the
/// system call commits to L2.
pub fn group_public_key(outcome: &DkgOutcome) -> &GroupPublic {
    outcome.public().public()
}

/// The boundary qualification gate ("C", share-on-polynomial): a share-holder
/// accepts a proposer-asserted DKG `outcome` for epoch E iff (a) its players are
/// exactly `committee` and the sharing's participant `total` is the committee
/// size, and (b) this node's own secret share lies on the asserted polynomial at
/// its index.
///
/// C alone rejects a forged polynomial that does not pass through the honest
/// shares: to be accepted by a quorum it must agree with the real aggregate at
/// `≥ quorum` player points, which pins a degree-`quorum−1` polynomial to the real
/// aggregate. It is not standalone-sufficient: `Sharing` does not pin the
/// polynomial degree to `quorum−1`, so a high-degree forged polynomial can be
/// fitted through the honest points while carrying an arbitrary constant term.
/// The per-round seed verify the always-active verify path runs alongside C closes
/// that: a seed recovered from the committee's real shares will not verify against
/// a forged `PK_E`, by BLS uniqueness. Callers must run both.
///
/// Observers (no share) cannot run this — the caller must withhold the qualifying
/// vote for them, never accept on shape alone.
///
/// The index is `me`'s seat, not the share's word for it: a share is checked at
/// the point `my_share.index` names, so another member's share would pass the
/// point check at that member's index. It is a share this node cannot sign with,
/// so `my_share.index` must be `me`'s position in `committee`.
pub(crate) fn validate_share_on_poly(
    outcome: &DkgOutcome,
    committee: &Set<PeerPubkey>,
    me: &PeerPubkey,
    my_share: &Share,
) -> bool {
    if outcome.players() != committee || outcome.public().total().get() as usize != committee.len()
    {
        return false;
    }
    if committee.position(me) != Some(usize::from(my_share.index)) {
        return false;
    }
    match outcome.public().partial_public(my_share.index) {
        Ok(pub_share) => pub_share == my_share.public::<MinSig>(),
        Err(_) => false,
    }
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
        use crate::beacon::dkg_oracle::run_local_dkg;
        let mut rng = StdRng::seed_from_u64(13);
        let keys: Vec<Ed25519PrivateKey> = (0..5)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let committee: Set<PeerPubkey> = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));

        let (out_a, shares_a) = run_local_dkg(&mut rng, b"ns", 0, &keys, &keys).expect("dkg a");
        // A different ceremony over the same committee -> a different aggregate poly.
        let (_out_b, shares_b) = run_local_dkg(&mut rng, b"ns", 1, &keys, &keys).expect("dkg b");

        for pk in committee.iter() {
            let mine = shares_a.get(pk).expect("share a");
            assert!(
                validate_share_on_poly(&out_a, &committee, pk, mine),
                "own share must lie on the asserted poly"
            );
            let forged = shares_b.get(pk).expect("share b");
            assert!(
                !validate_share_on_poly(&out_a, &committee, pk, forged),
                "a share from a different ceremony must NOT lie on this poly"
            );
        }

        // Outcome asserted for a different committee -> reject (players mismatch).
        let other: Set<PeerPubkey> =
            Set::from_iter_dedup((0..5).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()));
        let (any_pk, any) = shares_a.iter().next().expect("a share");
        assert!(!validate_share_on_poly(&out_a, &other, any_pk, any));
    }

    /// Another member's share — its own point, on this very polynomial, at that
    /// member's index — is refused for `me`: the point check alone would pass it,
    /// the seat binding does not.
    #[test]
    fn share_on_poly_refuses_another_members_share_at_that_members_index() {
        use crate::beacon::dkg_oracle::run_local_dkg;
        let mut rng = StdRng::seed_from_u64(17);
        let keys: Vec<Ed25519PrivateKey> = (0..5)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let committee: Set<PeerPubkey> = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let (out, shares) = run_local_dkg(&mut rng, b"ns", 0, &keys, &keys).expect("dkg");
        let mut members = committee.iter();
        let (me, other) = (members.next().expect("me"), members.next().expect("other"));
        let others_share = shares.get(other).expect("other's share");
        // Self-verification of the fixture: the share is on the polynomial at its
        // own index — the point check has nothing to refuse.
        assert_eq!(
            out.public()
                .partial_public(others_share.index)
                .expect("point"),
            others_share.public::<MinSig>()
        );
        assert_ne!(
            committee.position(me),
            Some(usize::from(others_share.index))
        );
        assert!(
            validate_share_on_poly(&out, &committee, other, others_share),
            "the share is valid for its owner"
        );
        assert!(
            !validate_share_on_poly(&out, &committee, me, others_share),
            "another member's share, on the polynomial at THAT member's index, is refused for me"
        );
    }

    /// A corrupt recompute (a share from a different ceremony over the same
    /// committee) is caught twice, with no fork surface: the local self-check
    /// returns `false`, so the share is never adopted; and even a hypothetically
    /// adopted corrupt share yields a seed partial rejected per-partial by
    /// `verify_seed_partial`, so it is not counted toward the quorum.
    #[test]
    fn corrupt_recompute_rejected_by_self_check_and_per_partial_verify() {
        use crate::beacon::dkg_oracle::run_local_dkg;
        use commonware_consensus::types::{Epoch, Round, View};
        use fluentbase_bls::beacon::{seed_namespace, sign_seed_partial, verify_seed_partial};
        let mut rng = StdRng::seed_from_u64(31);
        let keys: Vec<Ed25519PrivateKey> = (0..5)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let committee: Set<PeerPubkey> = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let (outcome, real_shares) =
            run_local_dkg(&mut rng, b"ns", 0, &keys, &keys).expect("real dkg");
        // A different ceremony over the same committee → a corrupt (wrong-poly) share.
        let (_bad, bad_shares) = run_local_dkg(&mut rng, b"ns", 1, &keys, &keys).expect("bad dkg");

        let seed_ns = seed_namespace(b"ns");
        let round = Round::new(Epoch::new(0), View::new(7));
        for pk in committee.iter() {
            let real_share = real_shares.get(pk).expect("real share");
            let corrupt_share = bad_shares.get(pk).expect("corrupt share");
            // (i) the local self-check adopts the real share, rejects the corrupt one.
            assert!(
                validate_share_on_poly(&outcome, &committee, pk, real_share),
                "the correct recomputed share self-verifies (adopted)"
            );
            assert!(
                !validate_share_on_poly(&outcome, &committee, pk, corrupt_share),
                "a corrupt recomputed share FAILS the self-check → never adopted (no fork)"
            );
            // (ii) per-partial belt: a partial under the corrupt share is rejected.
            let real_partial = sign_seed_partial(real_share, &seed_ns, round);
            let corrupt_partial = sign_seed_partial(corrupt_share, &seed_ns, round);
            assert!(
                verify_seed_partial(outcome.public(), &seed_ns, round, &real_partial),
                "an honest partial verifies against the group polynomial"
            );
            assert!(
                !verify_seed_partial(outcome.public(), &seed_ns, round, &corrupt_partial),
                "a corrupt-share partial is rejected PER-PARTIAL → not counted → no Nullify storm"
            );
        }
    }

    /// A different `PK_E` over the same committee that is not trivially
    /// shape-rejected (players and total match) yet fails every honest
    /// share-holder's C gate.
    #[test]
    fn forge_differs_in_pk_keeps_committee_shape_and_fails_honest_c() {
        use crate::beacon::dkg_oracle::run_local_dkg;
        let mut rng = StdRng::seed_from_u64(21);
        let keys: Vec<Ed25519PrivateKey> = (0..5)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let committee: Set<PeerPubkey> = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let (real, real_shares) = run_local_dkg(&mut rng, b"ns", 0, &keys, &keys).expect("dkg");

        let forged = crate::byzantine::forge_outcome_same_committee(&real);

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
        // It is decodable through the wire path verify uses, so it is not a
        // trivially-rejected malformed outcome — it reaches the C check.
        let bytes = encode_outcome(&forged);
        assert!(parse_outcome(&bytes).is_ok(), "forge round-trips the codec");
        // Yet every honest share-holder's C gate rejects it (their real share does
        // not lie on the forged poly).
        for pk in committee.iter() {
            let honest_share = real_shares.get(pk).expect("real share");
            assert!(
                !validate_share_on_poly(&forged, &committee, pk, honest_share),
                "an honest share must NOT lie on the forged polynomial"
            );
        }
    }
}
