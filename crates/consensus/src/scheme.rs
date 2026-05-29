//! Adapter from 03's [`ValidatorSetSnapshot`] to the typed
//! [`EpochCommittee`] that [`fluentbase_bls`] uses for evidence
//! extraction and BiMap-keyed participant identity.
//!
//! The snapshot stores validators in **contract order, verbatim**;
//! commonware re-sorts internally on `BiMap::try_from_iter_dedup`
//! by `PeerPubkey` byte-lex order. The resulting `Participant` index
//! (= position in the sorted list) is the protocol-canonical identifier
//! used by simplex's elector and slashing evidence.

use commonware_utils::ordered::Error as OrderedError;
use fluentbase_bls::EpochCommittee;
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;

/// Build the typed [`EpochCommittee`] for one epoch's committee snapshot.
///
/// This is the production constructor: it carries the on-chain PoP-verified
/// invariant from `Staking.setConsensusKeys` (every key in the resulting
/// BiMap passed `BLS12381Verifier.verify` on-chain at registration time).
///
/// Returns `Err` when the snapshot contains duplicate `PeerPubkey` or
/// `BlsPubkey` entries (commonware's `BiMap` rejects dups). The on-chain
/// `setConsensusKeys` does NOT currently enforce cross-validator uniqueness
/// of either key; a misconfigured operator or
/// future contract bug surfaces here as a typed error rather than the
/// previous `.expect(...)` panic at the engine boundary.
/// **** мне кажется слишком жирно отдельный файл, найди лучышее место
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
