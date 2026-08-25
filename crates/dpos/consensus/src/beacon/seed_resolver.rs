//! Serving and requesting σ by round.
//!
//! The last of the three sources of a σ this node did not produce, and the
//! smallest: ingress capture covers every round whose certificate the node saw,
//! which is the general case. What is left for this seam is what no ingress can
//! reach — an ancestry-finalized parent with no standalone certificate, two
//! nodes whose archives name different spin rounds for one height, and a node
//! that entered after the fact.
//!
//! SERVE BEFORE YOU REQUEST: the answering half is the load-bearing one. A
//! network where everyone asks and nobody answers is worse than one where
//! nobody asks.

use super::{
    certify::SeedStore,
    keys::{BeaconKeys, InvalidSeed},
    metrics::BeaconMetrics,
    oracle::KeyOnlyOracle,
};
use crate::beacon::verified_seed::VerifiedSeed;
use bytes::Bytes;
use commonware_codec::{DecodeExt as _, Encode as _};
use commonware_consensus::types::Round;
use fluentbase_bls::{
    oracle::{SeedCheck, SeedOracle},
    BlsSignature,
};
use futures::future::BoxFuture;
use std::sync::Arc;
use tracing::warn;

/// The `Producer`/`Consumer` half of the beacon resolver that owns
/// [`BeaconFetchKey::Seed`](super::log_resolver::BeaconFetchKey::Seed).
///
/// Holds the pieces rather than a `Randomness` handle because it is built before
/// that handle exists — and because the two things it needs, the served map and
/// the key store, are exactly the two the surface would have forwarded to.
#[derive(Clone)]
pub struct SeedBridge {
    seeds: SeedStore,
    keys: BeaconKeys,
    namespace: Vec<u8>,
    metrics: BeaconMetrics,
    /// Below this epoch no σ exists to serve or to hold an opinion about. Kept
    /// here rather than assumed so this seam refuses the same epochs
    /// `Randomness::oracle_for` refuses, from one written rule.
    bootstrap: u64,
}

impl SeedBridge {
    pub fn new(
        seeds: SeedStore,
        keys: BeaconKeys,
        namespace: Vec<u8>,
        metrics: BeaconMetrics,
        bootstrap: u64,
    ) -> Self {
        Self {
            seeds,
            keys,
            namespace,
            metrics,
            bootstrap,
        }
    }

    fn oracle(&self, epoch: u64) -> Option<Arc<dyn SeedOracle>> {
        (epoch >= self.bootstrap).then(|| {
            Arc::new(KeyOnlyOracle {
                epoch,
                keys: self.keys.clone(),
                namespace: self.namespace.clone(),
                metrics: self.metrics.clone(),
            }) as Arc<dyn SeedOracle>
        })
    }

    /// Answer a request for the range starting at `from`, or `None` when this
    /// node does not hold it.
    ///
    /// `None` means the caller must DROP the responder rather than send empty
    /// bytes, and that distinction decides whether the requester keeps looking.
    /// A sent response — empty or not — completes the fetch: the resolver counts
    /// it a success, clears the targets and never asks a second peer, and it
    /// credits the peer that answered fastest, so an instantly-empty answer also
    /// promotes that peer to first pick for every later beacon fetch. A dropped
    /// responder is the "no data" path the dealer-log arm already uses.
    ///
    /// Single-round service against a ranged key: the responder answers `from`
    /// and nothing else.
    ///
    /// Served ONLY out of the checked map. A node that cannot tell a good σ from
    /// garbage cannot relay one without becoming an involuntary amplifier, so
    /// the quarantine is not a source: the seam physically cannot name it.
    pub fn produce(&self, from: Round) -> Option<Bytes> {
        self.seeds
            .lookup(from)
            .map(|seed| Bytes::from(seed.encode().to_vec()))
    }

    /// Take a served σ for `from`.
    ///
    /// Returns `false` only for proven misbehaviour, and the bar is high because
    /// the price is not: a `false` puts the peer in the fetcher's `excluded` set,
    /// which is FETCHER-WIDE and has no removal path — the peer stops serving
    /// dealer logs and epoch-key artifacts too.
    ///
    /// So a signature that fails is proven misbehaviour only when the key it
    /// failed against is one a `committee[epoch]` quorum attested. Judged against
    /// a locally reconstructed key ([`KeySource::LocalDkg`], which can diverge
    /// from the chain) it proves nothing, and punishing on it would exclude every
    /// honest peer serving the right σ — including from the artifact pull that is
    /// the only thing that could replace the divergent key.
    pub fn deliver(&self, from: Round, value: &[u8]) -> bool {
        // NO early return for empty bytes. On this key space an honest producer
        // cannot send them: `produce` drops the responder on a miss, and the
        // resolver turns a dropped responder into `Payload::Error`, which
        // re-queues the fetch. A sent EMPTY response is therefore proven
        // misbehaviour by this protocol's own construction — and rewarding it
        // with `true` would be worse than useless, because the resolver counts a
        // delivered value as a COMPLETED fetch: one peer answering every seed
        // request with zero bytes would end every requester's search at itself,
        // and its instant reply would win the latency ranking on the way. Falls
        // through to the decode below, which fails on empty.
        let Ok(seed) = BlsSignature::decode(value) else {
            warn!(round = ?from, "seed response does not decode");
            return false;
        };
        let Some(oracle) = self.oracle(from.epoch().get()) else {
            return true;
        };
        match VerifiedSeed::check(oracle.as_ref(), from, seed) {
            Ok(verified) => {
                self.seeds.record(verified);
                true
            }
            Err(SeedCheck::NoKey) => {
                self.seeds.quarantine(from, seed);
                true
            }
            // ONE rule, and it lives in the key store because it turns on
            // provenance. An unattested key cannot convict the sender — and the
            // value is held rather than dropped, or this node would stay broken
            // after the attested key arrived with nothing left to re-check.
            Err(SeedCheck::Invalid) => match self.keys.on_invalid_seed(from.epoch().get()) {
                InvalidSeed::Quarantine => {
                    self.seeds.quarantine(from, seed);
                    true
                }
                InvalidSeed::RefuseLoud => {
                    warn!(round = ?from, "served seed does not verify under its epoch key");
                    false
                }
                InvalidSeed::RefuseQuiet => false,
            },
            Err(SeedCheck::Valid) => unreachable!("Valid is the Ok arm"),
        }
    }
}

/// Ask peers for σ of one round and wait a bounded time for it to land.
///
/// `true` iff the served map holds the round when the future resolves. Built in
/// `plane.rs`, where the resolver mailbox and the store exist together.
pub type PullSeed = Arc<dyn Fn(Round) -> BoxFuture<'static, bool> + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beacon::{keys::KeySource, verified_seed::PkOracle};
    use commonware_consensus::types::{Epoch, View};
    use commonware_cryptography::bls12381::{
        dkg::deal_anonymous,
        primitives::variant::MinSig,
    };
    use commonware_utils::{test_rng, N3f1, NZU32};
    use fluentbase_bls::beacon::{recover_seed, seed_namespace, sign_seed_partial};

    const EPOCH: u64 = 5;

    fn round(view: u64) -> Round {
        Round::new(Epoch::new(EPOCH), View::new(view))
    }

    /// A dealt committee plus the σ it produces, so a test can hand the bridge
    /// bytes a real network would have sent.
    struct Dealt {
        pk: fluentbase_bls::beacon::GroupPublic,
        namespace: Vec<u8>,
        sigma: Box<dyn Fn(Round) -> BlsSignature>,
    }

    fn dealt() -> Dealt {
        let mut rng = test_rng();
        let (sharing, shares) = deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(5));
        let namespace = seed_namespace(b"fluent-test");
        let pk = *sharing.public();
        let ns = namespace.clone();
        let sigma = move |r: Round| {
            let partials: Vec<_> = shares.iter().map(|s| sign_seed_partial(s, &ns, r)).collect();
            recover_seed::<N3f1>(&sharing, &partials).expect("recover")
        };
        Dealt {
            pk,
            namespace,
            sigma: Box::new(sigma),
        }
    }

    fn bridge(namespace: Vec<u8>, pk: Option<fluentbase_bls::beacon::GroupPublic>) -> SeedBridge {
        let keys = BeaconKeys::default();
        if let Some(pk) = pk {
            keys.set_pk(EPOCH, pk, KeySource::Agreed);
        }
        SeedBridge::new(
            SeedStore::new(),
            keys,
            namespace,
            BeaconMetrics::default(),
            0,
        )
    }

    // Serve only what this node checked. A node that cannot tell a good σ from
    // garbage cannot relay one without becoming an amplifier, so the quarantine
    // is not reachable from the serving path — not by policy, by construction.
    #[test]
    fn only_the_checked_map_is_served() {
        let Dealt { pk, namespace: ns, sigma } = dealt();
        let seam = bridge(ns.clone(), Some(pk));
        let r = round(3);
        assert!(seam.produce(r).is_none(), "nothing held, nothing served");

        seam.seeds.quarantine(r, sigma(r));
        assert!(
            seam.produce(r).is_none(),
            "a held-but-unchecked seed is not servable"
        );

        seam.seeds
            .record(PkOracle::new(pk, ns).witness(r, sigma(r)));
        assert!(seam.produce(r).is_some(), "a checked seed is served");
    }

    // `deliver` answers `false` ONLY for proven misbehaviour, because commonware's
    // `excluded` set has no removal: an honest peer punished once stays punished.
    #[test]
    fn deliver_refuses_only_proven_misbehaviour() {
        let Dealt { pk, namespace: ns, sigma } = dealt();
        let r = round(3);

        let known = bridge(ns.clone(), Some(pk));
        assert!(known.deliver(r, &sigma(r).encode()));
        assert!(known.seeds.lookup(r).is_some(), "a checked seed is served");

        // A σ of a different round, judged against an ATTESTED key: proven, and
        // the peer pays for it.
        assert!(!known.deliver(round(4), &sigma(r).encode()));
        assert!(!known.deliver(r, b"not a signature"));
        assert!(
            known.seeds.lookup(round(4)).is_none()
                && known.seeds.quarantined_epochs().is_empty(),
            "a refused seed is dropped, never stored"
        );

        // Same bytes, but this node holds no key: it cannot know, so it must not
        // punish — and must not serve them either.
        let keyless = bridge(ns.clone(), None);
        assert!(keyless.deliver(round(4), &sigma(r).encode()));
        assert!(keyless.seeds.lookup(round(4)).is_none());
        assert_eq!(keyless.seeds.quarantined_epochs(), vec![EPOCH]);

        // An EMPTY response is proven misbehaviour on this key space, not an
        // honest negative: `produce` drops the responder on a miss, and the
        // resolver turns that into an error that re-queues the fetch. Answering
        // `true` here would let ONE peer end every requester's search at itself
        // — the resolver counts a delivered value as a completed fetch — and its
        // instant reply would win the latency ranking on the way.
        assert!(!known.deliver(r, &[]));

        // And the same bytes against a key this node DERIVED rather than agreed:
        // the failure proves nothing about the peer, because the key itself can
        // diverge from the chain. Punishing here would exclude an honest peer
        // from the artifact pull that is the only cure for the divergence.
        let derived = bridge(ns, None);
        derived.keys.set_pk(EPOCH, pk, KeySource::LocalDkg);
        assert!(
            derived.deliver(round(4), &sigma(r).encode()),
            "a locally derived key is not grounds to exclude a peer"
        );
        assert_eq!(
            derived.seeds.quarantined_epochs(),
            vec![EPOCH],
            "and the value is HELD, or the node stays broken after the attested              key arrives with nothing left to re-check"
        );
    }
}
