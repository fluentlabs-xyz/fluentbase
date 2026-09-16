//! Adapter from the staking snapshot's [`ValidatorSetSnapshot`] to the typed
//! [`EpochCommittee`] that [`fluentbase_bls`] uses for evidence extraction and
//! BiMap-keyed participant identity.
//!
//! The snapshot stores validators in contract order; commonware re-sorts
//! internally on `BiMap::try_from_iter_dedup` by `PeerPubkey` byte-lex order. The
//! resulting `Participant` index (position in the sorted list) is the
//! protocol-canonical identifier used by simplex's elector and slashing evidence.

use commonware_utils::ordered::Error as OrderedError;
use fluentbase_bls::EpochCommittee;
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;

/// Build the typed [`EpochCommittee`] for one epoch's committee snapshot.
///
/// This is the production constructor: the keys it carries were PoP-verified
/// on-chain by `Staking.setConsensusKeys`.
///
/// Returns `Err` when the snapshot contains duplicate `PeerPubkey` or
/// `BlsPubkey` entries, since commonware's `BiMap` rejects duplicates. The
/// on-chain registration does not enforce cross-validator uniqueness of either
/// key, so a misconfigured operator or a future contract bug surfaces here as a
/// typed error rather than an `.expect(...)` panic at the engine boundary.
pub fn epoch_committee_from_snapshot(
    snap: &ValidatorSetSnapshot,
) -> Result<EpochCommittee, OrderedError> {
    EpochCommittee::from_pairs(
        snap.epoch,
        snap.validators
            .iter()
            .map(|v| (v.keys.peer_pubkey.clone(), v.keys.bls_pubkey)),
    )
}
