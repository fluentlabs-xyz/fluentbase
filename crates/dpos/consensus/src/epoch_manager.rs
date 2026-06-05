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
    application::FluentApp,
    block::Block,
    engine::{EpochEngine, EpochEngineConfig},
    epocher::OriginEpocher,
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
    fluent_namespace, keys::ValidatorBlsKeypair, scheme::build_verifier, Scheme as BlsScheme,
};
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use rand_core::CryptoRngCore;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::mpsc;
use tracing::{info, warn};

/// Concurrent active-epochs window. Keeps the current + prior epoch's engines
/// alive simultaneously so the boundary handoff (epoch E keeps producing until
/// E+1 takes over) is seamless. NOTE: this is this crate's own design, NOT a
/// commonware requirement — commonware's `Plan` enum is `{Propose, Forward}`
/// (a broadcast-relay plan), there is no `Plan::Sequential`; the `Sequential`
/// used by the engine is `commonware_parallel::Sequential` (a codec strategy).
pub const EPOCH_RETENTION_WINDOW: u64 = 2;

/// Bounded mpsc capacity for boundary triggers (tokio `mpsc::channel(N)`).
const BOUNDARY_BUFFER: usize = 64;

/// Per-epoch lifecycle actor.
pub struct Actor<E, B, PB, BE, AB, Attrs>
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
    /// `prune_old` (the scheme provider is never pruned).
    highest_entered_epoch: Epoch,
    /// Highest epoch observed on the vote backup channel — the live network
    /// epoch. Gates `is_live_epoch`: epochs below it only soft-enter.
    highest_observed_epoch: Epoch,
    cfg: Config<B, PB, BE, AB, Attrs>,
}

/// Configuration for the [`Actor`].
pub struct Config<B, PB, BE, AB, Attrs> {
    pub me: PublicKey,
    pub blocker: B,
    pub chain_id: u64,
    /// Single cross-epoch `OriginEpocher` — built once in
    /// `OuterBuilder::build`, cloned into both the marshal Config and
    /// every `EpochEngineConfig` constructed in `enter()`. `origin = dposActivationBlock`.
    pub epocher: OriginEpocher,
    pub signer_keypair: Option<ValidatorBlsKeypair>,
    pub app: FluentApp<PB, BE, AB, Attrs>,
    pub timeouts: ConsensusTimeouts,
    pub mailbox_size: usize,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`].
    pub marshal_mailbox: MarshalMailbox<BlsScheme, Standard<Block>>,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`].
    pub slasher_mailbox: SlasherMailbox,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`].
    pub page_cache: CacheRef,
    /// Callback into [`crate::outer::EpochSchemeProvider`] so marshal can verify
    /// cross-epoch finalization certificates (provider is never pruned).
    pub register_scheme: Arc<dyn Fn(Epoch, BlsScheme) + Send + Sync>,
}

impl<E, B, PB, BE, AB, Attrs> Actor<E, B, PB, BE, AB, Attrs>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics,
    B: Blocker<PublicKey = PublicKey> + Clone,
    PB: crate::application::PayloadBuilderLike<
            BuiltSealed = reth_primitives_traits::SealedBlock<reth_ethereum_primitives::Block>,
        > + Clone
        + Send
        + Sync
        + 'static,
    BE: crate::application::BeaconEngineLike<
            PayloadAttrs = Attrs,
            ExecutionData = reth_primitives_traits::SealedBlock<reth_ethereum_primitives::Block>,
        > + Clone
        + Send
        + Sync
        + 'static,
    AB: crate::application::PayloadAttrsBuilderLike<Attrs = Attrs, Header = alloy_consensus::Header>
        + Clone
        + Send
        + Sync
        + 'static,
    Attrs: Clone + Send + Sync + 'static,
{
    /// Construct the actor + return the bounded `boundary_tx` sender (held by
    /// 03's `EpochTransition`).
    pub fn new(
        context: E,
        cfg: Config<B, PB, BE, AB, Attrs>,
    ) -> (Self, mpsc::Sender<(Epoch, ValidatorSetSnapshot)>) {
        let (boundary_tx, boundary_rx) = mpsc::channel(BOUNDARY_BUFFER);
        let actor = Self {
            context: ContextCell::new(context),
            active_epochs: BTreeMap::new(),
            boundary_rx,
            highest_entered_epoch: Epoch::new(0),
            highest_observed_epoch: Epoch::new(0),
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
        self.highest_observed_epoch = self.highest_observed_epoch.max(their_epoch);
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

    /// True when `epoch` is at or past the highest epoch observed on the backup
    /// channel — i.e. the live frontier, not a historical catch-up epoch. Below
    /// the frontier we only soft-enter (register the scheme, NO participating
    /// engine): a Simplex engine for a stale epoch has no live peers and would
    /// drive the executor on a dead fork, intermittently wedging the catch-up.
    ///
    /// NB: must NOT add a retention window here. During fast catch-up
    /// `highest_observed_epoch` tracks only ~1-2 epochs ahead of the walk, so a
    /// `+ EPOCH_RETENTION_WINDOW` slack makes the gate true for nearly every
    /// catch-up epoch → they all full-enter → spurious engines → flaky wedge.
    /// Strict `>=` soft-enters every below-frontier epoch; once the walk reaches
    /// the frontier (votes arrive on a registered subchannel, not backup, so
    /// `highest_observed_epoch` stops rising) the frontier epoch full-enters.
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

        // Soft-enter for catch-up epochs: register the verifier so the marshal can
        // verify this epoch's finalization certs, but do NOT spawn a participating
        // engine. No peers vote in a historical epoch, and a live engine there would
        // drive the executor on a dead fork. The live epoch (and the retention window
        // below it) full-enters below.
        if !self.is_live_epoch(epoch) {
            match epoch_committee_from_snapshot(&snap) {
                Ok(committee) => (self.cfg.register_scheme)(
                    epoch,
                    build_verifier(&fluent_namespace(self.cfg.chain_id), committee.bimap),
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
                app: self.cfg.app.clone(),
                timeouts: self.cfg.timeouts,
                mailbox_size: self.cfg.mailbox_size,
                register_scheme: self.cfg.register_scheme.clone(),
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

    /// Abort epochs older than `current - EPOCH_RETENTION_WINDOW`.
    fn prune_old(&mut self, current: Epoch) {
        let cutoff = current.get().saturating_sub(EPOCH_RETENTION_WINDOW);
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

#[cfg(test)]
#[allow(unused_imports)]
mod tests {
    // Smoke tests are gated on a real `commonware_runtime::deterministic::Runner`
    // wiring of all 5 channels + a working marshal::core::Actor.
    // The smoke-level test sits in `crate::outer::tests` once OuterEngine
    // exists, because `Config::marshal_mailbox` and `page_cache` are produced
    // by OuterEngine.
}
