//! Per-epoch consensus engine lifecycle.
//!
//! Owns the active-epochs map and an event-driven boundary trigger
//! (`mpsc::Receiver<Epoch>`) fed by
//! [`fluentbase_staking_reader::EpochTransition`]. The vote/cert/resolver Muxers
//! live in the always-on plane (node crate); this manager receives their
//! `MuxHandle`s per promotion and registers/deregisters per-epoch sub-channels.
//! The marshal actor, buffered engine and immutable archives live in
//! [`crate::outer::OuterEngine`]; this manager threads only the 3 simplex broker
//! handles.

use crate::{
    application::{ExecutedChain, FluentApp, OrderingAssembler},
    beacon::{constant_fallback_seed, witness_fallback_seed},
    beacon::{Beacon, PinEffort, ShareProbe, SignerVerdict, WithheldReason},
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

/// The per-epoch validator role — a pure function of current state.
///
/// Membership in `committee[E]` at the live frontier makes a node a
/// [`Role::Signer`]. It additionally needs a usable DKG share and the E-1 boundary
/// block before its engine spawns, because a shareless Simplex member rejects
/// honest peers' seeded votes and the engine panics without its boundary block.
/// Those two are spawn gates, not part of the role verdict.
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
    /// No predecessor epoch (epoch 0) or no computable terminal height. Every node
    /// reaches this identically, so the constant base stays agreed.
    NotApplicable,
    /// The spawn must be deferred. The variant names which input is missing; the
    /// two have different repair paths.
    Missing(MissingInput),
    /// The block is present and its epoch's base is decided. `seed` is
    /// [`witness_fallback_seed`] of sigma at the terminal round; `None` means the
    /// epoch predicate says the predecessor was beacon-inactive, where no sigma can
    /// exist. A local sigma miss is [`BoundaryLookup::Missing`], not this.
    Present { seed: Option<[u8; 32]> },
}

/// Which of the two `Inline::genesis(E)` inputs the node does not hold.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum MissingInput {
    /// The E-1 terminal block is not in marshal storage. Normally transient;
    /// persistent means the height sits below the marshal floor, where no repair
    /// path fetches it.
    BoundaryBlock,
    /// The block is in hand but this node holds no sigma for E-1's terminal round.
    /// Repaired by a certificate for that round arriving on either cert door, never
    /// by fetching the block again.
    TerminalSeed,
}

/// Which base the leader elector's seedless arm gets for an epoch, and why — the
/// variant, not just the bytes, so the caller can meter the predictable case.
#[derive(Debug, PartialEq, Eq)]
enum SeedlessBase {
    /// Inherited from σ of E-1's terminal round: not derivable from constants,
    /// so the epoch's first leader is not known an epoch ahead.
    Witness([u8; 32]),
    /// The constant derivation, reached only where no witness can exist. Agreed
    /// across nodes but predictable.
    Constant([u8; 32]),
}

/// The base for epoch E, read from sigma of E-1's terminal round rather than from
/// the terminal block's body.
///
/// The round is named by the caller from agreed data — `Round(E-1,
/// terminal.proposal_view)` — and only then asked of the store, which may not hold
/// it.
///
/// `mandatory_at(E-1)` decides whether the beacon was active before the store is
/// read, so a sigma at a round the agreed map calls inactive can never become one
/// node's base while its peers take the constant one. A store miss is
/// [`BoundaryLookup::Missing`] — "I do not know yet" — never `Present { seed:
/// None }`, which would elect on the constant base while a peer that holds the
/// sigma elects on the witness one.
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
/// [`BoundaryLookup::Missing`], whose epoch defers its spawn. Pure; the metric for
/// the predictable case is emitted by the caller.
fn seedless_base(lookup: &BoundaryLookup, snap: &ValidatorSetSnapshot) -> Option<SeedlessBase> {
    match lookup {
        BoundaryLookup::Missing(_) => None,
        BoundaryLookup::Present {
            seed: Some(witness),
        } => Some(SeedlessBase::Witness(*witness)),
        // No predecessor or a beacon-inactive one: the constant base is
        // predictable, but no sigma exists to do better.
        BoundaryLookup::NotApplicable | BoundaryLookup::Present { seed: None } => {
            Some(SeedlessBase::Constant(constant_fallback_seed(snap)))
        }
    }
}

/// The counters the epoch manager owns: its spawn decision and its seedless-arm
/// base choice.
///
/// They are facts about membership and the `Inline::genesis` precondition, not
/// about randomness, so they do not live on the beacon struct.
///
/// Registered once per process against the launch context, on both node classes:
/// `prometheus_client::Registry::register` does not deduplicate, and an unowned
/// family silently vanishes from that class's scrape.
#[derive(Clone, Debug, Default)]
pub struct EpochEngineMetrics {
    /// A per-epoch engine self-demoted because the validator was rotated out of the
    /// epoch's committee.
    pub engine_demoted_rotated_out: Counter,
    /// A per-epoch engine spawn was deferred because one of the two
    /// `Inline::genesis(E)` inputs is missing: the previous epoch's terminal block,
    /// or sigma for that block's round. The member registers verify-only meanwhile.
    /// The counter does not distinguish the two; the INFO lines at the defer site
    /// do, because the repair paths differ. Persistently non-zero across an epoch
    /// means the block sits below the marshal floor with no repair path, or no
    /// certificate for E-1's terminal round is reaching either cert door.
    pub engine_spawn_deferred: Counter,
    /// An epoch resolved the constant seedless-arm base instead of the previous
    /// epoch's terminal-round witness, because no witness could exist there: epoch 0,
    /// a non-computable terminal height, or a pre-bootstrap link. Counted where the
    /// base is chosen, upstream of the promote gates, so an epoch that resolves the
    /// constant base and is then demoted counts too. That base is derivable an epoch
    /// ahead, so the first leader is predictable. Expected only around bootstrap.
    pub fallback_seed_constant: Counter,
    /// A live-epoch engine handle was found completed at a reconcile edge and the
    /// manager re-ran the spawn path. Under `catch_panics(true)` a child panic
    /// completes the handle without reaching the manager, so nothing else respawns
    /// it. 0 on a healthy chain.
    pub engine_respawned: Counter,
}

impl EpochEngineMetrics {
    /// Register every counter. Call once per process, against the same context
    /// `BeaconMetrics::register` uses: commonware prefixes each family with the
    /// context's label path, so a labelled child context would rename them.
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

// Finished engines are aborted at the transition; there is no concurrent
// active-epochs window. A finished engine has nothing left to produce and its
// boundary re-propose loop is unpaced, so it would spin BLS and marshal traffic and
// starve the live epoch into certification timeouts. Stragglers in the old epoch
// are served by marshal/resolver, and their late certificates verify via
// `EpochSchemeProvider` (trailing 8-epoch window).

/// Bounded mpsc capacity for boundary triggers (tokio `mpsc::channel(N)`).
const BOUNDARY_BUFFER: usize = 64;

/// The next beacon wake-up this actor acts on, with the seed class dropped inside
/// the future.
///
/// The seed class fires about once a round and belongs to the executor; answering it
/// with a `continue` would still cost a full turn of the manager's `select!` per
/// round. Cancel-safe: the only messages it discards are ones this actor would
/// discard anyway. `Lagged` and `Closed` are returned, not swallowed, because
/// either may hide a class this actor does act on.
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

/// The 3 plane-owned simplex broker handles a signer engine registers per-epoch
/// sub-channels against (vote/cert/resolver). `None` means a follower: it only
/// soft-enters, so [`Actor::spawn_engine`] is unreachable and the muxes are never
/// touched.
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
    boundary_rx: mpsc::Receiver<Epoch>,
    /// Highest epoch we have entered (full or soft), i.e. the highest epoch whose
    /// committee scheme is registered so the marshal can verify its certs. Drives
    /// the catch-up hint target. Monotonic; never decremented.
    highest_entered_epoch: Epoch,
    /// The marshal's ordering tip as [`crate::application::FluentApp`] publishes it
    /// — the height of the highest finalization this node has verified and stored,
    /// and the one input of [`Self::live_epoch`].
    ///
    /// A `watch` receiver rather than a marshal read: the same handle is the value at
    /// a decision point and the edge that re-decides when the tip moves without a
    /// boundary.
    tip: watch::Receiver<u64>,
    /// The live epoch the last reconcile of the live epoch ran for — the tip edge's
    /// memo, so that edge is not per-block work. A verified finalization arrives about
    /// once a second while the epoch it names changes once an epoch, and every
    /// reconcile runs `abort_below`, a `Beacon::observe_epoch` and a participation
    /// probe.
    ///
    /// Written by `reconcile_roles` rather than by the arm, because every other edge
    /// that reconciles the live epoch settles the same question. `None` until the
    /// geometry freezes, which is also the value the gate compares against while
    /// there is no live epoch.
    tip_reconciled_live: Option<Epoch>,
    /// The role this node currently holds per epoch — the single source of truth
    /// the reconciler diffs against. `Signer` ⟺ a participating engine in
    /// `active_epochs`; `Verifier` ⟺ a verify-only scheme registered, no engine.
    roles: BTreeMap<Epoch, Role>,
    /// Live-frontier epochs whose `Verifier→Signer` spawn is parked on the E-1
    /// boundary block not yet being in marshal storage. Re-checked on every reconcile
    /// edge; `reconcile_roles` is idempotent, so a parked epoch spawns the instant
    /// the block lands.
    deferred_spawns: BTreeSet<Epoch>,
    /// Epochs whose [`Self::reconcile_roles`] returned having decided nothing,
    /// because `committee[E]` was not readable at this node's anchor.
    ///
    /// Separate from `deferred_spawns`, which are parked at the opposite end of the
    /// same function: a deferred spawn has already run the whole reconcile and waits
    /// for one marshal block, while an epoch here has run none of it. The module's
    /// wake-up drains this set, which is the only retry such an epoch gets.
    ///
    /// Every miss but two is remembered ([`committee_miss`]): an epoch below the
    /// read window can never be read again, and an impossible committee halts. That
    /// bounds the set to the read window.
    deferred_reconciles: BTreeSet<Epoch>,
    cfg: Config<B, XC, A>,
}

/// Configuration for the [`Actor`].
pub struct Config<B, XC, A> {
    pub me: PublicKey,
    pub blocker: B,
    pub chain_id: u64,
    /// Single cross-epoch `OriginEpocher` — built once and cloned into the marshal
    /// Config and every `EpochEngineConfig`. `origin = dposActivationBlock`.
    pub epocher: OriginEpocher,
    pub signer_keypair: Option<ValidatorBlsKeypair>,
    pub app: FluentApp<XC, A>,
    pub timeouts: ConsensusTimeouts,
    pub mailbox_size: usize,
    /// The one randomness handle.
    pub randomness: Arc<dyn Beacon>,
    /// Edge-trigger fired by the executor each time it records a finalized
    /// OrderBlock. The mid-epoch promotion trigger: re-runs `reconcile_roles` for the
    /// live epoch so a caught-up member promotes the instant its
    /// `Inline::genesis(E)` precondition is met. Fires even in a thin-quorum stall,
    /// because the local executor still advances to the stall tip.
    pub spawn_unblocked: Arc<Notify>,
    /// Fork-safety latch. Read by [`Actor::reconcile_roles`] so a halted node is
    /// never re-promoted to a participating `Signer`, and awaited on the
    /// [`Actor::run`] select so engaging it aborts every running engine at once.
    /// Cross-launch singleton shared with the executor and the OuterEngine
    /// supervisor.
    pub safety_halt: crate::sync_metrics::SafetyHalt,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`].
    pub marshal_mailbox: MarshalMailbox<BlsScheme, Standard<OrderBlock>>,
    /// Peers to target when re-driving a finalization fetch: the highest known
    /// epoch's committee, from `EpochSchemeProvider::latest_scheme`. The same closure
    /// the executor's catch-up re-fetch uses.
    pub peers_for_finalization: crate::executor::PeersForFinalization,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`].
    pub slasher_mailbox: SlasherMailbox,
    /// The notarization arm of the simplex reporter, forwarding `SpecNotarized`
    /// commands to the executor for speculative execution.
    pub spec_exec_mailbox: crate::spec_exec::Mailbox,
    /// The manager's own counters (see [`EpochEngineMetrics`]).
    pub epoch_metrics: EpochEngineMetrics,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`].
    pub page_cache: CacheRef,
    /// Every per-epoch committee this manager needs, and the one place an epoch's
    /// scheme is produced.
    ///
    /// Reading `committee[E]` through this handle IS the registration: the module
    /// builds the epoch's verify-only scheme in the same map slot as the record, so
    /// "soft-enter" is a read. The marshal's `CertProvider` is this same handle, so
    /// the epoch of a certificate it is asked to verify registers itself on that
    /// read.
    pub committee: Arc<dyn Committee>,
    /// The repair sweep's view on that same map: which epochs hold a verify-only
    /// scheme is a question only the map can answer, covering every read that made a
    /// record.
    pub scheme_pins: EpochSchemeProvider,
    /// Prefix of every per-epoch journal partition this manager opens. Production
    /// passes `""`; the in-crate deterministic testbed passes `node{i}-`, because its
    /// nodes share one in-memory `Storage`.
    pub partition_prefix: String,
    /// Devnet/test-only byzantine validator behaviour (gated behind
    /// `dpos-devnet-byzantine`). `None` on every honest node.
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
    /// Construct the actor and return the bounded `boundary_tx` sender.
    pub fn new(context: E, cfg: Config<B, XC, A>) -> (Self, mpsc::Sender<Epoch>) {
        let (boundary_tx, boundary_rx) = mpsc::channel(BOUNDARY_BUFFER);
        let actor = Self {
            context: ContextCell::new(context),
            active_epochs: BTreeMap::new(),
            boundary_rx,
            highest_entered_epoch: Epoch::new(0),
            // Subscribed here rather than in `run`: the value is read on every role
            // decision, and a `watch` holds the last published value, so a manager
            // built after the marshal's startup tip starts from that tip.
            tip: cfg.app.ordering_tip(),
            tip_reconciled_live: None,
            roles: BTreeMap::new(),
            deferred_spawns: BTreeSet::new(),
            deferred_reconciles: BTreeSet::new(),
            cfg,
        };
        (actor, boundary_tx)
    }

    /// Start the manager. The 3 broker handles are owned by the always-on plane;
    /// this manager clones them per promotion and drops them on exit (the
    /// `SubReceiver`s auto-deregister).
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
        // The Muxers live in the always-on plane. Graceful exit is via boundary_rx
        // close; the plane's tasks are not aborted here.
        // Cloned Arcs so the per-iteration `notified()` futures borrow local handles,
        // not `self`.
        let spawn_unblocked = self.cfg.spawn_unblocked.clone();
        let safety_halt = self.cfg.safety_halt.clone();
        // Subscribed once, before the loop: a `broadcast` buffers from the
        // subscription onward, so an event fired before it would be lost. One stream
        // for the share and key classes; both arms answer by re-reading.
        let mut beacon_events = self.cfg.randomness.subscribe();
        // The committee module's own wake-up, and the only retry a reconcile parked
        // on `NotReadable` has. Subscribed once, before the loop, so a wake-up fired
        // between the first read and the first `changed()` is held.
        let mut committee_wake = self.cfg.committee.anchor_advances();
        // The marshal tip's edge: a second receiver on the same channel `self.tip`
        // reads the value from, because `changed()` needs a `&mut` the value-reading
        // arms cannot lend it. This is the only edge that fires when the tip moves
        // without a boundary delivery.
        let mut tip_wake = self.tip.clone();
        // The repair sweep runs off this loop; waking it is a synchronous
        // `send_replace`. Dropping this sender on the way out stops the task.
        let (sweep_wake, sweep_handle) = self.spawn_repair_sweep();
        // The fork-safety latch's 0→1 edge, armed once. A latch already engaged
        // resolves on every call, so the arm is disarmed after its one firing.
        let halt_edge = safety_halt.engaged_edge();
        tokio::pin!(halt_edge);
        let mut halt_seen = false;
        loop {
            // Arm the `spawn_unblocked` wakeup before the select: its producer uses
            // `notify_one` (permit-storing), so a signal fired while no waiter is
            // armed is consumed by the next `notified()`.
            let spawn_n = spawn_unblocked.notified();
            tokio::pin!(spawn_n);
            tokio::select! {
                // The fork-safety latch engaged. Abort every participating engine now
                // so the node stops signing, proposing and voting immediately, and clear
                // any parked promotion. `reconcile_roles` keeps it a Verifier forever.
                // The manager stays up; recovery is external.
                _ = &mut halt_edge, if !halt_seen => {
                    halt_seen = true;
                    for (epoch, handle) in std::mem::take(&mut self.active_epochs) {
                        warn!(?epoch, "SafetyHalt engaged — aborting participating engine \
                            (demote to verify-only permanently)");
                        handle.abort();
                    }
                    // The epoch-key agreement plane goes with them, and that is
                    // the beacon's own doing: its launcher waits on this same
                    // edge and reads the latch at every spawn (`beacon::dkg_engine`).
                    self.deferred_spawns.clear();
                    for role in self.roles.values_mut() {
                        *role = Role::Verifier;
                    }
                }
                recv = self.boundary_rx.recv() => {
                    match recv {
                        Some(epoch) => {
                            // The edge carrying an epoch this node's own execution has
                            // just reached. It is not necessarily the live frontier — a
                            // node catching up crosses boundaries the network left long
                            // ago — so it is reconciled as the epoch it is.
                            self.reconcile_roles(epoch, muxes.as_ref()).await;
                            // After the reconcile, so the frontier the sweep runs under
                            // includes this boundary's epoch.
                            sweep_wake.send_replace(self.sweep_frontier());
                        }
                        None => {
                            info!("boundary_rx closed, epoch_manager exiting");
                            break;
                        }
                    }
                }
                // The live epoch moved, as the marshal's verified tip reports it. The
                // tip moves once per finalized block and carries no obligation of its
                // own; this actor owes a reconcile when the epoch that tip names
                // changes, which is the only edge that fires while this node's own
                // execution is stalled behind the network. `reconcile_live` reconciles
                // the new live epoch, whose `abort_below` retires the engine of the
                // epoch the tip just left.
                //
                // The dead engine outlives the tip that killed it by one `Update::Tip`
                // except when `committee[live]` is not readable: `reconcile_roles` reads
                // the committee before `abort_below`, so an unreadable live epoch defers
                // the whole pass until the module's wake-up fires.
                Ok(()) = tip_wake.changed() => {
                    // The gate that makes this an epoch-rate edge. A tip inside the
                    // epoch the last reconcile already ran for adds nothing: the role
                    // rule reads the tip only through `live_epoch`, while the pass itself
                    // is not free. The block-rate retries have their own edges.
                    //
                    // Deliberately not a sweep wake: waking it here would repair the
                    // beacon key of an epoch this node's execution has not reached.
                    if self.live_epoch() != self.tip_reconciled_live {
                        self.reconcile_live(muxes.as_ref()).await;
                    }
                }
                // The committee module can read higher than it could. The consumer
                // contract is "wake and re-ask", so re-run the live epoch's reconcile:
                // if it was parked on an unreadable committee it now proceeds, and if it
                // was not, `reconcile_roles` is idempotent.
                _ = committee_wake.changed() => {
                    let live = self.live_epoch();
                    let targets =
                        committee_wake_targets(&mut self.deferred_reconciles, live);
                    for epoch in targets {
                        self.reconcile_roles(epoch, muxes.as_ref()).await;
                    }
                }
                // The executor recorded a finalized block — the mid-epoch promotion
                // trigger, gated on a pending parked spawn so the per-block fire is a
                // no-op in steady state.
                _ = &mut spawn_n => {
                    if !self.deferred_spawns.is_empty() {
                        self.reconcile_live(muxes.as_ref()).await;
                    }
                }
                // The beacon says something may have changed — a DKG share landed, or a
                // `PK_epoch` did. One arm for both classes; both answer by re-reading
                // the same state.
                //
                // A landed share re-runs the reconcile so a member parked by the
                // share-gate spawns now (the running scheme is frozen at construction,
                // so this is a respawn).
                //
                // A landed `PK_epoch` has two consumers: the repair sweep below the
                // frontier, for which this arm is the fast path, and the live epoch's
                // own acquisition. This arm cannot be the sweep's only trigger: the key
                // class fires on an accepted artifact insert, which a committee that has
                // not changed never produces.
                //
                // The seed class is filtered out inside `next_reconcile_wake`.
                event = next_reconcile_wake(&mut beacon_events) => {
                    // One effect for both classes: wake the repair sweep and re-run
                    // the live epoch's reconcile.
                    if let Ok(crate::beacon::BeaconEvent::SeedRecorded) = event {
                        // Filtered out by `next_reconcile_wake`.
                        unreachable!("the seed class never leaves next_reconcile_wake");
                    }
                    sweep_wake.send_replace(self.sweep_frontier());
                    self.reconcile_live(muxes.as_ref()).await;
                }
            }
        }

        // Abort all per-epoch engine handles; their `SubReceiver`s drop and
        // auto-deregister. `abort()` is idempotent. The MuxHandle clones drop here
        // too; the plane's broker tasks stay live.
        for (epoch, handle) in std::mem::take(&mut self.active_epochs) {
            info!(?epoch, "aborting active epoch engine on exit");
            handle.abort();
        }
        // Aborted rather than left to notice the dropped sender: it can be parked
        // inside a peer pull for the whole pull timeout, and nothing it could still do
        // matters once the manager owning the scheme registry is gone.
        sweep_handle.abort();
    }

    /// The one live epoch, as a function of the marshal tip.
    ///
    /// `epoch_of(tip)`, or `epoch_of(tip) + 1` once the tip is that epoch's last
    /// block. The second arm makes the function total: at `tip == last(E)`, epoch E is
    /// finished, so a conjunction would answer `None` for every epoch at exactly that
    /// height and a node sitting on a boundary would hold no live epoch.
    ///
    /// The same rule as [`crate::dpos::local_tracked_epoch`], deliberately: the two
    /// answer one question from two cursors, and disagreeing on the boundary would put
    /// the peer set one epoch away from the engines.
    ///
    /// `None` while the geometry has not frozen. A height below activation is not a
    /// second refusal: `Geometry::epoch_of` clamps it to epoch 0.
    fn live_epoch(&self) -> Option<Epoch> {
        Some(live_epoch_of(
            &self.cfg.committee.geometry()?,
            *self.tip.borrow(),
        ))
    }

    /// True when `epoch` is the live epoch. Every other epoch only soft-enters — a
    /// Simplex engine for an epoch the network has left has no live peers and would
    /// drive the executor on a dead fork.
    ///
    /// Equality, not `>=`: over a verified tip there is exactly one live epoch, and an
    /// epoch either side of it gets a verify-only scheme and no engine.
    fn is_live_epoch(&self, epoch: Epoch) -> bool {
        self.cfg
            .committee
            .geometry()
            .is_some_and(|g| is_live_epoch_at(&g, *self.tip.borrow(), epoch))
    }

    /// Reconcile this node's per-epoch role from current state — the single decision
    /// point. `role(E) = Signer iff (I ∈ committee[E]) ∧ is_live_epoch(E)`; a `Signer`
    /// additionally needs a usable DKG share and the `Inline::genesis(E)` precondition
    /// before its engine spawns. Idempotent.
    async fn reconcile_roles<HS, HR>(&mut self, epoch: Epoch, muxes: Option<&Muxes<HS, HR>>)
    where
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
    {
        // The tip edge's memo, taken before the read below can defer this pass:
        // whatever it decides, this is the answer for the live epoch. A pass that
        // defers on an unreadable committee memoizes too, because its retry is
        // `deferred_reconciles`, drained by the module's own wake-up.
        if self.live_epoch() == Some(epoch) {
            self.tip_reconciled_live = Some(epoch);
        }
        // The epoch's committee, as one frozen value read at this node's own anchor.
        // Every edge carries an epoch and nothing else, and a snapshot handed in by
        // whoever fired the edge would be a second authority on a value the module
        // already froze.
        //
        // Not readable is a deferral, never a decision: the anchor only moves up, and
        // the module's wake-up re-runs this reconcile. Nothing below may run on a guess
        // about the committee.
        let record = match self.cfg.committee.committee(epoch.get()) {
            Ok(record) => {
                // This epoch is being reconciled now, so it is no longer owed one.
                self.deferred_reconciles.remove(&epoch);
                record
            }
            Err(e) => {
                park_committee_miss(
                    &mut self.deferred_reconciles,
                    &self.cfg.safety_halt,
                    epoch,
                    &e,
                );
                return;
            }
        };
        // Boundary bookkeeping (idempotent; monotone).
        self.highest_entered_epoch = self.highest_entered_epoch.max(epoch);
        // Tell the randomness subsystem where the core stands; it owns what that
        // means. The report sits after the frontier update because the retention floor
        // is taken from `highest_entered_epoch`.
        self.cfg
            .randomness
            .observe_epoch(epoch, self.highest_entered_epoch);

        // Exit-at-transition: abort every engine strictly below the live epoch.
        // `e < cutoff` only, so a replayed boundary for an old epoch can never abort a
        // newer engine.
        self.abort_below(epoch);

        // Not the live epoch → soft-enter: verify-only scheme, no engine. A Simplex
        // engine for a stale epoch has no live peers and would drive a dead fork.
        // Verify-only lets the marshal verify this epoch's certs.
        if !self.is_live_epoch(epoch) {
            self.soft_enter(epoch).await;
            self.deferred_spawns.remove(&epoch);
            info!(?epoch, "epoch soft-entered (scheme only, catch-up)");
            return;
        }

        // At the live frontier: role = f(membership). A member only spawns once the
        // share-gate and `boundary_lookup` both hold below, which together mean the
        // local executor has derived up to E-1's boundary.
        //
        // A SafetyHalted node is never a member for role purposes; it stays a Verifier
        // forever.
        let is_member = !self.cfg.safety_halt.is_engaged()
            && self.cfg.signer_keypair.is_some()
            && record.participants.position(&self.cfg.me).is_some();

        // Already a running signer for the live epoch — keep it unless its handle has
        // completed. Membership cannot change mid-epoch, but under `catch_panics(true)`
        // a child panic completes the handle without reaching the manager, so poll it:
        // `Pending` keeps it, `Ready` falls through to the spawn path.
        if let Some(handle) = self.active_epochs.get_mut(&epoch) {
            if !engine_handle_dead(handle) {
                // A live engine is still subject to the participation question: a
                // quorum can attest an epoch key after the spawn, and if it differs from
                // the one this node reconstructed, every vote it casts carries a seed
                // partial the rest of the committee rejects. The probe is stable rather
                // than transient, so this cannot flap an engine down and up.
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
                // Share-gate: a beacon-active member that cannot participate must not
                // run a participating engine, because a seedless Simplex member rejects
                // honest peers' seeded votes and wedges. The probe is non-blocking; on
                // `Withheld`, register verify-only and stay off the consensus plane.
                //
                // The position is load-bearing: it sits before the boundary lookup so a
                // member that cannot participate does not pay a marshal `get_block` on
                // every participation edge.
                if let ShareProbe::Withheld(reason) = self.cfg.randomness.can_participate(epoch) {
                    self.soft_enter(epoch).await;
                    info!(
                        ?epoch,
                        ?reason,
                        "committee member cannot participate — verify-only (share-gate)"
                    );
                    return;
                }

                // `Inline::genesis(E)` precondition: the E-1 boundary block must be in
                // marshal storage before the engine starts, else the engine hits
                // `unreachable!`. On a mid-epoch promotion the marshal may still be
                // backfilling it — defer, never panic; register verify-only meanwhile.
                //
                // `snap` is a projection of the one frozen record, used by all three
                // surfaces this crate does not own, so they cannot be handed three
                // different committees for one epoch.
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
                        // Two causes, two lines: an operator chasing a stuck block fetch
                        // when the block is in hand and the sigma is absent looks in the
                        // wrong place entirely.
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

                // The leader lottery, derived from the frozen record before anything is
                // registered or spawned: a spawn that raised this epoch's scheme to the
                // signer half and then found no leader schedule would leave the epoch
                // half-entered.
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

                // The scheme this epoch votes with, built whole by the randomness
                // subsystem. A successful signer pays a second material resolve per
                // reconcile edge, because the material is sampled once by `share_probe`
                // and once here across the boundary await.
                let Some(keypair) = self.cfg.signer_keypair.clone() else {
                    unreachable!("Role::Signer requires is_member, which requires a signer keypair")
                };
                let scheme =
                    match signer_decision(self.cfg.randomness.signer(epoch, &snap, &keypair)) {
                        SignerDecision::Spawn(scheme) => *scheme,
                        SignerDecision::SoftEnter(reason) => {
                            match reason {
                                // Misconfiguration safety net, not a wedge path: this node's
                                // BLS key is not in the committee BiMap, so the scheme it was
                                // handed cannot sign and an engine over it would only be
                                // aborted by the next reconcile.
                                SoftEnterReason::RotatedKey => {
                                    metrics::counter!("epoch_engine_rotated_out_total")
                                        .increment(1);
                                    warn!(
                                        ?epoch,
                                        "validator BLS key not in committee BiMap — verify-only"
                                    );
                                }
                                SoftEnterReason::Withheld(reason) => {
                                    info!(?epoch, ?reason, "withheld from signing — verify-only");
                                }
                            }
                            self.soft_enter(epoch).await;
                            return;
                        }
                        // The engine's own decode failed: warn and skip the epoch.
                        SignerDecision::Skip(e) => {
                            warn!(
                                ?epoch,
                                ?e,
                                "skipping epoch spawn — invalid committee snapshot"
                            );
                            return;
                        }
                    };
                // Raise the module's verify-only entry to this node's signer half — the
                // one upgrade path. A refusal preserves the stronger entry and is not a
                // spawn gate: the engine holds its own instance either way.
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
                    // Stable greppable token for the production-path smoke test.
                    info!(
                        ?epoch,
                        "promoted to Signer in-process: per-epoch BFT engine started"
                    );
                }
            }
        }
    }

    /// Register a verify-only (multisig) scheme for `epoch` and record the
    /// `Verifier` role, unless this node already holds a `Signer` for `epoch`.
    /// Idempotent: never downgrades an active signer, which the provider refuses and
    /// which would desync `self.roles` from the provider.
    async fn soft_enter(&mut self, epoch: Epoch) {
        if self.active_epochs.contains_key(&epoch) || self.roles.get(&epoch) == Some(&Role::Signer)
        {
            return;
        }
        // Reading the committee is the registration: the module builds this epoch's
        // verify-only scheme in the same map slot as the record. A `None` here means
        // the epoch is not readable at this anchor yet, which the module's wake-up
        // retries.
        let registered = self.cfg.committee.scheme(epoch.get()).is_some();
        debug!(
            ?epoch,
            registered, "soft-enter: verify-only scheme taken from the committee module"
        );
        self.roles.insert(epoch, Role::Verifier);
    }

    /// Start the repair sweep on its own task and return the handle that wakes it.
    ///
    /// The sweep must not run on the `select!`: its ladder spends a bounded peer pull
    /// per unpinned epoch, sleeping until the epoch's next slot before each fetch, so
    /// awaited inline it would block the arms that turn this node into a signer.
    ///
    /// Everything it touches is a cross-epoch singleton held by `Arc`; the only
    /// per-wake state is the frontier, which rides the wake. A `watch` because the
    /// receiver is created once here, before the task's loop.
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
    /// strictly below `max(frontier, entered)`. The frontier's only job is to be the
    /// live epoch, which the sweep must exclude. With no geometry there is no live
    /// epoch and `0` leaves the entered tip as the whole frontier.
    fn sweep_frontier(&self) -> (Epoch, Epoch) {
        (
            self.live_epoch().unwrap_or(Epoch::new(0)),
            self.highest_entered_epoch,
        )
    }

    /// Re-run [`Self::reconcile_roles`] for the live epoch. The edges that carry no
    /// epoch of their own (tip / share / spawn_unblocked / the module's wake-up)
    /// reconcile this.
    ///
    /// The epoch can still move between the choice here and the liveness gate inside
    /// `reconcile_roles`, because the tip is a `watch` another task writes. What that
    /// costs is one `soft_enter` pass; it cannot cause a downgrade, because
    /// `soft_enter` returns early on a running engine or a recorded `Signer`.
    async fn reconcile_live<HS, HR>(&mut self, muxes: Option<&Muxes<HS, HR>>)
    where
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
    {
        if let Some(epoch) = self.live_epoch() {
            self.reconcile_roles(epoch, muxes).await;
        }
    }

    /// The `Inline::genesis(E)` precondition lookup on the E-1 terminal block — the
    /// exact lookup `Inline::genesis` performs — which also yields the seedless arm's
    /// base. Gate and base come from one `get_block`, so it is impossible to spawn on a
    /// block the base was not taken from.
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

    /// Abort engines of all epochs strictly below `current`, and prune
    /// `deferred_spawns` of every parked epoch below the same cutoff. A frontier that
    /// advances via a catch-up span leaves an orphaned `deferred_spawns` entry whose
    /// precondition is moot; pruning makes an empty set the true "no pending
    /// promotion" signal the `spawn_unblocked` edge gates on.
    fn abort_below(&mut self, current: Epoch) {
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
        self.deferred_spawns.retain(|e| e.get() >= cutoff);
        // Prune `roles` to the trailing scheme-retention window: the sole reader is
        // `soft_enter`'s lookup for the epoch under reconcile, so older entries are
        // never read again.
        let roles_floor = cutoff.saturating_sub(SCHEME_RETENTION_EPOCHS as u64);
        self.roles.retain(|e, _| e.get() >= roles_floor);
    }

    /// Build and start the per-epoch Simplex engine and register its 3 sub-channels.
    /// Returns `false` (spawning nothing) on an invalid committee snapshot or a
    /// muxer-register failure; the epoch stays un-promoted.
    ///
    /// `false` schedules no retry deliberately: both failure causes are conditions
    /// this node cannot resolve by waiting, and the `spawn_unblocked` edge is gated on
    /// `deferred_spawns`, which the caller does not enter.
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
        // `None` means a follower manager: its `signer_keypair` is `None`, so
        // `is_member` is always false and the `Role::Signer` arm that reaches here is
        // never taken.
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

/// The repair sweep's driver: one sweep per wake, never two at once, on its own
/// task. Exits when the last wake sender drops.
///
/// Wakes coalesce rather than queue, which keeps a sweep that is spending network
/// pulls from stacking concurrent pulls for the same epoch. The frontier is the one
/// from the wake it consumed, so a stale frontier is a lower one — the frontier is an
/// upper exclusion bound, so staleness can only skip a repairable epoch, never pin
/// under a live engine.
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
/// unpinned and below the live frontier. Returns the epochs this call upgraded.
///
/// The work list comes from the scheme provider rather than from a set the manager
/// keeps: the provider sees every registration, including the bulk catch-up span
/// and the marshal `CertProvider` path.
///
/// Epochs at or above the frontier are excluded: a live epoch's scheme belongs to
/// its engine, and this sweep runs on its own task, so a pin applied here would race
/// the registration. `reconcile_live` re-runs the whole ladder for the live epoch on
/// every non-boundary edge.
///
/// The frontier is the higher of `observed` (the live epoch) and `entered` (the
/// highest epoch registered through a reconcile), because `observed` is `Epoch(0)`
/// while the geometry has not frozen.
///
/// A wrong key is terminal for that epoch, so this path asks for the thorough
/// resolution rather than the cheap one. Which provenance it admits is stated in
/// [`Beacon::ensure_key`].
///
/// `hinted` is the sweep's memo of epochs it has already re-driven, bounded by the
/// registry: it is intersected with the candidate list on every pass.
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
        // The work list is keyed on the key store rather than on the scheme: a scheme
        // reads its epoch key live through its oracle, so there is nothing to repair in
        // the registry. `Local` resolves and writes into the store the oracle reads.
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

/// What a reconcile does with a committee it could not read.
#[derive(Debug, PartialEq, Eq)]
enum CommitteeMiss {
    /// Park the epoch in `deferred_reconciles`; the module's wake-up retries it.
    Defer,
    /// Engage the fork-safety latch: the chain contradicted its own invariants.
    Halt,
    /// Nothing to do: the epoch is below the read window and can never be read.
    Refuse,
}

/// The reconcile's verdict on a committee read failure.
///
/// A [`fluentbase_staking_reader::ReadClass::Permanent`] failure — a revert, or
/// this node's own storage — is deferred exactly like a transient miss: the store
/// re-reads on every call, so the next anchor advance is a retry, and the failure
/// says nothing about the committed state. Only an epoch below the read window is
/// refused outright, because the window never moves down.
fn committee_miss(e: &crate::committee::CommitteeError) -> CommitteeMiss {
    use crate::committee::CommitteeError;
    match e {
        CommitteeError::NotReadable { .. } => CommitteeMiss::Defer,
        CommitteeError::OutOfWindow { epoch, hi, .. } if epoch > hi => CommitteeMiss::Defer,
        CommitteeError::OutOfWindow { .. } => CommitteeMiss::Refuse,
        CommitteeError::Read(read) if read.is_contract_impossible() => CommitteeMiss::Halt,
        CommitteeError::Read(_) => CommitteeMiss::Defer,
    }
}

/// Act on a committee read failure inside `reconcile_roles`: park the epoch for the
/// module's wake-up, engage the fork-safety latch, or refuse it. Free so the arm
/// itself is unit-testable without an actor.
///
/// The halt is the one site that turns an impossible committee into a node halt.
/// The reconcile that reaches it was owed — the epoch arrived on a boundary this
/// node's own execution reached, on the live-epoch edge, or on the module's
/// wake-up — never as a lookahead or a stranger's claim, which is why the halt
/// lives here and not in the module. `Impossible` and not `!is_transient()`: a
/// revert or a storage fault is deferred like a transient miss (the read could not
/// be served, which says nothing about the committed state), while an impossible
/// answer is the chain contradicting its own invariants, which every validator
/// reads the same way; skipping it would stop the network in silence. The latch
/// aborts every running engine and keeps this node a Verifier.
fn park_committee_miss(
    deferred: &mut BTreeSet<Epoch>,
    safety_halt: &crate::sync_metrics::SafetyHalt,
    epoch: Epoch,
    e: &crate::committee::CommitteeError,
) {
    match committee_miss(e) {
        CommitteeMiss::Defer => {
            deferred.insert(epoch);
            debug!(
                ?epoch,
                %e,
                "reconcile deferred — committee[E] not readable at this node's anchor yet, \
                 or its read failed; retried on the module's wake-up"
            );
        }
        CommitteeMiss::Halt => {
            if !safety_halt.is_engaged() {
                error!(
                    ?epoch,
                    %e,
                    reason = SyncReason::ContractFork.as_str(),
                    "committee[E] of an epoch this node must enter is IMPOSSIBLE — the \
                     contract contradicted its own invariants; SafetyHalt: this node stops \
                     signing, proposing and voting (marshal keeps serving). Recovery is a \
                     repaired chain and a fresh start."
                );
            }
            // Idempotent, and the first verdict is the one recorded.
            safety_halt.engage(SyncReason::ContractFork);
        }
        CommitteeMiss::Refuse => debug!(
            ?epoch,
            %e,
            "reconcile refused — committee[E] is below this node's read window"
        ),
    }
}

/// Why a member at the live frontier registers verify-only instead of spawning.
#[derive(Debug)]
enum SoftEnterReason {
    /// This node's BLS key is not in the committee BiMap ([`SignerVerdict::RotatedKey`]).
    RotatedKey,
    /// The beacon withheld the signing material ([`SignerVerdict::Withheld`]).
    Withheld(WithheldReason),
}

/// What the manager does with a [`SignerVerdict`] at the live frontier.
#[derive(Debug)]
enum SignerDecision {
    /// Raise the epoch's scheme to this signer half and spawn its engine.
    Spawn(Box<BlsScheme>),
    /// Register verify-only and return; no engine.
    SoftEnter(SoftEnterReason),
    /// Neither: the snapshot could not be decoded.
    Skip(commonware_utils::ordered::Error),
}

/// Map the beacon's verdict to the manager's decision.
///
/// `RotatedKey` soft-enters: the scheme it carries cannot sign (this node's key is
/// absent from the BiMap), so an engine over it would only ever be aborted by the
/// next reconcile, and the verify-only registration is what that engine would have
/// amounted to.
fn signer_decision(verdict: SignerVerdict) -> SignerDecision {
    match verdict {
        SignerVerdict::Signs(scheme) => SignerDecision::Spawn(Box::new(scheme)),
        SignerVerdict::RotatedKey(_) => SignerDecision::SoftEnter(SoftEnterReason::RotatedKey),
        SignerVerdict::Withheld(reason) => {
            SignerDecision::SoftEnter(SoftEnterReason::Withheld(reason))
        }
        SignerVerdict::InvalidCommittee(e) => SignerDecision::Skip(e),
    }
}

/// The live epoch for a marshal tip, over the frozen geometry — the body of
/// [`Actor::live_epoch`], free so the rule is unit-testable.
///
/// Its only inputs are the geometry and a height the marshal has verified, so no
/// epoch tag off the wire can name the live epoch.
///
/// Also the follower's `T` (`dpos::local_tracked_epoch`): a node that runs no
/// `EpochTransition` computes the epoch it tracks by this same rule.
pub(crate) fn live_epoch_of(geometry: &crate::committee::Geometry, tip: u64) -> Epoch {
    let e = geometry.epoch_of(tip);
    // `tip == last(e)` means epoch `e` is finished and `e + 1` is live.
    Epoch::new(if geometry.last(e) == tip { e + 1 } else { e })
}

/// The role gate — [`Actor::is_live_epoch`]'s body, free for the same reason
/// [`live_epoch_of`] is.
///
/// Equality: over a verified tip there is exactly one live epoch, and an epoch on
/// either side of it gets a verify-only scheme and no engine.
fn is_live_epoch_at(geometry: &crate::committee::Geometry, tip: u64, epoch: Epoch) -> bool {
    live_epoch_of(geometry, tip) == epoch
}

/// The epochs one committee wake-up must reconcile, in order, and the set of owed
/// reconciles it clears.
///
/// First every epoch whose reconcile deferred on an unreadable committee; then the
/// live epoch, which may already be in the first group and must not be reconciled
/// twice.
///
/// The deferred set is drained rather than filtered: `reconcile_roles` re-enters the
/// epoch itself if the committee is still unreadable.
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

/// True when a per-epoch engine `Handle` has completed — normal exit or a panic
/// caught by `catch_panics(true)`. Commonware `Handle` exposes only `abort()` and
/// `impl Future`, so it is polled once with a no-op waker.
fn engine_handle_dead(handle: &mut Handle<()>) -> bool {
    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    matches!(Pin::new(handle).poll(&mut cx), Poll::Ready(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        beacon::testing::LiveBeaconConfig, beacon::testing::MintFixture,
        beacon::testing::DETERMINISTIC_BOOTSTRAP_EPOCH, outer::EpochSchemeProvider,
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

    use fluentbase_bls::scheme::build_signer;
    use fluentbase_bls::BlsPubkey;
    use fluentbase_staking_reader::reader::{ConsensusKeys, ValidatorWithKeys};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;

    /// Committee fixture for the repair-sweep tests: BLS keypairs behind the
    /// snapshot, and a provider over the real key index so the tests exercise the
    /// shipped resolve rather than canned answers.
    fn randomness_over(mints: &MintFixture) -> Arc<dyn Beacon> {
        randomness_over_seeds(crate::beacon::testing::SeedStore::new(), mints)
    }

    fn randomness_over_seeds(
        seeds: crate::beacon::testing::SeedStore,
        mints: &MintFixture,
    ) -> Arc<dyn Beacon> {
        crate::beacon::testing::LiveBeacon::build(LiveBeaconConfig {
            seeds,
            keys: mints.keys.clone(),
            ceremony: Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new())),
            acquire: None,
            metrics: crate::beacon::testing::BeaconMetrics::default(),
            chain_id: 1,
            artifacts: mints.artifacts.clone(),
            geometry: tokio::sync::watch::channel(Some((0, 1))).1,
        })
    }

    /// An [`AcquireArtifact`] that parks inside its bounded fetch, so a test can hold
    /// the sweep in the middle of one.
    struct ParkingAcquire {
        parked: Arc<Notify>,
        gate: Arc<Notify>,
        pulls: Arc<std::sync::atomic::AtomicUsize>,
        /// Where the artifact lands when the gate releases.
        artifacts: crate::beacon::testing::ArtifactStore,
    }

    impl crate::beacon::testing::AcquireArtifact for ParkingAcquire {
        fn fetch(&self, minted_at: u64) -> BoxFuture<'_, bool> {
            Box::pin(async move {
                self.pulls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                self.parked.notify_one();
                self.gate.notified().await;
                // First-wins: a second fetch of one epoch keeps the first value.
                drop(self.artifacts.insert(
                    minted_at,
                    crate::beacon::testing::artifact_with_key(minted_at, some_outcome(0xB2)),
                ));
                true
            })
        }
    }

    /// [`randomness_over`] with an acquisition route wired — the only thing a
    /// `Thorough` effort can spend.
    fn randomness_over_acquiring(
        mints: &MintFixture,
        acquire: crate::beacon::testing::AcquireMint,
    ) -> Arc<dyn Beacon> {
        crate::beacon::testing::LiveBeacon::build(LiveBeaconConfig {
            seeds: crate::beacon::testing::SeedStore::new(),
            keys: mints.keys.clone(),
            ceremony: Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new())),
            acquire: Some(acquire),
            metrics: crate::beacon::testing::BeaconMetrics::default(),
            chain_id: 1,
            artifacts: mints.artifacts.clone(),
            geometry: tokio::sync::watch::channel(Some((0, 1))).1,
        })
    }

    /// A real DKG outcome, for a fixture that has to state a mint.
    fn some_outcome(seed: u64) -> crate::beacon::testing::DkgOutcome {
        use commonware_cryptography::bls12381::{dkg::deal, primitives::sharing::Mode};
        use commonware_utils::ordered::Set;
        let mut rng = StdRng::seed_from_u64(seed);
        let players: Set<fluentbase_bls::PeerPubkey> =
            Set::from_iter_dedup((0..4).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()));
        deal::<MinSig, fluentbase_bls::PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
            .expect("deal")
            .0
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

    /// Register `epoch` pin-less, straight into the provider, touching no
    /// epoch-manager state. Takes the randomness handle because the scheme's
    /// verification strength comes from the oracle [`Beacon::oracle_for`] attaches.
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

    /// A committee module holding only what the sweep reads — the scheme map. The
    /// work list IS `verifier_epochs()`.
    fn sweep_committee() -> Arc<crate::committee::testing::SchemeCommittee> {
        crate::committee::testing::SchemeCommittee::new(|_| None)
    }

    /// A finalization certificate over `epoch`'s committee whose multisig half is
    /// genuine and whose seed slot carries a signature from an unrelated dealing.
    /// The multisig quorum has to verify, or the seed arm is never reached; the seed
    /// has to be a well-formed G1 point, or it would be refused as garbage rather
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

        // One member's partial from a different dealing: a real G1 signature over the
        // right round under the wrong group.
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

    /// A node that followed epoch E as a verifier must have E in
    /// `verifier_epochs()`, the sweep's work list. Once the frontier passes E the
    /// sweep is what fetches `PK_E`; without it the sigma captured at ingress could
    /// never leave quarantine.
    #[tokio::test]
    async fn a_soft_entered_epoch_is_swept_for_its_key_once_the_frontier_passes_it() {
        let epoch = Epoch::new(DETERMINISTIC_BOOTSTRAP_EPOCH + 3);
        let mints = MintFixture::new();
        let r = randomness_over(&mints);
        let module = sweep_committee();
        let provider = EpochSchemeProvider::new(module.clone());
        register_verifier(module.as_ref(), epoch, r.as_ref());
        assert!(
            provider.verifier_epochs().contains(&epoch),
            "a soft-entered epoch is on the sweep's work list"
        );

        // While the frontier is at the epoch, it is live and the sweep leaves it
        // alone.
        assert!(repair_keyless_schemes(
            &provider,
            r.as_ref(),
            &mut BTreeSet::new(),
            epoch,
            Epoch::new(0),
        )
        .await
        .is_empty());

        mints.mint(epoch.get(), some_outcome(0xA1));
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

    /// The property the sweep exists for, asserted on the scheme rather than on the
    /// return value: the same `Arc<BlsScheme>` handle must admit the foreign seed
    /// while the epoch is keyless (the certificate is carried by a genuine multisig
    /// quorum) and refuse it once `PK_epoch` is in the store, with no re-registration
    /// in between.
    #[tokio::test]
    async fn the_registered_scheme_starts_refusing_a_foreign_seed_when_the_key_lands() {
        use commonware_cryptography::certificate::{Provider as _, Scheme as _};
        let epoch = Epoch::new(DETERMINISTIC_BOOTSTRAP_EPOCH + 3);
        let mints = MintFixture::new();
        // The chain's bit is frozen up front; the artifact is what lands mid-test.
        mints.changed(epoch.get());
        let r = randomness_over(&mints);
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

        mints.arrive(epoch.get(), some_outcome(0xA1));
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

    /// The sweep runs on every boundary delivery, so the steady state — every key
    /// held and already hinted — must cost nothing. Reds if it spends the network rung
    /// on an epoch whose key the store holds, or if the hint memo stops holding.
    #[tokio::test]
    async fn a_sweep_over_held_keys_spends_no_network_and_hints_once() {
        let epoch = Epoch::new(5);
        let module = sweep_committee();
        let provider = EpochSchemeProvider::new(module.clone());
        // "Nothing to repair" means the key store holds the epoch's key.
        let mints = MintFixture::new();
        mints.mint(epoch.get(), some_outcome(0xA1));

        // No acquisition route is wired at all: an epoch whose key is held must
        // resolve without one.
        let r = randomness_over(&mints);
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

    /// The scope half: the edge this replaces looked at one cached epoch, so an epoch
    /// that missed its own boundary window was never retried. Here the key for the
    /// older of two unpinned epochs resolves; a sweep scoped to the newest would report
    /// nothing.
    #[tokio::test]
    async fn the_sweep_repairs_every_unpinned_epoch_not_just_the_newest() {
        let (older, newer) = (Epoch::new(5), Epoch::new(6));
        let module = sweep_committee();
        let provider = EpochSchemeProvider::new(module.clone());
        // Both epochs re-minted, so neither can borrow the other's key through a
        // carry; only `older`'s artifact is on hand.
        let mints = MintFixture::new();
        mints.mint(older.get(), some_outcome(0xA1));
        mints.changed(newer.get());
        let r = randomness_over(&mints);
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

    /// `verify_certificate` reads oracle presence as "this epoch is beacon-active" and
    /// rejects every seedless certificate under it. Pre-bootstrap epochs are
    /// legitimately seedless, so `ensure_key` must not resolve a key there and
    /// `oracle_for` must not attach an oracle there.
    #[tokio::test]
    async fn the_sweep_never_pins_below_the_bootstrap_epoch() {
        use commonware_cryptography::certificate::Provider as _;
        let pre_beacon = Epoch::new(DETERMINISTIC_BOOTSTRAP_EPOCH - 1);
        let module = sweep_committee();
        let provider = EpochSchemeProvider::new(module.clone());
        let mints = MintFixture::new();
        mints.mint(pre_beacon.get(), some_outcome(0xA1));
        let r = randomness_over(&mints);
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

    /// A signer's engine resolves its own epoch key through the promote gates, which
    /// have a value-gate this sweep does not. The sweep's thorough rung writes what it
    /// resolves as the network's `PK_epoch`, so spending it under an engine that
    /// already judged its own material is redundant work with a terminal failure
    /// mode.
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

        let mints = MintFixture::new();
        mints.mint(epoch.get(), some_outcome(0xA1));

        let upgraded = repair_keyless_schemes(
            &provider,
            randomness_over(&mints).as_ref(),
            &mut BTreeSet::new(),
            Epoch::new(7),
            Epoch::new(0),
        )
        .await;

        assert!(upgraded.is_empty());
    }

    /// The coverage the provider-sourced work list buys: the bulk catch-up span
    /// registers straight into the provider and writes no manager state, so a
    /// shadow-sourced sweep would be blind to every epoch it registered.
    #[tokio::test]
    async fn a_span_registered_epoch_is_repairable() {
        let epoch = Epoch::new(5);
        let module = sweep_committee();
        let provider = EpochSchemeProvider::new(module.clone());
        let mints = MintFixture::new();
        mints.mint(epoch.get(), some_outcome(0xA1));
        let r = randomness_over(&mints);
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
    /// freezes there is no live epoch, so the `observed` leg is `Epoch(0)` and every
    /// registered epoch is at or above the frontier. The epoch a boundary delivery
    /// entered is the evidence in that window.
    #[tokio::test]
    async fn the_sweep_reaches_below_an_entered_epoch_with_no_live_epoch_yet() {
        let epoch = Epoch::new(5);
        let module = sweep_committee();
        let provider = EpochSchemeProvider::new(module.clone());
        let mints = MintFixture::new();
        mints.mint(epoch.get(), some_outcome(0xA1));

        let r = randomness_over(&mints);
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

    /// The caller re-drives a marshal fetch per returned epoch, so the return value
    /// has to be the keyless-to-keyed transition and not "every epoch I looked at".
    /// Otherwise every trigger re-hints for as long as anything is keyless.
    #[tokio::test]
    async fn a_repaired_epoch_re_drives_its_finalization_hint_exactly_once() {
        let epoch = Epoch::new(5);
        let module = sweep_committee();
        let provider = EpochSchemeProvider::new(module.clone());
        let mints = MintFixture::new();
        mints.mint(epoch.get(), some_outcome(0xA1));

        let r = randomness_over(&mints);
        register_verifier(module.as_ref(), epoch, r.as_ref());
        let mut hinted = BTreeSet::new();
        let sweep = async |hinted: &mut BTreeSet<Epoch>| {
            repair_keyless_schemes(&provider, r.as_ref(), hinted, Epoch::new(7), Epoch::new(0))
                .await
        };
        assert_eq!(sweep(&mut hinted).await, vec![epoch]);
        assert_eq!(sweep(&mut hinted).await, Vec::<Epoch>::new());
    }

    /// After the key store collapsed to one tier, the only thing that makes an epoch
    /// resolvable is the artifact a `committee[minted_at]` quorum certified. The sweep
    /// upgrades an epoch only when the chain-named mint's artifact is held, and leaves
    /// an epoch with no artifact alone rather than pinning it on something weaker.
    #[tokio::test]
    async fn the_sweep_upgrades_only_an_epoch_whose_mint_artifact_is_held() {
        let (held, absent) = (Epoch::new(5), Epoch::new(6));
        let module = sweep_committee();
        let provider = EpochSchemeProvider::new(module.clone());
        // `held` re-minted with its artifact on hand; `absent` re-minted but its
        // artifact never arrived. Both are change epochs, so neither can borrow the
        // other's key.
        let mints = MintFixture::new();
        mints.mint(held.get(), some_outcome(0xA1));
        mints.changed(absent.get());
        let r = randomness_over(&mints);
        register_verifier(module.as_ref(), held, r.as_ref());
        register_verifier(module.as_ref(), absent, r.as_ref());

        let upgraded = repair_keyless_schemes(
            &provider,
            r.as_ref(),
            &mut BTreeSet::new(),
            Epoch::new(9),
            Epoch::new(0),
        )
        .await;
        assert_eq!(
            upgraded,
            vec![held],
            "only the epoch whose chain-named mint is on hand may be upgraded: {upgraded:?}"
        );
    }

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
            randomness_over(&MintFixture::new()).as_ref(),
        );

        // Both directions are permit-storing, so neither side can miss the other's
        // signal by being late.
        let parked = Arc::new(Notify::new());
        let gate = Arc::new(Notify::new());
        let pulls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        // The artifact is absent, and the acquisition parks inside the bounded fetch.
        let mints = MintFixture::new();
        mints.changed(9);
        let acquire: crate::beacon::testing::AcquireMint = Arc::new(ParkingAcquire {
            parked: parked.clone(),
            gate: gate.clone(),
            pulls: pulls.clone(),
            artifacts: mints.artifacts.clone(),
        });

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
            randomness_over_acquiring(&mints, acquire),
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

    /// The seedless arm inherits the E-1 terminal block's witness whenever the block
    /// carries one, and falls back to the constant derivation only where no witness
    /// can exist. `Missing` is not a base at all: that epoch defers its spawn.
    ///
    /// The fixture committee is empty on purpose: what is under test is which arm is
    /// selected, not the constant derivation's inputs.
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

    /// A store holding a real threshold sigma for exactly `round`, so the boundary
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

    /// The base for epoch E is sigma of E-1's terminal round, read from the store at
    /// the round the boundary block names. A local miss is `Missing`, which defers the
    /// spawn, never `Present { seed: None }`, which would elect on the constant base
    /// while a peer that holds the sigma elects on the witness one.
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
        let held = randomness_over_seeds(seeds.clone(), &MintFixture::new());
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

        // The same block on a node whose store holds sigma for a neighbouring round:
        // the pin answers only its own round, so this is the ordinary "sigma has not
        // landed here yet" miss.
        let stale = randomness_over_seeds(
            seeds_holding(SimplexRound::new(prev, View::new(TERMINAL_VIEW - 1))),
            &MintFixture::new(),
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

        // Predicate first: below the bootstrap edge no sigma can exist, so `None` is
        // the agreed answer and the store is never consulted.
        let inactive = Epoch::new(DETERMINISTIC_BOOTSTRAP_EPOCH - 1);
        assert_eq!(
            boundary_base(
                randomness_over_seeds(
                    seeds_holding(SimplexRound::new(inactive, View::new(TERMINAL_VIEW))),
                    &MintFixture::new(),
                )
                .as_ref(),
                inactive,
                TERMINAL_VIEW,
            ),
            BoundaryLookup::Present { seed: None },
            "a beacon-inactive predecessor takes the constant base even with a σ in the store"
        );
    }

    // A single peer (even naming u64::MAX) must not advance the live frontier.
    // n = 4 ⇒ f = 1 ⇒ threshold f+1 = 2.

    /// A held artifact answers for the epoch, and nothing local can answer instead.
    #[tokio::test]
    async fn a_held_artifact_is_the_whole_of_what_answers_for_an_epoch() {
        let mints = MintFixture::new();
        // The chain's record is frozen first; only the artifact arrives later.
        mints.changed(9);
        let r = randomness_over(&mints);
        assert!(
            !crate::beacon::Beacon::ensure_key(r.as_ref(), 9, crate::beacon::PinEffort::Local)
                .await,
            "with no artifact anywhere nothing may answer for the epoch"
        );
        mints.arrive(9, some_outcome(0xC3));
        assert!(
            crate::beacon::Beacon::ensure_key(r.as_ref(), 9, crate::beacon::PinEffort::Local).await,
            "the mint's artifact is the rung — and it answers without a network round-trip"
        );
    }

    // `engine_handle_dead` must read a parked engine as alive and both completed and
    // aborted handles as dead. Under `catch_panics(true)` a panicked child surfaces as
    // a completed handle, so this is the signal that revives it.
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

            // Drive the scheduler until the completed and aborted handles read dead,
            // bounded so a regression fails instead of hanging. A completed handle must
            // not be re-polled.
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

    // The manager's loop must not turn once a round. The seed class fires at the block
    // rate and belongs to the executor; this actor's `select!` rebuilds two `Notify`
    // futures every iteration.
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

        // A dead publisher is returned, not swallowed: it may hide a class this actor
        // acts on, and a swallowed `Closed` would park the arm forever.
        drop(tx);
        assert!(matches!(
            next_reconcile_wake(&mut rx).await,
            Err(tokio::sync::broadcast::error::RecvError::Closed)
        ));
    }

    /// The live epoch is `epoch_of(verified tip)`, and the epoch after it once the
    /// tip is that epoch's terminal block.
    ///
    /// With the old `>=` gate the third case fails: a node three epochs behind would
    /// call its own stale epoch live and spawn an engine on a fork the network has
    /// left.
    #[test]
    fn the_live_epoch_is_the_verified_tips_epoch_and_the_next_one_at_a_terminal() {
        let geometry = crate::committee::Geometry::new(0, 32).expect("nonzero interval");
        let live = |tip: u64| live_epoch_of(&geometry, tip);
        // The production gate, not a mirror of it.
        let is_live = |epoch: u64, tip: u64| is_live_epoch_at(&geometry, tip, Epoch::new(epoch));

        // Mid-epoch: the tip is inside epoch 4 (blocks 128..=159).
        assert_eq!(live(140), Epoch::new(4), "a tip inside E makes E live");
        assert!(is_live(4, 140));
        assert!(
            !is_live(5, 140),
            "the epoch ABOVE the tip is not live either"
        );

        // Terminal: `tip == last(4) == 159`, so epoch 4 is finished and the live epoch
        // is 5. A conjunction would leave no epoch live at exactly this height.
        assert_eq!(geometry.last(4), 159);
        assert_eq!(live(159), Epoch::new(5), "at last(E) the live epoch is E+1");
        assert!(!is_live(4, 159));
        assert!(is_live(5, 159));

        // Three epochs behind: the node's own finalized epoch is 1 while the verified
        // tip sits in epoch 4, so epoch 1 is not live.
        assert!(
            !is_live(1, 140),
            "an epoch the verified tip has passed is dead"
        );
        assert!(!is_live(2, 140));
        assert!(!is_live(3, 140));

        // Pre-activation heights clamp to epoch 0, so a node with no stored
        // finalization is live at 0 — the cold start.
        assert_eq!(live(0), Epoch::new(0));
    }

    /// The committee wake-up owes two groups, and the deferred one has no other
    /// retry: an epoch whose reconcile deferred on an unreadable committee is one the
    /// tip has usually already passed, so `reconcile_live` would never reach it.
    #[test]
    fn a_committee_wake_up_reconciles_every_deferred_epoch_past_the_live_gate() {
        // Two deferred epochs, neither of them the live one: both are reconciled.
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

        // The live epoch is reconciled exactly once, even when it also deferred.
        let mut deferred = BTreeSet::from([Epoch::new(9)]);
        assert_eq!(
            committee_wake_targets(&mut deferred, Some(Epoch::new(9))),
            vec![Epoch::new(9)],
            "the live epoch must not be reconciled twice by one wake-up"
        );

        // No live epoch yet: only the deferred set is owed.
        let mut deferred = BTreeSet::new();
        assert!(committee_wake_targets(&mut deferred, None).is_empty());
        let mut deferred = BTreeSet::from([Epoch::new(4)]);
        assert_eq!(
            committee_wake_targets(&mut deferred, None),
            vec![Epoch::new(4)],
            "an unfrozen geometry names no live epoch, and owes the deferred set anyway"
        );
    }

    /// A committee read that fails permanently — this node's own storage faulted
    /// under the anchor — is a deferral, not a refusal: the store re-reads on every
    /// call, so the module's next wake-up is the retry, and the epoch has to be in
    /// the deferred set for that wake-up to reach it. Driven through the same three
    /// pieces `reconcile_roles` runs: the read, the miss verdict that parks the epoch,
    /// and the wake-up's target list that drains it.
    #[test]
    fn a_permanent_read_fault_is_deferred_and_the_epoch_is_entered_on_the_next_wake() {
        use crate::committee::{Committee as _, CommitteeError};
        use fluentbase_staking_reader::ReadError;

        let epoch = Epoch::new(DETERMINISTIC_BOOTSTRAP_EPOCH + 3);
        let (snap, _) = repair_fixture(epoch);
        let record = crate::committee::CommitteeRecord {
            epoch: epoch.get(),
            members: snap
                .validators
                .iter()
                .map(|v| crate::committee::Member {
                    address: v.address,
                    peer: v.keys.peer_pubkey.clone(),
                    bls: v.keys.bls_pubkey,
                })
                .collect(),
            weights: vec![1; snap.validators.len()],
            changed: false,
            snapshot: (snap.block_number, snap.block_hash),
            participants: commonware_utils::ordered::Set::from_iter_dedup(
                snap.validators.iter().map(|v| v.keys.peer_pubkey.clone()),
            ),
            bls: epoch_committee_from_snapshot(&snap).expect("valid committee"),
        };
        let module = crate::committee::testing::SchemeCommittee::with_geometry(
            |_| None,
            move |_| Some(record.clone()),
            crate::committee::Geometry::new(0, 1),
        );
        module.fault_reads(
            epoch.get(),
            [ReadError::Backend("block_hash(..) is None".into())],
        );
        let mut deferred: BTreeSet<Epoch> = BTreeSet::new();
        let halt =
            crate::sync_metrics::SafetyHalt::new(crate::sync_metrics::SyncMetrics::default());

        // First pass: the fault. Permanent, not impossible — parked, not halted.
        let first = module
            .committee(epoch.get())
            .expect_err("the fault is handed out once");
        assert!(matches!(first, CommitteeError::Read(ReadError::Backend(_))));
        assert!(!first.is_transient() && !first.is_contract_impossible());
        assert_eq!(
            committee_miss(&first),
            CommitteeMiss::Defer,
            "a permanent-but-not-impossible read fault must be parked for the wake-up"
        );
        park_committee_miss(&mut deferred, &halt, epoch, &first);
        assert!(deferred.contains(&epoch), "the arm must park the epoch");
        assert!(!halt.is_engaged(), "a permanent read fault is not a fork");

        // The wake-up drains the parked epoch and the second read succeeds.
        let targets = committee_wake_targets(&mut deferred, None);
        assert_eq!(
            targets,
            vec![epoch],
            "the wake-up must reach the parked epoch"
        );
        let entered = module
            .committee(epoch.get())
            .expect("the second read answers the record");
        assert_eq!(entered.epoch, epoch.get());

        // And the other two classes keep their verdicts — the impossible one engages
        // the latch and parks nothing.
        let impossible = CommitteeError::Read(ReadError::PeerKey);
        assert_eq!(committee_miss(&impossible), CommitteeMiss::Halt);
        park_committee_miss(&mut deferred, &halt, epoch, &impossible);
        assert!(
            halt.is_engaged(),
            "an impossible committee must engage SafetyHalt"
        );
        assert!(
            deferred.is_empty(),
            "a halted epoch is not parked for a retry"
        );
        assert_eq!(
            committee_miss(&CommitteeError::OutOfWindow {
                epoch: 1,
                lo: 4,
                hi: 6
            }),
            CommitteeMiss::Refuse
        );
        assert_eq!(
            committee_miss(&CommitteeError::OutOfWindow {
                epoch: 9,
                lo: 4,
                hi: 6
            }),
            CommitteeMiss::Defer,
            "above the window the anchor will grow into the epoch"
        );
    }

    /// A member whose BLS key is not in the committee BiMap gets a verify-only scheme
    /// from the beacon. That scheme cannot sign, so the manager registers verify-only
    /// and spawns nothing — an engine over it would only be aborted by the next
    /// reconcile.
    #[test]
    fn a_rotated_key_soft_enters_and_spawns_no_engine() {
        use commonware_cryptography::certificate::Scheme as _;

        let epoch = Epoch::new(9);
        let (snap, _) = repair_fixture(epoch);
        let bimap = epoch_committee_from_snapshot(&snap)
            .expect("valid committee")
            .bimap;
        let verify_only = fluentbase_bls::scheme::build_verifier(
            &fluentbase_bls::fluent_namespace(1),
            bimap,
            epoch.get(),
            None,
        );
        assert!(
            verify_only.me().is_none(),
            "fixture: the scheme cannot sign"
        );

        match signer_decision(SignerVerdict::RotatedKey(verify_only)) {
            SignerDecision::SoftEnter(SoftEnterReason::RotatedKey) => {}
            other => panic!("RotatedKey must soft-enter, got {other:?}"),
        }
        assert!(matches!(
            signer_decision(SignerVerdict::Withheld(WithheldReason::NoUsableShare)),
            SignerDecision::SoftEnter(SoftEnterReason::Withheld(_))
        ));
    }

    // An overflow is returned too: the run that was dropped may have held either
    // class, so the arm applies both effects.
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
