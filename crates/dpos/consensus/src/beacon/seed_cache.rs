//! Cache of recovered per-height beacon seeds, bridging the recover→derive gap.
//!
//! `prev_randao` for the EVM block at ordering-height h is `H(seed(h))`, but
//! `seed(h)` is signed AFTER h finalizes — so it is NOT on the OrderBlock being
//! derived. In the current (C4) wiring this cache is fed by the live beacon seed
//! actor ([`crate::beacon::seed_actor::SeedSigner::commit`]) as seeds recover
//! over `BEACON_CHANNEL`, and read by the deriver to look up `seed(h)`. It is
//! pruned to a bounded window so it cannot grow without bound.
//!
//! NOTE: this live feed is NOT consensus-agreed (the present/absent decision is
//! per-node, wall-clock-gated in the deriver), which is the determinism gap the
//! agreed embed-in-`OrderBlock.beacon_seed` rework (#4) addresses; the `Seed`
//! wire field + this cache are the building blocks for that.

use crate::beacon::types::Seed;
use std::collections::BTreeMap;

/// How many heights below the highest-seen seed to retain. Must comfortably
/// exceed `K` (`order_block::K`): a seed for height h is consumed when EVM
/// block h is derived at ordering-tip ≥ h+K, so it has to survive from
/// insertion (around h..h+K) until then. A few multiples of K give margin for
/// nullified-view height/view drift without unbounded growth.
pub const DEFAULT_SEED_RETAIN: u64 = 64;

/// Maps `target_height` → recovered [`Seed`], pruned to a trailing window.
#[derive(Debug)]
pub struct SeedCache {
    seeds: BTreeMap<u64, Seed>,
    retain: u64,
    highest: u64,
}

impl SeedCache {
    pub fn new(retain: u64) -> Self {
        Self {
            seeds: BTreeMap::new(),
            retain,
            highest: 0,
        }
    }

    /// Record a seed observed in a block's `beacon_seed`, then prune everything
    /// more than `retain` heights below the highest seed seen so far.
    pub fn insert(&mut self, seed: Seed) {
        self.highest = self.highest.max(seed.target_height);
        self.seeds.insert(seed.target_height, seed);
        let floor = self.highest.saturating_sub(self.retain);
        // Drop the stale prefix (heights strictly below the retention floor).
        self.seeds = self.seeds.split_off(&floor);
    }

    /// The seed for `height`, if still in the window.
    pub fn get(&self, height: u64) -> Option<&Seed> {
        self.seeds.get(&height)
    }

    pub fn len(&self) -> usize {
        self.seeds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seeds.is_empty()
    }
}

impl Default for SeedCache {
    fn default() -> Self {
        Self::new(DEFAULT_SEED_RETAIN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beacon::seed::{recover_seed, seed_namespace, sign_seed_partial};
    use commonware_cryptography::bls12381::{dkg::deal_anonymous, primitives::variant::MinSig};
    use commonware_utils::{test_rng, N3f1, NZU32};

    fn seed_at(height: u64) -> Seed {
        // A real recovered threshold seed (deterministic per height under the
        // fixed test rng) — the cache stores whatever the seed sub-protocol
        // produced, so use the genuine type rather than a hand-built signature.
        let mut rng = test_rng();
        let (sharing, shares) =
            deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(4));
        let ns = seed_namespace(b"fluent-devnet");
        let partials: Vec<_> = shares
            .iter()
            .map(|s| sign_seed_partial(s, &ns, height))
            .collect();
        recover_seed(&sharing, &partials, height).expect("recover")
    }

    #[test]
    fn get_returns_seed_within_window() {
        let mut cache = SeedCache::new(8);
        cache.insert(seed_at(10));
        cache.insert(seed_at(11));
        assert_eq!(cache.get(10).unwrap().target_height, 10);
        assert_eq!(cache.get(11).unwrap().target_height, 11);
        assert!(cache.get(12).is_none());
    }

    #[test]
    fn prunes_below_retention_floor() {
        let mut cache = SeedCache::new(3);
        for h in 10..=20 {
            cache.insert(seed_at(h));
        }
        // highest=20, retain=3 → floor=17; heights <17 are dropped.
        assert!(cache.get(16).is_none(), "stale height pruned");
        assert!(cache.get(17).is_some(), "floor retained");
        assert!(cache.get(20).is_some(), "newest retained");
        assert_eq!(cache.len(), 4); // 17,18,19,20
    }
}
