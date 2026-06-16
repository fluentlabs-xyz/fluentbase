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
//! the test oracle; a single-process devnet bootstrap can call it directly.

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
        let sig = recover_seed(outcome.public(), &partials).expect("recover seed");
        assert!(
            verify_seed(group_public_key(&outcome), &ns, round, &sig),
            "seed from DKG shares must verify against the DKG group key PK_epoch"
        );
    }
}
