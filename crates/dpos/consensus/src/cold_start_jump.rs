//! The EL fast-forward — one forkchoice toward an already-authenticated tip.
//!
//! A node whose ordering plane has run far past its EL closes the gap here: one
//! forkchoice (head = safe = finalized = the committee-attested derived hash)
//! lets reth's devp2p backfill canonicalize the branch, and the caller re-seeds
//! its anchor at the landing.
//!
//! The target is authenticated by the caller before `sync_to`, never by this
//! module. The jump is forward-only: a landing that does not advance the anchor
//! is dropped, because reth ancestor-skips a backward FCU.
//!
//! Every caller drives reth alone while the jump runs, so the jump is a prep path
//! rather than a long-lived second writer.

use crate::{
    application::BeaconEngineLike, cert_follow::UpstreamFinalized, cert_inlet::CommitteeSource,
    order_block::K,
};
use alloy_primitives::B256;
use alloy_rpc_types_engine::{ForkchoiceState, PayloadStatusEnum};
use commonware_parallel::Sequential;
use commonware_runtime::{tokio::Context, Clock};
use eyre::{ensure, eyre, WrapErr as _};
use rand_core::CryptoRngCore;
use reth_storage_api::{BlockHashReader, BlockNumReader};
use std::{future::Future, sync::Arc, time::Duration};
use tracing::{error, info, warn};

/// One L1 batch = 1024 blocks = ~17 min of chain time at the committed 1 blk/s.
/// Above this gap the cold-start re-runs the EL-sync phase instead of resuming
/// block-by-block: batched pipeline sync avoids the per-height RPC round-trip
/// and per-block engine-API cost. It is also the serving-window cap, so a
/// downstream follower's repairable gap and its own jump threshold coincide.
pub const JUMP_THRESHOLD: u64 = 1024;

/// Terminal classification of a jump attempt. The caller owns the entire
/// backfill wait, so there is no in-progress variant — [`jump_to_target`] only
/// ever returns a terminal one.
pub enum JumpOutcome {
    /// A jump landed: re-seed the anchor + finalized cursor at `(landing, hash)`
    /// and advance the running marshal floor to `floor` (= landing − K).
    Landed {
        landing: u64,
        hash: B256,
        floor: u64,
    },
    /// Shallow gap / stale-or-backward landing — no-op; the inlet's ordinary
    /// pulls and the executor gap-walk cover the residual gap.
    Lagging,
    /// Transport/timeout `sync_to` failure, or an L1 `holds()` probe that errored
    /// rather than answered. Non-fatal: the steady-state caller retries on the
    /// next `Update::Tip`. A probe error carries no verdict, so nothing may be
    /// concluded from it.
    Stalled(eyre::Report),
    /// The EL did not end up on the attested branch, either because `sync_to` saw
    /// reth declare the served branch `Invalid` or because the landing does not
    /// hold the attested `block.result`. The steady-state caller treats this as
    /// `Fault::corruption`: the target came out of this node's own attested
    /// archive, so the contradiction is between the local EL and an authenticated
    /// certificate.
    InvalidTarget(eyre::Report),
    /// `sync_to` tripped the connected-but-no-progress net
    /// ([`SyncFailure::StalledWithPeers`]): reth had peers yet its executed head
    /// did not advance for [`EL_SYNC_STALL_ESCAPE`]. Non-fatal and distinct from
    /// [`Stalled`](JumpOutcome::Stalled): the divergence root cause is
    /// deterministic, so a re-jump re-wedges; the caller re-arms and keeps the
    /// refill deferred, leaving the node observable.
    StalledWithPeers(eyre::Report),
    /// The post-sync L1 `holds()` re-assert answered false: the EL-synced head
    /// does not descend from the L1-finalized checkpoint, an L1 fork. The runtime
    /// caller halts (verify-only, stops driving reth, stays observable). A probe
    /// that errored is [`Stalled`](JumpOutcome::Stalled), not this.
    ///
    /// [`SafetyHalt`]: crate::sync_metrics::SafetyHalt
    L1Fork(eyre::Report),
}

/// EL-sync loop tick. Tick counts bound elapsed time monotonically without
/// reading a clock, so an NTP step can neither fire the nets spuriously nor
/// silently disable them.
pub(crate) const EL_SYNC_TICK: Duration = Duration::from_secs(2);

/// Secondary net for the EL-sync wait: tick-counted and deliberately generous,
/// not a sync-duration estimate. Healthy completion exits via the FCU
/// `PayloadStatus::Valid` terminator in [`RethElSync::sync_to`], so this ceiling
/// only fires on a non-completing attempt (a forged or withheld tip, or a broken
/// engine channel), converting an indefinite silent hang into a loud abort. A
/// trip yields `Stalled`, which the steady-state caller re-attempts on the next
/// tip, so oversizing is the cheap failure direction. 6 h ≈ a day-and-change
/// offline at 1 blk/s.
const EL_SYNC_BACKSTOP_CEILING: Duration = Duration::from_secs(6 * 60 * 60);

/// Primary net: when reth reports zero connected devp2p peers continuously for
/// this long, the sync can never complete (no uplink, or the trusted peer was
/// never configured). Sized above the worst-case trusted-peer dial latency so a
/// healthy boot is not false-tripped; the window resets when a peer connects.
const EL_SYNC_NO_PEERS_GRACE: Duration = Duration::from_secs(90);

/// Tertiary net: reth is connected but its executed head does not advance for
/// this long — the wedge where the pipeline Execution stage unwound to a bad
/// ancestor and went idle answering every FCU `SYNCING` forever. The no-peers net
/// never fires here and the 6-h backstop would leave the node silently wedged. The
/// escape is deliberately non-fatal ([`JumpOutcome::StalledWithPeers`]): the root
/// cause is deterministic, so each re-jump re-wedges, and the node stays
/// observable (an error log and a counter per attempt) instead of silent.
pub(crate) const EL_SYNC_STALL_ESCAPE: Duration = Duration::from_secs(300);

/// Which net tripped the EL-sync watchdog.
#[derive(Debug, PartialEq, Eq)]
enum WatchdogTrip {
    /// Zero connected devp2p peers for [`EL_SYNC_NO_PEERS_GRACE`].
    NoPeers,
    /// The backstop ceiling elapsed without a `Valid` terminator.
    Ceiling,
    /// Peers > 0 but the executed head (`best_block_number`) did not advance for
    /// [`EL_SYNC_STALL_ESCAPE`].
    StalledWithPeers,
}

/// Pure, tick-counted watchdog for [`RethElSync::sync_to`]; keeping the net logic
/// runtime-free makes it unit-testable, since `RethElSync` itself is bound to the
/// concrete tokio `Context`.
struct SyncWatchdog {
    ticks: u64,
    no_peer_ticks: u64,
    max_ticks: u64,
    no_peer_max_ticks: u64,
    /// Consecutive ticks with peers > 0 and no advance in the executed head — the
    /// [`WatchdogTrip::StalledWithPeers`] accumulator.
    stall_ticks: u64,
    /// Last observed executed head (`best_block_number`); `None` before the first
    /// tick. Any forward move resets [`Self::stall_ticks`].
    last_progress: Option<u64>,
    stall_max_ticks: u64,
}

impl SyncWatchdog {
    fn new() -> Self {
        Self {
            ticks: 0,
            no_peer_ticks: 0,
            max_ticks: EL_SYNC_BACKSTOP_CEILING.as_secs() / EL_SYNC_TICK.as_secs(),
            no_peer_max_ticks: EL_SYNC_NO_PEERS_GRACE.as_secs() / EL_SYNC_TICK.as_secs(),
            stall_ticks: 0,
            last_progress: None,
            stall_max_ticks: EL_SYNC_STALL_ESCAPE.as_secs() / EL_SYNC_TICK.as_secs(),
        }
    }

    /// Advance one tick with the current connected-peer count and the executed
    /// head (`best_block_number`). `Some(trip)` means the attempt must abort; `None`
    /// means keep waiting. The no-peers counter resets the instant a peer connects,
    /// so interleaved zero-peer windows never accumulate; the stall counter accrues
    /// only while peers > 0 and the head is unchanged, so a healthy but
    /// bandwidth-limited backfill is never false-tripped.
    fn on_tick(&mut self, connected_peers: usize, latest_block: u64) -> Option<WatchdogTrip> {
        self.ticks += 1;

        match self.last_progress {
            Some(prev) if prev == latest_block && connected_peers > 0 => self.stall_ticks += 1,
            _ => self.stall_ticks = 0,
        }
        self.last_progress = Some(latest_block);
        if self.stall_ticks >= self.stall_max_ticks {
            return Some(WatchdogTrip::StalledWithPeers);
        }

        if connected_peers == 0 {
            self.no_peer_ticks += 1;
            if self.no_peer_ticks >= self.no_peer_max_ticks {
                return Some(WatchdogTrip::NoPeers);
            }
        } else {
            self.no_peer_ticks = 0;
        }
        (self.ticks >= self.max_ticks).then_some(WatchdogTrip::Ceiling)
    }

    /// Elapsed wall-clock equivalent (for diagnostics in a trip message only).
    fn elapsed(&self) -> Duration {
        EL_SYNC_TICK * self.ticks as u32
    }
}

/// Typed terminal failure of an [`ElSync::sync_to`] attempt, distinguishing a
/// branch reth itself declared invalid from a timeout/transport stall, so
/// [`jump_to_target`] can route them to different [`JumpOutcome`]s.
///
/// `From<eyre::Report>` classifies any generic failure as [`Stalled`].
///
/// [`Stalled`]: SyncFailure::Stalled
/// [`Invalid`]: SyncFailure::Invalid
#[derive(Debug)]
pub enum SyncFailure {
    /// reth's FCU polling loop observed `PayloadStatusEnum::Invalid` — the
    /// served branch is structurally wrong. A genuine terminal verdict, never a
    /// transient mid-backfill state: reth short-circuits to `Syncing` while
    /// backfill is non-idle, so `Invalid` can only fire once reth itself judged
    /// the branch bad.
    Invalid(eyre::Report),
    /// Any other `sync_to` failure: transport error, provider read failure,
    /// zero-peers grace expired, or the backstop ceiling tripped. reth rendered
    /// no verdict on the branch at all.
    Stalled(eyre::Report),
    /// reth is connected (peers > 0) but its executed head did not advance for
    /// [`EL_SYNC_STALL_ESCAPE`]: an Execution-stage divergence unwound to a bad
    /// ancestor and reth went idle answering `SYNCING` forever. Distinct from
    /// [`Stalled`](SyncFailure::Stalled): reth is demonstrably reachable yet
    /// making no progress, which is a different operator signal from a missing
    /// uplink.
    StalledWithPeers(eyre::Report),
}

impl std::fmt::Display for SyncFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(e) | Self::Stalled(e) | Self::StalledWithPeers(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SyncFailure {}

impl From<eyre::Report> for SyncFailure {
    fn from(report: eyre::Report) -> Self {
        Self::Stalled(report)
    }
}

/// EL-sync seam: drive reth onto the attested tip of `latest` (FCU + devp2p
/// backfill, with the no-progress stall detector) and return the `(height, hash)`
/// it landed on. Implemented over the provider + beacon engine by
/// [`RethElSync`]; a fake implements it in tests.
pub trait ElSync: Send + Sync {
    fn sync_to(
        &self,
        latest: &UpstreamFinalized,
    ) -> impl Future<Output = Result<(u64, B256), SyncFailure>> + Send;

    /// Drive reth onto an operator-supplied checkpoint block hash and return the
    /// `(height, hash)` it landed on — the one entry that carries no certificate
    /// at all (the fresh-datadir follower: no geometry, no committee, nothing
    /// local to check a peer's answer against).
    ///
    /// Same FCU shape and same nets as [`Self::sync_to`]; the height is learned
    /// from the landing (`block_number(hash)`) instead of being carried on the
    /// wire, which is why the config needs no height field. A hash reth never
    /// canonicalizes yields a stall, never a silent landing at some other height.
    fn sync_to_checkpoint(
        &self,
        checkpoint: B256,
    ) -> impl Future<Output = Result<(u64, B256), SyncFailure>> + Send;

    /// Whether the local chain holds `hash` canonically — the post-jump L1
    /// trust-root re-assert (the synced head must be a descendant of the
    /// L1-finalized block).
    fn holds(&self, hash: B256) -> eyre::Result<bool>;
}

/// [`ElSync`] over the node's reth: FCU toward the attested derived hash (the
/// upstream tip's `result` = derived hash of tip − K) and wait for reth to declare
/// the tip canonical+executed via the FCU `PayloadStatus::Valid` terminator,
/// failing only on the zero-peers / backstop / stall nets.
pub struct RethElSync<Provider, BeaconEngine> {
    ctx: Context,
    provider: Provider,
    beacon_engine: BeaconEngine,
    activation: u64,
    /// Read-only probe of reth's connected devp2p peer count, driving the primary
    /// no-peers net in [`Self::sync_to`]. A closure avoids a `reth-network-api`
    /// dependency and an extra generic parameter.
    peer_count: Arc<dyn Fn() -> usize + Send + Sync>,
}

impl<Provider, BeaconEngine> RethElSync<Provider, BeaconEngine> {
    pub fn new(
        ctx: Context,
        provider: Provider,
        beacon_engine: BeaconEngine,
        activation: u64,
        peer_count: Arc<dyn Fn() -> usize + Send + Sync>,
    ) -> Self {
        Self {
            ctx,
            provider,
            beacon_engine,
            activation,
            peer_count,
        }
    }
}

impl<Provider, BeaconEngine> RethElSync<Provider, BeaconEngine>
where
    Provider: BlockHashReader + BlockNumReader + Clone + Send + Sync + 'static,
    BeaconEngine: BeaconEngineLike + Clone + Send + Sync + 'static,
{
    /// The executed height/hash reth currently sits at, clamped to ≥ activation
    /// (the ordering chain starts there; pre-activation blocks carry no certs).
    /// Reads the canonical/executed head (`best_block_number`), not the static-file
    /// header tip (`last_block_number`): a landing reseeds the anchor and feeds the
    /// post-sync committee state read, so it must point at materialized state.
    fn local_landing(&self) -> eyre::Result<(u64, B256)> {
        let tip = self
            .provider
            .best_block_number()
            .wrap_err("provider failed to report chain head")?
            .max(self.activation);
        let hash = self
            .provider
            .block_hash(tip)?
            .ok_or_else(|| eyre!("reth does not hold its own reported tip {tip}"))?;
        Ok((tip, hash))
    }

    /// The FCU-drive itself, shared by [`ElSync::sync_to`] (target = the attested
    /// derived hash) and [`ElSync::sync_to_checkpoint`] (target = the operator's
    /// checkpoint hash). Returns once reth declares the target canonical+executed,
    /// or a typed [`SyncFailure`] when one of the three nets trips. `sync_target`
    /// is for the log lines only.
    async fn drive_fcu(&self, tip_hash: B256, sync_target: &str) -> Result<(), SyncFailure> {
        // head == safe == finalized is load-bearing: pointing `finalized` at the
        // not-yet-held target is what makes reth run the staged pipeline backfill;
        // with `finalized = genesis` reth would live-download disconnected blocks
        // that never canonicalize. Driving reth onto the target accepts nothing —
        // the target was authenticated by the caller.
        let fc_state = ForkchoiceState {
            head_block_hash: tip_hash,
            safe_block_hash: tip_hash,
            finalized_block_hash: tip_hash,
        };

        // Completion is reth's own declared sync verdict, not a DB-counter proxy:
        // re-issue the (idempotent) seeding FCU and read its `PayloadStatus`.
        // `Valid { latest_valid_hash == tip }` means the tip is canonical and its
        // state is materialized (exactly what the post-sync committee read needs);
        // `Invalid` is a verdict against the branch; `Syncing | Accepted` means
        // keep waiting. Re-issuing FCU(head=tip) each tick is a cheap status read
        // while backfill is non-idle, and the executor suppresses its heartbeat FCU
        // during a jump (single writer), so the re-FCU loop is safe. The wall-clock
        // nets below are the only failure paths — completion is the `Valid`
        // terminator.
        let mut watchdog = SyncWatchdog::new();
        let mut last_probe_error: Option<eyre::Report> = None;
        // A probe error reuses the previous value, so a transient read failure only
        // delays the stall verdict, never fakes progress.
        let mut last_best_block: u64 = self.provider.best_block_number().unwrap_or(0);
        loop {
            match self
                .beacon_engine
                .fork_choice_updated(fc_state)
                .await
                .wrap_err("forkchoice_updated probe during EL-sync")
            {
                Ok(resp) => match &resp.payload_status.status {
                    PayloadStatusEnum::Valid
                        if resp.payload_status.latest_valid_hash == Some(tip_hash) =>
                    {
                        break
                    }
                    PayloadStatusEnum::Invalid { validation_error } => {
                        return Err(SyncFailure::Invalid(eyre!(
                            "EL-sync: reth REJECTED the target {tip_hash:?} ({sync_target}) as \
                             INVALID: {validation_error} — reth rendered a verdict on the branch, \
                             it did not merely fail to finish"
                        )));
                    }
                    _ => {}
                },
                // A transient per-tick FCU error must not end the attempt; the backstop
                // bounds total wait, and the error is recorded for the trip message.
                Err(report) => {
                    warn!(
                        sync_target,
                        error = %report,
                        "EL-sync: transient FCU probe error; retrying (the backstop ceiling \
                         bounds total wait)"
                    );
                    last_probe_error = Some(report);
                }
            }

            let peers = (self.peer_count)();
            if let Ok(best) = self.provider.best_block_number() {
                last_best_block = best;
            }
            if peers == 0 && watchdog.no_peer_ticks == 0 {
                warn!(
                    sync_target,
                    grace = ?EL_SYNC_NO_PEERS_GRACE,
                    "EL-sync: reth reports ZERO connected devp2p peers — EL-sync cannot \
                     progress; will fail if none connects within the grace window \
                     (check --trusted-peers / firewall)"
                );
            }
            match watchdog.on_tick(peers, last_best_block) {
                Some(WatchdogTrip::NoPeers) => {
                    return Err(SyncFailure::Stalled(eyre!(
                        "EL-sync: reth had ZERO connected devp2p peers for \
                         {EL_SYNC_NO_PEERS_GRACE:?} ({sync_target}) — no uplink, or no trusted \
                         peer configured; EL-sync cannot complete"
                    )));
                }
                Some(WatchdogTrip::StalledWithPeers) => {
                    error!(
                        sync_target,
                        best_block = last_best_block,
                        peers,
                        stall = ?EL_SYNC_STALL_ESCAPE,
                        "EL-sync: reth is CONNECTED ({peers} peers) but its executed head has not \
                         advanced past {last_best_block} for {EL_SYNC_STALL_ESCAPE:?} \
                         ({sync_target}) — the EL pipeline is wedged (e.g. an unwound \
                         bad-ancestor answering SYNCING forever); escaping so the node stays \
                         observable instead of silently stuck at the 6-h backstop"
                    );
                    return Err(SyncFailure::StalledWithPeers(eyre!(
                        "EL-sync: reth CONNECTED ({peers} peers) but executed head frozen at \
                         {last_best_block} for {EL_SYNC_STALL_ESCAPE:?} ({sync_target}) — EL \
                         pipeline wedged"
                    )));
                }
                Some(WatchdogTrip::Ceiling) => {
                    return Err(SyncFailure::Stalled(eyre!(
                        "EL-sync: reth did not reach a VALID head within the \
                         {EL_SYNC_BACKSTOP_CEILING:?} backstop ({sync_target}, elapsed \
                         {:?}) — connected but not serving the branch, the engine is hung, or the \
                         gap exceeds the bounded catch-up window (a from-genesis deep sync must \
                         bootstrap from a state snapshot, not this path). Last probe error: {:?}",
                        watchdog.elapsed(),
                        last_probe_error
                    )));
                }
                None => {}
            }
            self.ctx.sleep(EL_SYNC_TICK).await;
        }
        Ok(())
    }
}

impl<Provider, BeaconEngine> ElSync for RethElSync<Provider, BeaconEngine>
where
    Provider: BlockHashReader + BlockNumReader + Clone + Send + Sync + 'static,
    BeaconEngine: BeaconEngineLike + Clone + Send + Sync + 'static,
{
    async fn sync_to(&self, latest: &UpstreamFinalized) -> Result<(u64, B256), SyncFailure> {
        // The upstream serves ordering artifacts; the only real EVM hash on the
        // wire is the committee-attested `result` (derived hash of tip − K).
        let tip_hash = latest.block.result;
        let tip_height = latest.block.height.saturating_sub(K);
        if tip_hash == B256::ZERO {
            info!(
                tip = latest.block.height,
                "EL-sync: the target is inside the pre-K window; nothing to EL-sync"
            );
            return self.local_landing().map_err(SyncFailure::Stalled);
        }
        // Gate on the executed head, not header presence: a header-only tip from an
        // interrupted backfill is not yet a valid landing.
        if self
            .provider
            .best_block_number()
            .wrap_err("best_block_number probe before EL-sync")?
            >= tip_height
        {
            return self.local_landing().map_err(SyncFailure::Stalled);
        }
        info!(
            tip_height,
            "EL-sync: driving reth toward the attested derived hash"
        );
        self.drive_fcu(tip_hash, &format!("target height {tip_height}"))
            .await?;
        // Clamp before resolving the hash so the returned pair is always
        // self-consistent (height and hash of the same block).
        let landing = tip_height.max(self.activation);
        let hash = self
            .provider
            .block_hash(landing)
            .wrap_err("block_hash(landing) probe after EL-sync")?
            .ok_or_else(|| eyre!("EL-sync landed but block {landing} vanished"))?;
        Ok((landing, hash))
    }

    async fn sync_to_checkpoint(&self, checkpoint: B256) -> Result<(u64, B256), SyncFailure> {
        // Already canonical (a restart on a datadir that once synced here): the
        // checkpoint entry is a no-op, and the height is the one reth reports.
        if let Some(height) = self
            .provider
            .block_number(checkpoint)
            .wrap_err("block_number(checkpoint) probe before EL-sync")?
        {
            info!(
                height,
                checkpoint = ?checkpoint,
                "EL-sync: the operator checkpoint is already canonical; nothing to EL-sync"
            );
            return Ok((height, checkpoint));
        }
        info!(checkpoint = ?checkpoint, "EL-sync: driving reth toward the operator checkpoint");
        self.drive_fcu(checkpoint, &format!("operator checkpoint {checkpoint:?}"))
            .await?;
        let height = self
            .provider
            .block_number(checkpoint)
            .wrap_err("block_number(checkpoint) probe after EL-sync")?
            .ok_or_else(|| {
                eyre!("EL-sync reported the operator checkpoint {checkpoint:?} VALID but reth does not hold it")
            })?;
        Ok((height, checkpoint))
    }

    fn holds(&self, hash: B256) -> eyre::Result<bool> {
        Ok(self
            .provider
            .block_number(hash)
            .wrap_err("block_number(l1 checkpoint) probe failed")?
            .is_some())
    }
}

/// Structural check of a single by-height finalization fetched outside
/// `FrontierHandler::deliver`'s reach: the cert must sign the served body
/// (`cert.payload == block.digest()`).
///
/// `deliver` runs this as its payload↔digest step; the three by-height seams that
/// fetch one height on their own bypass both writers, so none inherits the bind
/// `store_finalization` applies.
pub(crate) fn verify_jump_structural(latest: &UpstreamFinalized) -> eyre::Result<()> {
    if latest.finalization.proposal.payload != latest.block.digest() {
        return Err(eyre!(
            "finalization cert payload != block digest at height {}",
            latest.block.height
        ));
    }
    Ok(())
}

/// Committee-BLS authentication of a single by-height finalization: read
/// `committee[epoch]` at `at_hash` and verify the 2f+1 multisig against it,
/// failing closed on anything else.
///
/// Unlike [`verify_jump_structural`], this is not a step of
/// `FrontierHandler::deliver`: `deliver` runs its own committee read and BLS
/// check, because an unreadable committee is a drop on the wire and a refusal
/// here. No caller has an L1 fallback, so an unreadable committee is always a
/// refusal.
///
/// Cold-start / boundary verify passes `oracle = None` (no local beacon key is
/// resolvable there), which gives a vote-only cert verify.
pub(crate) fn verify_jump_authenticated<C: CommitteeSource>(
    latest: &UpstreamFinalized,
    committees: &C,
    at_hash: B256,
    ctx: &mut (impl Clock + CryptoRngCore),
) -> eyre::Result<()> {
    let epoch = latest.finalization.proposal.round.epoch().get();
    let scheme = committees.scheme_at(epoch, at_hash, None).map_err(|e| {
        eyre!(
            "committee[{epoch}] is unreadable at {at_hash:?} (height {}) — there is no way to \
             authenticate this finalization; refusing it ({e:#})",
            latest.block.height
        )
    })?;
    ensure!(
        latest.finalization.verify(ctx, &scheme, &Sequential),
        "finalization FAILED BLS verification against committee[{epoch}] read at {at_hash:?} \
         (height {}) — refusing it",
        latest.block.height
    );
    Ok(())
}

/// The forward-only EL fast-forward over a target the caller supplies and a
/// caller-supplied `jump_threshold`.
///
/// The target is already committee-authenticated by the caller (the steady-state
/// re-jump reads it from this node's own marshal archive), so this function does
/// not re-check it.
///
/// `jump_threshold` is `min(JUMP_THRESHOLD, epoch_block_interval)` on both
/// production paths, so the jump preempts the ≥2-epoch "committee[E] not
/// committed" defer deadlock at any epoch interval. The deadlock is
/// epoch-relative, so its recovery gate is too.
pub async fn jump_to_target<ES>(
    anchor: u64,
    latest: UpstreamFinalized,
    el: &ES,
    l1_checkpoint: Option<B256>,
    activation: u64,
    jump_threshold: u64,
) -> JumpOutcome
where
    ES: ElSync,
{
    if latest.block.height <= anchor + jump_threshold {
        return JumpOutcome::Lagging;
    }
    let (landing_h, landing_hash) = match el.sync_to(&latest).await {
        Ok(landing) => landing,
        Err(SyncFailure::Invalid(e)) => return JumpOutcome::InvalidTarget(e),
        Err(SyncFailure::StalledWithPeers(e)) => return JumpOutcome::StalledWithPeers(e),
        Err(SyncFailure::Stalled(e)) => return JumpOutcome::Stalled(e),
    };
    if landing_h <= anchor {
        return JumpOutcome::Lagging;
    }
    // `Valid` is reth's verdict on the branch it chose to canonicalize, not a
    // statement that the branch is the attested one: ask whether the EL holds the
    // committee-attested `block.result`. `holds` and not a byte comparison, because
    // `sync_to` short-circuits to the local executed head for a zero result and
    // for a node already executed past the target, where `landing_hash` is a good
    // hash of a different height. The zero case has nothing to ask.
    if latest.block.result != B256::ZERO {
        match el
            .holds(latest.block.result)
            .wrap_err("attested-result probe after EL-sync failed")
        {
            Ok(true) => {}
            Ok(false) => {
                return JumpOutcome::InvalidTarget(eyre!(
                    "EL-sync landed at {landing_h} but the committee-attested result \
                     {:?} of jump target height {} is NOT canonical in the local chain — \
                     the EL did not land on the attested branch",
                    latest.block.result,
                    latest.block.height
                ));
            }
            Err(e) => return JumpOutcome::Stalled(e),
        }
    }
    if let Some(l1) = l1_checkpoint {
        match el
            .holds(l1)
            .wrap_err("L1 checkpoint probe after jump failed")
        {
            Ok(true) => {}
            Ok(false) => {
                return JumpOutcome::L1Fork(eyre!(
                    "L1 Rollup checkpoint {l1:?} is NOT in the local chain after an EL-sync \
                     jump — the synced head does not extend the L1-finalized history (possible \
                     upstream equivocation); refusing to follow"
                ));
            }
            Err(e) => return JumpOutcome::Stalled(e),
        }
    }
    // The K blocks below the landing are covered by the inlet's ordinary pulls;
    // the landing itself is not result-attested yet.
    let floor = landing_h.saturating_sub(K).max(activation);
    info!(
        from = anchor,
        to = landing_h,
        floor,
        "EL-sync jump: fast-forwarded the anchor"
    );
    JumpOutcome::Landed {
        landing: landing_h,
        hash: landing_hash,
        floor,
    }
}

/// Asserts that an L1 Rollup checkpoint hash is canonical in our reth after
/// EL-sync; a missing hash means the synced head does not extend the
/// L1-finalized history (possible upstream equivocation).
///
/// The two log/error strings here are byte-asserted by the local-dpos-smoke
/// `smoke-cert-cascade` case, so they must survive verbatim.
pub fn assert_l1_checkpoint<Provider>(provider: &Provider, l1_hash: B256) -> eyre::Result<()>
where
    Provider: reth_storage_api::BlockReader + Send + Sync,
{
    let num = provider
        .block_number(l1_hash)
        .wrap_err("block_number(l1 checkpoint) probe failed")?;
    match num {
        Some(n) => {
            info!(
                hash = ?l1_hash,
                height = n,
                "cert-follow: L1 Rollup checkpoint verified against local chain"
            );
            Ok(())
        }
        None => Err(eyre!(
            "cert-follow: L1 Rollup checkpoint {l1_hash:?} is NOT in the local \
             chain after EL-sync — the synced head does not extend the \
             L1-finalized history (possible upstream equivocation); refusing to follow"
        )),
    }
}

/// Pin — the prune configuration must keep the hash → number lookup that
/// [`RethElSync::holds`] and [`assert_l1_checkpoint`] read. Neither has a
/// fallback: if the lookup were pruned away, the post-EL-sync L1 trust-root
/// re-assert would read `None` and the node would refuse a chain it is on.
///
/// Nothing in this workspace constructs a `PruneModes`; the profile comes from
/// reth's own `Cli`, so the pin is taken over reth's prune surface itself. The
/// `PruneModes` destructure has no `..`, so a new prunable field stops these tests
/// compiling until it is classified here, and every segment the pruner can be
/// handed is swept against the tables it deletes.
///
/// The per-segment table lists are a hand transcription, so an existing segment
/// that starts deleting `HeaderNumbers` under an unchanged name is not caught.
#[cfg(test)]
mod prune_config_pin {
    use reth_prune_types::{PruneModes, PruneSegment};

    /// The tables the hash → number resolution lives in. `HeaderNumbers` is the one
    /// `holds` reads; the other two are what the retired `PruneSegment::Headers`
    /// took with it, kept so an un-deprecation is caught by table name too.
    const HASH_TO_NUMBER_TABLES: [&str; 3] = ["HeaderNumbers", "CanonicalHeaders", "Headers"];

    /// What each configurable segment deletes, transcribed from reth's own segment
    /// docs. The catch-all is the point: a segment nobody classified is a threat to
    /// the lookup, not waved through.
    fn deleted_tables(segment: PruneSegment) -> &'static [&'static str] {
        match segment {
            PruneSegment::SenderRecovery => &["TransactionSenders"],
            PruneSegment::TransactionLookup => &["TransactionHashNumbers"],
            PruneSegment::Receipts | PruneSegment::ContractLogs => &["Receipts"],
            PruneSegment::AccountHistory => &["AccountChangeSets", "AccountsHistory"],
            PruneSegment::StorageHistory => &["StorageChangeSets", "StoragesHistory"],
            PruneSegment::Bodies => &["Transactions"],
            other => panic!(
                "reth gained prune segment {other:?} and nothing here says which tables it \
                 deletes. `cold_start_jump::holds` resolves its L1 checkpoint through \
                 `HeaderNumbers` (hash -> number) and fails CLOSED when the lookup misses, so \
                 the new segment has to be classified before this pin can pass again."
            ),
        }
    }

    #[test]
    fn no_prune_configuration_can_drop_the_hash_to_number_lookup() {
        // No `..`: a new prunable field breaks this destructure, and the fix is to
        // classify the new segment in `deleted_tables`, never to widen the pattern.
        let PruneModes {
            sender_recovery: _,
            transaction_lookup: _,
            receipts: _,
            account_history: _,
            storage_history: _,
            bodies_history: _,
            receipts_log_filter: _,
        } = PruneModes::all();

        let mut swept = 0;
        for segment in PruneSegment::variants() {
            assert!(
                !format!("{segment:?}").contains("Header"),
                "reth exposes prune segment {segment:?} — a HEADER segment is configurable \
                 again, and `cold_start_jump::holds` resolves its L1 checkpoint through the \
                 header hash -> number lookup"
            );
            for table in deleted_tables(segment) {
                assert!(
                    !HASH_TO_NUMBER_TABLES.contains(table),
                    "prune segment {segment:?} deletes `{table}`, which the hash -> number \
                     resolution `cold_start_jump::holds` and `assert_l1_checkpoint` depend on. \
                     A node on this prune profile would read `None` for a checkpoint it \
                     actually holds and refuse its own chain."
                );
            }
            swept += 1;
        }
        assert_eq!(
            swept, 7,
            "reth's configurable prune surface moved ({swept} segments, was 7) — re-read \
             `crates/prune/types/src/segment.rs` and re-take this pin against the new set"
        );
    }
}

#[cfg(test)]
mod watchdog_tests {
    use super::{
        SyncWatchdog, WatchdogTrip, EL_SYNC_BACKSTOP_CEILING, EL_SYNC_NO_PEERS_GRACE,
        EL_SYNC_STALL_ESCAPE, EL_SYNC_TICK,
    };

    const MAX_TICKS: u64 = EL_SYNC_BACKSTOP_CEILING.as_secs() / EL_SYNC_TICK.as_secs();
    const GRACE_TICKS: u64 = EL_SYNC_NO_PEERS_GRACE.as_secs() / EL_SYNC_TICK.as_secs();
    const STALL_TICKS: u64 = EL_SYNC_STALL_ESCAPE.as_secs() / EL_SYNC_TICK.as_secs();

    #[test]
    fn backstop_trips_at_ceiling_and_not_before() {
        let mut w = SyncWatchdog::new();
        for i in 0..(MAX_TICKS - 1) {
            assert_eq!(w.on_tick(3, i), None);
        }
        assert_eq!(w.on_tick(3, MAX_TICKS - 1), Some(WatchdogTrip::Ceiling));
    }

    #[test]
    fn no_peers_trips_at_grace_and_resets_on_connect() {
        let mut w = SyncWatchdog::new();
        for _ in 0..(GRACE_TICKS - 1) {
            assert_eq!(w.on_tick(0, 42), None);
        }
        assert_eq!(w.on_tick(0, 42), Some(WatchdogTrip::NoPeers));

        let mut w = SyncWatchdog::new();
        for _ in 0..(GRACE_TICKS - 1) {
            w.on_tick(0, 42);
        }
        assert_eq!(
            w.on_tick(1, 42),
            None,
            "a connected tick resets the no-peers streak"
        );
        for _ in 0..(GRACE_TICKS - 1) {
            assert_eq!(w.on_tick(0, 42), None);
        }
        assert_eq!(w.on_tick(0, 42), Some(WatchdogTrip::NoPeers));
    }

    #[test]
    fn interleaved_zero_peer_windows_never_accumulate() {
        let mut w = SyncWatchdog::new();
        for i in 0..(GRACE_TICKS * 4) {
            assert_eq!(w.on_tick(0, i * 2), None);
            assert_eq!(w.on_tick(2, i * 2 + 1), None);
        }
    }

    #[test]
    fn stall_with_peers_trips_when_head_frozen_and_not_before() {
        let mut w = SyncWatchdog::new();
        // The first tick only sets the baseline, so the trip lands one tick after
        // STALL_TICKS.
        for _ in 0..STALL_TICKS {
            assert_eq!(w.on_tick(2, 42), None);
        }
        assert_eq!(w.on_tick(2, 42), Some(WatchdogTrip::StalledWithPeers));
    }

    #[test]
    fn head_progress_resets_the_stall_streak() {
        let mut w = SyncWatchdog::new();
        for _ in 0..STALL_TICKS {
            w.on_tick(2, 42);
        }
        // One forward move resets the streak; a fresh full window is then needed.
        assert_eq!(
            w.on_tick(2, 43),
            None,
            "a head advance resets the stall streak"
        );
        for _ in 0..(STALL_TICKS - 1) {
            assert_eq!(w.on_tick(2, 43), None);
        }
        assert_eq!(w.on_tick(2, 43), Some(WatchdogTrip::StalledWithPeers));
    }

    #[test]
    fn stall_streak_requires_peers() {
        let mut w = SyncWatchdog::new();
        // Head frozen but zero peers: kept under the no-peers grace so nothing trips
        // at all, isolating the stall arm from the no-peers arm.
        for _ in 0..(GRACE_TICKS - 1) {
            assert_eq!(w.on_tick(0, 42), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{digest::Digest, order_block::OrderBlock};
    use alloy_primitives::Bytes;
    use commonware_codec::DecodeExt as _;
    use commonware_consensus::{
        simplex::types::{Finalization, Finalize, Proposal},
        types::{Epoch, Round, View},
    };
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
    use commonware_math::algebra::Random as _;
    use commonware_runtime::{deterministic, Runner as _};
    use commonware_utils::{ordered::BiMap, TryCollect as _};
    use fluentbase_bls::oracle::SeedOracle;
    use fluentbase_bls::{
        fluent_namespace, keys::ValidatorBlsKeypair, scheme::build_signer, BlsPubkey, PeerPubkey,
        Scheme as BlsScheme,
    };
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;
    use std::sync::{Arc, Mutex};

    const CHAIN_ID: u64 = 20_994;
    const COMMITTEE_N: usize = 4;
    const ACTIVATION: u64 = 64;
    const ANCHOR_HASH: B256 = B256::repeat_byte(0xc0);

    struct Committee {
        signers: Vec<BlsScheme>,
        verifier: BlsScheme,
    }

    /// Every fixture certifies at epoch 0 (see `certify`), so the schemes are bound
    /// to it.
    fn committee(seed: u64) -> Committee {
        let mut rng = StdRng::seed_from_u64(seed);
        let peer_sks: Vec<_> = (0..COMMITTEE_N)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let bls_kps: Vec<_> = (0..COMMITTEE_N)
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
        let ns = fluent_namespace(CHAIN_ID);
        let signers = bls_kps
            .iter()
            .map(|kp| build_signer(&ns, bimap.clone(), kp, 0, None).expect("member"))
            .collect();
        let verifier = fluentbase_bls::scheme::build_verifier(&ns, bimap, 0, None);
        Committee { signers, verifier }
    }

    fn sample_order(parent: Digest, height: u64, result: B256) -> OrderBlock {
        OrderBlock {
            parent,
            height,
            proposal_view: 0,
            timestamp: 1_700_000_000 + height,
            gas_limit: 30_000_000,
            extra_data: Bytes::new(),
            result,
            txs: Vec::new(),
            equivocation: None,
        }
    }

    /// 2f+1 finalize votes over the block's digest, producing a real finalization
    /// cert.
    fn certify(c: &Committee, epoch: u64, block: &OrderBlock) -> UpstreamFinalized {
        let round = Round::new(Epoch::new(epoch), View::new(block.height));
        let prop = Proposal::new(round, View::new(block.height - 1), block.digest());
        let finalizes: Vec<_> = c
            .signers
            .iter()
            .take(3)
            .map(|s| Finalize::sign(s, prop.clone()).expect("sign"))
            .collect();
        let finalization = Finalization::from_finalizes(&c.verifier, finalizes.iter(), &Sequential)
            .expect("quorum");
        UpstreamFinalized {
            finalization,
            block: block.clone(),
        }
    }

    struct CannedCommittees {
        verifier: BlsScheme,
        reads: Arc<Mutex<Vec<(u64, B256)>>>,
        /// `false` makes `scheme_at` return `Err` (committee unreadable even at the
        /// synced landing).
        readable: bool,
    }

    impl CommitteeSource for CannedCommittees {
        fn scheme_at(
            &self,
            epoch: u64,
            at_hash: B256,
            _oracle: Option<Arc<dyn SeedOracle>>,
        ) -> eyre::Result<BlsScheme> {
            self.reads.lock().unwrap().push((epoch, at_hash));
            if !self.readable {
                return Err(eyre!(
                    "epoch {epoch} has no committed committee at {at_hash}"
                ));
            }
            Ok(self.verifier.clone())
        }
    }

    struct FakeElSync {
        landing: (u64, B256),
        calls: Arc<Mutex<u32>>,
        holds_l1: bool,
        /// The attested `block.result` this EL canonicalized, recorded by `sync_to`.
        landed_on: Arc<Mutex<Option<B256>>>,
        /// Makes `sync_to` report success without landing on the attested result,
        /// exercising the landing check.
        refuse_result: Arc<Mutex<bool>>,
    }

    impl ElSync for FakeElSync {
        async fn sync_to_checkpoint(&self, _checkpoint: B256) -> Result<(u64, B256), SyncFailure> {
            unreachable!("the jump never takes the operator-checkpoint entry")
        }
        async fn sync_to(&self, latest: &UpstreamFinalized) -> Result<(u64, B256), SyncFailure> {
            *self.calls.lock().unwrap() += 1;
            if !*self.refuse_result.lock().unwrap() {
                *self.landed_on.lock().unwrap() = Some(latest.block.result);
            }
            Ok(self.landing)
        }
        fn holds(&self, hash: B256) -> eyre::Result<bool> {
            if *self.landed_on.lock().unwrap() == Some(hash) {
                return Ok(true);
            }
            // Anything else is the L1 checkpoint probe.
            Ok(self.holds_l1)
        }
    }

    struct Fixture {
        committee: Committee,
        el: FakeElSync,
        scheme_reads: Arc<Mutex<Vec<(u64, B256)>>>,
        el_sync_calls: Arc<Mutex<u32>>,
    }

    fn fixture(landing: (u64, B256), holds_l1: bool) -> (CannedCommittees, Fixture) {
        fixture_with_committee(landing, holds_l1, true)
    }

    fn fixture_with_committee(
        landing: (u64, B256),
        holds_l1: bool,
        committee_readable: bool,
    ) -> (CannedCommittees, Fixture) {
        let committee = committee(1);
        let scheme_reads = Arc::new(Mutex::new(Vec::new()));
        let el_sync_calls = Arc::new(Mutex::new(0));
        let committees = CannedCommittees {
            verifier: committee.verifier.clone(),
            reads: scheme_reads.clone(),
            readable: committee_readable,
        };
        let fx = Fixture {
            committee,
            el: FakeElSync {
                landing,
                calls: el_sync_calls.clone(),
                holds_l1,
                landed_on: Arc::new(Mutex::new(None)),
                refuse_result: Arc::new(Mutex::new(false)),
            },
            scheme_reads,
            el_sync_calls,
        };
        (committees, fx)
    }

    /// A deep gap above [`JUMP_THRESHOLD`] drives the EL-sync once and re-seeds the
    /// anchor + floor at the landing (`floor == landing − K`), reading no committee.
    #[test]
    fn jump_triggers_el_sync_and_reseeds_anchor() {
        let runtime = deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let landing = (5000, B256::repeat_byte(0xe1));
            let (_committees, fx) = fixture(landing, true);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            let out = jump_to_target(
                ACTIVATION,
                certify(&fx.committee, 0, &live),
                &fx.el,
                None,
                ACTIVATION,
                JUMP_THRESHOLD,
            )
            .await;
            let JumpOutcome::Landed {
                landing,
                hash,
                floor,
            } = out
            else {
                panic!("expected Landed, got a different JumpOutcome");
            };
            assert_eq!(landing, 5000, "anchor height = landing");
            assert_eq!(hash, B256::repeat_byte(0xe1), "anchor hash = landing hash");
            assert_eq!(
                floor,
                5000 - K,
                "marshal floor = landing − K (the landing is not result-attested yet)"
            );
            assert_eq!(*fx.el_sync_calls.lock().unwrap(), 1, "el_sync ran once");
            assert!(
                fx.scheme_reads.lock().unwrap().is_empty(),
                "the jump read a committee — the POST-sync authentication stage is back"
            );
        });
    }

    /// The landing check: `sync_to` returned `Valid`, but reth's `Valid` is a verdict
    /// on the branch it chose, not on the attested one. If the EL does not hold the
    /// target's attested `block.result`, the outcome is `InvalidTarget`; if it does,
    /// the jump lands.
    #[test]
    fn a_landing_off_the_attested_result_is_invalid_and_one_on_it_lands() {
        let runtime = deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let target_of = |fx: &Fixture| {
                let live = sample_order(
                    Digest(B256::repeat_byte(0xaa)),
                    far,
                    B256::repeat_byte(0x44),
                );
                certify(&fx.committee, 0, &live)
            };

            let (_committees, fx) = fixture((5000, B256::repeat_byte(0xe1)), false);
            *fx.el.refuse_result.lock().unwrap() = true;
            let out = jump_to_target(
                ACTIVATION,
                target_of(&fx),
                &fx.el,
                None,
                ACTIVATION,
                JUMP_THRESHOLD,
            )
            .await;
            assert_eq!(
                *fx.el_sync_calls.lock().unwrap(),
                1,
                "the EL-sync never ran — the check below is vacuous"
            );
            let JumpOutcome::InvalidTarget(e) = out else {
                panic!("a landing off the attested branch was accepted; expected InvalidTarget");
            };
            assert!(
                format!("{e:#}").contains("NOT canonical"),
                "the refusal is not the landing check: {e:#}"
            );

            // Same fixture, same target; the EL lands where it was told.
            let (_committees, fx) = fixture((5000, B256::repeat_byte(0xe1)), false);
            let out = jump_to_target(
                ACTIVATION,
                target_of(&fx),
                &fx.el,
                None,
                ACTIVATION,
                JUMP_THRESHOLD,
            )
            .await;
            assert_eq!(*fx.el_sync_calls.lock().unwrap(), 1, "el_sync ran once");
            assert!(
                matches!(out, JumpOutcome::Landed { landing: 5000, .. }),
                "an EL that landed on the attested result did not Land"
            );
        });
    }

    /// A target inside the pre-K window carries `result == B256::ZERO`: no derived
    /// hash exists, so the landing check has nothing to ask and must not fire.
    #[test]
    fn a_pre_k_target_with_no_derived_result_skips_the_landing_check() {
        let runtime = deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (_committees, fx) = fixture((5000, B256::repeat_byte(0xe1)), false);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            *fx.el.refuse_result.lock().unwrap() = true;
            let live = sample_order(Digest(B256::repeat_byte(0xaa)), far, B256::ZERO);
            let out = jump_to_target(
                ACTIVATION,
                certify(&fx.committee, 0, &live),
                &fx.el,
                None,
                ACTIVATION,
                JUMP_THRESHOLD,
            )
            .await;
            assert!(
                matches!(out, JumpOutcome::Landed { landing: 5000, .. }),
                "a pre-K target (result = ZERO) was refused by the landing check"
            );
        });
    }

    /// A target within [`JUMP_THRESHOLD`] of the anchor is a no-op — the
    /// residual gap is the inlet's ordinary-pull job; the EL-sync never runs.
    #[test]
    fn shallow_gap_does_not_jump() {
        let runtime = deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (_committees, fx) = fixture((5000, B256::repeat_byte(0xe1)), true);
            let near = ACTIVATION + 8; // well below JUMP_THRESHOLD
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                near,
                B256::repeat_byte(0x44),
            );
            let out = jump_to_target(
                ACTIVATION,
                certify(&fx.committee, 0, &live),
                &fx.el,
                None,
                ACTIVATION,
                JUMP_THRESHOLD,
            )
            .await;
            assert!(
                matches!(out, JumpOutcome::Lagging),
                "shallow gap must not jump"
            );
            assert_eq!(*fx.el_sync_calls.lock().unwrap(), 0, "el_sync never ran");
        });
    }

    /// With an L1 checkpoint configured and the synced head not descending from
    /// it, the post-jump `holds()` probe answers false and the jump is an
    /// [`JumpOutcome::L1Fork`].
    #[test]
    fn jump_reasserts_l1_checkpoint_and_fails_closed() {
        let runtime = deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (_committees, fx) = fixture((5000, B256::repeat_byte(0xe1)), false);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            let out = jump_to_target(
                ACTIVATION,
                certify(&fx.committee, 0, &live),
                &fx.el,
                Some(B256::repeat_byte(0x1a)),
                ACTIVATION,
                JUMP_THRESHOLD,
            )
            .await;
            let JumpOutcome::L1Fork(err) = out else {
                panic!("expected L1Fork (L1 holds()==false), got a different JumpOutcome");
            };
            assert!(err.to_string().contains("NOT in the local chain"), "{err}");
        });
    }

    /// A target whose multisig was built by a different committee (structurally
    /// intact — `payload == digest` — but cryptographically worthless) still lands,
    /// because a real jump's target comes out of this node's own marshal archive or
    /// through `FrontierHandler::deliver` and is never handed to the jump
    /// unauthenticated.
    #[test]
    fn a_target_with_a_broken_multisig_lands_when_the_el_holds_the_attested_result() {
        let runtime = deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let landing = (far, B256::repeat_byte(0x55));
            let (_committees, fx) = fixture(landing, true);
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            // An independent committee signs the target: `payload == digest` holds,
            // but the multisig does not verify against the fixture's committee.
            let other = committee(2);
            let out = jump_to_target(
                ACTIVATION,
                certify(&other, 0, &live),
                &fx.el,
                None,
                ACTIVATION,
                JUMP_THRESHOLD,
            )
            .await;
            assert!(
                matches!(out, JumpOutcome::Landed { .. }),
                "the jump refused a target on its signatures — a verify stage survived"
            );
            assert_eq!(*fx.el_sync_calls.lock().unwrap(), 1, "el_sync ran once");
            assert!(
                fx.scheme_reads.lock().unwrap().is_empty(),
                "the jump read a committee — the POST-sync authentication stage is back"
            );
        });
    }

    /// A structurally broken target (`cert.payload != block.digest()`) is not refused
    /// by the jump either; the EL is driven and the jump lands.
    #[test]
    fn a_structurally_broken_target_no_longer_stops_the_jump_before_sync() {
        let runtime = deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let (_committees, fx) = fixture((far, B256::repeat_byte(0x55)), true);
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            let mut forged = certify(&fx.committee, 0, &live);
            // The cert signs a different block than the one served.
            forged.block = sample_order(
                Digest(B256::repeat_byte(0xab)),
                far,
                B256::repeat_byte(0x44),
            );
            let out =
                jump_to_target(ACTIVATION, forged, &fx.el, None, ACTIVATION, JUMP_THRESHOLD).await;
            assert!(
                matches!(out, JumpOutcome::Landed { .. }),
                "the jump refused a target on its structure — the PRE-sync stage survived"
            );
            assert_eq!(
                *fx.el_sync_calls.lock().unwrap(),
                1,
                "the EL was never driven — the PRE-sync structural gate is back"
            );
        });
    }

    /// An L1 `holds()` probe that errors is [`JumpOutcome::Stalled`], not a fork
    /// verdict: a transport failure says nothing about ancestry.
    #[test]
    fn an_l1_probe_error_is_stalled_not_a_fork_verdict() {
        /// Lands on the attested result, then fails the L1 probe with a transport
        /// error.
        struct L1ProbeErrorElSync {
            landing: (u64, B256),
            landed_on: Arc<Mutex<Option<B256>>>,
        }
        impl ElSync for L1ProbeErrorElSync {
            async fn sync_to_checkpoint(
                &self,
                _checkpoint: B256,
            ) -> Result<(u64, B256), SyncFailure> {
                unreachable!("the jump never takes the operator-checkpoint entry")
            }
            async fn sync_to(
                &self,
                latest: &UpstreamFinalized,
            ) -> Result<(u64, B256), SyncFailure> {
                *self.landed_on.lock().unwrap() = Some(latest.block.result);
                Ok(self.landing)
            }
            fn holds(&self, hash: B256) -> eyre::Result<bool> {
                if *self.landed_on.lock().unwrap() == Some(hash) {
                    return Ok(true);
                }
                Err(eyre!("l1 rpc unreachable"))
            }
        }
        let runtime = deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let c = committee(1);
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            let el = L1ProbeErrorElSync {
                landing: (far, B256::repeat_byte(0x55)),
                landed_on: Arc::new(Mutex::new(None)),
            };
            let out = jump_to_target(
                ACTIVATION,
                certify(&c, 0, &live),
                &el,
                Some(B256::repeat_byte(0x1a)),
                ACTIVATION,
                JUMP_THRESHOLD,
            )
            .await;
            let JumpOutcome::Stalled(err) = out else {
                panic!("an L1 probe ERROR must be Stalled, not L1Fork/Landed");
            };
            assert!(
                format!("{err:#}").contains("L1 checkpoint probe after jump failed"),
                "the stall is not the L1 probe: {err:#}"
            );
        });
    }

    /// A landing that does not advance the resolved anchor (a stale target or an
    /// upstream reorg) must not reseed backward.
    #[test]
    fn stale_jump_landing_does_not_reseed_backward() {
        let runtime = deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (_committees, fx) = fixture((ACTIVATION, ANCHOR_HASH), true);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            let out = jump_to_target(
                ACTIVATION,
                certify(&fx.committee, 0, &live),
                &fx.el,
                None,
                ACTIVATION,
                JUMP_THRESHOLD,
            )
            .await;
            assert!(
                matches!(out, JumpOutcome::Lagging),
                "stale landing must not reseed backward"
            );
            assert_eq!(
                *fx.el_sync_calls.lock().unwrap(),
                1,
                "el_sync ran but landing dropped"
            );
        });
    }

    /// A `sync_to` transport stall is classified `Stalled` (non-fatal): the executor
    /// keeps the loop running and retries on the next `Update::Tip`.
    #[test]
    fn sync_to_stall_is_classified_stalled() {
        struct StallingElSync;
        impl ElSync for StallingElSync {
            async fn sync_to_checkpoint(
                &self,
                _checkpoint: B256,
            ) -> Result<(u64, B256), SyncFailure> {
                unreachable!("the jump never takes the operator-checkpoint entry")
            }
            async fn sync_to(
                &self,
                _latest: &UpstreamFinalized,
            ) -> Result<(u64, B256), SyncFailure> {
                Err(SyncFailure::Stalled(eyre!(
                    "reth EL-sync stalled for 120s at height 0"
                )))
            }
            fn holds(&self, _hash: B256) -> eyre::Result<bool> {
                Ok(true)
            }
        }
        let runtime = deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let c = committee(1);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            let out = jump_to_target(
                ACTIVATION,
                certify(&c, 0, &live),
                &StallingElSync,
                None,
                ACTIVATION,
                JUMP_THRESHOLD,
            )
            .await;
            let JumpOutcome::Stalled(err) = out else {
                panic!("a sync_to transport error must be Stalled, not InvalidTarget/Landed");
            };
            assert!(err.to_string().contains("stalled"), "{err}");
        });
    }

    /// A `sync_to` failure where reth declared the served branch `Invalid` is
    /// classified `InvalidTarget`, distinct from `Stalled`'s "no verdict at all".
    #[test]
    fn sync_to_invalid_branch_is_classified_invalid_target() {
        struct InvalidBranchElSync;
        impl ElSync for InvalidBranchElSync {
            async fn sync_to_checkpoint(
                &self,
                _checkpoint: B256,
            ) -> Result<(u64, B256), SyncFailure> {
                unreachable!("the jump never takes the operator-checkpoint entry")
            }
            async fn sync_to(
                &self,
                _latest: &UpstreamFinalized,
            ) -> Result<(u64, B256), SyncFailure> {
                Err(SyncFailure::Invalid(eyre!(
                    "reth REJECTED the attested tip as INVALID during EL-sync"
                )))
            }
            fn holds(&self, _hash: B256) -> eyre::Result<bool> {
                Ok(true)
            }
        }
        let runtime = deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let c = committee(1);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            let out = jump_to_target(
                ACTIVATION,
                certify(&c, 0, &live),
                &InvalidBranchElSync,
                None,
                ACTIVATION,
                JUMP_THRESHOLD,
            )
            .await;
            let JumpOutcome::InvalidTarget(err) = out else {
                panic!("an Invalid sync_to verdict must be InvalidTarget, not Stalled/Landed");
            };
            assert!(err.to_string().contains("REJECTED"), "{err}");
        });
    }

    /// A `sync_to` failure where reth was connected but its executed head stayed
    /// frozen is classified as the separate [`JumpOutcome::StalledWithPeers`].
    #[test]
    fn sync_to_stalled_with_peers_is_classified_stalled_with_peers() {
        struct WedgedElSync;
        impl ElSync for WedgedElSync {
            async fn sync_to_checkpoint(
                &self,
                _checkpoint: B256,
            ) -> Result<(u64, B256), SyncFailure> {
                unreachable!("the jump never takes the operator-checkpoint entry")
            }
            async fn sync_to(
                &self,
                _latest: &UpstreamFinalized,
            ) -> Result<(u64, B256), SyncFailure> {
                Err(SyncFailure::StalledWithPeers(eyre!(
                    "reth CONNECTED (2 peers) but executed head frozen at 360 for 300s — EL \
                     pipeline wedged"
                )))
            }
            fn holds(&self, _hash: B256) -> eyre::Result<bool> {
                Ok(true)
            }
        }
        let runtime = deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let c = committee(1);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            let out = jump_to_target(
                ACTIVATION,
                certify(&c, 0, &live),
                &WedgedElSync,
                None,
                ACTIVATION,
                JUMP_THRESHOLD,
            )
            .await;
            let JumpOutcome::StalledWithPeers(err) = out else {
                panic!(
                    "a connected-but-wedged sync_to verdict must be StalledWithPeers, not \
                     Stalled/InvalidTarget/Landed"
                );
            };
            assert!(err.to_string().contains("wedged"), "{err}");
        });
    }

    use crate::sync_metrics::SyncMetrics;

    /// By-height upstream for the boundary-seeding seam: serves exactly what it is
    /// given, so a test can model a wrong-height response and an absent height.
    #[derive(Clone, Default)]
    struct ByHeightUpstream {
        served: Arc<Mutex<std::collections::BTreeMap<u64, UpstreamFinalized>>>,
    }

    impl crate::cert_follow::CertUpstream for ByHeightUpstream {
        async fn get_finalization(
            &self,
            height: commonware_consensus::types::Height,
        ) -> Option<UpstreamFinalized> {
            self.served.lock().unwrap().get(&height.get()).cloned()
        }
        async fn get_latest(&self) -> Option<UpstreamFinalized> {
            None
        }
        async fn rotate(&self) {}
    }

    fn seeding_committees(c: &Committee, readable: bool) -> CannedCommittees {
        CannedCommittees {
            verifier: c.verifier.clone(),
            reads: Arc::new(Mutex::new(Vec::new())),
            readable,
        }
    }

    #[test]
    fn boundary_fetch_authenticates_and_pins_height() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let c = committee(21);
            let block = sample_order(Digest(B256::ZERO), 899, B256::ZERO);
            let up = ByHeightUpstream::default();
            up.served
                .lock()
                .unwrap()
                .insert(899, certify(&c, 0, &block));
            let metrics = SyncMetrics::default();

            let got = crate::cert_follow::fetch_verified_boundary(
                &up,
                &seeding_committees(&c, true),
                &mut ctx,
                &metrics,
                B256::repeat_byte(0x11),
                899,
            )
            .await;

            assert_eq!(got.map(|uf| uf.block.height), Some(899));
            // A fetch alone injects nothing, so the counter stays flat; it is bumped at
            // the injection sites.
            assert_eq!(metrics.jump_boundary_refetched.get(), 0);
            assert_eq!(metrics.jump_boundary_refetch_failed.get(), 0);
        });
    }

    /// The height pin. Nothing else binds the response to the request: the structural
    /// check ties the cert only to the block it arrived with, and the epoch comes from
    /// the cert's own round — so a valid finalization for a different height would be
    /// stored under the index we asked for.
    #[test]
    fn wrong_height_response_is_rejected() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let c = committee(22);
            let other = sample_order(Digest(B256::ZERO), 900, B256::ZERO);
            let up = ByHeightUpstream::default();
            up.served
                .lock()
                .unwrap()
                .insert(899, certify(&c, 0, &other));
            let metrics = SyncMetrics::default();

            let got = crate::cert_follow::fetch_verified_boundary(
                &up,
                &seeding_committees(&c, true),
                &mut ctx,
                &metrics,
                B256::repeat_byte(0x22),
                899,
            )
            .await;

            assert!(
                got.is_none(),
                "a valid cert for the wrong height must be refused"
            );
            assert_eq!(metrics.jump_boundary_refetch_failed.get(), 1);
        });
    }

    /// An unreadable committee is refused unconditionally, both from the direct call
    /// and through the boundary seam.
    #[test]
    fn an_unreadable_committee_is_refused() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let c = committee(23);
            let block = sample_order(Digest(B256::ZERO), 899, B256::ZERO);
            let uf = certify(&c, 0, &block);
            let up = ByHeightUpstream::default();
            up.served.lock().unwrap().insert(899, uf.clone());
            let metrics = SyncMetrics::default();
            let unreadable = seeding_committees(&c, false);

            let err =
                verify_jump_authenticated(&uf, &unreadable, B256::repeat_byte(0x33), &mut ctx)
                    .expect_err("an unreadable committee must refuse, unconditionally");
            assert!(
                format!("{err:#}").contains("is unreadable at"),
                "the refusal is not the unreadable-committee arm: {err:#}"
            );

            let got = crate::cert_follow::fetch_verified_boundary(
                &up,
                &unreadable,
                &mut ctx,
                &metrics,
                B256::repeat_byte(0x33),
                899,
            )
            .await;

            assert!(
                got.is_none(),
                "boundary seeding must fail closed on an unreadable committee"
            );
            assert_eq!(metrics.jump_boundary_refetch_failed.get(), 1);
        });
    }

    #[test]
    fn missing_upstream_height_yields_none() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let c = committee(24);
            let metrics = SyncMetrics::default();

            let got = crate::cert_follow::fetch_verified_boundary(
                &ByHeightUpstream::default(),
                &seeding_committees(&c, true),
                &mut ctx,
                &metrics,
                B256::repeat_byte(0x44),
                899,
            )
            .await;

            assert!(got.is_none());
            assert_eq!(metrics.jump_boundary_refetch_failed.get(), 1);
        });
    }
}
