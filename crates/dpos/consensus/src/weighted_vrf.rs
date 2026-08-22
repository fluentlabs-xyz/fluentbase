//! Stake-weighted VRF leader elector.
//!
//! One selection path: `leader = weighted_cdf(stake, randomness(round, cert))`.
//! The only variable is the 32-byte randomness — the prior view's threshold seed
//! σ (`CombinedCertificate::seed()`, k-lagged ⇒ unbiasable) when present, else a
//! deterministic per-epoch fallback (view-1-of-epoch / nullify-justified views,
//! where the cert carries no seed). The fallback's base is not derivable from
//! constants: it is [`crate::beacon::witness_fallback_seed`] of the previous epoch's terminal
//! block's `parent_seed`, supplied by the epoch manager that reads that block;
//! [`crate::beacon::constant_fallback_seed`] is the last resort where no witness can exist.
//! Block share ∝ on-chain stake in expectation
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

/// Separate tag for the seedless arm. The base fed to that arm is a σ that
/// already drove a [`LEADER_DOMAIN`] draw inside the previous epoch, so the two
/// arms must not be able to hash the same preimage — otherwise the first block
/// of E+1 could be led by whoever led the last block of E.
///
/// The two tags must stay PREFIX-FREE (neither a prefix of the other) for that
/// to hold, because the tag is the only self-delimiting part of the preimage:
/// with a tag like `b"fluent/leader/fallback"` the arms are separated only by
/// `σ.encode()` being 48 bytes against this arm's 32-byte base plus 8-byte view,
/// i.e. by an encoding length rather than by the domains. Any replacement must
/// keep the prefix-free property; the exact bytes are otherwise arbitrary and
/// are pinned by `the_seedless_arm_is_pinned_to_its_own_domain_tag`.
const LEADER_FALLBACK_DOMAIN: &[u8] = b"fluent/seedless-leader";

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
    /// Build from the epoch's frozen committee snapshot and the epoch's
    /// seedless-arm base. The base is the previous epoch's terminal-block
    /// witness seed when one exists, else [`crate::beacon::constant_fallback_seed`].
    ///
    /// **Fails rather than degrading when the weights are absent.** The contract
    /// keeps membership forever but only the last N epochs of weights, so a
    /// snapshot can legitimately arrive with `weights: None`. Falling back to a
    /// uniform lottery there would be the worst available answer: every node
    /// that still had the weights would elect a different leader, and the split
    /// would be silent. An error is recoverable — the caller skips the epoch and
    /// retries — where a wrong leader is not.
    pub fn try_new(
        snap: &ValidatorSetSnapshot,
        fallback_seed: [u8; 32],
    ) -> Result<Self, WeightsUnavailable> {
        let Some(frozen) = snap.weights.as_ref() else {
            return Err(WeightsUnavailable {
                epoch: snap.epoch,
                members: snap.validators.len(),
                weights: None,
            });
        };
        if frozen.len() != snap.validators.len() {
            return Err(WeightsUnavailable {
                epoch: snap.epoch,
                members: snap.validators.len(),
                weights: Some(frozen.len()),
            });
        }
        Ok(Self {
            weights: snap
                .validators
                .iter()
                .zip(frozen)
                .map(|(v, weight)| (v.keys.peer_pubkey.clone(), *weight))
                .collect(),
            fallback_seed,
        })
    }
}

/// The epoch's frozen leader weights are gone or do not line up with its
/// membership, so no leader schedule can be derived for it.
#[derive(Debug, Clone, Copy)]
pub struct WeightsUnavailable {
    pub epoch: u64,
    pub members: usize,
    /// `None` when the contract reported the weight ring had wrapped; `Some(n)`
    /// when it returned `n` weights for a committee of a different size, which
    /// is corruption rather than absence.
    pub weights: Option<usize>,
}

impl core::fmt::Display for WeightsUnavailable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.weights {
            None => write!(
                f,
                "epoch {}: frozen leader weights are no longer retained on chain \
                 (committee of {} members); the weight ring has wrapped past it",
                self.epoch, self.members
            ),
            Some(n) => write!(
                f,
                "epoch {}: {} frozen leader weights for a committee of {} members",
                self.epoch, n, self.members
            ),
        }
    }
}

impl core::error::Error for WeightsUnavailable {}

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
        // (< 2^112) ≈ 2^119 ≪ u128::MAX. That bound is now enforced where the
        // weights enter, by `staking-reader`'s MAX_COMPACT_STAKE, not just
        // asserted by the contract.
        //
        // `saturating_add` is belt to that brace, and the belt is what makes
        // `elect_index` sound rather than merely unlikely to be unsound: release
        // builds run with overflow-checks off, so a plain `+=` would WRAP rather
        // than panic, and a wrapped `acc` yields a NON-MONOTONIC `cum`.
        // `cum.partition_point` only guarantees "result < len" for sorted input,
        // so a wrapped prefix sum can hand back an out-of-range Participant and
        // corrupt leader election. Saturating keeps `cum` non-decreasing for any
        // input at all; the worst case is a skewed draw, never a bad index.
        for x in w {
            acc = acc.saturating_add(x);
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
/// deterministic per-epoch fallback bound to `(fallback_seed, view)`; the two arms carry
/// prefix-free domain tags, so they cannot share a preimage whatever they are fed, and both
/// are disjoint from `prev_randao` (D6). A free fn so every caller shares the EXACT bytes
/// with the live elector — a divergent copy would split leader election.
pub(crate) fn randomness_bytes(
    round: Round,
    seed: Option<BlsSignature>,
    fallback_seed: &[u8; 32],
) -> [u8; 32] {
    let mut h = Sha256::new();
    match seed {
        Some(sigma) => {
            h.update(LEADER_DOMAIN);
            h.update(sigma.encode().as_ref());
        }
        None => {
            h.update(LEADER_FALLBACK_DOMAIN);
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
    use crate::beacon::{constant_fallback_seed, witness_fallback_seed, Seed};
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
            .map(|(i, _)| {
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
                    tombstoned: false,
                }
            })
            .collect();
        ValidatorSetSnapshot {
            block_hash: B256::repeat_byte(0xAB),
            block_number: epoch * 100,
            epoch,
            validators,
            weights: Some(stakes.to_vec()),
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

    /// Absent weights must FAIL the elector, never degrade it to uniform.
    ///
    /// The degraded answer is the dangerous one: a node that still holds the
    /// weights and a node that does not would elect different leaders for the
    /// same round, and neither would log anything. The error is recoverable —
    /// `EpochEngine::new` propagates it and `epoch_manager` skips and retries the
    /// epoch — where a leader split is not.
    #[test]
    fn absent_or_mismatched_weights_fail_the_elector_instead_of_going_uniform() {
        let mut s = snapshot(7, &[3, 5, 2]);

        s.weights = None;
        let Err(err) = WeightedVrf::try_new(&s, constant_fallback_seed(&s)) else {
            panic!("a wrapped ring must not build an elector");
        };
        assert_eq!(err.epoch, 7);
        assert_eq!(err.members, 3);
        assert!(err.weights.is_none());

        s.weights = Some(vec![3, 5]);
        let Err(err) = WeightedVrf::try_new(&s, constant_fallback_seed(&s)) else {
            panic!("a short weight vector is corruption, not absence");
        };
        assert_eq!(err.weights, Some(2));

        s.weights = Some(vec![3, 5, 2]);
        assert!(
            WeightedVrf::try_new(&s, constant_fallback_seed(&s)).is_ok(),
            "and the matching case still builds"
        );
    }

    #[test]
    fn build_is_deterministic_and_order_invariant() {
        // Cross-node agreement: nodes observing the epoch's keys in any order build
        // the byte-identical elector.
        let s = snapshot(7, &[3, 5, 2]);
        let p = participants(&s);
        let e1 = WeightedVrf::try_new(&s, constant_fallback_seed(&s))
            .unwrap()
            .build(&p);
        let e2 = WeightedVrf::try_new(&s, constant_fallback_seed(&s))
            .unwrap()
            .build(&p);
        let mut s_rev = s.clone();
        s_rev.validators.reverse();
        // Weights are paired with members by position, so a reversal has to
        // carry them along — reversing one leg alone would reassign the weights,
        // which is a different test.
        if let Some(w) = s_rev.weights.as_mut() {
            w.reverse();
        }
        let e3 = WeightedVrf::try_new(&s_rev, constant_fallback_seed(&s_rev))
            .unwrap()
            .build(&participants(&s_rev));

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
    fn unequal_stake_is_proportional() {
        // seam-2: the full snapshot → weights → pick path under skew. Driving
        // `elect` over many views (the fallback randomness, uniform per view) Monte-
        // Carlo-samples the weighted CDF — distributionally identical to the σ path.
        let s = snapshot(1, &[1, 2, 7]);
        let e = WeightedVrf::try_new(&s, constant_fallback_seed(&s))
            .unwrap()
            .build(&participants(&s));
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
        let e = WeightedVrf::try_new(&s, constant_fallback_seed(&s))
            .unwrap()
            .build(&participants(&s));
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
        let e = WeightedVrf::try_new(&s, constant_fallback_seed(&s))
            .unwrap()
            .build(&participants(&s));
        let idx: usize = e
            .elect(Round::new(Epoch::new(1), View::new(2)), None)
            .into();
        assert!(idx < 3);
    }

    #[test]
    fn sigma_path_deterministic_and_differs_from_fallback() {
        let s = snapshot(1, &[1, 1, 1]);
        let e = WeightedVrf::try_new(&s, constant_fallback_seed(&s))
            .unwrap()
            .build(&participants(&s));
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

    fn sigma(seed: u64) -> BlsSignature {
        let mut rng = StdRng::seed_from_u64(seed);
        ops::sign_message::<MinSig>(&Private::random(&mut rng), b"ns", b"leader-test")
    }

    /// The seedless base for epoch E+1 is a σ that already drove a `Some(σ)` draw
    /// inside epoch E, so the arms must stay disjoint even when fed that same σ —
    /// and the prefix-free tag is the only thing that makes them so. The expected
    /// digest is therefore recomputed here from the LITERAL tag bytes, not from
    /// [`LEADER_FALLBACK_DOMAIN`]: this fails the moment the seedless arm is
    /// pointed at another domain. Comparing the two arms' elected indices instead
    /// proves nothing — those differ with or without a tag, because `σ.encode()`
    /// and `base ‖ view_be` are different lengths.
    #[test]
    fn the_seedless_arm_is_pinned_to_its_own_domain_tag() {
        let round = Round::new(Epoch::new(2), View::new(1));
        let base = witness_fallback_seed(&Seed {
            target_round: round,
            signature: sigma(7),
        });
        let mut h = Sha256::new();
        h.update(b"fluent/seedless-leader");
        h.update(&base);
        h.update(&round.view().get().to_be_bytes());
        let want = <[u8; 32]>::try_from(h.finalize().as_ref()).unwrap();

        assert_eq!(randomness_bytes(round, None, &base), want);
    }

    /// The point of the change: with the epoch and the committee both fixed, the
    /// epoch's leader sequence must move when the inherited terminal-block witness
    /// moves. Under the old constant-only base it could not — the sequence was a
    /// function of `(epoch, peers)` and therefore computable an epoch ahead.
    #[test]
    fn the_leader_sequence_moves_with_the_inherited_witness() {
        let s = snapshot(2, &[1; 7]);
        let round = |view| Round::new(Epoch::new(2), View::new(view));
        let sequence = |base: [u8; 32]| -> Vec<usize> {
            let e = WeightedVrf::try_new(&s, base)
                .unwrap()
                .build(&participants(&s));
            (1..=8).map(|v| e.elect(round(v), None).into()).collect()
        };
        let witness = |seed| {
            witness_fallback_seed(&Seed {
                target_round: round(1),
                signature: sigma(seed),
            })
        };
        assert_ne!(sequence(witness(1)), sequence(witness(2)));
        assert_ne!(sequence(witness(1)), sequence(constant_fallback_seed(&s)));
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
    use crate::beacon::constant_fallback_seed;
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
        (2, 1, 1),
        (2, 2, 4),
        (2, 3, 4),
        (2, 4, 3),
        (2, 5, 3),
        (2, 6, 2),
        (2, 7, 4),
        (2, 8, 4),
        (5, 1, 5),
        (5, 2, 2),
        (5, 3, 6),
        (5, 4, 6),
        (5, 5, 1),
        (5, 6, 5),
        (5, 7, 6),
        (5, 8, 5),
    ];

    fn elector_for(epoch: u64) -> WeightedVrfElector {
        let validators: Vec<ValidatorWithKeys> = PKS
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
                    tombstoned: false,
                }
            })
            .collect();
        let weights = vec![COMPACTED_STAKE; validators.len()];
        let snap = ValidatorSetSnapshot {
            block_hash: B256::repeat_byte(0xAB),
            block_number: 1,
            epoch,
            validators,
            weights: Some(weights),
        };
        let parts = commonware_utils::ordered::Set::try_from_iter(
            snap.validators.iter().map(|v| v.keys.peer_pubkey.clone()),
        )
        .unwrap();
        WeightedVrf::try_new(&snap, constant_fallback_seed(&snap))
            .unwrap()
            .build(&parts)
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
                    tombstoned: false,
                })
                .collect::<Vec<_>>();
            let weights = vec![stake; validators.len()];
            let snap = ValidatorSetSnapshot {
                block_hash: B256::repeat_byte(0xAB),
                block_number: 1,
                epoch: 2,
                validators,
                weights: Some(weights),
            };
            let parts = commonware_utils::ordered::Set::try_from_iter(
                snap.validators.iter().map(|v| v.keys.peer_pubkey.clone()),
            )
            .unwrap();
            let e = WeightedVrf::try_new(&snap, constant_fallback_seed(&snap))
                .unwrap()
                .build(&parts);
            let idx: usize = e
                .elect(Round::new(Epoch::new(2), View::new(1)), None)
                .into();
            idx
        };
        assert_ne!(build(1), build(COMPACTED_STAKE));
    }
}
