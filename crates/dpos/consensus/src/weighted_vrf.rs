//! Stake-weighted VRF leader elector.
//!
//! One selection path: `leader = weighted_cdf(stake, randomness(round, cert))`.
//! The only variable is the 32-byte randomness — the prior view's threshold seed
//! σ (`CombinedCertificate::seed()`, k-lagged ⇒ unbiasable) when present, else a
//! deterministic per-epoch fallback (view-1-of-epoch / nullify-justified views,
//! where the cert carries no seed). Block share ∝ on-chain stake in expectation
//! (D1); weights are the epoch's FROZEN snapshot stake (D3), never live balance —
//! frozen ON-CHAIN since 2026-07-31 (`leaderStakes[epoch]`, stamped at
//! `commitEpochCommittee` from the selection epoch), so the vector no longer
//! depends on the height each node reads at.
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
///
/// The leader lottery is stake-only ON PURPOSE: letting the on-chain production
/// verdict feed back into the weights would make the schedule self-referential —
/// computable only by replaying every epoch since genesis, which a state-synced node
/// cannot do, and disagreement here is a leader-election split. `ProductionLiveness`
/// therefore never touches the weights: it spends its verdict on selection visibility
/// (agreed contract state), which the NEXT epoch's committee is drawn from.
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

/// The 32-byte leader randomness: the prior view's threshold seed σ when present, else a
/// deterministic per-epoch fallback bound to `(committee, view)`; domain-separated from
/// `prev_randao` (D6). A free fn so every caller shares the EXACT bytes with the live
/// elector — a divergent copy would split leader election.
pub(crate) fn randomness_bytes(
    round: Round,
    seed: Option<BlsSignature>,
    fallback_seed: &[u8; 32],
) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(LEADER_DOMAIN);
    match seed {
        Some(sigma) => {
            h.update(sigma.encode().as_ref());
        }
        None => {
            h.update(fallback_seed);
            h.update(&round.view().get().to_be_bytes());
        }
    }
    <[u8; 32]>::try_from(h.finalize().as_ref()).expect("sha256 is 32 bytes")
}

/// Leader-schedule inputs (inclusive prefix sums, total, fallback seed) — they travel as
/// one unit so no caller can pair a stale total with fresh prefix sums.
pub(crate) struct Schedule<'a> {
    pub cum: &'a [u128],
    pub total: u128,
    pub fallback_seed: &'a [u8; 32],
}

/// Which participant leads `round` under `sched` and the `seed`/fallback randomness. The
/// single election core behind [`WeightedVrfElector::elect`].
pub(crate) fn elect_index(
    sched: &Schedule,
    round: Round,
    seed: Option<BlsSignature>,
) -> Participant {
    let rand = randomness_bytes(round, seed, sched.fallback_seed);
    let target = (U256::from_be_bytes(rand) % U256::from(sched.total)).to::<u128>();
    Participant::from_usize(sched.cum.partition_point(|&c| c <= target))
}

#[cfg(test)]
impl WeightedVrfElector {
    fn randomness(&self, round: Round, seed: Option<BlsSignature>) -> [u8; 32] {
        randomness_bytes(round, seed, &self.fallback_seed)
    }

    fn pick(&self, rand: [u8; 32]) -> Participant {
        let target = (U256::from_be_bytes(rand) % U256::from(self.total)).to::<u128>();
        Participant::from_usize(self.cum.partition_point(|&c| c <= target))
    }
}

impl Elector<BlsScheme> for WeightedVrfElector {
    fn elect(&self, round: Round, certificate: Option<&CombinedCertificate>) -> Participant {
        // Every certificate of a beacon-active epoch carries σ — nullifications
        // included — so both branches of "did view v produce a block" hand this
        // the SAME seed, and the leader of v+1 does not depend on that bit. The
        // fallback arm survives for the pre-beacon epochs (0-1) and for view 1 of
        // each epoch, where simplex passes `None` because no view v-1 exists in
        // this epoch's view space. Keep it: commonware's own `Random` elector
        // panics on `assert!(seed.is_some() || view == 1)` instead.
        elect_index(&self.schedule(), round, certificate.and_then(|c| c.seed()))
    }
}

impl WeightedVrfElector {
    /// Borrow this elector's inputs as a [`Schedule`] for offline reconstruction reuse.
    pub(crate) fn schedule(&self) -> Schedule<'_> {
        Schedule {
            cum: &self.cum,
            total: self.total,
            fallback_seed: &self.fallback_seed,
        }
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

#[cfg(test)]
mod xlang_conformance {
    //! Cross-language conformance vector for the FALLBACK arm.
    //!
    //! `devnet/local-dpos-smoke/dpos_harness/cases/seed_continuity.py` reimplements this
    //! arm in Python to predict, offline, the leader a PRE-change binary would have
    //! elected after a nullified view — the prediction the live case asserts against.
    //! A silently divergent reimplementation makes that case mismatch everywhere and
    //! read as a vacuous pass, so the two must be pinned to the same vector.
    //!
    //! The magnitude of the weights is load-bearing, not just their ratio: the
    //! election is `rand % total`, so uniform-1 and uniform-5e9 committees elect
    //! DIFFERENT leaders. A first live run of the case was wrong for exactly that
    //! reason; keep the fixture's stake at the compacted devnet value.
    use super::*;
    use alloy_primitives::{Address, B256};
    use commonware_codec::DecodeExt as _;
    use commonware_consensus::types::{Epoch, View};
    use commonware_utils::TryFromIterator as _;
    use fluentbase_bls::{keys::ValidatorBlsKeypair, BlsPubkey, PeerPubkey};
    use fluentbase_staking_reader::reader::{ConsensusKeys, ValidatorWithKeys};
    use rand_core::SeedableRng as _;

    /// The devnet's deterministic 7-peer committee (`genesis-bootstrap consensus-keys`).
    const PKS: [&str; 7] = [
        "2e71978f382869ff2f2ac15424a86125610cccafb8629ca9f72c5aa5e5a9fefe",
        "0bd49e62f8033187d06ef14ef76ac78c26d3dc640613bf86cf7267a949cd9c50",
        "a6b3db1592dfaed7e0aebe01f2f1df8f71c06abd558df30f6a55b0159afee225",
        "2a23be9412ba671627da659cfee2bb01db7b81dbed2d595407e8102c62940b75",
        "0f89339953580de411151a06a1d8bbc8030b77e467e3d3670f7c6bfdf2be63e3",
        "fac42278ce587337d76a08cdcb21fed8e27dfba5d55bca0bbf6d1842fba7c999",
        "596918e015ca3b4b2bc2482dc36a58398423cc6f0c89b9d018ae8c928e73977c",
    ];

    /// 50e18 wei self-delegation compacted by `BALANCE_COMPACT_PRECISION` (1e10).
    const COMPACTED_STAKE: u128 = 5_000_000_000;

    /// `(epoch, view, participant_index)` — the Python mirror asserts the same list.
    const VECTOR: [(u64, u64, usize); 16] = [
        (2, 1, 5),
        (2, 2, 2),
        (2, 3, 5),
        (2, 4, 1),
        (2, 5, 0),
        (2, 6, 4),
        (2, 7, 5),
        (2, 8, 1),
        (5, 1, 2),
        (5, 2, 5),
        (5, 3, 6),
        (5, 4, 1),
        (5, 5, 6),
        (5, 6, 3),
        (5, 7, 0),
        (5, 8, 3),
    ];

    fn elector_for(epoch: u64) -> WeightedVrfElector {
        let validators = PKS
            .iter()
            .enumerate()
            .map(|(i, hex)| {
                let raw: Vec<u8> = (0..32)
                    .map(|j| u8::from_str_radix(&hex[2 * j..2 * j + 2], 16).unwrap())
                    .collect();
                ValidatorWithKeys {
                    address: Address::repeat_byte(i as u8),
                    keys: ConsensusKeys {
                        bls_pubkey: BlsPubkey::decode(
                            ValidatorBlsKeypair::generate(
                                &mut rand_08::rngs::StdRng::seed_from_u64(i as u64),
                            )
                            .public_bytes()
                            .as_slice(),
                        )
                        .unwrap(),
                        peer_pubkey: PeerPubkey::decode(raw.as_slice()).unwrap(),
                        activation_epoch: 1,
                    },
                    stake: COMPACTED_STAKE,
                }
            })
            .collect();
        let snap = ValidatorSetSnapshot {
            block_hash: B256::repeat_byte(0xAB),
            block_number: 1,
            epoch,
            validators,
        };
        let parts = commonware_utils::ordered::Set::try_from_iter(
            snap.validators.iter().map(|v| v.keys.peer_pubkey.clone()),
        )
        .unwrap();
        WeightedVrf::from_snapshot(&snap).build(&parts)
    }

    #[test]
    fn fallback_vector_is_stable() {
        for (epoch, view, want) in VECTOR {
            let got: usize = elector_for(epoch)
                .elect(Round::new(Epoch::new(epoch), View::new(view)), None)
                .into();
            assert_eq!(
                got, want,
                "fallback leader changed at epoch {epoch} view {view}"
            );
        }
    }

    #[test]
    fn weight_magnitude_changes_the_winner_not_just_the_distribution() {
        // The defect the live case hit: `rand % total` is magnitude-sensitive, so a
        // mirror that normalises uniform weights to 1 silently elects someone else.
        let mut snap_validators = Vec::new();
        for (i, hex) in PKS.iter().enumerate() {
            let raw: Vec<u8> = (0..32)
                .map(|j| u8::from_str_radix(&hex[2 * j..2 * j + 2], 16).unwrap())
                .collect();
            snap_validators.push((i, raw));
        }
        let build = |stake: u128| {
            let validators = snap_validators
                .iter()
                .map(|(i, raw)| ValidatorWithKeys {
                    address: Address::repeat_byte(*i as u8),
                    keys: ConsensusKeys {
                        bls_pubkey: BlsPubkey::decode(
                            ValidatorBlsKeypair::generate(
                                &mut rand_08::rngs::StdRng::seed_from_u64(*i as u64),
                            )
                            .public_bytes()
                            .as_slice(),
                        )
                        .unwrap(),
                        peer_pubkey: PeerPubkey::decode(raw.as_slice()).unwrap(),
                        activation_epoch: 1,
                    },
                    stake,
                })
                .collect();
            let snap = ValidatorSetSnapshot {
                block_hash: B256::repeat_byte(0xAB),
                block_number: 1,
                epoch: 2,
                validators,
            };
            let parts = commonware_utils::ordered::Set::try_from_iter(
                snap.validators.iter().map(|v| v.keys.peer_pubkey.clone()),
            )
            .unwrap();
            let e = WeightedVrf::from_snapshot(&snap).build(&parts);
            let idx: usize = e
                .elect(Round::new(Epoch::new(2), View::new(1)), None)
                .into();
            idx
        };
        assert_ne!(build(1), build(COMPACTED_STAKE));
    }
}
