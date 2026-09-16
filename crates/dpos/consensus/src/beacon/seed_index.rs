//! The `round -> σ` index: the one owner of the seed fact.
//!
//! Every σ reaches [`SeedIndex`] through one insertion path
//! ([`crate::beacon::Beacon::observe_certificate`]) and is read synchronously by
//! the executor's finalized derive and the epoch manager's boundary base.
//!
//! An entry is either [`Entry::Verified`] (checked under the epoch's `PK_E`, so
//! servable) or [`Entry::Pending`] (arrived for an epoch whose key is not
//! resolvable here yet). The two live in one map as states: a `Pending` admission
//! against a `Verified` entry is dropped, and only the `Verified` arm is served.
//!
//! Retention distinguishes the states three ways:
//!
//! 1. [`SEED_RETENTION`] bounds each state, not the sum, so a `Pending` flood can
//!    never evict a `Verified` σ the executor has yet to consume.
//! 2. Eviction skips the highest verified round of each epoch inside the trailing
//!    [`crate::SCHEME_RETENTION_EPOCHS`] window: a Signer starting mid-epoch
//!    elects on σ of `E-1`'s terminal round, and `SEED_RETENTION` is a global
//!    round count that would otherwise have dropped it. The protection is
//!    `Verified`-only — reading it off bare map keys would let a `Pending` round
//!    above the real terminal take it and the checked terminal be evicted under
//!    it, which sends `boundary_base` into an open-ended `Missing`.
//! 3. Past the [`crate::SCHEME_RETENTION_EPOCHS`] edge no key can arrive, so a
//!    held σ can never settle and is only memory a peer can grow. The sweep
//!    retires such an epoch as a unit: one `PK_e` settles every round of its epoch
//!    at once, so retiring by round would tear an epoch in half.
//!
//! Reads never touch disk: [`crate::beacon::seed_journal`] is the durable mirror
//! joined at startup by [`SeedIndex::with_persistence`]. The journal exists for
//! restart only — reading it is async, while [`crate::beacon::Beacon::seed`] is
//! synchronous on every caller.

use crate::beacon::{
    surface::{BeaconEvent, EVENT_BUFFER},
    verified_seed::VerifiedSeed,
};
use commonware_consensus::types::Round;
use fluentbase_bls::{
    oracle::{SeedCheck, SeedOracle},
    BlsSignature,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::sync::{broadcast, mpsc};
use tracing::{error, warn};

/// Bound on retained `round → σ` entries, per state. A σ is 48 B, and a round
/// notarized while this node lags a long block at a boundary can stay wanted for
/// many later notarizations, so the window is sized well past any realistic
/// in-flight backlog.
pub(crate) const SEED_RETENTION: usize = 4096;

/// One round's σ, and what is known about it.
///
/// `Pending` is never served: a σ that reached
/// [`crate::beacon::prev_randao_from_seed`] unchecked is a fork, not a miss.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Entry {
    /// Checked under the epoch's `PK_E`.
    Verified(BlsSignature),
    /// Received for an epoch whose key is not resolvable here yet; settled by
    /// [`SeedIndex::settle_epoch`] when the key lands and never written to the
    /// durable half, so losing it on restart is correct.
    Pending(BlsSignature),
}

/// The `round → σ` index. Cheap to clone (one `Arc` per field); every clone is
/// the same index.
#[derive(Clone)]
pub struct SeedIndex {
    entries: Arc<Mutex<BTreeMap<Round, Entry>>>,
    /// Durable sink; `None` is RAM-only, which is what every test and the
    /// `--cert-follow` follower get.
    ///
    /// The unbounded send is synchronous and never blocks, which is what lets the
    /// durable write sit inside [`SeedIndex::record`] without putting the reporter
    /// behind an await — a bounded channel would.
    persist: Option<mpsc::UnboundedSender<(Round, BlsSignature)>>,
    /// The wake-up publisher, fired on every verified admission from every writer:
    /// the late settle files through this same door, so a wake-up owned by one
    /// caller would leave a settled σ un-woken.
    events: broadcast::Sender<BeaconEvent>,
}

impl SeedIndex {
    /// Construct an empty, RAM-only index.
    pub fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(BTreeMap::new())),
            events: broadcast::channel(EVENT_BUFFER).0,
            persist: None,
        }
    }

    /// Construct an index backed by the durable journal, pre-loaded with what was
    /// replayed from it.
    ///
    /// The channel is created here so a sender can only ever exist inside a
    /// `SeedIndex`; that is what keeps an unchecked σ off disk by construction.
    ///
    /// Both arguments bypass the durable send (they came out of the journal and must
    /// not be written back) and go through the same `admit` as `record`, so
    /// [`SEED_RETENTION`] bounds this route too.
    ///
    /// `terminals` is separate because `replay_window` stops at `retention` records
    /// and can miss the previous epoch's terminal round — the one round a restarting
    /// Signer needs. The eviction rule protects the highest round of each retained
    /// epoch, so it need not be filed anywhere special.
    pub fn with_persistence(
        rehydrated: Vec<(Round, BlsSignature)>,
        terminals: Vec<(Round, BlsSignature)>,
    ) -> (Self, mpsc::UnboundedReceiver<(Round, BlsSignature)>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let index = Self {
            entries: Arc::new(Mutex::new(BTreeMap::new())),
            events: broadcast::channel(EVENT_BUFFER).0,
            persist: Some(tx),
        };
        // The terminals go in first so the window's own eviction sees them and
        // protects them; inserting them after a full window would make the
        // protection depend on insertion order.
        for (round, seed) in terminals.into_iter().chain(rehydrated) {
            let witness = VerifiedSeed::from_journal(round, seed);
            index.admit((witness.round(), witness.seed()), true, false);
        }
        (index, rx)
    }

    /// File a σ that verified under its epoch key.
    ///
    /// Idempotent because σ is unique per round; the wake-up is published even on an
    /// idempotent re-record so a held tip waiting on a late record is woken.
    pub fn record(&self, verified: VerifiedSeed) {
        self.admit((verified.round(), verified.seed()), true, true);
    }

    /// Hold a σ this node cannot check yet, for the epoch key to settle later.
    pub fn hold(&self, round: Round, seed: BlsSignature) {
        self.admit((round, seed), false, false);
    }

    /// The one insertion path. `persist` is false for entries replayed from the
    /// journal, which must not be written back.
    ///
    /// Two differing verified values under one round would mean σ is not unique per
    /// round — the argument every consumer of this index rests on — so the overwrite
    /// is refused loudly rather than serving a value the assembler and the witness
    /// disagree about.
    fn admit(&self, (round, seed): (Round, BlsSignature), verified: bool, persist: bool) {
        let mut entries = self.lock();
        let previous = entries.get(&round).copied();
        match (previous, verified) {
            (Some(Entry::Verified(held)), true) if held != seed => {
                error!(
                    ?round,
                    "two verified seeds differ for one round; keeping the first"
                );
                return;
            }
            // A checked value already stands: an unchecked one says nothing new about
            // this round, and taking it would be a last-wins overwrite.
            (Some(Entry::Verified(_)), false) => return,
            // `Pending` to `Pending` is last-wins: neither value can be adjudicated
            // without the key, and the one that fails the settle is dropped rather than
            // kept, so the round becomes askable again either way.
            _ => {}
        }
        // A fresh durable value is one whose round did not already hold a checked σ:
        // promoting a `Pending` to `Verified` is the first time that round is worth
        // writing down.
        let fresh = !matches!(previous, Some(Entry::Verified(_)));
        entries.insert(
            round,
            if verified {
                Entry::Verified(seed)
            } else {
                Entry::Pending(seed)
            },
        );
        evict(&mut entries);
        drop(entries);
        if !verified {
            // Nothing to wake and nothing to write: a held σ is not servable and the
            // journal holds checked values only.
            return;
        }
        // Wake the executor's held-tip arm. The broadcast buffers from each consumer's
        // subscription, which is why every consumer subscribes before its first read.
        let _ = self.events.send(BeaconEvent::SeedRecorded);
        // Durable half, after the wake-up so the wake latency is unchanged, and
        // non-blocking so the reporter never parks.
        if fresh && persist {
            if let Some(tx) = self.persist.as_ref() {
                if tx.send((round, seed)).is_err() {
                    warn!("seed journal writer is gone; seed recorded in memory only");
                }
            }
        }
    }

    /// The σ in force at `round`, if this node holds it checked.
    ///
    /// Exact round only: a neighbouring round and a `Pending` entry are both misses.
    /// The terminal-round exemption is an eviction rule, so a caller asking for
    /// `E-1`'s terminal gets σ of exactly the round it named or nothing.
    pub fn seed(&self, round: Round) -> Option<BlsSignature> {
        match self.lock().get(&round)? {
            Entry::Verified(seed) => Some(*seed),
            Entry::Pending(_) => None,
        }
    }

    /// A poisoned lock (a caller panicked while holding it) is recovered, not
    /// propagated: the guarded map is never mutated across more than one statement,
    /// so it cannot be left half-written, and dropping the σ would fail silently
    /// while [`super::surface::certificate_verdict`] still answers `Recorded`.
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<Round, Entry>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The epochs holding at least one `Pending` round, ascending.
    pub fn pending_epochs(&self) -> Vec<u64> {
        let entries = self.lock();
        let mut epochs: Vec<u64> = entries
            .iter()
            .filter(|(_, entry)| matches!(entry, Entry::Pending(_)))
            .map(|(round, _)| round.epoch().get())
            .collect();
        epochs.dedup();
        epochs
    }

    /// Re-check every `Pending` round of `epoch` now that its key has landed.
    /// Returns `(promoted, refused)`.
    ///
    /// A σ that fails is dropped rather than re-checked forever, so the round can be
    /// asked for again; keeping it would let a peer park garbage under exactly the
    /// round a boundary needs.
    pub fn settle_epoch(&self, epoch: u64, oracle: &dyn SeedOracle) -> (usize, usize) {
        let candidates: Vec<(Round, BlsSignature)> = {
            let entries = self.lock();
            entries
                .iter()
                .filter(|(round, entry)| {
                    round.epoch().get() == epoch && matches!(entry, Entry::Pending(_))
                })
                .map(|(round, entry)| {
                    let Entry::Pending(seed) = entry else {
                        unreachable!("filtered to Pending")
                    };
                    (*round, *seed)
                })
                .collect()
        };
        // The lock is released before the BLS check and re-taken to apply: a
        // threshold verification per round is not something to hold the index's
        // one lock across, and the reporter writes through that lock.
        let (mut promoted, mut refused) = (0, 0);
        let mut dropped = Vec::new();
        for (round, seed) in candidates {
            match VerifiedSeed::check(oracle, round, seed) {
                Ok(verified) => {
                    self.record(verified);
                    promoted += 1;
                }
                Err(SeedCheck::Invalid) => {
                    dropped.push((round, seed));
                    refused += 1;
                    error!(?round, "held seed does not verify under its epoch key");
                }
                // Still unresolvable: the key did not land for this epoch, so the
                // entry stays `Pending`.
                Err(_) => {}
            }
        }
        let mut entries = self.lock();
        for (round, seed) in dropped {
            // Remove only what is still the refused value: another door may have filed
            // a checked σ for the round during verification.
            if entries.get(&round) == Some(&Entry::Pending(seed)) {
                entries.remove(&round);
            }
        }
        drop(entries);
        (promoted, refused)
    }

    /// The wake-up publisher every consumer of this index subscribes to.
    pub fn events(&self) -> &broadcast::Sender<BeaconEvent> {
        &self.events
    }

    /// Every round the index holds, with its state, for this module's tests. Every
    /// production caller asks about one round it named.
    #[cfg(test)]
    fn snapshot(&self) -> BTreeMap<Round, Entry> {
        self.lock().clone()
    }
}

/// Apply the retention rule: retire the `Pending` epochs the window has closed on,
/// then bound each state by [`SEED_RETENTION`]. The order matters — a closed
/// epoch's held σ is dead weight, so it goes before anything live makes room.
fn evict(entries: &mut BTreeMap<Round, Entry>) {
    // The window is measured from the index's own highest epoch, so the rule is
    // self-contained: σ arrives per round, and the highest round held is the frontier.
    let Some(top_epoch) = entries.keys().next_back().map(|round| round.epoch().get()) else {
        return;
    };
    let floor = top_epoch.saturating_sub(crate::SCHEME_RETENTION_EPOCHS as u64);
    retire_closed_pending_epochs(entries, floor);
    // Each state has its own budget, so neither can be over budget while the whole
    // index is under one; the `len` read keeps the two counting scans off the
    // admission path.
    if entries.len() <= SEED_RETENTION {
        return;
    }
    bound_pending(entries);
    bound_verified(entries, floor);
}

/// Drop every `Pending` round of an epoch below `floor`. Past the scheme-retention
/// edge the epoch's key can never arrive, so the σ can never settle and is only
/// memory a peer grows.
///
/// The unit is the epoch, not the round: one `PK_e` settles every round of its epoch
/// at once, so retiring by round would tear an epoch in half. Entries are ordered by
/// `(epoch, view)`, so the closed epochs are the prefix below `floor`.
fn retire_closed_pending_epochs(entries: &mut BTreeMap<Round, Entry>, floor: u64) {
    let closed: Vec<Round> = entries
        .iter()
        .take_while(|(round, _)| round.epoch().get() < floor)
        .filter(|(_, entry)| matches!(entry, Entry::Pending(_)))
        .map(|(round, _)| *round)
        .collect();
    for round in closed {
        entries.remove(&round);
    }
}

/// Bound the `Pending` state by [`SEED_RETENTION`] on its own count. Whole epochs go
/// first, oldest first. The last remaining pending epoch is trimmed from its oldest
/// end instead of dropped whole: it is the epoch whose key can still land, and a
/// boundary asks for the highest round held. The cost is that a node keyless for
/// more than `SEED_RETENTION` rounds of one epoch loses that epoch's oldest held σ
/// and must re-ask once the key lands.
fn bound_pending(entries: &mut BTreeMap<Round, Entry>) {
    let mut pending: Vec<Round> = entries
        .iter()
        .filter(|(_, entry)| matches!(entry, Entry::Pending(_)))
        .map(|(round, _)| *round)
        .collect();
    while pending.len() > SEED_RETENTION {
        let oldest_epoch = pending[0].epoch().get();
        let victims = if pending[pending.len() - 1].epoch().get() == oldest_epoch {
            pending.len() - SEED_RETENTION
        } else {
            pending
                .iter()
                .take_while(|round| round.epoch().get() == oldest_epoch)
                .count()
        };
        for round in pending.drain(..victims) {
            entries.remove(&round);
        }
    }
}

/// Bound the `Verified` state by [`SEED_RETENTION`] on its own count, evicting
/// oldest-first and skipping the rounds the terminal rule protects.
fn bound_verified(entries: &mut BTreeMap<Round, Entry>, floor: u64) {
    let mut verified = entries
        .values()
        .filter(|entry| matches!(entry, Entry::Verified(_)))
        .count();
    while verified > SEED_RETENTION {
        let Some(victim) = oldest_evictable(entries, floor) else {
            // Every checked entry left is a protected terminal — at most one per epoch
            // in the trailing window, so this cannot leak. It is reachable only with
            // `SEED_RETENTION` below the window's size.
            break;
        };
        entries.remove(&victim);
        verified -= 1;
    }
}

/// The lowest checked round that may be dropped: the oldest [`Entry::Verified`] that
/// is not the highest verified round of an epoch inside the trailing retention
/// window.
///
/// `Pending` rounds are neither candidates (they have their own bound) nor
/// protection-conferring: reading the protection off bare map keys would let a held
/// σ above the real terminal take it and the checked terminal be evicted, leaving
/// `boundary_base` missing a round nothing else can supply.
///
/// The rule protects the highest *held* round rather than the terminal one: a hard
/// kill between an admission and the journal's sync loses the tail, so the highest
/// round a restarted node holds can sit below the epoch's real last round. Nothing
/// may trust this rule to name the terminal; it decides only what survives, and
/// [`SeedIndex::seed`] answers the round the caller named.
///
/// `None` ⇒ nothing checked is evictable.
fn oldest_evictable(entries: &BTreeMap<Round, Entry>, floor: u64) -> Option<Round> {
    let mut rounds = entries
        .iter()
        .filter(|(_, entry)| matches!(entry, Entry::Verified(_)))
        .map(|(round, _)| *round)
        .peekable();
    while let Some(round) = rounds.next() {
        let epoch = round.epoch().get();
        // Ordered by `(epoch, view)`, so this is the highest verified round of its
        // epoch exactly when the next checked one belongs to another epoch or there is
        // none.
        let highest_of_epoch = rounds.peek().map(|next| next.epoch().get()) != Some(epoch);
        if highest_of_epoch && epoch >= floor {
            continue;
        }
        return Some(round);
    }
    None
}

impl Default for SeedIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beacon::verified_seed::PkOracle;
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
    /// seed an entry carries has to be a genuine threshold signature, not
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

    /// The same σ behind the witness the index takes. The outcome's own public
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
        let index = SeedIndex::new();
        for v in 0..(SEED_RETENTION as u64 + 50) {
            let r = round_at(v);
            index.record(witness_for(&outcome, &shares, &ns, r));
        }
        // Idempotent re-insert (same unique seed) does not grow the map.
        let r0 = round_at(SEED_RETENTION as u64 + 49);
        index.record(witness_for(&outcome, &shares, &ns, r0));

        let held = index.snapshot();
        assert_eq!(held.len(), SEED_RETENTION, "index is bounded");
        assert!(held.contains_key(&r0), "newest retained");
        assert!(index.seed(round_at(0)).is_none(), "oldest evicted");
    }

    // Both receiver orderings must wake without a lost notification: a record with
    // nobody polling is buffered for the next `recv()`, and a receiver parked before
    // the record is woken by it. The receiver is taken before the first record.
    #[test]
    fn seed_index_record_notifies_without_a_lost_wakeup() {
        use std::future::Future;
        use std::task::{Context, Poll};

        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let index = SeedIndex::new();
        let mut rx = index.events().subscribe();
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        // A record that lands with nobody polling is buffered and read by the next
        // `recv()`.
        let r0 = round_at(0);
        index.record(witness_for(&outcome, &shares, &ns, r0));
        {
            let f0 = rx.recv();
            futures::pin_mut!(f0);
            assert!(
                matches!(
                    f0.as_mut().poll(&mut cx),
                    Poll::Ready(Ok(BeaconEvent::SeedRecorded))
                ),
                "a buffered record is read by a later `recv()` (no lost wakeup)"
            );
        }

        // A receiver parked before the next record is woken by it.
        let f1 = rx.recv();
        futures::pin_mut!(f1);
        assert!(
            f1.as_mut().poll(&mut cx).is_pending(),
            "nothing buffered yet ⇒ the fresh receiver parks"
        );
        let r1 = round_at(1);
        index.record(witness_for(&outcome, &shares, &ns, r1));
        assert!(
            matches!(
                f1.as_mut().poll(&mut cx),
                Poll::Ready(Ok(BeaconEvent::SeedRecorded))
            ),
            "the parked receiver is woken by the record"
        );
    }

    // The same two receiver orderings against a persisting index: the durable sink
    // sits after the event send in `admit`, so it must not change either verdict.
    #[test]
    fn a_persisting_index_still_notifies_without_a_lost_wakeup() {
        use std::future::Future;
        use std::task::{Context, Poll};

        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let (index, mut rx) = SeedIndex::with_persistence(Vec::new(), Vec::new());
        let mut events = index.events().subscribe();
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        let r0 = round_at(0);
        index.record(witness_for(&outcome, &shares, &ns, r0));
        {
            let f0 = events.recv();
            futures::pin_mut!(f0);
            assert!(
                matches!(
                    f0.as_mut().poll(&mut cx),
                    Poll::Ready(Ok(BeaconEvent::SeedRecorded))
                ),
                "the buffered event still survives a record with nobody polling"
            );
        }

        let f1 = events.recv();
        futures::pin_mut!(f1);
        assert!(f1.as_mut().poll(&mut cx).is_pending());
        let r1 = round_at(1);
        index.record(witness_for(&outcome, &shares, &ns, r1));
        assert!(
            matches!(
                f1.as_mut().poll(&mut cx),
                Poll::Ready(Ok(BeaconEvent::SeedRecorded))
            ),
            "the parked receiver is still woken by a persisting record"
        );

        assert_eq!(rx.try_recv().map(|(r, _)| r), Ok(r0));
        assert_eq!(rx.try_recv().map(|(r, _)| r), Ok(r1));
        assert!(
            rx.try_recv().is_err(),
            "exactly one queued write per record"
        );
    }

    // A σ the node cannot check yet is held as `Pending`, not served: a value that
    // reached `seed` unchecked is a fork, not a miss.
    #[test]
    fn pending_seeds_are_never_served_and_promote_when_the_key_lands() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let index = SeedIndex::new();
        let r = round_at(7);
        index.hold(r, recover_seed_for(&outcome, &shares, &ns, r));
        assert_eq!(index.seed(r), None, "a held σ is not served");
        assert_eq!(index.pending_epochs(), vec![1]);

        let oracle = PkOracle::new(*outcome.public().public(), ns.clone());
        assert_eq!(index.settle_epoch(1, &oracle), (1, 0));
        assert!(index.seed(r).is_some(), "a checked seed is served");
        assert!(
            index.pending_epochs().is_empty(),
            "a promoted round leaves the pending state"
        );
    }

    // A valid multisig admits a certificate whose seed slot was never checked, so a
    // peer can park bytes under exactly the round a boundary will ask for; the
    // re-check must drop them.
    #[test]
    fn a_pending_seed_that_fails_its_key_is_dropped_not_retained() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let index = SeedIndex::new();
        let wanted = round_at(7);
        // A genuine σ of a different round: a decodable curve point that verifies
        // under no key for `wanted`.
        index.hold(
            wanted,
            recover_seed_for(&outcome, &shares, &ns, round_at(8)),
        );

        let oracle = PkOracle::new(*outcome.public().public(), ns.clone());
        assert_eq!(index.settle_epoch(1, &oracle), (0, 1));
        assert_eq!(index.seed(wanted), None, "a refused seed is not served");
        assert!(
            !index.snapshot().contains_key(&wanted),
            "a refused seed is evicted so the round can be asked for again"
        );
    }

    // An unchecked admission may never overwrite a checked one, and a checked one
    // always wins over a held value.
    #[test]
    fn a_checked_entry_wins_over_a_held_one_in_both_arrival_orders() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let index = SeedIndex::new();
        let ns_other = seed_namespace(&fluent_namespace(20995));
        let junk = recover_seed_for(&outcome, &shares, &ns_other, round_at(3));

        let first = round_at(3);
        index.hold(first, junk);
        index.record(witness_for(&outcome, &shares, &ns, first));
        assert_eq!(
            index.seed(first),
            Some(recover_seed_for(&outcome, &shares, &ns, first)),
            "a checked σ replaces a held one"
        );

        let second = round_at(4);
        index.record(witness_for(&outcome, &shares, &ns, second));
        index.hold(second, junk);
        assert_eq!(
            index.seed(second),
            Some(recover_seed_for(&outcome, &shares, &ns, second)),
            "an unchecked σ cannot overwrite a checked one"
        );
        assert!(
            index.pending_epochs().is_empty(),
            "and it is not held beside it either"
        );
    }

    // σ is unique per round, so two differing values under one round mean that
    // argument broke; the index must not silently take the second.
    #[test]
    fn a_second_differing_seed_for_one_round_is_refused() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let index = SeedIndex::new();
        let r = round_at(3);
        let first = recover_seed_for(&outcome, &shares, &ns, r);
        let other = recover_seed_for(&outcome, &shares, &ns, round_at(4));
        index.record(VerifiedSeed::from_journal(r, first));
        index.record(VerifiedSeed::from_journal(r, other));
        assert_eq!(index.seed(r), Some(first), "the first value stands");
    }

    // `SEED_RETENTION` is a global round count, so a few thousand rounds into epoch E
    // a plain count bound would evict E-1's terminal round — the round a Signer
    // starting mid-epoch asks for; the terminal rule keeps it in the index.
    #[test]
    fn the_epochs_terminal_round_outlives_the_retention_window() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let index = SeedIndex::new();
        let terminal = Round::new(TEpoch::new(1), View::new(9));
        let below = Round::new(TEpoch::new(1), View::new(8));
        index.record(witness_for(&outcome, &shares, &ns, below));
        index.record(witness_for(&outcome, &shares, &ns, terminal));
        // Enough of the next epoch to evict everything evictable by count. The
        // filler's value is irrelevant, so it skips the per-round threshold recovery.
        let filler = recover_seed_for(&outcome, &shares, &ns, terminal);
        for v in 0..(SEED_RETENTION as u64 + 50) {
            let r = Round::new(TEpoch::new(2), View::new(v));
            index.record(VerifiedSeed::from_journal(r, filler));
        }
        assert!(
            index.seed(terminal).is_some(),
            "the epoch's highest round survives the count bound"
        );
        assert_eq!(
            index.seed(below),
            None,
            "and it is EXACTLY one round per epoch: the round below it went"
        );
        assert_eq!(
            index.snapshot().len(),
            SEED_RETENTION,
            "the protected round is inside the bound, not beside it"
        );
    }

    // The protection tracks the highest round held: by the time anyone asks, from the
    // next epoch, that is the terminal one. Out-of-order arrival must not move it
    // backwards.
    #[test]
    fn the_rule_protects_the_highest_round_of_its_epoch() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let index = SeedIndex::new();
        for v in [3u64, 9, 5] {
            let r = Round::new(TEpoch::new(4), View::new(v));
            index.record(witness_for(&outcome, &shares, &ns, r));
        }
        let filler = recover_seed_for(
            &outcome,
            &shares,
            &ns,
            Round::new(TEpoch::new(4), View::new(9)),
        );
        for v in 0..(SEED_RETENTION as u64 + 50) {
            let r = Round::new(TEpoch::new(5), View::new(v));
            index.record(VerifiedSeed::from_journal(r, filler));
        }
        assert!(
            index
                .seed(Round::new(TEpoch::new(4), View::new(9)))
                .is_some(),
            "out-of-order arrival does not move the protection backwards"
        );
        for v in [3u64, 5] {
            assert_eq!(
                index.seed(Round::new(TEpoch::new(4), View::new(v))),
                None,
                "the protection is one round, not a range"
            );
        }
    }

    // Past the scheme-retention window an epoch has no asker left, so its terminal
    // stops being protected and becomes evictable.
    #[test]
    fn the_protection_ages_out_with_the_scheme_retention_window() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let index = SeedIndex::new();
        let stale = Round::new(TEpoch::new(1), View::new(9));
        index.record(witness_for(&outcome, &shares, &ns, stale));
        // An epoch far enough above `stale` that the window no longer covers it.
        let top = 1 + crate::SCHEME_RETENTION_EPOCHS as u64 + 1;
        let filler = recover_seed_for(&outcome, &shares, &ns, stale);
        for v in 0..(SEED_RETENTION as u64 + 50) {
            let r = Round::new(TEpoch::new(top), View::new(v));
            index.record(VerifiedSeed::from_journal(r, filler));
        }
        assert_eq!(
            index.seed(stale),
            None,
            "an epoch below the retention window is evictable again"
        );
    }

    // A node keyless for the live epoch parks `Pending` rounds without limit. Under
    // one budget for the sum that flood evicts oldest-first, and the oldest entries
    // are the `Verified` σ of the epoch below that this node's executor has not
    // consumed yet.
    #[test]
    fn a_pending_flood_cannot_evict_a_verified_seed() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let index = SeedIndex::new();
        let checked: Vec<Round> = (0..=10)
            .map(|v| Round::new(TEpoch::new(1), View::new(v)))
            .collect();
        for round in &checked {
            index.record(witness_for(&outcome, &shares, &ns, *round));
        }
        // The keyless epoch above, over the budget on its own.
        let held = recover_seed_for(&outcome, &shares, &ns, checked[0]);
        for v in 0..(SEED_RETENTION as u64 + 50) {
            index.hold(Round::new(TEpoch::new(2), View::new(v)), held);
        }
        for round in &checked {
            assert!(
                index.seed(*round).is_some(),
                "a held flood evicted the checked σ of the epoch below: {round:?}"
            );
        }
        assert_eq!(
            index.pending_epochs(),
            vec![2],
            "and the held state is still bounded on its own"
        );
    }

    // The terminal protection is checked-only: a σ the node cannot verify can be
    // filed for any round of the epoch, including one above the terminal block's, and
    // if the rule read the protection off bare map keys that held round would take it
    // and the checked terminal underneath would be evicted — leaving `boundary_base`
    // with `None` for a round nothing else can supply.
    #[test]
    fn a_pending_round_above_the_terminal_does_not_take_its_protection() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let index = SeedIndex::new();
        let terminal = Round::new(TEpoch::new(1), View::new(9));
        index.record(witness_for(&outcome, &shares, &ns, terminal));
        let above = Round::new(TEpoch::new(1), View::new(50));
        index.hold(above, recover_seed_for(&outcome, &shares, &ns, above));
        assert_eq!(index.seed(above), None, "the held round is not servable");

        // Enough of the next epoch to drive the checked bound.
        let filler = recover_seed_for(&outcome, &shares, &ns, terminal);
        for v in 0..(SEED_RETENTION as u64 + 50) {
            let round = Round::new(TEpoch::new(2), View::new(v));
            index.record(VerifiedSeed::from_journal(round, filler));
        }
        assert_eq!(
            index.seed(terminal),
            Some(recover_seed_for(&outcome, &shares, &ns, terminal)),
            "a held round above the terminal took its protection and the checked \
             terminal was evicted under it"
        );
    }

    // Past the scheme-retention edge the epoch's key can never arrive, so its held σ
    // can never settle. The sweep's unit is the epoch: one `PK_e` settles every round
    // of its epoch at once, so a half-swept epoch would leave a remainder nothing can
    // adjudicate.
    #[test]
    fn a_closed_epochs_pending_rounds_are_retired_as_a_unit() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let index = SeedIndex::new();
        let held = recover_seed_for(&outcome, &shares, &ns, round_at(1));
        for epoch in [1u64, 2] {
            for v in 1..=5u64 {
                index.hold(Round::new(TEpoch::new(epoch), View::new(v)), held);
            }
        }
        assert_eq!(index.pending_epochs(), vec![1, 2]);

        // One checked σ far enough above that the window closes on epoch 1 and
        // not on epoch 2 (`floor = top - SCHEME_RETENTION_EPOCHS` = 2).
        let top = 1 + crate::SCHEME_RETENTION_EPOCHS as u64 + 1;
        let round = Round::new(TEpoch::new(top), View::new(0));
        index.record(witness_for(&outcome, &shares, &ns, round));

        assert_eq!(
            index.pending_epochs(),
            vec![2],
            "the epoch the window closed on is no longer held"
        );
        let snapshot = index.snapshot();
        assert!(
            (1..=5).all(|v| !snapshot.contains_key(&Round::new(TEpoch::new(1), View::new(v)))),
            "and it went as a UNIT, not a round at a time"
        );
        assert!(
            (1..=5).all(|v| snapshot.contains_key(&Round::new(TEpoch::new(2), View::new(v)))),
            "the epoch still inside the window is untouched"
        );
    }

    // A poisoned lock must not lose the fact: dropping the σ would let
    // `certificate_verdict` answer `Recorded` for a value the index never took. The
    // map is never mutated across more than one statement, so the guard is recovered.
    #[test]
    fn a_poisoned_index_still_files_and_still_answers() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let index = SeedIndex::new();

        // Poison it exactly as a panicking caller would: a guard dropped during an
        // unwind. The hook is swapped so the deliberate panic does not read as a
        // test failure in the log.
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = index.lock();
            panic!("poison the index lock");
        }));
        std::panic::set_hook(hook);
        assert!(poisoned.is_err(), "the lock must actually be poisoned");
        assert!(
            index.entries.lock().is_err(),
            "…and the poison must have taken effect, or this test proves nothing"
        );

        let round = round_at(3);
        index.record(witness_for(&outcome, &shares, &ns, round));
        assert!(
            index.seed(round).is_some(),
            "a `Recorded` verdict must mean the σ was filed"
        );
        let held = round_at(4);
        index.hold(held, recover_seed_for(&outcome, &shares, &ns, held));
        assert_eq!(
            index.pending_epochs(),
            vec![1],
            "and a `Pending` verdict must mean the σ is held"
        );
    }

    // Rehydrated entries came out of the journal, so writing them back would double
    // it on every restart; only a genuinely new round is queued.
    #[test]
    fn rehydrated_entries_are_not_written_back_to_the_journal() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let r0 = round_at(0);
        let seed0 = recover_seed_for(&outcome, &shares, &ns, r0);

        let (index, mut rx) = SeedIndex::with_persistence(vec![(r0, seed0)], Vec::new());
        assert_eq!(index.seed(r0), Some(seed0), "rehydrated round is readable");
        assert!(
            rx.try_recv().is_err(),
            "construction from the journal queues no writes"
        );

        index.record(witness_for(&outcome, &shares, &ns, r0));
        assert!(
            rx.try_recv().is_err(),
            "re-recording an already-held round queues no write"
        );

        let r1 = round_at(1);
        index.record(witness_for(&outcome, &shares, &ns, r1));
        assert_eq!(
            rx.try_recv().map(|(r, _)| r),
            Ok(r1),
            "a genuinely new round IS queued"
        );
    }

    // A terminal read off the journal is outside the replayed window, and it must
    // survive the window's insertion or a restarting Signer loses exactly the round it
    // was read for.
    #[test]
    fn a_rehydrated_terminal_survives_the_window_it_is_replayed_beside() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let terminal = Round::new(TEpoch::new(1), View::new(9));
        let seed = recover_seed_for(&outcome, &shares, &ns, terminal);
        let window: Vec<_> = (0..(SEED_RETENTION as u64 + 25))
            .map(|v| (Round::new(TEpoch::new(2), View::new(v)), seed))
            .collect();

        let (index, _rx) = SeedIndex::with_persistence(window, vec![(terminal, seed)]);
        assert_eq!(
            index.seed(terminal),
            Some(seed),
            "the terminal read is not evicted by the window replayed with it"
        );
        assert_eq!(index.snapshot().len(), SEED_RETENTION);
    }

    // The rehydrated map obeys the same bound as the live admission path.
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

        let (index, _rx) = SeedIndex::with_persistence(rehydrated, Vec::new());
        assert_eq!(index.snapshot().len(), SEED_RETENTION);
        assert_eq!(index.seed(round_at(0)), None, "oldest dropped");
        assert!(index.seed(round_at(over - 1)).is_some(), "newest kept");
    }
}
