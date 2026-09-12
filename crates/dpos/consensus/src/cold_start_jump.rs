//! The EL fast-forward — one forkchoice toward an ALREADY-AUTHENTICATED tip.
//!
//! A node whose ordering plane has run far past its EL closes the gap here: one
//! forkchoice (head = safe = finalized = the committee-attested derived hash)
//! lets reth's devp2p backfill canonicalize the branch, and the caller re-seeds
//! its anchor at the landing.
//!
//! **The target is verified BEFORE `sync_to`, and not by this module** (§5.2
//! "Правило единое"). `sync_to` is only ever handed a pair that already carries
//! 2f+1 under a committee the caller read itself:
//!
//!   * the steady-state re-jump (`executor::maybe_re_jump`) reads
//!     `(finalization, block)` out of THIS node's own marshal archive at its own
//!     tip, where the single writer is `store_finalization` and only after
//!     `verify_delivered` (CW `marshal/core/actor.rs:1404-1463`, the
//!     payload↔digest bind at `:987-993`);
//!   * the frontier channel puts every answer through `FrontierHandler::deliver`
//!     first (`plane_upstream.rs`), which rejects a foreign height, a swapped
//!     body and a bad multisig, and now DROPS what it cannot authenticate.
//!
//! So there is no PRE-sync structural stage and no POST-sync committee-BLS stage
//! in the jump any more: both re-ran a check the target had already passed, and
//! both are gone together with `JumpOutcome::BadTarget` / `JumpOutcome::AuthFailed`
//! (pass Б2). [`verify_jump_structural`] and [`verify_jump_authenticated`]
//! survive as FUNCTIONS, and their caller sets differ:
//!
//!   * [`verify_jump_authenticated`] has TWO callers, the by-height seams that
//!     fetch a single finalization outside `deliver`'s reach —
//!     `dpos::refetch_verified_archive_hole` and
//!     `cert_follow::fetch_verified_boundary`;
//!   * [`verify_jump_structural`] has THOSE TWO **and a third**: it IS step (2) of
//!     `FrontierHandler::deliver` itself (`plane_upstream.rs`, the payload↔digest
//!     bind that runs before any committee read). So the bind on the wire is this
//!     function, not a copy of it.
//!
//! **Forward-only.** The jump only moves the anchor/finalized FORWARD (a landing
//! that does not advance the resolved anchor is dropped) — reth ancestor-skips a
//! backward FCU, so a backward jump is both useless and unsafe.
//!
//! **What still runs AFTER `sync_to`**, because both read state the jump itself
//! had to materialize first:
//!
//!   1. THE LANDING CHECK — `el.holds(target.block.result)`. `Valid` is reth's
//!      verdict on the branch it chose to canonicalize, not a statement that the
//!      branch is the ATTESTED one; if the EL does not hold the attested result
//!      canonically it sat down somewhere else ⇒ [`JumpOutcome::InvalidTarget`],
//!      which the steady-state caller takes as `Fault::corruption` (§5.4).
//!   2. The L1 `holds()` re-assert when a checkpoint is configured ⇒
//!      [`JumpOutcome::L1Fork`] on a false, [`JumpOutcome::Stalled`] on a probe
//!      error (a transport failure is not a fork verdict).
//!
//! **Single-writer safety.** Every caller drives reth alone while the jump runs:
//! the follower/validator checkpoint sync runs strictly before
//! `OuterBuilder::build` (the SAME mutual-exclusion `dpos`'s
//! `recover_finalized_tail_into_reth` relies on), and the steady-state re-jump
//! suppresses the executor's heartbeat FCU for its duration
//! (`executor.rs` `jump_done`). It is a prep path, NOT a long-lived second writer.

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

/// Terminal classification of a jump attempt. The caller (the steady-state spawn,
/// or the pre-engine checkpoint sync) owns the entire backfill wait, so there is
/// no in-progress variant — [`jump_to_target`] only ever returns a terminal one.
///
/// There is no `BadTarget` and no `AuthFailed` variant (pass Б2): the structural
/// and committee-BLS stages they classified are gone, because the target is
/// authenticated BEFORE `sync_to` and not by this module (see the module docs).
pub enum JumpOutcome {
    /// A jump landed: re-seed the anchor + finalized cursor at `(landing, hash)`
    /// and advance the running marshal floor to `floor` (= landing − K).
    Landed {
        landing: u64,
        hash: B256,
        floor: u64,
    },
    /// Shallow gap / stale-or-backward landing — no-op (the inlet's ordinary
    /// pulls and the executor gap-walk still cover the residual gap).
    Lagging,
    /// Transport/timeout `sync_to` failure (zero-peers grace or the absolute
    /// ceiling tripped, a generic transport error), OR an L1 `holds()` PROBE that
    /// errored rather than answered — NON-fatal: the steady-state caller retries
    /// on the next `Update::Tip` (the marshal inlet keeps storing frontier certs
    /// while contiguous dispatch is stalled). A genuinely-INVALID served branch is
    /// its own [`InvalidTarget`](JumpOutcome::InvalidTarget) variant, not folded in
    /// here — reth actually rendered a verdict on the branch, which is a different
    /// (and more actionable) condition than "no verdict yet". A probe ERROR is the
    /// same class: no verdict, so nothing may be concluded from it.
    Stalled(eyre::Report),
    /// The EL did not end up on the ATTESTED branch. Two causes reach here and
    /// they say the same thing:
    ///
    ///   * `sync_to` observed reth itself declare the served branch
    ///     `PayloadStatusEnum::Invalid` during the attempt (reth rendered a
    ///     verdict — distinct from `Stalled`, which is no verdict at all);
    ///   * THE LANDING CHECK (§5.2): `sync_to` returned `Valid`, but the
    ///     committee-attested `block.result` is not canonical in the local chain,
    ///     so the EL sat down somewhere else.
    ///
    /// THE STEADY-STATE REACTION IS `Fault::corruption` (§5.4 rows "Посадка не на
    /// заверенную ветку" and "reth Invalid"; review B1-04): the target came out of
    /// this node's OWN attested archive, so no upstream chose it and there is
    /// nobody to rotate away from — the contradiction is between the local EL and
    /// an authenticated certificate.
    InvalidTarget(eyre::Report),
    /// `sync_to` tripped the connected-but-no-progress net
    /// ([`SyncFailure::StalledWithPeers`]): reth had peers > 0 yet its executed
    /// head did not advance for [`EL_SYNC_STALL_ESCAPE`] — the soak-v43 wedge
    /// (a pipeline unwound to a bad ancestor and answers `SYNCING` forever). NON-fatal
    /// and DISTINCT from [`Stalled`](JumpOutcome::Stalled): the divergence root cause
    /// is unknown + deterministic, so a re-jump re-wedges; the steady-state caller
    /// RE-ARMS on the next tip (bumping a counter + ERROR-logging each attempt) and
    /// keeps the refill DEFERRED, so the node stays observable instead of silently
    /// stuck.
    StalledWithPeers(eyre::Report),
    /// The POST-sync L1 `holds()` re-assert ANSWERED false: the EL-synced head
    /// does NOT descend from the L1-FINALIZED checkpoint (#10 — an L1 fork). At
    /// runtime this is a Phase-3 [`SafetyHalt`]: the synced chain conflicts with L1
    /// finality, the strongest trust root, so the node HALTS (verify-only, stops
    /// driving reth, stays observable) rather than exiting. A probe that ERRORED
    /// is [`Stalled`](JumpOutcome::Stalled), not this.
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
/// A trip yields `Stalled`, which the steady-state caller re-attempts on the next
/// `Update::Tip` / heartbeat rather than failing, so oversizing (never false-killing a
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
/// advance for this long — the wedge signature observed in soak v43: reth's
/// pipeline Execution stage hit a re-execution divergence, unwound, marked the
/// block a bad ancestor, and went idle answering
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
pub(crate) const EL_SYNC_STALL_ESCAPE: Duration = Duration::from_secs(300);

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
/// [`jump_to_target`] can route them to different
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

    /// Drive reth onto an OPERATOR-SUPPLIED checkpoint block hash and return the
    /// `(height, hash)` it landed on — the one entry that carries no certificate
    /// at all (§5.2 "Правило единое", the fresh-datadir follower: no geometry, no
    /// committee, nothing local to check a peer's answer against).
    ///
    /// Same FCU shape and same nets as [`Self::sync_to`]; the difference is only
    /// where the hash comes from and that the HEIGHT is learned from the landing
    /// (`block_number(hash)`) instead of being carried on the wire. That is why
    /// the config needs no height field: the operator names a block, and reth —
    /// once it holds it canonically — names its number. A hash reth never
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
    BeaconEngine: BeaconEngineLike + Clone + Send + Sync + 'static,
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

    /// The FCU-drive itself, shared by [`ElSync::sync_to`] (target = the attested
    /// derived hash) and [`ElSync::sync_to_checkpoint`] (target = the operator's
    /// checkpoint hash). Returns once reth DECLARES the target canonical+executed,
    /// or a typed [`SyncFailure`] when one of the three nets trips. `sync_target`
    /// is for the log lines only — the checkpoint entry has no height to name until
    /// the block lands.
    async fn drive_fcu(&self, tip_hash: B256, sync_target: &str) -> Result<(), SyncFailure> {
        // OPTIMISTIC / checkpoint-sync FCU shape: head == safe == finalized == the
        // attested tip. Pointing `finalized` at the (missing) TARGET is what makes reth
        // run the staged PIPELINE backfill (`backfill_sync_target` returns the target ⇒
        // `BackfillAction::Start`); with `finalized = genesis` (already on disk) reth
        // would instead live-download disconnected blocks that never canonicalize. This
        // shape (Part A) is load-bearing — DO NOT change it. Driving reth's EL onto the
        // target is NOT acceptance of anything: the target was authenticated BEFORE
        // this call (the module docs), and the landing check + the L1 `holds()`
        // re-assert in `jump_to_target` still run after it.
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
        //   • `Invalid` ⇒ reth rendered a verdict against the branch — error out as
        //     `SyncFailure::Invalid`, which the caller routes to
        //     `JumpOutcome::InvalidTarget` (a corruption witness, not a stall).
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
                            "EL-sync: reth REJECTED the target {tip_hash:?} ({sync_target}) as \
                             INVALID: {validation_error} — reth rendered a verdict on the branch, \
                             it did not merely fail to finish"
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
                        sync_target,
                        error = %report,
                        "EL-sync: transient FCU probe error; retrying (the backstop ceiling \
                         bounds total wait)"
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
                    // Connected but the executed head is frozen — reth's pipeline
                    // wedged (soak v43 bad-ancestor idle-SYNCING). ERROR (not warn):
                    // this is a stuck EL, not a transient hiccup, and the caller
                    // keeps the node observable + deferred rather than silent.
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
        // F-type: the upstream serves ORDERING artifacts — the only real EVM
        // hash on the wire is the committee-attested `result` (derived hash
        // of tip − K); FCU toward it and let reth devp2p backfill the bodies.
        let tip_hash = latest.block.result;
        let tip_height = latest.block.height.saturating_sub(K);
        if tip_hash == B256::ZERO {
            info!(
                tip = latest.block.height,
                "EL-sync: the target is inside the pre-K window; nothing to EL-sync"
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
            "EL-sync: driving reth toward the attested derived hash"
        );
        self.drive_fcu(tip_hash, &format!("target height {tip_height}"))
            .await?;
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
        // The HEIGHT comes from the landing, which is why the config carries no
        // height field: reth names the number once it holds the block canonically.
        // A `None` here after a `Valid` terminator would be reth contradicting its
        // own verdict — a stall, not a landing.
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

/// Structural check of a SINGLE by-height finalization fetched outside
/// `FrontierHandler::deliver`'s reach: the cert must sign the served body
/// (`cert.payload == block.digest()`).
///
/// **NOT a stage of the jump any more** (pass Б2). The jump's target is
/// authenticated before `sync_to` — out of the node's own marshal archive, whose
/// single writer binds exactly this (CW `marshal/core/actor.rs:987-993` before
/// `store_finalization` at `:1404-1463`), or through `deliver`, which applies the
/// same bind on the wire (`plane_upstream.rs`). Re-running it there checked
/// nothing new, and `JumpOutcome::BadTarget` went with it.
///
/// THREE production callers are left, and the first of them is the wire itself:
///
///   1. `plane_upstream::FrontierHandler::deliver` step (2) — every frontier answer
///      passes THROUGH this function before any committee is read, and an `Err`
///      there is a `false` (the lying peer is excluded). That is the "same bind on
///      the wire" the paragraph above names: it is this code, not a copy of it.
///   2. `dpos::refetch_verified_archive_hole` — the #8 below-floor archive hole,
///      which the marshal's own resolver will not repair.
///   3. `cert_follow::fetch_verified_boundary` — the boundary block below the floor.
///
/// (2) and (3) pull ONE height on their own and so bypass both writers: neither
/// goes through `store_finalization`, so neither inherits its checks — which is why
/// this function still exists as a function.
pub(crate) fn verify_jump_structural(latest: &UpstreamFinalized) -> eyre::Result<()> {
    if latest.finalization.proposal.payload != latest.block.digest() {
        return Err(eyre!(
            "finalization cert payload != block digest at height {}",
            latest.block.height
        ));
    }
    Ok(())
}

/// Committee-BLS authentication of a SINGLE by-height finalization: read
/// `committee[E]` at `at_hash` and verify the 2f+1 multisig against it, FAILING
/// CLOSED on anything else.
///
/// **NOT a stage of the jump any more** (pass Б2). The jump's target already
/// carries 2f+1 under a committee this node read itself — the marshal
/// BLS-verifies in `verify_delivered` before `store_finalization` writes
/// (CW `marshal/core/actor.rs:1404-1463`, scheme selection `:955-990`), and on the
/// frontier channel `FrontierHandler::deliver` checks the same quorum under
/// `committee[round.epoch]` from the committee module. Re-reading the committee at
/// the LANDING and re-running the same multisig was a second opinion on a settled
/// question, and `JumpOutcome::AuthFailed` went with it.
///
/// TWO production callers are left — the by-height seams, each of which pulls one
/// height outside both writers: `dpos::refetch_verified_archive_hole` (at the
/// already-recovered parent's state) and `cert_follow::fetch_verified_boundary` (at
/// the boundary's own read hash). Unlike [`verify_jump_structural`], this function
/// is NOT a step of `FrontierHandler::deliver`: `deliver` runs its own committee
/// read and BLS check inline (step (4)), because the two verdicts differ there — an
/// unreadable committee is a DROP on the wire and a REFUSAL here.
///
/// There is no L1 fallback arm: it existed for the far-ahead jump target whose
/// committee might not be committed even at the landing, and BOTH surviving
/// callers ask about a height at or below their own anchor, where the committee is
/// either readable or the node has a real read fault. An unreadable committee here
/// is therefore a refusal, not a reason to fall back on an operator hash.
///
/// Cold-start / boundary verify: no local beacon key is resolvable at these call
/// sites, so `oracle = None` ⇒ vote-only cert verify — the accepted residual
/// window (bug 2 sign-off item 1).
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

/// The forward-only EL fast-forward, over a target the CALLER supplies and a
/// caller-supplied `jump_threshold`.
///
/// **Where the target comes from is the whole §5.2 change, and it is no longer
/// this function's business to re-check it.** The steady-state re-jump reads
/// `(finalization, block)` out of its OWN marshal archive at its own tip
/// (`executor::maybe_re_jump`), where the single writer is `store_finalization`
/// after `verify_delivered` (CW `marshal/core/actor.rs:1404-1463`) — the target
/// arrives already committee-authenticated and cannot be chosen by a peer. There
/// is no caller left that hands in an unauthenticated answer: the pre-engine
/// `cold_start_jump` wrapper (which passed a raw `CertUpstream::get_latest`) is
/// gone with pass Б2.
///
/// `jump_threshold` is `min(JUMP_THRESHOLD, epoch_block_interval)` on both
/// production paths, so the jump preempts the ≥2-epoch "committee[E] not
/// committed" defer deadlock at ANY epoch interval (real-prod epochs ≫ 1024 keep
/// 1024 unchanged; a compressed test epoch scales the gate down to ~1 epoch). The
/// deadlock is epoch-relative, so its recovery gate is too.
///
/// Classification ([`JumpOutcome`]):
///   - shallow gap / stale-or-backward landing ⇒ [`Lagging`];
///   - `el.sync_to` transport/timeout stall ([`SyncFailure::Stalled`]), or an L1
///     `holds()` probe that ERRORED ⇒ [`Stalled`] (NON-fatal; no verdict was
///     rendered, so nothing may be concluded);
///   - `el.sync_to` observes reth declare the served branch INVALID, or the
///     landing does not hold the attested `block.result` ⇒ [`InvalidTarget`],
///     which the steady-state caller takes as `Fault::corruption` (§5.4; review
///     B1-04);
///   - a configured L1 checkpoint that the landing does not descend from ⇒
///     [`L1Fork`] (SafetyHalt);
///   - success ⇒ [`Landed`].
///
/// [`Lagging`]: JumpOutcome::Lagging
/// [`InvalidTarget`]: JumpOutcome::InvalidTarget
/// [`L1Fork`]: JumpOutcome::L1Fork
/// [`Stalled`]: JumpOutcome::Stalled
/// [`Landed`]: JumpOutcome::Landed
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
    // Forward-only need-gate: only re-run the EL-sync phase for a gap beyond
    // `jump_threshold`.
    if latest.block.height <= anchor + jump_threshold {
        return JumpOutcome::Lagging;
    }
    // A `sync_to` transport/timeout stall is NON-fatal: the steady-state caller
    // retries on the next `Update::Tip`. A genuinely-INVALID served branch is a
    // DIFFERENT outcome (`InvalidTarget`, NOT `Stalled`) — reth rendered an actual
    // verdict on a branch the committee attested.
    let (landing_h, landing_hash) = match el.sync_to(&latest).await {
        Ok(landing) => landing,
        Err(SyncFailure::Invalid(e)) => return JumpOutcome::InvalidTarget(e),
        Err(SyncFailure::StalledWithPeers(e)) => return JumpOutcome::StalledWithPeers(e),
        Err(SyncFailure::Stalled(e)) => return JumpOutcome::Stalled(e),
    };
    // A landing that does not advance the resolved anchor (stale target,
    // upstream reorg) must not reseed backward.
    if landing_h <= anchor {
        return JumpOutcome::Lagging;
    }
    // THE LANDING CHECK (§5.2 "Порядок «аутентификация до `sync_to`»"): reth
    // reported `Valid`, but `Valid` is reth's verdict on the branch it chose to
    // canonicalize — it is not a statement that the branch is the ATTESTED one.
    // `target.block.result` is the committee-attested derived hash of `height − K`
    // and the only real EVM hash on this wire; if the EL does not hold it
    // canonically after the sync, the EL sat down somewhere else.
    //
    // `holds` and not `landing_hash == result`, and the difference is not
    // cosmetic: `sync_to` returns `block_hash(max(height − K, activation))` and
    // SHORT-CIRCUITS to the local executed head on two legitimate paths — a
    // `result` inside the pre-K window (`B256::ZERO`) and a node already executed
    // past the target. On both, `landing_hash` is a perfectly good hash of a
    // different height, so a byte comparison would call a healthy node corrupt.
    // `block_number(result).is_some()` asks the question the design asks — is the
    // attested branch ours — at every one of them. The `ZERO` case is the one
    // where there is nothing to ask: the tip is inside the pre-K window, no
    // derived hash exists yet, and `sync_to` did not drive the EL at all.
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
            // A transport error on the probe is not a verdict: retry on the next
            // tip rather than declare the landing wrong.
            Err(e) => return JumpOutcome::Stalled(e),
        }
    }
    // L1 re-assert when configured: the synced head must descend from the
    // L1-finalized block.
    if let Some(l1) = l1_checkpoint {
        match el
            .holds(l1)
            .wrap_err("L1 checkpoint probe after jump failed")
        {
            Ok(true) => {}
            Ok(false) => {
                // #10: the synced head does not extend L1-finalized history — an L1
                // fork. `L1Fork` so the runtime caller SafetyHalts.
                return JumpOutcome::L1Fork(eyre!(
                    "L1 Rollup checkpoint {l1:?} is NOT in the local chain after an EL-sync \
                     jump — the synced head does not extend the L1-finalized history (possible \
                     upstream equivocation); refusing to follow"
                ));
            }
            // A transport error on the probe itself is not a fork verdict, and
            // there is no longer an upstream to rotate away from: it is exactly
            // the "no verdict" class `Stalled` names, re-evaluated on the next tip.
            Err(e) => return JumpOutcome::Stalled(e),
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
        "EL-sync jump: fast-forwarded the anchor"
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
/// by the `smoke-cert-cascade` smoke (`verdicts_follow.L1_VERIFIED_LINE` /
/// `BOGUS_REJECT_LINE`) and MUST survive verbatim. The phase-3 one is now the
/// case's SOLE witness of the refusal: the container-state witness that used to
/// back it up was removed for reading an OOM as a working trust root.
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

/// PIN — the prune configuration must keep the lookup [`RethElSync::holds`] reads.
///
/// `holds` resolves the L1 checkpoint through
/// [`reth_storage_api::BlockNumReader::block_number`], which on the pinned fork is one read of
/// the `HeaderNumbers` table (hash → number,
/// `crates/storage/provider/src/providers/database/provider.rs:1807-1808`);
/// [`assert_l1_checkpoint`] does the same. Neither has a fallback: if that lookup were ever
/// pruned away, the post-EL-sync L1 trust-root re-assert would read `None` and the node would
/// refuse a chain it is actually on.
///
/// Nothing in this workspace constructs a `PruneModes`. `bins/fluent` runs reth's own `Cli`, so
/// the profile is whatever `--full` / `--minimal` / `--prune.*` produce (the devnet passes
/// `--full`; `devnet/local-dpos-smoke/dpos_harness/stack/compose_gen.py`, `SIM_PRUNE_PROFILE`),
/// and a reth bump that made headers prunable again would land with no call site in this repo to
/// notice it. So the pin is taken where the property actually lives — over reth's prune surface
/// itself, which every profile is a subset of:
///
///  * the `PruneModes` destructure has NO `..` rest pattern, so a new prunable field stops this
///    crate's tests COMPILING until someone classifies it here;
///  * every segment the pruner can be handed is swept against the tables it deletes.
///    `reth_prune` builds its entire segment set by destructuring those same seven fields
///    (`crates/prune/prune/src/segments/set.rs::from_components`), so the two halves cover the
///    same surface from both directions.
///
/// This is deliberately not an assertion about a struct's shape: the per-segment table lists are
/// reth's own (each segment's doc comment in `crates/prune/types/src/segment.rs` and its
/// implementation under `crates/prune/prune/src/segments/user/`), and the test goes red the
/// moment any configurable segment claims a table the hash → number resolution needs.
///
/// What it does NOT catch: an EXISTING segment whose implementation starts deleting
/// `HeaderNumbers` under an unchanged name — the per-segment table lists here are a hand
/// transcription, and reth cannot invalidate them. It fires on a new segment, a new
/// `PruneModes` field, a rename, or a changed count.
#[cfg(test)]
mod prune_config_pin {
    use reth_prune_types::{PruneModes, PruneSegment};

    /// The tables the hash → number resolution lives in. `HeaderNumbers` is the one `holds`
    /// reads; the other two are what the retired `PruneSegment::Headers` used to take with it,
    /// kept here so an un-deprecation is caught by table name as well as by segment name.
    const HASH_TO_NUMBER_TABLES: [&str; 3] = ["HeaderNumbers", "CanonicalHeaders", "Headers"];

    /// What each configurable segment deletes, transcribed from reth's own segment docs and
    /// implementations. The catch-all is the point of the function: a segment nobody has
    /// classified is treated as a threat to the lookup, not waved through.
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
        // COMPILE-TIME half. `all()` is the most aggressive profile reth offers — a strict
        // superset of `--full`. No `..`: a new prunable field breaks this line, and the fix is
        // to classify the new segment in `deleted_tables`, never to widen the pattern.
        let PruneModes {
            sender_recovery: _,
            transaction_lookup: _,
            receipts: _,
            account_history: _,
            storage_history: _,
            bodies_history: _,
            receipts_log_filter: _,
        } = PruneModes::all();

        // RUNTIME half: every segment the pruner can be configured with, whatever profile
        // selected it.
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
        /// The attested `block.result` this EL actually canonicalized — recorded
        /// by `sync_to`, because that is what a healthy EL does: it is told to
        /// drive onto `latest.block.result` and it lands there.
        landed_on: Arc<Mutex<Option<B256>>>,
        /// Make `sync_to` report success WITHOUT landing on the attested result:
        /// reth answered `Valid` for some other branch. The condition the §5.2
        /// landing check exists for, and the only way to reach it here.
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

    /// A deep gap above [`JUMP_THRESHOLD`] drives the EL-sync once and re-seeds
    /// the anchor + floor at the landing (`floor == landing − K`), and it reads NO
    /// committee at all: after pass Б2 the target is authenticated before it gets
    /// here, so a `scheme_at` call from this path would be the deleted stage
    /// reappearing.
    ///
    /// Falsifier: a non-`Landed` outcome; a floor that is not `landing − K`; an
    /// `el_sync_calls` other than 1; ANY committee read.
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

    /// (4.2 Б1.2) THE LANDING CHECK. `sync_to` returned `Valid`, but `Valid` is
    /// reth's verdict on the branch it chose — not a statement that the branch is
    /// the ATTESTED one. After the sync the jump asks the EL whether it holds the
    /// committee-attested `block.result` of the target; if it does not, the EL sat
    /// down somewhere else and the outcome is `InvalidTarget` (§5.4: a corruption
    /// witness, not a stall and not a rotate-the-peer).
    ///
    /// Both arms in one test, over ONE fixture, because the separating fact is
    /// exactly one bit: whether the fake EL landed on what it was told to.
    ///
    /// RED under the mutation that disables the check (`if false && …` on the
    /// `latest.block.result != B256::ZERO` guard in `jump_to_target`): `a landing
    /// off the attested branch was accepted; expected InvalidTarget` — the
    /// refusing arm fell through to a terminal that is NOT `InvalidTarget`, and
    /// with `l1_checkpoint = None` and a readable committee the only one left is
    /// `Landed { landing: 5000 }`: a jump that reseeds the anchor onto a branch
    /// nobody attested. (Run 2026-09-12; the mutation was reverted.)
    ///
    /// Falsifier: a `Landed` from the refusing arm (no check); an `InvalidTarget`
    /// from the holding arm (the check fires on an honest landing); an
    /// `el_sync_calls` of 0 in either (the fixture never reached the check).
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

            // (1) The EL reports Valid but does NOT hold the attested result.
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

            // (2) Same fixture, same target, the EL lands where it was told.
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

    /// (4.2 Б1.2) A target inside the pre-K window carries `result ==
    /// B256::ZERO`: nothing was derived at `height − K` yet, `sync_to` drives the
    /// EL nowhere, and there is no attested branch to ask about. The landing check
    /// must not fire there — that is the one case where a byte comparison against
    /// the returned `landing_hash` would have called a healthy node corrupt.
    ///
    /// Falsifier: an `InvalidTarget` (the ZERO case is not excused).
    #[test]
    fn a_pre_k_target_with_no_derived_result_skips_the_landing_check() {
        let runtime = deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            let (_committees, fx) = fixture((5000, B256::repeat_byte(0xe1)), false);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            // `result == ZERO` and the EL refuses to record ANY landing: even so,
            // the check has nothing to ask and the jump proceeds.
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

    /// With an L1 checkpoint configured and the synced head NOT descending from
    /// it, the post-jump `holds()` probe ANSWERS false and the jump is an
    /// [`JumpOutcome::L1Fork`] (the SafetyHalt verdict).
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

    /// (4.2 Б2.4/Б2.7б) THE JUMP NO LONGER JUDGES ITS TARGET'S SIGNATURES. A pair
    /// whose multisig was built by a DIFFERENT committee — structurally intact
    /// (`payload == digest`), cryptographically worthless — LANDS, provided the EL
    /// holds the attested `block.result`. Both verify stages are gone: the target of
    /// a real jump came out of this node's own marshal archive, where
    /// `store_finalization` writes only what `verify_delivered` accepted, and a
    /// frontier answer met `FrontierHandler::deliver` first.
    ///
    /// RED BEFORE THIS CHANGE, verbatim: on HEAD `f8ec4939` the same input returned
    /// `JumpOutcome::AuthFailed` from `verify_jump_authenticated` ("jump target
    /// finalization FAILED BLS verification against committee[0] …"), so the
    /// `else` branch below panicked with `expected Landed`.
    ///
    /// POSITIVE CONTROL — the property that makes this safe is that such a pair
    /// never REACHES a jump. It is pinned where the wire is, not duplicated here:
    /// `plane_upstream::tests::a_multisig_that_fails_under_a_readable_committee_is_a_lie`
    /// (a cert whose multisig does not verify under the committee this node CAN read
    /// ⇒ `deliver` returns `false`, reason `bls`, and the marshal is not driven).
    ///
    /// Falsifier: any outcome other than `Landed` (a signature stage survives); an
    /// `el_sync_calls` of 0 (the fixture never reached the jump body); a committee
    /// read (the POST-sync stage is back).
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
            // An INDEPENDENT committee signs the target: `payload == digest` holds,
            // the multisig does not verify against the fixture's committee.
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

    /// (4.2 Б2.4) A structurally-broken target (`cert.payload != block.digest()`) is
    /// no longer refused by the jump either: the PRE-sync structural stage went with
    /// the authenticated one, for the same reason (the marshal binds body↔payload
    /// before it stores, CW `marshal/core/actor.rs:987-993`; `deliver` binds it on
    /// the wire). The jump drives the EL and lands.
    ///
    /// RED BEFORE THIS CHANGE, verbatim: on HEAD `f8ec4939` this returned
    /// `JumpOutcome::BadTarget` with `el_sync_calls == 0`.
    ///
    /// Falsifier: a non-`Landed` outcome; an `el_sync_calls` of 0 (the PRE-sync
    /// gate is back and the EL was never driven).
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
            // The cert signs a DIFFERENT block than the one served.
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

    /// (4.2 Б2.4) An L1 `holds()` probe that ERRORS is [`JumpOutcome::Stalled`], not
    /// a fork verdict: a transport failure says nothing about ancestry, and there is
    /// no upstream left to rotate away from. Distinct from
    /// `jump_reasserts_l1_checkpoint_and_fails_closed`, where the probe ANSWERS
    /// false and the verdict is `L1Fork`.
    ///
    /// Falsifier: an `L1Fork` (a probe error is read as a fork); a `Landed` (the
    /// probe error is swallowed and the anchor moves onto an unchecked branch).
    #[test]
    fn an_l1_probe_error_is_stalled_not_a_fork_verdict() {
        /// Lands on the attested result, then FAILS the L1 probe with a transport
        /// error. `landed_on` is what tells the two probes apart, exactly as
        /// `FakeElSync` does.
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

    /// A landing that does not advance the resolved anchor (stale target,
    /// upstream reorg) must NOT reseed backward — [`JumpOutcome::Lagging`].
    #[test]
    fn stale_jump_landing_does_not_reseed_backward() {
        let runtime = deterministic::Runner::default();
        runtime.start(|_ctx| async move {
            // The EL-sync lands AT the anchor — must not move it.
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

    /// A `sync_to` transport stall is classified `Stalled` (NON-fatal) — NOT a
    /// fatal `?`-propagated error. This is the steady-state transient-stall fix:
    /// the executor's completion arm keeps the loop running on `Stalled` and
    /// retries on the next `Update::Tip`.
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

    /// A `sync_to` failure where reth itself declared the served branch
    /// `PayloadStatusEnum::Invalid` is classified `InvalidTarget` — reth rendered a
    /// verdict, which is a different (and more actionable) condition than `Stalled`'s
    /// "no verdict at all". The executor's steady-state reaction, `Fault::corruption`,
    /// is tested in `executor.rs`.
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

    /// A `sync_to` failure where reth was CONNECTED but its executed head stayed
    /// frozen (the connected-but-wedged pipeline, soak v43 — `SyncFailure::
    /// StalledWithPeers`, DISTINCT from the transport `Stalled`) is classified as
    /// the SEPARATE `JumpOutcome::StalledWithPeers` so the executor completion arm
    /// can bump the observability counter + re-arm without rotating the upstream.
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

    /// An unreadable committee is a REFUSAL, unconditionally.
    ///
    /// WHAT THIS TEST PROVED BEFORE (4.2 Б2.4). It was
    /// `unreadable_committee_fails_even_with_l1_checkpoint` and it pinned an
    /// ASYMMETRY: `verify_jump_authenticated` answered `Ok` on an unreadable
    /// committee when an L1 checkpoint was passed (sound for a far-ahead jump
    /// LANDING, whose ancestry the checkpoint authenticates), so the boundary seam
    /// had to be careful never to forward one. WHAT IT PROVES NOW: there is no such
    /// arm left. The L1 fallback existed for the jump's POST-sync stage, that stage
    /// is gone, and both surviving callers
    /// (`dpos::refetch_verified_archive_hole`, `cert_follow::fetch_verified_boundary`)
    /// ask about a height at or below their own anchor — so the asymmetry the seam
    /// had to defend against no longer exists and the refusal is unconditional.
    ///
    /// Falsifier: an `Ok` from the direct call; a `Some` from the seam; a
    /// `jump_boundary_refetch_failed` that did not move.
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
