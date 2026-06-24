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
    engine::{EpochEngine, EpochEngineConfig},
    epocher::OriginEpocher,
    order_block::OrderBlock,
    outer::SharedMux,
    scheme::soft_enter_verifier,
    slasher::Mailbox as SlasherMailbox,
    timeouts::ConsensusTimeouts,
};
use commonware_consensus::{
    marshal::{core::Mailbox as MarshalMailbox, standard::Standard},
    types::{Epoch, Epocher as _, Height},
};
use commonware_cryptography::ed25519::PublicKey;
use commonware_p2p::{Blocker, Receiver, Sender};
use commonware_runtime::{
    buffer::paged::CacheRef, spawn_cell, BufferPooler, Clock, ContextCell, Handle, Metrics,
    Spawner, Storage,
};
use commonware_utils::vec::NonEmptyVec;
use fluentbase_bls::{keys::ValidatorBlsKeypair, scheme::BeaconKey, Scheme as BlsScheme};
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use futures::future::BoxFuture;
use rand_core::CryptoRngCore;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use tokio::sync::{mpsc, Notify};
use tracing::{info, warn};

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

/// Resolves the per-epoch [`BeaconKey`] (live-DKG store + carry-forward + genesis
/// fallback). `None` ⇒ a fallback (pure-multisig) epoch. Built at the launch site
/// over the `CeremonyStore`; see `dpos.rs::launch`. This is the LOCAL DKG material
/// (full polynomial + this node's share) — required to SIGN seed partials and
/// verify individual partials. The polynomial is NOT on-chain, so this stays
/// node-local.
pub type BeaconResolver = Arc<dyn Fn(u64) -> Option<BeaconKey> + Send + Sync>;

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
    /// Live-frontier epochs whose `Verifier→Signer` spawn is parked on the
    /// `Inline::genesis(E)` precondition (the E-1 boundary block not yet in marshal
    /// storage). Re-checked on every reconcile edge (boundary / share /
    /// spawn_unblocked / vote_backup) — `reconcile_roles` is idempotent, so a parked
    /// epoch spawns the instant the boundary block lands. Never panics (defer, never
    /// `unreachable!`).
    deferred_spawns: BTreeSet<Epoch>,
    /// The most-recent boundary delivery `(epoch, snapshot)`. The non-boundary
    /// edges (share / spawn_unblocked / vote_backup) carry no snapshot, so they
    /// reconcile the CURRENT live epoch from this cache.
    latest_live: Option<(Epoch, ValidatorSetSnapshot)>,
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
    /// store with carry-forward (most-recent ceremony ≤ E) and the genesis
    /// bootstrap fallback. `None` ⇒ a fallback (pure-multisig) epoch. Called per
    /// epoch in `enter()` for the live engine + the soft-enter verifier.
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
        loop {
            // Arm the edge wakeups BEFORE the select. The producers use `notify_one`
            // (permit-storing), so even a signal that fires while no waiter is armed —
            // between a reconcile and the next select — is held as a permit and
            // consumed by the next `notified()` (no lost wakeup).
            let share_n = share_notify.notified();
            let spawn_n = spawn_unblocked.notified();
            tokio::pin!(share_n, spawn_n);
            tokio::select! {
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
                // scheme is frozen at construction, so this is a respawn).
                _ = &mut share_n => {
                    self.reconcile_live(muxes.as_ref()).await;
                }
                // Edge: the executor recorded a finalized block — the MID-EPOCH
                // promotion trigger. A caught-up member promotes the instant its
                // `Inline::genesis` precondition is met; gated on a pending parked
                // spawn so the per-block fire is a no-op in steady state.
                _ = &mut spawn_n => {
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
        // fork. Verify-only lets the marshal verify this epoch's certs (the verifier
        // is MULTISIG-ONLY — `verify_certificate` ignores the seed; no group key).
        if !self.is_live_epoch(epoch) {
            self.soft_enter(epoch, &snap);
            self.deferred_spawns.remove(&epoch);
            info!(?epoch, "epoch soft-entered (scheme only, catch-up)");
            return;
        }

        // At the live frontier (checked above): role = f(member). "Caught up" is not
        // a separate input: a member only spawns a participating engine once the
        // share-gate AND `boundary_block_present` both hold below, which together
        // mean the local executor has derived up to E-1's boundary.
        let is_member = self.cfg.signer_keypair.is_some()
            && snap
                .validators
                .iter()
                .any(|v| v.keys.peer_pubkey == self.cfg.me);

        // Already a running signer for the live epoch — keep it. The committee is
        // frozen per epoch, so membership cannot change mid-epoch; a frontier move
        // aborts this engine via `abort_below` on the next boundary.
        if self.active_epochs.contains_key(&epoch) {
            return;
        }

        // Role is a pure function of membership (the caller is already at the
        // live frontier, so liveness is not a separate input); see [`Role`].
        let assigned_role = if is_member { Role::Signer } else { Role::Verifier };
        match assigned_role {
            // Not a member (rotated out). Register verify-only so the marshal
            // verifies this epoch's certs; no participating engine.
            Role::Verifier => {
                if self.cfg.signer_keypair.is_some() && !is_member {
                    self.cfg.beacon_metrics.engine_demoted_rotated_out.inc();
                }
                self.soft_enter(epoch, &snap);
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
                let beacon = (self.cfg.beacon_resolver)(epoch.get());
                if beacon_active && beacon.is_none() {
                    self.cfg.beacon_metrics.engine_demoted_no_polynomial.inc();
                    self.soft_enter(epoch, &snap);
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
                    self.soft_enter(epoch, &snap);
                    info!(
                        ?epoch,
                        "signer spawn deferred — E-1 boundary block not yet in marshal"
                    );
                    return;
                }

                if self.spawn_engine(epoch, snap, beacon, muxes).await {
                    self.roles.insert(epoch, Role::Signer);
                    self.deferred_spawns.remove(&epoch);
                    // Stable greppable token for the production-path smoke
                    // (`case-production-path.sh`): the in-process Verifier→Signer
                    // promotion — a joiner that catches up + holds its DKG share
                    // re-promotes here without a process restart.
                    info!(?epoch, "promoted to Signer in-process: per-epoch BFT engine started");
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
    fn soft_enter(&mut self, epoch: Epoch, snap: &ValidatorSetSnapshot) {
        if self.active_epochs.contains_key(&epoch)
            || self.roles.get(&epoch) == Some(&Role::Signer)
        {
            return;
        }
        if let Some(scheme) = soft_enter_verifier(snap, self.cfg.chain_id) {
            (self.cfg.register_scheme)(epoch, scheme);
        }
        self.roles.insert(epoch, Role::Verifier);
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
) {
    if their_epoch <= *highest_entered_epoch {
        return;
    }
    let entered = highest_entered_epoch.get();
    let span_from = Epoch::new(entered + 1);
    let span_top = highest_observed_epoch.min(Epoch::new(entered + CATCHUP_SPAN_CAP));
    let registered_to = if span_top >= span_from {
        soft_enter_span(span_from, span_top).await
    } else {
        *highest_entered_epoch
    };
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
}
