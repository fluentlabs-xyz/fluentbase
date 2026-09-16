//! The dealer-log serve store: the cached and durable tiers behind "give me the
//! `SignedDealerLog` for `{epoch, dealer, hash}`" — the exact body, never "a log of
//! that dealer": an equivocating dealer's two logs are both servable, each under
//! its own hash.
//!
//! The DKG-log recovery `Producer` ([`DkgActor::serve_log`](crate::beacon::actor))
//! has three sources, in order: the live ceremony's recorded `signed_logs`, the
//! bounded in-memory cache here, and — on a cold miss after a restart — a one-time
//! parse of the durable per-epoch journal. Only the last two live in this module;
//! the live ceremony stays with the actor that owns it, so a ceremony's lifetime
//! never leaks into the serve path.
//!
//! The cache is a strict subset copy of the durable journal, not a second source
//! of truth. It is populated eagerly at finalize ([`DealerLogStore::seed`], so the
//! no-restart hot path never reads disk) and lazily on a cold miss
//! ([`DealerLogStore::get`], which parses and re-`check`s the journal once and
//! caches the result, so subsequent serves are O(1) with no per-request BLS
//! `check`).
//!
//! # The invariant this module owns: never cache a negative
//!
//! The cache holds only non-empty positive maps — an epoch this node finalized, or
//! one it re-parsed from a present journal into at least one `check`-valid log.
//! Every write goes through the private [`DealerLogStore::cache_positive`], and no
//! method on this type takes a map and stores it unconditionally, so the rule
//! cannot be bypassed from outside. Two failures are what the rule exists for:
//!
//! - A transiently unreadable committee. `committee_for` is an EVM read and can
//!   answer `None`; a cold parse under that race yields an empty map for an epoch
//!   whose journal is fully present, and caching it would make the emptiness
//!   permanent for the retention window. Not caching it costs one re-parse and the
//!   next serve is correct.
//! - Unbounded growth. `key.epoch` is attacker-controlled wire input and a
//!   far-future epoch has no journal file, so its parse is empty. Caching empties
//!   would let one Byzantine peer grow the cache by one entry per distinct epoch it
//!   names; declining keeps the map bounded by the epochs this node actually
//!   finalized inside the retention window.
//!
//! The residual is a genuinely present-but-torn journal for one of our own served
//! epochs: it re-parses per request, bounded by the resolver quota and by the
//! finalize-to-boundary window. It is not attacker-inducible.
//!
//! # Retention
//!
//! [`DealerLogStore::retain`] ages the cache out on the actor's one window
//! ([`JOURNAL_RETENTION_EPOCHS`](crate::beacon::JOURNAL_RETENTION_EPOCHS)) and
//! returns the epochs it dropped, which the caller unions into its own
//! journal-reclaim pass. The store deliberately does not reclaim: the journal is
//! written by the actor (`append_journal`) and read by paths this store has no part
//! in.

use crate::beacon::{
    actor::CommitteeFor,
    ceremony::{checked_serve_map, LogId},
    dkg_msg::DealerReveal,
    share_state::{self, JournalLoad, ShareState},
};
use bytes::Bytes;
use commonware_codec::Encode as _;
use std::{collections::BTreeMap, num::NonZeroU32, path::PathBuf, sync::Arc};

/// Test-only counter of cold-cache journal parses, so the fetch-burst-bound test can
/// assert "one parse per epoch, not per request". Incremented on each disk parse.
#[cfg(test)]
pub(crate) static COLD_PARSE_COUNT: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// One epoch's servable dealer logs, by `(dealer, hash)`. `Arc` so a serve clones a
/// cheap handle.
pub(crate) type ServeMap = Arc<BTreeMap<LogId, DealerReveal>>;

/// Cached + durable dealer-log lookup for the DKG-log recovery `Producer`. The
/// never-cache-a-negative invariant this type owns is in the module docs.
pub(crate) struct DealerLogStore {
    /// Namespace the journaled logs were signed under; passed to
    /// [`checked_serve_map`] so a re-parse verifies against the same domain.
    namespace: Vec<u8>,
    /// The epoch's committed roster, needed to build the `Info` a journaled log is
    /// `check`ed against. An EVM read, so it may transiently answer `None`.
    committee_for: CommitteeFor,
    /// Where the per-epoch journals live. `None` ⇒ no persistence (in-process/test
    /// default) ⇒ every cold miss is a cheap `NoFile`.
    share_dir: Option<PathBuf>,
    /// At-rest framing of the journal records. Shared with the actor, which writes
    /// them — one instance, so the encrypted arm's seal key is not duplicated.
    share_state: Arc<ShareState>,
    /// Positive-only, epoch-keyed. Written only through [`Self::cache_positive`].
    cache: BTreeMap<u64, ServeMap>,
}

impl DealerLogStore {
    pub(crate) fn new(
        namespace: Vec<u8>,
        committee_for: CommitteeFor,
        share_dir: Option<PathBuf>,
        share_state: Arc<ShareState>,
    ) -> Self {
        Self {
            namespace,
            committee_for,
            share_dir,
            share_state,
            cache: BTreeMap::new(),
        }
    }

    /// Serve the encoded `SignedDealerLog` held under exactly `(epoch, id)` from the
    /// cache, or — on a miss — from a one-time parse of the durable journal, cached
    /// only if it produced anything. A cache hit does no per-request BLS `check`.
    ///
    /// `None` when neither tier holds that exact body (a dealer's other log is not an
    /// answer); the caller drops the responder, the resolver sends an empty "no data"
    /// response and the requester retries elsewhere.
    pub(crate) fn get(&mut self, epoch: u64, id: &LogId) -> Option<Bytes> {
        if let Some(logs) = self.cache.get(&epoch) {
            return logs.get(id).map(|s| s.encode());
        }
        let logs = self.parse_journal(epoch);
        let bytes = logs.get(id).map(|s| s.encode());
        self.cache_positive(epoch, logs);
        bytes
    }

    /// Hand the store a known-good map — the finalize path, which already holds the
    /// ceremony's recorded logs in memory and must not re-read disk to serve them.
    ///
    /// An empty map is dropped, not stored (the invariant). A successful finalize ran
    /// over a dealer-quorum of recorded logs, so this cannot be empty in practice; the
    /// guard is here so the rule holds for the type, not for one caller's argument.
    pub(crate) fn seed(&mut self, epoch: u64, logs: BTreeMap<LogId, DealerReveal>) {
        self.cache_positive(epoch, Arc::new(logs));
    }

    /// Parse the epoch's journal without touching the cache — the recompute-heal's
    /// "which pinned `(dealer, hash)` bodies do I already hold?" read, which asks about
    /// an epoch it is not (yet) serving and must not warm the serve path on that
    /// question alone.
    pub(crate) fn parse_journal(&self, epoch: u64) -> ServeMap {
        let Some(dir) = &self.share_dir else {
            return Arc::new(BTreeMap::new());
        };
        let max = NonZeroU32::new(fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as u32)
            .expect("MAX_COMMITTEE_SIZE > 0");
        let JournalLoad::Present(records) =
            share_state::load_journal(dir, epoch, &self.share_state, max)
        else {
            // An absent/torn journal yields an empty map: a serve declines
            // un-verifiable logs and the resolver retries elsewhere. A boundary-passed
            // epoch already had its journal reconciled/evicted → `NoFile` → empty.
            return Arc::new(BTreeMap::new());
        };
        #[cfg(test)]
        COLD_PARSE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let map = (self.committee_for)(epoch)
            .and_then(|committee| {
                checked_serve_map(&self.namespace, epoch, committee, records).ok()
            })
            .unwrap_or_default();
        Arc::new(map)
    }

    /// Move an epoch off its journal and into the cache: parse it once and keep the
    /// (positive) result, so the epoch stays servable to peers for the rest of the
    /// window after the caller reclaims the now-superseded journal file.
    pub(crate) fn warm_from_journal(&mut self, epoch: u64) {
        let logs = self.parse_journal(epoch);
        self.cache_positive(epoch, logs);
    }

    /// Age the cache out at `floor` (an epoch is kept while `epoch >= floor`) and return
    /// the epochs dropped, so the caller can union them into its one journal-reclaim
    /// pass. See the module docs for why the reclaim itself stays with the caller.
    pub(crate) fn retain(&mut self, floor: u64) -> Vec<u64> {
        let dropped: Vec<u64> = self.cache.range(..floor).map(|(e, _)| *e).collect();
        for e in &dropped {
            self.cache.remove(e);
        }
        dropped
    }

    /// The only write into `cache`. An empty map is never stored — see the
    /// never-cache-a-negative section of the module docs.
    fn cache_positive(&mut self, epoch: u64, logs: ServeMap) {
        if logs.is_empty() {
            return;
        }
        self.cache.insert(epoch, logs);
    }

    /// The cached map for `epoch`, or `None` if the epoch is a miss. Test-only: the
    /// serve path reaches the cache through [`Self::get`].
    #[cfg(test)]
    pub(crate) fn cached(&self, epoch: u64) -> Option<&ServeMap> {
        self.cache.get(&epoch)
    }

    /// Whether the cache holds nothing at all. Test-only — the bound the
    /// unbounded-growth case asserts on.
    #[cfg(test)]
    pub(crate) fn cache_is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::B256;
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
    use commonware_math::algebra::Random as _;
    use commonware_utils::ordered::Set;
    use fluentbase_bls::PeerPubkey;
    use rand_08::{rngs::StdRng, SeedableRng as _};

    /// The never-cache-a-negative invariant at the store's own seam, on both entry
    /// points that can be handed an empty map.
    ///
    /// The end-to-end half — a transiently-unreadable committee does not poison a
    /// present journal, and the next serve succeeds — is pinned over the whole actor
    /// by `actor::tests::transient_committee_none_does_not_poison_serve`; this pins
    /// the rule on the type that owns it.
    #[test]
    fn an_empty_map_is_never_cached() {
        let mut rng = StdRng::seed_from_u64(3);
        let keys: Vec<Ed25519PrivateKey> = (0..4)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let committee: Set<PeerPubkey> = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let committee_for: CommitteeFor = Arc::new(move |_e| Some(committee.clone()));
        // No `share_dir` ⇒ every cold parse is `NoFile` ⇒ an empty map, which is the
        // exact shape of both an attacker's far-future epoch and a torn journal.
        let mut store = DealerLogStore::new(
            b"ns".to_vec(),
            committee_for,
            None,
            Arc::new(ShareState::Plaintext),
        );

        // Entry point 1 — the finalize seed. An empty map is dropped on the floor.
        store.seed(7, BTreeMap::new());
        assert!(
            store.cached(7).is_none(),
            "seeding an empty map caches NOTHING — the invariant is the type's, not the caller's"
        );

        // Entry point 2 — the cold miss. Many distinct attacker-chosen `key.epoch`s
        // never grow the cache, and each stays a miss rather than a cached negative,
        // so a later servable parse of the same epoch is still reachable.
        let id = (keys[1].public_key(), B256::repeat_byte(0x11));
        for epoch in 1_000u64..1_050 {
            assert!(
                store.get(epoch, &id).is_none(),
                "an epoch with no journal serves no log"
            );
            assert!(
                store.cached(epoch).is_none(),
                "an unservable cold miss leaves the epoch UNCACHED — it must re-parse later"
            );
        }
        assert!(
            store.cache_is_empty(),
            "no negative ever enters the cache — it stays bounded by what this node finalized"
        );
    }
}
