//! The per-height seed-collection round — the live-actor core that turns ≥t
//! partial signatures over an ordering height into the recovered threshold
//! [`Seed`] (`prev_randao(h) = H(seed(h))`).
//!
//! The networked beacon seed actor drives one `SeedRound` per ordering height
//! AFTER that height finalizes (sign-after-finalize, [`crate::beacon::seed`]):
//! it `start`s the round with this node's DKG share (broadcasting its own
//! partial), feeds peers' partials in as they arrive over `BEACON_CHANNEL`,
//! and on recovery pushes the seed into the deriver's seed cache (the live
//! beacon→executor channel, Decision C4). The round itself is pure state with
//! NO I/O, so the quorum-collection + recovery logic is exhaustively
//! unit-testable without the network or a real DKG.

use crate::beacon::seed::{recover_seed, sign_seed_partial, verify_seed, verify_seed_partial};
use crate::beacon::types::Seed;
use commonware_cryptography::bls12381::primitives::{
    group::Share,
    sharing::Sharing,
    variant::{MinSig, PartialSignature},
};
use commonware_utils::{N3f1, Participant};
use std::collections::BTreeMap;

/// Collects partial seed-signatures for one ordering height until a quorum
/// recovers the unique threshold seed.
#[derive(Debug)]
pub struct SeedRound {
    height: u64,
    namespace: Vec<u8>,
    sharing: Sharing<MinSig>,
    /// Verified partials keyed by signer index — the key dedups honest
    /// retransmits and caps a Byzantine peer to one contribution.
    partials: BTreeMap<Participant, PartialSignature<MinSig>>,
    /// `N3f1` quorum for this committee — the minimum distinct partials to
    /// recover (`Sharing::required`).
    quorum: usize,
    /// Latched once a seed is recovered so `add_partial` reports it exactly
    /// once and later partials are ignored.
    done: bool,
}

impl SeedRound {
    /// Begin a round for `height`, contributing this node's own partial signed
    /// with its DKG `share`. The returned partial is what the actor broadcasts
    /// over `BEACON_CHANNEL`.
    pub fn start(
        height: u64,
        namespace: Vec<u8>,
        sharing: Sharing<MinSig>,
        share: &Share,
    ) -> (Self, PartialSignature<MinSig>) {
        let mut round = Self::observe(height, namespace, sharing);
        // A fresh round can never recover from its own single partial, so the
        // recovery result is necessarily `None` here.
        let (own, _) = round.contribute(share);
        (round, own)
    }

    /// Contribute THIS node's own partial to the round (signing with its DKG
    /// `share`), e.g. when it observes the height finalize after having started
    /// the round as a verifier. Returns the partial to broadcast plus any seed
    /// that this contribution completes (a peer-majority may already be held).
    /// Own partials are trusted (valid by construction) so they skip the
    /// public-polynomial check.
    pub fn contribute(&mut self, share: &Share) -> (PartialSignature<MinSig>, Option<Seed>) {
        let own = sign_seed_partial(share, &self.namespace, self.height);
        let recovered = self.insert(own.clone());
        (own, recovered)
    }

    /// Begin a verifier-only round (no share): collect peers' partials toward
    /// recovery without contributing one. Used by a node that holds no share
    /// this epoch — the Q4 verifier-only / catch-up path.
    pub fn observe(height: u64, namespace: Vec<u8>, sharing: Sharing<MinSig>) -> Self {
        let quorum = sharing.required::<N3f1>() as usize;
        Self {
            height,
            namespace,
            sharing,
            partials: BTreeMap::new(),
            quorum,
            done: false,
        }
    }

    /// Feed a partial received from a peer. Partials that fail verification
    /// against the public polynomial, or repeat an already-seen signer index,
    /// are dropped. Returns `Some(seed)` exactly once — when ≥`quorum` distinct
    /// valid partials recover a seed that ALSO verifies against `PK_epoch`
    /// (defense in depth: a sub-quorum or mixed set can never be accepted).
    pub fn add_partial(&mut self, partial: PartialSignature<MinSig>) -> Option<Seed> {
        if !verify_seed_partial(&self.sharing, &self.namespace, self.height, &partial) {
            return None;
        }
        self.insert(partial)
    }

    /// Insert a partial the CALLER has already verified against the public
    /// polynomial (e.g. [`crate::beacon::seed_actor::SeedSigner`] pre-validates
    /// before opening a round, so an invalid partial never allocates one).
    /// Skips the re-verification `add_partial` performs.
    pub fn add_verified_partial(&mut self, partial: PartialSignature<MinSig>) -> Option<Seed> {
        self.insert(partial)
    }

    /// Insert a partial whose validity is already established (own partial, or
    /// a peer partial that passed `verify_seed_partial`) and attempt recovery.
    fn insert(&mut self, partial: PartialSignature<MinSig>) -> Option<Seed> {
        if self.done {
            return None;
        }
        self.partials.insert(partial.index, partial);
        self.try_recover()
    }

    fn try_recover(&mut self) -> Option<Seed> {
        if self.partials.len() < self.quorum {
            return None;
        }
        let collected: Vec<_> = self.partials.values().cloned().collect();
        let seed = recover_seed(&self.sharing, &collected, self.height).ok()?;
        // Accept only a seed that verifies against the group public key — makes
        // recovery correctness independent of the partial set's provenance.
        if verify_seed(self.sharing.public(), &self.namespace, &seed) {
            self.done = true;
            Some(seed)
        } else {
            None
        }
    }

    pub fn height(&self) -> u64 {
        self.height
    }

    /// Distinct verified partials collected so far.
    pub fn collected(&self) -> usize {
        self.partials.len()
    }

    pub fn is_done(&self) -> bool {
        self.done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beacon::seed::{prev_randao_from_seed, seed_namespace};
    use crate::beacon::seed_cache::SeedCache;
    use commonware_cryptography::bls12381::dkg::deal_anonymous;
    use commonware_utils::{test_rng, NZU32};

    fn committee(n: u32) -> (Sharing<MinSig>, Vec<Share>) {
        let mut rng = test_rng();
        deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(n))
    }

    // The live path end to end (sans network): each committee member starts a
    // round and broadcasts its partial; cross-feeding the partials into one
    // node's round recovers a seed that verifies against PK_epoch and lands in
    // the deriver's seed cache as a usable prev_randao.
    #[test]
    fn quorum_of_partials_recovers_seed_into_cache() {
        let (sharing, shares) = committee(5); // n=5 → N3f1 quorum = 4
        let ns = seed_namespace(b"fluent-devnet");
        let height = 100u64;

        // Every member's own partial (what they would broadcast).
        let mut broadcasts = Vec::new();
        for share in &shares {
            let (_round, own) = SeedRound::start(height, ns.clone(), sharing.clone(), share);
            broadcasts.push(own);
        }

        // One node collects: it starts with its own (index 0) then ingests the
        // others until quorum recovers.
        let (mut round, _own) = SeedRound::start(height, ns.clone(), sharing.clone(), &shares[0]);
        let mut recovered = None;
        for partial in broadcasts.iter().skip(1) {
            if let Some(seed) = round.add_partial(partial.clone()) {
                recovered = Some(seed);
                break;
            }
        }

        let seed = recovered.expect("seed recovered at quorum");
        assert_eq!(seed.target_height, height);
        assert!(verify_seed(sharing.public(), &ns, &seed));

        // It is usable as prev_randao and survives in the deriver's cache.
        let mut cache = SeedCache::default();
        cache.insert(seed.clone());
        assert_eq!(
            prev_randao_from_seed(cache.get(height).unwrap()),
            prev_randao_from_seed(&seed)
        );
    }

    #[test]
    fn below_quorum_yields_nothing() {
        let (sharing, shares) = committee(5); // quorum = 4
        let ns = seed_namespace(b"fluent-devnet");
        let height = 7u64;
        let (mut round, _own) = SeedRound::start(height, ns.clone(), sharing.clone(), &shares[0]);
        // Two more partials (3 of 4) — still short of quorum.
        for share in &shares[1..3] {
            let (_r, p) = SeedRound::start(height, ns.clone(), sharing.clone(), share);
            assert!(round.add_partial(p).is_none());
        }
        assert_eq!(round.collected(), 3);
        assert!(!round.is_done());
    }

    #[test]
    fn duplicate_index_does_not_count_twice() {
        let (sharing, shares) = committee(5);
        let ns = seed_namespace(b"fluent-devnet");
        let height = 9u64;
        let (mut round, _own) = SeedRound::start(height, ns.clone(), sharing.clone(), &shares[0]);
        let (_r, p1) = SeedRound::start(height, ns.clone(), sharing.clone(), &shares[1]);
        round.add_partial(p1.clone());
        round.add_partial(p1.clone()); // same signer index again
        round.add_partial(p1);
        assert_eq!(
            round.collected(),
            2,
            "a repeated signer index is not double-counted"
        );
    }

    #[test]
    fn invalid_partial_is_dropped() {
        let (sharing, shares) = committee(5);
        let ns = seed_namespace(b"fluent-devnet");
        let height = 11u64;
        let (mut round, _own) = SeedRound::start(height, ns.clone(), sharing.clone(), &shares[0]);
        // A partial signed under a DIFFERENT namespace must not verify.
        let wrong_ns = seed_namespace(b"other-chain");
        let (_r, bad) = SeedRound::start(height, wrong_ns, sharing.clone(), &shares[1]);
        assert!(round.add_partial(bad).is_none());
        assert_eq!(
            round.collected(),
            1,
            "invalid partial dropped, not collected"
        );
    }

    // A verifier-only node (no share) recovers purely from peers' partials.
    #[test]
    fn verifier_only_round_recovers_from_peers() {
        let (sharing, shares) = committee(4); // quorum = 3
        let ns = seed_namespace(b"fluent-devnet");
        let height = 42u64;
        let mut round = SeedRound::observe(height, ns.clone(), sharing.clone());
        let mut recovered = None;
        for share in &shares {
            let (_r, p) = SeedRound::start(height, ns.clone(), sharing.clone(), share);
            if let Some(seed) = round.add_partial(p) {
                recovered = Some(seed);
                break;
            }
        }
        let seed = recovered.expect("verifier-only recovery at quorum");
        assert!(verify_seed(sharing.public(), &ns, &seed));
    }
}
