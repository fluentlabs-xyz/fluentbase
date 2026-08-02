//! Single-shot, pre-engine EL-sync JUMP — the deep cold-start fast-forward.
//!
//! A fresh / deeply-behind node WITH AN UPSTREAM needs a deep catch-up as a
//! cold-start prep step — run AFTER the discriminator resolves the anchor and
//! BEFORE the OuterEngine/executor task starts, so the inlet+marshal then close
//! the residual gap by ordinary live pulls. The JUMP issues ONE forkchoice
//! (read-side fast-forward) toward the committee-attested derived tip and lets
//! reth's devp2p backfill canonicalize it, then re-seeds the cold-start anchor
//! at the landing.
//!
//! **Single-writer safety.** [`cold_start_jump`] runs strictly before
//! `OuterBuilder::build` (and thus before the executor task starts), the SAME
//! mutual-exclusion property [`crate::dpos`]'s `recover_finalized_tail_into_reth`
//! relies on — there is exactly one writer touching reth at this point. It is a
//! cold-start prep path, NOT a long-lived second writer.
//!
//! **Forward-only.** The jump only moves the anchor/finalized FORWARD (a
//! landing that does not advance the resolved anchor is dropped) — reth
//! ancestor-skips a backward FCU, so a backward jump is both useless and unsafe.
//!
//! **Authenticated (fail-closed).** Two gates protect the jump:
//!
//!   1. [`verify_jump_structural`] runs BEFORE `sync_to` and rejects a
//!      structurally-broken target (`cert.payload != block.digest()`) so reth's
//!      devp2p backfill is never aimed at a tip whose served body does not match
//!      its cert.
//!   2. [`verify_jump_authenticated`] runs AFTER `sync_to` lands — at which point
//!      we DO hold the target state, so we read `committee[E]` at the landing
//!      hash and BLS-verify the finalization against it. A mismatch FAILS CLOSED
//!      (the launcher's `?` aborts the launch: the node refuses to participate on
//!      an unauthenticated branch rather than signing/serving it).
//!
//! This is the structural dual of the post-jump L1 `holds()` re-assert: both run
//! AFTER `sync_to` because the thing they check (the target's committee / the
//! L1-finalized ancestor) only becomes locally readable once reth has been driven
//! onto the branch. A pre-`sync_to` committee read is the chicken-and-egg trap —
//! `committee[far_epoch]` is NOT committed in the state at the stale resolved
//! anchor, so it was structurally unreachable exactly in the deep-catch-up case
//! it exists to protect. Reading it post-sync at the landing closes that hole
//! WITHOUT requiring an L1 checkpoint (it is the trustless default).
//!
//! The committee read is sound at the landing because committees are
//! ahead-committed at epoch-(E-1)-start and the landing (`tip − K`, K=3) shares
//! the tip's epoch (interval ≥ 32 ≫ K) — so `committee[E_tip]` is committed at
//! the landing state. So a malicious far-ahead upstream cannot steer the EL onto
//! an unagreed branch: a forged cert fails BLS against the genuine committee read
//! at the synced state.

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

/// One L1 batch = 1024 blocks = ~17 min of chain time at the committed
/// 1 blk/s. Above this gap the cold-start re-runs the EL-sync phase instead of
/// resuming block-by-block — batched pipeline sync avoids the per-height RPC
/// round-trip + per-block engine-API cost. Also the serving-window cap (a
/// downstream follower's repairable gap and its own jump threshold coincide
/// by construction).
pub const JUMP_THRESHOLD: u64 = 1024;

/// Terminal classification of a (cold-start or steady-state) jump attempt. The
/// steady-state spawn owns the entire backfill wait, so there is no in-progress
/// variant — `cold_start_jump` only ever returns a terminal outcome.
pub enum JumpOutcome {
    /// A jump landed: re-seed the anchor + finalized cursor at `(landing, hash)`
    /// and advance the running marshal floor to `floor` (= landing − K).
    Landed {
        landing: u64,
        hash: B256,
        floor: u64,
    },
    /// Shallow gap / stale-or-backward landing / no `get_latest` — no-op (the
    /// inlet's ordinary pulls still cover the residual gap).
    Lagging,
    /// Transport/timeout `sync_to` failure (zero-peers grace or the absolute
    /// ceiling tripped, or a generic transport error) — NON-fatal: the
    /// steady-state caller retries on the next `Update::Tip` (the marshal inlet
    /// keeps storing frontier certs while contiguous dispatch is stalled). The
    /// cold-start single-shot caller deliberately re-fuses this to fatal (see
    /// [`crate::dpos::jump_landing_or_abort`]). A genuinely-INVALID served
    /// branch is its own [`InvalidTarget`](JumpOutcome::InvalidTarget) variant,
    /// not folded in here — reth actually rendered a verdict on the branch,
    /// which is a different (and more actionable) condition than "no verdict
    /// yet".
    Stalled(eyre::Report),
    /// `verify_jump_structural` rejected the target (`cert.payload !=
    /// block.digest()`) — a SIGNATURE-FREE, attacker-controlled structural mismatch
    /// served PRE-anchor (`el_sync_calls == 0`, tested). NON-fatal (Rule S): the
    /// steady-state caller `rotate()`s to the next upstream URL (Phase 2); the
    /// cold-start caller boots anyway (`jump_landing_or_abort` → `Ok(None)`).
    /// Distinct from `AuthFailed`, which is reserved for POST-sync
    /// committee-BLS / L1 `holds()` rejection (genuine equivocation on a block
    /// that canonicalized). Distinct from [`InvalidTarget`](JumpOutcome::InvalidTarget),
    /// which is the analogous POST-`sync_to`-attempt case (`el_sync_calls >= 1`)
    /// — kept as a separate variant precisely so this invariant stays pinned.
    BadTarget(eyre::Report),
    /// `sync_to` observed reth itself declare the served branch
    /// `PayloadStatusEnum::Invalid` during the EL-sync attempt
    /// (`el_sync_calls >= 1` — `verify_jump_structural`'s PRE-sync digest
    /// check already passed; reth is the one rejecting the served body/state
    /// once backfill validates it). Same NON-fatal treatment as
    /// [`BadTarget`](JumpOutcome::BadTarget):
    /// the steady-state caller rotates immediately, the cold-start caller boots
    /// anyway. Kept as a SEPARATE variant rather than reusing `BadTarget`
    /// because `BadTarget`'s own doc/tests pin `el_sync_calls == 0` — folding
    /// this in would silently break that invariant. Distinct from `Stalled`:
    /// reth rendered an actual verdict here, it did not merely time out.
    InvalidTarget(eyre::Report),
    /// `sync_to` tripped the connected-but-no-progress net
    /// ([`SyncFailure::StalledWithPeers`]): reth had peers > 0 yet its executed
    /// head did not advance for [`EL_SYNC_STALL_ESCAPE`] — the soak-v43 wedge
    /// (a pipeline unwound to a bad ancestor and answers `SYNCING` forever). NON-fatal
    /// and DISTINCT from [`Stalled`](JumpOutcome::Stalled): the divergence root cause
    /// is unknown + deterministic, so a re-jump re-wedges; the steady-state caller
    /// RE-ARMS on the next tip (bumping a counter + ERROR-logging each attempt) and
    /// keeps the refill DEFERRED, so the node stays observable instead of silently
    /// stuck. The cold-start single-shot caller retries forever (same as `Stalled`).
    StalledWithPeers(eyre::Report),
    /// `verify_jump_authenticated` (POST-sync committee BLS) rejected the target
    /// — a forged / unagreed branch that canonicalized. NON-fatal #1 self-heal:
    /// the steady-state caller ROTATES the upstream (a forged UPSTREAM, not a
    /// forked chain — a different honest source recovers). The PRE-sync structural
    /// mismatch is the NON-fatal [`BadTarget`](JumpOutcome::BadTarget) instead.
    AuthFailed(eyre::Report),
    /// The POST-sync L1 `holds()` re-assert failed: the EL-synced head does NOT
    /// descend from the L1-FINALIZED checkpoint (#10 — an L1 fork, distinct from a
    /// forged upstream). At runtime this is a Phase-3 [`SafetyHalt`]: the synced
    /// chain conflicts with L1 finality, the strongest trust root, so the node
    /// HALTS (verify-only, stops driving reth, stays observable) rather than
    /// rotating (there is no honest upstream to rotate to — L1 finality itself
    /// disagrees) and rather than exiting. Split out of `AuthFailed` precisely so
    /// the L1-fork verdict routes to the halt while a committee-BLS failure keeps
    /// rotating.
    ///
    /// [`SafetyHalt`]: crate::sync_metrics::SafetyHalt
    L1Fork(eyre::Report),
}

/// EL-sync loop tick: each [`RethElSync::sync_to`] iteration sleeps this, so tick
/// counts bound elapsed time MONOTONICALLY without reading a clock (bug 8 —
/// commonware's tokio `Clock::current` is `SystemTime`; an NTP step must neither
/// fire the nets spuriously nor silently disable them, both of which a wall-clock
/// deadline does on a forward/backward step).
pub(crate) const EL_SYNC_TICK: Duration = Duration::from_secs(2);

/// SECONDARY net for the EL-sync wait — a pure BACKSTOP (bug 5), tick-counted and
/// deliberately generous: it is NOT a sync-duration estimate. Healthy completion —
/// at ANY depth/duration — exits via the FCU-`PayloadStatus::Valid` terminator in
/// [`RethElSync::sync_to`] (stage-agnostic + executed-implying), so this ceiling
/// only ever fires on a NON-completing pathological attempt (a forged / withheld
/// tip, or a permanently-broken engine channel), converting an indefinite silent
/// hang into a loud abort. Its exact value is uncritical as long as it exceeds any
/// plausible healthy backfill — it predicts nothing.
///
/// Progress-keyed alternatives are REFUTED (2026-07-06 research addendum): the
/// binding pipeline stages have no reth duration constant to derive a window from
/// (Headers is all-or-nothing; Bodies is download-rate-bound), so any
/// progress-window false-trips a healthy bandwidth-limited sync — the Gen-1
/// no-progress-timer bug relocated onto the stage checkpoint. Gap-scaling was also
/// dropped as YAGNI: the only case this ceiling fires is the non-completing one,
/// where gap precision buys nothing. The REAL fix for spurious trips on HEALTHY
/// syncs is the 5b single-writer gating (bugs 6/7/9 stop competing executor FCUs
/// from retargeting reth's backfill so the `Valid` terminator actually fires), not
/// this constant's precision.
///
/// There is no cold-start retry above this path (a trip is fatal at cold-start via
/// [`crate::dpos::jump_landing_or_abort`]), so oversizing (never false-killing a
/// healthy sync) is the cheap failure direction; 6 h ≈ a day-and-change offline at
/// 1 blk/s. A from-genesis, millions-of-blocks deep sync remains an out-of-scope
/// ops antipattern (bootstrap from a state snapshot, NOT this path). (User sign-off
/// 2026-07-06; supersedes the former flat 45-min `EL_SYNC_ABSOLUTE_CEILING`.)
const EL_SYNC_BACKSTOP_CEILING: Duration = Duration::from_secs(6 * 60 * 60);

/// PRIMARY net: when reth reports ZERO connected devp2p peers continuously for
/// this long, the sync can never complete — the actual root cause of a permanent
/// EL-sync stall (no uplink, or the trusted peer was never configured; project
/// memory: "connected_peers=0 makes it permanent"). Firing on it gives a FAST,
/// precise failure with a "no peers" message instead of waiting out the absolute
/// ceiling. Sized ABOVE the worst-case (~75 s) trusted-peer dial latency so a
/// healthy boot — which legitimately shows 0 peers until its first devp2p session
/// is established — is never false-tripped; the window resets the instant a peer
/// connects.
const EL_SYNC_NO_PEERS_GRACE: Duration = Duration::from_secs(90);

/// TERTIARY net: reth is CONNECTED (peers > 0) but its executed head does not
/// advance for this long — the wedge signature observed in soak v43
/// (bundle-20260718T140358Z): reth's pipeline Execution stage hit a re-execution
/// divergence, unwound, marked the block a bad ancestor, and went idle answering
/// every FCU `SYNCING` FOREVER (never `Invalid`). The pre-existing nets do not
/// catch it: the no-peers net never fires (peers stay ≥ 1) and the 6-h backstop
/// leaves the node SILENTLY wedged for the whole run. Sized FAR below the backstop
/// (a fast, precise failure) yet WELL above any normal backfill hiccup — a healthy
/// pipeline advances its executed head (`best_block_number`) within any 5-minute
/// window at the committed 1 blk/s; the Headers/Bodies download stages predate
/// Execution, so `best_block_number` only starts climbing once the branch is being
/// executed, i.e. exactly when a genuine wedge would show. The escape is
/// deliberately NON-fatal ([`JumpOutcome::StalledWithPeers`]): the divergence root
/// cause is unknown and deterministic, so each re-jump re-wedges — the node stays
/// OBSERVABLE (ERROR log + counter per attempt) and DEFERRED rather than silent.
const EL_SYNC_STALL_ESCAPE: Duration = Duration::from_secs(300);

/// Which net tripped the EL-sync watchdog.
#[derive(Debug, PartialEq, Eq)]
enum WatchdogTrip {
    /// Zero connected devp2p peers for [`EL_SYNC_NO_PEERS_GRACE`] (the primary net).
    NoPeers,
    /// The backstop ceiling elapsed without a `Valid` terminator (bugs 5/8).
    Ceiling,
    /// Peers > 0 but the executed head (`best_block_number`) did not advance for
    /// [`EL_SYNC_STALL_ESCAPE`] — the connected-but-wedged pipeline signature.
    StalledWithPeers,
}

/// Pure, tick-counted watchdog for [`RethElSync::sync_to`] — a runtime-free state
/// machine so the net logic is unit-testable (gotcha #23: `RethElSync` itself is
/// bound to the concrete tokio `Context` and stays untested under the
/// deterministic runner). Each loop iteration sleeps [`EL_SYNC_TICK`], so tick
/// counts bound elapsed time monotonically WITHOUT reading a clock (bug 8).
struct SyncWatchdog {
    ticks: u64,
    no_peer_ticks: u64,
    max_ticks: u64,
    no_peer_max_ticks: u64,
    /// Consecutive ticks with peers > 0 and NO advance in the executed head — the
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
    /// head (`best_block_number`). `Some(trip)` ⇒ the attempt must abort; `None` ⇒
    /// keep waiting. The no-peers counter resets the instant a peer connects, so
    /// interleaved zero-peer windows never accumulate; the stall counter accrues
    /// ONLY while peers > 0 AND the head is unchanged (any forward progress or a
    /// peer drop — handed to the no-peers net — resets it), so a healthy but
    /// bandwidth-limited backfill (which keeps advancing its head) is never
    /// false-tripped.
    fn on_tick(&mut self, connected_peers: usize, latest_block: u64) -> Option<WatchdogTrip> {
        self.ticks += 1;

        // TERTIARY net: connected-but-no-progress. Mutually exclusive per-tick with
        // the no-peers net (this arm requires peers > 0), so the two never race.
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

/// Typed terminal failure of an [`ElSync::sync_to`] attempt — distinguishes a
/// genuinely-INVALID served branch (reth itself rendered a verdict) from a
/// timeout/transport stall (reth rendered no verdict at all), so
/// [`cold_start_jump_with_threshold`] can route them to different
/// [`JumpOutcome`]s ([`JumpOutcome::InvalidTarget`] vs [`JumpOutcome::Stalled`])
/// instead of folding both into one undifferentiated error.
///
/// `From<eyre::Report>` classifies any generic/untyped failure as [`Stalled`]
/// (the pre-existing default for transport errors, provider read failures, the
/// zero-peers grace, and the absolute ceiling); [`RethElSync::sync_to`]
/// constructs [`Invalid`] explicitly, the one case with more information than a
/// generic report.
///
/// [`Stalled`]: SyncFailure::Stalled
/// [`Invalid`]: SyncFailure::Invalid
#[derive(Debug)]
pub enum SyncFailure {
    /// reth's FCU polling loop observed `PayloadStatusEnum::Invalid` — the
    /// served branch is structurally wrong. This is a genuine terminal
    /// verdict, never a transient mid-backfill state: reth's own
    /// `validate_forkchoice_state` short-circuits to `Syncing` while backfill
    /// is non-idle (pinned reth fork, `crates/engine/tree/src/tree/mod.rs`), so
    /// `Invalid` can only fire once backfill is idle and reth itself judged
    /// the branch bad.
    Invalid(eyre::Report),
    /// Any other `sync_to` failure: transport error, provider read failure,
    /// zero-peers grace expired, or the absolute ceiling tripped. reth
    /// rendered no verdict on the branch at all — it simply didn't finish.
    Stalled(eyre::Report),
    /// reth is CONNECTED (peers > 0) but its executed head did not advance for
    /// [`EL_SYNC_STALL_ESCAPE`] — the connected-but-wedged pipeline signature
    /// (soak v43: an Execution-stage re-execution divergence unwound to a bad
    /// ancestor and reth went idle answering `SYNCING` forever, never `Invalid`).
    /// Distinct from [`Stalled`](SyncFailure::Stalled) (which folds the no-peers /
    /// ceiling / transport cases): here reth is demonstrably reachable yet making
    /// no progress, so the actionable operator signal (a stuck EL, not a missing
    /// uplink) differs — and the escape fires ~72× faster than the 6-h backstop.
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
/// backfill, with the no-progress stall detector) and return the
/// `(height, hash)` it landed on. Implemented over the provider + beacon engine
/// by [`RethElSync`]; a fake implements it in tests. Relocated from
/// `follow.rs::ElSync`.
pub trait ElSync: Send + Sync {
    fn sync_to(
        &self,
        latest: &UpstreamFinalized,
    ) -> impl Future<Output = Result<(u64, B256), SyncFailure>> + Send;

    /// Whether the local chain holds `hash` canonically — the post-jump L1
    /// trust-root re-assert (the synced head must be a descendant of the
    /// L1-finalized block).
    fn holds(&self, hash: B256) -> eyre::Result<bool>;
}

/// [`ElSync`] over the node's reth: FCU toward the attested derived hash (the
/// upstream tip's `result` = derived hash of tip − K) and wait for reth to
/// DECLARE the tip canonical+executed via the FCU `PayloadStatus::Valid`
/// terminator, failing only on the zero-peers / absolute-ceiling nets. Relocated
/// from `cert_follow/mod.rs::RethElSync`.
pub struct RethElSync<Provider, BeaconEngine> {
    ctx: Context,
    provider: Provider,
    beacon_engine: BeaconEngine,
    activation: u64,
    /// Read-only probe of reth's connected devp2p peer count, driving the primary
    /// no-peers net in [`Self::sync_to`]. A closure (built host-side from
    /// `node.network`, which implements `reth_network_api::PeersInfo`) rather than
    /// a typed handle keeps this consensus crate decoupled from `reth-network-api`
    /// and adds no generic parameter to the four call sites.
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
{
    /// The EXECUTED height/hash reth currently sits at, clamped to ≥ activation
    /// (the ordering chain starts there; pre-activation blocks carry no certs).
    /// Reads the canonical/executed head (`best_block_number`), NOT the static-file
    /// header tip (`last_block_number`): a landing reseeds the anchor and feeds the
    /// post-sync committee state read, so it must point at materialized state — a
    /// header-only tip races ahead of execution (project memory `reth-sync-progress`).
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
}

impl<Provider, BeaconEngine> ElSync for RethElSync<Provider, BeaconEngine>
where
    Provider: BlockHashReader + BlockNumReader + Clone + Send + Sync + 'static,
    BeaconEngine: BeaconEngineLike + Clone + Send + Sync + 'static,
{
    async fn sync_to(&self, latest: &UpstreamFinalized) -> Result<(u64, B256), SyncFailure> {
        // F-type: the upstream serves ORDERING artifacts — the only real EVM
        // hash on the wire is the committee-attested `result` (derived hash
        // of tip − K); FCU toward it and let reth devp2p backfill the bodies.
        let tip_hash = latest.block.result;
        let tip_height = latest.block.height.saturating_sub(K);
        if tip_hash == B256::ZERO {
            info!(
                tip = latest.block.height,
                "cold-start jump: upstream tip is inside the pre-K window; nothing to EL-sync"
            );
            return self.local_landing().map_err(SyncFailure::Stalled);
        }
        // Already EXECUTED past the target (e.g. a re-run after a prior landing)?
        // Nothing to EL-sync. Gate on the executed head, not header presence — a
        // header-only tip from an interrupted backfill is not yet a valid landing.
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
            "cold-start jump: driving reth EL-sync toward attested derived hash"
        );
        // OPTIMISTIC / checkpoint-sync FCU shape: head == safe == finalized == the
        // attested tip. Pointing `finalized` at the (missing) TARGET is what makes reth
        // run the staged PIPELINE backfill (`backfill_sync_target` returns the target ⇒
        // `BackfillAction::Start`); with `finalized = genesis` (already on disk) reth
        // would instead live-download disconnected blocks that never canonicalize. This
        // shape (Part A) is load-bearing — DO NOT change it. Driving reth's EL onto the
        // tip is NOT acceptance: the post-sync committee BLS + L1 `holds()` gates in
        // `cold_start_jump` remain the trust root and fail closed on a forged branch.
        let fc_state = ForkchoiceState {
            head_block_hash: tip_hash,
            safe_block_hash: tip_hash,
            finalized_block_hash: tip_hash,
        };

        // Completion is reth's OWN declared sync verdict, not a DB-counter proxy:
        // re-issue the (idempotent) seeding FCU and read its `PayloadStatus`.
        //   • `Valid{latest_valid_hash == tip}` ⇒ DONE — the tip is canonical AND its
        //     state is materialized (exactly the guarantee the post-sync committee read
        //     needs). Stage-agnostic and executed-implying: reth declares `Syncing` for
        //     the WHOLE backfill incl. the long execution stage (validate_forkchoice_state
        //     short-circuits while backfill is non-idle), so there is no flat-counter
        //     window to misread as a stall — this DELETES the mid-execution false-stall.
        //   • `Invalid` ⇒ the served branch is structurally wrong — error out so the
        //     caller rotates / boots anyway. This is NOT `AuthFailed` (reserved for the
        //     POST-sync committee-BLS / L1 gates).
        //   • `Syncing | Accepted` ⇒ keep waiting (`Accepted` ⇒ not yet executed).
        // Re-issuing FCU(head=tip) each tick is a cheap status read while backfill is
        // non-idle, and the executor suppresses its heartbeat FCU during a jump (single
        // writer), so the re-FCU loop is safe. The wall-clock nets below are the only
        // failure paths — completion is the `Valid` terminator.
        let mut watchdog = SyncWatchdog::new();
        let mut last_probe_error: Option<eyre::Report> = None;
        // Sticky last-known executed head for the connected-but-wedged net. A probe
        // Err reuses the previous value (observed = "no progress this tick"), so a
        // transient read failure only DELAYS the stall verdict, never fakes progress.
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
                            "cold-start jump: reth REJECTED the attested tip {tip_hash:?} (height \
                             {tip_height}) as INVALID during EL-sync: {validation_error} — the upstream \
                             served a structurally-wrong branch"
                        )));
                    }
                    // `Valid` for a stale/other head (reth ignored the FCU as stale), or
                    // `Syncing` / `Accepted`: not done — keep waiting.
                    _ => {}
                },
                // bug 9: a transient per-tick FCU error must NOT end the attempt — the
                // backstop bounds total wait. Record it for the eventual trip message.
                Err(report) => {
                    warn!(
                        tip_height,
                        error = %report,
                        "cold-start jump: transient FCU probe error during EL-sync; retrying \
                         (the backstop ceiling bounds total wait)"
                    );
                    last_probe_error = Some(report);
                }
            }

            // PRIMARY net (zero peers) + SECONDARY net (backstop) + TERTIARY net
            // (connected-but-no-progress) via the pure, tick-counted watchdog —
            // monotone by construction, no clock reads (bug 8).
            let peers = (self.peer_count)();
            if let Ok(best) = self.provider.best_block_number() {
                last_best_block = best;
            }
            if peers == 0 && watchdog.no_peer_ticks == 0 {
                warn!(
                    tip_height,
                    grace = ?EL_SYNC_NO_PEERS_GRACE,
                    "cold-start jump: reth reports ZERO connected devp2p peers — EL-sync cannot \
                     progress; will fail if none connects within the grace window \
                     (check --trusted-peers / firewall)"
                );
            }
            match watchdog.on_tick(peers, last_best_block) {
                Some(WatchdogTrip::NoPeers) => {
                    return Err(SyncFailure::Stalled(eyre!(
                        "cold-start jump: reth had ZERO connected devp2p peers for \
                         {EL_SYNC_NO_PEERS_GRACE:?} (target tip {tip_height}) — no uplink, or no \
                         trusted peer configured; EL-sync cannot complete"
                    )));
                }
                Some(WatchdogTrip::StalledWithPeers) => {
                    // Connected but the executed head is frozen — reth's pipeline
                    // wedged (soak v43 bad-ancestor idle-SYNCING). ERROR (not warn):
                    // this is a stuck EL, not a transient hiccup, and the caller
                    // keeps the node observable + deferred rather than silent.
                    error!(
                        tip_height,
                        best_block = last_best_block,
                        peers,
                        stall = ?EL_SYNC_STALL_ESCAPE,
                        "cold-start jump: reth is CONNECTED ({peers} peers) but its executed head \
                         has not advanced past {last_best_block} for {EL_SYNC_STALL_ESCAPE:?} \
                         (target tip {tip_height}) — the EL pipeline is wedged (e.g. an unwound \
                         bad-ancestor answering SYNCING forever); escaping so the node stays \
                         observable instead of silently stuck at the 6-h backstop"
                    );
                    return Err(SyncFailure::StalledWithPeers(eyre!(
                        "cold-start jump: reth CONNECTED ({peers} peers) but executed head frozen \
                         at {last_best_block} for {EL_SYNC_STALL_ESCAPE:?} (target tip \
                         {tip_height}) — EL pipeline wedged"
                    )));
                }
                Some(WatchdogTrip::Ceiling) => {
                    return Err(SyncFailure::Stalled(eyre!(
                        "cold-start jump: reth EL-sync did not reach a VALID head within the \
                         {EL_SYNC_BACKSTOP_CEILING:?} backstop (target tip {tip_height}, elapsed \
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
        // Clamp BEFORE resolving the hash so the returned pair is always
        // self-consistent (height and hash of the SAME block).
        let landing = tip_height.max(self.activation);
        let hash = self
            .provider
            .block_hash(landing)
            .wrap_err("block_hash(landing) probe after EL-sync")?
            .ok_or_else(|| eyre!("EL-sync landed but block {landing} vanished"))?;
        Ok((landing, hash))
    }

    fn holds(&self, hash: B256) -> eyre::Result<bool> {
        Ok(self
            .provider
            .block_number(hash)
            .wrap_err("block_number(l1 checkpoint) probe failed")?
            .is_some())
    }
}

/// PRE-`sync_to` structural gate: the served cert must sign the served body
/// (`cert.payload == block.digest()`). This is the ONLY check that can run
/// before `sync_to`, because the per-epoch committee that authenticates the cert
/// is not yet locally readable (the whole point of the jump is to sync TO the
/// state that holds it). A structural mismatch ABORTS the jump (the caller gets
/// [`JumpOutcome::BadTarget`] and rotates / boots anyway) — still never drives
/// reth's devp2p backfill onto a tip whose served body does not match its cert,
/// but is NON-fatal (Rule S): a signature-free, attacker-controlled pre-anchor
/// mismatch must not crash a node.
pub(crate) fn verify_jump_structural(latest: &UpstreamFinalized) -> eyre::Result<()> {
    if latest.finalization.proposal.payload != latest.block.digest() {
        return Err(eyre!(
            "jump target cert payload != block digest at height {}",
            latest.block.height
        ));
    }
    Ok(())
}

/// POST-`sync_to` trustless authentication: BLS-verify the jump target's
/// finalization against `committee[E]` read at the now-materialized landing
/// state (`landing_hash`), and FAIL CLOSED on mismatch.
///
/// This runs AFTER `sync_to` — once reth holds the landing block's state, the
/// committee read is local and complete (committees are ahead-committed at
/// epoch-(E-1)-start, and the landing `tip − K` shares the tip's epoch since
/// interval ≥ 32 ≫ K=3, so `committee[E_tip]` is committed at the landing
/// state). A pre-`sync_to` read at the stale resolved anchor was the
/// chicken-and-egg trap: `committee[far_epoch]` is NOT committed there, so the
/// gate was structurally unreachable in the deep-catch-up case it exists to
/// protect. Reading it here closes that hole WITHOUT requiring an L1 checkpoint —
/// it is the trustless default.
///
/// `Err` ⇒ fail closed: the launcher's `?` aborts the launch, so the node never
/// participates (signs / serves) on an unauthenticated branch. The cert is the
/// committee's own attestation of the derived tip, so this is sound even though
/// reth has already devp2p-synced the branch (the executor/engine has NOT yet
/// started — see the single-writer note on [`cold_start_jump`]).
///
/// `l1_checkpoint`: when the trustless committee read is genuinely impossible
/// (`scheme_at` returns `Err` because `committee[E]` is unreadable even at the
/// synced landing — e.g. the upstream served a tip whose state does not commit
/// its own committee), the post-jump L1 `holds()` re-assert in [`cold_start_jump`]
/// is the operator-gated alternative trust anchor: with an L1 checkpoint set the
/// jump still authenticates via the L1 ancestry, so an unreadable committee is a
/// LOUD warn-and-defer-to-L1; WITHOUT one it is FATAL (no trust anchor at all).
pub(crate) fn verify_jump_authenticated<C: CommitteeSource>(
    latest: &UpstreamFinalized,
    committees: &C,
    landing_hash: B256,
    l1_checkpoint: Option<B256>,
    ctx: &mut (impl Clock + CryptoRngCore),
) -> eyre::Result<()> {
    let epoch = latest.finalization.proposal.round.epoch().get();
    // Cold-start jump landing verify: no local beacon key resolvable here (the
    // marshal is empty right after a deep jump), so `cert_seed_pin = None` ⇒
    // vote-only cert verify — the accepted residual window (bug 2 sign-off item 1).
    match committees.scheme_at(epoch, landing_hash, None) {
        Ok(scheme) => {
            ensure!(
                latest.finalization.verify(ctx, &scheme, &Sequential),
                "jump target finalization FAILED BLS verification against committee[{epoch}] \
                 read at the synced landing {landing_hash:?} (height {}) — the upstream served \
                 a forged / unagreed branch; refusing to follow",
                latest.block.height
            );
            Ok(())
        }
        Err(read_err) => {
            // The committee is unreadable even at the synced landing state. This
            // is NOT the deep-catch-up case (that committee IS committed at the
            // landing) — it is a degenerate upstream (a tip whose own state does
            // not commit its committee) or a real read fault. Defer to the L1
            // trust anchor when configured; otherwise fail closed.
            ensure!(
                l1_checkpoint.is_some(),
                "jump target committee[{epoch}] is unreadable at the synced landing \
                 {landing_hash:?} (height {}) and no --dpos.l1-checkpoint is configured — \
                 there is NO trust anchor to authenticate the upstream branch; refusing to \
                 follow ({read_err:#})",
                latest.block.height
            );
            tracing::warn!(
                height = latest.block.height,
                epoch,
                error = %read_err,
                "jump target committee unreadable at the synced landing: authenticating via \
                 the configured --dpos.l1-checkpoint instead of the on-chain committee \
                 (the post-jump holds() probe is the trust root)"
            );
            Ok(())
        }
    }
}

/// Single-shot (cold-start) OR steady-state forward-only EL fast-forward.
///
/// At cold-start it runs BEFORE the OuterEngine (mutually exclusive with the
/// executor — the same property `recover_finalized_tail_into_reth` relies on); in
/// steady state the executor spawns it as a READ-ONLY waiter and reacts to its
/// terminal [`JumpOutcome`] (§9.6). It NEVER returns an in-progress variant — the
/// caller (cold-start `?`-abort, or the executor's spawn) owns the wait.
///
/// Classification ([`JumpOutcome`]):
///   - no `get_latest` / shallow gap / stale-or-backward landing ⇒ [`Lagging`];
///   - `verify_jump_structural` reject (PRE-sync `payload != digest`) ⇒
///     [`BadTarget`] (NON-fatal, Rule S — a forgeable, attacker-controlled
///     pre-anchor mismatch: the steady-state caller rotates, the cold-start
///     caller boots anyway);
///   - `verify_jump_authenticated` (POST-sync committee BLS) / L1 `holds()`
///     reject ⇒ [`AuthFailed`] (FATAL — a forged / unagreed branch that
///     canonicalized);
///   - `el.sync_to` transport/timeout stall ([`SyncFailure::Stalled`]) ⇒
///     [`Stalled`] (NON-fatal in steady state — was the `?` that propagated as
///     fatal: THIS is the transient-stall-crash fix);
///   - `el.sync_to` observes reth declare the served branch INVALID
///     ([`SyncFailure::Invalid`]) ⇒ [`InvalidTarget`] (NON-fatal, same
///     treatment as `BadTarget` — the steady-state caller rotates immediately,
///     the cold-start caller boots anyway — kept as a SEPARATE variant since
///     `BadTarget` is pinned to the PRE-sync, `el_sync_calls == 0` case);
///   - success ⇒ [`Landed`].
///
/// Gating is the caller's job: the launcher only calls this when
/// `kind != FreshMigration && upstream.is_some()`. Inside, the need-gate is
/// forward-only — a target ≤ `anchor + JUMP_THRESHOLD` is [`Lagging`], and a
/// landing that does not advance `anchor` is dropped (never reseed backward).
///
/// [`Lagging`]: JumpOutcome::Lagging
/// [`BadTarget`]: JumpOutcome::BadTarget
/// [`InvalidTarget`]: JumpOutcome::InvalidTarget
/// [`AuthFailed`]: JumpOutcome::AuthFailed
/// [`Stalled`]: JumpOutcome::Stalled
/// [`Landed`]: JumpOutcome::Landed
pub async fn cold_start_jump<U, C, ES>(
    anchor: u64,
    upstream: &U,
    committees: &C,
    el: &ES,
    l1_checkpoint: Option<B256>,
    activation: u64,
    ctx: &mut (impl Clock + CryptoRngCore),
) -> JumpOutcome
where
    U: crate::cert_follow::CertUpstream,
    C: CommitteeSource,
    ES: ElSync,
{
    // The COLD-START callers use the fixed serving-window need-gate; the
    // STEADY-STATE re-jump uses `cold_start_jump_with_threshold` with an
    // epoch-relative gate (see `ReJump::threshold`).
    cold_start_jump_with_threshold(
        anchor,
        upstream,
        committees,
        el,
        l1_checkpoint,
        activation,
        JUMP_THRESHOLD,
        ctx,
    )
    .await
}

/// As [`cold_start_jump`] but with a caller-supplied forward-only `jump_threshold`
/// instead of the fixed [`JUMP_THRESHOLD`]. The cold-start path uses
/// [`cold_start_jump`] (threshold = [`JUMP_THRESHOLD`], the serving-window size);
/// the STEADY-STATE re-jump passes `min(JUMP_THRESHOLD, epoch_block_interval)` so
/// the jump preempts the ≥2-epoch "committee[E] not committed" defer deadlock at
/// ANY epoch interval (real-prod epochs ≫ 1024 keep 1024 unchanged; a compressed
/// test epoch scales the gate down to ~1 epoch). The deadlock is epoch-relative,
/// so its recovery gate is too. The post-sync trustless authentication
/// (`verify_jump_authenticated` + L1 `holds()`) is UNCHANGED — a lower gate only
/// makes the jump ATTEMPT a shallower gap; a forged target still fails closed.
#[allow(clippy::too_many_arguments)]
pub async fn cold_start_jump_with_threshold<U, C, ES>(
    anchor: u64,
    upstream: &U,
    committees: &C,
    el: &ES,
    l1_checkpoint: Option<B256>,
    activation: u64,
    jump_threshold: u64,
    ctx: &mut (impl Clock + CryptoRngCore),
) -> JumpOutcome
where
    U: crate::cert_follow::CertUpstream,
    C: CommitteeSource,
    ES: ElSync,
{
    let Some(latest) = upstream.get_latest().await else {
        return JumpOutcome::Lagging;
    };
    // Forward-only need-gate: only re-run the EL-sync phase for a gap beyond
    // `jump_threshold`.
    if latest.block.height <= anchor + jump_threshold {
        return JumpOutcome::Lagging;
    }
    // PRE-sync structural gate: never devp2p-drive reth onto a tip whose served
    // body does not match its cert. The committee-backed BLS authentication is
    // POST-sync (`verify_jump_authenticated` below) — the committee that
    // authenticates a far-ahead target is only locally readable once `sync_to`
    // has materialized its state.
    if let Err(e) = verify_jump_structural(&latest) {
        return JumpOutcome::BadTarget(e);
    }
    // A `sync_to` transport/timeout stall is NON-fatal (was a `?` that
    // propagated as fatal): the steady-state caller retries on the next
    // `Update::Tip`. A genuinely-INVALID served branch is a DIFFERENT,
    // non-fatal outcome (`InvalidTarget`, NOT `Stalled`) — reth rendered an
    // actual verdict, so the caller should rotate immediately / boot anyway,
    // exactly like `BadTarget`, rather than tolerating it as a transient stall.
    let (landing_h, landing_hash) = match el.sync_to(&latest).await {
        Ok(landing) => landing,
        Err(SyncFailure::Invalid(e)) => return JumpOutcome::InvalidTarget(e),
        Err(SyncFailure::StalledWithPeers(e)) => return JumpOutcome::StalledWithPeers(e),
        Err(SyncFailure::Stalled(e)) => return JumpOutcome::Stalled(e),
    };
    // A landing that does not advance the resolved anchor (lagging get_latest,
    // upstream reorg) must not reseed backward.
    if landing_h <= anchor {
        return JumpOutcome::Lagging;
    }
    // POST-sync trustless authentication: now that reth holds the landing state,
    // read `committee[E]` at the landing hash and BLS-verify the finalization —
    // FAIL CLOSED on mismatch. This is the missing piece that makes a deep jump
    // trustless without an L1 checkpoint, the structural dual of the L1 `holds()`
    // re-assert below (both run post-sync because both read state the jump just
    // synced).
    if let Err(e) = verify_jump_authenticated(&latest, committees, landing_hash, l1_checkpoint, ctx)
    {
        return JumpOutcome::AuthFailed(e);
    }
    // L1 re-assert when configured: the synced head must descend from the
    // L1-finalized block (relocated `reseed_at_jump_landing` L1 probe).
    if let Some(l1) = l1_checkpoint {
        match el
            .holds(l1)
            .wrap_err("L1 checkpoint probe after jump failed")
        {
            Ok(true) => {}
            Ok(false) => {
                // #10: the synced head does not extend L1-finalized history — an L1
                // fork, NOT a forged upstream. `L1Fork` (not `AuthFailed`) so the
                // runtime caller SafetyHalts rather than rotating.
                return JumpOutcome::L1Fork(eyre!(
                    "L1 Rollup checkpoint {l1:?} is NOT in the local chain after an EL-sync \
                     jump — the synced head does not extend the L1-finalized history (possible \
                     upstream equivocation); refusing to follow"
                ));
            }
            // A transport error on the probe itself is not a fork verdict — treat
            // it like a forged/unreadable upstream and rotate (AuthFailed).
            Err(e) => return JumpOutcome::AuthFailed(e),
        }
    }
    // Floor = landing − K (clamped to activation): the K below-landing blocks
    // are derivable via the inlet's ordinary pulls + the executor gap-walk, and
    // the landing itself is not result-attested yet (two-tier contract).
    let floor = landing_h.saturating_sub(K).max(activation);
    info!(
        from = anchor,
        to = landing_h,
        floor,
        "cold-start jump: EL-sync fast-forwarded the anchor"
    );
    JumpOutcome::Landed {
        landing: landing_h,
        hash: landing_hash,
        floor,
    }
}

/// B2 — the cold-start L1 Rollup-checkpoint assert, used by the follower
/// cold-start path ([`crate::dpos::DposLayer::launch_follower`]) so a
/// `--cert-follow` follower fails closed on a bogus L1 checkpoint after EL-sync.
/// The L1-finalized batch's last block hash must be canonical in OUR reth
/// (post-`cold_start_jump` EL-sync); a missing hash means the synced head does
/// not extend the L1-finalized history (possible upstream equivocation).
///
/// The two log/error strings — `"L1 Rollup checkpoint verified against local
/// chain"` and `"is NOT in the local chain after EL-sync"` — are byte-asserted
/// by `case-cert-cascade.sh` (phase-1 `:59` / phase-3 `:89`) and MUST survive
/// verbatim.
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
        // Peers present + head ADVANCING every tick (distinct block each call) — so
        // neither the no-peers nor the stall net fires; only the backstop, at MAX_TICKS.
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

        // A single connected tick resets the streak; a fresh full window is then
        // needed to trip.
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
        // Alternating zero/nonzero peers with the head ADVANCING every tick: both the
        // no-peers streak (resets on the connected tick) and the stall streak (resets
        // on the progress) never reach their windows over many rounds.
        for i in 0..(GRACE_TICKS * 4) {
            assert_eq!(w.on_tick(0, i * 2), None);
            assert_eq!(w.on_tick(2, i * 2 + 1), None);
        }
    }

    #[test]
    fn stall_with_peers_trips_when_head_frozen_and_not_before() {
        let mut w = SyncWatchdog::new();
        // Peers present, head FROZEN at 42 → the stall streak accrues (the first
        // tick only sets the baseline, so the trip lands one tick after STALL_TICKS).
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
        // One forward move resets the streak (and re-baselines at 43); a fresh full
        // window is then needed — STALL_TICKS unchanged ticks after the re-baseline.
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
        // Head frozen but ZERO peers for well past the stall window: the stall net
        // must NOT fire (that case belongs to the no-peers net) — kept under the
        // no-peers grace so nothing trips at all.
        for _ in 0..(GRACE_TICKS - 1) {
            assert_eq!(w.on_tick(0, 42), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{beacon::seed::GroupPublic, digest::Digest, order_block::OrderBlock};
    use alloy_primitives::{Address, Bytes};
    use commonware_codec::DecodeExt as _;
    use commonware_consensus::{
        simplex::types::{Finalization, Finalize, Proposal},
        types::{Epoch, Round, View},
    };
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
    use commonware_math::algebra::Random as _;
    use commonware_runtime::{deterministic, Runner as _};
    use commonware_utils::{ordered::BiMap, TryCollect as _};
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
            .map(|kp| build_signer(&ns, bimap.clone(), kp, None).expect("member"))
            .collect();
        let verifier = fluentbase_bls::scheme::build_verifier(&ns, bimap, None, None);
        Committee { signers, verifier }
    }

    fn sample_order(parent: Digest, height: u64, result: B256) -> OrderBlock {
        OrderBlock {
            parent,
            height,
            proposal_view: 0,
            timestamp: 1_700_000_000 + height,
            fee_recipient: Address::ZERO,
            gas_limit: 30_000_000,
            extra_data: Bytes::new(),
            result,
            txs: Vec::new(),
            beacon_outcome: None,
            dkg_logs: Vec::new(),
            parent_seed: None,
        }
    }

    /// 2f+1 finalize votes over the block's digest → a REAL finalization cert.
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
        /// `false` ⇒ `scheme_at` returns `Err` (committee unreadable even at the
        /// synced landing — the degenerate-upstream / read-fault case that
        /// `verify_jump_authenticated` defers to L1, or fails closed without one).
        readable: bool,
    }

    impl CommitteeSource for CannedCommittees {
        fn scheme_at(
            &self,
            epoch: u64,
            at_hash: B256,
            _cert_seed_pin: Option<GroupPublic>,
        ) -> eyre::Result<BlsScheme> {
            self.reads.lock().unwrap().push((epoch, at_hash));
            if !self.readable {
                return Err(eyre!(
                    "epoch {epoch} has no committed committee at {at_hash}"
                ));
            }
            Ok(self.verifier.clone())
        }
        // The cold-start jump never reads at the finalized tip — it always has a
        // specific `at_hash`; satisfy the trait with a canned verifier.
        fn scheme_at_finalized_tip(
            &self,
            _epoch: u64,
            _cert_seed_pin: Option<GroupPublic>,
        ) -> eyre::Result<Option<BlsScheme>> {
            Ok(Some(self.verifier.clone()))
        }
    }

    #[derive(Clone, Default)]
    struct FakeUpstream {
        latest: Arc<Mutex<Option<UpstreamFinalized>>>,
    }

    impl crate::cert_follow::CertUpstream for FakeUpstream {
        async fn get_finalization(
            &self,
            _height: commonware_consensus::types::Height,
        ) -> Option<UpstreamFinalized> {
            None
        }
        async fn get_latest(&self) -> Option<UpstreamFinalized> {
            self.latest.lock().unwrap().clone()
        }
        async fn rotate(&self) {}
    }

    struct FakeElSync {
        landing: (u64, B256),
        calls: Arc<Mutex<u32>>,
        holds_l1: bool,
    }

    impl ElSync for FakeElSync {
        async fn sync_to(&self, _latest: &UpstreamFinalized) -> Result<(u64, B256), SyncFailure> {
            *self.calls.lock().unwrap() += 1;
            Ok(self.landing)
        }
        fn holds(&self, _hash: B256) -> eyre::Result<bool> {
            Ok(self.holds_l1)
        }
    }

    struct Fixture {
        committee: Committee,
        upstream: FakeUpstream,
        el: FakeElSync,
        scheme_reads: Arc<Mutex<Vec<(u64, B256)>>>,
        el_sync_calls: Arc<Mutex<u32>>,
    }

    fn fixture(landing: (u64, B256), holds_l1: bool) -> (CannedCommittees, Fixture) {
        fixture_with_committee(landing, holds_l1, true)
    }

    /// `committee_readable == false` makes `scheme_at` return `Err` (committee
    /// unreadable even at the synced landing).
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
            upstream: FakeUpstream::default(),
            el: FakeElSync {
                landing,
                calls: el_sync_calls.clone(),
                holds_l1,
            },
            scheme_reads,
            el_sync_calls,
        };
        (committees, fx)
    }

    /// A deep gap above [`JUMP_THRESHOLD`] drives the EL-sync once and re-seeds
    /// the anchor + floor at the landing (`floor == landing − K`).
    #[test]
    fn jump_triggers_el_sync_and_reseeds_anchor() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let landing = (5000, B256::repeat_byte(0xe1));
            let (committees, fx) = fixture(landing, true);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            *fx.upstream.latest.lock().unwrap() = Some(certify(&fx.committee, 0, &live));
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &fx.el,
                None,
                ACTIVATION,
                &mut ctx,
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
            assert_eq!(
                fx.scheme_reads.lock().unwrap()[0].1,
                B256::repeat_byte(0xe1),
                "committee read POST-sync at the landing hash (not the stale anchor) — the \
                 far-epoch committee is only committed in the state the jump just synced"
            );
        });
    }

    /// A target within [`JUMP_THRESHOLD`] of the anchor is a no-op — the
    /// residual gap is the inlet's ordinary-pull job; the EL-sync never runs.
    #[test]
    fn shallow_gap_does_not_jump() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let (committees, fx) = fixture((5000, B256::repeat_byte(0xe1)), true);
            let near = ACTIVATION + 8; // well below JUMP_THRESHOLD
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                near,
                B256::repeat_byte(0x44),
            );
            *fx.upstream.latest.lock().unwrap() = Some(certify(&fx.committee, 0, &live));
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &fx.el,
                None,
                ACTIVATION,
                &mut ctx,
            )
            .await;
            assert!(
                matches!(out, JumpOutcome::Lagging),
                "shallow gap must not jump"
            );
            assert_eq!(*fx.el_sync_calls.lock().unwrap(), 0, "el_sync never ran");
        });
    }

    /// No upstream tip available ⇒ no jump (the caller keeps the discriminator
    /// anchor).
    #[test]
    fn no_latest_does_not_jump() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let (committees, fx) = fixture((5000, B256::repeat_byte(0xe1)), true);
            // upstream.latest stays None.
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &fx.el,
                None,
                ACTIVATION,
                &mut ctx,
            )
            .await;
            assert!(matches!(out, JumpOutcome::Lagging), "no latest ⇒ no jump");
            assert_eq!(*fx.el_sync_calls.lock().unwrap(), 0, "el_sync never ran");
        });
    }

    /// With an L1 checkpoint configured and the synced head NOT descending from
    /// it, the post-jump `holds()` probe fails closed (the trust-root assert).
    #[test]
    fn jump_reasserts_l1_checkpoint_and_fails_closed() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let (committees, fx) = fixture((5000, B256::repeat_byte(0xe1)), false);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            *fx.upstream.latest.lock().unwrap() = Some(certify(&fx.committee, 0, &live));
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &fx.el,
                Some(B256::repeat_byte(0x1a)),
                ACTIVATION,
                &mut ctx,
            )
            .await;
            let JumpOutcome::L1Fork(err) = out else {
                panic!("expected L1Fork (L1 holds()==false), got a different JumpOutcome");
            };
            assert!(err.to_string().contains("NOT in the local chain"), "{err}");
        });
    }

    /// A structurally-broken jump target (cert payload != block digest) is a
    /// NON-fatal `BadTarget` (Rule S — a forgeable, signature-free PRE-anchor
    /// mismatch) and must NOT drive `sync_to` — the PRE-sync structural gate
    /// (`verify_jump_structural`). The committee-backed BLS authentication is
    /// POST-sync and fail-closed `AuthFailed` (see `forged_far_ahead_target_is_rejected`).
    #[test]
    fn unverifiable_jump_target_is_bad_target_before_sync() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let (committees, fx) = fixture((5000, B256::repeat_byte(0xe1)), true);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            let mut forged = certify(&fx.committee, 0, &live);
            // Cert signs a DIFFERENT block than the one served.
            forged.block = sample_order(
                Digest(B256::repeat_byte(0xab)),
                far,
                B256::repeat_byte(0x44),
            );
            *fx.upstream.latest.lock().unwrap() = Some(forged);
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &fx.el,
                None,
                ACTIVATION,
                &mut ctx,
            )
            .await;
            let JumpOutcome::BadTarget(err) = out else {
                panic!(
                    "expected BadTarget (structural payload mismatch), got a different JumpOutcome"
                );
            };
            assert!(err.to_string().contains("payload != block digest"), "{err}");
            assert_eq!(
                *fx.el_sync_calls.lock().unwrap(),
                0,
                "must NOT sync onto an unverified tip"
            );
        });
    }

    /// THE security property: a forged far-ahead target (cert structurally valid
    /// — payload == digest — but signed by an INDEPENDENT committee, so it FAILS
    /// BLS against the genuine committee read at the synced landing) is REJECTED
    /// fail-closed. `sync_to` ran (the upstream is followed before we can read its
    /// committee), but the launch then ABORTS (`Err`) instead of reseeding the
    /// anchor onto the unagreed branch. This is the deep-catch-up case the gate
    /// exists to protect — previously a silent warn-and-proceed.
    #[test]
    fn forged_far_ahead_target_is_rejected() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let landing = (
                ACTIVATION + 1 + JUMP_THRESHOLD + 10,
                B256::repeat_byte(0x55),
            );
            let (committees, fx) = fixture(landing, true);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            // Independent committee: structural check passes, BLS fails against
            // the fixture's genuine committee read at the landing.
            let other = committee(2);
            *fx.upstream.latest.lock().unwrap() = Some(certify(&other, 0, &live));
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &fx.el,
                None,
                ACTIVATION,
                &mut ctx,
            )
            .await;
            let JumpOutcome::AuthFailed(err) = out else {
                panic!("expected AuthFailed (BLS verification), got a different JumpOutcome");
            };
            assert!(
                err.to_string().contains("FAILED BLS verification"),
                "forged target must fail closed, got: {err}"
            );
            assert_eq!(
                *fx.el_sync_calls.lock().unwrap(),
                1,
                "sync_to runs (upstream followed) but the launch aborts after the post-sync \
                 authentication fails — the anchor is NOT reseeded onto the forged branch"
            );
        });
    }

    /// Committee unreadable even at the synced landing (degenerate upstream / read
    /// fault) WITH no L1 checkpoint ⇒ no trust anchor at all ⇒ FAIL CLOSED.
    #[test]
    fn unreadable_committee_without_l1_fails_closed() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let landing = (ACTIVATION + 1 + JUMP_THRESHOLD + 10, B256::repeat_byte(0x55));
            let (committees, fx) = fixture_with_committee(landing, true, false);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(Digest(B256::repeat_byte(0xaa)), far, B256::repeat_byte(0x44));
            *fx.upstream.latest.lock().unwrap() = Some(certify(&fx.committee, 0, &live));
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &fx.el,
                None,
                ACTIVATION,
                &mut ctx,
            )
            .await;
            let JumpOutcome::AuthFailed(err) = out else {
                panic!("expected AuthFailed (unreadable committee + no L1), got a different JumpOutcome");
            };
            assert!(
                err.to_string().contains("NO trust anchor"),
                "unreadable committee + no L1 must fail closed, got: {err}"
            );
        });
    }

    /// Committee unreadable at the synced landing BUT an L1 checkpoint is
    /// configured ⇒ the operator-gated alternative trust anchor: defer to the
    /// post-jump L1 `holds()` probe (here it holds) and proceed. The L1 ancestry
    /// authenticates the branch in lieu of the committee read.
    #[test]
    fn unreadable_committee_with_l1_defers_to_checkpoint() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let landing = (
                ACTIVATION + 1 + JUMP_THRESHOLD + 10,
                B256::repeat_byte(0x55),
            );
            // holds_l1 = true so the post-jump `holds()` probe passes.
            let (committees, fx) = fixture_with_committee(landing, true, false);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            *fx.upstream.latest.lock().unwrap() = Some(certify(&fx.committee, 0, &live));
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &fx.el,
                Some(B256::repeat_byte(0x1a)),
                ACTIVATION,
                &mut ctx,
            )
            .await;
            assert!(
                matches!(out, JumpOutcome::Landed { .. }),
                "must reseed at the landing under the L1 trust anchor"
            );
            assert_eq!(*fx.el_sync_calls.lock().unwrap(), 1, "synced to live");
        });
    }

    /// A landing that does not advance the resolved anchor (lagging
    /// get_latest, upstream reorg) must NOT reseed backward — [`JumpOutcome::Lagging`].
    #[test]
    fn stale_jump_landing_does_not_reseed_backward() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            // The EL-sync lands AT the anchor — must not move it.
            let (committees, fx) = fixture((ACTIVATION, ANCHOR_HASH), true);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            *fx.upstream.latest.lock().unwrap() = Some(certify(&fx.committee, 0, &live));
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &fx.el,
                None,
                ACTIVATION,
                &mut ctx,
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

    /// A `sync_to` transport stall is classified `Stalled` (NON-fatal) — NOT a
    /// fatal `?`-propagated error. This is the steady-state transient-stall fix:
    /// the executor's completion arm keeps the loop running on `Stalled` and
    /// retries on the next `Update::Tip`. (The cold-start `jump_landing_or_abort`
    /// adapter deliberately re-fuses it to fatal; that mapping is tested in
    /// `dpos.rs`.)
    #[test]
    fn sync_to_stall_is_classified_stalled() {
        struct StallingElSync;
        impl ElSync for StallingElSync {
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
        runtime.start(|mut ctx| async move {
            let (committees, fx) = fixture((5000, B256::repeat_byte(0xe1)), true);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            *fx.upstream.latest.lock().unwrap() = Some(certify(&fx.committee, 0, &live));
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &StallingElSync,
                None,
                ACTIVATION,
                &mut ctx,
            )
            .await;
            let JumpOutcome::Stalled(err) = out else {
                panic!("a sync_to transport error must be Stalled, not AuthFailed/Landed");
            };
            assert!(err.to_string().contains("stalled"), "{err}");
        });
    }

    /// A `sync_to` failure where reth itself declared the served branch
    /// `PayloadStatusEnum::Invalid` (`el_sync_calls >= 1` — discovered DURING the
    /// EL-sync attempt, unlike `BadTarget`'s PRE-sync `el_sync_calls == 0`) is
    /// classified `InvalidTarget`, a SEPARATE variant from both `Stalled` (no
    /// verdict at all) and `BadTarget` (whose `el_sync_calls == 0` invariant,
    /// asserted by `unverifiable_jump_target_is_bad_target_before_sync`, must
    /// stay intact). `jump_landing_or_abort`'s cold-start mapping of
    /// `InvalidTarget` to `Ok(None)` ("boots anyway") is tested in `dpos.rs`.
    #[test]
    fn sync_to_invalid_branch_is_classified_invalid_target() {
        struct InvalidBranchElSync;
        impl ElSync for InvalidBranchElSync {
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
        runtime.start(|mut ctx| async move {
            let (committees, fx) = fixture((5000, B256::repeat_byte(0xe1)), true);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(Digest(B256::repeat_byte(0xaa)), far, B256::repeat_byte(0x44));
            *fx.upstream.latest.lock().unwrap() = Some(certify(&fx.committee, 0, &live));
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &InvalidBranchElSync,
                None,
                ACTIVATION,
                &mut ctx,
            )
            .await;
            let JumpOutcome::InvalidTarget(err) = out else {
                panic!(
                    "an Invalid sync_to verdict must be InvalidTarget, not Stalled/BadTarget/AuthFailed/Landed"
                );
            };
            assert!(err.to_string().contains("REJECTED"), "{err}");
        });
    }

    /// A `sync_to` failure where reth was CONNECTED but its executed head stayed
    /// frozen (the connected-but-wedged pipeline, soak v43 — `SyncFailure::
    /// StalledWithPeers`, DISTINCT from the transport `Stalled`) is classified as
    /// the SEPARATE `JumpOutcome::StalledWithPeers` so the executor completion arm
    /// can bump the observability counter + re-arm without rotating the upstream.
    #[test]
    fn sync_to_stalled_with_peers_is_classified_stalled_with_peers() {
        struct WedgedElSync;
        impl ElSync for WedgedElSync {
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
        runtime.start(|mut ctx| async move {
            let (committees, fx) = fixture((5000, B256::repeat_byte(0xe1)), true);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            *fx.upstream.latest.lock().unwrap() = Some(certify(&fx.committee, 0, &live));
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &WedgedElSync,
                None,
                ACTIVATION,
                &mut ctx,
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
    /// given, so a test can model "wrong height served" and "height absent" without
    /// touching the jump's own `FakeUpstream`.
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
            // A fetch alone injects nothing, and the counter now lives at the injection
            // sites — so it stays flat here.
            assert_eq!(metrics.jump_boundary_refetched.get(), 0);
            assert_eq!(metrics.jump_boundary_refetch_failed.get(), 0);
        });
    }

    /// The height pin. Nothing else binds the response to the request: the structural
    /// check ties the cert only to the block it arrived with, and the epoch comes from
    /// the cert's own round — so a perfectly valid finalization for a DIFFERENT height
    /// passes every other check and would be stored under the index we asked for.
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

    /// P2: `verify_jump_authenticated` treats an unreadable committee as success when
    /// an L1 checkpoint is configured — sound for the LANDING, whose ancestry the
    /// checkpoint authenticates, and unsound for an arbitrary older height. The seam
    /// must therefore never forward the checkpoint.
    #[test]
    fn unreadable_committee_fails_even_with_l1_checkpoint() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let c = committee(23);
            let block = sample_order(Digest(B256::ZERO), 899, B256::ZERO);
            let uf = certify(&c, 0, &block);
            let up = ByHeightUpstream::default();
            up.served.lock().unwrap().insert(899, uf.clone());
            let metrics = SyncMetrics::default();
            let unreadable = seeding_committees(&c, false);

            // The landing path DOES pass with a checkpoint configured — the contrast
            // that makes the seam's choice load-bearing rather than incidental.
            assert!(
                verify_jump_authenticated(
                    &uf,
                    &unreadable,
                    B256::repeat_byte(0x33),
                    Some(B256::repeat_byte(0x99)),
                    &mut ctx,
                )
                .is_ok(),
                "landing verify defers an unreadable committee to the L1 checkpoint"
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
