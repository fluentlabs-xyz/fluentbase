//! Constructors for `bls12381_multisig::Scheme<PeerPubkey, MinSig>`.
//!
//! `BiMap` ordering: Commonware sorts the participant set by lex byte order
//! of the *key* type (`PeerPubkey` = `ed25519::PublicKey`). The resulting
//! `Participant` index in every Attestation/Certificate/evidence equals the
//! validator's position in this sorted list. Downstream consumers
//! (the on-chain signer-index resolution in `Staking.sol`) MUST mirror
//! this ordering.

use commonware_cryptography::bls12381::primitives::{group::Share, sharing::Sharing, variant::MinSig};
use commonware_utils::{ordered::BiMap, TryCollect};

use crate::{
    combined_scheme::CombinedScheme, keys::ValidatorBlsKeypair, BlsPubkey, PeerPubkey, Scheme,
    VoteScheme,
};

/// Per-epoch threshold beacon material for the [`CombinedScheme`]: the public
/// polynomial `PK_epoch`, this node's share (`None` for a verifier-only / no-share
/// node), and the seed namespace. `None` for the whole tuple ⇒ a fallback
/// (pure-multisig) epoch.
pub type BeaconKey = (Sharing<MinSig>, Option<Share>, Vec<u8>);

/// Per-epoch consensus committee: an epoch identifier paired with the
/// commonware-sorted `BiMap<PeerPubkey, BlsPubkey>` that defines the
/// Simplex Participant index for that epoch.
///
/// Invariant carried by [`Self::from_pairs`]: every pubkey in the resulting
/// BiMap has had its Proof-of-Possession verified on-chain at
/// `Staking.setConsensusKeys` time. This type trusts the on-chain contract
/// and does not re-verify PoP at construction. The test-only
/// [`Self::from_unverified`] constructor relaxes this contract.
#[derive(Clone, Debug)]
pub struct EpochCommittee {
    /// On-chain epoch identifier — used by the consensus slasher's
    /// `evidence::extract_from_*` to assert the evidence's claimed epoch
    /// matches the committee.
    pub epoch: u64,
    /// Commonware-sorted participant BiMap; signer indices in
    /// `Activity::Conflicting*` reference slots in this BiMap.
    pub bimap: BiMap<PeerPubkey, BlsPubkey>,
}

impl EpochCommittee {
    /// Trusted constructor: caller guarantees the pubkeys passed PoP
    /// on-chain. Production callers go through
    /// `fluentbase_consensus::scheme::epoch_committee_from_snapshot`
    /// (reads a frozen on-chain committee at a finalized hash).
    pub fn from_pairs<I>(epoch: u64, pairs: I) -> Result<Self, commonware_utils::ordered::Error>
    where
        I: IntoIterator<Item = (PeerPubkey, BlsPubkey)>,
    {
        let bimap: BiMap<PeerPubkey, BlsPubkey> = pairs.into_iter().try_collect()?;
        Ok(Self { epoch, bimap })
    }

    /// Test-only constructor — does NOT carry the PoP-verified invariant.
    /// Marked `doc(hidden)` to discourage production use.
    #[doc(hidden)]
    pub fn from_unverified(epoch: u64, bimap: BiMap<PeerPubkey, BlsPubkey>) -> Self {
        Self { epoch, bimap }
    }
}

/// Build a signer-capable scheme.
///
/// Takes `&ValidatorBlsKeypair` rather than a raw `Private` so the secret never
/// leaves this crate's encapsulation: the scalar clone happens internally and
/// `ValidatorBlsKeypair::secret()` stays `pub(crate)`.
///
/// Returns `None` if the keypair's public key is not present in `participants`
/// — Commonware uses this case to express “you're not a member of this
/// committee”.
pub fn build_signer(
    namespace: &[u8],
    participants: BiMap<PeerPubkey, BlsPubkey>,
    keypair: &ValidatorBlsKeypair,
    beacon: Option<BeaconKey>,
) -> Option<Scheme> {
    let vote = VoteScheme::signer(namespace, participants, keypair.secret().clone())?;
    Some(CombinedScheme::new(vote, beacon))
}

/// Build a verifier-only scheme (full nodes, light clients, slashers). A
/// `beacon` part (with `share = None`) lets the verifier check recovered seeds
/// against `PK_epoch`; `None` ⇒ a fallback (pure-multisig) epoch.
pub fn build_verifier(
    namespace: &[u8],
    participants: BiMap<PeerPubkey, BlsPubkey>,
    beacon: Option<BeaconKey>,
) -> Scheme {
    let vote = VoteScheme::verifier(namespace, participants);
    CombinedScheme::new(vote, beacon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fluent_namespace, keys::ValidatorBlsKeypair};
    use commonware_codec::DecodeExt;
    use commonware_cryptography::ed25519::PrivateKey as Ed25519PrivateKey;
    use commonware_cryptography::Signer;
    use commonware_math::algebra::Random;
    use commonware_utils::TryCollect;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;

    fn fixture(
        seed: u64,
        n: usize,
    ) -> (
        Vec<Ed25519PrivateKey>,
        Vec<ValidatorBlsKeypair>,
        BiMap<PeerPubkey, BlsPubkey>,
    ) {
        let mut rng = StdRng::seed_from_u64(seed);
        let peer_sks: Vec<_> = (0..n)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let bls_kps: Vec<_> = (0..n)
            .map(|_| ValidatorBlsKeypair::generate(&mut rng))
            .collect();
        let pairs = peer_sks.iter().zip(bls_kps.iter()).map(|(p, b)| {
            (p.public_key(), {
                let bytes = b.public_bytes();
                BlsPubkey::decode(bytes.as_slice()).unwrap()
            })
        });
        let bimap: BiMap<_, _> = pairs.try_collect().unwrap();
        (peer_sks, bls_kps, bimap)
    }

    #[test]
    fn build_signer_succeeds_for_member() {
        let (_, bls_kps, bimap) = fixture(1, 4);
        let scheme = build_signer(&fluent_namespace(20994), bimap, &bls_kps[0], None);
        assert!(scheme.is_some());
    }

    #[test]
    fn build_signer_returns_none_for_non_member() {
        let (_, _bls_kps, bimap) = fixture(1, 4);
        // Generate an outsider keypair not in the committee.
        let outsider = ValidatorBlsKeypair::generate(&mut StdRng::seed_from_u64(999));
        let scheme = build_signer(&fluent_namespace(20994), bimap, &outsider, None);
        assert!(scheme.is_none());
    }

    #[test]
    fn build_verifier_does_not_panic_with_empty_committee() {
        let empty = BiMap::<PeerPubkey, BlsPubkey>::default();
        let _ = build_verifier(&fluent_namespace(20994), empty, None);
        // Just exercises the constructor; verify-on-empty quorum is Engine concern.
    }
}
