//! Slasher actor: per-Activity filter → committee resolve → verify → hold the
//! charge for a block, or hand it to the transaction fallback once its epoch has
//! ended.
//!
//! The producer keeps the votes it used to discard. Simplex only raises a
//! `Conflicting*` activity when one node sees both halves of an equivocation,
//! which a split-delivering equivocator never allows; [`VoteStore`] pairs the
//! halves instead. Every charge — assembled here or witnessed as a
//! `Conflicting*` — is verified and held in the shared [`ChargeStore`], which
//! [`ChargeStore::next_charge`] hands to a proposer one charge per block.
//!
//! **A charge is block-eligible only inside its own epoch**: the vote-time gate
//! ([`super::evidence::verify_block_charge`]) refuses any other, because only
//! that epoch's committee can resolve a verifier for it. So an epoch turn — the
//! event [`Actor::handle`] watches for — strands whatever is still queued, and
//! [`Actor::drain_stale_charges`] hands those to the transaction route instead.
//! That route is the actor's producer/consumer split: the producer enqueues
//! `(victim || calldata)` blobs onto a `commonware_storage::queue::shared` WAL,
//! and `run_consumer` dequeues, hands them to the [`SlasherTxSink`], and acks on
//! Mined / AlreadySlashed (goal achieved on-chain) or leaves the entry un-acked
//! on submission failure. NOTE: an un-acked entry is re-delivered ONLY after a
//! process restart — `recv` advances `read_pos` unconditionally, so there is no
//! automatic in-session retry.

use super::evidence::{
    attributable_signer_idx, extract_from_conflicting_finalize, extract_from_conflicting_notarize,
    extract_from_nullify_finalize, verify_pre_submit_vote_only, SlashCallArgs, SlashKind,
};
use crate::{
    digest::Digest,
    scheme::epoch_committee_from_snapshot,
    slasher::{
        gossip::{encode_batch, verify_vote, EvidenceBatch, EvidenceBridge},
        ingress::{Envelope, Mailbox, Message, Provenance},
    },
};
use alloy_primitives::{Address, Bytes, B256};
use alloy_sol_types::SolCall;
use commonware_consensus::{
    simplex::types::{
        Activity, Attributable, ConflictingFinalize, ConflictingNotarize, Finalize, Notarize,
        Nullify, NullifyFinalize, Proposal, Vote,
    },
    types::Round,
    Epochable, Viewable,
};
use commonware_runtime::{spawn_cell, Clock, ContextCell, Handle, Metrics, Spawner, Storage};
use commonware_storage::queue::shared as wal_queue;
use fluentbase_bls::{fluent_namespace, EpochCommittee, Scheme as BlsScheme, VoteScheme};
use fluentbase_p2p::constants::MAX_COMMITTEE_SIZE;
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use rand_core::OsRng;
use std::{
    collections::{btree_map::Entry, BTreeMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, PoisonError, RwLock, RwLockReadGuard,
    },
};
use tokio::sync::{mpsc, Mutex as TokioMutex};
use tracing::{debug, error, info, instrument, warn};

// The three slash entry points come from `fluentbase-staking-abi`, the ONE
// declaration the contract derives its dispatch selectors from. This used to be
// a second `sol!` block here, kept in step with a merge checklist that described
// a delta against an unmerged contract branch; the branch merged as `f16fdd90`
// and the contract now sits in `contracts/staking` of this tree, so the delta and
// the checklist are both gone. The production encoder and the conformance tests
// still go through one declaration — the tests must never re-declare it, or they
// would validate the code against itself.
pub use fluentbase_staking_abi::{
    slashEquivocationFinalizeCall, slashEquivocationNotarizeCall,
    slashEquivocationNullifyFinalizeCall,
};

// The slasher consumes `StakingStateRead` re-exported from
// `fluentbase-staking-reader`. The blanket impl on
// `RethStakingStateReader<P, E>` in
// `crates/staking-reader/src/epoch_transition.rs` provides the production impl.
use fluentbase_staking_reader::StakingStateRead;

/// Closure returning the latest finalized block hash (or `None` if not yet
/// known). Threaded in from `dpos.rs`, wraps the reth provider.
pub type LatestFinalizedHash = Arc<dyn Fn() -> Option<B256> + Send + Sync>;

/// Backoff between producer retries of a transient `handle` failure.
const SLASHER_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_secs(2);
/// Max producer attempts for one Activity before it is dropped (a missing
/// committee/scheme/finalized-hash at startup resolves within seconds; a
/// dependency that is still down after this bound is an operational failure
/// surfaced by the drop log + metric, not a silent loss on the first hiccup).
const SLASHER_MAX_RETRIES: u32 = 30;

/// Outcome of [`Actor::handle`] classifying a failure by retry-ability.
/// TRANSIENT failures (startup races: no finalized hash, RPC/state read error,
/// scheme/committee not yet registered, storage hiccup) are re-attempted by the
/// producer; PERMANENT failures (malformed/variant-mismatch evidence, BiMap
/// divergence, an epoch that was never committed) are dropped — retrying the same
/// bytes can never succeed.
enum HandleError {
    Transient(eyre::Report),
    Permanent(eyre::Report),
}

impl HandleError {
    fn transient(msg: impl Into<String>) -> Self {
        Self::Transient(eyre::eyre!("{}", msg.into()))
    }
    fn permanent(msg: impl Into<String>) -> Self {
        Self::Permanent(eyre::eyre!("{}", msg.into()))
    }
}

/// Transport abstraction over the reth `TransactionPool`. Production
/// impl in `dpos.rs` owns the slasher EOA key + `node.pool` + `node.provider`
/// and signs + submits + awaits transaction inclusion. Tests provide a
/// recording stub.
///
/// The consumer task hands `(target, calldata)` and waits for an outcome;
/// the sink is responsible for nonce management and on-chain confirmation
/// semantics (no HTTP RPC; uses `TransactionPool::add_consensus_transaction`).
pub trait SlasherTxSink: Send + Sync + 'static {
    fn submit<'a>(
        &'a self,
        target: Address,
        calldata: Bytes,
    ) -> std::pin::Pin<Box<dyn core::future::Future<Output = SubmitOutcome> + Send + 'a>>;
}

/// Outcome categories used by the consumer to decide whether to ack the WAL
/// entry. The production sink **pre-flight-simulates** the slash call before
/// submitting (a receipt carries only a success bit, not the revert reason, so
/// revert classification must happen at simulation time):
/// - `Mined` — tx submitted AND confirmed on-chain with `status == 1`. Ack.
/// - `AlreadySlashed` — pre-flight simulation reverted with
///   `AlreadySlashedForEquivocation` (victim already tombstoned). The goal is
///   already achieved; ack without submitting a tx.
/// - `Failed` — simulation or submission failed (a deterministic encoding bug,
///   an unexpected revert, or a transient pool/inclusion error). Do NOT ack;
///   the entry is re-delivered after a process restart (NOT in-session).
///   Always paired with a loud log.
#[derive(Debug, Clone)]
pub enum SubmitOutcome {
    /// Tx submitted and confirmed on-chain with receipt `status == 1`.
    Mined { tx_hash: B256 },
    /// Pre-flight simulation showed the victim is already tombstoned
    /// (`AlreadySlashedForEquivocation`); no tx submitted. Goal achieved → ack.
    AlreadySlashed,
    /// Simulation/submission failed (bug, unexpected revert, or transient). Not
    /// acked; retried only after a process restart.
    Failed(String),
}

/// Configuration passed to [`Actor::init`]. The WAL queue handles are
/// constructed by the outer layer (via [`init_wal_queue`]) — they cannot
/// be built inside `Actor::init` because `init` is a synchronous function
/// and `queue::shared::init` is async.
pub struct Config<R, E>
where
    R: StakingStateRead + Send + Sync + 'static,
    E: Clock + Metrics + Spawner + Storage + Send + 'static,
{
    /// Staking predeploy address.
    pub staking_address: Address,
    /// L2 chain id — used to rebuild a verifier scheme (`fluent_namespace`) for
    /// an evidence epoch whose scheme the provider has pruned but whose
    /// committee is still on chain (§14).
    pub chain_id: u64,
    /// Reader for committee resolution (dedicated instance, NOT shared with ET).
    pub reader: R,
    /// Latest finalized hash provider (used as `at` block for snapshot lookup).
    pub latest_finalized_hash: LatestFinalizedHash,
    /// TxPool transport. Production impl wraps signer + pool + provider.
    pub sink: Arc<dyn SlasherTxSink>,
    /// WAL writer half. Producer (`handle`) enqueues
    /// `(victim || calldata)` payloads here after `verify_pre_submit`.
    pub wal_writer: wal_queue::Writer<E, Vec<u8>>,
    /// WAL reader half. Consumer (`run_consumer`) dequeues + submits + acks.
    pub wal_reader: wal_queue::Reader<E, Vec<u8>>,
    /// Evidence-channel bridge to the node's gossip task. `None` on the
    /// follower path, whose slasher is constructed but never started.
    pub evidence: Option<EvidenceBridge>,
    /// The store this actor fills and the proposer drains. Passed in rather
    /// than created here because `FluentApp` is built before the slasher is.
    pub charges: ChargeStore,
}

/// Verified charges awaiting inclusion in a block, keyed `(epoch, signer_idx)`
/// so the queue drains deterministically. The slasher fills it; the proposer
/// reads it through [`Self::next_charge`] while building a block, which is why
/// it is a shared handle rather than a field of [`Actor`].
///
/// **Charges outlive votes, deliberately.** [`VoteStore`] is pruned on the
/// engine's own 64-view window; a charge is not. A block carries at most one
/// charge and a holder may not reach a leader slot before that window would have
/// expired — dropping the charge there would lose a provable fault while the
/// mechanism was working perfectly. The set is small by construction (one entry
/// per equivocator per epoch), so it needs no cap.
///
/// Not persisted, and it does not need to be: every node that assembled the same
/// pair holds the same charge, so losing one node's copy loses nothing.
#[derive(Clone, Default)]
pub struct ChargeStore(Arc<RwLock<BTreeMap<(u64, u8), Message>>>);

impl ChargeStore {
    /// Lowest-indexed charge held for `epoch` — what a proposer stamps into the
    /// block it is building.
    ///
    /// The epoch argument is not optional: only the committee of a charge's own
    /// epoch can verify it, so offering an older charge would produce a block
    /// every voter rejects, and `BTreeMap` ordering would keep re-offering that
    /// same lowest key. Deterministic order means successive proposers drain the
    /// queue without coordinating — two proposers picking the same victim is
    /// harmless, the second system call is idempotent.
    ///
    /// A charge whose victim `tombstoned` reports as already slashed is DROPPED,
    /// not skipped: the verdict it carries has landed, so nothing it could still
    /// achieve is lost, and leaving it in place would let the lowest such key
    /// occupy the one-charge-per-block slot ahead of every later charge for the
    /// same epoch. That is the whole reason the walk prunes as it goes rather
    /// than filtering on the way out.
    pub fn next_charge(
        &self,
        epoch: u64,
        tombstoned: impl Fn(u8) -> bool,
    ) -> Option<(u8, Message)> {
        let mut map = self.0.write().unwrap_or_else(PoisonError::into_inner);
        let mut settled = Vec::new();
        let mut next = None;
        for (&(_, accused), charge) in map.range((epoch, u8::MIN)..=(epoch, u8::MAX)) {
            if tombstoned(accused) {
                settled.push((epoch, accused));
                continue;
            }
            next = Some((accused, charge.clone()));
            break;
        }
        for key in settled {
            metrics::counter!("slasher_charges_settled_total").increment(1);
            map.remove(&key);
        }
        next
    }

    /// Whether this equivocator is already charged for this epoch. `pub(crate)`
    /// alongside [`Self::hold`]: the proposer side asserts on it too.
    pub(crate) fn contains(&self, epoch: u64, accused: u8) -> bool {
        self.read().contains_key(&(epoch, accused))
    }

    /// Every charge held for an epoch below `epoch`, cloned out in key order.
    ///
    /// The drain reads rather than takes: an entry is released only once its
    /// transaction is on the WAL, so a committee that will not resolve yet
    /// leaves the charge queued for the next epoch turn to retry instead of
    /// dropping a provable fault.
    fn stale(&self, epoch: u64) -> Vec<((u64, u8), Message)> {
        self.read()
            .range(..(epoch, u8::MIN))
            .map(|(key, charge)| (*key, charge.clone()))
            .collect()
    }

    fn release(&self, key: (u64, u8)) {
        self.0
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&key);
    }

    /// Hold `charge` unless this equivocator is already charged for this epoch.
    /// Returns whether it was newly held.
    pub(crate) fn hold(&self, epoch: u64, accused: u8, charge: Message) -> bool {
        let mut map = self.0.write().unwrap_or_else(PoisonError::into_inner);
        match map.entry((epoch, accused)) {
            Entry::Vacant(slot) => {
                slot.insert(charge);
                true
            }
            Entry::Occupied(_) => false,
        }
    }

    /// A poisoned lock means a holder panicked mid-update. The map is a plain
    /// `BTreeMap` of owned values, so it cannot be torn — recovering keeps one
    /// panic from silencing the accountability path for the rest of the process.
    fn read(&self) -> RwLockReadGuard<'_, BTreeMap<(u64, u8), Message>> {
        self.0.read().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The highest epoch this node's own simplex engine has reported, published as a
/// read handle for the evidence-gossip consumer.
///
/// A shared handle for the same reason [`ChargeStore`] and
/// [`super::TombstoneSet`] are: the reader — the node's evidence task — is built
/// before the actor that writes it exists.
///
/// **Single writer, monotone.** [`Actor::handle`] is the only writer and only
/// ever raises it, and only for [`Provenance::Engine`] activities: a
/// peer-forwarded vote names whatever epoch its signer chose, so letting gossip
/// move this would hand one member of a future committee a switch for the
/// in-block charge route (see [`Provenance`]). Monotone and single-writer is why
/// this is an atomic rather than the `RwLock` its two sibling handles use — there
/// is no multi-field invariant to hold across the update.
#[derive(Clone, Default, Debug)]
pub struct EpochCursor(Arc<AtomicU64>);

impl EpochCursor {
    /// The highest epoch the local engine has reported.
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }

    /// Whether a forwarded vote naming `epoch` is inside the window
    /// [`VoteStore::retain_floor`] actually keeps entries for.
    ///
    /// Anything outside it is work with no possible product: a vote for an older
    /// epoch is pruned the moment it lands, and one for a later epoch names a
    /// round this node has no reason to believe exists. Checked **before** the
    /// committee is resolved, so an unbounded claimed epoch cannot buy a state
    /// read per message either.
    pub fn retains(&self, epoch: u64) -> bool {
        let current = self.get();
        epoch <= current && epoch >= current.saturating_sub(1)
    }

    /// Raise the cursor. `fetch_max` rather than a store because the cursor is
    /// monotone by contract, not by the order calls happen to arrive in.
    ///
    /// `pub(crate)` only so a test elsewhere in the crate can stand the cursor
    /// up without a running engine; [`Actor::handle`] is the sole production
    /// writer, and it writes only for [`Provenance::Engine`].
    pub(crate) fn advance(&self, epoch: u64) {
        self.0.fetch_max(epoch, Ordering::Relaxed);
    }
}

/// Mirrors the engine's own tracking window (`timeouts.rs`
/// `activity: ViewDelta::new(64)`) so the vote store never holds votes simplex
/// itself has already forgotten.
const RETAIN_VIEWS: u64 = 64;

/// `(epoch, view, signer_index)` — the round and signer a vote is bound to.
/// Two votes of one kind under the same key are either a duplicate or the two
/// halves of an equivocation; the key already pins the round and signer
/// equality that `Conflicting*::new` asserts on.
type VoteKey = (u64, u64, u32);

fn vote_key(round: Round, signer_idx: u32) -> VoteKey {
    (round.epoch().get(), round.view().get(), signer_idx)
}

/// Inclusive key bounds covering every signer of `round` — the signer index is
/// the last key component, so one round is one contiguous `BTreeMap` range.
fn round_key_span(round: Round) -> (VoteKey, VoteKey) {
    (vote_key(round, u32::MIN), vote_key(round, u32::MAX))
}

/// The peers' own signed votes, one map per kind.
///
/// A split-delivered equivocation never produces a `Conflicting*` activity on
/// any single node — each half reaches a disjoint set of peers and the halves
/// only meet if someone keeps them. Keeping them is this store's whole job:
/// a second vote under an existing key is paired into the matching evidence.
///
/// Simplex reports votes BEFORE signature verification
/// (`COMMONWARE_INTERNALS.md`, "Reporter sees unverified votes"), so a pair
/// assembled here is a *candidate*. The crypto gate is [`verify_charge`],
/// applied before the charge is held.
#[derive(Default)]
struct VoteStore {
    notarizes: BTreeMap<VoteKey, Notarize<BlsScheme, Digest>>,
    finalizes: BTreeMap<VoteKey, Finalize<BlsScheme, Digest>>,
    nullifies: BTreeMap<VoteKey, Nullify<BlsScheme>>,
    /// Highest finalized view observed inside [`Self::floor_epoch`]. View
    /// numbers restart per epoch, so a floor is only comparable within the
    /// epoch it was measured in.
    floor: u64,
    floor_epoch: u64,
}

impl VoteStore {
    fn remember_notarize(&mut self, notarize: Notarize<BlsScheme, Digest>) -> Option<Message> {
        let key = vote_key(notarize.round(), notarize.signer().get());
        match self.notarizes.entry(key) {
            Entry::Occupied(held) if held.get().proposal != notarize.proposal => {
                Some(Activity::ConflictingNotarize(ConflictingNotarize::new(
                    held.get().clone(),
                    notarize,
                )))
            }
            Entry::Occupied(_) => None,
            Entry::Vacant(slot) => {
                slot.insert(notarize);
                None
            }
        }
    }

    fn remember_finalize(&mut self, finalize: Finalize<BlsScheme, Digest>) -> Option<Message> {
        let key = vote_key(finalize.round(), finalize.signer().get());
        // A nullify and a finalize for the same round contradict each other on
        // their own — a nullify carries no proposal to compare.
        if let Some(nullify) = self.nullifies.get(&key) {
            return Some(Activity::NullifyFinalize(NullifyFinalize::new(
                nullify.clone(),
                finalize,
            )));
        }
        match self.finalizes.entry(key) {
            Entry::Occupied(held) if held.get().proposal != finalize.proposal => {
                Some(Activity::ConflictingFinalize(ConflictingFinalize::new(
                    held.get().clone(),
                    finalize,
                )))
            }
            Entry::Occupied(_) => None,
            Entry::Vacant(slot) => {
                slot.insert(finalize);
                None
            }
        }
    }

    fn remember_nullify(&mut self, nullify: Nullify<BlsScheme>) -> Option<Message> {
        let key = vote_key(nullify.round, nullify.signer().get());
        if let Some(finalize) = self.finalizes.get(&key) {
            return Some(Activity::NullifyFinalize(NullifyFinalize::new(
                nullify,
                finalize.clone(),
            )));
        }
        self.nullifies.entry(key).or_insert(nullify);
        None
    }

    /// Every proposal-bearing vote held for `round`.
    ///
    /// Nullifies are left out on purpose: a nullify carries no proposal, so it
    /// can only pair with a finalize, and the node holding that finalize
    /// publishes it from here. Publishing both sides would double the traffic
    /// and pair nothing extra.
    fn round_votes(&self, round: Round) -> EvidenceBatch {
        let (lo, hi) = round_key_span(round);
        self.notarizes
            .range(lo..=hi)
            .map(|(_, n)| Vote::Notarize(n.clone()))
            .chain(
                self.finalizes
                    .range(lo..=hi)
                    .map(|(_, f)| Vote::Finalize(f.clone())),
            )
            .collect()
    }

    /// The votes held for `proposal`'s round that back a DIFFERENT proposal.
    /// The certificate settled the round, so each of these is one half of an
    /// equivocation whose partner is on some other node.
    fn votes_against(&self, proposal: &Proposal<Digest>) -> EvidenceBatch {
        let (lo, hi) = round_key_span(proposal.round);
        self.notarizes
            .range(lo..=hi)
            .filter(|(_, n)| n.proposal != *proposal)
            .map(|(_, n)| Vote::Notarize(n.clone()))
            .chain(
                self.finalizes
                    .range(lo..=hi)
                    .filter(|(_, f)| f.proposal != *proposal)
                    .map(|(_, f)| Vote::Finalize(f.clone())),
            )
            .collect()
    }

    /// Raise the view floor. Only a finalization inside the store's current
    /// epoch moves it — a view number from another epoch is not comparable.
    fn note_finalized(&mut self, epoch: u64, view: u64) {
        if epoch == self.floor_epoch {
            self.floor = self.floor.max(view);
        }
    }

    /// House pattern: explicit floor-retain (cf. `epoch_manager.rs`,
    /// `cert_inlet.rs`) — but those key on `epoch`, which is monotonic. View
    /// numbers restart per epoch, so the two components have to be composed:
    /// the view floor is reset when the epoch turns, and the epoch bound is
    /// what finally evicts an entry a restarted view floor can no longer reach.
    ///
    /// **The one-epoch grace is load-bearing, not slack.** Pruning at exactly
    /// `current_epoch` would discard a vote half at the instant the epoch
    /// turns, including one whose conflicting partner is still in flight —
    /// nothing orders that round trip against the epoch counter. Such a pair
    /// can no longer become an in-block charge, but it can still become a
    /// fallback transaction, and an attacker already controls delivery timing
    /// and would simply equivocate at a boundary to lose it on both routes at
    /// once.
    fn retain_floor(&mut self, current_epoch: u64) {
        if current_epoch > self.floor_epoch {
            self.floor_epoch = current_epoch;
            self.floor = 0;
        }
        let keep = self.floor.saturating_sub(RETAIN_VIEWS);
        let oldest = current_epoch.saturating_sub(1);
        let alive = |&(epoch, view, _): &VoteKey| epoch >= oldest && view >= keep;
        self.notarizes.retain(|key, _| alive(key));
        self.finalizes.retain(|key, _| alive(key));
        self.nullifies.retain(|key, _| alive(key));
    }
}

/// Local cryptographic gate for a charge, ALWAYS vote-only (bug 4). On-chain
/// `_slashEquivocation` accepts evidence over the ATTRIBUTABLE VOTE HALF only
/// (the threshold seed partial is non-attributable and dropped —
/// `evidence.rs`), so verifying exactly that half against a `VoteScheme`
/// verifier rebuilt from the recovered `committee.bimap` is correct and
/// sufficient. The prior asymmetry — full-verify via the registered
/// `CombinedScheme` when `scoped()` returned `Some` — WRONGLY rejected a
/// genuine seeded Notarize/Finalize equivocation whenever that scheme was
/// verifier-flavored (`beacon = None`): `verify_attestation`'s
/// `_ => combined.seed.is_none()` arm rejects a present seed. Vote-only adds
/// nothing for submission even when the full scheme is available (the seed
/// never reaches the chain), and rebuilding the verifier from the committee's
/// own `bimap` means the gate depends on no registered per-epoch scheme:
/// `resolve_committee` reads the frozen committee for ANY epoch off the latest
/// finalized head, so a charge stays verifiable after the local schemes for its
/// epoch have been pruned.
///
/// Structural invariants are NOT re-checked here — `Conflicting*::new`
/// asserted them at assembly.
fn verify_charge(
    charge: &Message,
    committee: &EpochCommittee,
    chain_id: u64,
) -> Result<(), fluentbase_bls::Error> {
    let vote_scheme = VoteScheme::verifier(&fluent_namespace(chain_id), committee.bimap.clone());
    verify_pre_submit_vote_only(charge, &vote_scheme, &mut OsRng)
}

/// The accused rides in `extra_data` as a single byte and a committee index is
/// always below `MAX_COMMITTEE_SIZE`, so a wider index means the committee
/// itself diverged rather than that the byte is too narrow.
fn accused_index(signer_idx: u32) -> Result<u8, HandleError> {
    u8::try_from(signer_idx).map_err(|_| {
        HandleError::permanent(format!(
            "signer_idx {signer_idx} exceeds the {MAX_COMMITTEE_SIZE}-member committee bound"
        ))
    })
}

/// **** Actor не лучшее название, очень просто в них потеряться если их несколько п о проекту
pub struct Actor<E, R>
where
    E: Clock + Metrics + Spawner + Storage + Send + 'static,
    R: StakingStateRead + Send + Sync + 'static,
{
    context: ContextCell<E>,
    mailbox_rx: mpsc::UnboundedReceiver<Envelope>,
    staking_address: Address,
    chain_id: u64,
    reader: R,
    latest_finalized_hash: LatestFinalizedHash,
    sink: Arc<dyn SlasherTxSink>,
    wal_writer: wal_queue::Writer<E, Vec<u8>>,
    /// WAL reader; consumer side. `Option` so it can be moved into the
    /// consumer task on `start()`.
    wal_reader: Option<wal_queue::Reader<E, Vec<u8>>>,
    /// Dedup: victim addresses where an attempt has been observed on-chain
    /// this session. Producer checks before enqueue; consumer populates after
    /// `Mined`/`Reverted` outcome.
    submitted_this_session: Arc<TokioMutex<HashSet<Address>>>,
    votes: VoteStore,
    /// Shared with the proposer — see [`ChargeStore`].
    charges: ChargeStore,
    /// Highest epoch the LOCAL ENGINE has reported an activity for. A charge for
    /// anything below it can no longer enter a block, which is what
    /// [`Self::drain_stale_charges`] acts on. Shared with the evidence consumer,
    /// which bounds a forwarded batch against it — see [`EpochCursor`].
    epoch_cursor: EpochCursor,
    /// Outbound half of the evidence channel — see [`Actor::republish`].
    evidence: Option<EvidenceBridge>,
}

impl<E, R> Actor<E, R>
where
    E: Clock + Metrics + Spawner + Storage + Send + Sync + 'static,
    R: StakingStateRead + Send + Sync + 'static,
{
    pub fn init(context: E, cfg: Config<R, E>) -> (Self, Mailbox) {
        let (tx, rx) = mpsc::unbounded_channel();
        let mailbox = Mailbox::new(tx);
        // Late-bind the inbound direction: the node's evidence task was spawned
        // before the consensus layer launched, so this mailbox is the first
        // thing it can be given.
        if let Some(bridge) = &cfg.evidence {
            bridge.bind_slasher(&mailbox);
        }
        // The cursor lives on the bridge because that is the seam the consumer
        // reads it through. With no bridge there is no evidence channel and
        // nobody but this actor ever reads it, so a private one is the same
        // object with a shorter reach.
        let epoch_cursor = cfg
            .evidence
            .as_ref()
            .map(|bridge| bridge.epoch_cursor().clone())
            .unwrap_or_default();

        let actor = Self {
            context: ContextCell::new(context),
            mailbox_rx: rx,
            staking_address: cfg.staking_address,
            chain_id: cfg.chain_id,
            reader: cfg.reader,
            latest_finalized_hash: cfg.latest_finalized_hash,
            sink: cfg.sink,
            wal_writer: cfg.wal_writer,
            wal_reader: Some(cfg.wal_reader),
            submitted_this_session: Arc::new(TokioMutex::new(HashSet::new())),
            votes: VoteStore::default(),
            charges: cfg.charges,
            epoch_cursor,
            evidence: cfg.evidence,
        };
        (actor, mailbox)
    }

    pub fn start(mut self) -> Handle<()> {
        // Spawn the consumer first so the WAL has a reader ready before any
        // producer enqueues fire. The consumer's `Handle` is detached: when
        // the producer exits (mailbox closed) it drops the writer, the
        // consumer's `recv()` then returns `None` after draining, and the
        // consumer task exits naturally.
        let reader = self
            .wal_reader
            .take()
            .expect("wal_reader present at start (Actor::init seeds Some)");
        let sink = self.sink.clone();
        let staking_address = self.staking_address;
        let submitted = self.submitted_this_session.clone();
        let consumer_ctx = self.context.with_label("slasher_consumer");
        let _consumer_handle = consumer_ctx.spawn(move |_| async move {
            run_consumer(reader, sink, staking_address, submitted).await;
        });

        spawn_cell!(self.context, self.run_producer().await)
    }

    async fn run_producer(mut self) {
        info!("slasher producer starting");
        // Transient-error retry buffer: a `handle` failure on a TRANSIENT cause
        // (no finalized hash yet at startup, RPC Err, scheme/committee not yet
        // registered for the evidence epoch) must NOT lose the Activity —
        // simplex reports a conflict exactly once and there is no replay path.
        // Re-attempt with a short backoff before pulling the next mailbox item,
        // up to a bound; only a PERMANENT failure (malformed/variant-mismatch
        // evidence, BiMap divergence) is dropped.
        let mut retry: Option<(Envelope, u32)> = None;
        loop {
            if let Some((entry, attempts)) = retry.take() {
                self.context.sleep(SLASHER_RETRY_BACKOFF).await;
                match self.handle(entry.clone()).await {
                    Ok(()) => {}
                    Err(HandleError::Permanent(e)) => {
                        warn!(
                            ?e,
                            "slasher producer handle failed permanently; dropping evidence"
                        );
                    }
                    Err(HandleError::Transient(e)) => {
                        if attempts + 1 >= SLASHER_MAX_RETRIES {
                            error!(
                                ?e,
                                attempts = attempts + 1,
                                "slasher producer exhausted retries; dropping slashable evidence"
                            );
                            metrics::counter!("slasher_evidence_dropped_total").increment(1);
                        } else {
                            debug!(
                                ?e,
                                attempts = attempts + 1,
                                "slasher producer transient failure; will retry"
                            );
                            retry = Some((entry, attempts + 1));
                        }
                    }
                }
                continue;
            }
            let Some(entry) = self.mailbox_rx.recv().await else {
                break;
            };
            match self.handle(entry.clone()).await {
                Ok(()) => {}
                Err(HandleError::Permanent(e)) => {
                    warn!(
                        ?e,
                        "slasher producer handle failed permanently; dropping evidence"
                    );
                }
                Err(HandleError::Transient(e)) => {
                    debug!(?e, "slasher producer transient failure; will retry");
                    retry = Some((entry, 0));
                }
            }
        }
        info!("slasher producer exiting");
    }

    /// Resolve an epoch's committee on-chain at the latest finalized block. The
    /// snapshot is returned alongside the typed committee because victim
    /// resolution needs the validator addresses the BiMap does not carry.
    ///
    /// There is no second source. This used to fall back to a durable local
    /// cache when the on-chain array read empty, because the contract pruned old
    /// committees. It does not any more, so an empty array means the epoch was
    /// never committed — and the cache could not have answered that either: it
    /// was written from a FINALIZED snapshot, so a hit implied this node's
    /// finalized head was already at or past the commit, at which head the
    /// on-chain read is not empty. Finalized heads never move backwards, so the
    /// fallback could not change an outcome in any reachable state.
    async fn resolve_committee(
        &self,
        epoch: u64,
    ) -> Result<(ValidatorSetSnapshot, EpochCommittee), HandleError> {
        // No finalized hash yet (startup) is transient — the next finalization
        // supplies one and the buffered Activity retries.
        let head = (self.latest_finalized_hash)()
            .ok_or_else(|| HandleError::transient("no latest finalized hash available"))?;
        let snap = match self.reader.epoch_committee_snapshot(epoch, head) {
            Ok(s) if !s.validators.is_empty() => s,
            Ok(_) => {
                return Err(HandleError::permanent(format!(
                    "epoch {epoch} evidence: no committee was ever committed for that epoch; \
                     this evidence is unrecoverable"
                )))
            }
            // An on-chain read error (RPC / state lookup) is transient.
            Err(e) => {
                return Err(HandleError::transient(format!(
                    "committee read failed: {e:?}"
                )))
            }
        };
        let committee = epoch_committee_from_snapshot(&snap).map_err(|e| {
            HandleError::permanent(format!("epoch_committee_from_snapshot failed: {e:?}"))
        })?;
        Ok((snap, committee))
    }

    /// Broadcast the votes this node personally received for a round that has
    /// just been decided against them, so the peers holding the other half of a
    /// split-delivered equivocation can pair them.
    ///
    /// Best-effort by design. A committee that will not resolve (startup, no
    /// finalized hash yet) costs this round's publication and nothing else — the
    /// votes stay in the store and the next trigger publishes them. Propagating
    /// the failure instead would re-run the whole `handle`, whose WAL enqueue is
    /// not idempotent.
    async fn republish(&self, epoch: u64, votes: EvidenceBatch) {
        let Some(bridge) = self.evidence.as_ref() else {
            return;
        };
        let committee = match self.resolve_committee(epoch).await {
            Ok((_, committee)) => committee,
            Err(HandleError::Transient(e) | HandleError::Permanent(e)) => {
                debug!(
                    ?e,
                    epoch, "evidence: committee unresolved; not republishing"
                );
                return;
            }
        };
        let vote_scheme =
            VoteScheme::verifier(&fluent_namespace(self.chain_id), committee.bimap.clone());
        // Nothing here has been verified yet — simplex reports votes before its
        // batcher checks them — and a bogus signature spread under an honest
        // validator's name is exactly what this channel must not carry.
        let verified: EvidenceBatch = votes
            .into_iter()
            .filter(|vote| verify_vote(vote, &vote_scheme))
            .collect();
        if verified.is_empty() {
            return;
        }
        metrics::counter!("slasher_evidence_published_total").increment(1);
        debug!(
            epoch,
            votes = verified.len(),
            "evidence: republishing votes"
        );
        bridge.publish(encode_batch(&verified));
    }

    /// Verify a candidate charge and route it: hold it for a proposal while its
    /// epoch is still running, hand it to the transaction fallback once it is
    /// not.
    async fn hold_charge(&mut self, charge: Message) -> Result<(), HandleError> {
        let epoch = charge.epoch().get();
        let signer_idx = attributable_signer_idx(&charge)
            .ok_or_else(|| HandleError::permanent("charge carries no attributable signer"))?;
        let accused = accused_index(signer_idx)?;
        // Skip the committee resolve and the two pairings when the same
        // equivocator is already charged for this epoch.
        if self.charges.contains(epoch, accused) {
            return Ok(());
        }
        let (snap, committee) = self.resolve_committee(epoch).await?;
        verify_charge(&charge, &committee, self.chain_id).map_err(|e| {
            HandleError::permanent(format!("charge failed vote-only verify: {e:?}"))
        })?;
        // [`VoteStore`] keeps one epoch of grace, so a half arriving just after
        // a boundary can still pair — into a charge for an epoch that is
        // already past. Queueing that for a block it can never enter would
        // strand it forever, the drain for its epoch having already run. The
        // grace exists for exactly this pair; send it the only way still open.
        if epoch < self.epoch_cursor.get() {
            return self.enqueue_fallback(&charge, &snap, &committee).await;
        }
        self.hold_verified_charge(epoch, accused, charge);
        Ok(())
    }

    /// Hand every charge queued for an epoch below `epoch` to the transaction
    /// sink, because the epoch turn just took the block route away from them.
    ///
    /// Best-effort, and deliberately not propagating: a charge whose committee
    /// will not resolve yet stays in the store and is drained again at the next
    /// epoch turn — the same event, no timer — whereas propagating would park
    /// the producer's mailbox behind up to [`SLASHER_MAX_RETRIES`] backoffs for
    /// a charge that has nothing to do with the activity being handled.
    async fn drain_stale_charges(&mut self, epoch: u64) {
        for (key, charge) in self.charges.stale(epoch) {
            let (charged_epoch, accused) = key;
            let outcome = match self.resolve_committee(charged_epoch).await {
                Ok((snap, committee)) => self.enqueue_fallback(&charge, &snap, &committee).await,
                Err(e) => Err(e),
            };
            match outcome {
                Ok(()) => self.charges.release(key),
                // Nothing can make these bytes submittable, so releasing is what
                // stops the drain re-attempting them at every later turn.
                Err(HandleError::Permanent(e)) => {
                    warn!(
                        ?e,
                        charged_epoch, accused, "stale charge unsubmittable; dropping"
                    );
                    self.charges.release(key);
                }
                Err(HandleError::Transient(e)) => {
                    debug!(
                        ?e,
                        charged_epoch,
                        accused,
                        "stale charge not submittable yet; retrying at the next epoch turn"
                    );
                }
            }
        }
    }

    /// The transaction route: ABI-encode the charge and put it on the WAL the
    /// consumer drains.
    ///
    /// Where the block route has the committee verify at vote time, here the
    /// contract verifies the evidence in the calldata — which is what lets it
    /// work for an epoch no live committee can speak for.
    async fn enqueue_fallback(
        &mut self,
        charge: &Message,
        snap: &ValidatorSetSnapshot,
        committee: &EpochCommittee,
    ) -> Result<(), HandleError> {
        let kind = SlashKind::from_activity(charge)
            .ok_or_else(|| HandleError::permanent("charge is not a slashable variant"))?;
        let args: SlashCallArgs = match (kind, charge) {
            (SlashKind::ConflictingNotarize, Activity::ConflictingNotarize(ev)) => {
                extract_from_conflicting_notarize(ev, committee)
                    .map_err(|e| HandleError::permanent(format!("{e:?}")))?
            }
            (SlashKind::ConflictingFinalize, Activity::ConflictingFinalize(ev)) => {
                extract_from_conflicting_finalize(ev, committee)
                    .map_err(|e| HandleError::permanent(format!("{e:?}")))?
            }
            (SlashKind::NullifyFinalize, Activity::NullifyFinalize(ev)) => {
                extract_from_nullify_finalize(ev, committee)
                    .map_err(|e| HandleError::permanent(format!("{e:?}")))?
            }
            // `SlashKind::from_activity` already filtered to a slashable variant,
            // so this is unreachable today; degrade gracefully (log + skip) rather
            // than panic the accountability actor if a future variant desyncs
            // `from_activity` from this match.
            _ => {
                return Err(HandleError::permanent(format!(
                    "SlashKind/Activity variant mismatch ({kind:?}); skipping"
                )))
            }
        };

        let signer_idx = attributable_signer_idx(charge).ok_or_else(|| {
            HandleError::permanent("Activity variant carries no slashable signer; skipping")
        })?;
        let signer_peer = committee.bimap.get(signer_idx as usize).ok_or_else(|| {
            HandleError::permanent(format!("signer_idx {signer_idx} not in BiMap"))
        })?;
        let victim = snap
            .validators
            .iter()
            .find(|v| &v.keys.peer_pubkey == signer_peer)
            .ok_or_else(|| {
                HandleError::permanent(format!(
                    "BiMap-resolved peer pubkey not in snapshot — \
                     contract / BiMap ordering divergence; signer_idx={signer_idx}"
                ))
            })?
            .address;
        tracing::Span::current().record("victim", tracing::field::display(victim));

        // Skip enqueue if a Mined/Reverted outcome already observed
        // this session (preserves the existing dedup behaviour). The
        // consumer populates `submitted_this_session` after a non-Failed
        // outcome.
        {
            let dedup = self.submitted_this_session.lock().await;
            if dedup.contains(&victim) {
                debug!(%victim, "already slashed this session; skipping enqueue");
                return Ok(());
            }
        }

        let calldata = encode_calldata(&args);
        let payload = encode_wal_payload(victim, &calldata);
        let pos = self
            .wal_writer
            .enqueue(payload)
            .await
            .map_err(|e| HandleError::transient(format!("WAL enqueue failed: {e:?}")))?;
        debug!(%victim, pos, "enqueued slash evidence to WAL");
        metrics::counter!("slasher_wal_enqueued_total", "kind" => kind_label(kind)).increment(1);
        Ok(())
    }

    fn hold_verified_charge(&mut self, epoch: u64, accused: u8, charge: Message) {
        if self.charges.hold(epoch, accused, charge) {
            metrics::counter!("slasher_charges_held_total").increment(1);
            info!(epoch, accused, "holding verified equivocation charge");
        }
    }

    #[instrument(skip_all, fields(kind, epoch, victim))]
    async fn handle(&mut self, entry: Envelope) -> Result<(), HandleError> {
        let Envelope {
            activity,
            provenance,
        } = entry;
        let epoch = activity.epoch().get();
        // The epoch turning is what strands a charge: from here on no committee
        // can verify one for an older epoch from the block in front of it, so
        // whatever is still queued has only the transaction route left. Draining
        // first also means a pair assembled further down from a half that
        // arrived late takes that route rather than joining a queue whose drain
        // has already run.
        //
        // ONLY the local engine may move the cursor. A peer-forwarded vote names
        // an epoch of its signer's choosing, so honouring it here would let one
        // member of a future committee flush the vote store and push every live
        // charge onto the transaction route at will — repeatedly, since the
        // cursor is monotone. See [`Provenance`].
        if provenance == Provenance::Engine && epoch > self.epoch_cursor.get() {
            self.epoch_cursor.advance(epoch);
            self.drain_stale_charges(epoch).await;
        }
        // Every vote reaches this mailbox and the filter below used to throw the
        // non-slashable ones away. A split-delivered equivocation is visible
        // ONLY as two halves that never meet on the cert path, so the halves are
        // kept here and paired as they arrive.
        let assembled = match &activity {
            Activity::Notarize(n) => self.votes.remember_notarize(n.clone()),
            Activity::Finalize(f) => self.votes.remember_finalize(f.clone()),
            Activity::Nullify(n) => self.votes.remember_nullify(n.clone()),
            Activity::Finalization(f) => {
                self.votes.note_finalized(epoch, f.view().get());
                None
            }
            _ => None,
        };
        // Retain around the CURSOR, not around the activity's own epoch: a
        // forwarded vote's epoch is not this node's clock, and even an
        // out-of-order engine activity would otherwise widen the window rather
        // than hold it.
        self.votes.retain_floor(self.epoch_cursor.get());
        if let Some(charge) = assembled {
            self.hold_charge(charge).await?;
        }

        // The two moments a round stops being able to resolve locally, and the
        // votes held for it become worth putting in front of the rest of the
        // network.
        let republish = match &activity {
            // The view failed with no notarization, so nothing local will ever
            // explain the proposals this node was shown for it. If the leader
            // split its proposal, these are one side of the split; if it merely
            // ran late, they are a handful of identical votes and cost a
            // message.
            Activity::Nullification(nullification) => self.votes.round_votes(nullification.round),
            // A certificate settled the round, and a held vote disagrees with
            // it — one half of an equivocation, its partner on another node.
            Activity::Notarization(notarization) => {
                self.votes.votes_against(&notarization.proposal)
            }
            _ => EvidenceBatch::new(),
        };
        if !republish.is_empty() {
            self.republish(epoch, republish).await;
        }

        let Some(kind) = SlashKind::from_activity(&activity) else {
            return Ok(()); // not slashable on its own — the vote store kept it
        };
        tracing::Span::current().record("kind", tracing::field::debug(kind));
        tracing::Span::current().record("epoch", epoch);

        // A conflict simplex witnessed for us is a charge like any other, and
        // takes the same two routes an assembled one does.
        self.hold_charge(activity).await
    }
}

/// Consumer task: dequeues `(victim || calldata)` payloads from the WAL,
/// hands them to the sink, and acks based on the outcome.
async fn run_consumer<E>(
    mut reader: wal_queue::Reader<E, Vec<u8>>,
    sink: Arc<dyn SlasherTxSink>,
    staking_address: Address,
    submitted: Arc<TokioMutex<HashSet<Address>>>,
) where
    E: Clock + Metrics + Spawner + Storage + Send + 'static,
{
    info!("slasher consumer starting");
    loop {
        match reader.recv().await {
            Ok(None) => break, // writer dropped — drain complete
            Ok(Some((pos, payload))) => {
                let (victim, calldata) = match decode_wal_payload(&payload) {
                    Ok(v) => v,
                    Err(e) => {
                        error!(pos, ?e, "WAL payload decode failed; acking to drop");
                        if let Err(ack_err) = reader.ack(pos).await {
                            error!(pos, ?ack_err, "WAL ack of malformed entry failed");
                        }
                        continue;
                    }
                };
                let outcome = sink.submit(staking_address, Bytes::from(calldata)).await;
                match outcome {
                    SubmitOutcome::Mined { tx_hash } => {
                        info!(%victim, %tx_hash, "slash mined");
                        metrics::counter!("slasher_submitted_total").increment(1);
                        submitted.lock().await.insert(victim);
                        if let Err(e) = reader.ack(pos).await {
                            error!(pos, ?e, "WAL ack failed after Mined");
                        }
                    }
                    SubmitOutcome::AlreadySlashed => {
                        // Pre-flight sim confirmed the victim is already
                        // tombstoned — goal achieved, ack without a tx.
                        info!(%victim, "victim already tombstoned (pre-flight); acking");
                        metrics::counter!("slasher_already_slashed_total").increment(1);
                        submitted.lock().await.insert(victim);
                        if let Err(e) = reader.ack(pos).await {
                            error!(pos, ?e, "WAL ack failed after AlreadySlashed");
                        }
                    }
                    SubmitOutcome::Failed(msg) => {
                        // Do NOT ack — entry re-delivered only after a restart
                        // (no automatic in-session retry). A simulated revert
                        // here is a deterministic bug (calldata/EIP-2537
                        // encoding) — alert; retrying the same bytes won't help.
                        error!(%victim, %msg, "slash submission/simulation failed");
                        metrics::counter!("slasher_submit_failed_total").increment(1);
                    }
                }
            }
            Err(e) => {
                error!(?e, "WAL recv failed; consumer exiting");
                break;
            }
        }
    }
    info!("slasher consumer exiting");
}

/// Convenience constructor: initialize the WAL queue under the slasher's
/// own context label. Called from `outer.rs::build` (async); the returned
/// `(Writer, Reader)` is passed into [`Config`].
pub async fn init_wal_queue<E>(
    context: E,
    partition: String,
) -> Result<(wal_queue::Writer<E, Vec<u8>>, wal_queue::Reader<E, Vec<u8>>), eyre::Report>
where
    E: Clock
        + Metrics
        + Spawner
        + Storage
        + commonware_runtime::BufferPooler
        + Send
        + Sync
        + 'static,
{
    use commonware_runtime::buffer::paged::CacheRef;
    use commonware_storage::queue::Config as QueueConfig;
    use commonware_utils::{NZUsize, NZU16, NZU64};

    let page_cache = CacheRef::from_pooler(&context, NZU16!(4096), NZUsize!(64));
    let cfg = QueueConfig {
        partition,
        // One section per ~256 slash events; pruning is exact at section
        // granularity (slashing is rare so granularity is uncritical).
        items_per_section: NZU64!(256),
        compression: None,
        // Vec<u8> codec: open-ended length range + unit cfg for u8.
        codec_config: ((0..).into(), ()),
        page_cache,
        write_buffer: NZUsize!(1 << 16),
    };

    let (writer, reader) = wal_queue::init::<_, Vec<u8>>(context, cfg)
        .await
        .map_err(|e| eyre::eyre!("queue::shared::init failed: {e:?}"))?;
    Ok((writer, reader))
}

fn kind_label(kind: SlashKind) -> &'static str {
    match kind {
        SlashKind::ConflictingNotarize => "conflicting_notarize",
        SlashKind::ConflictingFinalize => "conflicting_finalize",
        SlashKind::NullifyFinalize => "nullify_finalize",
    }
}

/// ABI-encode one slash charge into the calldata the sink submits.
///
/// Public so the conformance tests can pin THIS encoder — the production one —
/// against literal selectors and literal calldata, instead of re-declaring the
/// ABI privately and checking it against itself.
pub fn encode_calldata(args: &SlashCallArgs) -> Vec<u8> {
    let evidence = Bytes::from(args.evidence.clone());
    let pk = Bytes::from(args.pk_uncompressed.to_vec());
    let s1 = Bytes::from(args.sig1_uncompressed.to_vec());
    let s2 = Bytes::from(args.sig2_uncompressed.to_vec());
    match args.kind {
        SlashKind::ConflictingNotarize => slashEquivocationNotarizeCall {
            evidence,
            pkUncompressed: pk,
            sig1Uncompressed: s1,
            sig2Uncompressed: s2,
        }
        .abi_encode(),
        SlashKind::ConflictingFinalize => slashEquivocationFinalizeCall {
            evidence,
            pkUncompressed: pk,
            sig1Uncompressed: s1,
            sig2Uncompressed: s2,
        }
        .abi_encode(),
        SlashKind::NullifyFinalize => slashEquivocationNullifyFinalizeCall {
            evidence,
            pkUncompressed: pk,
            sig1Uncompressed: s1,
            sig2Uncompressed: s2,
        }
        .abi_encode(),
    }
}

/// WAL payload format: 20 bytes victim address || N bytes ABI-encoded calldata.
fn encode_wal_payload(victim: Address, calldata: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(20 + calldata.len());
    out.extend_from_slice(victim.as_slice());
    out.extend_from_slice(calldata);
    out
}

fn decode_wal_payload(bytes: &[u8]) -> Result<(Address, Vec<u8>), eyre::Report> {
    if bytes.len() < 20 {
        return Err(eyre::eyre!(
            "WAL payload too short: {} bytes (need >= 20)",
            bytes.len()
        ));
    }
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&bytes[..20]);
    Ok((Address::from(addr), bytes[20..].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_codec::DecodeExt as _;
    use commonware_consensus::{
        simplex::types::Proposal,
        types::{Epoch, View},
    };
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer};
    use commonware_math::algebra::Random;
    use commonware_utils::{ordered::BiMap, TryCollect};
    use fluentbase_bls::{keys::ValidatorBlsKeypair, scheme::build_signer, BlsPubkey, PeerPubkey};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;

    const TEST_CHAIN_ID: u64 = 20_994;
    const TEST_EPOCH: u64 = 7;

    fn make_address(byte: u8) -> Address {
        let mut a = [0u8; 20];
        a[0] = byte;
        Address::from(a)
    }

    fn test_committee(seed: u64, n: usize) -> (Vec<ValidatorBlsKeypair>, EpochCommittee) {
        let mut rng = StdRng::seed_from_u64(seed);
        let peer_sks: Vec<_> = (0..n)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let bls_kps: Vec<_> = (0..n)
            .map(|_| ValidatorBlsKeypair::generate(&mut rng))
            .collect();
        let bimap: BiMap<PeerPubkey, BlsPubkey> = peer_sks
            .iter()
            .zip(bls_kps.iter())
            .map(|(p, b)| {
                (
                    p.public_key(),
                    BlsPubkey::decode(b.public_bytes().as_slice()).unwrap(),
                )
            })
            .try_collect()
            .unwrap();
        (bls_kps, EpochCommittee::from_unverified(TEST_EPOCH, bimap))
    }

    /// A signer BOUND to `epoch`: a scheme refuses a subject from any other, so
    /// a test that spans epochs needs one per epoch.
    fn offender_signer_at(
        kps: &[ValidatorBlsKeypair],
        committee: &EpochCommittee,
        epoch: u64,
    ) -> BlsScheme {
        build_signer(
            &fluent_namespace(TEST_CHAIN_ID),
            committee.bimap.clone(),
            &kps[0],
            epoch,
            None,
        )
        .expect("offender must be a committee member")
    }

    fn offender_signer(kps: &[ValidatorBlsKeypair], committee: &EpochCommittee) -> BlsScheme {
        offender_signer_at(kps, committee, TEST_EPOCH)
    }

    fn test_round(epoch: u64, view: u64) -> Round {
        Round::new(Epoch::new(epoch), View::new(view))
    }

    fn proposal(round: Round, tag: u8) -> Proposal<Digest> {
        Proposal::new(
            round,
            View::new(round.view().get().saturating_sub(1)),
            Digest(B256::repeat_byte(tag)),
        )
    }

    fn notarize(signer: &BlsScheme, round: Round, tag: u8) -> Notarize<BlsScheme, Digest> {
        Notarize::sign(signer, proposal(round, tag)).expect("offender signs")
    }

    #[test]
    fn conflicting_notarizes_delivered_separately_assemble_the_extractable_charge() {
        let (kps, committee) = test_committee(1, 4);
        let signer = offender_signer(&kps, &committee);
        let round = test_round(TEST_EPOCH, 42);
        let first = notarize(&signer, round, 0xaa);
        let second = notarize(&signer, round, 0xbb);

        let mut store = VoteStore::default();
        assert!(store.remember_notarize(first.clone()).is_none());
        let charge = store
            .remember_notarize(second.clone())
            .expect("the second half must pair with the first");

        verify_charge(&charge, &committee, TEST_CHAIN_ID)
            .expect("a charge assembled from two real votes must verify");

        let Activity::ConflictingNotarize(assembled) = &charge else {
            panic!("two conflicting notarizes must assemble a ConflictingNotarize");
        };
        let split = extract_from_conflicting_notarize(assembled, &committee).unwrap();
        let paired =
            extract_from_conflicting_notarize(&ConflictingNotarize::new(first, second), &committee)
                .unwrap();
        assert_eq!(split.evidence, paired.evidence);
        assert_eq!(split.pk_uncompressed, paired.pk_uncompressed);
        assert_eq!(split.sig1_uncompressed, paired.sig1_uncompressed);
        assert_eq!(split.sig2_uncompressed, paired.sig2_uncompressed);
    }

    #[test]
    fn a_repeated_vote_for_the_same_proposal_is_not_a_charge() {
        let (kps, committee) = test_committee(2, 4);
        let signer = offender_signer(&kps, &committee);
        let round = test_round(TEST_EPOCH, 42);

        let mut store = VoteStore::default();
        assert!(store
            .remember_notarize(notarize(&signer, round, 0xaa))
            .is_none());
        assert!(store
            .remember_notarize(notarize(&signer, round, 0xaa))
            .is_none());
    }

    #[test]
    fn a_nullify_and_a_finalize_for_one_round_assemble_a_charge_in_either_order() {
        let (kps, committee) = test_committee(3, 4);
        let signer = offender_signer(&kps, &committee);
        let round = test_round(TEST_EPOCH, 42);
        let nullify = Nullify::sign::<Digest>(&signer, round).expect("offender signs");
        let finalize = Finalize::sign(&signer, proposal(round, 0xaa)).expect("offender signs");

        let mut nullify_first = VoteStore::default();
        assert!(nullify_first.remember_nullify(nullify.clone()).is_none());
        let charge = nullify_first
            .remember_finalize(finalize.clone())
            .expect("the finalize must pair with the held nullify");
        verify_charge(&charge, &committee, TEST_CHAIN_ID).expect("assembled charge must verify");
        assert!(matches!(charge, Activity::NullifyFinalize(_)));

        let mut finalize_first = VoteStore::default();
        assert!(finalize_first.remember_finalize(finalize).is_none());
        let charge = finalize_first
            .remember_nullify(nullify)
            .expect("the nullify must pair with the held finalize");
        verify_charge(&charge, &committee, TEST_CHAIN_ID).expect("assembled charge must verify");
        assert!(matches!(charge, Activity::NullifyFinalize(_)));
    }

    #[test]
    fn retain_floor_composes_the_epoch_bound_with_the_view_window() {
        let (kps, committee) = test_committee(4, 4);

        let mut store = VoteStore::default();
        for round in [
            test_round(4, 300),
            test_round(5, 10),
            test_round(5, 199),
            test_round(5, 200),
        ] {
            let signer = offender_signer_at(&kps, &committee, round.epoch().get());
            store.remember_notarize(notarize(&signer, round, 0xaa));
        }
        store.retain_floor(5);
        store.note_finalized(5, 200);
        store.retain_floor(5);

        // floor 200 − RETAIN_VIEWS 64 = 136: view 10 is outside the window, the
        // prior epoch is inside the one-epoch grace.
        let views_left: Vec<_> = store.notarizes.keys().map(|&(e, v, _)| (e, v)).collect();
        assert_eq!(views_left, vec![(4, 300), (5, 199), (5, 200)]);

        // The next epoch turn resets the view floor and the grace expires for
        // epoch 4, which no restarted view number could have evicted.
        store.retain_floor(6);
        let views_left: Vec<_> = store.notarizes.keys().map(|&(e, v, _)| (e, v)).collect();
        assert_eq!(views_left, vec![(5, 199), (5, 200)]);
    }

    #[test]
    fn encode_calldata_dispatches_on_kind() {
        let args = SlashCallArgs {
            kind: SlashKind::ConflictingNotarize,
            evidence: vec![0xAA, 0xBB],
            pk_uncompressed: [0xCC; fluentbase_bls::PUBKEY_EIP2537_BYTES],
            sig1_uncompressed: [0xDD; fluentbase_bls::SIGNATURE_EIP2537_BYTES],
            sig2_uncompressed: [0xEE; fluentbase_bls::SIGNATURE_EIP2537_BYTES],
        };
        let calldata_notarize = encode_calldata(&args);

        let args_fin = SlashCallArgs {
            kind: SlashKind::ConflictingFinalize,
            ..args.clone()
        };
        let calldata_finalize = encode_calldata(&args_fin);

        let args_nf = SlashCallArgs {
            kind: SlashKind::NullifyFinalize,
            ..args
        };
        let calldata_nullify_fin = encode_calldata(&args_nf);

        // Each variant must produce a distinct selector (first 4 bytes).
        assert_ne!(&calldata_notarize[..4], &calldata_finalize[..4]);
        assert_ne!(&calldata_finalize[..4], &calldata_nullify_fin[..4]);
        assert_ne!(&calldata_notarize[..4], &calldata_nullify_fin[..4]);
    }

    #[test]
    fn wal_payload_roundtrip() {
        let victim = make_address(0x42);
        let calldata = vec![0xAA, 0xBB, 0xCC];
        let payload = encode_wal_payload(victim, &calldata);
        assert_eq!(payload.len(), 23);
        let (dec_victim, dec_calldata) = decode_wal_payload(&payload).unwrap();
        assert_eq!(dec_victim, victim);
        assert_eq!(dec_calldata, calldata);
    }

    #[test]
    fn wal_payload_decode_rejects_short() {
        let too_short = vec![0u8; 10];
        let err = decode_wal_payload(&too_short).unwrap_err();
        assert!(err.to_string().contains("too short"));
    }
}
