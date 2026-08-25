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

use crate::beacon::verified_seed::VerifiedSeed;
use commonware_consensus::types::Round;
use fluentbase_bls::{
    oracle::{SeedCheck, SeedOracle},
    BlsSignature,
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};
use tokio::sync::{mpsc, oneshot, Notify};
use tracing::{error, warn};

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
/// because `seed_for` is a synchronous trait method.
///
/// The served map holds only σ that verified against `PK_e`: every insertion
/// path takes a [`VerifiedSeed`]. A σ this node received but could not yet check
/// lives in the separate [`quarantine`](SeedStore::quarantine) map, which
/// [`lookup`](SeedStore::lookup) never reads — a flag on a shared map would let
/// an unchecked value overwrite a checked one, since insertion is last-wins.
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
    /// σ received from the network for an epoch whose key is not resolvable
    /// here yet. Never read by [`lookup`](SeedStore::lookup) and never sent to
    /// the durable half — losing it on restart is correct, the node re-asks.
    quarantined: Arc<Mutex<BTreeMap<Round, BlsSignature>>>,
    /// Per-ROUND wakeups for the by-round pull.
    ///
    /// Deliberately NOT the `notify` above: that one is `notify_one` with a
    /// documented waiter population of ONE (the executor's held-tip arm), and a
    /// second consumer of it would both end its own wait on an unrelated round's
    /// record and swallow the executor's wake. A oneshot per waiter ends only on
    /// the round it asked about.
    waiters: Arc<Mutex<HashMap<Round, Vec<oneshot::Sender<()>>>>>,
}

impl SeedStore {
    /// Construct an empty, RAM-only store.
    pub fn new() -> Self {
        Self {
            seeds: Arc::new(Mutex::new(BTreeMap::new())),
            notify: Arc::new(Notify::new()),
            persist: None,
            quarantined: Arc::new(Mutex::new(BTreeMap::new())),
            waiters: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Construct a store backed by the durable journal, pre-loaded with the
    /// window replayed from it.
    ///
    /// The channel is created HERE and its receiving half handed back, rather
    /// than taken as an argument: a sender that only ever exists inside a
    /// `SeedStore` cannot be used to write an unchecked σ to disk, and that is a
    /// property of the type rather than of the current call sites. See
    /// [`VerifiedSeed::from_journal`] for the other two feeders of the same
    /// partition, which the same argument has to name.
    ///
    /// `rehydrated` entries bypass the durable send — they came OUT of the
    /// journal and must not be written back — but they go through the same
    /// insert as [`record`](Self::record), so the served map still holds nothing
    /// but witnessed values. Truncating here keeps [`SEED_RETENTION`] the bound
    /// on this route too.
    pub fn with_persistence(
        rehydrated: Vec<(Round, BlsSignature)>,
    ) -> (Self, mpsc::UnboundedReceiver<(Round, BlsSignature)>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let store = Self {
            seeds: Arc::new(Mutex::new(BTreeMap::new())),
            notify: Arc::new(Notify::new()),
            persist: Some(tx),
            quarantined: Arc::new(Mutex::new(BTreeMap::new())),
            waiters: Arc::new(Mutex::new(HashMap::new())),
        };
        for (round, seed) in rehydrated {
            store.insert(VerifiedSeed::from_journal(round, seed), false);
        }
        (store, rx)
    }

    /// Record a witnessed seed, evicting the oldest entries past
    /// [`SEED_RETENTION`]. Idempotent: σ is unique per round, so a re-report
    /// (peer cert after self-assembly, or replay) writes the same value. Fires
    /// the `notify` permit unconditionally (even on an idempotent re-record —
    /// harmless, the executor arm's eager derive is idempotent) so a held tip
    /// waiting on a late seed record is woken.
    pub fn record(&self, verified: VerifiedSeed) {
        self.insert(verified, true);
    }

    /// Shared body of the two insertion paths. `persist` is false for entries
    /// replayed from the journal, which must not be written back to it.
    ///
    /// Two witnessed values differing under one round would mean σ is not unique
    /// per round — the argument every consumer of this map rests on, including
    /// the fork-safety of keying an ingress-captured σ by the certificate's own
    /// round (§13 rule 28). Refuse the overwrite and be loud rather than let the
    /// served map hold a value the assembler and the witness disagree about.
    fn insert(&self, verified: VerifiedSeed, persist: bool) {
        let (round, seed) = (verified.round(), verified.seed());
        let Ok(mut map) = self.seeds.lock() else {
            // A poisoned lock means a prior panic while holding it — the seed gate
            // can no longer function; log once rather than propagate a panic into
            // the reporter hot path.
            warn!("beacon certify seed store poisoned; dropping recorded seed");
            return;
        };
        if let Some(held) = map.get(&round) {
            if *held != seed {
                error!(
                    ?round,
                    "two verified seeds differ for one round; keeping the first"
                );
                return;
            }
        }
        let fresh = map.insert(round, seed).is_none();
        while map.len() > SEED_RETENTION {
            // Evict the oldest (lowest-round) entry. `BTreeMap` orders by `Round`, so
            // `pop_first` is the lowest round (matches the `outer.rs` eviction idiom).
            map.pop_first();
        }
        drop(map);
        // Per-round waiters first: the pull that asked for exactly this round is
        // released here, and nowhere else.
        if let Ok(mut waiting) = self.waiters.lock() {
            for tx in waiting.remove(&round).unwrap_or_default() {
                // A dropped receiver (the pull timed out first) is the ordinary
                // case, not an error: it re-checks the map on its own.
                let _ = tx.send(());
            }
        }
        // Wake the executor's seed-notify arm (a HELD tip whose own round's seed
        // just landed). `notify_one` stores a permit if no waiter is parked, so
        // the wakeup survives a record that races the executor's miss→await.
        self.notify.notify_one();
        // Durable half, strictly AFTER the notify so the wakeup latency is
        // unchanged, and strictly non-blocking so the reporter never parks.
        // Only on a fresh insert: a re-report writes the same bytes (the seed is
        // unique per round), so appending again would only grow the journal. The
        // notify above stays unconditional, as its own comment requires.
        if fresh && persist {
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

    /// Hold a σ this node cannot check yet, for the epoch key to settle later.
    ///
    /// Kept out of the served map on purpose: [`record`](Self::record) is
    /// last-wins, so a shared map with a provenance flag would leave an
    /// unverified value structurally able to overwrite a verified one.
    pub fn quarantine(&self, round: Round, seed: BlsSignature) {
        let Ok(mut map) = self.quarantined.lock() else {
            warn!("beacon seed quarantine poisoned; dropping unverified seed");
            return;
        };
        map.insert(round, seed);
        // Bounded by COUNT as well as by epoch: the epoch window is the rule that
        // matters — resolution arrives per epoch — but it only prunes on an epoch
        // edge, and rounds accrue between two edges.
        //
        // The oldest goes, exactly as in the served map, and here that is not
        // merely symmetric — it is what the map exists for. The entry a boundary
        // asks for is the TERMINAL round of the epoch being followed, i.e. the
        // HIGHEST one held, so the newest end is the end to keep.
        //
        // That reading is only safe because no caller can file a round of its own
        // choosing. All three writers key on a round that is already attested: the
        // two ingresses capture only after the certificate verified against
        // `committee[epoch]`, and the transport files under the round THIS node
        // asked for. A future writer that takes a round from an unverified source
        // would break the reasoning above, not just this bound.
        while map.len() > SEED_RETENTION {
            map.pop_first();
        }
    }

    /// Re-check every quarantined round of `epoch` now that its key may have
    /// landed. Returns `(promoted, refused)`.
    ///
    /// A σ that now fails is not left behind to be re-checked forever: it is
    /// dropped, so the round can be asked for again. Keeping it would let a peer
    /// park garbage under exactly the round a boundary needs and have it answer
    /// every later re-check.
    pub fn promote_epoch(&self, epoch: u64, oracle: &dyn SeedOracle) -> (usize, usize) {
        let candidates: Vec<(Round, BlsSignature)> = {
            let Ok(held) = self.quarantined.lock() else {
                warn!("beacon seed quarantine poisoned; cannot promote");
                return (0, 0);
            };
            held.iter()
                .filter(|(round, _)| round.epoch().get() == epoch)
                .map(|(round, seed)| (*round, *seed))
                .collect()
        };
        // The quarantine lock is released before `record` takes the served-map
        // lock: the two are never held together, in either order.
        let (mut promoted, mut refused) = (0, 0);
        let mut settled = Vec::new();
        for (round, seed) in candidates {
            match VerifiedSeed::check(oracle, round, seed) {
                Ok(verified) => {
                    self.record(verified);
                    settled.push(round);
                    promoted += 1;
                }
                Err(SeedCheck::Invalid) => {
                    settled.push(round);
                    refused += 1;
                    error!(?round, "quarantined seed does not verify under its epoch key");
                }
                Err(_) => {}
            }
        }
        if let Ok(mut held) = self.quarantined.lock() {
            for round in settled {
                held.remove(&round);
            }
        }
        (promoted, refused)
    }

    /// The epochs holding at least one quarantined round, for the promoter to
    /// re-ask about when a key lands.
    pub fn quarantined_epochs(&self) -> Vec<u64> {
        let Ok(held) = self.quarantined.lock() else {
            return Vec::new();
        };
        let mut epochs: Vec<u64> = held.keys().map(|round| round.epoch().get()).collect();
        epochs.dedup();
        epochs
    }

    /// Drop quarantined rounds belonging to epochs below `oldest`. Accounting
    /// and eviction are per epoch because resolution is: one `PK_e` settles
    /// every round of its epoch at once, so evicting by round would tear an
    /// epoch in half and leave a remainder that can never resolve meaningfully.
    pub fn retain_quarantine_from(&self, oldest: u64) {
        let Ok(mut held) = self.quarantined.lock() else {
            return;
        };
        held.retain(|round, _| round.epoch().get() >= oldest);
    }

    /// Drop waiters whose receiver is gone.
    ///
    /// A pull that timed out leaves its sender behind, and the round it asked
    /// about is by definition one nothing recorded — so without this the entry
    /// grows one closed sender per attempt, forever, exactly while the node is
    /// already in an incident. The artifact pull prunes its own waiter map for
    /// the same reason.
    fn prune_waiters(&self) {
        if let Ok(mut waiting) = self.waiters.lock() {
            waiting.retain(|_, senders| {
                senders.retain(|tx| !tx.is_closed());
                !senders.is_empty()
            });
        }
    }

    /// A wakeup that fires when `round` — and only `round` — is recorded.
    ///
    /// The receiver errors if the store is dropped, which the caller reads the
    /// same way as a timeout: it re-checks the map and answers from that.
    pub fn wait_for(&self, round: Round) -> oneshot::Receiver<()> {
        self.prune_waiters();
        let (tx, rx) = oneshot::channel();
        match self.waiters.lock() {
            Ok(mut waiting) => waiting.entry(round).or_default().push(tx),
            // A poisoned lock cannot wake anyone; drop the sender so the caller
            // falls through to its own re-check rather than waiting out its
            // whole budget.
            Err(_) => drop(tx),
        }
        rx
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
    use crate::beacon::verified_seed::PkOracle;
    use crate::beacon::verified_seed::VerifiedSeed;
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

    /// The same σ behind the witness the store takes. The outcome's own public
    /// sharing is the key it was dealt under, so the check is real.
    fn witness_for(
        outcome: &crate::beacon::outcome::DkgOutcome,
        shares: &[Share],
        ns: &[u8],
        round: Round,
    ) -> VerifiedSeed {
        PkOracle::new(*outcome.public().public(), ns.to_vec())
            .witness(round, recover_seed_for(outcome, shares, ns, round))
    }

    #[test]
    fn record_seed_is_bounded_and_idempotent() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let store = SeedStore::new();
        for v in 0..(SEED_RETENTION as u64 + 50) {
            let r = round_at(v);
            store.record(witness_for(&outcome, &shares, &ns, r));
        }
        // Idempotent re-insert (same unique seed) does not grow the map.
        let r0 = round_at(SEED_RETENTION as u64 + 49);
        store.record(witness_for(&outcome, &shares, &ns, r0));

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
        store.record(witness_for(&outcome, &shares, &ns, r0));
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
        store.record(witness_for(&outcome, &shares, &ns, r1));
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
        let (store, mut rx) = SeedStore::with_persistence(Vec::new());
        let notifier = store.notifier();
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        let r0 = round_at(0);
        store.record(witness_for(&outcome, &shares, &ns, r0));
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
        store.record(witness_for(&outcome, &shares, &ns, r1));
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

    // A σ the node cannot check yet is HELD, not served: `lookup` is the read
    // every consumer of `prev_randao` ends up behind, and a value that reached it
    // unchecked is a fork, not a miss.
    #[test]
    fn quarantined_seeds_are_never_served_and_promote_when_the_key_lands() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let store = SeedStore::new();
        let r = round_at(7);
        store.quarantine(r, recover_seed_for(&outcome, &shares, &ns, r));
        assert_eq!(store.lookup(r), None, "quarantine is not the served map");
        assert_eq!(store.quarantined_epochs(), vec![1]);

        let oracle = PkOracle::new(*outcome.public().public(), ns.clone());
        assert_eq!(store.promote_epoch(1, &oracle), (1, 0));
        assert!(store.lookup(r).is_some(), "a checked seed is served");
        assert!(
            store.quarantined_epochs().is_empty(),
            "a promoted round leaves the quarantine"
        );
    }

    // A valid multisig admits a certificate whose seed slot was never checked, so
    // a peer can park bytes under exactly the round a boundary will ask for. The
    // re-check must DROP them, not keep answering itself with them for ever.
    #[test]
    fn a_quarantined_seed_that_fails_its_key_is_dropped_not_retained() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let store = SeedStore::new();
        let wanted = round_at(7);
        // A genuine σ of a DIFFERENT round: a decodable curve point that verifies
        // under no key for `wanted`.
        store.quarantine(wanted, recover_seed_for(&outcome, &shares, &ns, round_at(8)));

        let oracle = PkOracle::new(*outcome.public().public(), ns.clone());
        assert_eq!(store.promote_epoch(1, &oracle), (0, 1));
        assert_eq!(store.lookup(wanted), None, "a refused seed is not served");
        assert!(
            store.quarantined_epochs().is_empty(),
            "a refused seed is evicted so the round can be asked for again"
        );
    }

    // Resolution arrives per epoch — one `PK_e` settles every round of its epoch —
    // so eviction is per epoch too. Past the key store's retention edge no key can
    // arrive any more and a held σ is only memory a peer could grow.
    #[test]
    fn quarantine_eviction_is_per_epoch() {
        let store = SeedStore::new();
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let old = Round::new(TEpoch::new(1), View::new(1));
        let new = Round::new(TEpoch::new(9), View::new(1));
        store.quarantine(old, recover_seed_for(&outcome, &shares, &ns, old));
        store.quarantine(new, recover_seed_for(&outcome, &shares, &ns, new));
        store.retain_quarantine_from(9);
        assert_eq!(store.quarantined_epochs(), vec![9]);
    }

    // σ is unique per round — the argument the witness, the executor and the
    // ingress capture all rest on. Two differing values under one round mean that
    // argument broke somewhere, and the served map must not silently take the
    // second: `from_journal` is the only way to stage the impossible pair.
    #[test]
    fn a_second_differing_seed_for_one_round_is_refused() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let store = SeedStore::new();
        let r = round_at(3);
        let first = recover_seed_for(&outcome, &shares, &ns, r);
        let other = recover_seed_for(&outcome, &shares, &ns, round_at(4));
        store.record(VerifiedSeed::from_journal(r, first));
        store.record(VerifiedSeed::from_journal(r, other));
        assert_eq!(store.lookup(r), Some(first), "the first value stands");
    }

    // The pull's wakeup must end on ITS round and no other. The store's `notify`
    // is `notify_one` with a documented waiter population of one (the executor's
    // held-tip arm); a pull sharing it would be released by any unrelated record —
    // usually instantly, since a permit is stored even with nobody parked — and
    // would answer "not arrived" after a wait that never happened.
    #[test]
    fn a_round_waiter_is_woken_by_its_own_round_and_not_by_another() {
        use std::{future::Future, task::{Context, Poll}};
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let store = SeedStore::new();
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        let wanted = round_at(7);
        let mut waiting = store.wait_for(wanted);
        assert!(std::pin::Pin::new(&mut waiting).poll(&mut cx).is_pending());

        store.record(witness_for(&outcome, &shares, &ns, round_at(8)));
        assert!(
            std::pin::Pin::new(&mut waiting).poll(&mut cx).is_pending(),
            "another round's record must not release this waiter"
        );

        store.record(witness_for(&outcome, &shares, &ns, wanted));
        assert!(matches!(
            std::pin::Pin::new(&mut waiting).poll(&mut cx),
            Poll::Ready(Ok(()))
        ));
    }

    // Following ONE epoch: the round a boundary will ask for is that epoch's
    // TERMINAL round, i.e. the newest thing held, so the bound must drop the
    // oldest.
    #[test]
    fn within_one_epoch_the_quarantine_bound_keeps_the_terminal_round() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let store = SeedStore::new();
        let over = SEED_RETENTION as u64 + 50;
        for v in 0..over {
            let r = round_at(v);
            store.quarantine(r, recover_seed_for(&outcome, &shares, &ns, r));
        }
        let held = store.quarantined.lock().unwrap();
        assert_eq!(held.len(), SEED_RETENTION, "bounded");
        assert!(
            held.contains_key(&round_at(over - 1)),
            "the newest round of the epoch being followed survives"
        );
        assert!(!held.contains_key(&round_at(0)), "the oldest is what goes");
    }

    // Across epochs the same rule holds and for the same reason: what a boundary
    // asks for is the newest thing held, whichever epoch it belongs to.
    #[test]
    fn the_quarantine_bound_keeps_the_newest_rounds_across_epochs() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let store = SeedStore::new();
        let stale = Round::new(TEpoch::new(1), View::new(9));
        store.quarantine(stale, recover_seed_for(&outcome, &shares, &ns, stale));
        let newest = Round::new(TEpoch::new(2), View::new(SEED_RETENTION as u64 + 49));
        for v in 0..(SEED_RETENTION as u64 + 50) {
            let r = Round::new(TEpoch::new(2), View::new(v));
            store.quarantine(r, recover_seed_for(&outcome, &shares, &ns, r));
        }
        let held = store.quarantined.lock().unwrap();
        assert_eq!(held.len(), SEED_RETENTION, "bounded");
        assert!(held.contains_key(&newest), "the newest round survives");
        assert!(!held.contains_key(&stale), "the stale epoch's round is what goes");
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

        let (store, mut rx) = SeedStore::with_persistence(vec![(r0, seed0)]);
        assert_eq!(
            store.lookup(r0),
            Some(seed0),
            "rehydrated round is readable"
        );
        assert!(
            rx.try_recv().is_err(),
            "construction from the journal queues no writes"
        );

        store.record(witness_for(&outcome, &shares, &ns, r0));
        assert!(
            rx.try_recv().is_err(),
            "re-recording an already-held round queues no write"
        );

        let r1 = round_at(1);
        store.record(witness_for(&outcome, &shares, &ns, r1));
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

        let (store, _rx) = SeedStore::with_persistence(rehydrated);
        assert_eq!(store.seeds.lock().unwrap().len(), SEED_RETENTION);
        assert_eq!(store.lookup(round_at(0)), None, "oldest dropped");
        assert!(store.lookup(round_at(over - 1)).is_some(), "newest kept");
    }
}
