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
    application::{
        attested_group_key, insert_group_key, pk_prefix, ExecutedChain, FluentApp, GroupKeys,
        KeySource, OrderingAssembler,
    },
    beacon::{
        outcome::{group_public_key, parse_outcome},
        seed::GroupPublic,
    },
    engine::{EpochEngine, EpochEngineConfig},
    epocher::OriginEpocher,
    order_block::OrderBlock,
    outer::{SharedMux, SCHEME_RETENTION_EPOCHS},
    scheme::soft_enter_verifier,
    slasher::Mailbox as SlasherMailbox,
    timeouts::ConsensusTimeouts,
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
use fluentbase_bls::{
    beacon as beacon_bls, keys::ValidatorBlsKeypair, scheme::BeaconKey, Scheme as BlsScheme,
};
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use futures::future::BoxFuture;
use rand_core::CryptoRngCore;
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::{mpsc, Notify};
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

/// Outcome of a per-epoch beacon-key resolve (see [`BeaconResolver`]).
pub enum BeaconResolve {
    /// The epoch's key + share — the mint at the chain's `dkgQual` key epoch
    /// (`beacon::carry::select_carry_scheme`), an exact-epoch ceremony or a
    /// carried one no re-mint superseded.
    Key(BeaconKey),
    /// No usable local material for the epoch: nothing stored at or below it,
    /// a chain-declined or superseded mint, or an undecided (unreadable)
    /// `dkgQual` bit. ⇒ a fallback (pure-multisig) epoch / share-gate demote;
    /// re-resolved on the next edge.
    Absent,
}

/// Resolves the per-epoch [`BeaconKey`] (live-DKG store + `dkgQual`-bit-gated
/// carry-forward). Built at the launch site over the `CeremonyStore`; see
/// `dpos.rs::beacon_share_resolver`. This is the LOCAL DKG material (full
/// polynomial + this node's share) — required to SIGN seed partials and
/// verify individual partials. The polynomial is NOT on-chain, so this stays
/// node-local.
pub type BeaconResolver = Arc<dyn Fn(u64) -> BeaconResolve + Send + Sync>;

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
/// than `outer.rs::SCHEME_RETENTION_EPOCHS` (= 8): the marshal verifies the
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
    /// Per-epoch beacon resolver: returns the [`BeaconKey`] (`PK_epoch` sharing +
    /// this node's share + namespace) for `epoch`, sourced from the live-DKG
    /// store under the frozen on-chain `dkgQual`-bit carry arbitration
    /// (`beacon::carry`). [`BeaconResolve::Absent`] ⇒ a fallback (pure-multisig)
    /// epoch. Called per epoch in `reconcile_roles` for the live engine + the
    /// soft-enter verifier.
    pub beacon_resolver: BeaconResolver,
    /// Edge-trigger the `DkgActor` fires when a share lands in the live-DKG store.
    /// `enter()` arms `notified()` and re-checks `beacon_resolver`, so a signer that
    /// reaches the boundary before its share is memoized wakes the instant it lands
    /// instead of polling. Same `Arc` the actor holds (via `SharedBeaconPlane`).
    pub beacon_share_notify: Arc<Notify>,
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
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`].
    pub slasher_mailbox: SlasherMailbox,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`]: the
    /// notarization arm of the simplex reporter, forwarding `SpecNotarized`
    /// commands to the executor for speculative execution.
    pub spec_exec_mailbox: crate::spec_exec::Mailbox,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`]: the shared
    /// `round → recovered seed` map for the Stage-2 beacon certify gate. Written
    /// by `spec_exec_mailbox`, read by each per-epoch [`crate::beacon::certify::BeaconCertify`].
    pub seed_store: crate::beacon::certify::SeedStore,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`]: the SAME
    /// `Arc` handed to [`FluentApp`] (an `Arc` clone, not a second map — W1
    /// writes here and the vote path reads through `FluentApp`). The manager
    /// cannot reach the map through `cfg.app` (no accessor; `app` is moved),
    /// so it holds its own clone for writer W1 (insert `(E, PK_E)` BEFORE
    /// `spawn_engine`) and writer W3 (best-effort `E−1` backfill).
    pub group_keys: GroupKeys,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`]: beacon counters,
    /// threaded into each per-epoch engine for the demote counters.
    pub beacon_metrics: crate::beacon::metrics::BeaconMetrics,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`].
    pub page_cache: CacheRef,
    /// Callback into [`crate::outer::EpochSchemeProvider`] so marshal can verify
    /// cross-epoch finalization certificates (trailing-window pruned; see SCHEME_RETENTION_EPOCHS).
    pub register_scheme: Arc<dyn Fn(Epoch, BlsScheme) + Send + Sync>,
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

    /// Start the manager. The 3 simplex broker handles (vote/cert/resolver) are
    /// owned by the always-on plane (node crate); this manager CLONES them per
    /// promotion to register per-epoch sub-channels and drops them on exit (the
    /// `SubReceiver`s auto-deregister, freeing the slots for the next promotion). The
    /// vote Muxer's backup receiver is the plane's re-settable forwarder, fresh per
    /// promotion.
    pub fn start<HS, HR>(
        mut self,
        muxes: Option<Muxes<HS, HR>>,
        vote_backup: mpsc::Receiver<(u64, (PublicKey, commonware_runtime::IoBuf))>,
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
        mut vote_backup: mpsc::Receiver<(u64, (PublicKey, commonware_runtime::IoBuf))>,
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
        let share_notify = self.cfg.beacon_share_notify.clone();
        let spawn_unblocked = self.cfg.spawn_unblocked.clone();
        let safety_halt = self.cfg.safety_halt.clone();
        loop {
            // Arm the edge wakeups BEFORE the select. The producers use `notify_one`
            // (permit-storing), so even a signal that fires while no waiter is armed —
            // between a reconcile and the next select — is held as a permit and
            // consumed by the next `notified()` (no lost wakeup).
            let share_n = share_notify.notified();
            let spawn_n = spawn_unblocked.notified();
            let halt_n = safety_halt.engaged_edge();
            tokio::pin!(share_n, spawn_n, halt_n);
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
                            self.handle_msg_for_unregistered_epoch(Epoch::new(their_epoch), from).await;
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
                // Edge: a DKG share landed — re-run reconcile so a member parked by
                // the share-gate spawns now that its share is present (the running
                // scheme is frozen at construction, so this is a respawn). Also
                // clears the catch-up no-progress memo (a landed share can make a
                // previously-unresolvable span read succeed — bug 15).
                _ = &mut share_n => {
                    self.catchup_no_progress = None;
                    self.reconcile_live(muxes.as_ref()).await;
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
        // W3 (P1) — best-effort group-key backfill for the PREVIOUS epoch, off
        // the vote path: covers a node promoted mid-`E` that never ran `E−1`'s
        // engine (no W1 entry for `E−1`). Insert ONLY on success — a failure is
        // never cached, so this re-attempts on every reconcile edge (boundary /
        // share / spawn_unblocked / vote_backup). A warm-up, not the boundary
        // repair path: the first block of `E+1` is verified ~1 s after the
        // spawn, so the repair that fires there is the per-vote lazy resolve.
        if let Some(prev) = epoch.get().checked_sub(1) {
            let missing = self
                .cfg
                .group_keys
                .read()
                .map(|m| !m.contains_key(&prev))
                .unwrap_or(false);
            if missing {
                // Best-effort: an undecided resolve is NOT backfilled — the
                // next edge re-attempts.
                if let BeaconResolve::Key((sharing, _, _)) = (self.cfg.beacon_resolver)(prev) {
                    let pk = *sharing.public();
                    debug!(
                        epoch = prev,
                        group_public = %pk_prefix(&pk),
                        "W3: backfilling previous-epoch group key from own DKG material"
                    );
                    insert_group_key(&self.cfg.group_keys, prev, pk, KeySource::LocalDkg);
                }
            }
        }

        // Boundary bookkeeping (idempotent; monotone). Committee size is keyed on
        // the HIGHEST-ENTERED epoch (follows validator-set growth and shrink) — it
        // feeds the f+1 corroboration threshold. Reaching an epoch RESOLVES it:
        // free pending corroboration pins ≤ it so a healthy node's boundary-race
        // pins don't permanently mute honest senders.
        self.highest_entered_epoch = self.highest_entered_epoch.max(epoch);
        if epoch == self.highest_entered_epoch {
            self.committee_size = snap.validators.len();
        }
        // Prune the cross-epoch group-key map to the same trailing scheme-retention
        // window: every reader is an exact `get(&epoch)` for the epoch under
        // reconcile (near the entered frontier — W1/W3/attested/ladder), oldest
        // bounded by SCHEME_RETENTION_EPOCHS, so entries older than that can never
        // be read again. Without this the map grows unbounded across a months-long
        // process. The manager holds the `Arc`; take the write lock briefly.
        let highest = self.highest_entered_epoch.get();
        if let Ok(mut m) = self.cfg.group_keys.write() {
            m.retain(|e, _| *e + SCHEME_RETENTION_EPOCHS as u64 >= highest);
        }
        prune_resolved(
            &mut self.observed_reporters,
            &mut self.sender_pins,
            self.highest_entered_epoch,
        );

        // Exit-at-transition: abort every engine strictly below the live epoch
        // (folded `prune_old`; `e < cutoff` only, so a stale/replayed boundary for
        // an OLD epoch can never abort a newer engine).
        self.abort_below(epoch);

        // Below the live frontier → soft-enter (verify-only scheme, NO engine): a
        // Simplex engine for a stale epoch has no live peers and would drive a dead
        // fork. Verify-only lets the marshal verify this epoch's certs (with a
        // `PK_epoch` seed pin when the boundary block is resolvable — see
        // `soft_enter` / `resolve_beacon_key`; else vote-only).
        if !self.is_live_epoch(epoch) {
            self.soft_enter(epoch, &snap).await;
            self.deferred_spawns.remove(&epoch);
            info!(?epoch, "epoch soft-entered (scheme only, catch-up)");
            return;
        }

        // At the live frontier (checked above): role = f(member). "Caught up" is not
        // a separate input: a member only spawns a participating engine once the
        // share-gate AND `boundary_block_present` both hold below, which together
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
                return;
            }
            self.active_epochs.remove(&epoch);
            self.cfg.beacon_metrics.engine_respawned.inc();
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
                    self.cfg.beacon_metrics.engine_demoted_rotated_out.inc();
                }
                self.soft_enter(epoch, &snap).await;
            }
            Role::Signer => {
                // Share-gate: a beacon-active member with no usable DKG share must
                // NOT run a participating engine — a `beacon: None` Simplex member
                // rejects honest peers' seeded votes (`combined_scheme::verify_attestation`)
                // and the batcher blocks them → wedge. Resolve the local share
                // NON-BLOCKINGLY (blocking would stall the whole reconcile loop, and
                // re-block on every share edge for a genuinely shareless member);
                // on absence, register verify-only + stay off the consensus plane
                // (the surviving NoBeaconPolynomial effect). The `beacon_share_notify`
                // edge re-runs reconcile and promotes the instant the share lands.
                let beacon_active =
                    epoch.get() >= crate::beacon::actor::DETERMINISTIC_BOOTSTRAP_EPOCH;
                let beacon = match (self.cfg.beacon_resolver)(epoch.get()) {
                    BeaconResolve::Key(key) => Some(key),
                    BeaconResolve::Absent => None,
                };
                if beacon_active && beacon.is_none() {
                    self.cfg.beacon_metrics.engine_demoted_no_polynomial.inc();
                    self.soft_enter(epoch, &snap).await;
                    info!(
                        ?epoch,
                        "committee member without a usable DKG share — verify-only (share-gate)"
                    );
                    return;
                }

                // `Inline::genesis(E)` precondition: the E-1 boundary block must be
                // in marshal storage before the per-epoch engine starts (else the
                // engine hits `unreachable!`). On a mid-epoch promotion the marshal
                // may still be backfilling it — DEFER, never panic; the executor's
                // `spawn_unblocked` edge (or the next boundary) re-pokes. Register
                // verify-only meanwhile so the marshal verifies this epoch's certs.
                if !self.boundary_block_present(epoch).await {
                    self.deferred_spawns.insert(epoch);
                    self.soft_enter(epoch, &snap).await;
                    self.cfg.beacon_metrics.engine_spawn_deferred.inc();
                    info!(
                        ?epoch,
                        boundary = ?epoch
                            .get()
                            .checked_sub(1)
                            .and_then(|prev| self.cfg.epocher.last(Epoch::new(prev)))
                            .map(|h| h.get()),
                        "signer spawn deferred — E-1 boundary block not yet in marshal; \
                         verify-only until it lands"
                    );
                    return;
                }

                // Promote-gate VALUE check (defense-in-depth; f297cc36 extended
                // from key PRESENCE to key VALUE): when the network already
                // attests a `PK_epoch` for E — the W4 observed-outcome map
                // entry (agreed chain data) or the finalized boundary block's
                // own `beacon_outcome` — a resolver key that DIFFERS is a
                // diverged local reconstruction, whatever produced it. Never
                // sign, W1-publish, or witness-check under it: demote to
                // verify-only; the recompute-heal stores the correct
                // exact-epoch `(PK_E, share)` and its `share_notify` edge
                // re-runs this reconcile, which then promotes with the
                // matching key.
                if let Some((sharing, _, _)) = beacon.as_ref() {
                    let pk = *sharing.public();
                    let observed = match attested_group_key(&self.cfg.group_keys, epoch.get()) {
                        Some(net) => Some(net),
                        None => self.boundary_outcome_key(epoch).await,
                    };
                    if let Some(net) = observed {
                        if net != pk {
                            self.cfg.beacon_metrics.engine_demoted_key_divergence.inc();
                            warn!(
                                ?epoch,
                                resolved = %pk_prefix(&pk),
                                network = %pk_prefix(&net),
                                "resolved PK_epoch DIVERGES from the network-attested \
                                 key — verify-only (promote value-gate)"
                            );
                            self.soft_enter(epoch, &snap).await;
                            return;
                        }
                    }
                }

                // Promote-gate SHARE check. `CombinedScheme::new` asserts only that
                // the share's INDEX equals this node's participant index — never
                // that its VALUE lies on the sharing. While blocks flow, a bad share
                // is exposed on the notarize path; in a sustained stall there are no
                // proposals, so it is not, and since every Nullify now carries a seed
                // partial and `t == quorum`, one such member on the plane makes the
                // nullify quorum unreachable exactly when nullification is the escape
                // hatch. The probe is purely local (share vs its own sharing), so it
                // also covers the cold-ceremony window where the VALUE gate above is
                // a no-op for want of a network-attested key.
                if let Some((sharing, Some(share), namespace)) = beacon.as_ref() {
                    let probe = Round::new(epoch, View::new(1));
                    let partial = beacon_bls::sign_seed_partial(share, namespace, probe);
                    if !beacon_bls::verify_seed_partial(sharing, namespace, probe, &partial) {
                        self.cfg.beacon_metrics.engine_demoted_bad_share.inc();
                        warn!(
                            ?epoch,
                            "resolved DKG share does not verify against its own sharing — \
                             verify-only (promote share-gate)"
                        );
                        self.soft_enter(epoch, &snap).await;
                        return;
                    }
                }

                // W1 (P1) — publish `PK_epoch` into the cross-epoch group-key
                // map BEFORE the engine exists (never inside `spawn_engine`):
                // the engine cannot cast a vote before it is spawned, so every
                // node that votes on epoch E finds `group_keys[E]` populated at
                // its first vote — the same-epoch quorum argument (`b = 0`).
                // Infallible here: `beacon` is already resolved and the
                // share-gate above demoted the shareless case to verify-only.
                // One epoch later this same entry IS the boundary warm (W2):
                // `E+1`'s gate needs `PK_E` and reads the map, no I/O.
                if let Some((sharing, _, _)) = beacon.as_ref() {
                    let pk = *sharing.public();
                    // The value fingerprint is the point: a restarted signer whose
                    // carried-forward key diverged from the network's is only
                    // diagnosable by grepping this line across nodes (soak
                    // 2026-07-14 v5@epoch77 reject{bad_signature}).
                    info!(
                        ?epoch,
                        group_public = %pk_prefix(&pk),
                        "W1: publishing own epoch group key"
                    );
                    insert_group_key(&self.cfg.group_keys, epoch.get(), pk, KeySource::LocalDkg);
                }
                if self.spawn_engine(epoch, snap, beacon, muxes).await {
                    self.roles.insert(epoch, Role::Signer);
                    self.deferred_spawns.remove(&epoch);
                    // Stable greppable token for the production-path smoke
                    // (`case-production-path.sh`): the in-process Verifier→Signer
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
        // Ladder first (§5 e): the shared group-key map (W1/W4), then this
        // node's own DKG material — both resolve on a stable committee older
        // than the 8-hop outcome walk below, which is exhausted past 8 stable
        // epochs even with every block on disk (the pre-existing
        // `verify_certificate` weakness: pin `None` ⇒ vote-only cert
        // acceptance). The walk stays as the last rung.
        let cert_seed_pin =
            match group_key_ladder(&self.cfg.group_keys, &self.cfg.beacon_resolver, epoch.get()) {
                Some(pk) => Some(pk),
                None => self.resolve_beacon_key(epoch).await,
            };
        debug!(
            ?epoch,
            pinned = cert_seed_pin.is_some(),
            "soft-enter: registering verify-only scheme"
        );
        if let Some(scheme) = soft_enter_verifier(snap, self.cfg.chain_id, cert_seed_pin) {
            (self.cfg.register_scheme)(epoch, scheme);
        }
        self.roles.insert(epoch, Role::Verifier);
    }

    /// Resolve the epoch beacon group key `PK_epoch` (bug 2 seed pin) for a
    /// soft-entered verifier by walking epoch first-blocks backward from `epoch`
    /// through the marshal's stored blocks until one carries a `beacon_outcome`
    /// (the change-epoch DKG rotation whose key carries forward). The walk may
    /// continue past a PRESENT boundary block without an outcome (a stable epoch
    /// provably carried the key forward) but MUST STOP at an ABSENT one: absent is
    /// UNKNOWN, not "no rotation" — `epoch` itself may be the change boundary whose
    /// missing block holds the ROTATED key, and walking past it pins the STALE
    /// pre-rotation key, which then makes the marshal's `verify_delivered` silently
    /// reject (`verified = false`, no log) every valid cert of `epoch`. A validator
    /// demoted AT a change boundary froze exactly this way (its demotion-boundary
    /// block was never stored locally). Absent ⇒ `None` ⇒ vote-only admission (the
    /// accepted residual window). Bounded to the scheme-retention window (see
    /// `outer.rs::SCHEME_RETENTION_EPOCHS = 8`), so a non-change stretch does not
    /// walk unboundedly.
    async fn resolve_beacon_key(&mut self, epoch: Epoch) -> Option<GroupPublic> {
        let mut e = epoch;
        for _ in 0..8 {
            let first = self.cfg.epocher.first(e)?;
            let Some(block) = self.cfg.marshal_mailbox.get_block(first).await else {
                debug!(
                    ?epoch,
                    walk = ?e,
                    "beacon-key walk hit an ABSENT boundary block — vote-only admission \
                     (walking past it could pin a stale pre-rotation key)"
                );
                return None;
            };
            if let Some(bytes) = block.beacon_outcome.as_ref() {
                if let Ok(outcome) = parse_outcome(bytes) {
                    return Some(*group_public_key(&outcome));
                }
                // Present-but-unparseable outcome: never guess a key from further
                // back — degrade to vote-only.
                warn!(
                    ?epoch,
                    walk = ?e,
                    "beacon outcome present but unparseable; vote-only admission"
                );
                return None;
            }
            e = Epoch::new(e.get().checked_sub(1)?);
        }
        None
    }

    /// The chain's OWN attested `PK_epoch` from the finalized first block of
    /// `epoch`, when that block is already in marshal storage and carries a
    /// `beacon_outcome` (i.e. `epoch` is a change epoch that finalized without
    /// us). `None` ⇒ no network observation available (a stable epoch, or the
    /// boundary block not yet produced/stored) — NOT "no divergence".
    async fn boundary_outcome_key(&mut self, epoch: Epoch) -> Option<GroupPublic> {
        let first = self.cfg.epocher.first(epoch)?;
        let block = self.cfg.marshal_mailbox.get_block(first).await?;
        let outcome = parse_outcome(block.beacon_outcome.as_ref()?).ok()?;
        Some(*group_public_key(&outcome))
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

    /// True when the `Inline::genesis(E)` precondition holds — the E-1 terminal
    /// (boundary) block is present in marshal `finalized_blocks` storage. `epoch 0`
    /// has no predecessor (genesis needs nothing). This is the exact lookup
    /// `Inline::genesis` itself performs, so the guard is precise, not heuristic.
    async fn boundary_block_present(&mut self, epoch: Epoch) -> bool {
        let Some(prev) = epoch.get().checked_sub(1).map(Epoch::new) else {
            return true; // epoch 0 — genesis needs no predecessor block
        };
        let Some(last) = self.cfg.epocher.last(prev) else {
            return true;
        };
        self.cfg.marshal_mailbox.get_block(last).await.is_some()
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
    /// invalid committee snapshot or a muxer-register failure — the caller leaves
    /// the epoch un-promoted to retry on the next edge, rather than panicking.
    async fn spawn_engine<HS, HR>(
        &mut self,
        epoch: Epoch,
        snap: ValidatorSetSnapshot,
        beacon: Option<BeaconKey>,
        muxes: Option<&Muxes<HS, HR>>,
    ) -> bool
    where
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
    {
        // W1 ordering tripwire (P1-b): by the time the engine object can come
        // into existence, this beacon-active signer's `PK_epoch` MUST already
        // be in the shared group-key map — the insert happens-before the
        // engine, hence before any vote. A future refactor that moves the W1
        // insert after (or inside) the spawn trips this on every deterministic
        // run.
        debug_assert!(
            beacon.is_none()
                || self
                    .cfg
                    .group_keys
                    .read()
                    .is_ok_and(|m| m.contains_key(&epoch.get())),
            "W1 ordering violated: PK_epoch not in group_keys before spawn_engine({epoch:?})"
        );
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
                epocher: self.cfg.epocher.clone(),
                chain_id: self.cfg.chain_id,
                signer_keypair: self.cfg.signer_keypair.clone(),
                app: self.cfg.app.clone(),
                timeouts: self.cfg.timeouts,
                mailbox_size: self.cfg.mailbox_size,
                register_scheme: self.cfg.register_scheme.clone(),
                beacon,
                seed_store: self.cfg.seed_store.clone(),
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

/// Synchronous rungs of the `PK_epoch` ladder for the soft-enter cert-seed pin
/// (§5 e, ladder steps 2 then 1): an exact hit in the shared group-key map
/// (W1/W4-filled), else this node's own DKG material via the
/// `dkgQual`-bit-gated carry-forward `beacon_resolver` (key only — the share
/// is discarded). `None` ⇒ the caller falls through to the 8-hop marshal
/// outcome walk (`resolve_beacon_key`), which remains the last rung. Extracted
/// as a free function so the "a stable committee > 8 epochs old still pins"
/// repair is unit-testable without standing up the full generic `Actor`.
fn group_key_ladder(
    group_keys: &GroupKeys,
    beacon_resolver: &BeaconResolver,
    epoch: u64,
) -> Option<GroupPublic> {
    if let Some(pk) = group_keys
        .read()
        .ok()
        .and_then(|m| m.get(&epoch).map(|&(pk, _)| pk))
    {
        return Some(pk);
    }
    match beacon_resolver(epoch) {
        BeaconResolve::Key((sharing, _, _)) => Some(*sharing.public()),
        BeaconResolve::Absent => None,
    }
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
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer};
    use commonware_math::algebra::Random as _;
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

    /// §5 e (the `verify_certificate`-pin bonus fix): the soft-enter cert-seed
    /// pin consults the group-key ladder BEFORE the 8-hop marshal outcome walk.
    /// Models a stable committee > 8 epochs past its change epoch — the walk
    /// finds no `beacon_outcome` within 8 hops and returns `None`, so under the
    /// pre-fix spec the pin was `None` ⇒ silent vote-only cert acceptance.
    /// Ladder step 1 (own DKG material, key only) and step 2 (the shared map)
    /// must each pin on their own.
    #[test]
    fn soft_enter_pin_resolves_from_the_ladder_when_the_outcome_walk_is_exhausted() {
        use commonware_cryptography::bls12381::{dkg::deal_anonymous, primitives::variant::MinSig};
        use commonware_utils::{test_rng, N3f1, NZU32};
        use std::sync::{Arc, RwLock};

        let mut rng = test_rng();
        let (sharing, _shares) =
            deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(4));
        let pk = *sharing.public();

        // Step 1: empty map, own material via the carry-forward resolver.
        let empty: GroupKeys = Arc::new(RwLock::new(BTreeMap::new()));
        let confirmed_sharing = sharing.clone();
        let own: BeaconResolver =
            Arc::new(move |_| BeaconResolve::Key((confirmed_sharing.clone(), None, Vec::new())));
        assert_eq!(group_key_ladder(&empty, &own, 12), Some(pk));

        // Step 2: a map entry (W1/W4-filled) pins WITHOUT consulting the
        // resolver at all.
        let map: GroupKeys = Arc::new(RwLock::new(BTreeMap::from([(
            12u64,
            (pk, KeySource::LocalDkg),
        )])));
        let none_resolver: BeaconResolver =
            Arc::new(|_| panic!("a map hit must not consult the resolver"));
        assert_eq!(group_key_ladder(&map, &none_resolver, 12), Some(pk));

        // Neither rung ⇒ None: the caller falls through to the outcome walk
        // (the pre-existing last rung), unchanged.
        let unresolvable: BeaconResolver = Arc::new(|_| BeaconResolve::Absent);
        assert_eq!(group_key_ladder(&empty, &unresolvable, 12), None);
    }

    // A single peer (even naming u64::MAX) must NOT advance the live frontier —
    // the P2-11 permanent-soft-enter halt. n = 4 ⇒ f = 1 ⇒ threshold f+1 = 2.
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
}
