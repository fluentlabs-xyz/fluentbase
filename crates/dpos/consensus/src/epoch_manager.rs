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
use commonware_p2p::{
    Blocker, Receiver, Sender,
};
use commonware_runtime::{
    buffer::paged::CacheRef, spawn_cell, BufferPooler, Clock, ContextCell, Handle, Metrics,
    Spawner, Storage,
};
use commonware_utils::vec::NonEmptyVec;
use fluentbase_bls::{keys::ValidatorBlsKeypair, scheme::BeaconKey, Scheme as BlsScheme};
use futures::future::BoxFuture;
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use rand_core::CryptoRngCore;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};
use tokio::sync::mpsc;
use tracing::{info, warn};

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

/// Max distinct future epochs one peer may pin on the vote backup channel before
/// its live frontier is corroborated. Two covers the legitimate case (a peer is
/// at most ~1 boundary ahead of what it gossips) with slack; together with the
/// committee bound this caps the corroboration map at `n · 2` epochs and stops a
/// Byzantine minority from crowding out the honest frontier.
const PINS_PER_SENDER: usize = 2;

/// Bound on the edge-triggered beacon-readiness wait in `enter` (see
/// [`wait_for_beacon`]). On expiry a beacon-active signer proceeds pure-multisig
/// (the local DKG share never landed ⇒ option-A stall). Caps how long `enter`
/// may block the manager loop; the common case wakes on the share-landed edge
/// far sooner, so this is the no-quorum backstop, not the steady-state latency.
const BEACON_READY_TIMEOUT: Duration = Duration::from_secs(20);

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
    /// `0` until the first `enter`, during which backup corroboration is disabled
    /// (the cold-start epoch full-enters from the verified boundary trigger).
    committee_size: usize,
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
    /// Rotation-out signals to the unified supervisor (`None` = legacy).
    pub mode_events: Option<tokio::sync::mpsc::UnboundedSender<crate::dpos::ModeEvent>>,
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
    pub beacon_share_notify: std::sync::Arc<tokio::sync::Notify>,
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
    pub byzantine: Option<crate::application::ByzantineMode>,
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
        vote_mux: SharedMux<HS, HR>,
        cert_mux: SharedMux<HS, HR>,
        res_mux: SharedMux<HS, HR>,
        vote_backup: mpsc::Receiver<(u64, (PublicKey, commonware_runtime::IoBuf))>,
    ) -> Handle<()>
    where
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
    {
        spawn_cell!(
            self.context,
            self.run(vote_mux, cert_mux, res_mux, vote_backup).await
        )
    }

    async fn run<HS, HR>(
        mut self,
        vote_mux: SharedMux<HS, HR>,
        cert_mux: SharedMux<HS, HR>,
        res_mux: SharedMux<HS, HR>,
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
        loop {
            tokio::select! {
                recv = self.boundary_rx.recv() => {
                    match recv {
                        Some((epoch, snap)) => {
                            self.enter(epoch, snap, &vote_mux, &cert_mux, &res_mux).await;
                            self.prune_old(epoch);
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
                            self.handle_msg_for_unregistered_epoch(Epoch::new(their_epoch), from).await;
                        }
                        None => {
                            info!("vote backup channel closed, epoch_manager exiting");
                            break;
                        }
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

    async fn enter<HS, HR>(
        &mut self,
        epoch: Epoch,
        snap: ValidatorSetSnapshot,
        vote_mux: &SharedMux<HS, HR>,
        cert_mux: &SharedMux<HS, HR>,
        res_mux: &SharedMux<HS, HR>,
    ) where
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
    {
        if self.active_epochs.contains_key(&epoch) {
            warn!(?epoch, "epoch already active; skipping");
            return;
        }

        self.highest_entered_epoch = self.highest_entered_epoch.max(epoch);
        // Track the committee size of the HIGHEST-ENTERED epoch (monotonic in
        // epoch, not in size) — feeds the f+1 Byzantine threshold. Keying on the
        // newest entered epoch follows both validator-set growth and shrink
        // (a >3× shrink under monotonic-max would freeze the frontier, since the
        // tracked peer set == the current smaller committee can't reach the stale
        // higher threshold). A stale soft-enter has `epoch < highest_entered`, so
        // it cannot lower the threshold — preserving the R4-2 grow-attack guard.
        if epoch == self.highest_entered_epoch {
            self.committee_size = snap.validators.len();
        }
        // Entering an epoch RESOLVES it: prune any pending corroboration state for
        // it and below, so a healthy node's boundary-race pins (peers that voted
        // for E+1 on the backup channel just before the node registered E+1) are
        // freed instead of permanently consuming each sender's pin quota and
        // muting it — which would otherwise freeze the live frontier. `enter` is
        // gated by the verified on-chain boundary trigger, so this free is not
        // attacker-reachable; far-future decoy pins (epoch > entered) stay pinned.
        prune_resolved(
            &mut self.observed_reporters,
            &mut self.sender_pins,
            self.highest_entered_epoch,
        );

        // Soft-enter for catch-up epochs: register the verifier so the marshal can
        // verify this epoch's finalization certs, but do NOT spawn a participating
        // engine. No peers vote in a historical epoch, and a live engine there would
        // drive the executor on a dead fork. The live epoch (and the retention window
        // below it) full-enters below.
        if !self.is_live_epoch(epoch) {
            // Soft-enter is verify-only (no engine, no signing). The verifier is
            // MULTISIG-ONLY (`verify_certificate` ignores the seed now that the
            // PK_E layer is gone) — no group key to read, nothing to lag, no
            // defer. The verifier construction is shared with the bulk catch-up
            // span path (see [`soft_enter_verifier`] + `Config::soft_enter_span`).
            if let Some(scheme) = soft_enter_verifier(&snap, self.cfg.chain_id) {
                (self.cfg.register_scheme)(epoch, scheme);
            }
            info!(?epoch, "epoch soft-entered (scheme only, catch-up)");
            return;
        }

        // Self-heal the beacon-readiness boundary race BEFORE constructing the
        // engine: `enter` fires off the consensus ordering clock the instant the
        // boundary block finalizes, but this node's own DKG share lands in the
        // shared `ceremony_store` off the gossip plane (the quorum-th `Reveal`),
        // which can arrive just after `enter`. A signing member waits — edge-driven
        // off the actor's `share_notify`, bounded, deadlock-free — for its share
        // rather than baking a permanent verify-only demote for the whole epoch.
        // See `wait_for_beacon`.
        let beacon = {
            let beacon_active =
                epoch.get() >= crate::beacon::actor::DETERMINISTIC_BOOTSTRAP_EPOCH;
            let resolver = self.cfg.beacon_resolver.clone();
            let epoch_n = epoch.get();
            let wait_ctx = self.context.with_label("beacon");
            wait_for_beacon(
                &wait_ctx,
                move || resolver(epoch_n),
                &self.cfg.beacon_share_notify,
                beacon_active,
                self.cfg.signer_keypair.is_some(),
                BEACON_READY_TIMEOUT,
            )
            .await
        };

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
                mode_events: self.cfg.mode_events.clone(),
                app: self.cfg.app.clone(),
                timeouts: self.cfg.timeouts,
                mailbox_size: self.cfg.mailbox_size,
                register_scheme: self.cfg.register_scheme.clone(),
                beacon,
                seed_store: self.cfg.seed_store.clone(),
                beacon_metrics: self.cfg.beacon_metrics.clone(),
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
                // Invalid on-chain committee (e.g. non-unique participants). Skip
                // entering this epoch rather than panic — a panic here collapses
                // the entire DPoS stack via the outer supervisor. The boundary
                // trigger re-fires on later finalized blocks if the committee is
                // corrected; the prior epoch's engine keeps producing meanwhile.
                warn!(?epoch, %e, "skipping epoch enter — invalid committee snapshot");
                return;
            }
        };

        // Register the three sub-channels against the plane-owned Muxers (locking
        // each transiently — registration is boundary-rate). `register` errors are
        // AlreadyRegistered (unreachable — guarded by `active_epochs` above) or
        // Closed (muxer task gone, i.e. teardown). On any error, skip the enter
        // rather than `.expect()`-panic (which would cascade to the whole stack).
        // Partial registrations auto-deregister when their `SubReceiver` drops.
        let vote_sub = match vote_mux.lock().await.register(epoch.get()).await {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    ?epoch,
                    ?e,
                    "skipping epoch enter — vote muxer register failed"
                );
                return;
            }
        };
        let cert_sub = match cert_mux.lock().await.register(epoch.get()).await {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    ?epoch,
                    ?e,
                    "skipping epoch enter — cert muxer register failed"
                );
                return;
            }
        };
        let res_sub = match res_mux.lock().await.register(epoch.get()).await {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    ?epoch,
                    ?e,
                    "skipping epoch enter — res muxer register failed"
                );
                return;
            }
        };

        let handle = engine.start(vote_sub, cert_sub, res_sub);
        self.active_epochs.insert(epoch, handle);
        info!(?epoch, "epoch entered");
    }

    /// Abort engines of all epochs below `current` (exit-at-transition; see
    /// the lifecycle note above the actor). Called with the just-entered
    /// epoch, so a stale/replayed boundary for an OLD epoch can never abort
    /// a newer engine (`e < cutoff` only).
    fn prune_old(&mut self, current: Epoch) {
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
                info!(?e, "epoch exited (retention window)");
            }
        }
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

/// Edge-triggered wait for a beacon-active epoch's local DKG share.
///
/// `enter` fires off the consensus ordering clock the instant the boundary block
/// finalizes, but this node's own DKG share lands in the shared `ceremony_store`
/// off the gossip plane — the `DkgActor` finalizes it the moment the quorum-th
/// `Reveal` arrives (`actor.rs::drive_finalization`, event-driven, independent of
/// the frozen-at-the-boundary height feed). That can land just AFTER `enter`. If
/// `resolve` returned `None` and we proceeded, `EpochEngine::new` would bake a
/// PERMANENT verify-only demote for the whole epoch (`engine.rs`: `(None, true)
/// => false`), dropping a signing member out of the seed quorum — enough
/// simultaneous race-losers and the epoch's seed quorum is unreachable, wedging
/// it. A potential signer therefore waits for its share before resolving.
///
/// Edge-driven, not polled: the `DkgActor` fires `share_notify` the instant it
/// inserts a share, so this wakes on that edge rather than sampling on a clock.
/// `notify.notified()` is ARMED BEFORE the `resolve()` re-check, so a share that
/// lands in the gap between the check and the await still wakes us (no lost
/// wakeup). Deadlock-free: the actor advances on its own task while this manager
/// loop blocks. A non-signer, a pre-beacon epoch, or a genuinely sub-quorum DKG
/// (share never lands) returns `None` immediately / on `timeout` — option-A
/// pure-multisig, never an unbounded block.
async fn wait_for_beacon<C: Clock>(
    ctx: &C,
    resolve: impl Fn() -> Option<BeaconKey>,
    notify: &tokio::sync::Notify,
    beacon_active: bool,
    is_signer: bool,
    timeout: Duration,
) -> Option<BeaconKey> {
    if let Some(beacon) = resolve() {
        return Some(beacon);
    }
    // Only a signing member of a beacon-active epoch needs the local share; a
    // follower / pre-beacon epoch / rotated-out node is correctly keyless and
    // must not block on a share it will never hold.
    if !beacon_active || !is_signer {
        return None;
    }
    // One deadline for the whole wait (created once, polled across iterations — a
    // spurious wake for another epoch's share does not reset it).
    let deadline = ctx.sleep(timeout);
    tokio::pin!(deadline);
    loop {
        // Arm the wakeup BEFORE the re-check: if the share lands between `resolve`
        // and the `select!`, the already-registered `notified()` still fires.
        let notified = notify.notified();
        if let Some(beacon) = resolve() {
            return Some(beacon);
        }
        tokio::pin!(notified);
        tokio::select! {
            // Prefer the share-landed edge over the timeout (deterministic poll order).
            biased;
            _ = &mut notified => continue,
            _ = &mut deadline => {
                warn!(
                    "beacon share did not land within the self-heal window — \
                     entering epoch pure-multisig (option-A stall)"
                );
                return None;
            }
        }
    }
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

    // The beacon-readiness wait: the fast-None gates (non-signer, pre-beacon epoch)
    // short-circuit WITHOUT waiting; a signing member of a beacon-active epoch BLOCKS
    // on the share-landed edge (the actor's `share_notify`) and returns the moment its
    // DKG share lands; and a genuinely absent share times out to None after the BOUNDED
    // timeout (so `enter` can never block the manager loop indefinitely), not forever.
    #[test]
    fn wait_for_beacon_gates_and_self_heals() {
        use commonware_cryptography::bls12381::{dkg::deal_anonymous, primitives::variant::MinSig};
        use commonware_runtime::{deterministic, Runner as _, Spawner as _};
        use commonware_utils::N3f1;
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

        // A real (cheap) BeaconKey from a 4-party anonymous deal.
        let mut rng = StdRng::seed_from_u64(7);
        let (sharing, shares) =
            deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), 4u32.try_into().unwrap());
        let key: BeaconKey = (sharing, Some(shares[0].clone()), b"ns".to_vec());

        // Gate 1 — a non-signer never blocks on a share it will never hold: with an
        // empty resolve the closure is consulted exactly once, then None.
        let calls = Arc::new(AtomicU32::new(0));
        let got = deterministic::Runner::default().start({
            let calls = calls.clone();
            move |ctx| async move {
                let notify = tokio::sync::Notify::new();
                wait_for_beacon(
                    &ctx,
                    move || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        None
                    },
                    &notify,
                    true,
                    false,
                    Duration::from_millis(1),
                )
                .await
            }
        });
        assert!(got.is_none(), "non-signer must not acquire a beacon");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "non-signer must short-circuit, not wait"
        );

        // Gate 2 — a pre-beacon (pure-multisig) epoch likewise short-circuits.
        let calls = Arc::new(AtomicU32::new(0));
        let got = deterministic::Runner::default().start({
            let calls = calls.clone();
            move |ctx| async move {
                let notify = tokio::sync::Notify::new();
                wait_for_beacon(
                    &ctx,
                    move || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        None
                    },
                    &notify,
                    false,
                    true,
                    Duration::from_millis(1),
                )
                .await
            }
        });
        assert!(got.is_none());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "pre-beacon epoch must short-circuit, not wait"
        );

        // Self-heal — a signing member of a beacon-active epoch BLOCKS on the
        // share-landed edge: a concurrent task flips the resolver to Some and fires
        // the notifier; the wait wakes on that edge (not a clock) and returns the key.
        let got = deterministic::Runner::default().start({
            let key = key.clone();
            move |ctx| async move {
                let notify = Arc::new(tokio::sync::Notify::new());
                let ready = Arc::new(AtomicBool::new(false));
                // Producer: after a virtual delay, land the share + fire the edge.
                drop(ctx.with_label("share_lander").spawn({
                    let notify = notify.clone();
                    let ready = ready.clone();
                    move |c| async move {
                        c.sleep(Duration::from_millis(5)).await;
                        ready.store(true, Ordering::SeqCst);
                        notify.notify_waiters();
                    }
                }));
                wait_for_beacon(
                    &ctx,
                    move || ready.load(Ordering::SeqCst).then(|| key.clone()),
                    &notify,
                    true,
                    true,
                    Duration::from_secs(10), // generous; the edge wakes us long before this
                )
                .await
            }
        });
        assert!(
            got.is_some(),
            "signer must wake on the share-landed edge and return the key"
        );

        // Bounded timeout — an absent share (the notifier never fires) returns None
        // after the timeout, never forever.
        let got = deterministic::Runner::default().start(move |ctx| async move {
            let notify = tokio::sync::Notify::new();
            wait_for_beacon(&ctx, || None, &notify, true, true, Duration::from_millis(5)).await
        });
        assert!(got.is_none(), "absent share must time out to None");
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
        assert_eq!(entered, Epoch::new(3), "entered tip advanced to the frontier");

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
