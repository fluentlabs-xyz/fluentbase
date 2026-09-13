//! The `round -> σ` index: the ONE owner of the seed fact.
//!
//! [`SeedIndex`] is a RAM index with a single insertion path. Every σ that
//! reaches it comes through [`crate::beacon::Beacon::observe_certificate`] —
//! the notarization door (`crate::spec_exec`), the live-stream cert inlet, the
//! by-height upstream resolver and the crash-survivor replay all hand their
//! certificate to that one operation — and it is read synchronously by the
//! executor's finalized derive and by the epoch manager's boundary base.
//!
//! # What the index holds, and why it is ONE map
//!
//! An entry is either [`Entry::Verified`] (checked under the epoch's `PK_E`, so
//! servable) or [`Entry::Pending`] (arrived for an epoch whose key is not
//! resolvable here yet). They live in ONE map as two STATES rather than in two
//! maps, and the property that used to justify the split — "an unchecked value
//! can never overwrite a checked one, because insertion is last-wins" — is now
//! carried by the state machine in [`SeedIndex::admit`] instead: a `Pending`
//! admission against a `Verified` entry is dropped, and only the `Verified` arm
//! is ever served.
//!
//! # Retention is a RULE, not a second map
//!
//! One map, but the rule still DISTINGUISHES the two states, because the two
//! maps it replaces had three separate properties that a state-blind count bound
//! silently drops (review C-03/C-04/C-05):
//!
//! 1. **A budget each.** [`SEED_RETENTION`] bounds each STATE, not the sum, so a
//!    `Pending` flood can never evict a `Verified` σ the executor has yet to
//!    consume. That is what two maps with a bound apiece did structurally; here
//!    it is [`bound_pending`] and [`bound_verified`] counting their own state.
//! 2. **The terminal protection is `Verified`-only.** The one round that must
//!    outlive the count is the TERMINAL round of an epoch — a Signer starting
//!    mid-epoch elects on σ of `E-1`'s terminal round, and `SEED_RETENTION` is a
//!    global round count, so a few thousand rounds into `E` a plain count bound
//!    would have dropped it everywhere at once. The eviction therefore SKIPS the
//!    highest round held for each epoch inside the trailing
//!    [`crate::SCHEME_RETENTION_EPOCHS`] window — but only among CHECKED entries
//!    (see [`oldest_evictable`]). On `HEAD` the pin was fed from the verified
//!    insertion path alone and could not hold an unchecked value; reading the
//!    protection off bare map keys would let a `Pending` round above the real
//!    terminal take the protection and the `Verified` terminal be evicted under
//!    it, which sends `boundary_base` into a silent, open-ended `Missing`.
//! 3. **A closed epoch's `Pending` is retired as a UNIT.** Past the
//!    [`crate::SCHEME_RETENTION_EPOCHS`] edge no key can arrive any more, so a
//!    held σ can never settle and is only memory a peer can grow. That sweep is
//!    [`retire_closed_pending_epochs`], and its unit is the EPOCH because
//!    resolution is: one `PK_e` settles every round of its epoch at once, so
//!    retiring by round would tear an epoch in half and leave a remainder that
//!    can never resolve meaningfully.
//!
//! That is the whole of what the separate `terminal` and `quarantined` maps used
//! to do, minus the maps, minus their own eviction windows, and minus the
//! `observe_epoch`/`observe_cert` observations that drove them: the window is
//! measured from the highest epoch the index itself holds, so nothing outside has
//! to tell it where the frontier is.
//!
//! # Reads never touch disk
//!
//! [`crate::beacon::seed_journal`] is the durable mirror and
//! [`SeedIndex::with_persistence`] is how the two are joined at startup. The
//! journal exists for RESTART only: reading it is async (an `Ordinal` store),
//! and [`crate::beacon::Beacon::seed`] is synchronous on every caller.

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

/// Bound on retained `round → σ` entries. A reader wants the seed for a round
/// within a tiny trailing window of its notarization. Generous slack: a seed is
/// 48 B, so a few thousand entries is negligible memory, but a round that
/// notarizes while this node lags a long single block at a boundary may stay
/// wanted for many notarizations — evicting it would cost the derive of that
/// block. Size the window well past any realistic in-flight backlog.
///
/// It is a budget PER STATE, not one budget for the sum: the two maps row 5.2
/// folded together had a bound apiece, and sharing one would let a node keyless
/// for the live epoch spend the whole window on `Pending` entries and evict the
/// `Verified` σ of `E-1` its own executor has not consumed yet. The cost of
/// keeping them separate is the peak: an index holding both states full is
/// `2 · SEED_RETENTION` entries — the same ~400 KB the two maps held, since a σ
/// is 48 B.
pub(crate) const SEED_RETENTION: usize = 4096;

/// One round's σ, and what is known about it.
///
/// The states are what the two maps used to be. `Pending` is NEVER served: a σ
/// that reached [`crate::beacon::prev_randao_from_seed`] unchecked is a fork, not
/// a miss.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Entry {
    /// Checked under the epoch's `PK_E` — the only state [`SeedIndex::seed`]
    /// answers from.
    Verified(BlsSignature),
    /// Received for an epoch whose key is not resolvable here yet. Settled by
    /// [`SeedIndex::settle_epoch`] when the key lands, and never written to the
    /// durable half — losing it on restart is correct, the node re-asks.
    Pending(BlsSignature),
}

/// The `round → σ` index. Cheap to clone (one `Arc` per field); every clone is
/// the same index.
#[derive(Clone)]
pub struct SeedIndex {
    entries: Arc<Mutex<BTreeMap<Round, Entry>>>,
    /// Durable sink. `None` ⇒ RAM-only, which is what every test and the
    /// `--cert-follow` follower get.
    ///
    /// `UnboundedSender::send` is SYNCHRONOUS and never blocks, which is what
    /// lets the durable write sit inside [`SeedIndex::record`] without breaking
    /// the ORDERING-CRITICAL contract in [`crate::spec_exec`]. That contract
    /// constrains in-RAM visibility before the same round's derive; durability is
    /// only ever read by a LATER process. Do NOT swap this for a bounded channel
    /// — `send().await` would put the reporter behind an await, which the
    /// contract forbids.
    persist: Option<mpsc::UnboundedSender<(Round, BlsSignature)>>,
    /// The beacon's wake-up publisher, fired on every verified admission — from
    /// EVERY writer, which is why it lives here and not on the provider above:
    /// the late settle files through this same door, and a wake-up sourced from
    /// one caller would leave a settled σ silently un-woken.
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
    /// The channel is created HERE and its receiving half handed back, rather
    /// than taken as an argument: a sender that only ever exists inside a
    /// `SeedIndex` cannot be used to write an unchecked σ to disk, and that is a
    /// property of the type rather than of the current call sites. See
    /// [`VerifiedSeed::from_journal`] for the other two feeders of the same
    /// partition, which the same argument has to name.
    ///
    /// Both arguments bypass the durable send — they came OUT of the journal and
    /// must not be written back — and both go through the same
    /// [`admit`](Self::admit) as [`record`](Self::record), so the index holds
    /// nothing but witnessed values and [`SEED_RETENTION`] bounds this route too.
    ///
    /// `terminals` is a SEPARATE argument because `rehydrated` cannot carry it:
    /// `replay_window` walks newest-first and stops at `retention` RECORDS, so
    /// once the current epoch is past [`SEED_RETENTION`] rounds the previous
    /// epoch's terminal is not in that set — and that is the one round a
    /// restarting Signer needs. Inserting it here is enough to keep it: the
    /// eviction rule protects the highest round of each retained epoch, so it
    /// does not have to be filed anywhere special.
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
        // The terminals go in FIRST so the window's own eviction sees them and
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
    /// Idempotent: σ is unique per round, so a re-report (peer cert after
    /// self-assembly, or replay) writes the same value. Publishes
    /// [`BeaconEvent::SeedRecorded`] unconditionally (even on an idempotent
    /// re-record — harmless, the executor arm's eager derive is idempotent) so a
    /// held tip waiting on a late record is woken.
    pub fn record(&self, verified: VerifiedSeed) {
        self.admit((verified.round(), verified.seed()), true, true);
    }

    /// Hold a σ this node cannot check yet, for the epoch key to settle later.
    ///
    /// A STATE of the round rather than a second map: see [`Entry`].
    pub fn hold(&self, round: Round, seed: BlsSignature) {
        self.admit((round, seed), false, false);
    }

    /// The ONE insertion path. `verified` picks the state; `persist` is false for
    /// entries replayed from the journal, which must not be written back to it.
    ///
    /// Two VERIFIED values differing under one round would mean σ is not unique
    /// per round — the argument every consumer of this index rests on, including
    /// the fork-safety of keying an ingress-captured σ by the certificate's own
    /// round (§13 rule 28). Refuse the overwrite and be loud rather than let the
    /// served state hold a value the assembler and the witness disagree about.
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
            // A checked value already stands: an UNCHECKED one says nothing new
            // about this round, and taking it would be the last-wins overwrite
            // the two-map split existed to prevent.
            (Some(Entry::Verified(_)), false) => return,
            // `Pending` ⇒ `Pending` is LAST-WINS, unchanged from the map it
            // replaces. Neither value can be adjudicated without the key, and the
            // one that fails the settle is DROPPED rather than kept, so the round
            // becomes askable again either way.
            _ => {}
        }
        // A fresh DURABLE value is one whose round did not already hold a checked
        // σ: promoting a `Pending` to `Verified` is the first time that round is
        // worth writing down.
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
            // Nothing to wake and nothing to write: a held σ is not servable, and
            // the journal is for checked values only.
            return;
        }
        // Wake the executor's held-tip arm (a HELD tip whose own round's seed just
        // landed). The broadcast is buffered from each consumer's subscription,
        // which is why every consumer subscribes before its first read.
        let _ = self.events.send(BeaconEvent::SeedRecorded);
        // Durable half, strictly AFTER the wake-up so the wakeup latency is
        // unchanged, and strictly non-blocking so the reporter never parks.
        if fresh && persist {
            if let Some(tx) = self.persist.as_ref() {
                if tx.send((round, seed)).is_err() {
                    warn!("seed journal writer is gone; seed recorded in memory only");
                }
            }
        }
    }

    /// The σ in force at `round`, if this node holds it CHECKED.
    ///
    /// EXACT round only, in both directions: a neighbouring round is a miss, and
    /// a `Pending` entry is a miss too. The terminal-round exemption is an
    /// EVICTION rule (see [`oldest_evictable`]) — it decides what survives, never
    /// which round is answered, so the boundary base that asks for `E-1`'s
    /// terminal round gets σ of exactly the round it named or nothing.
    pub fn seed(&self, round: Round) -> Option<BlsSignature> {
        match self.lock().get(&round)? {
            Entry::Verified(seed) => Some(*seed),
            Entry::Pending(_) => None,
        }
    }

    /// The one lock, ALWAYS taken.
    ///
    /// A poisoned lock means some caller panicked while holding it. The guarded
    /// value is a plain [`BTreeMap`] and no path here mutates it across more than
    /// one statement, so it cannot be left half-written — the same argument
    /// [`crate::beacon::artifact::ArtifactStore`] makes about its own map, and the
    /// same conclusion: recover the guard rather than lose the fact.
    ///
    /// The alternative was tried and is WRONG (review C-13): answering `None` /
    /// dropping the σ made the operation fail SILENTLY while
    /// [`super::surface::certificate_verdict`] still answered `Recorded`, so the
    /// ingress believed it had filed a σ the index never took — and since row 5.2
    /// the crash-replay path acts on that verdict. Taking the node down instead
    /// would forfeit the very σ the index exists to serve.
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
    /// A σ that now fails is DROPPED, not left behind to be re-checked forever:
    /// the round can then be asked for again. Keeping it would let a peer park
    /// garbage under exactly the round a boundary needs and have it answer every
    /// later re-check.
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
                // Still unresolvable — the key did not land for THIS epoch after
                // all. Leave the entry `Pending`.
                Err(_) => {}
            }
        }
        let mut entries = self.lock();
        for (round, seed) in dropped {
            // Remove only what is still the value that was refused: another door
            // may have filed a CHECKED σ for the round while the verification
            // above ran.
            if entries.get(&round) == Some(&Entry::Pending(seed)) {
                entries.remove(&round);
            }
        }
        drop(entries);
        (promoted, refused)
    }

    /// The wake-up publisher every consumer of this index subscribes to. The
    /// beacon hands it out as its own — the seed class is the busiest of the
    /// three and the only one written from inside a store.
    pub fn events(&self) -> &broadcast::Sender<BeaconEvent> {
        &self.events
    }

    /// Every round the index holds, with its state, for this module's tests. NOT
    /// a production read: the states are an implementation detail of the
    /// eviction rule and of the settle, and every production caller asks about
    /// ONE round it named.
    #[cfg(test)]
    fn snapshot(&self) -> BTreeMap<Round, Entry> {
        self.lock().clone()
    }
}

/// Apply the retention rule: retire the `Pending` epochs the window has closed
/// on, then bound each STATE by [`SEED_RETENTION`] on its own.
///
/// The order matters: a closed epoch's held σ is dead weight by definition, so it
/// goes before anything live is asked to make room.
fn evict(entries: &mut BTreeMap<Round, Entry>) {
    // The window is measured from the index's OWN highest epoch. That is what
    // makes the rule self-contained: the two observations that used to carry a
    // frontier into the seed maps (`observe_epoch`, `observe_cert`) have no leg
    // left here, because σ arrives per round and the highest round held IS this
    // node's σ frontier.
    let Some(top_epoch) = entries.keys().next_back().map(|round| round.epoch().get()) else {
        return;
    };
    let floor = top_epoch.saturating_sub(crate::SCHEME_RETENTION_EPOCHS as u64);
    retire_closed_pending_epochs(entries, floor);
    // Each state has its own budget, so neither can be over it while the whole
    // index is under one of them: one `len` read covers the ordinary case and
    // keeps the two counting scans off the admission path.
    if entries.len() <= SEED_RETENTION {
        return;
    }
    bound_pending(entries);
    bound_verified(entries, floor);
}

/// Drop every `Pending` round of an epoch below `floor`.
///
/// This is what the deleted `retain_quarantine_from` did, minus the
/// `observe_epoch`/`observe_cert` caller that had to drive it from outside. Past
/// the scheme-retention edge the epoch's key can never arrive, so the σ can never
/// be settled — and a held σ nothing can adjudicate is memory a peer grows for
/// free.
///
/// The unit is the EPOCH, not the round: one `PK_e` settles every round of its
/// epoch at once, so retiring by round would tear an epoch in half and leave a
/// remainder that can never resolve meaningfully. Entries are ordered by
/// `(epoch, view)`, so the closed epochs are exactly the prefix below `floor`.
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

/// Bound the `Pending` state by [`SEED_RETENTION`] on its own count.
///
/// Whole epochs go first, oldest first, for the reason
/// [`retire_closed_pending_epochs`] gives. The LAST remaining pending epoch is
/// the exception: it is trimmed from its OLDEST end instead of dropped whole,
/// because it is the epoch whose key can still land, and the round a boundary
/// will ask for is the HIGHEST one held — so the newest end is the end to keep,
/// and what is left is not the unresolvable remainder the per-epoch rule guards
/// against. The cost, named: a node keyless for longer than
/// `SEED_RETENTION` rounds of ONE epoch loses that epoch's oldest held σ and has
/// to re-ask for them once the key lands.
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
/// oldest-first and SKIPPING the rounds the terminal rule protects.
fn bound_verified(entries: &mut BTreeMap<Round, Entry>, floor: u64) {
    let mut verified = entries
        .values()
        .filter(|entry| matches!(entry, Entry::Verified(_)))
        .count();
    while verified > SEED_RETENTION {
        let Some(victim) = oldest_evictable(entries, floor) else {
            // Every checked entry left is a protected terminal — at most one per
            // epoch in the trailing window, so this cannot be a leak. It can only
            // be reached with `SEED_RETENTION` below that window's size.
            break;
        };
        entries.remove(&victim);
        verified -= 1;
    }
}

/// The lowest CHECKED round that may be dropped: the oldest [`Entry::Verified`]
/// that is NOT the highest verified round of an epoch inside the trailing
/// retention window.
///
/// `Pending` rounds are skipped on BOTH counts — they are not candidates (they
/// have their own bound) and they do not confer the protection either. Reading
/// the protection off bare map keys would let a held σ above the real terminal
/// take it while the checked terminal underneath is evicted, and `boundary_base`
/// would then miss a round nothing else can supply. On `HEAD` that was
/// structurally impossible: the pin was written from the verified insertion path
/// alone.
///
/// "Highest HELD", and deliberately not "terminal": the two are usually the same
/// and cannot be shown to be. A hard kill between an admission and the journal's
/// sync loses the tail, so a restarted node's highest round for an epoch can sit
/// one round below that epoch's real last round. Nobody may trust this rule to
/// NAME the terminal round — the canonical round is
/// `Round(E, terminal_block.proposal_view)`, agreed data carried by the block —
/// and nobody does: the rule decides only what survives eviction, and
/// [`SeedIndex::seed`] answers the round the CALLER named.
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
        // Ordered by `(epoch, view)`, so this round is the highest VERIFIED round
        // of its epoch exactly when the next checked one belongs to another epoch
        // (or there is none).
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

    // LOST-WAKEUP ABSENCE (the event arm's correctness — the awaitable seed
    // lookup that REPLACES the `SpecNotarized` Poke, family2_finalized_tier.md
    // §2.2). Two records, two receiver orderings, both over a receiver taken
    // BEFORE the first record, which is the rule the trait writes down:
    //   (1) record while nobody is polling → the broadcast BUFFERS it → the next
    //       `recv()` is IMMEDIATELY ready. This is the load-bearing case: it
    //       closes the window between the executor's eager MISS lookup and its
    //       next await — a seed record landing there is not lost (the old Poke
    //       depended on the `SpecNotarized` mailbox ordering).
    //   (2) receiver parked BEFORE the record → woken by it.
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

        // (1) A record that lands with nobody polling is buffered and read by the
        // next `recv()` — the no-lost-notification window.
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

        // (2) A receiver parked before the next record is woken by it.
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

    // The same two receiver orderings against a PERSISTING index. The durable sink
    // sits after the event send in `admit`, so it must not change either verdict —
    // if it ever did, the executor's eager-derive arm would silently lose the
    // record-vs-delivery race that this permit closes.
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

    // A σ the node cannot check yet is HELD as `Pending`, not served: `seed` is
    // the read every consumer of `prev_randao` ends up behind, and a value that
    // reached it unchecked is a fork, not a miss.
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

    // A valid multisig admits a certificate whose seed slot was never checked, so
    // a peer can park bytes under exactly the round a boundary will ask for. The
    // re-check must DROP them, not keep answering itself with them for ever.
    #[test]
    fn a_pending_seed_that_fails_its_key_is_dropped_not_retained() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let index = SeedIndex::new();
        let wanted = round_at(7);
        // A genuine σ of a DIFFERENT round: a decodable curve point that verifies
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

    // The STATE MACHINE that replaces the two maps: an unchecked admission may
    // never overwrite a checked one (it would be the last-wins fork the split
    // used to prevent), and a checked one always wins over a held value.
    #[test]
    fn a_checked_entry_wins_over_a_held_one_in_both_arrival_orders() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let index = SeedIndex::new();
        let ns_other = seed_namespace(&fluent_namespace(20995));
        let junk = recover_seed_for(&outcome, &shares, &ns_other, round_at(3));

        // held → checked: the checked value replaces it and is served.
        let first = round_at(3);
        index.hold(first, junk);
        index.record(witness_for(&outcome, &shares, &ns, first));
        assert_eq!(
            index.seed(first),
            Some(recover_seed_for(&outcome, &shares, &ns, first)),
            "a checked σ replaces a held one"
        );

        // checked → held: the held value is dropped on the floor.
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

    // σ is unique per round — the argument the witness, the executor and the
    // ingress capture all rest on. Two differing values under one round mean that
    // argument broke somewhere, and the index must not silently take the second:
    // `from_journal` is the only way to stage the impossible pair.
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

    // THE TERMINAL RULE'S WHOLE REASON: `SEED_RETENTION` is a global round COUNT,
    // so a few thousand rounds into epoch E a plain count bound has evicted E-1's
    // terminal round — and that is the round a Signer starting mid-epoch asks
    // for. It is the same claim the deleted terminal MAP carried; what changed is
    // that the round now survives IN the index, so the one read (`seed`) answers
    // it instead of a second read over a second map.
    #[test]
    fn the_epochs_terminal_round_outlives_the_retention_window() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let index = SeedIndex::new();
        let terminal = Round::new(TEpoch::new(1), View::new(9));
        let below = Round::new(TEpoch::new(1), View::new(8));
        index.record(witness_for(&outcome, &shares, &ns, below));
        index.record(witness_for(&outcome, &shares, &ns, terminal));
        // Enough of the NEXT epoch to evict everything evictable by count. The
        // filler's VALUE is irrelevant — the claim is about the count bound — so
        // it skips the per-round threshold recovery that would dominate the test.
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

    // The protection tracks the highest round HELD, because nobody knows which
    // round is terminal until the epoch ends — and by the time anyone asks (from
    // the next epoch) the highest held is the terminal one. Out-of-order arrival
    // must not move it backwards.
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

    // Past the scheme-retention window an epoch has no asker left: the next epoch
    // asked for its terminal round, and that epoch is long gone. So the
    // protection is bounded by the window and the entry becomes evictable —
    // which is what the deleted `retain_terminal_from` did, minus the caller that
    // had to drive it.
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

    // SEPARATE BUDGETS (review C-03). A node keyless for the live epoch parks
    // `Pending` rounds without limit. Under ONE budget for the sum that flood
    // evicts oldest-first, and the oldest entries are the `Verified` σ of the
    // epoch BELOW — rounds this node's own executor has not consumed yet — so the
    // node holds those heights for ever. Two maps with a bound apiece made it
    // structurally impossible; here the bound is per STATE.
    //
    // FALSIFIER (one line): in `bound_verified`, make the loop count the whole
    // index — `while entries.len() > SEED_RETENTION {`. The held flood then
    // evicts the checked rounds below it and this test goes red.
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

    // THE TERMINAL PROTECTION IS CHECKED-ONLY (review C-04). A σ the node cannot
    // verify can be filed for ANY round of the epoch, including one above the
    // terminal block's. If the rule read the protection off bare map keys, that
    // held round would take it and the CHECKED terminal underneath would be
    // evicted — and `boundary_base` would then get `None` for a round nothing else
    // can supply, deferring the next epoch's engine silently and without end.
    //
    // FALSIFIER (one line): delete the
    // `.filter(|(_, entry)| matches!(entry, Entry::Verified(_)))` line in
    // `oldest_evictable`.
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

    // THE WINDOW SWEEP, PER EPOCH (review C-05). Past the scheme-retention edge
    // the epoch's key can never arrive, so its held σ can never settle and is only
    // memory a peer grows for free — the property `retain_quarantine_from` carried
    // before the two maps became one. The unit is the EPOCH because resolution is:
    // one `PK_e` settles every round of its epoch at once, so a half-swept epoch
    // would leave a remainder nothing can ever adjudicate.
    //
    // FALSIFIER (one line): delete the `retire_closed_pending_epochs(entries,
    // floor);` call in `evict`. The closed epoch then survives on the count bound
    // alone and `pending_epochs()` still names it.
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

    // A POISONED LOCK MAY NOT LOSE THE FACT (review C-13). The index used to warn
    // and drop the σ, while `certificate_verdict` above it still answered
    // `Recorded` — so the ingress believed it had filed a value the index never
    // took, and since row 5.2 the crash-replay path acts on that verdict. The map
    // cannot be left half-written (no path mutates it across more than one
    // statement), so the guard is recovered instead.
    //
    // FALSIFIER (one line): in `SeedIndex::lock`, replace
    // `.unwrap_or_else(std::sync::PoisonError::into_inner)` with
    // `.expect("index lock")`.
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

    // A terminal read off the journal is outside the replayed window by
    // construction, and it has to survive the window's own insertion — otherwise
    // a restarting Signer loses exactly the round `with_persistence` read it for.
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

    // The rehydrated map obeys the same bound as the live admission path, so a
    // journal window larger than the index's cannot grow it unbounded.
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
