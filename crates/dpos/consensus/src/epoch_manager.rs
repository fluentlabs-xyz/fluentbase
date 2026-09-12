//! Per-epoch consensus engine lifecycle.
//!
//! Owns the active-epochs map and an event-driven boundary trigger
//! (`mpsc::Receiver<Epoch>`) fed by
//! [`fluentbase_staking_reader::EpochTransition`]. The vote/cert/resolver Muxers
//! are NOT owned here — they live in the always-on plane (node crate); this manager
//! receives their `MuxHandle`s per promotion and registers/deregisters per-epoch
//! sub-channels against them.
//!
//! `marshal::core::Actor`, `buffered::Engine`, and the 2
//! `immutable::Archive` instances do **not** pass through here — they live
//! in [`crate::outer::OuterEngine`]. EpochManager threads only the 3
//! simplex broker handles.

use crate::{
    application::{ExecutedChain, FluentApp, OrderingAssembler},
    beacon::agreement_partition,
    beacon::{constant_fallback_seed, witness_fallback_seed},
    beacon::{Beacon, PinEffort, ShareProbe, SignerVerdict},
    committee::Committee,
    engine::{EpochEngine, EpochEngineConfig},
    epocher::OriginEpocher,
    order_block::OrderBlock,
    outer::{EpochSchemeProvider, SharedMux},
    slasher::Mailbox as SlasherMailbox,
    sync_metrics::SyncReason,
    timeouts::ConsensusTimeouts,
    weighted_vrf::WeightedVrf,
    SCHEME_RETENTION_EPOCHS,
};
use commonware_consensus::{
    marshal::{core::Mailbox as MarshalMailbox, standard::Standard},
    types::{Epoch, Epocher as _, Round, View},
};
use commonware_cryptography::ed25519::PublicKey;
use commonware_p2p::{Blocker, Receiver, Sender};
use commonware_runtime::{
    buffer::paged::CacheRef, spawn_cell, BufferPooler, Clock, ContextCell, Handle, Metrics,
    Spawner, Storage,
};
use fluentbase_bls::{keys::ValidatorBlsKeypair, Scheme as BlsScheme};
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use futures::future::BoxFuture;
use prometheus_client::metrics::counter::Counter;
use rand_core::CryptoRngCore;
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::{mpsc, watch, Notify};
use tracing::{debug, error, info, warn};

/// The per-epoch validator role — a pure function of current state, NOT the
/// emergent product of a boundary-coupled transition zoo. A committee member
/// becomes a [`Role::Signer`] the instant it is in `committee[E]` at the live
/// frontier and caught up to the upstream tip — no epoch-boundary wait (the
/// cycle-2 fix; see the `dpos_role_state_binding` plan).
///
/// The decision is BEACON-INDEPENDENT and SYNC-INDEPENDENT: whether a node holds
/// a usable DKG share, and whether the E-1 boundary block has reached the local
/// marshal yet, are SPAWN-time concerns [`Actor::reconcile_roles`] gates
/// SEPARATELY (the share-gate and the `Inline::genesis` precondition) — a
/// `Signer` that holds no share for a beacon-active epoch, or whose boundary
/// block has not yet landed, stays on the verify-only scheme (no participating
/// engine) until both hold, because a shareless Simplex member rejects honest
/// peers' seeded votes and wedges the chain, and the engine `unreachable!`s
/// without its boundary block. Neither is modelled by the role verdict itself.
///
/// "Caught up" is NOT a separate signal: a node reaches the live frontier when
/// `is_live` (`E` is the epoch of its own VERIFIED marshal tip) and its
/// always-on executor has derived the chain up to E-1's boundary (the
/// `Inline::genesis` spawn gate). The validator's executor is the sole reth
/// writer and follows the chain by LOCAL derivation, so no cert-follow plane and
/// no `caught_up` flag are needed on a validator. The membership→role map is
/// `Signer` iff `is_member`, evaluated inline at the sole call site in
/// [`Actor::reconcile_roles`] (the caller only reaches it at the live frontier,
/// so liveness is not a separate input).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Run a participating Simplex engine for the epoch (propose / vote / sign).
    Signer,
    /// Verify-only: follow finalized certs, never propose or sign.
    Verifier,
}

/// Outcome of the `Inline::genesis(E)` precondition lookup on the E-1 terminal
/// block (see [`Actor::boundary_lookup`]), which also supplies the seedless arm's
/// base for epoch E.
#[derive(Debug, PartialEq, Eq)]
enum BoundaryLookup {
    /// No predecessor epoch (epoch 0) or no computable terminal height. Nothing to
    /// wait for and no seed to inherit — every node reaches this identically, so
    /// the constant base stays agreed.
    NotApplicable,
    /// The spawn must be deferred, and WHICH input is missing — the two have
    /// different repair paths and different operator responses, so they are not
    /// one state with one log line.
    Missing(MissingInput),
    /// The block is present AND its epoch's base is decided. `seed` is
    /// [`witness_fallback_seed`] of σ at the terminal ROUND — a one-way
    /// compression of the threshold signature, neither the signature itself nor
    /// the round it signs. `None` is the EPOCH predicate's answer and nothing
    /// else: a beacon-inactive predecessor, where no σ can exist. A local σ MISS
    /// is [`BoundaryLookup::Missing`], not this.
    Present { seed: Option<[u8; 32]> },
}

/// Which of the two `Inline::genesis(E)` inputs the node does not hold.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum MissingInput {
    /// The E-1 terminal block is not in marshal storage. Normally transient
    /// (backfill in flight); persistent means the height sits below the marshal
    /// floor, where no repair path fetches it.
    BoundaryBlock,
    /// The block is in hand but this node holds no σ for E-1's terminal ROUND, so
    /// it cannot compute the leader-election base. Repaired by a certificate for
    /// that round arriving on either cert door — never by fetching the block
    /// again, which is why it must not read as a stuck block fetch.
    TerminalSeed,
}

/// Which base the leader elector's seedless arm gets for an epoch, and why —
/// the variant, not just the bytes, so the caller can meter the predictable case
/// without re-deriving the choice.
#[derive(Debug, PartialEq, Eq)]
enum SeedlessBase {
    /// Inherited from σ of E-1's terminal round: not derivable from constants,
    /// so the epoch's first leader is not known an epoch ahead.
    Witness([u8; 32]),
    /// The constant derivation, reached only where no witness can exist. Agreed
    /// across nodes but predictable.
    Constant([u8; 32]),
}

/// The base for epoch E, read from σ of E-1's TERMINAL ROUND rather than from
/// the terminal block's body.
///
/// The round is named by the CALLER from agreed data — `Round(E-1,
/// terminal.proposal_view)`, both halves of which every node reads off the same
/// committee-signed block — and only then asked of the store, which may not name
/// it (its pin decides what survives eviction, never which round is wanted).
///
/// PREDICATE FIRST, store second. `mandatory_at(E-1)` is the network-agreed "was
/// the beacon active there", independent of anything local, and it decides
/// BEFORE the store is read; a σ sitting at a round the agreed map calls
/// beacon-INACTIVE can therefore never become one node's base while its peers
/// take the constant one.
///
/// A store MISS is [`BoundaryLookup::Missing`] — "I do not know yet", which
/// DEFERS the spawn — and never `Present { seed: None }`, which asserts "there is
/// no σ here" and elects on the constant base. Conflating the two splits the
/// leader schedule: the deferring node re-poked would elect one leader while a
/// node that answered `None` elects another, for the same epoch, from the same
/// agreed block.
fn boundary_base(randomness: &dyn Beacon, prev: Epoch, terminal_view: u64) -> BoundaryLookup {
    if !randomness.mandatory_at(prev.get()) {
        return BoundaryLookup::Present { seed: None };
    }
    let round = Round::new(prev, View::new(terminal_view));
    match randomness.terminal_seed(round) {
        Some(seed) => BoundaryLookup::Present {
            seed: Some(witness_fallback_seed(&seed)),
        },
        None => BoundaryLookup::Missing(MissingInput::TerminalSeed),
    }
}

/// Choose the seedless arm's base from the E-1 terminal-block lookup. `None` for
/// [`BoundaryLookup::Missing`]: that epoch defers its spawn rather than electing
/// anything, so there is no base to choose. Pure — the metric for the predictable
/// case is emitted by the caller.
fn seedless_base(lookup: &BoundaryLookup, snap: &ValidatorSetSnapshot) -> Option<SeedlessBase> {
    match lookup {
        BoundaryLookup::Missing(_) => None,
        BoundaryLookup::Present {
            seed: Some(witness),
        } => Some(SeedlessBase::Witness(*witness)),
        // A beacon-inactive predecessor epoch or no predecessor at all: the
        // constant base is predictable, but no σ exists to do better, and every
        // node takes this branch on the same block.
        BoundaryLookup::NotApplicable | BoundaryLookup::Present { seed: None } => {
            Some(SeedlessBase::Constant(constant_fallback_seed(snap)))
        }
    }
}

/// The counters the epoch manager owns: its spawn decision and its seedless-arm
/// base choice.
///
/// Split out of `BeaconMetrics` with every family name unchanged. They are facts
/// about MEMBERSHIP and the `Inline::genesis` precondition, not about
/// randomness — a `Beacon` implementation that computes the seed from a
/// hash and runs no DKG still produces all four — so keeping them on the beacon
/// struct would have forced the core to name a beacon type in order to count its
/// own spawns.
///
/// Registered ONCE per process against the launch context (`dpos.rs`), on BOTH
/// node classes: `prometheus_client::Registry::register` does not deduplicate,
/// and a family with no owner on a class silently vanishes from that class's
/// scrape.
#[derive(Clone, Debug, Default)]
pub struct EpochEngineMetrics {
    /// A per-epoch engine self-demoted because the operator's validator was rotated
    /// out of the epoch's committee (`RotatedOut`).
    pub engine_demoted_rotated_out: Counter,
    /// A per-epoch engine spawn was deferred because one of the two
    /// `Inline::genesis(E)` inputs is missing: the previous epoch's terminal
    /// BLOCK, or σ for that block's ROUND (the leader-election base). The member
    /// registers verify-only meanwhile: no proposals, no votes. This counter does
    /// not distinguish the two — the deferring INFO line does, and it must be read
    /// to tell them apart, because they have different repair paths. Normally
    /// transient either way. Persistently non-zero across an epoch means, for the
    /// block, that the height sits below the marshal floor where no repair path
    /// fetches it and the boundary seeding at the floor-raise sites did not run;
    /// for the seed, that no certificate for E-1's terminal round is reaching
    /// either cert door — the recorded backfill seed-pin and seed-journal-loss
    /// residuals both land here.
    pub engine_spawn_deferred: Counter,
    /// An epoch resolved the CONSTANT seedless-arm base
    /// (`sha256(epoch ‖ sorted peers)`) instead of the previous epoch's terminal-block
    /// witness seed, because no witness could exist there: epoch 0, a non-computable
    /// terminal height, or a pre-bootstrap link. Counted where the base is CHOSEN, which is
    /// upstream of the promote gates — so an epoch that resolves the constant base and is
    /// then demoted to verify-only counts here without ever spawning an engine. That base
    /// is derivable an epoch ahead, so the epoch's first leader is predictable — the only
    /// signal that separates a chain on the intended unpredictable path from one silently
    /// on the old behaviour. Expected only around bootstrap.
    pub fallback_seed_constant: Counter,
    /// A live-epoch engine handle was found COMPLETED at a reconcile edge while
    /// its epoch is still the live frontier, and the manager re-ran the spawn
    /// path. Under `catch_panics(true)` a child engine panic completes the
    /// `Handle` without reaching the manager, so nothing else respawns it —
    /// non-zero means such a dead engine was detected and revived (or the engine
    /// exited early for any other reason). 0 on a healthy chain.
    pub engine_respawned: Counter,
}

impl EpochEngineMetrics {
    /// Register every counter. Call ONCE per process, against the SAME context
    /// `BeaconMetrics::register` is called against — commonware prefixes each
    /// family with the context's label path, so registering these on a labelled
    /// child context would rename them in the scrape without any gate noticing.
    pub fn register(&self, ctx: &impl Metrics) {
        ctx.register(
            "epoch_engine_demoted_rotated_out_total",
            "Per-epoch engines self-demoted because the validator was rotated out of the committee.",
            self.engine_demoted_rotated_out.clone(),
        );
        ctx.register(
            "epoch_engine_spawn_deferred_total",
            "Per-epoch engine spawns deferred because E-1's boundary input is missing — \
             EITHER the boundary block itself, OR the terminal-round seed the base needs \
             (member is verify-only meanwhile). Persistently non-zero = the height is below \
             the marshal floor and boundary seeding did not cover it, or the seed never \
             arrived. The counter does not distinguish the two; the two INFO lines at the \
             defer site do.",
            self.engine_spawn_deferred.clone(),
        );
        ctx.register(
            "dpos_fallback_seed_constant_total",
            "Epochs that resolved the constant (predictable) seedless-arm base because no \
             previous-epoch witness seed could exist. Counted at the choice, which precedes \
             the promote gates, so a demoted epoch counts too. Expected only around bootstrap.",
            self.fallback_seed_constant.clone(),
        );
        ctx.register(
            "epoch_engine_respawned_total",
            "Live-frontier engines found completed at a reconcile edge (panic caught by \
             catch_panics, or early exit) and revived via the spawn path.",
            self.engine_respawned.clone(),
        );
    }
}

// Finished engines are aborted at the transition (tempo's exit-at-transition
// pattern) — there is no concurrent active-epochs window. A finished engine
// has nothing left to produce (its boundary finalization is what triggers
// entering the next epoch) and its boundary re-propose loop is UNPACED
// (Inline re-proposes without calling `app.propose`), so at 1 blk/s it spins
// hundreds of views/s of BLS + marshal traffic and starves the live epoch
// into certification timeouts. Stragglers still in the old epoch do not need
// our engine: the boundary finalization is served via marshal/resolver, and
// their late certificates verify via `EpochSchemeProvider` (trailing
// 8-epoch window — see `SCHEME_RETENTION_EPOCHS`).

/// Bounded mpsc capacity for boundary triggers (tokio `mpsc::channel(N)`).
const BOUNDARY_BUFFER: usize = 64;

/// Target epochs below the cutoff whose agreement journal partition is swept on
/// every prune.
///
/// The sweep is what makes the removal survive CANCELLATION. An agreement
/// supervisor removes its own partition after it delivers, but every external
/// `abort()` — this prune, the SafetyHalt unwind, the manager's own exit —
/// cancels it at an await and that removal never runs. Nothing else reclaims
/// `dkg_epoch_{n}`, and unlike the ceremony journals there is no reconciler for
/// it, so the directory and its one blob per traversed view survive forever.
///
/// A band reclaims them all: it is driven by epoch NUMBER rather than by a map
/// this process populated, so it also collects a previous process's leftovers on
/// the first boundary after a restart, and it collects a partition that an
/// aborted-but-not-yet-stopped voter wrote one last time. The band is the
/// trailing scheme-retention window — the same distance the rest of this actor
/// keeps state for.
///
/// It is NOT swept on every prune, and that is the difference from the shape
/// this replaced. `abort_below` runs on every reconcile, a reconcile runs on
/// every finalized block through the committee module's wake-up
/// (`CommitteeStore::anchor_advanced` publishes unconditionally), and each sweep
/// is `AGREEMENT_SWEEP_SPAN` `Storage::remove` calls — every one of them a
/// process-global runtime lock and a directory removal. Per BLOCK, for a band
/// that changes once per EPOCH. The two states that can put a reclaimable
/// partition inside the band are both edges, so the sweep follows them instead:
/// the cutoff reaching a height it has never been at (a new epoch enters the
/// band, and a fresh process's first prune is this case), or this very call
/// having aborted an instance (its last journal write is on disk by then — the
/// abort is JOINED above the sweep).
const AGREEMENT_SWEEP_SPAN: u64 = SCHEME_RETENTION_EPOCHS as u64;

/// Drop every epoch-key agreement instance whose target is below `cutoff`,
/// aborting it on the way out, and reclaim the journal partitions of the targets
/// below the cutoff that no longer have one.
///
/// The SAME cutoff the per-epoch engines are pruned on, and for a matching
/// reason: below the frontier an instance has either delivered its artifact —
/// its supervisor has already returned, so the abort is a no-op — or it is still
/// agreeing a key for an epoch the chain has gone past.
///
/// `swept_to` is the highest cutoff whose band this process has already swept —
/// see [`AGREEMENT_SWEEP_SPAN`] for why the sweep is edge-driven. It is RAISED
/// rather than assigned: a prune at a cutoff below the highest one (a deferred
/// reconcile for an older epoch draining through the committee wake-up) sweeps
/// its own band when it aborted something, but it has not un-swept the higher
/// band, and a memo that went backwards there would re-sweep bands already done
/// on every block until the cutoff climbed back.
async fn prune_agreements<E: Storage>(
    context: &E,
    agreements: &mut BTreeMap<Epoch, Handle<()>>,
    cutoff: u64,
    partition_prefix: &str,
    swept_to: &mut u64,
) {
    let stale: Vec<Epoch> = agreements
        .keys()
        .copied()
        .filter(|e| e.get() < cutoff)
        .collect();
    // Taken BEFORE the loop consumes the list: an instance aborted here is the
    // one state that puts a partition in the band without the cutoff moving.
    let aborted_one = !stale.is_empty();
    for e in stale {
        if let Some(handle) = agreements.remove(&e) {
            handle.abort();
            // JOINED, and before the sweep below — for the same reason
            // `run_agreement` joins before destroying its own partition: `abort`
            // only REQUESTS cancellation, so the simplex voter under this
            // supervisor runs on to its next await and its `on_stopped` still
            // syncs the journal. A sweep that ran while it was still stopping
            // would remove a partition the voter then recreates with one last
            // write, and nothing reclaims that one. The map removal above is what
            // makes the guard below unable to protect these epochs, so the wait
            // has to be here.
            drop(handle.await);
            info!(?e, "epoch-key agreement instance pruned (transition)");
        }
    }
    if !(cutoff > *swept_to || aborted_one) {
        return;
    }
    for epoch in cutoff.saturating_sub(AGREEMENT_SWEEP_SPAN)..cutoff {
        // A live instance still owns its partition even below the cutoff: it was
        // adopted after this prune's abort pass.
        if agreements.contains_key(&Epoch::new(epoch)) {
            continue;
        }
        match context
            .remove(&agreement_partition(partition_prefix, epoch), None)
            .await
        {
            Ok(()) => info!(epoch, "epoch-key agreement journal partition reclaimed"),
            // Nothing to reclaim: the overwhelmingly common case, since the band
            // is swept whenever it moves whether or not an instance ever ran
            // there.
            Err(commonware_runtime::Error::PartitionMissing(_)) => {}
            Err(err) => warn!(
                epoch,
                ?err,
                "could not reclaim the epoch-key agreement journal partition"
            ),
        }
    }
    *swept_to = (*swept_to).max(cutoff);
}

/// `recv()` on an optional receiver, or park forever when it is `None` — so the
/// agreement-intake branch of the manager's `select!` is inert on a node with no
/// agreement plane wired, without a second loop shape.
async fn recv_agreement(
    rx: Option<&mut mpsc::Receiver<(Epoch, Handle<()>)>>,
) -> Option<(Epoch, Handle<()>)> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

/// The next beacon wake-up THIS actor acts on, with the seed class dropped
/// inside the future.
///
/// The seed class fires about once a round and belongs to the executor. Answering
/// it with a `continue` in the loop body would still cost a full turn of the
/// manager's `select!` per round — both `Notify` edges are rebuilt each
/// iteration — where the HEAD shape had no round-rate wake-up on this loop at
/// all. Dropping it here keeps the single shared stream and leaves the loop on
/// its own edges.
///
/// Cancel-safe in the way `select!` needs: the only messages it can consume and
/// discard are the ones this actor would have discarded anyway, and
/// `broadcast::Receiver::recv` is itself cancel-safe. `Lagged` and `Closed` are
/// returned, not swallowed — either may be hiding a class this actor does act on.
async fn next_reconcile_wake(
    rx: &mut tokio::sync::broadcast::Receiver<crate::beacon::BeaconEvent>,
) -> Result<crate::beacon::BeaconEvent, tokio::sync::broadcast::error::RecvError> {
    loop {
        match rx.recv().await {
            Ok(crate::beacon::BeaconEvent::SeedRecorded) => continue,
            other => return other,
        }
    }
}

/// The 3 plane-owned simplex broker handles a SIGNER engine registers per-epoch
/// sub-channels against (vote/cert/resolver). Bundled into one struct so the
/// manager threads a single `Option<Muxes>` instead of 3 positional handles —
/// `None` ⇒ a FOLLOWER (no plane): it only ever soft-enters (Verifier forever,
/// `signer_keypair == None`), so [`Actor::spawn_engine`] is unreachable and the
/// muxes are never touched. This makes "a follower has no plane" a COMPILE-time
/// fact (the `None` arm) rather than a fabricated-but-idle socket plane.
pub struct Muxes<HS, HR>
where
    HS: Sender<PublicKey = PublicKey>,
    HR: Receiver<PublicKey = PublicKey>,
{
    pub vote: SharedMux<HS, HR>,
    pub cert: SharedMux<HS, HR>,
    pub res: SharedMux<HS, HR>,
}

/// Per-epoch lifecycle actor.
pub struct Actor<E, B, XC, A>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics,
    B: Blocker<PublicKey = PublicKey>,
{
    context: ContextCell<E>,
    active_epochs: BTreeMap<Epoch, Handle<()>>,
    /// Supervisor handles of the epoch-key AGREEMENT instances, keyed by their
    /// TARGET epoch — a SECOND map, and it must stay separate from
    /// `active_epochs`.
    ///
    /// `reconcile_roles` polls `engine_handle_dead` over `active_epochs` and reads
    /// a COMPLETED handle as a caught panic: it drops the entry, bumps
    /// `engine_respawned` and spawns a replacement. An agreement instance is
    /// SUPPOSED to complete — it aborts itself the moment its own finalization
    /// lands (`beacon::dkg_engine`) — so an entry over there would be
    /// resurrected on every reconcile, for the life of the process. Here nothing
    /// polls it: a completed supervisor simply sits until `abort_below` drops it,
    /// and `abort()` on an already-completed handle is a no-op.
    dkg_agreements: BTreeMap<Epoch, Handle<()>>,
    /// The highest cutoff whose agreement-journal band [`prune_agreements`] has
    /// already swept in THIS process — the memo that makes the sweep an
    /// epoch-rate job instead of a per-block one. `0` at construction, which is
    /// what makes a fresh process's first prune sweep (and collect the previous
    /// process's leftovers); a cutoff of `0` has an empty band either way.
    agreements_swept_to: u64,
    /// Where those handles come from. `None` ⇒ no agreement plane wired ⇒ the
    /// branch parks forever and the map stays empty; the beacon plane holds the
    /// sending half and posts a handle per instance it starts.
    agreement_intake: Option<mpsc::Receiver<(Epoch, Handle<()>)>>,
    boundary_rx: mpsc::Receiver<Epoch>,
    /// Highest epoch we have entered (full or soft) — i.e. the highest epoch
    /// whose committee scheme is registered, so the marshal can verify its
    /// certs. Drives the catch-up hint target. Monotonic; never decremented by
    /// `prune_old` (the scheme provider keeps a trailing window).
    highest_entered_epoch: Epoch,
    /// The marshal's ordering tip as [`crate::application::FluentApp`] publishes
    /// it — the height of the highest finalization this node has VERIFIED and
    /// stored, and therefore the ONE input of [`Self::live_epoch`].
    ///
    /// It replaced `highest_observed_epoch` + `observed_reporters` +
    /// `sender_pins` + `corroborate_frontier`: the live frontier used to be
    /// inferred from unauthenticated epoch tags on the vote backup channel,
    /// corroborated by f+1 distinct senders because no single one of them could
    /// be believed (R-003). A verified finalization needs no corroborating
    /// witness — the certificate IS the witness — so the whole structure has one
    /// field left, and a peer naming `u64::MAX` moves nothing.
    ///
    /// A `watch` receiver and not a marshal read: the same handle is the VALUE
    /// at a decision point ([`Self::live_epoch`] borrows it) and the EDGE that
    /// re-decides when the tip moves without a boundary (the `tip_wake` arm in
    /// [`Self::run`]) — a dead epoch's engine therefore outlives the tip that
    /// killed it by one `Update::Tip` and no longer.
    tip: watch::Receiver<u64>,
    /// The live epoch the LAST reconcile of the live epoch ran for — the tip
    /// edge's memo, and the whole reason that edge is not per-block work.
    ///
    /// The edge this actor owes is "the LIVE EPOCH changed", and the tip is only
    /// its input: a verified finalization arrives roughly once a second, while
    /// the epoch it names changes once an epoch. Every pass of
    /// [`Self::reconcile_roles`] runs `abort_below`, and that unconditionally
    /// sweeps the epoch-key agreement band — `SCHEME_RETENTION_EPOCHS`
    /// `Storage::remove` calls, each one a process-global runtime lock and a
    /// directory removal — so an ungated tip arm would pay that, plus a
    /// `Beacon::observe_epoch` and a participation probe, on every block.
    ///
    /// Written by `reconcile_roles` itself rather than by the arm, because every
    /// OTHER edge that reconciles the live epoch (a boundary delivery that
    /// happens to be it, the committee wake-up, the beacon stream,
    /// `spawn_unblocked`) settles the same question — a memo the arm alone wrote
    /// would let the first tip after any of them pay a whole reconcile for
    /// nothing. `None` until the geometry freezes, which is also the value the
    /// gate compares against while there is no live epoch at all.
    tip_reconciled_live: Option<Epoch>,
    /// The role this node currently holds per epoch — the single source of truth
    /// the reconciler diffs against. `Signer` ⟺ a participating engine in
    /// `active_epochs`; `Verifier` ⟺ a verify-only scheme registered, no engine.
    roles: BTreeMap<Epoch, Role>,
    /// Live-frontier epochs whose `Verifier→Signer` spawn is parked on marshal
    /// block availability: the `Inline::genesis(E)` precondition (the E-1 boundary
    /// block not yet in marshal storage) — resumes as backfill lands.
    /// Re-checked on every reconcile edge (boundary / tip / share /
    /// spawn_unblocked) — `reconcile_roles` is idempotent, so a parked
    /// epoch spawns the instant the boundary block lands. Never panics (defer, never
    /// `unreachable!`).
    deferred_spawns: BTreeSet<Epoch>,
    /// Epochs whose [`Self::reconcile_roles`] returned having decided NOTHING,
    /// because `committee[E]` was not readable at this node's anchor when the
    /// edge that carried the epoch arrived.
    ///
    /// A SECOND parked set beside `deferred_spawns`, and it has to be separate
    /// because the two are parked at opposite ends of the same function: a
    /// deferred SPAWN has already run the whole reconcile (the bookkeeping, the
    /// frontier, `soft_enter`) and is waiting for one marshal block, while an
    /// epoch here has run NONE of it — the read is the first statement of the
    /// function (see the early return) and nothing is written before it.
    ///
    /// It exists because that early return has no other retry. Every other edge
    /// of the `select!` is gated on something a not-yet-readable epoch does not
    /// produce, and the module's own wake-up went through
    /// [`Self::reconcile_live`], which reconciles the LIVE epoch and nothing
    /// else — so a boundary for an epoch the tip has already passed got no retry
    /// at all and its scheme was never registered. Draining this set on the
    /// wake-up is that retry, and it deliberately skips the `is_live_epoch` gate:
    /// the gate guards against re-running a reconcile that ALREADY ran over a
    /// stale epoch (the downgrade churn `reconcile_live` documents), which is not
    /// the state an epoch in here is in.
    ///
    /// Only a RETRYABLE miss is remembered
    /// ([`crate::committee::CommitteeError::is_transient`]), which is also what
    /// bounds the set: an epoch below the read window can never be read again,
    /// so keeping it would be a permanent entry no wake-up can ever clear.
    deferred_reconciles: BTreeSet<Epoch>,
    cfg: Config<B, XC, A>,
}

/// Configuration for the [`Actor`].
pub struct Config<B, XC, A> {
    pub me: PublicKey,
    pub blocker: B,
    pub chain_id: u64,
    /// Single cross-epoch `OriginEpocher` — built once in
    /// `OuterBuilder::build`, cloned into both the marshal Config and
    /// every `EpochEngineConfig` constructed in `enter()`. `origin = dposActivationBlock`.
    pub epocher: OriginEpocher,
    pub signer_keypair: Option<ValidatorBlsKeypair>,
    pub app: FluentApp<XC, A>,
    pub timeouts: ConsensusTimeouts,
    pub mailbox_size: usize,
    /// The ONE randomness handle. Absorbs the beacon-shaped inputs this config
    /// used to carry one by one; a node with no beacon material holds the
    /// permanently-negative provider rather than a bundle of `None`s.
    pub randomness: Arc<dyn Beacon>,
    /// Edge-trigger fired by the executor each time it records a finalized
    /// OrderBlock (i.e. the marshal now holds another finalized block). It is the
    /// MID-EPOCH promotion trigger (the cycle-2 fix): re-runs `reconcile_roles` for
    /// the live epoch so a caught-up member promotes the instant its
    /// `Inline::genesis(E)` precondition is met (the E-1 boundary block landing IS
    /// an executor finalized-advance) — no boundary-finalize wait. Fires even in a
    /// thin-quorum stall, because the LOCAL executor still advances to the stall
    /// tip while the chain is globally stalled.
    pub spawn_unblocked: Arc<Notify>,
    /// Fork-safety latch (Phase 3 `SafetyHalt`). Read by [`Actor::reconcile_roles`]
    /// so a halted node is NEVER (re-)promoted to a participating `Signer`
    /// (verify-only forever), and awaited on the [`Actor::run`] select so the
    /// instant the executor / a re-jump engages it (result divergence / EL Invalid
    /// / L1 fork) every running engine is aborted — stop signing/proposing/voting
    /// immediately, not just at the next boundary. Cross-launch singleton from
    /// `dpos.rs::launch`, shared with the executor + the OuterEngine supervisor.
    pub safety_halt: crate::sync_metrics::SafetyHalt,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`].
    pub marshal_mailbox: MarshalMailbox<BlsScheme, Standard<OrderBlock>>,
    /// Peers to target when re-driving a finalization fetch: the highest known
    /// epoch's committee, from `EpochSchemeProvider::latest_scheme`. The SAME
    /// closure the executor's catch-up re-fetch uses — built once in
    /// [`crate::outer::OuterBuilder::build`].
    pub peers_for_finalization: crate::executor::PeersForFinalization,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`].
    pub slasher_mailbox: SlasherMailbox,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`]: the
    /// notarization arm of the simplex reporter, forwarding `SpecNotarized`
    /// commands to the executor for speculative execution.
    pub spec_exec_mailbox: crate::spec_exec::Mailbox,
    /// Cross-launch singleton from [`crate::outer::OuterEngine`]: the manager's
    /// OWN counters (see [`EpochEngineMetrics`]).
    pub epoch_metrics: EpochEngineMetrics,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`].
    pub page_cache: CacheRef,
    /// EVERY per-epoch committee this manager needs, and the ONE place an
    /// epoch's scheme is produced.
    ///
    /// It replaced three inputs at once: the `(epoch, snapshot)` the boundary
    /// channel used to carry, the `register_scheme` callback into a second map,
    /// and the bulk catch-up span closure that read committees at a cursor of
    /// its own. Reading `committee[E]` through this handle IS the registration —
    /// the module builds the epoch's verify-only scheme in the same map slot as
    /// the record — so "soft-enter" is a read, and the catch-up span is gone
    /// entirely: the marshal's `CertProvider` is this same handle
    /// ([`EpochSchemeProvider`]'s `CertProvider::scoped`), so the epoch of a certificate it is
    /// asked to verify registers itself on the read that verification needs, at
    /// an anchor at least as high as any earlier pre-registration would have
    /// used.
    pub committee: Arc<dyn Committee>,
    /// The repair sweep's view on that same map: which epochs hold a verify-only
    /// scheme is a question only the map can answer, and the answer covers every
    /// read that made a record — not only the ones this manager drove.
    pub scheme_pins: EpochSchemeProvider,
    /// Prefix of every per-epoch journal partition this manager opens or sweeps:
    /// the ordering engines' `{prefix}consensus_epoch_{E}`
    /// ([`crate::engine::engine_partition`]) and the adopted agreement instances'
    /// `{prefix}dkg_epoch_{E}` ([`crate::beacon::agreement_partition`]).
    /// Production passes `""` (the on-disk names are unchanged); the in-crate
    /// deterministic testbed passes `node{i}-`, because its N nodes share one
    /// in-memory `Storage`.
    pub partition_prefix: String,
    /// DEVNET/TEST-ONLY byzantine validator behaviour (gated behind
    /// `dpos-devnet-byzantine`). `None` on every honest node. Passed into every
    /// per-epoch [`EpochEngineConfig`] so the engine swaps in a
    /// [`crate::byzantine::VoteEquivocator`].
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub byzantine: Option<crate::byzantine::ByzantineMode>,
}

impl<E, B, XC, A> Actor<E, B, XC, A>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics,
    B: Blocker<PublicKey = PublicKey> + Clone,
    XC: ExecutedChain,
    A: OrderingAssembler,
{
    /// Construct the actor + return the bounded `boundary_tx` sender (held by
    /// 03's `EpochTransition`).
    pub fn new(context: E, cfg: Config<B, XC, A>) -> (Self, mpsc::Sender<Epoch>) {
        let (boundary_tx, boundary_rx) = mpsc::channel(BOUNDARY_BUFFER);
        let actor = Self {
            context: ContextCell::new(context),
            active_epochs: BTreeMap::new(),
            dkg_agreements: BTreeMap::new(),
            agreements_swept_to: 0,
            agreement_intake: None,
            boundary_rx,
            highest_entered_epoch: Epoch::new(0),
            // Subscribed HERE rather than in `run`, because the value is read on
            // every role decision and not only on the edge — and because a
            // `watch` receiver holds the last published value, so a manager built
            // after the marshal's startup tip starts from that tip and not from
            // the seed.
            tip: cfg.app.ordering_tip(),
            tip_reconciled_live: None,
            roles: BTreeMap::new(),
            deferred_spawns: BTreeSet::new(),
            deferred_reconciles: BTreeSet::new(),
            cfg,
        };
        (actor, boundary_tx)
    }

    /// Take ownership of the epoch-key agreement instances the beacon plane
    /// starts, so they are pruned on the same frontier cutoff as the per-epoch
    /// engines and torn down with the manager. Left unwired the manager owns none
    /// and the branch is inert.
    pub fn with_agreement_intake(
        mut self,
        agreement_intake: mpsc::Receiver<(Epoch, Handle<()>)>,
    ) -> Self {
        self.agreement_intake = Some(agreement_intake);
        self
    }

    /// Start the manager. The 3 simplex broker handles (vote/cert/resolver) are
    /// owned by the always-on plane (node crate); this manager CLONES them per
    /// promotion to register per-epoch sub-channels and drops them on exit (the
    /// `SubReceiver`s auto-deregister, freeing the slots for the next promotion).
    pub fn start<HS, HR>(mut self, muxes: Option<Muxes<HS, HR>>) -> Handle<()>
    where
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
    {
        spawn_cell!(self.context, self.run(muxes).await)
    }

    async fn run<HS, HR>(mut self, muxes: Option<Muxes<HS, HR>>)
    where
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
    {
        // The vote/cert/resolver Muxers live in the always-on plane (one set per
        // process). Graceful exit is via boundary_rx close; the plane's Muxer
        // tasks are NOT aborted here (they outlive this manager).
        // Cloned Arcs so the per-iteration `notified()` futures borrow the local
        // handles, not `self` (the arms below take `&mut self`).
        let spawn_unblocked = self.cfg.spawn_unblocked.clone();
        let safety_halt = self.cfg.safety_halt.clone();
        // SUBSCRIBED ONCE, before the loop and therefore before this actor's first
        // reconcile: a `broadcast` buffers from the subscription onward, so an
        // event fired before it would be lost, and the reconcile below is what
        // establishes the state a later wake-up asks this actor to re-derive.
        // ONE stream for the two classes this actor cares about, where there used
        // to be two `Notify` handles — the wake-up says only that something may
        // have changed, and both arms answer it by re-reading.
        let mut beacon_events = self.cfg.randomness.subscribe();
        // The committee module's own wake-up, and the ONLY retry a reconcile
        // parked on `NotReadable` has. `reconcile_roles` now reads `committee[E]`
        // itself, so a boundary that arrives before this node's anchor can see the
        // epoch returns having decided nothing — and every other edge here is
        // gated on something that a not-yet-readable epoch does not produce
        // (`spawn_unblocked` needs a parked spawn, the tip edge needs a verified
        // finalization, the beacon stream needs a share or a key). Subscribed ONCE, before
        // the loop, so a wake-up fired between the first read and the first
        // `changed()` is held by the watch's own "seen" marker rather than lost.
        let mut committee_readable = self.cfg.committee.subscribe();
        // The marshal tip's EDGE. A second receiver on the SAME channel
        // `self.tip` reads the value from — not a second source: `changed()`
        // needs a `&mut` the value-reading arms cannot lend it, and the two
        // receivers' "seen" markers are independent by construction. This is the
        // only edge that fires when the tip moves without a boundary delivery,
        // which is exactly the state a node whose execution lags is in: the
        // frontier passes `last(E)`, `E` stops being live, and the engine this
        // node still holds for `E` is aborted on the reconcile below.
        let mut tip_wake = self.tip.clone();
        // The repair sweep runs OFF this loop — see `spawn_repair_sweep`. Waking
        // it is a synchronous `send_replace`, so no arm below can be held by it.
        // Dropping this sender on the way out is what stops the task.
        let (sweep_wake, sweep_handle) = self.spawn_repair_sweep();
        // Taken out so the intake branch borrows the local rather than `self` (the
        // arms below take `&mut self`).
        let mut agreement_intake = self.agreement_intake.take();
        loop {
            // Arm the edge wakeups BEFORE the select. The two `Notify` producers
            // here use `notify_one` (permit-storing), so even a signal that fires
            // while no waiter is armed — between a reconcile and the next select —
            // is held as a permit and consumed by the next `notified()`. The
            // beacon's two classes ride the buffered subscription taken above and
            // are not re-armed per iteration.
            let spawn_n = spawn_unblocked.notified();
            let halt_n = safety_halt.engaged_edge();
            tokio::pin!(spawn_n, halt_n);
            tokio::select! {
                // Edge: the fork-safety latch was engaged (result divergence / EL
                // Invalid / L1 fork). Abort every participating engine NOW so the node
                // stops signing/proposing/voting immediately (not at the next
                // boundary), and clear any parked promotion. `reconcile_roles` keeps
                // it a Verifier forever thereafter (the latch is permanent). The
                // manager itself stays UP (marshal keeps verifying certs); recovery is
                // external (L1 proof + governance).
                _ = &mut halt_n => {
                    for (epoch, handle) in std::mem::take(&mut self.active_epochs) {
                        warn!(?epoch, "SafetyHalt engaged — aborting participating engine \
                            (demote to verify-only permanently)");
                        handle.abort();
                    }
                    // The agreement plane goes with them. Its output is a share
                    // this node will never get to sign with — the latch is
                    // permanent and `reconcile_roles` keeps it a Verifier forever
                    // — so leaving it voting on a second plane after a fork-safety
                    // halt buys nothing and costs the network a vote it should
                    // not be casting.
                    for (epoch, handle) in std::mem::take(&mut self.dkg_agreements) {
                        warn!(?epoch, "SafetyHalt engaged — aborting epoch-key agreement instance");
                        handle.abort();
                    }
                    self.deferred_spawns.clear();
                    for role in self.roles.values_mut() {
                        *role = Role::Verifier;
                    }
                }
                recv = self.boundary_rx.recv() => {
                    match recv {
                        Some(epoch) => {
                            // The edge carrying an epoch this node's own execution
                            // has just reached. It is NOT the live frontier by
                            // itself — a node walking a multi-epoch catch-up
                            // crosses boundaries the network left long ago — so it
                            // is reconciled as the epoch it is, and the liveness
                            // gate inside decides between a scheme and an engine.
                            self.reconcile_roles(epoch, muxes.as_ref()).await;
                            // AFTER the reconcile, so the frontier the sweep runs
                            // under already includes this boundary's epoch and it
                            // cannot pin under the engine just spawned for it.
                            sweep_wake.send_replace(self.sweep_frontier());
                        }
                        None => {
                            info!("boundary_rx closed, epoch_manager exiting");
                            break;
                        }
                    }
                }
                // Edge: the LIVE EPOCH moved, as the marshal's verified tip
                // reports it. The tip itself moves once per finalized block and
                // carries no obligation of its own — what this actor owes is a
                // reconcile when the epoch that tip names CHANGES, which is the
                // edge on which an epoch stops being live, and the only one that
                // fires while this node's own execution (and therefore its
                // boundary stream) is stalled behind the network.
                // `reconcile_live` reconciles the NEW live epoch, whose
                // `abort_below` retires the engine of the epoch the tip just
                // left.
                //
                // The engine of a dead epoch therefore outlives the tip that
                // killed it by one `Update::Tip` — EXCEPT when `committee[live]`
                // is not readable at this node's anchor. `reconcile_roles` reads
                // the committee BEFORE `abort_below`, so an unreadable live
                // epoch defers the whole pass (`deferred_reconciles`) and the
                // dead engine lives on until the committee module's own wake-up
                // fires. That window is reachable: `live ≤ epoch(fin) + 3` on a
                // node whose execution has stalled (4.2 Б2's two-epoch ceiling
                // plus the terminal arm), while the module reads at most
                // `epoch(anchor) + 2`.
                //
                // `Err` (the sender dropped) only happens with the app itself
                // gone; the arm then stops firing and the loop keeps its other
                // edges rather than exiting on a channel that carries no
                // obligation.
                Ok(()) = tip_wake.changed() => {
                    // The gate that makes this an epoch-rate edge and not a
                    // block-rate one. A tip inside the epoch the last reconcile
                    // already ran for has nothing to add: the role rule reads the
                    // tip only through `live_epoch`, so an unchanged live epoch
                    // means an unchanged verdict, while the pass itself is not
                    // free (`abort_below` sweeps the agreement journal band on
                    // every call, `observe_epoch` re-attempts the previous
                    // epoch's key backfill, and a live engine pays a
                    // participation probe). The retries that DO belong to a
                    // block-rate edge have their own: a parked spawn has
                    // `spawn_unblocked`, a deferred reconcile has the committee
                    // module's wake-up, a share or key landing has the beacon
                    // stream.
                    //
                    // Deliberately NOT a sweep wake either. The repair sweep's
                    // wake set is unchanged by this pass (boundary + the beacon
                    // stream); only the frontier VALUE it runs under changed.
                    // Poking it here as well would repair the beacon key of an
                    // epoch this node's own execution has not reached — a real
                    // liveness hole, and a behaviour change wide enough to retire
                    // the parked-node premise three stand fixtures are built on
                    // (one a proven precondition). Recorded as a finding, not
                    // taken.
                    if self.live_epoch() != self.tip_reconciled_live {
                        self.reconcile_live(muxes.as_ref()).await;
                    }
                }
                // Edge: the committee module can read higher than it could — its
                // anchor advanced. NOT the geometry freeze itself: the publisher
                // returns without publishing while there is no geometry
                // (`committee/store.rs:614-616`), so what carries a freeze here
                // is the FIRST anchor advance after it — one per finalized
                // derive (`executor.rs:3873`). The consumer contract is "wake
                // and re-ask", so re-run the live epoch's reconcile: if it was
                // parked on an unreadable committee it now proceeds, and if it
                // was not, `reconcile_roles` is idempotent.
                _ = committee_readable.changed() => {
                    let live = self.live_epoch();
                    let targets =
                        committee_wake_targets(&mut self.deferred_reconciles, live);
                    for epoch in targets {
                        self.reconcile_roles(epoch, muxes.as_ref()).await;
                    }
                }
                // Edge: the executor recorded a finalized block — the MID-EPOCH
                // promotion trigger. A caught-up member promotes the instant its
                // `Inline::genesis` precondition is met; gated on a pending parked
                // spawn so the per-block fire is a no-op in steady state. Execution
                // progress can also unblock a catch-up span read → clear the memo.
                _ = &mut spawn_n => {
                    if !self.deferred_spawns.is_empty() {
                        self.reconcile_live(muxes.as_ref()).await;
                    }
                }
                // Edge: the beacon says something may have changed — a DKG share
                // landed, or a `PK_epoch` did. ONE arm for both classes, where
                // there used to be two: both answer the wake-up by re-reading the
                // same state, and the only difference was that the share arm did
                // not also poke the repair sweep. Poking it on a share landing is
                // extra idempotent work on a task built to be poked, and it is the
                // price of the merge being honest rather than two arms that must
                // stay in step by hand.
                //
                // A landed share re-runs the reconcile so a member parked by the
                // share-gate spawns now that its share is present (the running
                // scheme is frozen at construction, so this is a respawn).
                //
                // A landed `PK_epoch` has TWO consumers and until this arm existed
                // only one of them was served.
                //
                // BELOW the frontier: the repair sweep, for which this arm is the
                // FAST path and not the load-bearing one — a below-frontier epoch
                // has no other retry, since `reconcile_live`'s `is_live_epoch` guard
                // returns before touching it.
                //
                // AT the frontier: the live epoch's own entry, which the sweep
                // refuses by design. The registered scheme needs no repair — it reads
                // the key live through its oracle — so what this arm still owes the
                // frontier is the ACQUISITION: `reconcile_live` re-runs the ladder,
                // and `soft_enter` does not short-circuit on a recorded
                // `Role::Verifier`, so it will. A member that will PROMOTE gets that
                // edge for free from the participation class; a node that will NOT promote
                // (not a member of `committee[E]`, or a share heal that never
                // completes) had no edge at all and stayed vote-only for the whole life
                // of the epoch.
                //
                // This arm CANNOT be the only trigger for the sweep, and that is the
                // whole reason the boundary arm also sweeps. The key class fires on
                // a `BeaconKeys` record, and every production writer of that store
                // needs either this node's own DKG material or a change epoch's
                // agreement artifact. A node with no DKG material on a committee that
                // has not changed — the case that repair exists for — never fires it
                // at all.
                //
                // NB a near-miss that makes a cheaper fix tempting and wrong: an
                // attested insert IS followed one task-hop later by `spawn_unblocked`, but
                // that arm is gated on a non-empty `deferred_spawns`, and a node
                // demoted by the promote value-gate never enters that set. Same
                // event, wrong gate.
                //
                // The SEED class is the third thing on this stream and does not
                // belong to this actor: it fires ~1/s and its consumer is the
                // executor's held-height release, so reconciling on it would put a
                // staking read behind every round. Filtered INSIDE the future
                // rather than by a `continue` in the body, so the round-rate class
                // does not re-arm this `select!`'s edges once a round — the two
                // `Notify` futures above are rebuilt per loop iteration, and the
                // HEAD shape had no round-rate wake-up for this loop at all.
                // Filtered, not subscribed away — one stream is what lets a class
                // gain a second consumer without its producer growing a second
                // fan-out.
                event = next_reconcile_wake(&mut beacon_events) => {
                    // ONE effect for both classes now, and that is a REMOVAL
                    // rather than a merge: the participation class used to also
                    // clear the catch-up span's no-progress memo, and the span it
                    // memoized is gone (the committee module registers an epoch's
                    // scheme on the read that needs it, so there is no span to
                    // re-attempt). What is left — wake the repair sweep, re-run
                    // the live epoch's reconcile — is what BOTH classes always
                    // owed: a landed share can promote a parked member, and a
                    // landed `PK_epoch` is both the sweep's input and the live
                    // epoch's acquisition.
                    if let Ok(crate::beacon::BeaconEvent::SeedRecorded) = event {
                        // Filtered out by `next_reconcile_wake`.
                        unreachable!("the seed class never leaves next_reconcile_wake");
                    }
                    sweep_wake.send_replace(self.sweep_frontier());
                    self.reconcile_live(muxes.as_ref()).await;
                }
                // Edge: the beacon plane started an epoch-key agreement instance
                // and handed us its supervisor. Adopting it here is what puts the
                // instance under the same frontier cutoff as the engines; a second
                // handle for an epoch we already hold replaces the first, aborting
                // it so two instances never run for one target.
                adopted = recv_agreement(agreement_intake.as_mut()) => match adopted {
                    Some((epoch, handle)) => {
                        // A HALTED node adopts nothing. The halt arm above aborts
                        // every instance it can SEE, but it is a one-shot edge —
                        // `SafetyHalt::engage` fires `notify_one` once and nothing
                        // notifies again — so an instance that finishes starting
                        // AFTER that firing lands here with the arm already spent,
                        // and `prune_agreements` never reaches it either (it only
                        // sweeps targets below the live cutoff).
                        //
                        // Not a race: the launcher lives on a node-side handle and
                        // `start_one` awaits four mux registrations plus a spawn,
                        // so the window is wide, and the latch is restored from
                        // its datadir marker at startup — meaning a node that
                        // halted yesterday re-adopts on every restart, for as long
                        // as the plane keeps starting instances. A standing leak,
                        // not a timing accident.
                        //
                        // The policy it violates is written ten lines up, in the
                        // halt arm: a halted node's vote on a second plane buys
                        // nothing and costs the network a vote it should not be
                        // casting. Enforced here rather than by re-arming the
                        // edge, because the latch is permanent — one read of it is
                        // the whole check.
                        if self.cfg.safety_halt.is_engaged() {
                            warn!(
                                ?epoch,
                                "SafetyHalt engaged — refusing to adopt an epoch-key agreement \
                                 instance that started after the halt"
                            );
                            handle.abort();
                        } else if let Some(previous) = self.dkg_agreements.insert(epoch, handle) {
                            warn!(?epoch, "replacing a live epoch-key agreement instance");
                            previous.abort();
                        }
                    }
                    None => agreement_intake = None,
                },
            }
        }

        // Abort all per-epoch engine handles (their `SubReceiver`s drop →
        // auto-deregister from the plane's persistent Muxers). abort() is idempotent
        // (no-op on already-completed handles per
        // monorepo/runtime/src/utils/handle.rs:107-118). The MuxHandle clones drop
        // here too — the plane's broker tasks stay live for the next promotion.
        for (epoch, handle) in std::mem::take(&mut self.active_epochs) {
            info!(?epoch, "aborting active epoch engine on exit");
            handle.abort();
        }
        for (epoch, handle) in std::mem::take(&mut self.dkg_agreements) {
            info!(?epoch, "aborting epoch-key agreement instance on exit");
            handle.abort();
        }
        // Aborted, not left to notice the dropped sender: it can be parked inside
        // a peer pull for the whole `PULL_TIMEOUT`, and nothing it could still do
        // matters once the manager that owns the scheme registry is gone.
        sweep_handle.abort();
    }

    /// The ONE live epoch, as a function of the marshal tip — the height of the
    /// highest finalization this node has VERIFIED and stored.
    ///
    /// `epoch_of(tip)` while the tip is inside that epoch, `epoch_of(tip) + 1`
    /// once the tip IS the epoch's last block. The second arm is not a
    /// refinement, it is what makes the function total: at `tip == last(E)`
    /// epoch `E` is finished — its terminal block is final, the engine for `E`
    /// builds no descendant of it (CW `standard/inline.rs:274-293` re-proposes
    /// the parent at a boundary instead) — so an `is_live(E) = E == epoch_of(tip)
    /// ∧ tip < last(E)` written as a conjunction would answer FALSE for every
    /// epoch at exactly that height, and a node sitting on a boundary would hold
    /// no live epoch at all. The two are the same rule: `E == epoch_of(tip) ∧ tip
    /// < last(E)` is `E == live_epoch(tip)` restricted to the non-boundary case,
    /// and the boundary case is `E + 1`.
    ///
    /// It is the SAME rule as [`crate::dpos::local_tracked_epoch`] (`dpos.rs`),
    /// which computes the epoch a node hands `track` from its finalized cursor —
    /// deliberately, because the two answer one question ("which epoch is this
    /// node's network in") from two cursors, and disagreeing on the boundary
    /// would put the peer set one epoch away from the engines.
    ///
    /// `None` while the geometry has not frozen: no height means anything yet,
    /// so no epoch is live and every reconcile soft-enters. That is the ONE
    /// refusal, here and in `local_tracked_epoch` alike — both are a `?` on
    /// `geometry()` and nothing else. A height below activation is NOT a second
    /// one: [`crate::committee::Geometry::epoch_of`] clamps it to epoch `0`
    /// (`epoch_at_block`'s `saturating_sub`), so a cursor still at its seed
    /// answers `Some(0)` and cold start full-enters epoch 0.
    fn live_epoch(&self) -> Option<Epoch> {
        Some(live_epoch_of(
            &self.cfg.committee.geometry()?,
            *self.tip.borrow(),
        ))
    }

    /// True when `epoch` is THE live epoch — the one the verified frontier is in.
    /// Every other epoch only soft-enters (register the scheme, NO participating
    /// engine): a Simplex engine for an epoch the network has left has no live
    /// peers and would drive the executor on a dead fork, intermittently wedging
    /// the catch-up.
    ///
    /// Equality and not `>=`, and that is the whole difference from the
    /// corroborated frontier this replaced. `>=` had to be inexact because the
    /// input was: an unauthenticated epoch tag could name anything, so the gate
    /// was written to fail SAFE (soft-enter) whenever it was not sure, and one
    /// peer naming `u64::MAX` therefore pinned the node into permanent
    /// verify-only (R-003). A verified finalization names exactly one epoch, so
    /// the gate can be exact — and an epoch ABOVE the live one is now refused
    /// too, which `>=` admitted.
    fn is_live_epoch(&self, epoch: Epoch) -> bool {
        self.cfg
            .committee
            .geometry()
            .is_some_and(|g| is_live_epoch_at(&g, *self.tip.borrow(), epoch))
    }

    /// Reconcile this node's per-epoch role from current state — the single
    /// decision point, folding the old `enter` + `prune_old`. `role(E) = Signer iff
    /// (I ∈ committee[E]) ∧ is_live_epoch(E)`, else `Verifier`; a `Signer`
    /// additionally needs a usable DKG share (share-gate) and the
    /// `Inline::genesis(E)` precondition before its engine spawns. Idempotent — safe
    /// to call repeatedly for the same `(epoch, snap)` on any edge.
    async fn reconcile_roles<HS, HR>(&mut self, epoch: Epoch, muxes: Option<&Muxes<HS, HR>>)
    where
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
    {
        // The tip edge's memo, taken BEFORE the read below can defer this pass:
        // whatever it decides, THIS is the answer for the live epoch, and the
        // next `Update::Tip` that does not move the live epoch has nothing to
        // add. A pass that defers on an unreadable committee memoizes too —
        // deliberately, because its retry is `deferred_reconciles` drained by
        // the module's own wake-up, and re-asking the same unreadable epoch once
        // per finalized block was the old shape's standing cost on exactly the
        // node that could least afford it (a stalled executor).
        //
        // Here rather than in the tip arm, so the memo covers every edge that
        // reconciles the live epoch — see the field's doc.
        if self.live_epoch() == Some(epoch) {
            self.tip_reconciled_live = Some(epoch);
        }
        // The epoch's committee, as ONE frozen value read at this node's own
        // anchor. It arrives HERE rather than riding the boundary channel: every
        // edge below (tip / share / spawn_unblocked / the module's own
        // wake-up) carries an epoch and nothing else, and a snapshot handed in by
        // whoever fired the edge would be a second authority on a value the
        // module already froze.
        //
        // NOT READABLE is a deferral, never a decision: the anchor only moves up,
        // and the module's wake-up (`Committee::subscribe`, the arm in `run`)
        // re-runs this reconcile the moment a higher anchor makes the epoch
        // readable. Nothing below it may run on a guess about the committee, so
        // the whole reconcile — bookkeeping included — waits for the value.
        let record = match self.cfg.committee.committee(epoch.get()) {
            Ok(record) => {
                // This epoch is being reconciled now, so it is no longer owed
                // one. Unconditional: the entry is only ever a debt, so removing
                // one that was never taken on is a no-op.
                self.deferred_reconciles.remove(&epoch);
                record
            }
            Err(e) => {
                // Remember the epoch so the module's own wake-up re-runs THIS
                // reconcile — `deferred_reconciles` says why nothing else will.
                // Only a retryable miss: a permanent refusal (an epoch the
                // window has dropped, a reverting contract) is a debt no wake-up
                // can ever settle, and parking it would leave an entry the set
                // can never lose.
                if e.is_transient() {
                    self.deferred_reconciles.insert(epoch);
                    debug!(
                        ?epoch,
                        %e,
                        "reconcile deferred — committee[E] not readable at this node's anchor yet"
                    );
                    return;
                }
                // THE ONE SITE THAT TURNS AN IMPOSSIBLE COMMITTEE INTO A HALT.
                //
                // This reconcile was owed: the epoch arrived on a boundary this
                // node's own execution reached, on the live-epoch edge, or on
                // the module's wake-up for an epoch already parked — never as a
                // lookahead or as a stranger's claim, which is why this is the
                // site and the module is not (a p2p frame naming an epoch, the
                // marshal verifying a certificate and the slasher resolving an
                // old charge all read the same store and must NOT stop the node
                // between them).
                //
                // `Impossible` and not `!is_transient()`: a revert keeps the
                // behaviour it had — the epoch is refused, the module's one
                // `error!` names it, the node stays up and an operator repairs
                // the module — because a revert says the read could not be
                // served, not that the committed state is impossible. An
                // impossible answer is different in kind: it is the chain
                // contradicting its own invariants, every validator reads the
                // same thing, and a node that merely skipped the epoch would go
                // on looking healthy while the whole network stopped
                // participating in silence (E4-REVIEW §9.1).
                //
                // The latch does the rest: the `engaged_edge` arm of `run`
                // aborts every running engine and the agreement instances, and
                // the `is_member` gate below keeps this node a Verifier for
                // good. Execution stops; the marshal keeps verifying and
                // serving what it already holds.
                if e.is_contract_impossible() {
                    if !self.cfg.safety_halt.is_engaged() {
                        error!(
                            ?epoch,
                            %e,
                            reason = SyncReason::ContractFork.as_str(),
                            "committee[E] of an epoch this node must enter is IMPOSSIBLE — the \
                             contract contradicted its own invariants; SafetyHalt: this node \
                             stops signing, proposing and voting (marshal keeps serving). \
                             Recovery is a repaired chain and a fresh start."
                        );
                    }
                    // Idempotent, and the FIRST verdict is the one recorded — a
                    // halt already engaged for another reason keeps its own.
                    self.cfg.safety_halt.engage(SyncReason::ContractFork);
                    return;
                }
                debug!(
                    ?epoch,
                    %e,
                    "reconcile refused — committee[E] is permanently unreadable at this node's \
                     anchor (a revert, this node's storage, or an epoch below the window)"
                );
                return;
            }
        };
        // Boundary bookkeeping (idempotent; monotone).
        self.highest_entered_epoch = self.highest_entered_epoch.max(epoch);
        // Tell the randomness subsystem where the core stands. It owns what that
        // means: the previous-epoch key warm-up (W3) and the retention of its own
        // store against the entered frontier. Both used to be written out here,
        // and both are facts ABOUT randomness that the core has no business
        // knowing — it reports the two epochs and nothing else.
        //
        // The report sits AFTER the frontier update deliberately: the retention
        // floor is taken from `highest_entered_epoch`, so reporting before the
        // `max` would prune against a stale frontier on the boundary that
        // advances it.
        self.cfg
            .randomness
            .observe_epoch(epoch, self.highest_entered_epoch);

        // Exit-at-transition: abort every engine strictly below the live epoch
        // (folded `prune_old`; `e < cutoff` only, so a stale/replayed boundary for
        // an OLD epoch can never abort a newer engine).
        self.abort_below(epoch).await;

        // Not the live epoch → soft-enter (verify-only scheme, NO engine): a
        // Simplex engine for a stale epoch has no live peers and would drive a dead
        // fork. Verify-only lets the marshal verify this epoch's certs — with the
        // seed slot checked against `PK_epoch` whenever the oracle can resolve it
        // at the moment the certificate arrives, and vote-only while it cannot.
        // Nothing about the registered scheme decides that; `repair_keyless_schemes`
        // is what drives the acquisition for below-frontier epochs.
        if !self.is_live_epoch(epoch) {
            self.soft_enter(epoch).await;
            self.deferred_spawns.remove(&epoch);
            info!(?epoch, "epoch soft-entered (scheme only, catch-up)");
            return;
        }

        // At the live frontier (checked above): role = f(member). "Caught up" is not
        // a separate input: a member only spawns a participating engine once the
        // share-gate AND `boundary_lookup` both hold below, which together
        // mean the local executor has derived up to E-1's boundary.
        //
        // Fork-safety latch (Phase 3): a SafetyHalted node is NEVER a member for
        // role purposes — it stays a Verifier forever (verify-only, never
        // re-promoted), whatever the committee says. The halt edge above already
        // aborted any running engine; this keeps future reconciles from re-spawning.
        let is_member = !self.cfg.safety_halt.is_engaged()
            && self.cfg.signer_keypair.is_some()
            && record.participants.position(&self.cfg.me).is_some();

        // Already a running signer for the live epoch — keep it, UNLESS its handle
        // has completed. The committee is frozen per epoch, so membership cannot
        // change mid-epoch; a frontier move aborts this engine via `abort_below` on
        // the next boundary. But under `catch_panics(true)` a child engine panic
        // completes the `Handle` without reaching the manager (nothing joins engine
        // handles), so a dead engine would otherwise wedge behind this gate forever.
        // Poll the handle: `Pending` ⇒ alive, keep it; `Ready` ⇒ dead, drop the
        // entry and fall through to the spawn path (re-spawn is safe — W1
        // `insert_group_key` is idempotent and a pre-drop `AlreadyRegistered` is
        // handled by `spawn_engine` returning false, retried next edge). The
        // respawn stays behind every gate below (share-gate, boundary-block,
        // safety-halt), reached only because the caller is at the live frontier.
        if let Some(handle) = self.active_epochs.get_mut(&epoch) {
            if !engine_handle_dead(handle) {
                // A LIVE ENGINE IS STILL SUBJECT TO THE PARTICIPATION QUESTION.
                // This early return used to skip every gate, on the reasoning that
                // committee membership is frozen per epoch — true, but the gates
                // do not only ask about membership. Whether this node may
                // participate can change UNDER a running engine: a quorum can
                // attest an epoch key after the spawn, and if it differs from the
                // one this node reconstructed, every vote it casts carries a seed
                // partial the rest of the committee rejects. Nothing revisited that
                // — the engine kept voting until a fork-safety halt or until the
                // epoch fell below the frontier and `abort_below` reached it.
                //
                // The probe is two store reads and runs on edges that already fire
                // (boundary / tip / participation / spawn_unblocked), and
                // its verdict is stable rather than transient: the attested tier
                // appears once and is never downgraded, so this cannot flap an
                // engine down and up.
                if let ShareProbe::Withheld(reason) = self.cfg.randomness.can_participate(epoch) {
                    warn!(
                        ?epoch,
                        ?reason,
                        "aborting a LIVE engine: this node may no longer participate at this \
                         epoch — verify-only for the rest of it"
                    );
                    if let Some(handle) = self.active_epochs.remove(&epoch) {
                        handle.abort();
                    }
                    self.roles.insert(epoch, Role::Verifier);
                    self.soft_enter(epoch).await;
                    return;
                }
                return;
            }
            self.active_epochs.remove(&epoch);
            self.cfg.epoch_metrics.engine_respawned.inc();
            warn!(
                ?epoch,
                "live-frontier engine handle completed unexpectedly (panic caught by \
                 catch_panics, or early exit) — respawning"
            );
        }

        // Role is a pure function of membership (the caller is already at the
        // live frontier, so liveness is not a separate input); see [`Role`].
        let assigned_role = if is_member {
            Role::Signer
        } else {
            Role::Verifier
        };
        match assigned_role {
            // Not a member (rotated out). Register verify-only so the marshal
            // verifies this epoch's certs; no participating engine.
            Role::Verifier => {
                if self.cfg.signer_keypair.is_some() && !is_member {
                    self.cfg.epoch_metrics.engine_demoted_rotated_out.inc();
                }
                self.soft_enter(epoch).await;
            }
            Role::Signer => {
                // Share-gate: a beacon-active member that cannot participate must
                // NOT run a participating engine — a seedless Simplex member
                // rejects honest peers' seeded votes (`combined_scheme::verify_attestation`)
                // and the batcher blocks them → wedge. The probe is
                // NON-BLOCKING by contract (blocking would stall the whole
                // reconcile loop, and re-block on every share edge for a
                // genuinely withholding member); on `Withheld`, register
                // verify-only + stay off the consensus plane. The participation
                // edge re-runs reconcile and promotes the instant the reason
                // clears.
                //
                // POSITION IS LOAD-BEARING: it sits BEFORE the boundary lookup
                // below so a member that cannot participate returns without
                // paying a marshal `get_block` on every participation edge.
                if let ShareProbe::Withheld(reason) = self.cfg.randomness.can_participate(epoch) {
                    self.soft_enter(epoch).await;
                    info!(
                        ?epoch,
                        ?reason,
                        "committee member cannot participate — verify-only (share-gate)"
                    );
                    return;
                }

                // `Inline::genesis(E)` precondition: the E-1 boundary block must be
                // in marshal storage before the per-epoch engine starts (else the
                // engine hits `unreachable!`). On a mid-epoch promotion the marshal
                // may still be backfilling it — DEFER, never panic; the executor's
                // `spawn_unblocked` edge (or the next boundary) re-pokes. Register
                // verify-only meanwhile so the marshal verifies this epoch's certs.
                // The record as the snapshot the surfaces this crate does not
                // own still speak (`Beacon::signer`, the seedless base, the
                // engine's committee index). A PROJECTION of the one frozen
                // record, built here and used by all three, so they cannot be
                // handed three different committees for one epoch.
                let snap = record.snapshot_view();

                let lookup = self.boundary_lookup(epoch).await;
                let fallback_seed = match seedless_base(&lookup, &snap) {
                    None => {
                        self.deferred_spawns.insert(epoch);
                        self.soft_enter(epoch).await;
                        self.cfg.epoch_metrics.engine_spawn_deferred.inc();
                        let boundary = epoch
                            .get()
                            .checked_sub(1)
                            .and_then(|prev| self.cfg.epocher.last(Epoch::new(prev)))
                            .map(|h| h.get());
                        // Two causes, two lines: an operator chasing a stuck block
                        // fetch when the block is already in hand and it is σ that
                        // is absent looks in the wrong place entirely.
                        match lookup {
                            BoundaryLookup::Missing(MissingInput::TerminalSeed) => info!(
                                ?epoch,
                                boundary,
                                "signer spawn deferred — E-1 boundary block is in marshal but \
                                 no σ for its terminal round; verify-only until a certificate \
                                 for that round arrives"
                            ),
                            _ => info!(
                                ?epoch,
                                boundary,
                                "signer spawn deferred — E-1 boundary block not yet in marshal; \
                                 verify-only until it lands"
                            ),
                        }
                        return;
                    }
                    Some(SeedlessBase::Witness(base)) => base,
                    Some(SeedlessBase::Constant(base)) => {
                        self.cfg.epoch_metrics.fallback_seed_constant.inc();
                        base
                    }
                };

                // The leader lottery, derived from the frozen record BEFORE
                // anything is registered or spawned (E4-10). Its weights are
                // non-optional by the module's window invariant, so the only way
                // this fails is a length disagreement the module would already
                // have refused — but a spawn that raised this epoch's scheme to
                // the signer half and THEN discovered it has no leader schedule
                // would leave the epoch half-entered, and that ordering is what
                // the hoist removes.
                let elector = match WeightedVrf::try_new(&snap, fallback_seed) {
                    Ok(elector) => elector,
                    Err(e) => {
                        error!(
                            ?epoch,
                            %e,
                            "skipping epoch spawn — no leader schedule can be derived from the \
                             frozen committee record"
                        );
                        self.soft_enter(epoch).await;
                        return;
                    }
                };

                // The scheme this epoch votes with, built whole by the
                // randomness subsystem. Everything the core used to do here —
                // the promote value-gate, the share self-probe, W1, and the
                // `build_signer` call itself — is inside that one operation now,
                // over ONE sample of the material, which is what keeps the gates
                // coherent with the key they admitted.
                //
                // KNOWN DEVIATION from the pre-split behaviour, recorded rather
                // than claimed neutral: the material is now sampled twice across
                // the boundary await above (once by `share_probe`, once here),
                // where it used to be sampled once before it. A recompute-heal
                // landing DURING the await is therefore used instead of demoting
                // on a stale sample — an improvement — but a successful signer
                // pays a second resolve per reconcile edge.
                let Some(keypair) = self.cfg.signer_keypair.clone() else {
                    unreachable!("Role::Signer requires is_member, which requires a signer keypair")
                };
                let scheme = match self.cfg.randomness.signer(epoch, &snap, &keypair) {
                    SignerVerdict::Signs(scheme) => scheme,
                    // Misconfiguration safety net, not a wedge path: this
                    // node's BLS key is not in the committee BiMap. Spawn
                    // verify-only anyway (as the engine used to do for
                    // itself) — the next reconcile aborts it.
                    SignerVerdict::RotatedKey(scheme) => {
                        metrics::counter!("epoch_engine_rotated_out_total").increment(1);
                        warn!(
                            ?epoch,
                            "validator BLS key not in committee BiMap — verify-only \
                                 (reconciler aborts this engine on its next reconcile)"
                        );
                        scheme
                    }
                    SignerVerdict::Withheld(reason) => {
                        info!(?epoch, ?reason, "withheld from signing — verify-only");
                        self.soft_enter(epoch).await;
                        return;
                    }
                    // Today's behaviour when the engine's own decode failed:
                    // warn and skip the epoch.
                    SignerVerdict::InvalidCommittee(e) => {
                        warn!(
                            ?epoch,
                            ?e,
                            "skipping epoch spawn — invalid committee snapshot"
                        );
                        return;
                    }
                };
                // The scheme the marshal verifies this epoch with, raised from
                // the module's own verify-only entry to this node's signer half
                // — the ONE upgrade path, and the only writer of a scheme slot
                // besides the module's verifier. A refusal is defence in depth
                // (it preserves the STRONGER entry), not a spawn gate: the
                // engine holds its own instance either way.
                if !self
                    .cfg
                    .committee
                    .upgrade_scheme(epoch.get(), scheme.clone())
                {
                    warn!(
                        ?epoch,
                        "the committee module refused this epoch's signer scheme — the marshal \
                         keeps the entry it already holds"
                    );
                }
                if self.spawn_engine(epoch, snap, scheme, elector, muxes).await {
                    self.roles.insert(epoch, Role::Signer);
                    self.deferred_spawns.remove(&epoch);
                    // Stable greppable token for the production-path smoke
                    // (`smoke-production-path`): the in-process Verifier→Signer
                    // promotion — a joiner that catches up + holds its DKG share
                    // re-promotes here without a process restart.
                    info!(
                        ?epoch,
                        "promoted to Signer in-process: per-epoch BFT engine started"
                    );
                }
            }
        }
    }

    /// Register a verify-only (multisig) scheme for `epoch` and record the
    /// `Verifier` role — UNLESS this node already holds a `Signer` for `epoch` (a
    /// running engine in `active_epochs` or a recorded `Signer` role). Idempotent:
    /// never downgrades an active signer to verify-only, which `EpochSchemeProvider`
    /// refuses (`signer→verifier downgrade`) and which would desync `self.roles`
    /// from the provider. This is the one site that READS `self.roles`, making it
    /// the diff source of truth the field doc promises.
    async fn soft_enter(&mut self, epoch: Epoch) {
        if self.active_epochs.contains_key(&epoch) || self.roles.get(&epoch) == Some(&Role::Signer)
        {
            return;
        }
        // READING the committee is the registration: the module builds this
        // epoch's verify-only scheme in the same map slot as the record, bound
        // to the epoch's beacon oracle from the one door that decides
        // beacon-activeness. There is nothing to hand it and nothing to hand
        // back — a `None` here means the epoch is not readable at this anchor
        // yet, which the module's wake-up retries.
        let registered = self.cfg.committee.scheme(epoch.get()).is_some();
        debug!(
            ?epoch,
            registered, "soft-enter: verify-only scheme taken from the committee module"
        );
        self.roles.insert(epoch, Role::Verifier);
    }

    /// Start the repair sweep on its OWN task and return the handle that wakes
    /// it. Called once, before [`Self::run`]'s `select!`.
    ///
    /// **The sweep must not run on that `select!`.** Its ladder spends one
    /// bounded peer pull per unpinned epoch, and the pull's caller-side rate
    /// bound SLEEPS until the epoch's next slot before it even issues the fetch
    /// (`PULL_MIN_INTERVAL` + `PULL_TIMEOUT`), once per epoch in a work list
    /// bounded only by [`SCHEME_RETENTION_EPOCHS`]. Awaited inline that is a
    /// ~100 s section during which the beacon wake-up arm, `spawn_unblocked` and
    /// the tip edge
    /// do not run — the three arms that turn this node into a signer.
    ///
    /// Everything the sweep touches is a cross-epoch singleton this actor holds
    /// by `Arc`, so the task needs no borrow of `self`; the only per-wake state
    /// is the frontier, which rides the wake. A `watch` (not a `Notify`) because
    /// the frontier has to ride it and because its receiver is created ONCE here,
    /// before the task's loop — the baseline-at-subscribe hazard that rules
    /// `watch` out in `beacon::keys` needs a per-iteration `subscribe`,
    /// which this is not.
    fn spawn_repair_sweep(&self) -> (watch::Sender<(Epoch, Epoch)>, Handle<()>) {
        let (wake_tx, wake_rx) = watch::channel(self.sweep_frontier());
        let hint: RepairHint = {
            let epocher = self.cfg.epocher.clone();
            let peers = self.cfg.peers_for_finalization.clone();
            let mailbox = self.cfg.marshal_mailbox.clone();
            Arc::new(move |upgraded: Vec<Epoch>| {
                let epocher = epocher.clone();
                let peers = peers.clone();
                let mailbox = mailbox.clone();
                Box::pin(async move {
                    for epoch in upgraded {
                        let (Some(boundary), Some(targets)) = (epocher.last(epoch), peers()) else {
                            continue;
                        };
                        info!(
                            ?epoch,
                            boundary = boundary.get(),
                            "below-frontier epoch obtained a late beacon key — re-driving its \
                             finalization fetch"
                        );
                        mailbox.hint_finalized(boundary, targets).await;
                    }
                }) as BoxFuture<'static, ()>
            })
        };
        let handle = self.context.with_label("repair_sweep").spawn({
            let provider = self.cfg.scheme_pins.clone();
            let randomness = self.cfg.randomness.clone();
            move |_| run_repair_sweep(wake_rx, provider, randomness, hint)
        });
        (wake_tx, handle)
    }

    /// The `(frontier, entered)` pair the repair sweep runs under.
    ///
    /// The sweep repairs the beacon key of every registered verify-only epoch
    /// STRICTLY BELOW `max(frontier, entered)`, so the frontier's only job here
    /// is to be the live epoch — which the sweep must exclude, because a live
    /// epoch's scheme reads its key through its own oracle and has nothing to
    /// repair. With no geometry there is no live epoch and `0` leaves the entered
    /// tip as the whole frontier: exactly the behaviour the un-corroborated
    /// `highest_observed_epoch` had, minus the ability of a peer to inflate it.
    fn sweep_frontier(&self) -> (Epoch, Epoch) {
        (
            self.live_epoch().unwrap_or(Epoch::new(0)),
            self.highest_entered_epoch,
        )
    }

    /// Re-run [`Self::reconcile_roles`] for the live epoch — the epoch the
    /// verified marshal tip is in. The edges that carry no epoch of their own
    /// (tip / share / spawn_unblocked / the module's own wake-up) reconcile this.
    ///
    /// There is no "still the frontier?" guard any more. The guard existed
    /// because the epoch came from a CACHE of the last boundary delivery, which a
    /// frontier move could leave behind — re-running a reconcile over such an
    /// epoch would soft-enter a verify-only scheme over one registered as
    /// `Signer` (`EpochSchemeProvider` downgrade-refusal churn + `roles`↔provider
    /// divergence). Deriving the epoch from the tip instead of caching it removes
    /// the state the guard was protecting.
    ///
    /// It does NOT make the epoch un-stale-able, and the doc used to claim it
    /// did. `reconcile_roles` re-reads the tip at its own liveness gate
    /// (`is_live_epoch`, the `self.tip.borrow()` inside it) AFTER an
    /// `abort_below(...).await` that does file I/O, so the tip can move — and the
    /// live epoch with it — between the choice here and the gate there, and that
    /// gate can then answer FALSE for the epoch this call picked as live. What
    /// the removal of the guard costs in that window is one `soft_enter` pass,
    /// and what it CANNOT cost is the divergence the guard was for: `soft_enter`
    /// returns early on a running engine or a recorded `Signer` (its first two
    /// lines), so the verify-only registration never lands over a signer entry.
    async fn reconcile_live<HS, HR>(&mut self, muxes: Option<&Muxes<HS, HR>>)
    where
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
    {
        if let Some(epoch) = self.live_epoch() {
            self.reconcile_roles(epoch, muxes).await;
        }
    }

    /// The `Inline::genesis(E)` precondition lookup on the E-1 terminal (boundary)
    /// block — the exact lookup `Inline::genesis` itself performs, so the guard is
    /// precise, not heuristic — which ALSO yields the seedless arm's base for epoch
    /// E. Gate and base come from one `get_block` so it is impossible to spawn on a
    /// block the base was not taken from, and the block is fetched exactly as often
    /// as when this only returned a bool.
    async fn boundary_lookup(&mut self, epoch: Epoch) -> BoundaryLookup {
        let Some(prev) = epoch.get().checked_sub(1).map(Epoch::new) else {
            return BoundaryLookup::NotApplicable; // epoch 0 — genesis needs no predecessor block
        };
        let Some(last) = self.cfg.epocher.last(prev) else {
            return BoundaryLookup::NotApplicable;
        };
        let Some(block) = self.cfg.marshal_mailbox.get_block(last).await else {
            return BoundaryLookup::Missing(MissingInput::BoundaryBlock);
        };
        boundary_base(self.cfg.randomness.as_ref(), prev, block.proposal_view)
    }

    /// Abort engines of all epochs strictly below `current` (exit-at-transition;
    /// see the lifecycle note above the actor). `e < cutoff` only, so a
    /// stale/replayed boundary for an OLD epoch can never abort a newer engine.
    ///
    /// Also PRUNES `deferred_spawns` of every parked epoch `< cutoff` — using the
    /// SAME cutoff as the engine abort. A frontier that advances E-1 → E+1 via a
    /// catch-up span (no boundary delivery for exactly epoch E) leaves
    /// `deferred_spawns[E]` orphaned: E is now below-frontier, will only ever
    /// soft-enter, and its `Inline::genesis(E)` precondition is moot. If it were
    /// left in the set, the `spawn_unblocked` edge would fire `reconcile_live`
    /// (a no-op for the stale E) on EVERY finalized block for the process
    /// lifetime. Pruning here makes an EMPTY set the true "no pending promotion"
    /// signal that the `spawn_unblocked` edge gates on.
    async fn abort_below(&mut self, current: Epoch) {
        let cutoff = current.get();
        let to_drop: Vec<Epoch> = self
            .active_epochs
            .keys()
            .copied()
            .filter(|e| e.get() < cutoff)
            .collect();
        for e in to_drop {
            if let Some(h) = self.active_epochs.remove(&e) {
                h.abort();
                self.roles.insert(e, Role::Verifier);
                info!(?e, "epoch exited (transition)");
            }
        }
        let context = self.context.as_present().clone();
        prune_agreements(
            &context,
            &mut self.dkg_agreements,
            cutoff,
            &self.cfg.partition_prefix,
            &mut self.agreements_swept_to,
        )
        .await;
        self.deferred_spawns.retain(|e| e.get() >= cutoff);
        // Prune `roles` to the trailing scheme-retention window: the sole reader is
        // `soft_enter`'s `roles.get(&epoch)` for the epoch under reconcile
        // (current / near-frontier == cutoff), so entries older than
        // `cutoff − SCHEME_RETENTION_EPOCHS` are never read again — keep current +
        // the trailing window, drop the rest (unbounded across a months-long
        // process otherwise).
        let roles_floor = cutoff.saturating_sub(SCHEME_RETENTION_EPOCHS as u64);
        self.roles.retain(|e, _| e.get() >= roles_floor);
    }

    /// Build + start the per-epoch Simplex engine and register its 3 sub-channels
    /// against the plane-owned Muxers. Returns `false` (spawning nothing) on an
    /// invalid committee snapshot or a muxer-register failure — the epoch stays
    /// un-promoted rather than panicking.
    ///
    /// **`false` schedules no retry, deliberately.** The caller falls through
    /// without inserting into `deferred_spawns`, and the `spawn_unblocked` edge
    /// is gated on that set being non-empty — so the next attempt comes only from
    /// a boundary delivery or from whatever else drives a reconcile, not from
    /// block progress. Closing that gap is not obviously right: both failure
    /// causes are conditions this node cannot resolve by waiting (a muxer route
    /// is owned by another subsystem and will not free itself; an invalid
    /// snapshot is a chain fact), so an every-block retry would re-run the whole
    /// ladder against a state that cannot have changed.
    async fn spawn_engine<HS, HR>(
        &mut self,
        epoch: Epoch,
        snap: ValidatorSetSnapshot,
        scheme: BlsScheme,
        elector: WeightedVrf,
        muxes: Option<&Muxes<HS, HR>>,
    ) -> bool
    where
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
    {
        // The W1 ordering tripwire that used to stand here has moved INSIDE the
        // randomness subsystem (`beacon::surface`), where the store it asserts
        // against still is. It is now structural rather than asserted: the
        // scheme this spawn requires is the return value of the same operation
        // that performs the W1 publish, so publish-happens-before-spawn is a
        // data dependency no refactor can silently reorder.
        //
        // `None` ⇒ a FOLLOWER manager (no plane). A follower's `signer_keypair`
        // is `None`, so `is_member` in `reconcile_roles` is always false → the
        // `Role::Signer` arm that reaches here is never taken. Defend it as a
        // compile-time fact rather than fabricating an idle plane.
        let Some(muxes) = muxes else {
            unreachable!("follower (Option<Muxes>::None) never spawns an engine: is_member==false")
        };
        let (vote_mux, cert_mux, res_mux) = (&muxes.vote, &muxes.cert, &muxes.res);
        let engine_ctx = self.context.with_label("simplex");
        let engine = match EpochEngine::new(
            engine_ctx,
            EpochEngineConfig {
                blocker: self.cfg.blocker.clone(),
                snapshot: snap,
                epoch,
                elector,
                epocher: self.cfg.epocher.clone(),
                app: self.cfg.app.clone(),
                timeouts: self.cfg.timeouts,
                mailbox_size: self.cfg.mailbox_size,
                scheme,
                partition_prefix: self.cfg.partition_prefix.clone(),
                #[cfg(feature = "dpos-devnet-byzantine")]
                byzantine: self.cfg.byzantine,
            },
            self.cfg.marshal_mailbox.clone(),
            self.cfg.slasher_mailbox.clone(),
            self.cfg.spec_exec_mailbox.clone(),
            self.cfg.page_cache.clone(),
        ) {
            Ok(engine) => engine,
            Err(e) => {
                warn!(?epoch, %e, "skipping epoch spawn — invalid committee snapshot");
                return false;
            }
        };
        let vote_sub = match vote_mux.lock().await.register(epoch.get()).await {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    ?epoch,
                    ?e,
                    "skipping epoch spawn — vote muxer register failed"
                );
                return false;
            }
        };
        let cert_sub = match cert_mux.lock().await.register(epoch.get()).await {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    ?epoch,
                    ?e,
                    "skipping epoch spawn — cert muxer register failed"
                );
                return false;
            }
        };
        let res_sub = match res_mux.lock().await.register(epoch.get()).await {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    ?epoch,
                    ?e,
                    "skipping epoch spawn — res muxer register failed"
                );
                return false;
            }
        };
        let handle = engine.start(vote_sub, cert_sub, res_sub);
        self.active_epochs.insert(epoch, handle);
        info!(?epoch, "epoch entered (signer)");
        true
    }
}

/// What the sweep does with the epochs it upgraded: re-drive each one's
/// finalization fetch. A callback rather than the mailbox itself so the sweep's
/// own loop is testable without a marshal.
type RepairHint = Arc<dyn Fn(Vec<Epoch>) -> BoxFuture<'static, ()> + Send + Sync>;

/// The repair sweep's driver: one sweep per wake, never two at once, on a task
/// of its own. Exits when the last wake sender drops (the epoch manager left).
///
/// Wakes COALESCE rather than queue — several arriving during a sweep produce one
/// more sweep, not one each — which is the property that keeps a sweep whose
/// ladder is spending network pulls from stacking concurrent pulls for the same
/// epoch behind the pull throttle.
///
/// The frontier it sweeps under is the one from the wake it consumed, so an
/// in-flight sweep can be running under a frontier the manager has already
/// advanced past. That is safe in ONE direction only, and it is this one: a stale
/// frontier is a LOWER frontier, and the frontier is an upper exclusion bound, so
/// staleness can only make the sweep skip an epoch it could have repaired — never
/// pin under a live epoch's engine. The next wake repairs the skipped one.
async fn run_repair_sweep(
    mut wake: watch::Receiver<(Epoch, Epoch)>,
    provider: EpochSchemeProvider,
    randomness: Arc<dyn Beacon>,
    hint: RepairHint,
) {
    let mut hinted = BTreeSet::new();
    loop {
        let (observed, entered) = *wake.borrow_and_update();
        let upgraded = repair_keyless_schemes(
            &provider,
            randomness.as_ref(),
            &mut hinted,
            observed,
            entered,
        )
        .await;
        if !upgraded.is_empty() {
            hint(upgraded).await;
        }
        if wake.changed().await.is_err() {
            debug!("epoch manager gone — repair sweep stopping");
            return;
        }
    }
}

/// Resolve and attach the cert-seed pin for every registered epoch that is still
/// unpinned and below the live frontier. Returns the epochs this call actually
/// upgraded, newest last.
///
/// The work list comes from the scheme provider rather than from any set the
/// epoch manager keeps. That is the whole point: a shadow set only knows about
/// the epochs whose registration also wrote it, so it misses the bulk catch-up
/// span and `cold_start_register`, and it goes stale the moment an epoch is
/// pinned by a path that does not update it.
///
/// **Epochs at or above the frontier are excluded, and not merely as a
/// nicety.** A live epoch's scheme belongs to its engine, and this sweep is a
/// SECOND writer of a field that engine also writes — it runs on its own task,
/// asynchronously with the reconcile that registers the scheme, so a pin applied
/// here would race the registration rather than follow it. The exclusion keeps the
/// two writes in one task instead of ordering them across two.
///
/// The live epoch is not left unrepaired by that: `reconcile_live` re-runs the
/// whole ladder on every non-boundary edge — including the `KeyAvailable` arm that
/// also wakes this sweep — and [`EpochSchemeProvider::register`] accepts an
/// unpinned → pinned replacement, so the frontier's entry upgrades in the task
/// that owns it.
///
/// **The frontier is the higher of the two epochs this node has evidence for,
/// and it has to be both.** `observed` is the LIVE epoch — `epoch_of(verified
/// marshal tip)` — which is the tighter bound whenever there is one, but it is
/// `Epoch(0)` while the geometry has not frozen, and a sweep scoped to it alone
/// would then discard every registered epoch. `entered` is the highest epoch
/// whose scheme this node has registered THROUGH A RECONCILE, advanced on every
/// boundary delivery, so it is evidence available in that window too.
///
/// `entered` is NOT "every epoch that holds a scheme", and the doc used to say
/// it was. The marshal's `CertProvider` is the committee module itself, so a
/// certificate arriving for an epoch this node never reconciled registers that
/// epoch's scheme on the read the verification needs — without passing through
/// `reconcile_roles`, where `highest_entered_epoch` is advanced. A node whose
/// execution lags therefore holds schemes ABOVE `entered` (the 4.2 stand test
/// `a_node_three_epochs_behind_registers_the_schemes_and_spawns_no_engine` is
/// exactly that state). What still holds is the conclusion the exclusion needs:
/// the LIVE epoch is at or above the frontier under either input, because the
/// `live` leg is the live epoch itself — and the epochs the `CertProvider` path
/// registers above `entered` are below it, which is what makes them sweep
/// candidates rather than an exclusion problem.
///
/// **A wrong key is terminal for that epoch on this node** — everything reading
/// the key store treats it as the network's `PK_epoch` — so this path asks for
/// the THOROUGH resolution rather than the cheap one. A locally reconstructed
/// key that diverged from the network's (soak 2026-07-14) would make the marshal
/// reject every valid certificate of the epoch. The promote path can afford to
/// act on this node's own DKG material because it has a value-gate that demotes
/// on a mismatch; this path has none and should not grow one.
///
/// That is what [`PinEffort::Thorough`] means here, and it is the whole reason
/// this call site does not simply reuse the cheap effort the vote paths use.
/// WHICH provenance it admits and which it refuses is stated once, inside
/// [`Beacon::ensure_key`] — deliberately not restated here, because a rule
/// written in two places is a rule that will be changed in one.
/// `hinted` is the sweep's OWN memo of epochs it has already re-driven, and it
/// has to exist now: the pin it used to apply was write-once, so `apply_pin`
/// returning `true` exactly once WAS the dedup. A key store is idempotent — it
/// cannot tell the caller that this write was the first — so the "hint once per
/// epoch" rule needs somewhere of its own to live, or every wake re-hints every
/// resolvable epoch for as long as one stays keyless.
///
/// The memo is bounded by the registry, not by time: it is intersected with the
/// candidate list on every pass, so an epoch that ages out of
/// [`crate::SCHEME_RETENTION_EPOCHS`] leaves it too.
async fn repair_keyless_schemes(
    provider: &EpochSchemeProvider,
    randomness: &dyn Beacon,
    hinted: &mut BTreeSet<Epoch>,
    observed: Epoch,
    entered: Epoch,
) -> Vec<Epoch> {
    let frontier = observed.max(entered);
    let candidates = provider.verifier_epochs();
    let mut upgraded = Vec::new();
    for &epoch in candidates.iter().filter(|e| **e < frontier) {
        if hinted.contains(&epoch) {
            continue;
        }
        // The work list is keyed on the KEY STORE rather than on the scheme: a
        // scheme reads its epoch key live through its oracle, so there is nothing
        // to repair in the registry and an epoch already resolvable locally costs
        // no network. `Local` is not merely a probe — it RESOLVES, and what it
        // resolves is written into the store the oracle reads.
        //
        // `Thorough` carries the provenance floor and the network rung; both are
        // the provider's business — see this function's docs for why the floor is
        // mandatory on a path whose write is terminal.
        if !randomness.ensure_key(epoch.get(), PinEffort::Local).await
            && !randomness
                .ensure_key(epoch.get(), PinEffort::Thorough)
                .await
        {
            continue;
        }
        hinted.insert(epoch);
        upgraded.push(epoch);
    }
    hinted.retain(|e| candidates.contains(e));
    upgraded
}

/// The live epoch for a marshal tip, over the frozen geometry — the body of
/// [`Actor::live_epoch`], as a free function so the rule itself is unit-testable
/// (an `Actor` cannot be constructed in a unit test: `Config::marshal_mailbox` is
/// `marshal::core::Mailbox`, whose constructor is `pub(crate)` upstream).
///
/// Its ONLY inputs are the geometry and a height the marshal has verified and
/// stored. That is the whole of R-003's closure: there is no parameter a peer
/// can supply, so no epoch tag off the wire can name the live epoch, and no
/// quorum of reporters is needed to disbelieve one.
fn live_epoch_of(geometry: &crate::committee::Geometry, tip: u64) -> Epoch {
    let e = geometry.epoch_of(tip);
    // `tip == last(e)` ⇒ epoch `e` is finished and `e + 1` is live. See
    // `Actor::live_epoch` for why this arm is what makes the rule total.
    Epoch::new(if geometry.last(e) == tip { e + 1 } else { e })
}

/// The role gate itself — [`Actor::is_live_epoch`]'s body, free for the same
/// reason [`live_epoch_of`] is.
///
/// EQUALITY, and it is the whole difference from the `epoch >=
/// highest_observed_epoch` this replaced: that gate had to fail SAFE over an
/// input it could not trust, so it called every epoch at or above the (possibly
/// inflated) frontier live. Over a verified tip there is exactly one live epoch,
/// and an epoch on either side of it — a stale one the network has left, a
/// future one nothing has finalized — gets a verify-only scheme and no engine.
fn is_live_epoch_at(geometry: &crate::committee::Geometry, tip: u64, epoch: Epoch) -> bool {
    live_epoch_of(geometry, tip) == epoch
}

/// The epochs ONE committee wake-up must reconcile, in order, and the set of
/// owed reconciles it clears by doing so.
///
/// Two groups, and the order between them is the point. FIRST every epoch whose
/// reconcile deferred on an unreadable committee
/// ([`Actor::deferred_reconciles`]) — such an epoch has not run its first
/// reconcile at all: no bookkeeping, no `soft_enter`, no registered scheme.
/// THEN the live epoch, which may already be in the first group and must not be
/// reconciled twice for one wake-up.
///
/// The deferred set is DRAINED rather than filtered: `reconcile_roles` re-enters
/// the epoch itself if the committee is still unreadable, so the set after this
/// call describes the reads that failed on THIS wake-up and not the ones that
/// failed on an earlier one.
///
/// Extracted as a free fn over the two state pieces because an `Actor` cannot be
/// constructed in a unit test (`Config::marshal_mailbox` is
/// `marshal::core::Mailbox`, whose constructor is `pub(crate)` upstream), so the
/// ordering invariant would otherwise be unobservable outside the stand.
fn committee_wake_targets(deferred: &mut BTreeSet<Epoch>, live: Option<Epoch>) -> Vec<Epoch> {
    let mut targets: Vec<Epoch> = Vec::new();
    targets.extend(std::mem::take(deferred));
    if let Some(epoch) = live {
        if !targets.contains(&epoch) {
            targets.push(epoch);
        }
    }
    targets
}

/// True when a per-epoch engine `Handle` has COMPLETED (its task finished — normal
/// exit or a panic caught by the runtime's `catch_panics(true)`, which completes
/// the handle without propagating to the manager). Commonware `Handle` exposes only
/// `abort()` + `impl Future` (no `is_finished`) and is `Unpin`, so we poll it once
/// with a no-op waker: `Ready` ⇒ dead, `Pending` ⇒ still running. Extracted as a
/// free fn (mirroring `live_epoch_of` / `committee_wake_targets`) so the
/// alive/completed/aborted branches are unit-testable on real spawned handles.
fn engine_handle_dead(handle: &mut Handle<()>) -> bool {
    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    matches!(Pin::new(handle).poll(&mut cx), Poll::Ready(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        beacon::testing::BeaconResolve,
        beacon::testing::LiveBeaconConfig,
        beacon::testing::DETERMINISTIC_BOOTSTRAP_EPOCH,
        beacon::testing::{AgreedKeys, BeaconKeys, KeySource, KeySources},
        outer::EpochSchemeProvider,
        scheme::epoch_committee_from_snapshot,
    };
    use alloy_primitives::{Address, B256};
    use commonware_codec::DecodeExt;
    use commonware_consensus::simplex::types::{Proposal, Subject};
    use commonware_consensus::types::{Round as SimplexRound, View};
    use commonware_cryptography::sha256::Digest as Sha256Digest;
    use commonware_cryptography::{
        bls12381::{dkg::deal_anonymous, primitives::variant::MinSig},
        ed25519::PrivateKey as Ed25519PrivateKey,
        Signer,
    };
    use commonware_math::algebra::Random as _;
    use commonware_parallel::Sequential;
    use commonware_utils::{test_rng, N3f1, NZU32};
    use fluentbase_bls::beacon::GroupPublic;

    use fluentbase_bls::scheme::build_signer;
    use fluentbase_bls::BlsPubkey;
    use fluentbase_staking_reader::reader::{ConsensusKeys, ValidatorWithKeys};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;

    /// Committee fixture for the repair-sweep tests. Returns the snapshot and the
    /// BLS keypairs behind it, because one test has to build a SIGNER scheme and
    /// that needs a keypair the committee actually contains.
    /// A provider over the given ladder pieces, so the sweep tests keep
    /// exercising the REAL ladder (store tiering, floor, rung order) rather
    /// than canned answers — that behaviour is what these tests exist to pin.
    fn randomness_over(
        store: BeaconKeys,
        held: Option<AgreedKeys>,
        pull: Option<AgreedKeys>,
    ) -> Arc<dyn Beacon> {
        randomness_over_seeds(crate::beacon::testing::SeedStore::new(), store, held, pull)
    }

    fn randomness_over_seeds(
        seeds: crate::beacon::testing::SeedStore,
        store: BeaconKeys,
        held: Option<AgreedKeys>,
        pull: Option<AgreedKeys>,
    ) -> Arc<dyn Beacon> {
        crate::beacon::testing::LiveBeacon::build(LiveBeaconConfig {
            seeds,
            keys: store,
            resolver: Arc::new(|_| BeaconResolve::Absent),
            ceremony: Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new())),
            dkg_qual: Arc::new(|_| Some(false)),
            held,
            pull,
            metrics: crate::beacon::testing::BeaconMetrics::default(),
            chain_id: 1,
            artifacts: crate::beacon::testing::ArtifactStore::new(),
            geometry: tokio::sync::watch::channel(Some((0, 1))).1,
        })
    }

    fn repair_fixture(epoch: Epoch) -> (ValidatorSetSnapshot, Vec<ValidatorBlsKeypair>) {
        let mut keypairs = Vec::new();
        let validators = (0..4u8)
            .map(|i| {
                let mut rng = StdRng::seed_from_u64(0x5EED + i as u64);
                let kp = ValidatorBlsKeypair::generate(&mut rng);
                let bls_pubkey = BlsPubkey::decode(kp.public_bytes().as_slice()).unwrap();
                keypairs.push(kp);
                ValidatorWithKeys {
                    address: Address::repeat_byte(i),
                    keys: ConsensusKeys {
                        bls_pubkey,
                        peer_pubkey: Ed25519PrivateKey::random(&mut rng).public_key(),
                        activation_epoch: 1,
                    },
                    tombstoned: false,
                }
            })
            .collect();
        (
            ValidatorSetSnapshot {
                block_hash: B256::repeat_byte(0x22),
                block_number: 7,
                epoch: epoch.get(),
                validators,
                weights: None,
            },
            keypairs,
        )
    }

    fn some_group_key() -> GroupPublic {
        let (sharing, _) =
            deal_anonymous::<MinSig, N3f1>(&mut test_rng(), Default::default(), NZU32!(4));
        *sharing.public()
    }

    /// Register `epoch` pin-less the way the bulk catch-up span does — straight
    /// into the provider, touching neither `roles` nor any epoch-manager state.
    /// A group key distinct from [`some_group_key`].
    fn some_other_group_key() -> GroupPublic {
        let mut rng = StdRng::seed_from_u64(0xC0FFEE);
        let (sharing, _) = commonware_cryptography::bls12381::dkg::deal_anonymous::<
            commonware_cryptography::bls12381::primitives::variant::MinSig,
            commonware_utils::N3f1,
        >(&mut rng, Default::default(), commonware_utils::NZU32!(4));
        *sharing.public()
    }

    /// Register `epoch` the way the bulk catch-up span does — straight into the
    /// provider, touching neither `roles` nor any epoch-manager state.
    ///
    /// It takes the randomness handle for the same reason the span itself does:
    /// the scheme's verification strength comes from the oracle
    /// [`Beacon::oracle_for`] attaches, and a fixture that hardcoded `None`
    /// would build a scheme permanently pinned to vote-only admission — which is
    /// exactly the regression the sweep tests were green on.
    fn register_verifier(committee: &dyn Committee, epoch: Epoch, r: &dyn Beacon) {
        let (snap, _) = repair_fixture(epoch);
        let bimap = epoch_committee_from_snapshot(&snap)
            .expect("valid committee")
            .bimap;
        let scheme = fluentbase_bls::scheme::build_verifier(
            &fluentbase_bls::fluent_namespace(1),
            bimap,
            epoch.get(),
            r.oracle_for(epoch.get()),
        );
        assert!(
            committee.upgrade_scheme(epoch.get(), scheme),
            "the fixture module accepts every upgrade"
        );
    }

    /// A committee module holding only what the sweep reads — the scheme map.
    /// It stands in for the production store the same way the sweep sees it: the
    /// work list IS `verifier_epochs()`, and an epoch is on it because something
    /// read its committee.
    fn sweep_committee() -> Arc<crate::committee::testing::SchemeCommittee> {
        crate::committee::testing::SchemeCommittee::new(|_| None)
    }

    /// A finalization certificate over `epoch`'s committee whose multisig half is
    /// GENUINE and whose seed slot carries a signature from an unrelated dealing.
    ///
    /// Both halves matter. The multisig quorum has to verify, or the seed arm is
    /// never reached and the test would pass for the wrong reason; the seed has
    /// to be a well-formed G1 point, or it would be refused as garbage rather
    /// than as the wrong seed.
    fn cert_with_a_foreign_seed(
        epoch: Epoch,
    ) -> (
        Proposal<Sha256Digest>,
        fluentbase_bls::combined_scheme::CombinedCertificate,
    ) {
        use commonware_cryptography::certificate::Scheme as _;
        let (snap, keypairs) = repair_fixture(epoch);
        let committee = epoch_committee_from_snapshot(&snap).expect("valid committee");
        let ns = fluentbase_bls::fluent_namespace(1);
        let signers: Vec<_> = keypairs
            .iter()
            .map(|kp| {
                build_signer(&ns, committee.bimap.clone(), kp, epoch.get(), None)
                    .expect("committee member")
            })
            .collect();

        let proposal = Proposal::new(
            SimplexRound::new(epoch, View::new(9)),
            View::new(8),
            Sha256Digest::decode([7u8; 32].as_slice()).unwrap(),
        );
        let subject = Subject::Finalize {
            proposal: &proposal,
        };
        let atts: Vec<_> = signers
            .iter()
            .map(|s| s.sign::<Sha256Digest>(subject).expect("sign"))
            .collect();
        let mut cert = signers[0]
            .assemble::<_, N3f1>(atts, &Sequential)
            .expect("assemble");

        // One member's partial from a DIFFERENT dealing: a real G1 signature over
        // the right round under the wrong group.
        let (_, shares) =
            deal_anonymous::<MinSig, N3f1>(&mut test_rng(), Default::default(), NZU32!(4));
        cert.seed = Some(
            fluentbase_bls::beacon::sign_seed_partial(
                &shares[0],
                &fluentbase_bls::beacon::seed_namespace(&ns),
                SimplexRound::new(epoch, View::new(9)),
            )
            .value,
        );
        (proposal, cert)
    }

    /// The premise the boundary rests on, pinned so a later change to
    /// `soft_enter`'s registration cannot quietly remove it.
    ///
    /// A node that followed epoch E as a VERIFIER — which is what a fresh member
    /// of `committee[E+1]` with zero overlap was during E — must have E in
    /// `verifier_epochs()`, because that list is the sweep's work list. Once the
    /// frontier passes E the sweep is the thing that fetches `PK_E`, and without
    /// it the σ this node captured at ingress could never leave quarantine and
    /// the first block of E+1 would have nothing to witness with.
    #[tokio::test]
    async fn a_soft_entered_epoch_is_swept_for_its_key_once_the_frontier_passes_it() {
        let epoch = Epoch::new(DETERMINISTIC_BOOTSTRAP_EPOCH + 3);
        let store = BeaconKeys::new();
        let r = randomness_over(store.clone(), None, None);
        let module = sweep_committee();
        let provider = EpochSchemeProvider::new(module.clone());
        register_verifier(module.as_ref(), epoch, r.as_ref());
        assert!(
            provider.verifier_epochs().contains(&epoch),
            "a soft-entered epoch is on the sweep's work list"
        );

        // While the frontier is still AT the epoch, it is the live one and the
        // sweep leaves it alone.
        assert!(repair_keyless_schemes(
            &provider,
            r.as_ref(),
            &mut BTreeSet::new(),
            epoch,
            Epoch::new(0),
        )
        .await
        .is_empty());

        store.set_pk(epoch.get(), some_group_key(), KeySource::Agreed);
        assert_eq!(
            repair_keyless_schemes(
                &provider,
                r.as_ref(),
                &mut BTreeSet::new(),
                Epoch::new(epoch.get() + 1),
                Epoch::new(0),
            )
            .await,
            vec![epoch],
            "the boundary that needs PK_E is the tick that fetches it"
        );
    }

    /// THE PROPERTY THE SWEEP EXISTS FOR, asserted on the SCHEME instead of on
    /// the sweep's return value.
    ///
    /// `upgraded == vec![epoch]` is what every other sweep test checks, and it is
    /// satisfied by a sweep that resolved a key no registered scheme can read —
    /// which is precisely what a span-registered `oracle: None` verifier was.
    /// Here the same `Arc<BlsScheme>` handle is interrogated before and after the
    /// key lands: it must ADMIT the foreign seed while the epoch is keyless (the
    /// certificate is still carried by a genuine multisig quorum, and refusing
    /// would punish the sender for a local miss) and REFUSE it once `PK_epoch` is
    /// in the store — with NO re-registration in between, which is the whole
    /// point of the oracle reading live state.
    #[tokio::test]
    async fn the_registered_scheme_starts_refusing_a_foreign_seed_when_the_key_lands() {
        use commonware_cryptography::certificate::{Provider as _, Scheme as _};
        let epoch = Epoch::new(DETERMINISTIC_BOOTSTRAP_EPOCH + 3);
        let store = BeaconKeys::new();
        let r = randomness_over(store.clone(), None, None);
        let module = sweep_committee();
        let provider = EpochSchemeProvider::new(module.clone());
        register_verifier(module.as_ref(), epoch, r.as_ref());

        let scheme = provider.scoped(epoch).expect("registered");
        assert!(
            scheme.is_beacon_active(),
            "the span-shaped registration must attach the epoch's oracle — without \
             one the epoch is vote-only for the life of the process and nothing \
             below can ever change"
        );

        let (proposal, cert) = cert_with_a_foreign_seed(epoch);
        let subject = Subject::Finalize {
            proposal: &proposal,
        };
        let mut rng = StdRng::seed_from_u64(0x5EA1);
        assert!(
            scheme.verify_certificate::<_, Sha256Digest, N3f1>(
                &mut rng,
                subject,
                &cert,
                &Sequential
            ),
            "keyless: NoKey admits on the multisig quorum alone"
        );

        store.set_pk(epoch.get(), some_group_key(), KeySource::Agreed);
        assert_eq!(
            repair_keyless_schemes(
                &provider,
                r.as_ref(),
                &mut BTreeSet::new(),
                Epoch::new(epoch.get() + 2),
                Epoch::new(0),
            )
            .await,
            vec![epoch],
        );

        assert!(
            !scheme.verify_certificate::<_, Sha256Digest, N3f1>(
                &mut rng,
                subject,
                &cert,
                &Sequential
            ),
            "keyed: the same registry entry now judges the seed slot and refuses \
             a seed that is not this epoch's"
        );
    }

    /// The sweep runs on EVERY boundary delivery, so the steady state — every
    /// epoch's key already held and already hinted — has to cost nothing.
    ///
    /// Reds if the sweep ever spends the network rung on an epoch whose key the
    /// store already holds: the pull here panics if consulted. Reds too if the
    /// hint memo stops holding, which would re-drive a finalization fetch per
    /// epoch on every boundary for as long as anything stays keyless.
    #[tokio::test]
    async fn a_sweep_over_held_keys_spends_no_network_and_hints_once() {
        let epoch = Epoch::new(5);
        let module = sweep_committee();
        let provider = EpochSchemeProvider::new(module.clone());
        // "Nothing to repair" now means the KEY STORE holds the epoch's key, not
        // that the scheme carries a pin.
        let store = BeaconKeys::new();
        store.set_pk(epoch.get(), some_group_key(), KeySource::Agreed);

        let exploding = AgreedKeys::new(
            Arc::new(|_| {
                Box::pin(async { panic!("an epoch whose key is held must not be resolved for") })
            }),
            Arc::new(|_| Some(true)),
        );
        let r = randomness_over(store, Some(exploding), None);
        register_verifier(module.as_ref(), epoch, r.as_ref());
        let mut hinted = BTreeSet::new();

        assert_eq!(
            repair_keyless_schemes(
                &provider,
                r.as_ref(),
                &mut hinted,
                Epoch::new(7),
                Epoch::new(0),
            )
            .await,
            vec![epoch],
            "the first pass still re-drives the epoch once — it may have been \
             admitting certificates vote-only before the key landed"
        );
        assert!(
            repair_keyless_schemes(
                &provider,
                r.as_ref(),
                &mut hinted,
                Epoch::new(7),
                Epoch::new(0),
            )
            .await
            .is_empty(),
            "and every pass after it is inert"
        );
    }

    /// The scope half of the gap this phase closes. The edge this replaces looked
    /// at ONE cached epoch — the most recently boundary-delivered — so an epoch
    /// that missed its own boundary window was never retried again. Here the key
    /// for the OLDER of two unpinned epochs is the one that resolves; a sweep
    /// scoped to the newest would report nothing.
    ///
    /// Reds if `verifier_epochs()` is narrowed back to a single entry.
    #[tokio::test]
    async fn the_sweep_repairs_every_unpinned_epoch_not_just_the_newest() {
        let (older, newer) = (Epoch::new(5), Epoch::new(6));
        let module = sweep_committee();
        let provider = EpochSchemeProvider::new(module.clone());
        let store = BeaconKeys::new();
        store.set_pk(older.get(), some_group_key(), KeySource::Agreed);
        let r = randomness_over(store.clone(), None, None);
        register_verifier(module.as_ref(), older, r.as_ref());
        register_verifier(module.as_ref(), newer, r.as_ref());

        let upgraded = repair_keyless_schemes(
            &provider,
            r.as_ref(),
            &mut BTreeSet::new(),
            Epoch::new(7),
            Epoch::new(0),
        )
        .await;

        assert_eq!(upgraded, vec![older]);
    }

    /// `verify_certificate` reads ORACLE presence as "this epoch is
    /// beacon-active" and rejects every SEEDLESS certificate under it.
    /// Pre-bootstrap epochs are legitimately seedless, so both halves have to
    /// stay away from them: `ensure_key` must not resolve a key there, and
    /// `oracle_for` must not attach an oracle there.
    ///
    /// Reds if the `DETERMINISTIC_BOOTSTRAP_EPOCH` guard leaves either.
    #[tokio::test]
    async fn the_sweep_never_pins_below_the_bootstrap_epoch() {
        use commonware_cryptography::certificate::Provider as _;
        let pre_beacon = Epoch::new(DETERMINISTIC_BOOTSTRAP_EPOCH - 1);
        let module = sweep_committee();
        let provider = EpochSchemeProvider::new(module.clone());
        let store = BeaconKeys::new();
        store.set_pk(pre_beacon.get(), some_group_key(), KeySource::Agreed);
        let r = randomness_over(store.clone(), None, None);
        register_verifier(module.as_ref(), pre_beacon, r.as_ref());
        assert!(
            !provider
                .scoped(pre_beacon)
                .expect("registered")
                .is_beacon_active(),
            "and no oracle is attached below the bootstrap epoch either — an \
             oracle IS the beacon-active claim, and it would reject every legal \
             seedless certificate of a pre-beacon epoch"
        );

        let upgraded = repair_keyless_schemes(
            &provider,
            r.as_ref(),
            &mut BTreeSet::new(),
            Epoch::new(7),
            Epoch::new(0),
        )
        .await;

        assert!(upgraded.is_empty());
    }

    /// A signer's engine resolves its own epoch key through the promote gates,
    /// which have a value-gate this sweep does not: the sweep's `Thorough` rung
    /// writes what it resolves into the shared store as the network's
    /// `PK_epoch`, and a wrong one there rejects every legal certificate of the
    /// epoch. Under an engine that already judged its own material, spending
    /// that rung is redundant work with a terminal failure mode.
    ///
    /// Reds if the `me().is_none()` filter leaves `verifier_epochs()`.
    #[tokio::test]
    async fn the_sweep_never_touches_a_signer_scheme() {
        let epoch = Epoch::new(5);
        let (snap, keypairs) = repair_fixture(epoch);
        let committee = epoch_committee_from_snapshot(&snap).expect("valid committee");
        let signer = build_signer(
            &fluentbase_bls::fluent_namespace(1),
            committee.bimap,
            &keypairs[0],
            epoch.get(),
            None,
        )
        .expect("keypair is a committee member");
        let module = sweep_committee();
        let provider = EpochSchemeProvider::new(module.clone());
        assert!(module.upgrade_scheme(epoch.get(), signer));

        let store = BeaconKeys::new();
        store.set_pk(epoch.get(), some_group_key(), KeySource::Agreed);

        let upgraded = repair_keyless_schemes(
            &provider,
            randomness_over(store.clone(), None, None).as_ref(),
            &mut BTreeSet::new(),
            Epoch::new(7),
            Epoch::new(0),
        )
        .await;

        assert!(upgraded.is_empty());
    }

    /// The coverage the provider-sourced work list buys over an epoch-manager
    /// shadow set: the bulk catch-up span registers straight into the provider
    /// and writes no manager state at all, so a shadow-sourced sweep is blind to
    /// every epoch it registered — which is the whole of a deep catch-up.
    ///
    /// Reds if the work list is re-sourced from `roles` (nothing put this epoch
    /// there).
    #[tokio::test]
    async fn a_span_registered_epoch_is_repairable() {
        let epoch = Epoch::new(5);
        let module = sweep_committee();
        let provider = EpochSchemeProvider::new(module.clone());
        let store = BeaconKeys::new();
        store.set_pk(epoch.get(), some_group_key(), KeySource::Agreed);
        let r = randomness_over(store.clone(), None, None);
        register_verifier(module.as_ref(), epoch, r.as_ref());

        let upgraded = repair_keyless_schemes(
            &provider,
            r.as_ref(),
            &mut BTreeSet::new(),
            Epoch::new(7),
            Epoch::new(0),
        )
        .await;

        assert_eq!(upgraded, vec![epoch]);
    }

    /// The half of the frontier that is not the live epoch. Before the geometry
    /// freezes there IS no live epoch, so the `observed` leg is `Epoch(0)` —
    /// under which every registered epoch is at or above the frontier and the
    /// sweep discards all of them. The epoch a boundary delivery entered is the
    /// evidence available in that window.
    ///
    /// Reds if the sweep is re-scoped to `observed` alone.
    #[tokio::test]
    async fn the_sweep_reaches_below_an_entered_epoch_with_no_live_epoch_yet() {
        let epoch = Epoch::new(5);
        let module = sweep_committee();
        let provider = EpochSchemeProvider::new(module.clone());
        let store = BeaconKeys::new();
        store.set_pk(epoch.get(), some_group_key(), KeySource::Agreed);

        let r = randomness_over(store.clone(), None, None);
        register_verifier(module.as_ref(), epoch, r.as_ref());
        let no_live_epoch = Epoch::new(0);
        assert!(
            repair_keyless_schemes(
                &provider,
                r.as_ref(),
                &mut BTreeSet::new(),
                no_live_epoch,
                Epoch::new(0),
            )
            .await
            .is_empty(),
            "with no frontier evidence at all the sweep must stay inert — the \
             pre-freeze state, kept here so the assertion below is not vacuous"
        );

        // The boundary for epoch 6 lands and `reconcile_roles` enters it.
        assert_eq!(
            repair_keyless_schemes(
                &provider,
                r.as_ref(),
                &mut BTreeSet::new(),
                no_live_epoch,
                Epoch::new(6),
            )
            .await,
            vec![epoch]
        );
    }

    /// The caller re-drives a marshal fetch per returned epoch, so the return
    /// value has to be the keyless→keyed TRANSITION and not "every epoch I looked
    /// at". Otherwise every trigger re-hints for as long as anything is keyless,
    /// and on a catch-up node that is every boundary.
    ///
    /// The write-once pin used to give this for free. It is the `hinted` memo
    /// now, and this is the test that says so.
    #[tokio::test]
    async fn a_repaired_epoch_re_drives_its_finalization_hint_exactly_once() {
        let epoch = Epoch::new(5);
        let module = sweep_committee();
        let provider = EpochSchemeProvider::new(module.clone());
        let store = BeaconKeys::new();
        store.set_pk(epoch.get(), some_group_key(), KeySource::Agreed);

        let r = randomness_over(store.clone(), None, None);
        register_verifier(module.as_ref(), epoch, r.as_ref());
        let mut hinted = BTreeSet::new();
        let sweep = async |hinted: &mut BTreeSet<Epoch>| {
            repair_keyless_schemes(&provider, r.as_ref(), hinted, Epoch::new(7), Epoch::new(0))
                .await
        };
        assert_eq!(sweep(&mut hinted).await, vec![epoch]);
        assert_eq!(sweep(&mut hinted).await, Vec::<Epoch>::new());
    }

    /// Everything that verifies a certificate of an epoch reads its key from the
    /// shared store, so a key this node reconstructed ITSELF and got wrong (soak
    /// 2026-07-14) makes every valid certificate of that epoch reject. Dropping
    /// the own-DKG rung does NOT prevent it: W1/W3 write the same locally derived
    /// key into that store, and the store rung answers before `held`/`pull` are
    /// consulted.
    ///
    /// The floor is a floor and not an `Agreed`-only gate, because that would
    /// hollow out rung 1: `Carried` is derived from chain facts alone (an
    /// attested mint plus the chain's `dkgQual` bit) and is how the ladder
    /// memoises a resolved carry.
    ///
    /// Reds if the store rung goes back to being provenance-blind.
    #[tokio::test]
    async fn the_sweep_refuses_a_locally_derived_store_key() {
        let (local, carried) = (Epoch::new(5), Epoch::new(6));
        let module = sweep_committee();
        let provider = EpochSchemeProvider::new(module.clone());
        let store = BeaconKeys::new();
        store.set_pk(local.get(), some_group_key(), KeySource::LocalDkg);
        store.set_pk(carried.get(), some_other_group_key(), KeySource::Carried);
        let r = randomness_over(store.clone(), None, None);
        register_verifier(module.as_ref(), local, r.as_ref());
        register_verifier(module.as_ref(), carried, r.as_ref());

        let upgraded = repair_keyless_schemes(
            &provider,
            r.as_ref(),
            &mut BTreeSet::new(),
            Epoch::new(9),
            Epoch::new(0),
        )
        .await;

        assert_eq!(upgraded, vec![carried]);
    }

    /// The sweep spends one bounded peer pull per unpinned epoch, and that pull
    /// SLEEPS on its caller-side rate bound before it even issues the fetch — up
    /// to `SCHEME_RETENTION_EPOCHS` × (`PULL_MIN_INTERVAL` + `PULL_TIMEOUT`) of
    /// awaiting per sweep. Awaited on the epoch manager's `select!`, that is a
    /// ~100 s window in which the beacon wake-up arm, `spawn_unblocked` and the
    /// tip edge do not run — three of the arms that turn this node into a
    /// signer.
    ///
    /// So the driver here is the manager's loop in miniature: a second arm with
    /// work queued on it, and the sweep wake sent from that arm exactly as the
    /// manager sends it. The whole window under test is the one in which the
    /// sweep is parked inside its pull, and the assertion is that the second arm
    /// is served throughout it. Under the pre-fix inline shape that count is
    /// zero, and `parked.notified()` below is never even reached.
    ///
    /// The timeout is there so a regression FAILS instead of hanging the suite.
    /// It costs nothing on the passing path — the future is dropped, not awaited.
    #[tokio::test]
    async fn a_parked_sweep_does_not_hold_its_driver() {
        const OTHER_ARM_EVENTS: usize = 8;
        let epoch = Epoch::new(5);
        let module = sweep_committee();
        let provider = EpochSchemeProvider::new(module.clone());
        register_verifier(
            module.as_ref(),
            epoch,
            randomness_over(BeaconKeys::new(), None, None).as_ref(),
        );

        // Both directions are permit-storing, so neither side can miss the
        // other's signal by being late to await it.
        let parked = Arc::new(Notify::new());
        let gate = Arc::new(Notify::new());
        let pulls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let pk = some_group_key();
        let pull = AgreedKeys::new(
            Arc::new({
                let (parked, gate, pulls) = (parked.clone(), gate.clone(), pulls.clone());
                move |_| {
                    let (parked, gate, pulls) = (parked.clone(), gate.clone(), pulls.clone());
                    Box::pin(async move {
                        pulls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        parked.notify_one();
                        gate.notified().await;
                        Some(pk)
                    }) as BoxFuture<'static, Option<GroupPublic>>
                }
            }),
            Arc::new(|_| Some(true)),
        );

        let (hint_tx, mut hint_rx) = mpsc::unbounded_channel();
        let hint: RepairHint = Arc::new(move |upgraded| {
            let hint_tx = hint_tx.clone();
            Box::pin(async move {
                drop(hint_tx.send(upgraded));
            }) as BoxFuture<'static, ()>
        });

        let frontier = (Epoch::new(9), Epoch::new(0));
        let (wake, wake_rx) = watch::channel(frontier);
        let sweep = tokio::spawn(run_repair_sweep(
            wake_rx,
            provider.clone(),
            randomness_over(BeaconKeys::new(), None, Some(pull)),
            hint,
        ));

        let served = tokio::time::timeout(std::time::Duration::from_secs(30), async move {
            let (work_tx, mut work_rx) = mpsc::unbounded_channel();
            for _ in 0..OTHER_ARM_EVENTS {
                work_tx.send(()).expect("receiver is alive");
            }
            parked.notified().await;

            let mut served = 0usize;
            while served < OTHER_ARM_EVENTS {
                tokio::select! {
                    Some(()) = work_rx.recv() => {
                        served += 1;
                        wake.send_replace(frontier);
                    }
                }
            }
            assert_eq!(
                pulls.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "wakes arriving during a sweep must coalesce into one more sweep, \
                 not stack concurrent pulls behind the pull throttle"
            );

            gate.notify_one();
            assert_eq!(
                hint_rx.recv().await,
                Some(vec![epoch]),
                "the released sweep still re-drives the epoch it pinned"
            );
            served
        })
        .await
        .expect("a sweep awaited on its driver never lets the driver reach these assertions");

        assert_eq!(served, OTHER_ARM_EVENTS);
        sweep.abort();
    }

    /// The seedless arm must inherit the E-1 terminal block's witness whenever the
    /// block carries one, and fall back to the predictable constant derivation ONLY
    /// where no witness can exist — a witness-less block or no predecessor epoch.
    /// `Missing` is not a base at all: that epoch defers its spawn.
    ///
    /// The fixture's committee is empty on purpose: what is under test is which arm
    /// is selected, not the constant derivation's own inputs (pinned in
    /// `weighted_vrf`).
    #[test]
    fn the_witness_base_is_inherited_whenever_the_boundary_block_carries_one() {
        let snap = ValidatorSetSnapshot {
            block_hash: B256::repeat_byte(0x11),
            block_number: 42,
            epoch: 9,
            validators: Vec::new(),
            weights: None,
        };
        let witness = [0xA7u8; 32];
        let constant = constant_fallback_seed(&snap);
        assert_ne!(witness, constant);

        assert_eq!(
            seedless_base(
                &BoundaryLookup::Present {
                    seed: Some(witness)
                },
                &snap
            ),
            Some(SeedlessBase::Witness(witness))
        );
        assert_eq!(
            seedless_base(&BoundaryLookup::Present { seed: None }, &snap),
            Some(SeedlessBase::Constant(constant))
        );
        assert_eq!(
            seedless_base(&BoundaryLookup::NotApplicable, &snap),
            Some(SeedlessBase::Constant(constant))
        );
        for cause in [MissingInput::BoundaryBlock, MissingInput::TerminalSeed] {
            assert_eq!(seedless_base(&BoundaryLookup::Missing(cause), &snap), None);
        }
    }

    /// A store holding a real threshold σ for exactly `round`, so the boundary
    /// lookup reads the production `SeedStore` pin rather than a canned answer.
    fn seeds_holding(round: SimplexRound) -> crate::beacon::testing::SeedStore {
        use crate::beacon::testing::PkOracle;
        use fluentbase_bls::beacon::{recover_seed, seed_namespace, sign_seed_partial};
        let mut rng = test_rng();
        let (sharing, shares) =
            deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(5));
        let ns = seed_namespace(b"fluent-test");
        let partials: Vec<_> = shares
            .iter()
            .map(|share| sign_seed_partial(share, &ns, round))
            .collect();
        let sigma = recover_seed::<N3f1>(&sharing, &partials).expect("the fixture recovers σ");
        let store = crate::beacon::testing::SeedStore::new();
        store.record(PkOracle::new(*sharing.public(), ns).witness(round, sigma));
        store
    }

    /// The base for epoch E is σ of E-1's TERMINAL ROUND, read from the store at
    /// the round the boundary block names — and a local MISS is `Missing`, which
    /// DEFERS the spawn, never `Present { seed: None }`, which would elect on the
    /// constant base while a peer that holds σ elects on the witness one.
    #[test]
    fn a_boundary_seed_miss_defers_the_spawn_instead_of_electing_on_the_constant_base() {
        const TERMINAL_VIEW: u64 = 31;
        let prev = Epoch::new(DETERMINISTIC_BOOTSTRAP_EPOCH + 5);
        let terminal = SimplexRound::new(prev, View::new(TERMINAL_VIEW));
        let snap = ValidatorSetSnapshot {
            block_hash: B256::repeat_byte(0x11),
            block_number: 42,
            epoch: prev.get() + 1,
            validators: Vec::new(),
            weights: None,
        };

        let seeds = seeds_holding(terminal);
        let held = randomness_over_seeds(seeds.clone(), BeaconKeys::new(), None, None);
        let expected = witness_fallback_seed(
            &held
                .terminal_seed(terminal)
                .expect("the fixture pinned this round"),
        );
        assert_eq!(
            boundary_base(held.as_ref(), prev, TERMINAL_VIEW),
            BoundaryLookup::Present {
                seed: Some(expected)
            }
        );
        assert_eq!(
            seedless_base(&boundary_base(held.as_ref(), prev, TERMINAL_VIEW), &snap),
            Some(SeedlessBase::Witness(expected)),
        );

        // The SAME block, on a node whose store holds σ for a NEIGHBOURING round
        // of the same epoch: the pin answers only its own round, so this is the
        // ordinary "σ has not landed here yet" miss.
        let stale = randomness_over_seeds(
            seeds_holding(SimplexRound::new(prev, View::new(TERMINAL_VIEW - 1))),
            BeaconKeys::new(),
            None,
            None,
        );
        assert_eq!(
            boundary_base(stale.as_ref(), prev, TERMINAL_VIEW),
            BoundaryLookup::Missing(MissingInput::TerminalSeed),
            "a σ miss is `I do not know yet`, not `there is no σ` — and it names \
             the SEED as the missing input, not the block"
        );
        assert_eq!(
            seedless_base(&boundary_base(stale.as_ref(), prev, TERMINAL_VIEW), &snap),
            None,
            "and no base at all means the spawn defers"
        );

        // Predicate first: below the bootstrap edge no σ can exist, so `None` is
        // the agreed answer and the store is never consulted for it.
        let inactive = Epoch::new(DETERMINISTIC_BOOTSTRAP_EPOCH - 1);
        assert_eq!(
            boundary_base(
                randomness_over_seeds(
                    seeds_holding(SimplexRound::new(inactive, View::new(TERMINAL_VIEW))),
                    BeaconKeys::new(),
                    None,
                    None,
                )
                .as_ref(),
                inactive,
                TERMINAL_VIEW,
            ),
            BoundaryLookup::Present { seed: None },
            "a beacon-inactive predecessor takes the constant base even with a σ in the store"
        );
    }

    // A single peer (even naming u64::MAX) must NOT advance the live frontier —
    // the P2-11 permanent-soft-enter halt. n = 4 ⇒ f = 1 ⇒ threshold f+1 = 2.

    /// The suppression is only sound because the artifact's entry answers the rung
    /// W1's did — and it is now ALSO the promote value-gate's comparand, which is
    /// what the shrink changed: the gate used to read a finalized boundary block's
    /// outcome, and no block carries one.
    #[tokio::test]
    async fn an_agreed_key_answers_rung_one_and_is_the_value_gate_comparand() {
        let store = BeaconKeys::new();
        let pk = some_group_key();
        store.set_pk(9, pk, KeySource::Agreed);
        assert_eq!(
            store.get_pk(9, KeySources::default()).await,
            Some(pk),
            "the artifact must answer the store rung W1 used to"
        );
        assert_eq!(
            store.attested(9),
            Some(pk),
            "and it is what the promote value-gate compares against"
        );
        // Tiering: a local reconstruction can never displace it, whatever the
        // write order — which is what makes W1 standing down a no-op for
        // readers rather than a loss.
        store.set_pk(9, some_other_group_key(), KeySource::LocalDkg);
        assert_eq!(store.cached_only(9), Some(pk));
        assert_eq!(store.attested(9), Some(pk));
    }

    // `engine_handle_dead` must read PENDING (parked engine) as alive and both
    // COMPLETED (returned) and ABORTED handles as dead — the three branches the
    // respawn gate in `reconcile_roles` diffs on. Under `catch_panics(true)` a
    // panicked child engine surfaces as a COMPLETED handle, so this is the exact
    // signal that revives it.
    #[test]
    fn engine_handle_dead_distinguishes_pending_completed_aborted() {
        use commonware_runtime::{deterministic, Clock, Runner as _, Spawner};
        use std::time::Duration;

        let runner = deterministic::Runner::default();
        runner.start(|ctx| async move {
            let mut pending = ctx.clone().spawn(|c| async move {
                c.sleep(Duration::from_secs(3600)).await;
            });
            let mut done = ctx.clone().spawn(|_| async move {});
            let mut killed = ctx.clone().spawn(|c| async move {
                c.sleep(Duration::from_secs(3600)).await;
            });
            killed.abort();

            // Drive the deterministic scheduler until the completed + aborted
            // handles are observably dead (bounded so a regression fails instead of
            // hanging). Stop polling a handle once dead — a completed handle must
            // not be re-polled, which the production respawn path guarantees by
            // removing the entry.
            let mut done_dead = false;
            let mut killed_dead = false;
            for _ in 0..2_000 {
                if !done_dead {
                    done_dead = engine_handle_dead(&mut done);
                }
                if !killed_dead {
                    killed_dead = engine_handle_dead(&mut killed);
                }
                if done_dead && killed_dead {
                    break;
                }
                ctx.sleep(Duration::from_millis(1)).await;
            }
            assert!(done_dead, "a completed engine eventually reads dead");
            assert!(killed_dead, "an aborted engine eventually reads dead");
            assert!(
                !engine_handle_dead(&mut pending),
                "the still-parked child reads alive throughout"
            );
        });
    }

    /// The agreement instances live in their OWN map and are pruned on the same
    /// frontier cutoff as the engines — and pruning really aborts them, which for
    /// a still-running instance is the only thing that stops it.
    ///
    /// It also WAITS for them. The partition sweep that follows the abort pass
    /// cannot see these epochs any more — the abort pass removed them from the map
    /// the sweep's guard consults — so an instance still stopping while the sweep
    /// runs would have its partition removed and then recreated by its voter's
    /// last journal write, leaving a directory nothing ever reclaims.
    #[test]
    fn agreement_instances_prune_on_the_engine_cutoff_and_are_aborted() {
        use commonware_runtime::{deterministic, Clock, Runner as _, Spawner};
        use std::{
            sync::atomic::{AtomicUsize, Ordering},
            time::Duration,
        };

        /// Set on drop, which for an aborted task is the moment its future is
        /// dropped — the observable proof that `abort()` reached it.
        struct Tombstone(Arc<AtomicUsize>);
        impl Drop for Tombstone {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let runner = deterministic::Runner::timed(Duration::from_secs(10));
        runner.start(|ctx| async move {
            let dropped = Arc::new(AtomicUsize::new(0));
            let mut agreements: BTreeMap<Epoch, Handle<()>> = BTreeMap::new();
            for epoch in [3u64, 4, 5] {
                let mark = Tombstone(dropped.clone());
                agreements.insert(
                    Epoch::new(epoch),
                    ctx.with_label("agreement").spawn(move |_| async move {
                        let _mark = mark;
                        std::future::pending::<()>().await;
                    }),
                );
            }
            // Let the spawned tasks reach their park before anything is aborted.
            ctx.sleep(Duration::from_millis(1)).await;

            prune_agreements(&ctx, &mut agreements, 5, "", &mut 0).await;
            assert_eq!(
                agreements.keys().copied().collect::<Vec<_>>(),
                vec![Epoch::new(5)],
                "the frontier's own instance must survive its cutoff"
            );

            assert_eq!(
                dropped.load(Ordering::SeqCst),
                2,
                "prune returned before the instances it aborted had stopped, so the partition \
                 sweep it runs next races their last journal write"
            );
        });
    }

    /// An agreement supervisor removes its own journal partition after it
    /// delivers, but every EXTERNAL abort — this prune, the SafetyHalt unwind, the
    /// manager's exit — cancels it at an await and that removal never runs. Nothing
    /// else reclaims `dkg_epoch_{n}`, so the prune sweeps the band below the cutoff
    /// itself, by epoch number and not by the map, which also collects what a
    /// previous process left behind.
    #[test]
    fn pruning_reclaims_the_journal_partitions_an_abort_left_behind() {
        use commonware_runtime::{deterministic, Runner as _, Spawner as _, Storage as _};
        use std::time::Duration;

        let runner = deterministic::Runner::timed(Duration::from_secs(10));
        runner.start(|ctx| async move {
            // Epoch 3: aborted by an earlier prune (or a previous process) — the
            // leak. Epoch 5: the frontier's own live instance. Epoch 20: outside the
            // swept band, so a sweep that ignored the band would look identical.
            for epoch in [3u64, 5, 20] {
                ctx.open(&agreement_partition("", epoch), b"blob")
                    .await
                    .expect("partition");
            }
            let mut agreements: BTreeMap<Epoch, Handle<()>> = BTreeMap::new();
            agreements.insert(
                Epoch::new(5),
                ctx.with_label("live")
                    .spawn(move |_| async move { std::future::pending::<()>().await }),
            );

            prune_agreements(&ctx, &mut agreements, 5, "", &mut 0).await;

            assert!(
                ctx.scan(&agreement_partition("", 3)).await.is_err(),
                "the partition an aborted supervisor left behind was never reclaimed"
            );
            assert!(
                ctx.scan(&agreement_partition("", 5)).await.is_ok(),
                "the live instance's own partition must survive"
            );
            assert!(
                ctx.scan(&agreement_partition("", 20)).await.is_ok(),
                "the sweep must stay inside its band"
            );
        });
    }

    /// The band sweep costs `AGREEMENT_SWEEP_SPAN` `Storage::remove` calls, each
    /// one a process-global runtime lock and a directory removal, and
    /// `abort_below` runs it on EVERY reconcile — which is every finalized block,
    /// because the committee module's wake-up publishes unconditionally on every
    /// anchor advance. The band itself moves once per EPOCH, so the repeat is
    /// pure cost.
    ///
    /// What this pins is that the repeat is gone WITHOUT the collection being
    /// gone: a partition that appears under a cutoff already swept is left alone
    /// until the cutoff moves, and then it is collected. The two edges that can
    /// legitimately put a reclaimable partition in the band are the other two
    /// cases — a cutoff the process has never been at (the first test above, and
    /// every new epoch) and an abort this call performed (the test above it,
    /// where the abort is joined before the sweep).
    ///
    /// Falsifier: the partition surviving the cutoff move (the gate is stuck
    /// shut, a real leak); the partition disappearing on the repeat (the gate
    /// never closed and this test proves nothing); the first call not collecting
    /// at all (`swept_to` starting above the cutoff).
    #[test]
    fn a_repeat_prune_at_a_cutoff_already_swept_does_not_touch_storage() {
        use commonware_runtime::{deterministic, Runner as _, Storage as _};
        use std::time::Duration;

        let runner = deterministic::Runner::timed(Duration::from_secs(10));
        runner.start(|ctx| async move {
            let mut agreements: BTreeMap<Epoch, Handle<()>> = BTreeMap::new();
            let mut swept_to = 0u64;

            // A fresh process: `swept_to = 0`, so the first prune sweeps and the
            // leftover goes.
            ctx.open(&agreement_partition("", 4), b"blob")
                .await
                .expect("partition");
            prune_agreements(&ctx, &mut agreements, 5, "", &mut swept_to).await;
            assert!(
                ctx.scan(&agreement_partition("", 4)).await.is_err(),
                "the first prune at a cutoff this process has never been at must sweep"
            );
            assert_eq!(swept_to, 5, "the memo must name the cutoff just swept");

            // The same cutoff again, nothing aborted: the band is where it was, so
            // the partition recreated under it is NOT the sweep's business yet.
            ctx.open(&agreement_partition("", 4), b"blob")
                .await
                .expect("partition");
            prune_agreements(&ctx, &mut agreements, 5, "", &mut swept_to).await;
            assert!(
                ctx.scan(&agreement_partition("", 4)).await.is_ok(),
                "the repeat prune swept the band again — `abort_below` runs per finalized \
                 block, so this is `AGREEMENT_SWEEP_SPAN` filesystem removals per block"
            );

            // The cutoff moves: the band moves with it and the partition goes.
            prune_agreements(&ctx, &mut agreements, 6, "", &mut swept_to).await;
            assert!(
                ctx.scan(&agreement_partition("", 4)).await.is_err(),
                "a cutoff this process has never been at must sweep, or the gate is a leak"
            );
            assert_eq!(swept_to, 6);

            // A prune BELOW the highest cutoff, with an instance to abort: it
            // sweeps its own band (the abort is the edge), and it must not lower
            // the memo — a memo that went backwards would re-sweep on every block
            // until the cutoff climbed back.
            ctx.open(&agreement_partition("", 1), b"blob")
                .await
                .expect("partition");
            agreements.insert(
                Epoch::new(1),
                ctx.with_label("stale")
                    .spawn(move |_| async move { std::future::pending::<()>().await }),
            );
            prune_agreements(&ctx, &mut agreements, 2, "", &mut swept_to).await;
            assert!(
                ctx.scan(&agreement_partition("", 1)).await.is_err(),
                "a prune that aborted an instance must sweep its band whatever the memo says"
            );
            assert_eq!(
                swept_to, 6,
                "the memo must be raised, never assigned: epoch 6's band is still swept"
            );
        });
    }

    // The manager's loop must not turn once a round. The seed class fires at the
    // block rate and belongs to the executor; this actor's `select!` rebuilds two
    // `Notify` futures on every iteration, so answering it with a `continue` in
    // the body would pay that per round where HEAD paid it never.
    #[tokio::test]
    async fn the_seed_class_never_leaves_the_reconcile_wake() {
        let (tx, mut rx) = tokio::sync::broadcast::channel(8);
        for _ in 0..3 {
            tx.send(crate::beacon::BeaconEvent::SeedRecorded).unwrap();
        }
        tx.send(crate::beacon::BeaconEvent::KeyAvailable).unwrap();
        assert_eq!(
            next_reconcile_wake(&mut rx).await,
            Ok(crate::beacon::BeaconEvent::KeyAvailable),
            "three seed records are dropped inside the future, not by the loop"
        );

        tx.send(crate::beacon::BeaconEvent::SeedRecorded).unwrap();
        tx.send(crate::beacon::BeaconEvent::ParticipationChanged)
            .unwrap();
        assert_eq!(
            next_reconcile_wake(&mut rx).await,
            Ok(crate::beacon::BeaconEvent::ParticipationChanged)
        );

        // A dead publisher is RETURNED, not swallowed: it may be hiding a class
        // this actor acts on, and a swallowed `Closed` would park the arm inside
        // a future that can never resolve.
        drop(tx);
        assert!(matches!(
            next_reconcile_wake(&mut rx).await,
            Err(tokio::sync::broadcast::error::RecvError::Closed)
        ));
    }

    /// The live epoch is `epoch_of(verified tip)`, and the epoch AFTER it once
    /// the tip is that epoch's terminal block. Three points, and the third is
    /// the one the old `>=` gate got wrong.
    ///
    /// RED BEFORE THE CHANGE, and the mutation that shows it: replace the
    /// equality in [`Actor::is_live_epoch`] with `epoch >= live`, which is the
    /// shape `is_live_epoch` had while the frontier was corroborated
    /// (`epoch >= highest_observed_epoch`). The first two assertions still pass
    /// — `>=` admits equality — and the third fails: with the tip three epochs
    /// below, `>=` calls the node's own stale epoch live and spawns an engine on
    /// a fork the network has left, which is the defect this rule exists to
    /// close. MUTATION №1, verbatim in the journal.
    #[test]
    fn the_live_epoch_is_the_verified_tips_epoch_and_the_next_one_at_a_terminal() {
        let geometry = crate::committee::Geometry::new(0, 32).expect("nonzero interval");
        let live = |tip: u64| live_epoch_of(&geometry, tip);
        // The PRODUCTION gate, not a mirror of it: `Actor::is_live_epoch` is
        // this call with the geometry and the tip taken off the actor.
        let is_live = |epoch: u64, tip: u64| is_live_epoch_at(&geometry, tip, Epoch::new(epoch));

        // (1) MID-EPOCH. The tip is inside epoch 4 (blocks 128..=159).
        assert_eq!(live(140), Epoch::new(4), "a tip inside E makes E live");
        assert!(is_live(4, 140));
        assert!(
            !is_live(5, 140),
            "the epoch ABOVE the tip is not live either"
        );

        // (2) TERMINAL. `tip == last(4) == 159`: epoch 4 is finished — its
        // terminal block is final and its engine builds no descendant — so the
        // live epoch is 5. Written as a conjunction (`E == epoch_of(tip) ∧ tip <
        // last(E)`) this height would make NO epoch live at all.
        assert_eq!(geometry.last(4), 159);
        assert_eq!(live(159), Epoch::new(5), "at last(E) the live epoch is E+1");
        assert!(!is_live(4, 159));
        assert!(is_live(5, 159));

        // (3) THREE EPOCHS BEHIND. The node's own finalized epoch is 1 while the
        // verified tip sits in epoch 4 — epoch 1 is NOT live, so a member of it
        // registers a verify-only scheme and spawns no engine.
        assert!(
            !is_live(1, 140),
            "an epoch the verified tip has passed is dead"
        );
        assert!(!is_live(2, 140));
        assert!(!is_live(3, 140));

        // Pre-activation heights clamp to epoch 0 (`Geometry::epoch_of`), so a
        // node with no stored finalization at all is live at 0 — the cold start,
        // which must not depend on anyone having reported a tip yet.
        assert_eq!(live(0), Epoch::new(0));
    }

    /// The committee wake-up owes TWO groups, and the deferred one is the group
    /// that has no other retry.
    ///
    /// Reds if the drain is dropped: an epoch whose reconcile deferred on an
    /// unreadable committee is one the tip has usually already passed (a node
    /// catching up crosses boundaries whose committee its anchor could not read
    /// at the time), so `reconcile_live` — the wake-up's only consumer before
    /// this — reconciles the LIVE epoch and never that one. Its scheme is then
    /// never registered and the marshal never verifies that epoch's certificates.
    #[test]
    fn a_committee_wake_up_reconciles_every_deferred_epoch_past_the_live_gate() {
        // Two deferred epochs, neither of them the live one: both are reconciled,
        // which is exactly what `reconcile_live` alone would not have done.
        let mut deferred = BTreeSet::from([Epoch::new(7), Epoch::new(9)]);
        assert_eq!(
            committee_wake_targets(&mut deferred, Some(Epoch::new(11))),
            vec![Epoch::new(7), Epoch::new(9), Epoch::new(11)],
            "a deferred epoch is reconciled on the wake-up whether or not it is live"
        );
        assert!(
            deferred.is_empty(),
            "the set is drained, so what it holds afterwards is what THIS wake-up failed to \
             read — `reconcile_roles` re-enters the epoch itself"
        );

        // The live epoch is reconciled exactly once, even when it is also the
        // epoch that deferred.
        let mut deferred = BTreeSet::from([Epoch::new(9)]);
        assert_eq!(
            committee_wake_targets(&mut deferred, Some(Epoch::new(9))),
            vec![Epoch::new(9)],
            "the live epoch must not be reconciled twice by one wake-up"
        );

        // No live epoch yet (the geometry has not frozen): only the deferred set
        // is owed, and with nothing deferred the wake-up is a no-op.
        let mut deferred = BTreeSet::new();
        assert!(committee_wake_targets(&mut deferred, None).is_empty());
        let mut deferred = BTreeSet::from([Epoch::new(4)]);
        assert_eq!(
            committee_wake_targets(&mut deferred, None),
            vec![Epoch::new(4)],
            "an unfrozen geometry names no live epoch, and owes the deferred set anyway"
        );
    }

    // An overflow is returned too — the run that was dropped may have held either
    // class, so the arm has to apply both effects.
    #[tokio::test]
    async fn an_overflowed_stream_reaches_the_reconcile_arm() {
        let (tx, mut rx) = tokio::sync::broadcast::channel(2);
        for _ in 0..5 {
            tx.send(crate::beacon::BeaconEvent::SeedRecorded).unwrap();
        }
        assert!(matches!(
            next_reconcile_wake(&mut rx).await,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_))
        ));
    }
}
