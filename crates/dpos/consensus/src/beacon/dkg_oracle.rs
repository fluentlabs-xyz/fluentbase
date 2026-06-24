//! Per-epoch DKG ceremony orchestration over commonware Joint-Feldman.
//!
//! The committee active in epoch E-1 deals a FRESH threshold key (`previous =
//! None`) to committee[E]: each dealer broadcasts its public commitment, sends
//! each player a private share, collects acks, and finalizes a signed dealer
//! log; players/observers aggregate the logs into the group `Output` (`PK_E` +
//! public polynomial) and their own secret share.
//!
//! [`run_local_dkg`] drives all roles in one process — the exact sequence the
//! async actor performs with real `BEACON_CHANNEL` message passing (each node
//! runs only its own dealer + player). It is the orchestration reference and
//! the test oracle — `#[cfg(test)]`-only (the networked `DkgActor`/`DkgCeremony`
//! path is what runs in production); see the module gate in `beacon/mod.rs`.
//
// TODO(dpos_vrf_live_dkg, v2 — reshare-heal on observers): cadence v1 runs a
// FRESH ceremony only on committee CHANGE and carry-forwards an unchanged
// committee. That never heals an OBSERVER (a committee member with no share —
// missed the E-1 window): under a long-stable committee it stays share-less,
// so the beacon's share-holder margin (tolerates <= f non-share-holders before
// the no-quorum stall) erodes silently with no repair path. The fix is a
// RESHARE (`Dealer::start(previous = Some(existing))`, NOT a fresh key) by the
// current share-holders: re-distributes the SAME secret to the full player set
// (incl. observers) so `PK_E` stays identical (no boundary re-commit / STF
// continuity break) while observers gain shares. Trigger must be CONDITIONAL
// (only when observers exist / margin drops) — not blind-periodic (the reason
// periodic reshare was rejected) — AND consensus-AGREED (observer-ness is local;
// an un-agreed trigger reintroduces the silent-failure liveness surface), and it
// needs a dedicated reshare ceremony-result log. Heal BEFORE observers exceed f,
// or the reshare itself can't reach a dealer quorum.

use crate::beacon::outcome::DkgOutcome;
use commonware_cryptography::{
    bls12381::{
        dkg::{Dealer, Error as DkgError, Info, Logs, Player},
        primitives::{group::Share, sharing::Mode, variant::MinSig},
    },
    ed25519::{self, PrivateKey as Ed25519PrivateKey},
    Signer as _,
};
use commonware_parallel::Sequential;
use commonware_utils::{ordered::Set, N3f1};
use fluentbase_bls::PeerPubkey;
use rand_core::CryptoRngCore;
use std::collections::BTreeMap;

/// Run a full Joint-Feldman DKG to completion (fresh key, `previous = None`),
/// returning the agreed group [`DkgOutcome`] (== `PK_epoch`) and each player's
/// secret [`Share`]. Every player computes the SAME outcome by construction.
pub fn run_local_dkg<R: CryptoRngCore>(
    rng: &mut R,
    namespace: &[u8],
    round: u64,
    dealer_keys: &[Ed25519PrivateKey],
    player_keys: &[Ed25519PrivateKey],
) -> Result<(DkgOutcome, BTreeMap<PeerPubkey, Share>), DkgError> {
    let dealers = Set::from_iter_dedup(dealer_keys.iter().map(|k| k.public_key()));
    let players = Set::from_iter_dedup(player_keys.iter().map(|k| k.public_key()));
    let info = Info::<MinSig, PeerPubkey>::new::<N3f1>(
        namespace,
        round,
        None,
        Mode::NonZeroCounter,
        dealers,
        players,
    )?;

    let mut player_objs: BTreeMap<PeerPubkey, Player<MinSig, Ed25519PrivateKey>> = BTreeMap::new();
    for k in player_keys {
        player_objs.insert(k.public_key(), Player::new(info.clone(), k.clone())?);
    }

    // Dealing round: each dealer distributes, collects acks, finalizes a log.
    let mut logs = Logs::<MinSig, PeerPubkey, N3f1>::new(info.clone());
    for dk in dealer_keys {
        let (mut dealer, pub_msg, priv_msgs) =
            Dealer::start::<N3f1>(&mut *rng, info.clone(), dk.clone(), None)?;
        for (player_pk, priv_msg) in priv_msgs {
            if let Some(player) = player_objs.get_mut(&player_pk) {
                if let Some(ack) =
                    player.dealer_message::<N3f1>(dk.public_key(), pub_msg.clone(), priv_msg)
                {
                    dealer.receive_player_ack(player_pk, ack)?;
                }
            }
        }
        if let Some((dealer_pk, log)) = dealer.finalize::<N3f1>().check(&info) {
            logs.record(dealer_pk, log);
        }
    }

    // Finalize: each player derives its share + the agreed group output.
    let mut shares = BTreeMap::new();
    let mut outcome = None;
    for (pk, player) in player_objs {
        let (out, share) =
            player.finalize::<N3f1, ed25519::Batch>(&mut *rng, logs.clone(), &Sequential)?;
        shares.insert(pk, share);
        outcome = Some(out);
    }
    // `Info::new` rejects an empty player set, so at least one player finalized.
    Ok((outcome.expect("players non-empty after Info::new"), shares))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beacon::outcome::group_public_key;
    use commonware_consensus::types::{Epoch, Round, View};
    use commonware_math::algebra::Random as _;
    use fluentbase_bls::beacon::{recover_seed, seed_namespace, sign_seed_partial, verify_seed};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    #[test]
    fn interactive_dkg_agrees_and_feeds_a_verifiable_seed() {
        let mut rng = StdRng::seed_from_u64(9);
        let keys: Vec<Ed25519PrivateKey> = (0..5)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();

        // Fresh full DKG: the active committee deals to itself (dealers==players).
        let (outcome, shares) =
            run_local_dkg(&mut rng, b"FLUENT_DPOS_V1_test", 0, &keys, &keys).expect("dkg");
        assert_eq!(shares.len(), keys.len(), "every player gets a share");

        // The DKG group key drives the per-round seed: sign with the DKG shares,
        // recover, and verify against PK_epoch — the full DKG → seed → verify path.
        let ns = seed_namespace(b"FLUENT_DPOS_V1_test");
        let round = Round::new(Epoch::new(0), View::new(100));
        let partials: Vec<_> = shares
            .values()
            .map(|s| sign_seed_partial(s, &ns, round))
            .collect();
        let sig = recover_seed::<N3f1>(outcome.public(), &partials).expect("recover seed");
        assert!(
            verify_seed(group_public_key(&outcome), &ns, round, &sig),
            "seed from DKG shares must verify against the DKG group key PK_epoch"
        );
    }

    // Item F: PK_E + seed determinism at the PRODUCTION committee size (n=51,
    // f=16, MAX_PEER_SET_SIZE). The threshold seed is UNIQUE — any two distinct
    // n−f (=35) seed-quorum subsets of the 51 partials recover the byte-identical signature
    // — which is exactly why every deriving node computes the same prev_randao
    // regardless of which quorum it observed. (Cross-player PK_E agreement is a
    // commonware DKG guarantee; this asserts OUR seed machinery + quorum math at
    // scale, not the library internals.) The n=51 ceremony completing in this
    // unit confirms DKG feasibility at scale; the per-block gossip reachability
    // within DKG_MARGIN_BLOCKS=10 is exercised by the rotation smoke at the real
    // committee size, so the margin constant is left at 10.
    #[test]
    fn seed_is_threshold_unique_at_n51() {
        use commonware_codec::Encode as _;

        let mut rng = StdRng::seed_from_u64(51);
        let keys: Vec<Ed25519PrivateKey> = (0..51)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let (outcome, shares) =
            run_local_dkg(&mut rng, b"FLUENT_DPOS_V1_test", 0, &keys, &keys).expect("n=51 dkg");
        assert_eq!(shares.len(), 51, "all 51 players get a share at n=51/f=16");

        let ns = seed_namespace(b"FLUENT_DPOS_V1_test");
        let round = Round::new(Epoch::new(0), View::new(100));
        let partials: Vec<_> = shares
            .values()
            .map(|s| sign_seed_partial(s, &ns, round))
            .collect();

        // Two DISTINCT seed-quorum subsets (n−f = 51−16 = 35) must recover the SAME
        // signature. [0..35] and [16..51] are different 35-member sets.
        let seed_all = recover_seed::<N3f1>(outcome.public(), &partials).expect("recover all");
        let seed_a =
            recover_seed::<N3f1>(outcome.public(), &partials[0..35]).expect("recover subset a");
        let seed_b =
            recover_seed::<N3f1>(outcome.public(), &partials[16..51]).expect("recover subset b");
        assert_eq!(
            seed_all.encode().as_ref(),
            seed_a.encode().as_ref(),
            "an n−f quorum subset recovers the canonical seed"
        );
        assert_eq!(
            seed_a.encode().as_ref(),
            seed_b.encode().as_ref(),
            "a different n−f quorum subset recovers the byte-identical seed (threshold uniqueness)"
        );
        assert!(
            verify_seed(group_public_key(&outcome), &ns, round, &seed_all),
            "the n=51 recovered seed verifies against PK_epoch"
        );
    }
}
