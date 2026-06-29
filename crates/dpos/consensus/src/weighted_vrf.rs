//! Stake-weighted VRF leader elector.
//!
//! One selection path: `leader = weighted_cdf(stake, randomness(round, cert))`.
//! The only variable is the 32-byte randomness — the prior view's threshold seed
//! σ (`CombinedCertificate::seed()`, k-lagged ⇒ unbiasable) when present, else a
//! deterministic per-epoch fallback (view-1-of-epoch / nullify-justified views,
//! where the cert carries no seed). Block share ∝ on-chain stake in expectation
//! (D1); weights are the epoch's FROZEN snapshot stake (D3), never live balance.
//! σ is domain-separated (`Sha256(LEADER_DOMAIN ‖ σ)`) from the EVM
//! `prev_randao = keccak256(σ)` (D6) so the two consumers share no bytes.
//!
//! This is a consensus-plane decision only: the STF / zk guest is NOT touched and
//! MUST NOT mirror it — its sole σ consumer is `prev_randao`.

use alloy_primitives::U256;
use commonware_codec::Encode as _;
use commonware_consensus::{
    simplex::elector::{Config, Elector},
    types::{Participant, Round},
};
use commonware_cryptography::{Hasher, Sha256};
use commonware_utils::ordered::Set;
use fluentbase_bls::{
    combined_scheme::CombinedCertificate, BlsSignature, PeerPubkey, Scheme as BlsScheme,
};
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use std::collections::BTreeMap;

/// Domain tag: `Sha256(LEADER_DOMAIN ‖ σ)` is disjoint from the EVM
/// `prev_randao = keccak256(σ)` (`beacon/seed.rs`). The exact bytes are
/// arbitrary; only the disjointness matters (D6).
const LEADER_DOMAIN: &[u8] = b"fluent/leader";

/// Elector config (built into [`WeightedVrfElector`] by simplex at
/// `voter/state.rs` from the commonware-sorted participant set). Carries the
/// per-validator frozen stake keyed by peer key — so `build` can align it to that
/// set — and the fallback seed. `Default` (empty) is required by the trait and
/// never used in production (degrades to uniform via the all-zero guard in `build`).
#[derive(Clone, Default)]
pub struct WeightedVrf {
    weights: BTreeMap<PeerPubkey, u128>,
    fallback_seed: [u8; 32],
}

impl WeightedVrf {
    /// Build from the epoch's frozen committee snapshot.
    pub fn from_snapshot(snap: &ValidatorSetSnapshot) -> Self {
        let weights = snap
            .validators
            .iter()
            .map(|v| (v.keys.peer_pubkey.clone(), v.stake))
            .collect();
        Self {
            weights,
            fallback_seed: fallback_seed(snap),
        }
    }
}

/// `sha256(epoch_be ‖ sorted peer pubkeys)` — deterministic, network-identical,
/// unpredictable until the committee is committed on-chain. (Folded in from the
/// deleted `elector_seed::epoch_leader_seed`; now the fallback entropy, no longer
/// a RoundRobin shuffle seed.) Sorting the peers makes the seed invariant under
/// any snapshot iteration order, so honest nodes that observe the epoch's keys in
/// any order derive the identical fallback.
fn fallback_seed(snap: &ValidatorSetSnapshot) -> [u8; 32] {
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
    <[u8; 32]>::try_from(h.finalize().as_ref()).expect("sha256 is 32 bytes")
}

impl Config<BlsScheme> for WeightedVrf {
    type Elector = WeightedVrfElector;

    fn build(self, participants: &Set<PeerPubkey>) -> WeightedVrfElector {
        assert!(!participants.is_empty(), "no participants");
        // Weight per participant index (set order == Participant index). Missing /
        // all-zero ⇒ uniform: the single clean guard that keeps `total > 0` (no
        // modulo-0) and is also where a future per-validator saturation cap would
        // clamp (D2 — not built; no cap field/metric now).
        let mut w: Vec<u128> = participants
            .iter()
            .map(|p| self.weights.get(p).copied().unwrap_or(0))
            .collect();
        if w.iter().sum::<u128>() == 0 {
            w.iter_mut().for_each(|x| *x = 1);
        }
        let mut cum = Vec::with_capacity(w.len());
        let mut acc = 0u128;
        // Overflow-safe: committee ≤ MAX_PEER_SET_SIZE (51) × compacted uint112
        // (< 2^112) ≈ 2^119 ≪ u128::MAX.
        for x in w {
            acc += x;
            cum.push(acc);
        }
        WeightedVrfElector {
            cum,
            total: acc,
            fallback_seed: self.fallback_seed,
        }
    }
}

/// Built elector. `cum` = inclusive prefix sums of per-participant weight;
/// `total == cum.last() > 0` by construction (the all-zero guard in [`build`]).
///
/// [`build`]: WeightedVrf::build
#[derive(Clone)]
pub struct WeightedVrfElector {
    cum: Vec<u128>,
    total: u128,
    fallback_seed: [u8; 32],
}

impl WeightedVrfElector {
    /// The 32-byte leader randomness: the prior view's threshold seed σ when the
    /// justifying cert carries one, else a deterministic per-epoch fallback bound
    /// to `(committee, view)`. Domain-separated from `prev_randao` (D6).
    fn randomness(&self, round: Round, seed: Option<BlsSignature>) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(LEADER_DOMAIN);
        match seed {
            Some(sigma) => {
                h.update(sigma.encode().as_ref());
            }
            None => {
                h.update(&self.fallback_seed); // binds epoch + committee
                h.update(&round.view().get().to_be_bytes());
            }
        }
        <[u8; 32]>::try_from(h.finalize().as_ref()).expect("sha256 is 32 bytes")
    }

    /// CDF lookup: first index whose inclusive prefix sum exceeds `rand mod total`.
    /// Pure; the single weighted-selection step shared by both entropy sources.
    fn pick(&self, rand: [u8; 32]) -> Participant {
        let target = (U256::from_be_bytes(rand) % U256::from(self.total)).to::<u128>();
        Participant::from_usize(self.cum.partition_point(|&c| c <= target))
    }
}

impl Elector<BlsScheme> for WeightedVrfElector {
    fn elect(&self, round: Round, certificate: Option<&CombinedCertificate>) -> Participant {
        // `Some(cert{seed: None})` (a nullify cert at any view) takes the identical
        // `and_then → None` fallback branch — no `assert!(seed.is_some()||view==1)`
        // panic trap (the commonware `Random` elector's, which our nullify certs
        // would trip at view ≥ 2).
        self.pick(self.randomness(round, certificate.and_then(|c| c.seed())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, B256};
    use commonware_codec::DecodeExt as _;
    use commonware_consensus::types::{Epoch, View};
    use commonware_cryptography::{
        bls12381::primitives::{group::Private, ops, variant::MinSig},
        ed25519::PrivateKey as Ed25519PrivateKey,
        Signer,
    };
    use commonware_math::algebra::Random as _;
    use commonware_utils::TryFromIterator as _;
    use fluentbase_bls::{keys::ValidatorBlsKeypair, BlsPubkey};
    use fluentbase_staking_reader::reader::{ConsensusKeys, ValidatorWithKeys};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;

    fn snapshot(epoch: u64, stakes: &[u128]) -> ValidatorSetSnapshot {
        let validators = stakes
            .iter()
            .enumerate()
            .map(|(i, &stake)| {
                let mut rng = StdRng::seed_from_u64(epoch * 1000 + i as u64);
                let peer = Ed25519PrivateKey::random(&mut rng).public_key();
                let bls = BlsPubkey::decode(
                    ValidatorBlsKeypair::generate(&mut rng)
                        .public_bytes()
                        .as_slice(),
                )
                .unwrap();
                ValidatorWithKeys {
                    address: Address::repeat_byte(i as u8),
                    keys: ConsensusKeys {
                        bls_pubkey: bls,
                        peer_pubkey: peer,
                        activation_epoch: 1,
                    },
                    stake,
                }
            })
            .collect();
        ValidatorSetSnapshot {
            block_hash: B256::repeat_byte(0xAB),
            block_number: epoch * 100,
            epoch,
            validators,
        }
    }

    fn participants(snap: &ValidatorSetSnapshot) -> Set<PeerPubkey> {
        Set::try_from_iter(snap.validators.iter().map(|v| v.keys.peer_pubkey.clone())).unwrap()
    }

    /// Per-index weight recovered from the inclusive prefix sums.
    fn per_index_weights(e: &WeightedVrfElector) -> Vec<u128> {
        e.cum
            .iter()
            .scan(0u128, |prev, &c| {
                let w = c - *prev;
                *prev = c;
                Some(w)
            })
            .collect()
    }

    #[test]
    fn build_is_deterministic_and_order_invariant() {
        // Cross-node agreement: nodes observing the epoch's keys in any order build
        // the byte-identical elector.
        let s = snapshot(7, &[3, 5, 2]);
        let p = participants(&s);
        let e1 = WeightedVrf::from_snapshot(&s).build(&p);
        let e2 = WeightedVrf::from_snapshot(&s).build(&p);
        let mut s_rev = s.clone();
        s_rev.validators.reverse();
        let e3 = WeightedVrf::from_snapshot(&s_rev).build(&participants(&s_rev));

        assert_eq!(e1.cum, e2.cum);
        assert_eq!(e1.total, e2.total);
        assert_eq!(e1.fallback_seed, e2.fallback_seed);
        assert_eq!(e1.cum, e3.cum);
        assert_eq!(e1.fallback_seed, e3.fallback_seed);
    }

    #[test]
    fn pick_follows_weighted_cdf() {
        // weights [1, 3] ⇒ cum [1, 4], total 4.
        let e = WeightedVrfElector {
            cum: vec![1, 4],
            total: 4,
            fallback_seed: [0u8; 32],
        };
        let leader = |t: u128| e.pick(U256::from(t).to_be_bytes::<32>()).get();
        assert_eq!(leader(0), 0);
        assert_eq!(leader(1), 1);
        assert_eq!(leader(2), 1);
        assert_eq!(leader(3), 1);
        assert_eq!(leader(4), 0, "target wraps mod total");
    }

    #[test]
    fn from_snapshot_unequal_stake_is_proportional() {
        // seam-2: the full snapshot → weights → pick path under skew. Driving
        // `elect` over many views (the fallback randomness, uniform per view) Monte-
        // Carlo-samples the weighted CDF — distributionally identical to the σ path.
        let s = snapshot(1, &[1, 2, 7]);
        let e = WeightedVrf::from_snapshot(&s).build(&participants(&s));
        let weights = per_index_weights(&e);
        let n = e.cum.len();
        let samples = 30_000u64;
        let mut tally = vec![0u64; n];
        for view in 1..=samples {
            let idx: usize = e
                .elect(Round::new(Epoch::new(1), View::new(view)), None)
                .into();
            tally[idx] += 1;
        }
        for i in 0..n {
            let expected = weights[i] as f64 / e.total as f64;
            let got = tally[i] as f64 / samples as f64;
            assert!(
                (got - expected).abs() < 0.03,
                "index {i}: expected ~{expected:.3}, got {got:.3}"
            );
        }
        let (heavy, _) = weights.iter().enumerate().max_by_key(|(_, &w)| w).unwrap();
        let (light, _) = weights.iter().enumerate().min_by_key(|(_, &w)| w).unwrap();
        assert!(
            tally[heavy] > tally[light],
            "heaviest validator must lead strictly more than the lightest"
        );
    }

    #[test]
    fn zero_total_weight_is_uniform() {
        let s = snapshot(1, &[0, 0, 0]);
        let e = WeightedVrf::from_snapshot(&s).build(&participants(&s));
        assert_eq!(e.total, 3, "all-zero guard sets each weight to 1");
        for view in 1..=50 {
            let idx: usize = e
                .elect(Round::new(Epoch::new(1), View::new(view)), None)
                .into();
            assert!(idx < 3);
        }
    }

    #[test]
    fn fallback_elects_without_panic_at_view_two() {
        // Regression vs commonware `Random`'s `assert!(seed.is_some()||view==1)`:
        // a seedless view ≥ 2 (here `None`; `Some(cert{seed:None})` is equivalent)
        // must elect, not panic.
        let s = snapshot(1, &[1, 1, 1]);
        let e = WeightedVrf::from_snapshot(&s).build(&participants(&s));
        let idx: usize = e
            .elect(Round::new(Epoch::new(1), View::new(2)), None)
            .into();
        assert!(idx < 3);
    }

    #[test]
    fn sigma_path_deterministic_and_differs_from_fallback() {
        let s = snapshot(1, &[1, 1, 1]);
        let e = WeightedVrf::from_snapshot(&s).build(&participants(&s));
        let mut rng = StdRng::seed_from_u64(42);
        let sk = Private::random(&mut rng);
        let sigma: BlsSignature = ops::sign_message::<MinSig>(&sk, b"ns", b"leader-test");
        let r = Round::new(Epoch::new(1), View::new(5));

        assert_eq!(
            e.randomness(r, Some(sigma)),
            e.randomness(r, Some(sigma)),
            "σ-path is deterministic"
        );
        assert_ne!(
            e.randomness(r, Some(sigma)),
            e.randomness(r, None),
            "σ-path differs from fallback (domain separation)"
        );
    }
}
