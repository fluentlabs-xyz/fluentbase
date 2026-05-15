//! Constructors for `bls12381_multisig::Scheme<PeerPubkey, MinSig>`.
//!
//! `BiMap` ordering: Commonware sorts the participant set by lex byte order
//! of the *key* type (`PeerPubkey` = `ed25519::PublicKey`). The resulting
//! `Participant` index in every Attestation/Certificate/evidence equals the
//! validator's position in this sorted list. Downstream consumers
//! (`01_staking_sol/signer_index_resolution`) MUST mirror this ordering.

use commonware_utils::ordered::BiMap;

use crate::{keys::ValidatorBlsKeypair, BlsPubkey, PeerPubkey, Scheme};

/// Build a signer-capable scheme.
///
/// Takes `&ValidatorBlsKeypair` rather than a raw `Private` so the secret never
/// leaves this crate's encapsulation: the scalar clone happens internally and
/// `ValidatorBlsKeypair::secret()` stays `pub(crate)` (Q-PL-1 resolution).
///
/// Returns `None` if the keypair's public key is not present in `participants`
/// — Commonware uses this case to express “you're not a member of this
/// committee”.
pub fn build_signer(
    namespace: &[u8],
    participants: BiMap<PeerPubkey, BlsPubkey>,
    keypair: &ValidatorBlsKeypair,
) -> Option<Scheme> {
    Scheme::signer(namespace, participants, keypair.secret().clone())
}

/// Build a verifier-only scheme (full nodes, light clients, slashers).
pub fn build_verifier(namespace: &[u8], participants: BiMap<PeerPubkey, BlsPubkey>) -> Scheme {
    Scheme::verifier(namespace, participants)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{keys::ValidatorBlsKeypair, namespace::fluent_namespace};
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
        let peer_sks: Vec<_> = (0..n).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
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
        let scheme = build_signer(&fluent_namespace(20994), bimap, &bls_kps[0]);
        assert!(scheme.is_some());
    }

    #[test]
    fn build_signer_returns_none_for_non_member() {
        let (_, _bls_kps, bimap) = fixture(1, 4);
        // Generate an outsider keypair not in the committee.
        let outsider = ValidatorBlsKeypair::generate(&mut StdRng::seed_from_u64(999));
        let scheme = build_signer(&fluent_namespace(20994), bimap, &outsider);
        assert!(scheme.is_none());
    }

    #[test]
    fn build_verifier_does_not_panic_with_empty_committee() {
        let empty = BiMap::<PeerPubkey, BlsPubkey>::default();
        let _ = build_verifier(&fluent_namespace(20994), empty);
        // Just exercises the constructor; verify-on-empty quorum is Engine concern.
    }
}
