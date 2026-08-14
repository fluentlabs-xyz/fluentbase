//! The committee members this node has seen slashed for equivocation, and will
//! therefore neither bind a proposal from nor keep a transport to.

use fluentbase_bls::PeerPubkey;
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use std::{
    collections::HashSet,
    sync::{Arc, PoisonError, RwLock},
};

/// Validators observed tombstoned on chain, keyed by consensus peer key.
///
/// **Driven from chain state, not from held evidence** — that is what makes the
/// reaction survive a restart. A restarted node re-reads the tombstone from the
/// committee snapshot on its first read and reaches the same conclusion; an
/// in-memory list of who misbehaved starts empty.
///
/// The set only grows, and that is sound rather than sloppy: a tombstone is
/// permanent on chain (`Staking`'s `tombstoned` map is written once and never
/// cleared), so an entry can never become wrong. A node's view of it therefore
/// converges from below and never oscillates — the property that lets the
/// refuse-to-bind gate read it without a quorum agreeing on the exact block at
/// which each node noticed.
#[derive(Clone, Default, Debug)]
pub struct TombstoneSet(Arc<RwLock<HashSet<PeerPubkey>>>);

impl TombstoneSet {
    /// Whether this peer has been observed tombstoned.
    pub fn contains(&self, peer: &PeerPubkey) -> bool {
        self.read().contains(peer)
    }

    /// Record every tombstoned member of `snapshot` and return the ones this
    /// call newly added.
    ///
    /// The delta is the return value on purpose: the caller's reaction to a new
    /// tombstone is a transport ban lasting four hours, which must fire once per
    /// peer rather than once per read of a snapshot that keeps reporting the
    /// same permanent flag.
    pub fn observe(&self, snapshot: &ValidatorSetSnapshot) -> Vec<PeerPubkey> {
        let mut set = self.0.write().unwrap_or_else(PoisonError::into_inner);
        snapshot
            .validators
            .iter()
            .filter(|member| member.tombstoned)
            .filter(|member| set.insert(member.keys.peer_pubkey.clone()))
            .map(|member| member.keys.peer_pubkey.clone())
            .collect()
    }

    /// A poisoned lock means a holder panicked mid-update. The set holds owned
    /// keys and cannot be torn, so recovering keeps one panic from disarming the
    /// reaction for the rest of the process — the same discipline the charge
    /// store applies.
    fn read(&self) -> std::sync::RwLockReadGuard<'_, HashSet<PeerPubkey>> {
        self.0.read().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::TombstoneSet;
    use alloy_primitives::{Address, B256};
    use commonware_codec::DecodeExt as _;
    use commonware_cryptography::{ed25519::PrivateKey, Signer as _};
    use commonware_math::algebra::Random as _;
    use fluentbase_bls::{keys::ValidatorBlsKeypair, BlsPubkey, PeerPubkey};
    use fluentbase_staking_reader::reader::{
        ConsensusKeys, ValidatorSetSnapshot, ValidatorWithKeys,
    };
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    fn member(seed: u64, tombstoned: bool) -> ValidatorWithKeys {
        let mut rng = StdRng::seed_from_u64(seed);
        let peer = PrivateKey::random(&mut rng).public_key();
        let bls = BlsPubkey::decode(
            ValidatorBlsKeypair::generate(&mut rng)
                .public_bytes()
                .as_slice(),
        )
        .expect("generated key decodes");
        ValidatorWithKeys {
            address: Address::repeat_byte(seed as u8),
            keys: ConsensusKeys {
                bls_pubkey: bls,
                peer_pubkey: peer,
                activation_epoch: 0,
            },
            stake: 1,
            tombstoned,
        }
    }

    fn snapshot(validators: Vec<ValidatorWithKeys>) -> ValidatorSetSnapshot {
        ValidatorSetSnapshot {
            block_hash: B256::ZERO,
            block_number: 1,
            epoch: 1,
            validators,
        }
    }

    fn peer_of(member: &ValidatorWithKeys) -> PeerPubkey {
        member.keys.peer_pubkey.clone()
    }

    #[test]
    fn only_the_tombstoned_member_is_recorded_and_only_once() {
        let offender = member(1, true);
        let honest = member(2, false);
        let set = TombstoneSet::default();

        let first = set.observe(&snapshot(vec![offender.clone(), honest.clone()]));
        assert_eq!(first, vec![peer_of(&offender)]);
        assert!(set.contains(&peer_of(&offender)));
        assert!(!set.contains(&peer_of(&honest)));

        // The flag is permanent, so the snapshot keeps reporting it; the delta
        // must not, or every read would re-ban a peer already banned.
        let second = set.observe(&snapshot(vec![offender.clone(), honest.clone()]));
        assert!(second.is_empty());
        assert!(set.contains(&peer_of(&offender)));
    }

    /// The committee turns over and the offender leaves it, so later snapshots
    /// stop mentioning it. The refusal must not lapse with the membership.
    #[test]
    fn a_recorded_tombstone_outlives_the_committee_that_carried_it() {
        let offender = member(1, true);
        let successor = member(3, false);
        let set = TombstoneSet::default();
        set.observe(&snapshot(vec![offender.clone()]));

        set.observe(&snapshot(vec![successor.clone()]));

        assert!(set.contains(&peer_of(&offender)));
        assert!(!set.contains(&peer_of(&successor)));
    }
}
