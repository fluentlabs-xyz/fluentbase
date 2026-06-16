//! Per-epoch consensus engine lifecycle.
//!
//! Owns the active-epochs map, 3 internal Muxers (vote/cert/resolver),
//! and an event-driven boundary trigger (`mpsc::Receiver<(Epoch, snap)>`)
//! fed by [`fluentbase_staking_reader::EpochTransition`].
//!
//! `marshal::core::Actor`, `buffered::Engine`, and the 2
//! `immutable::Archive` instances do **not** pass through here — they live
//! in [`crate::outer::OuterEngine`]. EpochManager threads only the 3
//! simplex channels.

use crate::{
    application::{ExecutedChain, FluentApp, OrderingAssembler},
    engine::{EpochEngine, EpochEngineConfig},
    epocher::OriginEpocher,
    order_block::OrderBlock,
    scheme::epoch_committee_from_snapshot,
    slasher::Mailbox as SlasherMailbox,
    timeouts::ConsensusTimeouts,
};
use commonware_consensus::{
    marshal::{core::Mailbox as MarshalMailbox, standard::Standard},
    types::{Epoch, Epocher as _},
};
use commonware_cryptography::ed25519::PublicKey;
use commonware_p2p::{
    utils::mux::{Builder, MuxHandle, Muxer},
    Blocker, Receiver, Sender,
};
use commonware_runtime::{
    buffer::paged::CacheRef, spawn_cell, BufferPooler, Clock, ContextCell, Handle, Metrics,
    Spawner, Storage,
};
use commonware_utils::vec::NonEmptyVec;
use fluentbase_bls::{
    fluent_namespace,
    keys::ValidatorBlsKeypair,
    scheme::{build_verifier, BeaconKey},
    Scheme as BlsScheme,
};
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use rand_core::CryptoRngCore;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use tokio::sync::mpsc;
use tracing::{info, warn};

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
    /// Per-epoch threshold beacon key (devnet: one genesis key for all epochs);
    /// `None` ⇒ fallback (pure-multisig) epochs. Threaded into each
    /// `EpochEngineConfig` and the soft-enter verifier scheme.
    pub beacon: Option<BeaconKey>,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`].
    pub marshal_mailbox: MarshalMailbox<BlsScheme, Standard<OrderBlock>>,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`].
    pub slasher_mailbox: SlasherMailbox,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`].
    pub page_cache: CacheRef,
    /// Callback into [`crate::outer::EpochSchemeProvider`] so marshal can verify
    /// cross-epoch finalization certificates (trailing-window pruned; see SCHEME_RETENTION_EPOCHS).
    pub register_scheme: Arc<dyn Fn(Epoch, BlsScheme) + Send + Sync>,
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

    /// Start the manager. Threads the 3 simplex p2p channels — vote/cert/resolver.
    pub fn start<VS, VR, CS, CR, RS, RR>(
        mut self,
        votes: (VS, VR),
        certs: (CS, CR),
        resolver: (RS, RR),
    ) -> Handle<()>
    where
        VS: Sender<PublicKey = PublicKey>,
        VR: Receiver<PublicKey = PublicKey>,
        CS: Sender<PublicKey = PublicKey>,
        CR: Receiver<PublicKey = PublicKey>,
        RS: Sender<PublicKey = PublicKey>,
        RR: Receiver<PublicKey = PublicKey>,
    {
        spawn_cell!(self.context, self.run(votes, certs, resolver).await)
    }

    async fn run<VS, VR, CS, CR, RS, RR>(
        mut self,
        votes: (VS, VR),
        certs: (CS, CR),
        resolver: (RS, RR),
    ) where
        VS: Sender<PublicKey = PublicKey>,
        VR: Receiver<PublicKey = PublicKey>,
        CS: Sender<PublicKey = PublicKey>,
        CR: Receiver<PublicKey = PublicKey>,
        RS: Sender<PublicKey = PublicKey>,
        RR: Receiver<PublicKey = PublicKey>,
    {
        let (vote_s, vote_r) = votes;
        let (cert_s, cert_r) = certs;
        let (res_s, res_r) = resolver;

        // Vote mux carries a backup channel: votes for epochs with no registered
        // sub-channel (i.e. epochs ahead of us, while catching up) surface on
        // `vote_backup` instead of being dropped, driving the catch-up hint.
        let (mux_vote, mut vote_mux, mut vote_backup) = Muxer::builder(
            self.context.with_label("epoch_mgr_vote_mux"),
            vote_s,
            vote_r,
            self.cfg.mailbox_size,
        )
        .with_backup()
        .build();
        let mut mux_vote_handle = mux_vote.start();
        let (mux_cert, mut cert_mux) = Muxer::new(
            self.context.with_label("epoch_mgr_cert_mux"),
            cert_s,
            cert_r,
            self.cfg.mailbox_size,
        );
        let mut mux_cert_handle = mux_cert.start();
        let (mux_res, mut res_mux) = Muxer::new(
            self.context.with_label("epoch_mgr_resolver_mux"),
            res_s,
            res_r,
            self.cfg.mailbox_size,
        );
        let mut mux_res_handle = mux_res.start();

        // Supervisor: boundary_rx-close is detected inline via the
        // Option pattern in the recv arm (bare `r = &mut handle` arms have
        // no pattern guard so `tokio::select!`'s `else` branch is structurally
        // unreachable while Muxer arms are present and pending — we must
        // check `recv() -> None` inline). Graceful epoch_manager exit happens
        // via either (a) boundary_rx close, or (b) Muxer exit triggered by
        // network teardown (aborting network_handle cascades through p2p
        // actors to close the Muxer's underlying receiver).
        loop {
            tokio::select! {
                recv = self.boundary_rx.recv() => {
                    match recv {
                        Some((epoch, snap)) => {
                            self.enter(epoch, snap, &mut vote_mux, &mut cert_mux, &mut res_mux).await;
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
                r = &mut mux_vote_handle => {
                    match r {
                        Ok(Ok(())) => warn!("vote Muxer exited cleanly (unexpected)"),
                        Ok(Err(e)) => tracing::error!(error = ?e, "vote Muxer p2p receiver failed"),
                        Err(e) => tracing::error!(error = ?e, "vote Muxer task failed"),
                    }
                    break;
                }
                r = &mut mux_cert_handle => {
                    match r {
                        Ok(Ok(())) => warn!("cert Muxer exited cleanly (unexpected)"),
                        Ok(Err(e)) => tracing::error!(error = ?e, "cert Muxer p2p receiver failed"),
                        Err(e) => tracing::error!(error = ?e, "cert Muxer task failed"),
                    }
                    break;
                }
                r = &mut mux_res_handle => {
                    match r {
                        Ok(Ok(())) => warn!("resolver Muxer exited cleanly (unexpected)"),
                        Ok(Err(e)) => tracing::error!(error = ?e, "resolver Muxer p2p receiver failed"),
                        Err(e) => tracing::error!(error = ?e, "resolver Muxer task failed"),
                    }
                    break;
                }
            }
        }

        // Abort all Mux handles + all per-epoch engine handles. abort() is
        // idempotent (no-op on already-completed handles per
        // monorepo/runtime/src/utils/handle.rs:107-118).
        mux_vote_handle.abort();
        mux_cert_handle.abort();
        mux_res_handle.abort();
        for (epoch, handle) in std::mem::take(&mut self.active_epochs) {
            info!(?epoch, "aborting active epoch engine on exit");
            handle.abort();
        }
    }

    /// A vote arrived for an epoch with no registered sub-channel — the network
    /// is ahead of us. Hint the marshal to fetch the finalization at the boundary
    /// of the highest epoch we can already verify; its gap-repair then walks our
    /// finalized tip forward. Crossing each boundary delivers the next epoch's
    /// blocks (boundary_hook → on_finalized → enter), registering the next scheme
    /// so the following hint reaches one epoch further — until we catch up.
    async fn handle_msg_for_unregistered_epoch(&mut self, their_epoch: Epoch, from: PublicKey) {
        self.corroborate_observed_epoch(their_epoch, from.clone());
        if their_epoch <= self.highest_entered_epoch {
            return;
        }
        let Some(boundary) = self.cfg.epocher.last(self.highest_entered_epoch) else {
            return;
        };
        info!(
            observed = their_epoch.get(),
            entered = self.highest_entered_epoch.get(),
            %boundary,
            "catch-up: behind network; hinting marshal toward entered-epoch boundary"
        );
        self.cfg
            .marshal_mailbox
            .hint_finalized(boundary, NonEmptyVec::new(from))
            .await;
    }

    /// Advance the live frontier ONLY when f+1 DISTINCT peers have named the same
    /// future epoch on the (unauthenticated) vote backup channel. With ≤ f
    /// Byzantine validators, f+1 distinct reporters always include ≥1 honest one,
    /// who only votes at the true live epoch — so a single (or up to f colluding)
    /// Byzantine peer(s) cannot inflate the frontier and force permanent
    /// soft-enter. Until the first `enter` sets the committee size, corroboration
    /// is disabled (the cold-start epoch full-enters from the verified boundary
    /// trigger, so an early backup message must not be able to gate it off).
    fn corroborate_observed_epoch(&mut self, their_epoch: Epoch, from: PublicKey) {
        corroborate_frontier(
            &mut self.observed_reporters,
            &mut self.sender_pins,
            &mut self.highest_observed_epoch,
            self.committee_size,
            their_epoch,
            from,
        );
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
    /// [`Self::corroborate_observed_epoch`].
    fn is_live_epoch(&self, epoch: Epoch) -> bool {
        epoch >= self.highest_observed_epoch
    }

    async fn enter<VS, VR, CS, CR, RS, RR>(
        &mut self,
        epoch: Epoch,
        snap: ValidatorSetSnapshot,
        vote_mux: &mut MuxHandle<VS, VR>,
        cert_mux: &mut MuxHandle<CS, CR>,
        res_mux: &mut MuxHandle<RS, RR>,
    ) where
        VS: Sender<PublicKey = PublicKey>,
        VR: Receiver<PublicKey = PublicKey>,
        CS: Sender<PublicKey = PublicKey>,
        CR: Receiver<PublicKey = PublicKey>,
        RS: Sender<PublicKey = PublicKey>,
        RR: Receiver<PublicKey = PublicKey>,
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
            match epoch_committee_from_snapshot(&snap) {
                Ok(committee) => (self.cfg.register_scheme)(
                    epoch,
                    build_verifier(
                        &fluent_namespace(self.cfg.chain_id),
                        committee.bimap,
                        self.cfg.beacon.clone(),
                    ),
                ),
                Err(e) => warn!(
                    ?epoch,
                    ?e,
                    "soft-enter skipped — invalid committee snapshot"
                ),
            }
            info!(?epoch, "epoch soft-entered (scheme only, catch-up)");
            return;
        }

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
                beacon: self.cfg.beacon.clone(),
            },
            self.cfg.marshal_mailbox.clone(),
            self.cfg.slasher_mailbox.clone(),
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

        // Register the three sub-channels. `Muxer::register` errors are
        // AlreadyRegistered (unreachable — guarded by `active_epochs` above) or
        // Closed (muxer task gone, i.e. teardown). On any error, skip the enter
        // rather than `.expect()`-panic (which would cascade to the whole stack).
        // Partial registrations auto-deregister when their `SubReceiver` drops.
        let vote_sub = match vote_mux.register(epoch.get()).await {
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
        let cert_sub = match cert_mux.register(epoch.get()).await {
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
        let res_sub = match res_mux.register(epoch.get()).await {
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
}
