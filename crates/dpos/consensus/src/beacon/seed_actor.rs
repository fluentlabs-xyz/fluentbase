//! `SeedSigner` — the per-node brain of the beacon seed sub-protocol: one
//! [`SeedRound`] per in-flight ordering height, plus this node's DKG share and
//! a handle to the deriver's seed cache.
//!
//! The networked actor is a thin I/O shell around this: it calls
//! [`SeedSigner::on_finalized`] when this node is about to NOTARIZE a height
//! (sign-at-notarize — the app's `verify→true` / own-propose path drives the
//! [`FinalizedFeed`]), broadcasting the returned own partial, and
//! [`SeedSigner::on_partial`] for each partial received from a peer. Triggering
//! at notarize (round-1) — not after finalization — is what makes `seed(h)`
//! recoverable BY the time `h` finalizes: a node holding a notarize quorum
//! holds ≥t partials. When a round recovers, the seed is written into the shared
//! [`SeedCache`] from which `derive(h)` reads `prev_randao(h) = H(seed(h))`. All
//! state transitions live here (no I/O) so they are unit-testable without the
//! network or a real DKG.

use crate::beacon::seed::verify_seed_partial;
use crate::beacon::seed_cache::SeedCache;
use crate::beacon::seed_round::SeedRound;
use crate::beacon::types::Seed;
use crate::beacon::wire::BeaconMessage;
use commonware_codec::{Encode as _, ReadExt as _};
use commonware_cryptography::bls12381::primitives::{
    group::Share,
    sharing::Sharing,
    variant::{MinSig, PartialSignature},
};
use commonware_p2p::{Receiver, Recipients, Sender};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::debug;

/// How many heights around the highest finalized height to keep open rounds
/// for. Late partials for an evicted height are dropped (their seed either
/// already recovered or missed its derive window → gated fallback). A round
/// must stay open at least as long as its recovered seed is cached, so this is
/// pinned to the seed cache's window — ONE source of truth, no independent
/// retune that could prune a round before its seed is read.
pub const DEFAULT_ROUND_RETAIN: u64 = crate::beacon::seed_cache::DEFAULT_SEED_RETAIN;

/// Per-node seed-collection state across heights.
pub struct SeedSigner {
    namespace: Vec<u8>,
    sharing: Sharing<MinSig>,
    /// This node's DKG share, or `None` for a verifier-only node (no share this
    /// epoch — it still collects peers' partials toward recovery, the Q4
    /// verifier-only path).
    share: Option<Share>,
    /// Live channel to the deriver: recovered seeds land here and are read as
    /// `prev_randao` (Decision C4).
    cache: Arc<Mutex<SeedCache>>,
    rounds: BTreeMap<u64, SeedRound>,
    retain: u64,
    highest: u64,
}

impl SeedSigner {
    pub fn new(
        namespace: Vec<u8>,
        sharing: Sharing<MinSig>,
        share: Option<Share>,
        cache: Arc<Mutex<SeedCache>>,
        retain: u64,
    ) -> Self {
        Self {
            namespace,
            sharing,
            share,
            cache,
            rounds: BTreeMap::new(),
            retain,
            highest: 0,
        }
    }

    /// This node is about to NOTARIZE height `h` (sign-at-notarize): open its
    /// round and, if this node holds a share, contribute our own partial.
    /// Returns the partial to broadcast over `BEACON_CHANNEL` (`None` for a
    /// verifier-only node). A seed recovered by our own contribution (a peer
    /// majority already held) is written to the cache before returning.
    pub fn on_finalized(&mut self, height: u64) -> Option<PartialSignature<MinSig>> {
        self.highest = self.highest.max(height);
        self.prune();
        let share = self.share.clone()?;
        let round = self.round_for(height);
        let (own, recovered) = round.contribute(&share);
        if let Some(seed) = recovered {
            self.commit(seed);
        }
        Some(own)
    }

    /// Ingest a partial received from a peer for `height`. On recovery the seed
    /// is written into the deriver's cache and returned. Partials for heights
    /// already pruned (below the retain floor) open a fresh verifier round,
    /// which is harmless — it will simply never reach quorum and is pruned in
    /// turn.
    pub fn on_partial(&mut self, height: u64, partial: PartialSignature<MinSig>) -> Option<Seed> {
        // Bound the live round set near the finalized tip. `prune` only evicts
        // heights BELOW `highest - retain`, so without this a peer (a Byzantine
        // committee member, the only sender on BEACON_CHANNEL) could broadcast
        // valid partials for arbitrary far-FUTURE heights and open unbounded,
        // never-pruned rounds (each cloning the public `Sharing`) → OOM.
        if height > self.highest.saturating_add(self.retain)
            || height < self.highest.saturating_sub(self.retain)
        {
            return None;
        }
        // Verify BEFORE creating the round so an invalid partial never allocates
        // a (Sharing-cloning) round entry; `round_for` then inserts only for a
        // partial that already passed the public-polynomial check.
        if !verify_seed_partial(&self.sharing, &self.namespace, height, &partial) {
            return None;
        }
        let seed = self.round_for(height).add_verified_partial(partial);
        if let Some(seed) = &seed {
            self.commit(seed.clone());
        }
        seed
    }

    fn commit(&mut self, seed: Seed) {
        self.cache.lock().expect("seed cache mutex").insert(seed);
        // The round is NOT removed: its latched `done` flag makes later partials
        // for this height no-ops (no re-open, no double-commit). It is dropped
        // by height-window pruning in due course.
    }

    fn round_for(&mut self, height: u64) -> &mut SeedRound {
        let (ns, sharing) = (&self.namespace, &self.sharing);
        self.rounds
            .entry(height)
            .or_insert_with(|| SeedRound::observe(height, ns.clone(), sharing.clone()))
    }

    /// Drop rounds more than `retain` heights below the highest finalized.
    fn prune(&mut self) {
        let floor = self.highest.saturating_sub(self.retain);
        self.rounds = self.rounds.split_off(&floor);
    }

    /// Rounds still collecting (not yet recovered) — test/metrics visibility.
    /// Recovered rounds linger (latched `done`) until pruned but are not "open".
    pub fn open_rounds(&self) -> usize {
        self.rounds.values().filter(|r| !r.is_done()).count()
    }
}

/// Feeds about-to-be-notarized heights to the [`SeedActor`]. The app sends a
/// height here the moment this node decides to notarize it (`verify→true` or
/// own propose) so the actor signs+broadcasts its partial at notarize-time;
/// clone is cheap (an `mpsc` sender).
#[derive(Clone, Debug)]
pub struct FinalizedFeed {
    tx: mpsc::UnboundedSender<u64>,
}

impl FinalizedFeed {
    /// Non-blocking; a closed actor (shutdown) drops the height silently.
    pub fn notify(&self, height: u64) {
        let _ = self.tx.send(height);
    }
}

/// The networked I/O shell around [`SeedSigner`]: it broadcasts this node's own
/// partial when a height finalizes and ingests peers' partials off
/// `BEACON_CHANNEL`, driving recovered seeds into the deriver cache. All the
/// protocol state lives in [`SeedSigner`]; this layer is just transport.
pub struct SeedActor<S, R> {
    signer: SeedSigner,
    sender: S,
    receiver: R,
    finalized_rx: mpsc::UnboundedReceiver<u64>,
}

impl<S, R> SeedActor<S, R>
where
    S: Sender,
    R: Receiver,
{
    /// Build the actor over an (already per-epoch-muxed) `BEACON_CHANNEL`
    /// sender/receiver, returning the [`FinalizedFeed`] the reporter writes to.
    pub fn new(signer: SeedSigner, sender: S, receiver: R) -> (Self, FinalizedFeed) {
        let (tx, finalized_rx) = mpsc::unbounded_channel();
        (
            Self {
                signer,
                sender,
                receiver,
                finalized_rx,
            },
            FinalizedFeed { tx },
        )
    }

    /// Run until both inputs close. On a finalized height: contribute + broadcast
    /// our partial (best-effort; offline peers recover from their own rounds). On
    /// an inbound message: decode and feed seed partials to the signer (DKG
    /// envelopes belong to the DKG actor and are ignored here).
    pub async fn run(mut self) {
        loop {
            tokio::select! {
                maybe_height = self.finalized_rx.recv() => {
                    let Some(height) = maybe_height else { break };
                    if let Some(own) = self.signer.on_finalized(height) {
                        let msg = BeaconMessage::SeedPartial { height, partial: own };
                        let _ = self
                            .sender
                            .send(Recipients::All, msg.encode(), false)
                            .await;
                    }
                }
                inbound = self.receiver.recv() => {
                    let Ok((_from, mut payload)) = inbound else { break };
                    match BeaconMessage::read(&mut payload) {
                        Ok(BeaconMessage::SeedPartial { height, partial }) => {
                            self.signer.on_partial(height, partial);
                        }
                        // DKG traffic shares the channel but is the DKG actor's.
                        Ok(BeaconMessage::Dkg(_)) => {}
                        Err(error) => debug!(?error, "dropping undecodable beacon message"),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beacon::seed::{prev_randao_from_seed, seed_namespace, verify_seed};
    use commonware_cryptography::bls12381::dkg::deal_anonymous;
    use commonware_utils::{test_rng, N3f1, NZU32};

    /// A committee of `n` signers, each wrapped in its own `SeedSigner` sharing
    /// the public polynomial — mirroring `n` nodes on the live network.
    fn committee(n: u32) -> (Sharing<MinSig>, Vec<SeedSigner>) {
        let mut rng = test_rng();
        let (sharing, shares) =
            deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(n));
        let ns = seed_namespace(b"fluent-devnet");
        let signers = shares
            .into_iter()
            .map(|share| {
                SeedSigner::new(
                    ns.clone(),
                    sharing.clone(),
                    Some(share),
                    Arc::new(Mutex::new(SeedCache::default())),
                    DEFAULT_ROUND_RETAIN,
                )
            })
            .collect();
        (sharing, signers)
    }

    // The full per-node flow (sans the network transport): every node signs on
    // finalization and broadcasts; delivering those partials to one node's
    // signer recovers seed(h) and writes it into that node's deriver cache.
    #[test]
    fn finalization_then_partials_populate_the_cache() {
        let (sharing, mut signers) = committee(5); // quorum = 4
        let ns = seed_namespace(b"fluent-devnet");
        let height = 200u64;

        // Each node observes the finalization and emits its own partial.
        let broadcasts: Vec<_> = signers
            .iter_mut()
            .map(|s| s.on_finalized(height).expect("share node contributes"))
            .collect();

        // Node 0 receives the others' partials until recovery.
        let node0 = &mut signers[0];
        for partial in broadcasts.iter().skip(1) {
            node0.on_partial(height, partial.clone());
        }

        let cache = node0.cache.lock().unwrap();
        let seed = cache.get(height).expect("seed in node0 cache");
        assert_eq!(seed.target_height, height);
        assert!(verify_seed(sharing.public(), &ns, seed));
        // Round closed out once recovered.
        drop(cache);
        assert_eq!(node0.open_rounds(), 0);
    }

    // A partial arriving BEFORE this node sees the height finalize is buffered
    // in a verifier round; the node's own later contribution then completes it.
    #[test]
    fn out_of_order_partial_before_finalization_still_recovers() {
        let (_sharing, mut signers) = committee(4); // quorum = 3
        let ns = seed_namespace(b"fluent-devnet");
        let height = 5u64;

        // Collect peers' partials first.
        let peer_partials: Vec<_> = signers[1..]
            .iter_mut()
            .map(|s| s.on_finalized(height).unwrap())
            .collect();

        // Node 0 ingests two peers BEFORE it observes finalization (no share
        // contributed yet → below quorum of 3).
        assert!(signers[0]
            .on_partial(height, peer_partials[0].clone())
            .is_none());
        assert!(signers[0]
            .on_partial(height, peer_partials[1].clone())
            .is_none());
        assert_eq!(signers[0].open_rounds(), 1, "verifier round buffered");

        // Now node 0 finalizes and contributes its own → reaches quorum 3.
        let _own = signers[0].on_finalized(height).expect("contributes");
        assert!(
            signers[0].cache.lock().unwrap().get(height).is_some(),
            "own contribution completes the buffered round"
        );
        let _ = ns;
    }

    // A verifier-only node (no share) never emits a partial but still recovers
    // the seed from peers and caches it.
    #[test]
    fn verifier_only_node_caches_from_peers() {
        let (sharing, mut signers) = committee(4); // quorum = 3
        let ns = seed_namespace(b"fluent-devnet");
        let height = 77u64;
        let peer_partials: Vec<_> = signers
            .iter_mut()
            .map(|s| s.on_finalized(height).unwrap())
            .collect();

        let mut verifier = SeedSigner::new(
            ns.clone(),
            sharing.clone(),
            None, // no share
            Arc::new(Mutex::new(SeedCache::default())),
            DEFAULT_ROUND_RETAIN,
        );
        assert!(
            verifier.on_finalized(height).is_none(),
            "no share → no partial"
        );
        for partial in &peer_partials {
            verifier.on_partial(height, partial.clone());
        }
        let cache = verifier.cache.lock().unwrap();
        let seed = cache.get(height).expect("verifier recovered seed");
        assert_eq!(prev_randao_from_seed(seed), prev_randao_from_seed(seed));
        assert!(verify_seed(sharing.public(), &ns, seed));
    }

    // Rounds far below the highest finalized height are pruned so memory stays
    // bounded under sustained operation.
    #[test]
    fn stale_rounds_are_pruned() {
        let (_sharing, mut signers) = committee(4);
        // A single valid peer partial (same committee) opens a sub-quorum round
        // at height 3 on node 0.
        let peer = signers[1].on_finalized(3).unwrap();
        signers[0].on_partial(3, peer);
        assert_eq!(signers[0].open_rounds(), 1);
        // Advancing finalization far past the retain window evicts height 3
        // (only the new high round remains).
        signers[0].on_finalized(3 + DEFAULT_ROUND_RETAIN + 10);
        assert_eq!(
            signers[0].open_rounds(),
            1,
            "stale height-3 round pruned; only the new high round remains"
        );
    }
}
