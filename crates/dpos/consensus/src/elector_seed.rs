//! Per-epoch RoundRobin shuffle seed.
//!
//! `RandomElector` needs threshold-VRF certificates; Fluent uses
//! `bls12381_multisig` (no threshold) ⇒ `RoundRobin` is entailed. To
//! kill cross-epoch leader-schedule precomputation at zero cost, the
//! per-epoch `RoundRobin::shuffled(seed)` is fed a seed deterministically
//! derived from the epoch's **on-chain frozen committee** — identical
//! across all honest nodes (same `Staking.sol` state) so the leader
//! permutation cannot desync.

use commonware_cryptography::{Hasher, Sha256};
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;

/// `seed = sha256(epoch_be || sorted_peer_pubkey_0 || sorted_peer_pubkey_1 || …)`
/// over the committee canonicalized by raw byte-lex of `peer_pubkey`.
/// Deterministic and network-identical; unpredictable until the epoch's
/// committee is committed on-chain.
///
/// Explicitly sort by `peer_pubkey` before hashing so the seed is
/// invariant under any snapshot iteration-order quirk. Today
/// `snap.validators` happens to be in contract ascending-`peerPubkey` order
/// (commitEpochCommittee enforces strict-ascending order on-chain) and
/// commonware's `BiMap` sorts the same way — but relying on that
/// coincidence as a safety pin is fragile. The sort here makes the seed
/// independent of upstream ordering changes; the leader schedule cannot
/// desync between honest nodes that observe the same epoch's keys in any
/// order.
pub(crate) fn epoch_leader_seed(snap: &ValidatorSetSnapshot) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(&snap.epoch.to_be_bytes());
    let mut peers: Vec<&[u8]> = snap
        .validators
        .iter()
        .map(|v| v.keys.peer_pubkey.as_ref())
        .collect();
    peers.sort_unstable();
    for p in peers {
        h.update(p);
    }
    <[u8; 32]>::try_from(h.finalize().as_ref()).expect("sha256 digest is 32 bytes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_codec::DecodeExt as _;
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer};
    use commonware_math::algebra::Random as _;
    use fluentbase_bls::BlsPubkey;
    use fluentbase_staking_reader::reader::{ConsensusKeys, ValidatorWithKeys};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;

    fn snap(epoch: u64, n: u64) -> ValidatorSetSnapshot {
        let validators = (0..n)
            .map(|i| {
                let mut rng = StdRng::seed_from_u64(epoch * 1000 + i);
                let peer = Ed25519PrivateKey::random(&mut rng).public_key();
                let bls = BlsPubkey::decode(
                    fluentbase_bls::keys::ValidatorBlsKeypair::generate(&mut rng)
                        .public_bytes()
                        .as_slice(),
                )
                .unwrap();
                ValidatorWithKeys {
                    address: alloy_primitives::Address::repeat_byte(i as u8),
                    keys: ConsensusKeys {
                        bls_pubkey: bls,
                        peer_pubkey: peer,
                        activation_epoch: 1,
                    },
                }
            })
            .collect();
        ValidatorSetSnapshot {
            block_hash: alloy_primitives::B256::repeat_byte(0xCC),
            block_number: epoch * 100,
            epoch,
            validators,
        }
    }

    #[test]
    fn deterministic_same_input_same_seed() {
        assert_eq!(
            epoch_leader_seed(&snap(7, 5)),
            epoch_leader_seed(&snap(7, 5))
        );
    }

    #[test]
    fn distinct_epoch_distinct_seed() {
        assert_ne!(
            epoch_leader_seed(&snap(7, 5)),
            epoch_leader_seed(&snap(8, 5))
        );
    }

    #[test]
    fn seed_invariant_under_reverse_order() {
        // Seed is canonicalized by sorting peer pubkeys
        // before hashing, so any input-order permutation of the same
        // committee yields the same seed. (Pre-fix this asserted `_ne`
        // — the seed was order-sensitive and relied on contract-order
        // / commonware-BiMap-order coincidence.)
        let mut a = snap(7, 3);
        let mut b = a.clone();
        b.validators.reverse();
        assert_eq!(epoch_leader_seed(&a), epoch_leader_seed(&b));
        // sanity: cloning without reorder keeps it stable
        a.block_number += 1; // non-committee field must not affect the seed
        assert_eq!(epoch_leader_seed(&a), epoch_leader_seed(&snap(7, 3)));
    }
}
