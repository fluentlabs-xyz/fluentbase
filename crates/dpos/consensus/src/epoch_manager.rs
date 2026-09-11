//! Per-epoch consensus engine lifecycle.
//!
//! Owns the active-epochs map and an event-driven boundary trigger
//! (`mpsc::Receiver<(Epoch, snap)>`) fed by
//! [`fluentbase_staking_reader::EpochTransition`]. The vote/cert/resolver Muxers
//! are NOT owned here — they live in the always-on plane (node crate); this manager
//! receives their `MuxHandle`s + the vote backup forwarder per promotion and
//! registers/deregisters per-epoch sub-channels against them.
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
    dpos::VoteBackupItem,
    engine::{EpochEngine, EpochEngineConfig},
    epocher::OriginEpocher,
    order_block::OrderBlock,
    outer::{EpochSchemeProvider, SharedMux},
    scheme::soft_enter_verifier,
    slasher::Mailbox as SlasherMailbox,
    timeouts::ConsensusTimeouts,
    SCHEME_RETENTION_EPOCHS,
};
use commonware_consensus::{
    marshal::{core::Mailbox as MarshalMailbox, standard::Standard},
    types::{Epoch, Epocher as _, Height, Round, View},
};
use commonware_cryptography::ed25519::PublicKey;
use commonware_p2p::{Blocker, Receiver, Sender};
use commonware_runtime::{
    buffer::paged::CacheRef, spawn_cell, BufferPooler, Clock, ContextCell, Handle, Metrics,
    Spawner, Storage,
};
use commonware_utils::vec::NonEmptyVec;
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
use tracing::{debug, info, warn};

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
/// `is_live` (its f+1-corroborated `highest_observed_epoch` reaches `E`) and its
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
/// A band swept on every prune reclaims them all: it is driven by epoch NUMBER
/// rather than by a map this process populated, so it also collects a previous
/// process's leftovers on the first boundary after a restart, and it re-collects a
/// partition that an aborted-but-not-yet-stopped voter recreated with one last
/// journal write. The band is the trailing scheme-retention window — the same
/// distance the rest of this actor keeps state for.
const AGREEMENT_SWEEP_SPAN: u64 = SCHEME_RETENTION_EPOCHS as u64;

/// Drop every epoch-key agreement instance whose target is below `cutoff`,
/// aborting it on the way out, and reclaim the journal partitions of the targets
/// below the cutoff that no longer have one.
///
/// The SAME cutoff the per-epoch engines are pruned on, and for a matching
/// reason: below the frontier an instance has either delivered its artifact —
/// its supervisor has already returned, so the abort is a no-op — or it is still
/// agreeing a key for an epoch the chain has gone past.
async fn prune_agreements<E: Storage>(
    context: &E,
    agreements: &mut BTreeMap<Epoch, Handle<()>>,
    cutoff: u64,
    partition_prefix: &str,
) {
    let stale: Vec<Epoch> = agreements
        .keys()
        .copied()
        .filter(|e| e.get() < cutoff)
        .collect();
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
            // Nothing to reclaim: the overwhelmingly common case, since the band is
            // swept on every prune whether or not an instance ever ran there.
            Err(commonware_runtime::Error::PartitionMissing(_)) => {}
            Err(err) => warn!(
                epoch,
                ?err,
                "could not reclaim the epoch-key agreement journal partition"
            ),
        }
    }
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

/// Max distinct future epochs one peer may pin on the vote backup channel before
/// its live frontier is corroborated. Two covers the legitimate case (a peer is
/// at most ~1 boundary ahead of what it gossips) with slack; together with the
/// committee bound this caps the corroboration map at `n · 2` epochs and stops a
/// Byzantine minority from crowding out the honest frontier.
const PINS_PER_SENDER: usize = 2;

/// Max epochs to pre-register ahead of the entered tip in ONE catch-up step (the
/// span soft-entered by [`Config::soft_enter_span`] on a single backup-vote hint).
/// Bounds the per-hint catch-up work AND — crucially — MUST stay strictly less
/// than `crate::SCHEME_RETENTION_EPOCHS` (= 8): the marshal verifies the
/// span's finalization certs against schemes the provider retains in a trailing
/// window, so a span wider than that window would evict the low end before the
/// gap-walk reaches it (the cert at the bottom boundary would fail to verify).
const CATCHUP_SPAN_CAP: u64 = 6;

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
    /// Where those handles come from. `None` ⇒ no agreement plane wired ⇒ the
    /// branch parks forever and the map stays empty; the beacon plane holds the
    /// sending half and posts a handle per instance it starts.
    agreement_intake: Option<mpsc::Receiver<(Epoch, Handle<()>)>>,
    boundary_rx: mpsc::Receiver<(Epoch, ValidatorSetSnapshot)>,
    /// Highest epoch we have entered (full or soft) — i.e. the highest epoch
    /// whose committee scheme is registered, so the marshal can verify its
    /// certs. Drives the catch-up hint target. Monotonic; never decremented by
    /// `prune_old` (the scheme provider keeps a trailing window).
    highest_entered_epoch: Epoch,
    /// Highest live-network epoch corroborated by f+1 DISTINCT peers on the vote
    /// backup channel. Gates `is_live_epoch`: epochs below it only soft-enter.
    /// NEVER advanced from a single peer's wire-supplied epoch tag (that is
    /// unauthenticated — one Byzantine peer naming `u64::MAX` would otherwise
    /// pin every honest node into permanent soft-enter = network liveness halt).
    /// f+1 distinct corroboration guarantees ≥1 honest reporter, so the value
    /// only ever reaches an epoch the honest majority is actually voting at.
    highest_observed_epoch: Epoch,
    /// Distinct backup-vote senders per future epoch, pending the f+1 threshold.
    /// Bounded by the per-sender pin quota (see `sender_pins`); entries ≤
    /// `highest_observed_epoch` are pruned on every advance.
    observed_reporters: BTreeMap<Epoch, BTreeSet<PublicKey>>,
    /// Per-sender quota of future epochs each peer may pin
    /// ([`PINS_PER_SENDER`]). Bounds memory to `n · PINS_PER_SENDER` epochs AND
    /// stops ≤f Byzantine from flooding many decoy epochs to crowd out the
    /// honestly-corroborated true frontier — they can occupy at most `f ·
    /// PINS_PER_SENDER` slots, so the frontier always has room to reach f+1.
    sender_pins: BTreeMap<PublicKey, BTreeSet<Epoch>>,
    /// Committee size of the HIGHEST-ENTERED epoch, used to derive the Byzantine
    /// threshold f = (n−1)/3 for corroboration. Keyed on the newest entered epoch
    /// (set in `enter` only when `epoch == highest_entered_epoch`) so it follows
    /// both validator-set growth and shrink; a stale soft-enter (epoch <
    /// highest_entered) cannot lower it, preserving the R4-2 grow-attack guard.
    /// `0` until the first reconcile full-enters an epoch, during which backup
    /// corroboration is disabled (the cold-start epoch full-enters from the
    /// verified boundary trigger).
    committee_size: usize,
    /// The role this node currently holds per epoch — the single source of truth
    /// the reconciler diffs against. `Signer` ⟺ a participating engine in
    /// `active_epochs`; `Verifier` ⟺ a verify-only scheme registered, no engine.
    roles: BTreeMap<Epoch, Role>,
    /// Live-frontier epochs whose `Verifier→Signer` spawn is parked on marshal
    /// block availability: the `Inline::genesis(E)` precondition (the E-1 boundary
    /// block not yet in marshal storage) — resumes as backfill lands.
    /// Re-checked on every reconcile edge (boundary / share /
    /// spawn_unblocked / vote_backup) — `reconcile_roles` is idempotent, so a parked
    /// epoch spawns the instant the boundary block lands. Never panics (defer, never
    /// `unreachable!`).
    deferred_spawns: BTreeSet<Epoch>,
    /// The most-recent boundary delivery `(epoch, snapshot)`. The non-boundary
    /// edges (share / spawn_unblocked / vote_backup) carry no snapshot, so they
    /// reconcile the CURRENT live epoch from this cache.
    latest_live: Option<(Epoch, ValidatorSetSnapshot)>,
    /// Last `(highest_entered, highest_observed)` pair for which
    /// [`pipeline_catchup_span`] ran and registered nothing new (bug 15).
    /// Identical re-attempts are suppressed until an edge (share landed /
    /// execution progressed / frontier moved) clears it, so a backup-vote storm
    /// (128/s/peer × n) cannot re-run the EVM span fan-out + marshal hint per
    /// vote while nothing changed.
    catchup_no_progress: Option<(Epoch, Epoch)>,
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
    /// Callback into [`crate::outer::EpochSchemeProvider`] so marshal can verify
    /// cross-epoch finalization certificates (trailing-window pruned; see SCHEME_RETENTION_EPOCHS).
    pub register_scheme: Arc<dyn Fn(Epoch, BlsScheme) + Send + Sync>,
    /// The same [`crate::outer::EpochSchemeProvider`] `register_scheme` writes
    /// into, held directly because the repair sweep has to READ it: which epochs
    /// are registered unpinned is a question only the registry can answer, and
    /// the answer covers registration paths the epoch manager never sees (the
    /// bulk catch-up span, `cold_start_register`).
    pub scheme_pins: EpochSchemeProvider,
    /// Bulk catch-up soft-enter: register a verify-only scheme for EVERY epoch in
    /// the inclusive span `[from, to]`, reading each committee from the CURRENT
    /// finalized state (at the result-final read height — see
    /// [`fluentbase_staking_reader::EpochTransition::soft_enter_span`]). Returns
    /// the HIGHEST epoch actually registered (a missed/unreadable committee
    /// truncates the contiguous on-chain prefix). Called ONCE per backup-vote
    /// hint from [`Actor::handle_msg_for_unregistered_epoch`] to pre-register a
    /// whole gap in one step (instead of one boundary per finalized round-trip),
    /// so the marshal hint can target the frontier directly. Built in
    /// [`crate::outer::OuterBuilder::build`] over `register_scheme` + `chain_id`
    /// + the node-side committee reader threaded from `dpos.rs`.
    pub soft_enter_span: Arc<dyn Fn(Epoch, Epoch) -> BoxFuture<'static, Epoch> + Send + Sync>,
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
    pub fn new(
        context: E,
        cfg: Config<B, XC, A>,
    ) -> (Self, mpsc::Sender<(Epoch, ValidatorSetSnapshot)>) {
        let (boundary_tx, boundary_rx) = mpsc::channel(BOUNDARY_BUFFER);
        let actor = Self {
            context: ContextCell::new(context),
            active_epochs: BTreeMap::new(),
            dkg_agreements: BTreeMap::new(),
            agreement_intake: None,
            boundary_rx,
            highest_entered_epoch: Epoch::new(0),
            highest_observed_epoch: Epoch::new(0),
            observed_reporters: BTreeMap::new(),
            sender_pins: BTreeMap::new(),
            committee_size: 0,
            roles: BTreeMap::new(),
            deferred_spawns: BTreeSet::new(),
            latest_live: None,
            catchup_no_progress: None,
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
    /// `SubReceiver`s auto-deregister, freeing the slots for the next promotion). The
    /// vote Muxer's backup receiver is the plane's re-settable forwarder, fresh per
    /// promotion.
    pub fn start<HS, HR>(
        mut self,
        muxes: Option<Muxes<HS, HR>>,
        vote_backup: mpsc::Receiver<VoteBackupItem>,
    ) -> Handle<()>
    where
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
    {
        spawn_cell!(self.context, self.run(muxes, vote_backup).await)
    }

    async fn run<HS, HR>(
        mut self,
        muxes: Option<Muxes<HS, HR>>,
        mut vote_backup: mpsc::Receiver<VoteBackupItem>,
    ) where
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
    {
        // The vote/cert/resolver Muxers live in the always-on plane (one set per
        // process). Catch-up votes for an unregistered epoch surface on the plane's
        // `vote_backup` forwarder (re-pointed to THIS manager per promotion), driving
        // the catch-up hint. Graceful exit is via boundary_rx close OR vote_backup
        // close (the plane parks the forwarder while no engine is up); the plane's
        // Muxer tasks are NOT aborted here (they outlive this manager).
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
                        Some((epoch, snap)) => {
                            // The only edge carrying a fresh snapshot — cache it for the
                            // non-boundary edges, then reconcile (folds enter + prune_old).
                            self.latest_live = Some((epoch, snap.clone()));
                            self.reconcile_roles(epoch, snap, muxes.as_ref()).await;
                            // AFTER the reconcile, so the frontier the sweep runs
                            // under already includes this boundary's epoch and it
                            // cannot pin under the engine just spawned for it.
                            sweep_wake.send_replace((
                                self.highest_observed_epoch,
                                self.highest_entered_epoch,
                            ));
                        }
                        None => {
                            info!("boundary_rx closed, epoch_manager exiting");
                            break;
                        }
                    }
                }
                backup = vote_backup.recv() => {
                    match backup {
                        Some((their_epoch, (from, _bytes))) => {
                            // Corroboration / catch-up only affects a role decision when it
                            // moves the frontier (`highest_observed_epoch` flips `is_live`)
                            // or the entered tip (`highest_entered_epoch` registers new
                            // schemes). Reconcile ONLY on that change — otherwise every
                            // backup vote (>100/s during catch-up) re-runs an identical
                            // no-op reconcile. The change-gate still breaks the cycle-2
                            // deadlock: the FIRST vote that corroborates the new frontier
                            // reconciles, even with the chain stalled.
                            let before = (self.highest_observed_epoch, self.highest_entered_epoch);
                            self.handle_msg_for_unregistered_epoch(their_epoch, from).await;
                            if (self.highest_observed_epoch, self.highest_entered_epoch) != before {
                                self.reconcile_live(muxes.as_ref()).await;
                            }
                        }
                        None => {
                            info!("vote backup channel closed, epoch_manager exiting");
                            break;
                        }
                    }
                }
                // Edge: the executor recorded a finalized block — the MID-EPOCH
                // promotion trigger. A caught-up member promotes the instant its
                // `Inline::genesis` precondition is met; gated on a pending parked
                // spawn so the per-block fire is a no-op in steady state. Execution
                // progress can also unblock a catch-up span read → clear the memo.
                _ = &mut spawn_n => {
                    self.catchup_no_progress = None;
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
                    // The two classes are NOT the same edge, and merging their
                    // effects is how one of them quietly acquires the other's:
                    //
                    // - PARTICIPATION: a landed share clears the catch-up
                    //   no-progress memo — it can make a previously-unresolvable
                    //   span read succeed (bug 15).
                    // - KEY: wakes the repair sweep (below the frontier this arm is
                    //   its fast path; at the frontier it owes the ACQUISITION).
                    //
                    // The share class ALSO wakes the sweep, which the HEAD share arm
                    // did not: the sweep is idempotent and built to be woken, and
                    // two arms that must agree by hand is what this merge removes.
                    // It does NOT gain the memo clear in the other direction.
                    match event {
                        Ok(crate::beacon::BeaconEvent::ParticipationChanged) => {
                            self.catchup_no_progress = None;
                        }
                        Ok(crate::beacon::BeaconEvent::KeyAvailable) => {}
                        // `Lagged`/`Closed`: either class may have been dropped, so
                        // both effects apply. Re-reading everything is all this
                        // actor would have done for each message it missed.
                        Err(_) => {
                            self.catchup_no_progress = None;
                        }
                        // Filtered out by `next_reconcile_wake`.
                        Ok(crate::beacon::BeaconEvent::SeedRecorded) => unreachable!(
                            "the seed class never leaves next_reconcile_wake"
                        ),
                    }
                    sweep_wake.send_replace((
                        self.highest_observed_epoch,
                        self.highest_entered_epoch,
                    ));
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

    /// A vote arrived for an epoch with no registered sub-channel — the network
    /// is ahead of us. PRE-REGISTER a bounded SPAN of verify-only schemes ahead of
    /// our entered tip in one step (`soft_enter_span`), then hint the marshal to
    /// fetch the finalization at the boundary of the HIGHEST epoch we just
    /// registered — so its gap-repair can walk our finalized tip across the whole
    /// span at once instead of stalling one boundary per finalized round-trip
    /// (the deep-catch-up wedge: each boundary cost ~14s while the chain paced
    /// 1 blk/s, so a multi-epoch gap never converged). The span is bounded by the
    /// f+1-corroborated observed frontier AND [`CATCHUP_SPAN_CAP`]
    /// (< `SCHEME_RETENTION_EPOCHS`, so the marshal never evicts the span's low
    /// end before the walk reaches it). `highest_entered_epoch` advances to the
    /// highest registered epoch so a repeat backup vote does not re-register the
    /// same span and the hint stays monotone.
    async fn handle_msg_for_unregistered_epoch(&mut self, their_epoch: Epoch, from: PublicKey) {
        // Advance the live frontier ONLY when f+1 DISTINCT peers have named the
        // same future epoch on the (unauthenticated) vote backup channel. With
        // ≤ f Byzantine validators, f+1 distinct reporters always include ≥1
        // honest one, who only votes at the true live epoch — so a single (or up
        // to f colluding) Byzantine peer(s) cannot inflate the frontier and force
        // permanent soft-enter. Until the first `enter` sets the committee size,
        // corroboration is disabled (the cold-start epoch full-enters from the
        // verified boundary trigger, so an early backup message must not gate it
        // off). `corroborate_frontier` is a free fn so this logic is unit-testable
        // without an `Actor`.
        corroborate_frontier(
            &mut self.observed_reporters,
            &mut self.sender_pins,
            &mut self.highest_observed_epoch,
            self.committee_size,
            their_epoch,
            from.clone(),
        );
        let mailbox = self.cfg.marshal_mailbox.clone();
        let hint = move |boundary| {
            Box::pin(async move {
                mailbox
                    .hint_finalized(boundary, NonEmptyVec::new(from))
                    .await;
            }) as BoxFuture<'static, ()>
        };
        // The span-pipeline body is a free async fn over the state pieces +
        // callbacks so the pipelining invariant (ONE bounded span per hint, the
        // hint targeting the registered frontier) is unit-testable without
        // standing up the full generic `Actor` / a real marshal mailbox.
        pipeline_catchup_span(
            &mut self.highest_entered_epoch,
            self.highest_observed_epoch,
            their_epoch,
            &self.cfg.epocher,
            self.cfg.soft_enter_span.as_ref(),
            hint,
            &mut self.catchup_no_progress,
        )
        .await;
    }

    /// True when `epoch` is at or past the highest epoch observed on the backup
    /// channel — i.e. the live frontier, not a historical catch-up epoch. Below
    /// the frontier we only soft-enter (register the scheme, NO participating
    /// engine): a Simplex engine for a stale epoch has no live peers and would
    /// drive the executor on a dead fork, intermittently wedging the catch-up.
    ///
    /// NB: must NOT add a retention window here. During fast catch-up
    /// `highest_observed_epoch` tracks only ~1-2 epochs ahead of the walk, so a
    /// retention-window slack makes the gate true for nearly every
    /// catch-up epoch → they all full-enter → spurious engines → flaky wedge.
    /// Strict `>=` soft-enters every below-frontier epoch; once the walk reaches
    /// the frontier (votes arrive on a registered subchannel, not backup, so
    /// `highest_observed_epoch` stops rising) the frontier epoch full-enters.
    /// The frontier itself is corroboration-gated — see
    /// [`corroborate_frontier`].
    fn is_live_epoch(&self, epoch: Epoch) -> bool {
        epoch >= self.highest_observed_epoch
    }

    /// Reconcile this node's per-epoch role from current state — the single
    /// decision point, folding the old `enter` + `prune_old`. `role(E) = Signer iff
    /// (I ∈ committee[E]) ∧ is_live_epoch(E)`, else `Verifier`; a `Signer`
    /// additionally needs a usable DKG share (share-gate) and the
    /// `Inline::genesis(E)` precondition before its engine spawns. Idempotent — safe
    /// to call repeatedly for the same `(epoch, snap)` on any edge.
    async fn reconcile_roles<HS, HR>(
        &mut self,
        epoch: Epoch,
        snap: ValidatorSetSnapshot,
        muxes: Option<&Muxes<HS, HR>>,
    ) where
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
    {
        // Boundary bookkeeping (idempotent; monotone). Committee size is keyed on
        // the HIGHEST-ENTERED epoch (follows validator-set growth and shrink) — it
        // feeds the f+1 corroboration threshold. Reaching an epoch RESOLVES it:
        // free pending corroboration pins ≤ it so a healthy node's boundary-race
        // pins don't permanently mute honest senders.
        self.highest_entered_epoch = self.highest_entered_epoch.max(epoch);
        if epoch == self.highest_entered_epoch {
            self.committee_size = snap.validators.len();
        }
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
        prune_resolved(
            &mut self.observed_reporters,
            &mut self.sender_pins,
            self.highest_entered_epoch,
        );

        // Exit-at-transition: abort every engine strictly below the live epoch
        // (folded `prune_old`; `e < cutoff` only, so a stale/replayed boundary for
        // an OLD epoch can never abort a newer engine).
        self.abort_below(epoch).await;

        // Below the live frontier → soft-enter (verify-only scheme, NO engine): a
        // Simplex engine for a stale epoch has no live peers and would drive a dead
        // fork. Verify-only lets the marshal verify this epoch's certs — with the
        // seed slot checked against `PK_epoch` whenever the oracle can resolve it
        // at the moment the certificate arrives, and vote-only while it cannot.
        // Nothing about the registered scheme decides that; `repair_keyless_schemes`
        // is what drives the acquisition for below-frontier epochs.
        if !self.is_live_epoch(epoch) {
            self.soft_enter(epoch, &snap).await;
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
            && snap
                .validators
                .iter()
                .any(|v| v.keys.peer_pubkey == self.cfg.me);

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
                // (boundary / participation / spawn_unblocked / vote_backup), and
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
                    self.soft_enter(epoch, &snap).await;
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
                self.soft_enter(epoch, &snap).await;
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
                    self.soft_enter(epoch, &snap).await;
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
                let lookup = self.boundary_lookup(epoch).await;
                let fallback_seed = match seedless_base(&lookup, &snap) {
                    None => {
                        self.deferred_spawns.insert(epoch);
                        self.soft_enter(epoch, &snap).await;
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
                        self.soft_enter(epoch, &snap).await;
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
                if self
                    .spawn_engine(epoch, snap, scheme, fallback_seed, muxes)
                    .await
                {
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
    async fn soft_enter(&mut self, epoch: Epoch, snap: &ValidatorSetSnapshot) {
        if self.active_epochs.contains_key(&epoch) || self.roles.get(&epoch) == Some(&Role::Signer)
        {
            return;
        }
        register_soft_entered(
            self.cfg.randomness.as_ref(),
            epoch,
            snap,
            self.cfg.chain_id,
            self.cfg.register_scheme.as_ref(),
        )
        .await;
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
    /// `vote_backup`
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
        let (wake_tx, wake_rx) =
            watch::channel((self.highest_observed_epoch, self.highest_entered_epoch));
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

    /// Re-run [`Self::reconcile_roles`] for the CURRENT live epoch (the cached most
    /// recent boundary delivery). The non-boundary edges (share / spawn_unblocked /
    /// vote_backup) carry no fresh snapshot, so they reconcile this.
    ///
    /// Only reconciles the cached epoch while it is STILL the live frontier. Once
    /// corroboration advanced the frontier past it, the cached epoch is
    /// below-frontier — already aborted/soft-entered by `abort_below` on the
    /// boundary that passed it, and carrying NO signer obligation. Re-running
    /// `reconcile_roles` on it would soft-enter a verify-only scheme over an epoch
    /// registered as `Signer` → `EpochSchemeProvider` downgrade-refusal churn +
    /// `roles`↔provider divergence. The next BOUNDARY delivery refreshes
    /// `latest_live` to the new frontier and reconciles it there (with its snapshot).
    async fn reconcile_live<HS, HR>(&mut self, muxes: Option<&Muxes<HS, HR>>)
    where
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
    {
        if let Some((epoch, snap)) = self.latest_live.clone() {
            if !self.is_live_epoch(epoch) {
                return;
            }
            self.reconcile_roles(epoch, snap, muxes).await;
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
        fallback_seed: [u8; 32],
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
                fallback_seed,
                epocher: self.cfg.epocher.clone(),
                app: self.cfg.app.clone(),
                timeouts: self.cfg.timeouts,
                mailbox_size: self.cfg.mailbox_size,
                register_scheme: self.cfg.register_scheme.clone(),
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

/// Register the verify-only scheme for `epoch`, bound to that epoch's beacon
/// oracle.
///
/// Returns nothing, and that is the honest signature: it used to report whether
/// the epoch's key had been pinned, which was a fact the caller acted on. Nothing
/// is decided here any more — the scheme reads `PK_epoch` live through the oracle
/// — so the sole caller had nothing left to do with the answer and discarded it.
///
/// This call resolves NO key. It asks [`Beacon::oracle_for`] for the epoch's
/// threshold face and hands it to the scheme, which reads `PK_epoch` live through
/// it on every certificate. A key that is unresolvable at registration time
/// therefore costs vote-only admission only until it lands — the scheme picks it
/// up with no re-registration, and there is nothing to repair in the registry.
/// Acquisition is the separate, off-path job of [`Beacon::ensure_key`], driven
/// for below-frontier epochs by [`repair_keyless_schemes`].
///
/// A free fn over the pieces so registration is testable without standing up the
/// full generic `Actor` (which needs a live marshal, a slasher and a spec-exec
/// mailbox). The caller keeps the role bookkeeping and the already-a-Signer
/// guard, which are `Actor` state.
async fn register_soft_entered(
    randomness: &dyn Beacon,
    epoch: Epoch,
    snap: &ValidatorSetSnapshot,
    chain_id: u64,
    register: &(dyn Fn(Epoch, BlsScheme) + Send + Sync),
) {
    let oracle = randomness.oracle_for(epoch.get());
    debug!(
        ?epoch,
        beacon_active = oracle.is_some(),
        "soft-enter: registering verify-only scheme"
    );
    if let Some(scheme) = soft_enter_verifier(snap, chain_id, oracle) {
        register(epoch, scheme);
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
/// and it has to be both.** `observed` is the f+1-corroborated live epoch, which
/// is the tighter bound whenever it is available — but it is fed ONLY by the
/// vote-backup arm, and a follower PARKS that arm, so on a follower it is
/// `Epoch(0)` for the life of the process and a sweep scoped to it discards every
/// registered epoch. `entered` is the highest epoch whose scheme this node has
/// registered, advanced on every boundary delivery and every catch-up span, so it
/// is evidence a follower does have. Taking the max never widens the exclusion
/// zone below what a signer needs: every path that registers a scheme for `E`
/// advances `entered` to `E` first, so a live epoch is at or above the frontier
/// under either input.
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

/// Live-frontier corroboration step. Advances `highest_observed_epoch` to
/// `their_epoch` only once f+1 DISTINCT peers (f = (n−1)/3) have named it on the
/// unauthenticated vote backup channel — with ≤f Byzantine, f+1 distinct
/// reporters always include ≥1 honest one, so the frontier only ever reaches an
/// epoch the honest majority is actually voting at. A per-sender pin quota bounds
/// memory AND prevents a Byzantine minority from flooding decoy epochs to crowd
/// the honest frontier out of the map. Extracted as a free function over the
/// state pieces so the Byzantine-resistance invariant is unit-testable without
/// standing up the full generic `Actor`.
fn corroborate_frontier(
    observed_reporters: &mut BTreeMap<Epoch, BTreeSet<PublicKey>>,
    sender_pins: &mut BTreeMap<PublicKey, BTreeSet<Epoch>>,
    highest_observed_epoch: &mut Epoch,
    committee_size: usize,
    their_epoch: Epoch,
    from: PublicKey,
) {
    if committee_size == 0 || their_epoch <= *highest_observed_epoch {
        return;
    }
    let threshold = (committee_size - 1) / 3 + 1; // f + 1, n = 3f + 1

    // Per-sender quota: a peer may pin at most PINS_PER_SENDER distinct future
    // epochs. f Byzantine therefore occupy ≤ f·PINS_PER_SENDER slots and cannot
    // evict/crowd out the honestly-corroborated true frontier.
    let pins = sender_pins.entry(from.clone()).or_default();
    if !pins.contains(&their_epoch) {
        if pins.len() >= PINS_PER_SENDER {
            return;
        }
        pins.insert(their_epoch);
    }

    let reporters = observed_reporters.entry(their_epoch).or_default();
    reporters.insert(from);
    if reporters.len() >= threshold {
        *highest_observed_epoch = (*highest_observed_epoch).max(their_epoch);
        // Prune everything now at or below the advanced frontier and free the
        // senders' quota for those epochs.
        prune_resolved(observed_reporters, sender_pins, *highest_observed_epoch);
    }
}

/// The catch-up span pipeline: PRE-REGISTER a bounded span of verify-only
/// schemes ahead of the entered tip in ONE step, then hint the marshal toward
/// the registered frontier's boundary so its gap-repair walks the whole span at
/// once (replacing the one-boundary-per-finalized-round-trip walk that never
/// converged on a deep gap). Extracted as a free async fn over the state pieces
/// and callbacks so the pipelining invariant is unit-testable without an `Actor`
/// or a real marshal mailbox.
///
/// - early-outs when `their_epoch ≤ *highest_entered_epoch` (caught up);
/// - span = `[entered+1 .. min(highest_observed_epoch, entered+CATCHUP_SPAN_CAP)]`
///   (bounded; `CATCHUP_SPAN_CAP < SCHEME_RETENTION_EPOCHS` so the provider never
///   evicts the span's low end before the walk reaches it);
/// - `soft_enter_span(from, to)` registers the contiguous on-chain prefix and
///   returns the highest epoch actually registered;
/// - `*highest_entered_epoch` advances to that frontier so a repeat backup vote
///   does not re-register the same span and the hint stays monotone;
/// - `hint(boundary)` targets `epocher.last(registered_to)`.
async fn pipeline_catchup_span(
    highest_entered_epoch: &mut Epoch,
    highest_observed_epoch: Epoch,
    their_epoch: Epoch,
    epocher: &OriginEpocher,
    soft_enter_span: &(dyn Fn(Epoch, Epoch) -> BoxFuture<'static, Epoch> + Send + Sync),
    hint: impl FnOnce(Height) -> BoxFuture<'static, ()>,
    no_progress: &mut Option<(Epoch, Epoch)>,
) {
    if their_epoch <= *highest_entered_epoch {
        return;
    }
    // Suppress an identical re-attempt (bug 15): if the last run at this exact
    // (entered, observed) state registered nothing new, a backup-vote storm must
    // not re-run the EVM span fan-out + marshal hint while nothing changed. The
    // memo is cleared on the share / spawn_unblocked edges (a share landing or
    // execution progressing can change the outcome) and a frontier move changes
    // `state` itself — so the first vote corroborating a NEW frontier still runs.
    let state = (*highest_entered_epoch, highest_observed_epoch);
    if *no_progress == Some(state) {
        return;
    }
    let prior_entered = *highest_entered_epoch;
    let entered = highest_entered_epoch.get();
    let span_from = Epoch::new(entered + 1);
    let span_top = highest_observed_epoch.min(Epoch::new(entered + CATCHUP_SPAN_CAP));
    let registered_to = if span_top >= span_from {
        soft_enter_span(span_from, span_top).await
    } else {
        *highest_entered_epoch
    };
    // Memoize a no-progress attempt (nothing new registered) so identical
    // re-votes early-out above; a real advance clears the memo (progress moves
    // `state`, and this stores `None`).
    *no_progress = (registered_to == prior_entered).then_some(state);
    let Some(boundary) = epocher.last(registered_to) else {
        return;
    };
    info!(
        observed = their_epoch.get(),
        entered,
        registered_to = registered_to.get(),
        %boundary,
        "catch-up: behind network; span soft-entered, hinting marshal toward frontier"
    );
    // Advance the entered tip to the registered frontier so the next backup
    // vote does not re-register the same span and the hint stays monotone.
    *highest_entered_epoch = registered_to;
    hint(boundary).await;
}

/// Drop pending corroboration state for every epoch `≤ floor` and free those
/// epochs from each sender's pin quota. Called when the frontier advances
/// (corroboration threshold met) AND when an epoch is entered (resolved by the
/// verified boundary trigger) — the latter is what keeps a healthy node's
/// boundary-race pins from permanently muting honest senders.
fn prune_resolved(
    observed_reporters: &mut BTreeMap<Epoch, BTreeSet<PublicKey>>,
    sender_pins: &mut BTreeMap<PublicKey, BTreeSet<Epoch>>,
    floor: Epoch,
) {
    observed_reporters.retain(|e, _| *e > floor);
    sender_pins
        .values_mut()
        .for_each(|eps| eps.retain(|e| *e > floor));
    sender_pins.retain(|_, eps| !eps.is_empty());
}

/// True when a per-epoch engine `Handle` has COMPLETED (its task finished — normal
/// exit or a panic caught by the runtime's `catch_panics(true)`, which completes
/// the handle without propagating to the manager). Commonware `Handle` exposes only
/// `abort()` + `impl Future` (no `is_finished`) and is `Unpin`, so we poll it once
/// with a no-op waker: `Ready` ⇒ dead, `Pending` ⇒ still running. Extracted as a
/// free fn (mirroring `corroborate_frontier` / `pipeline_catchup_span`) so the
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

    fn distinct_keys(n: usize) -> Vec<PublicKey> {
        (0..n)
            .map(|i| {
                let mut rng = StdRng::seed_from_u64(0xF100 + i as u64);
                Ed25519PrivateKey::random(&mut rng).public_key()
            })
            .collect()
    }

    /// Test harness mirroring the actor's three corroboration state pieces.
    struct Frontier {
        observed: BTreeMap<Epoch, BTreeSet<PublicKey>>,
        pins: BTreeMap<PublicKey, BTreeSet<Epoch>>,
        epoch: Epoch,
        committee_size: usize,
    }
    impl Frontier {
        fn new(committee_size: usize, start: u64) -> Self {
            Self {
                observed: BTreeMap::new(),
                pins: BTreeMap::new(),
                epoch: Epoch::new(start),
                committee_size,
            }
        }
        fn report(&mut self, epoch: u64, from: &PublicKey) {
            corroborate_frontier(
                &mut self.observed,
                &mut self.pins,
                &mut self.epoch,
                self.committee_size,
                Epoch::new(epoch),
                from.clone(),
            );
        }
        /// Mirror the actor's `enter`: resolve `epoch` (free its corroboration
        /// state below the entered floor).
        fn enter(&mut self, epoch: u64) {
            prune_resolved(&mut self.observed, &mut self.pins, Epoch::new(epoch));
        }
        fn pin_count(&self, from: &PublicKey) -> usize {
            self.pins.get(from).map_or(0, |e| e.len())
        }
    }

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
    fn register_verifier(provider: &EpochSchemeProvider, epoch: Epoch, r: &dyn Beacon) {
        let (snap, _) = repair_fixture(epoch);
        let scheme =
            soft_enter_verifier(&snap, 1, r.oracle_for(epoch.get())).expect("valid committee");
        provider.register(epoch, scheme);
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
        let provider = EpochSchemeProvider::new();
        register_verifier(&provider, epoch, r.as_ref());
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
        let provider = EpochSchemeProvider::new();
        register_verifier(&provider, epoch, r.as_ref());

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
        let provider = EpochSchemeProvider::new();
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
        register_verifier(&provider, epoch, r.as_ref());
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
        let provider = EpochSchemeProvider::new();
        let store = BeaconKeys::new();
        store.set_pk(older.get(), some_group_key(), KeySource::Agreed);
        let r = randomness_over(store.clone(), None, None);
        register_verifier(&provider, older, r.as_ref());
        register_verifier(&provider, newer, r.as_ref());

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
        let provider = EpochSchemeProvider::new();
        let store = BeaconKeys::new();
        store.set_pk(pre_beacon.get(), some_group_key(), KeySource::Agreed);
        let r = randomness_over(store.clone(), None, None);
        register_verifier(&provider, pre_beacon, r.as_ref());
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
        let provider = EpochSchemeProvider::new();
        provider.register(epoch, signer);

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
        let provider = EpochSchemeProvider::new();
        let store = BeaconKeys::new();
        store.set_pk(epoch.get(), some_group_key(), KeySource::Agreed);
        let r = randomness_over(store.clone(), None, None);
        register_verifier(&provider, epoch, r.as_ref());

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

    /// The follower half of the frontier. A follower PARKS the vote-backup arm,
    /// so nothing ever corroborates `highest_observed_epoch` and it stays at
    /// `Epoch(0)` for the life of the process — under which every registered
    /// epoch is at or above the frontier and the sweep discards all of them. The
    /// epoch a boundary delivery entered is the evidence a follower does have.
    ///
    /// Reds if the sweep is re-scoped to `observed` alone.
    #[tokio::test]
    async fn the_sweep_reaches_below_an_entered_epoch_with_no_corroborated_frontier() {
        let epoch = Epoch::new(5);
        let provider = EpochSchemeProvider::new();
        let store = BeaconKeys::new();
        store.set_pk(epoch.get(), some_group_key(), KeySource::Agreed);

        let r = randomness_over(store.clone(), None, None);
        register_verifier(&provider, epoch, r.as_ref());
        let never_corroborated = Epoch::new(0);
        assert!(
            repair_keyless_schemes(
                &provider,
                r.as_ref(),
                &mut BTreeSet::new(),
                never_corroborated,
                Epoch::new(0),
            )
            .await
            .is_empty(),
            "with no frontier evidence at all the sweep must stay inert — the \
             pre-fix follower state, kept here so the assertion below is not \
             vacuous"
        );

        // The boundary for epoch 6 lands and `reconcile_roles` enters it.
        assert_eq!(
            repair_keyless_schemes(
                &provider,
                r.as_ref(),
                &mut BTreeSet::new(),
                never_corroborated,
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
        let provider = EpochSchemeProvider::new();
        let store = BeaconKeys::new();
        store.set_pk(epoch.get(), some_group_key(), KeySource::Agreed);

        let r = randomness_over(store.clone(), None, None);
        register_verifier(&provider, epoch, r.as_ref());
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
        let provider = EpochSchemeProvider::new();
        let store = BeaconKeys::new();
        store.set_pk(local.get(), some_group_key(), KeySource::LocalDkg);
        store.set_pk(carried.get(), some_other_group_key(), KeySource::Carried);
        let r = randomness_over(store.clone(), None, None);
        register_verifier(&provider, local, r.as_ref());
        register_verifier(&provider, carried, r.as_ref());

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
    /// ~100 s window in which the beacon wake-up arm, `spawn_unblocked` and
    /// `vote_backup` do
    /// not run — the three arms that turn this node into a signer.
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
        let provider = EpochSchemeProvider::new();
        register_verifier(
            &provider,
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

    #[test]
    fn single_peer_cannot_advance_frontier() {
        let keys = distinct_keys(4);
        let mut f = Frontier::new(4, 5);
        f.report(u64::MAX, &keys[0]);
        assert_eq!(
            f.epoch,
            Epoch::new(5),
            "one peer must not move the frontier"
        );
        // Repeated messages from the SAME peer stay at one distinct reporter.
        f.report(u64::MAX, &keys[0]);
        assert_eq!(f.epoch, Epoch::new(5));
    }

    // f+1 distinct peers (≥1 honest) DO advance the frontier; lower pending
    // entries are pruned.
    #[test]
    fn fplus1_distinct_peers_advance_frontier() {
        let keys = distinct_keys(4);
        let mut f = Frontier::new(4, 5);
        f.report(9, &keys[0]);
        assert_eq!(f.epoch, Epoch::new(5), "first reporter is below threshold");
        f.report(9, &keys[1]);
        assert_eq!(f.epoch, Epoch::new(9), "f+1=2 distinct reporters advance");
        assert!(
            f.observed.is_empty(),
            "entries ≤ frontier pruned after advance"
        );
    }

    // Before the first entered epoch (committee_size == 0) corroboration is
    // disabled, so a pre-enter backup message can't gate off the cold-start epoch.
    #[test]
    fn no_corroboration_before_first_committee() {
        let keys = distinct_keys(4);
        let mut f = Frontier::new(0, 0);
        for k in &keys {
            f.report(99, k);
        }
        assert_eq!(f.epoch, Epoch::new(0));
        assert!(f.observed.is_empty());
    }

    // R4-1 regression: f Byzantine cannot freeze the honest frontier by flooding
    // many DECOY epochs each corroborated to count f. With n=7 (f=2), the 2
    // Byzantine keys flood 100 high decoy epochs (each reaching count 2 < 3), then
    // 3 honest peers back the true frontier 10. The per-sender pin quota stops the
    // decoys from crowding it out, so the honest frontier still reaches f+1=3 and
    // advances. (The old count-based eviction would have dropped epoch 10 forever.)
    #[test]
    fn byzantine_decoy_flood_cannot_freeze_honest_frontier() {
        let keys = distinct_keys(7); // n=7 ⇒ f=2 ⇒ threshold 3; keys[5],[6] Byzantine
        let mut f = Frontier::new(7, 0);
        for e in 1_000..1_100u64 {
            f.report(e, &keys[5]);
            f.report(e, &keys[6]);
        }
        // Memory stays bounded by the per-sender quota (≤ n · PINS_PER_SENDER).
        assert!(
            f.observed.len() <= 7 * PINS_PER_SENDER,
            "map bounded by quota: {}",
            f.observed.len()
        );
        // 3 honest peers corroborate the true frontier 10 → must advance.
        f.report(10, &keys[0]);
        f.report(10, &keys[1]);
        f.report(10, &keys[2]);
        assert_eq!(
            f.epoch,
            Epoch::new(10),
            "honest frontier advanced despite the decoy flood"
        );
    }

    // The per-sender quota caps how many distinct future epochs one peer pins;
    // beyond PINS_PER_SENDER its further (new-epoch) reports are ignored.
    #[test]
    fn per_sender_quota_caps_pins() {
        let keys = distinct_keys(7);
        let mut f = Frontier::new(7, 0);
        for e in 50..60u64 {
            f.report(e, &keys[6]);
        }
        assert_eq!(
            f.observed.len(),
            PINS_PER_SENDER,
            "one peer pins at most PINS_PER_SENDER epochs"
        );
    }

    // Regression: a healthy node's boundary-race pins must be FREED when the node
    // enters the epoch, not permanently consume the sender's quota. Without the
    // enter()-time prune, a peer that races a vote for E+1 onto the backup channel
    // each boundary would exhaust PINS_PER_SENDER after 2 boundaries and be muted
    // → the live frontier freezes.
    #[test]
    fn entering_an_epoch_frees_boundary_race_pins() {
        let keys = distinct_keys(7); // threshold 3 — single races never fire it
        let mut f = Frontier::new(7, 0);
        // Simulate many boundaries: each, one peer races a single vote for the
        // next epoch, then the node enters it.
        for e in 1..=20u64 {
            f.report(e, &keys[6]); // race vote for epoch e (below threshold)
            f.enter(e); // node enters e → its pin must be freed
            assert_eq!(
                f.pin_count(&keys[6]),
                0,
                "pin for entered epoch {e} must be freed, not retained"
            );
        }
        // The racer was never muted, so it can still corroborate a real future
        // frontier together with f+1−1 others.
        f.report(25, &keys[6]);
        f.report(25, &keys[0]);
        f.report(25, &keys[1]);
        assert_eq!(
            f.epoch,
            Epoch::new(25),
            "frontier still advances after 20 boundaries"
        );
    }

    /// Records every `(from, to)` span the catch-up pipeline soft-enters and
    /// returns `to` (the whole span registered).
    fn recording_span(
        log: std::sync::Arc<std::sync::Mutex<Vec<(u64, u64)>>>,
    ) -> Arc<dyn Fn(Epoch, Epoch) -> BoxFuture<'static, Epoch> + Send + Sync> {
        Arc::new(move |from: Epoch, to: Epoch| {
            log.lock().unwrap().push((from.get(), to.get()));
            Box::pin(async move { to }) as BoxFuture<'static, Epoch>
        })
    }

    // A DEEP gap (entered 0, observed frontier 3) must be pipelined in ONE hint:
    // a single soft_enter_span(1, 3) and a single marshal hint at last(3) — NOT
    // three serialized one-boundary-at-a-time round-trips. A repeat vote at the
    // now-entered frontier must be a no-op. Then a CAP variant: a 20-deep
    // observed frontier is capped to (1, CATCHUP_SPAN_CAP) and hints last(CAP).
    #[test]
    fn deep_catchup_pipelines_span_in_one_hint() {
        use commonware_consensus::types::Epocher as _;
        use std::sync::Mutex as StdMutex;

        let epocher = OriginEpocher::new(0, 32u64.try_into().unwrap());

        // A hint recorder that records each targeted boundary.
        let mk_hint = |hints: std::sync::Arc<StdMutex<Vec<Height>>>| {
            move |b: Height| {
                hints.lock().unwrap().push(b);
                Box::pin(async move {}) as BoxFuture<'static, ()>
            }
        };

        // Deep gap: entered 0, observed frontier 3.
        let spans = std::sync::Arc::new(StdMutex::new(Vec::<(u64, u64)>::new()));
        let hints = std::sync::Arc::new(StdMutex::new(Vec::<Height>::new()));
        let soft = recording_span(spans.clone());
        let mut entered = Epoch::new(0);

        futures::executor::block_on(pipeline_catchup_span(
            &mut entered,
            Epoch::new(3),
            Epoch::new(3),
            &epocher,
            soft.as_ref(),
            mk_hint(hints.clone()),
            &mut None,
        ));
        assert_eq!(
            *spans.lock().unwrap(),
            vec![(1, 3)],
            "deep gap pipelined in ONE span call, not three serialized hops"
        );
        assert_eq!(
            *hints.lock().unwrap(),
            vec![epocher.last(Epoch::new(3)).unwrap()],
            "single hint targets the registered frontier's boundary last(3)"
        );
        assert_eq!(
            entered,
            Epoch::new(3),
            "entered tip advanced to the frontier"
        );

        // A second identical vote at the now-entered frontier is a no-op (early-out).
        futures::executor::block_on(pipeline_catchup_span(
            &mut entered,
            Epoch::new(3),
            Epoch::new(3),
            &epocher,
            soft.as_ref(),
            mk_hint(hints.clone()),
            &mut None,
        ));
        assert_eq!(
            spans.lock().unwrap().len(),
            1,
            "a repeat vote at the entered frontier must NOT re-register the span"
        );
        assert_eq!(hints.lock().unwrap().len(), 1, "no second hint");

        // CAP variant: a 20-deep observed frontier caps the span at
        // (1, CATCHUP_SPAN_CAP) and hints last(CAP).
        let spans = std::sync::Arc::new(StdMutex::new(Vec::<(u64, u64)>::new()));
        let hints = std::sync::Arc::new(StdMutex::new(Vec::<Height>::new()));
        let soft = recording_span(spans.clone());
        let mut entered = Epoch::new(0);
        futures::executor::block_on(pipeline_catchup_span(
            &mut entered,
            Epoch::new(20),
            Epoch::new(20),
            &epocher,
            soft.as_ref(),
            mk_hint(hints.clone()),
            &mut None,
        ));
        assert_eq!(
            *spans.lock().unwrap(),
            vec![(1, CATCHUP_SPAN_CAP)],
            "span capped at CATCHUP_SPAN_CAP, not the full 20-deep frontier"
        );
        assert_eq!(
            *hints.lock().unwrap(),
            vec![epocher.last(Epoch::new(CATCHUP_SPAN_CAP)).unwrap()],
            "hint targets last(CATCHUP_SPAN_CAP)"
        );
        assert_eq!(entered, Epoch::new(CATCHUP_SPAN_CAP));
    }

    /// A span that registers NOTHING (unresolvable committee read) is memoized
    /// (bug 15): an identical re-vote must NOT re-run the fan-out; but clearing
    /// the memo (a share/spawn edge) OR a frontier advance must let it re-run.
    #[test]
    fn no_progress_span_is_memoized_and_cleared_by_edges() {
        use std::sync::Mutex as StdMutex;

        let epocher = OriginEpocher::new(0, 32u64.try_into().unwrap());
        let calls = std::sync::Arc::new(StdMutex::new(Vec::<(u64, u64)>::new()));
        // Span callback that registers nothing: returns `from-1` (== entered).
        let no_progress_span: Arc<dyn Fn(Epoch, Epoch) -> BoxFuture<'static, Epoch> + Send + Sync> = {
            let calls = calls.clone();
            Arc::new(move |from: Epoch, to: Epoch| {
                calls.lock().unwrap().push((from.get(), to.get()));
                Box::pin(async move { Epoch::new(from.get() - 1) }) as BoxFuture<'static, Epoch>
            })
        };
        let noop_hint = |_b: Height| Box::pin(async move {}) as BoxFuture<'static, ()>;

        let mut entered = Epoch::new(0);
        let mut memo = None;
        let run = |entered: &mut Epoch, memo: &mut Option<(Epoch, Epoch)>, observed: u64| {
            futures::executor::block_on(pipeline_catchup_span(
                entered,
                Epoch::new(observed),
                Epoch::new(observed),
                &epocher,
                no_progress_span.as_ref(),
                noop_hint,
                memo,
            ));
        };

        run(&mut entered, &mut memo, 3);
        assert_eq!(
            calls.lock().unwrap().len(),
            1,
            "first attempt runs the span"
        );
        // Identical re-vote: suppressed by the memo.
        run(&mut entered, &mut memo, 3);
        assert_eq!(
            calls.lock().unwrap().len(),
            1,
            "identical no-progress re-vote must NOT re-run the span"
        );
        // An edge clears the memo → the span runs again.
        memo = None;
        run(&mut entered, &mut memo, 3);
        assert_eq!(
            calls.lock().unwrap().len(),
            2,
            "clearing the memo re-enables the span"
        );
        // A frontier advance changes `state` → the span runs again without a clear.
        run(&mut entered, &mut memo, 4);
        assert_eq!(
            calls.lock().unwrap().len(),
            3,
            "a frontier advance re-enables the span"
        );
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

            prune_agreements(&ctx, &mut agreements, 5, "").await;
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

            prune_agreements(&ctx, &mut agreements, 5, "").await;

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
