//! The shared `round -> recovered seed` map.
//!
//! [`SeedStore`] is written by the notarization [`Reporter`](commonware_consensus::Reporter)
//! ([`crate::spec_exec::Mailbox`]) and read synchronously by `build_proposal`'s
//! parent-seed witness and by the executor's finalized derive.
//!
//! ## Why there is no beacon gate at `certify` any more
//!
//! There used to be one, and it existed for exactly one reason: a CHANGE-epoch
//! boundary block ASSERTED its own `PK_E` in `OrderBlock.beacon_outcome`, so the
//! round's recovered seed had to be checked against that assertion before it
//! finalized and became authoritative on-chain. The epoch key is now agreed on
//! the p2p agreement plane and delivered as a quorum-signed artifact; no block
//! asserts a key, so there is nothing at `certify` left to check. Every block's
//! seed is pinned at notarization-accept by
//! [`CombinedScheme::verify_certificate`](fluentbase_bls) against the resolvable
//! `PK_E`, which is what the gate delegated to for non-boundary blocks all along.
//! The per-epoch engine hands `Inline` to simplex directly, and `Inline`'s own
//! `certify` keeps the availability gate.

use commonware_consensus::types::Round;
use fluentbase_bls::BlsSignature;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::sync::{mpsc, Notify};
use tracing::warn;

/// Bound on retained `round → seed` entries. A reader wants the seed for a round
/// within a tiny trailing window of its notarization. Generous slack: a seed is
/// 48 B, so a few thousand entries is negligible memory, but a round that
/// notarizes while this node lags a long single block at a boundary may stay
/// wanted for many notarizations — evicting it would cost the witness that block
/// needs. Size the window well past any realistic in-flight backlog.
pub(crate) const SEED_RETENTION: usize = 4096;

/// Shared, bounded `round → recovered seed` map. Written by the notarization
/// [`Reporter`](commonware_consensus::Reporter) ([`crate::spec_exec::Mailbox`])
/// via [`SeedStore::record`], read by `build_proposal`'s witness embed and the
/// executor's finalized derive. A newtype rather
/// than a bare alias so the [`SEED_RETENTION`] eviction is the ONLY insertion path:
/// holders cannot lock the inner map and grow it unbounded.
///
/// Every [`record`](SeedStore::record) fires the `notify` permit, which the
/// executor awaits in a `select!` arm to re-run the eager finalized derive of a
/// HELD tip whose own round's seed had not yet landed (the record-vs-delivery
/// race, formerly closed by the `SpecNotarized` Poke). `notify_one` stores a
/// permit even with no waiter, so a record that lands between the executor's
/// miss lookup and its next await is NOT lost — no lost-notification window.
///
/// The map is RAM; [`crate::beacon::seed_journal`] is its durable mirror, and
/// [`SeedStore::with_persistence`] is how the two are joined at startup. Reads
/// never touch disk — [`lookup`](SeedStore::lookup) has to stay synchronous
/// because `certify` and `build_proposal` call it without an await.
#[derive(Clone)]
pub struct SeedStore {
    seeds: Arc<Mutex<BTreeMap<Round, BlsSignature>>>,
    notify: Arc<Notify>,
    /// Durable sink. `None` ⇒ RAM-only, the pre-durability behaviour, which is
    /// what every test and any config without a journal partition gets.
    ///
    /// `UnboundedSender::send` is SYNCHRONOUS and never blocks, which is what
    /// lets the durable write sit inside [`record`](SeedStore::record) without
    /// breaking the ORDERING-CRITICAL contract in [`crate::spec_exec`]. That
    /// contract constrains in-RAM visibility before the same round's certify
    /// scan; durability is only ever read by a LATER process. Do NOT swap this
    /// for a bounded channel — `send().await` would put the reporter behind an
    /// await, which the contract forbids.
    persist: Option<mpsc::UnboundedSender<(Round, BlsSignature)>>,
}

impl SeedStore {
    /// Construct an empty, RAM-only store.
    pub fn new() -> Self {
        Self {
            seeds: Arc::new(Mutex::new(BTreeMap::new())),
            notify: Arc::new(Notify::new()),
            persist: None,
        }
    }

    /// Construct a store backed by the durable journal, pre-loaded with the
    /// window replayed from it.
    ///
    /// `rehydrated` is inserted directly rather than through
    /// [`record`](Self::record): those entries came OUT of the journal and must
    /// not be written back into it. Truncating here preserves the reason
    /// [`record`] is the only other insertion path — the map cannot exceed
    /// [`SEED_RETENTION`] by this route either.
    pub fn with_persistence(
        rehydrated: Vec<(Round, BlsSignature)>,
        persist: mpsc::UnboundedSender<(Round, BlsSignature)>,
    ) -> Self {
        let mut map: BTreeMap<Round, BlsSignature> = rehydrated.into_iter().collect();
        while map.len() > SEED_RETENTION {
            map.pop_first();
        }
        Self {
            seeds: Arc::new(Mutex::new(map)),
            notify: Arc::new(Notify::new()),
            persist: Some(persist),
        }
    }

    /// Record the recovered seed for `round`, evicting the oldest entries past
    /// [`SEED_RETENTION`]. Idempotent: the seed is unique per round, so a re-report
    /// (peer cert after self-assembly, or replay) writes the same value. Fires
    /// the `notify` permit unconditionally (even on an idempotent re-record —
    /// harmless, the executor arm's eager derive is idempotent) so a held tip
    /// waiting on a late seed record is woken.
    pub fn record(&self, round: Round, seed: BlsSignature) {
        let Ok(mut map) = self.seeds.lock() else {
            // A poisoned lock means a prior panic while holding it — the seed gate
            // can no longer function; log once rather than propagate a panic into
            // the reporter hot path.
            warn!("beacon certify seed store poisoned; dropping recorded seed");
            return;
        };
        let fresh = map.insert(round, seed).is_none();
        while map.len() > SEED_RETENTION {
            // Evict the oldest (lowest-round) entry. `BTreeMap` orders by `Round`, so
            // `pop_first` is the lowest round (matches the `outer.rs` eviction idiom).
            map.pop_first();
        }
        drop(map);
        // Wake the executor's seed-notify arm (a HELD tip whose own round's seed
        // just landed). `notify_one` stores a permit if no waiter is parked, so
        // the wakeup survives a record that races the executor's miss→await.
        self.notify.notify_one();
        // Durable half, strictly AFTER the notify so the wakeup latency is
        // unchanged, and strictly non-blocking so the reporter never parks.
        // Only on a fresh insert: a re-report writes the same bytes (the seed is
        // unique per round), so appending again would only grow the journal. The
        // notify above stays unconditional, as its own comment requires.
        if fresh {
            if let Some(tx) = self.persist.as_ref() {
                if tx.send((round, seed)).is_err() {
                    warn!("seed journal writer is gone; seed recorded in memory only");
                }
            }
        }
    }

    /// The recovered seed for `round`, if present. `pub`: read by the certify
    /// gate here and by the parent-seed witness consumers (the propose-side
    /// embed and the executor's speculative re-canonicalisation).
    pub fn lookup(&self, round: Round) -> Option<BlsSignature> {
        self.seeds.lock().ok()?.get(&round).copied()
    }

    /// A clone of the record-notifier, for the executor's `select!` arm.
    /// `notified()` on the returned handle consumes any permit stored by a
    /// [`record`](Self::record) that fired before the waiter parked.
    pub fn notifier(&self) -> Arc<Notify> {
        self.notify.clone()
    }
}

impl Default for SeedStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_consensus::types::{Epoch as TEpoch, View};
    use commonware_cryptography::{
        bls12381::{
            dkg::deal,
            primitives::{group::Share, sharing::Mode, variant::MinSig},
        },
        ed25519::PrivateKey as Ed25519PrivateKey,
        Signer as _,
    };
    use commonware_math::algebra::Random as _;
    use commonware_utils::{ordered::Set, N3f1};
    use fluentbase_bls::{
        beacon::{recover_seed, seed_namespace, sign_seed_partial},
        fluent_namespace, PeerPubkey,
    };
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    fn round_at(view: u64) -> Round {
        Round::new(TEpoch::new(1), View::new(view))
    }

    /// Deal a real `n`-party committee DKG with a fixed RNG seed; the recovered
    /// seed a store entry carries has to be a genuine threshold signature, not
    /// arbitrary bytes (a `BlsSignature` decode enforces a valid curve point).
    fn deal_committee(seed: u64, n: u32) -> (crate::beacon::outcome::DkgOutcome, Vec<Share>) {
        let mut rng = StdRng::seed_from_u64(seed);
        let players: Set<PeerPubkey> =
            Set::from_iter_dedup((0..n).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()));
        let (outcome, share_map) =
            deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
                .expect("deal");
        let shares: Vec<Share> = share_map.values().to_vec();
        (outcome, shares)
    }

    /// Recover the seed using the outcome's own public sharing.
    fn recover_seed_for(
        outcome: &crate::beacon::outcome::DkgOutcome,
        shares: &[Share],
        ns: &[u8],
        round: Round,
    ) -> BlsSignature {
        let partials: Vec<_> = shares
            .iter()
            .map(|s| sign_seed_partial(s, ns, round))
            .collect();
        recover_seed::<N3f1>(outcome.public(), &partials).expect("recover")
    }

    #[test]
    fn record_seed_is_bounded_and_idempotent() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let store = SeedStore::new();
        for v in 0..(SEED_RETENTION as u64 + 50) {
            let r = round_at(v);
            store.record(r, recover_seed_for(&outcome, &shares, &ns, r));
        }
        // Idempotent re-insert (same unique seed) does not grow the map.
        let r0 = round_at(SEED_RETENTION as u64 + 49);
        store.record(r0, recover_seed_for(&outcome, &shares, &ns, r0));

        let map = store.seeds.lock().unwrap();
        assert_eq!(map.len(), SEED_RETENTION, "store is bounded");
        assert!(map.contains_key(&r0), "newest retained");
        assert!(!map.contains_key(&round_at(0)), "oldest evicted");
    }

    // LOST-WAKEUP ABSENCE (the Notify arm's correctness — the awaitable seed
    // lookup that REPLACES the `SpecNotarized` Poke, family2_finalized_tier.md
    // §2.2). Two records, two waiter orderings:
    //   (1) record BEFORE the waiter is created → `notify_one` stores a permit →
    //       a `notified()` created afterwards is IMMEDIATELY ready. This is the
    //       load-bearing case: it closes the window between the executor's eager
    //       MISS lookup and its next await — a seed record landing there is not
    //       lost (the old Poke depended on the `SpecNotarized` mailbox ordering;
    //       the permit makes the arm correct regardless of ordering).
    //   (2) waiter parked BEFORE the record → woken by it.
    #[test]
    fn seed_store_record_notifies_without_a_lost_wakeup() {
        use std::future::Future;
        use std::task::{Context, Poll};

        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let store = SeedStore::new();
        let notifier = store.notifier();
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        // (1) A permit stored by a record with no parked waiter is consumed by a
        // waiter created AFTER the record — the no-lost-notification window.
        let r0 = round_at(0);
        store.record(r0, recover_seed_for(&outcome, &shares, &ns, r0));
        let f0 = notifier.notified();
        futures::pin_mut!(f0);
        assert!(
            matches!(f0.as_mut().poll(&mut cx), Poll::Ready(())),
            "a stored permit is consumed by a later-created waiter (no lost wakeup)"
        );

        // (2) A waiter parked before the next record is woken by it.
        let f1 = notifier.notified();
        futures::pin_mut!(f1);
        assert!(
            f1.as_mut().poll(&mut cx).is_pending(),
            "no permit yet ⇒ the fresh waiter parks"
        );
        let r1 = round_at(1);
        store.record(r1, recover_seed_for(&outcome, &shares, &ns, r1));
        assert!(
            matches!(f1.as_mut().poll(&mut cx), Poll::Ready(())),
            "the parked waiter is woken by the record"
        );
    }

    // The same two waiter orderings against a PERSISTING store. The durable sink
    // sits after `notify_one` in `record`, so it must not change either verdict —
    // if it ever did, the executor's eager-derive arm would silently lose the
    // record-vs-delivery race that this permit closes.
    #[test]
    fn a_persisting_store_still_notifies_without_a_lost_wakeup() {
        use std::future::Future;
        use std::task::{Context, Poll};

        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let store = SeedStore::with_persistence(Vec::new(), tx);
        let notifier = store.notifier();
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        let r0 = round_at(0);
        store.record(r0, recover_seed_for(&outcome, &shares, &ns, r0));
        let f0 = notifier.notified();
        futures::pin_mut!(f0);
        assert!(
            matches!(f0.as_mut().poll(&mut cx), Poll::Ready(())),
            "the permit still survives a record with no parked waiter"
        );

        let f1 = notifier.notified();
        futures::pin_mut!(f1);
        assert!(f1.as_mut().poll(&mut cx).is_pending());
        let r1 = round_at(1);
        store.record(r1, recover_seed_for(&outcome, &shares, &ns, r1));
        assert!(
            matches!(f1.as_mut().poll(&mut cx), Poll::Ready(())),
            "the parked waiter is still woken by a persisting record"
        );

        assert_eq!(rx.try_recv().map(|(r, _)| r), Ok(r0));
        assert_eq!(rx.try_recv().map(|(r, _)| r), Ok(r1));
        assert!(
            rx.try_recv().is_err(),
            "exactly one queued write per record"
        );
    }

    // Rehydrated entries came OUT of the journal; writing them back would double
    // the journal on every restart. Only a genuinely new round is queued, and a
    // re-report of a round already held is not queued at all (the seed is unique
    // per round, so the bytes would be identical).
    #[test]
    fn rehydrated_entries_are_not_written_back_to_the_journal() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let r0 = round_at(0);
        let seed0 = recover_seed_for(&outcome, &shares, &ns, r0);

        let (tx, mut rx) = mpsc::unbounded_channel();
        let store = SeedStore::with_persistence(vec![(r0, seed0)], tx);
        assert_eq!(
            store.lookup(r0),
            Some(seed0),
            "rehydrated round is readable"
        );
        assert!(
            rx.try_recv().is_err(),
            "construction from the journal queues no writes"
        );

        store.record(r0, seed0);
        assert!(
            rx.try_recv().is_err(),
            "re-recording an already-held round queues no write"
        );

        let r1 = round_at(1);
        store.record(r1, recover_seed_for(&outcome, &shares, &ns, r1));
        assert_eq!(
            rx.try_recv().map(|(r, _)| r),
            Ok(r1),
            "a genuinely new round IS queued"
        );
    }

    // The rehydrated map obeys the same bound as `record`'s eviction path, so a
    // journal window larger than the store's cannot grow it unbounded.
    #[test]
    fn rehydration_is_capped_at_the_retention_bound() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let over = SEED_RETENTION as u64 + 25;
        let rehydrated: Vec<_> = (0..over)
            .map(|v| {
                let r = round_at(v);
                (r, recover_seed_for(&outcome, &shares, &ns, r))
            })
            .collect();

        let (tx, _rx) = mpsc::unbounded_channel();
        let store = SeedStore::with_persistence(rehydrated, tx);
        assert_eq!(store.seeds.lock().unwrap().len(), SEED_RETENTION);
        assert_eq!(store.lookup(round_at(0)), None, "oldest dropped");
        assert!(store.lookup(round_at(over - 1)).is_some(), "newest kept");
    }
}
