//! Networked live-DKG actor: wraps [`DkgCeremony`] and drives committee[E]'s
//! self-DKG over `BEACON_CHANNEL` during epoch E-1.
//!
//! Single-ceremony-per-epoch, NO Muxer: each `DkgMsg` carries its `ceremony_epoch`.
//! A dealing (`Commitment`/`Share`) that arrives for a near-future epoch BEFORE this
//! node started its own ceremony for it is BUFFERED (`pending`, drained by
//! `maybe_start`) so the start-race never silently drops it; any other message not for
//! an active ceremony is dropped (epoch-tag filter). Ceremonies for E and E+1 are
//! temporally disjoint (the collection window spans ~all of E-1), so at most a couple
//! are in flight.
//!
//! Lifecycle, driven by the finalized-height stream + chain committee reads:
//! - entering epoch E-1 (committee[E] != committee[E-1] AND this node ∈
//!   committee[E]) → `DkgCeremony::start`, broadcast commitment + send shares;
//! - finalized height reaches `epoch_start(E) - DKG_MARGIN_BLOCKS` → `seal_dealings`
//!   (broadcast the signed log);
//! - once a sealed ceremony has a selectable quorum AND its log set has SETTLED
//!   (all-in, or the deterministic settle deadline — so every honest node selects the
//!   IDENTICAL set ⇒ identical `PK_E`; probed event-driven on each recording, our seal
//!   or an incoming `Reveal`, via [`DkgActor::drive_finalization`]) →
//!   `DkgCeremony::finalize` → memoize `(PK_E, share)` into the per-epoch
//!   [`CeremonyStore`] + fire `share_notify`. The consensus verify
//!   path reads the share for the C share-on-polynomial gate, `propose` reads
//!   `PK_E` for the boundary `beacon_outcome`, and Phase 5's finalized-boundary
//!   swap reads both for the per-epoch signing slot + `commitEpochBeaconKey`.
//!
//! The actor never finalizes over a locally-selected Q before sealing, and never
//! over an under-quorum log set (`ready` gates it). <quorum valid logs → no store
//! entry → the beacon naturally stalls for that epoch (option A), not a crash.
//!
//! Mid-window restart durability (§8.11.1): ceremony progress is journaled to
//! `beacon-dkgjournal-e<E>.bin`; on restart `maybe_start` RESUMES PLAYER-ONLY
//! (`DkgCeremony::resume` — never re-deals) and re-fetches missing peer logs via the
//! DKG-log recovery resolver (`fetch_missing_logs`/`on_resolver_message`, the
//! `commonware_resolver::p2p` engine on `BEACON_RESOLVER_CHANNEL`), so a routine
//! restart no longer leaves the member shareless + liveness-slashed.

use crate::beacon::{
    ceremony::{checked_serve_map, CeremonyOutput, DkgCeremony, Outgoing, Target},
    dkg_msg::{DealerReveal, DkgBody, DkgMsg},
    log_resolver::{DkgLogKey, LogMessage},
    share_state::{self, JournalLoad, JournalRecord, ShareState},
    wire::BeaconMessage,
};
use bytes::Bytes;
use commonware_codec::{Encode as _, Read as _, ReadExt as _};
use commonware_cryptography::{
    bls12381::primitives::group::Share, ed25519::PrivateKey as Ed25519PrivateKey, Signer as _,
};
use commonware_p2p::{Receiver, Recipients, Sender};
use commonware_resolver::Resolver;
use commonware_utils::{ordered::Set, vec::NonEmptyVec};
use fluentbase_bls::PeerPubkey;
use rand_core::CryptoRngCore;
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU32,
    path::PathBuf,
    sync::{Arc, RwLock},
};

/// Blocks of slack before the epoch-E boundary at which dealing collection closes
/// and dealers seal (broadcast their signed logs) — the echo-settle tail. Pinned
/// off the on-chain `epochBlockInterval`, not an absolute window (see Q4).
pub const DKG_MARGIN_BLOCKS: u64 = 10;

/// Blocks AFTER the seal deadline to keep collecting peer logs before finalizing
/// over whatever valid set has settled — the deterministic fallback when a dealer is
/// genuinely absent (the all-present case finalizes earlier, the instant the last
/// `Reveal` lands). Bounds the canonical-set wait so every honest node selects over
/// the IDENTICAL log set at the identical height ⇒ identical `PK_E` ⇒ the C gate
/// passes. MUST stay `< DKG_MARGIN_BLOCKS` so finalize lands before the boundary.
pub const DKG_SETTLE_BLOCKS: u64 = 4;
const _: () = assert!(
    DKG_SETTLE_BLOCKS < DKG_MARGIN_BLOCKS,
    "settle window must finalize before the epoch boundary"
);

/// The epoch the beacon goes live at, deterministically. `committee[2]` runs its
/// DKG during epoch 1 EVEN IF unchanged from `committee[1]`, so a long-stable
/// initial committee still seeds the beacon (on-change-only activation would
/// leave it seedless indefinitely). Epoch 1 stays seedless (`order.digest()`);
/// on-change re-DKG + carry-forward apply thereafter. The same constant gates the
/// `application::is_change_epoch_first_block` boundary so the two never drift.
pub const DETERMINISTIC_BOOTSTRAP_EPOCH: u64 = 2;

/// Test-only counter of cold-cache journal parses, so the fetch-burst-bound test can
/// assert "one parse per epoch, not per request" (the DoS-is-one-shot property Option C
/// relies on). Incremented in `cold_load_serve_cache` on each disk parse.
#[cfg(test)]
pub(crate) static COLD_PARSE_COUNT: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// The agreed DKG result for an epoch this node is a MEMBER of: the group output
/// (`PK_E` + public polynomial) and this node's secret share, memoized by the
/// actor during the post-seal margin window — BEFORE the epoch-E boundary block
/// is proposed/verified. Read by the consensus verify/propose path (the C gate's
/// share + the proposer's `beacon_outcome`) and by Phase 5's signing-slot swap at
/// the finalized boundary. Non-members never get an entry (⇒ observer ⇒ withhold).
pub type CeremonyStore = Arc<RwLock<BTreeMap<u64, (CeremonyOutput, Share)>>>;

/// Resolves committee[epoch] (the Commonware-ordered peer set) at a finalized
/// state hash — provided by the launch site over the staking reader.
pub type CommitteeFor = Arc<dyn Fn(u64) -> Option<Set<PeerPubkey>> + Send + Sync>;

/// `recv()` on an optional mpsc receiver, or park forever when it is `None` — so the
/// resolver branch of the actor's `select!` is inert on a node with no resolver
/// wired (in-process / test default) without a second loop shape.
async fn recv_or_never(rx: Option<&mut tokio::sync::mpsc::Receiver<LogMessage>>) -> Option<LogMessage> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

/// The networked DKG actor. Generic over the p2p sender/receiver (the spawn site
/// passes the `BEACON_CHANNEL` halves) and over the DKG-log recovery resolver `R`
/// (the `commonware_resolver::p2p::Mailbox` in production; a no-op in unit tests).
/// Testable with mock channels.
pub struct DkgActor<Se, Re, R> {
    namespace: Vec<u8>,
    me_key: Ed25519PrivateKey,
    sender: Se,
    receiver: Re,
    /// Mailbox to the beacon-plane DKG-log recovery resolver — a shorthanded
    /// ceremony `fetch_targeted`s its missing dealer logs through it (replacing the
    /// former best-effort `BEACON_CHANNEL` `LogRequest` gossip pull). `None` ⇒ no
    /// resolver wired (in-process/test default) ⇒ recovery is gossip-only.
    resolver: Option<R>,
    /// Inbound `Produce`/`Deliver` requests from the resolver engine
    /// (`log_resolver::LogHandler`), served against the live ceremonies + persisted
    /// journal in the single-threaded run loop.
    resolver_rx: Option<tokio::sync::mpsc::Receiver<LogMessage>>,
    committee_for: CommitteeFor,
    store: CeremonyStore,
    /// Edge-trigger fired (`notify_one`) the instant a share lands in `store`, so a
    /// racing `epoch_manager::enter` wakes immediately instead of polling. The SAME
    /// `Arc` is held by the manager (threaded via `SharedBeaconPlane`).
    share_notify: Arc<tokio::sync::Notify>,
    /// Frozen `(dposActivationBlock, epochBlockInterval)` — the immutable epoch
    /// geometry, resolved ONCE by the beacon plane's `EpochTransition` (the single
    /// in-plane source) and handed in as plain values at spawn AFTER that freeze.
    /// The actor never re-reads the chain for it, so there is no codeless/genesis-
    /// fallback race in this path: the spawn site only constructs the actor once the
    /// geometry is frozen (see `build_beacon_plane`).
    dpos_activation: u64,
    epoch_interval: u64,
    metrics: crate::beacon::metrics::BeaconMetrics,
    /// Directory for on-disk persistence of the live-DKG per-epoch shares this
    /// actor memoizes into [`CeremonyStore`] — the always-on plane passes
    /// `<datadir>/beacon/` (see `node/dpos.rs::build_beacon_plane`), reloaded once
    /// at plane startup. `None` ⇒ no persistence dir (in-process/test default) ⇒
    /// memoized shares stay in-memory only (lost on restart).
    share_dir: Option<PathBuf>,
    /// At-rest framing for the persisted shares: [`ShareState::Encrypted`] (the
    /// HKDF-derived seal key) on a keystore-mode validator, [`ShareState::Plaintext`]
    /// otherwise. Built from the `Option<ShareSealKey>` the plane derives at launch
    /// (gated on `--dpos.bls-keystore-path`).
    share_state: ShareState,
    /// Active ceremonies keyed by their target epoch E.
    ceremonies: BTreeMap<u64, DkgCeremony>,
    /// Bounded serve cache: the recorded signed logs of a FINALIZED-but-not-yet-past-
    /// boundary epoch, so the DKG-log recovery `Producer` (`serve_log`) keeps serving
    /// them O(1) (no disk read, no per-request `check` on the actor's hot path). It is
    /// a strict subset-COPY of the durable journal, NOT a second source of truth:
    /// seeded eagerly at finalize (the no-restart path never touches disk) AND lazily
    /// on a cold `serve_log` miss after a restart (parse the epoch journal ONCE,
    /// re-`check`, cache). Evicted at the boundary sweep on the SAME `*e > now`
    /// predicate as the journal, so a restart re-reads from disk with nothing to
    /// repopulate (R1 closed by construction). `Arc` so a serve clones a cheap handle.
    serve_cache: BTreeMap<u64, Arc<BTreeMap<PeerPubkey, DealerReveal>>>,
    /// Whether the one-shot startup journal reconcile has run. Driven on the actor's
    /// FIRST `on_height` tick (where the frozen epoch geometry is finally available),
    /// it deletes every boundary-passed journal off disk — reclaiming orphaned
    /// journals + stale at-rest secrets a finalize-then-restart-before-boundary left
    /// (the running sweep cannot, since the restart wiped the in-memory keys it scans).
    reconciled_journals: bool,
    /// Dealings (`Commitment`/`Share`) that arrived for an epoch BEFORE this node
    /// started its own ceremony for it — the start-race. Drained into the ceremony by
    /// `maybe_start` before any seal, so a peer dealing that raced ahead of our start
    /// is never silently dropped (which would leave that dealer un-acked ⇒
    /// `TooManyReveals` ⇒ `DkgFailed`). Bounded: only the next 1–2 epochs are
    /// bufferable (`is_bufferable`), ≤ `MAX_COMMITTEE_SIZE * 2` entries per epoch, and
    /// stale epochs are evicted each height tick.
    pending: BTreeMap<u64, Vec<(PeerPubkey, DkgBody)>>,
    /// Last finalized height seen on the `on_height` stream — the current chain time
    /// the event-driven `on_message` finalize uses for its deterministic-settle gate.
    last_height: u64,
    /// Target epochs whose committee first-became-readable has been logged
    /// (one-shot diagnostic; see `maybe_start`).
    eval_logged: BTreeSet<u64>,
    /// Target epochs whose `JournalLoad::Torn` sit-out has already been warned —
    /// `maybe_start` re-runs every height tick (no ceremony is inserted to
    /// short-circuit it), so without this the Torn warn floods the log ~once per
    /// second for a whole epoch. One-shot per epoch, like `eval_logged`.
    torn_warned: BTreeSet<u64>,
}

impl<Se, Re, R> DkgActor<Se, Re, R>
where
    Se: Sender<PublicKey = PeerPubkey>,
    Re: Receiver<PublicKey = PeerPubkey>,
    R: Resolver<Key = DkgLogKey, PublicKey = PeerPubkey>,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        namespace: Vec<u8>,
        me_key: Ed25519PrivateKey,
        sender: Se,
        receiver: Re,
        resolver: Option<R>,
        resolver_rx: Option<tokio::sync::mpsc::Receiver<LogMessage>>,
        committee_for: CommitteeFor,
        store: CeremonyStore,
        share_notify: Arc<tokio::sync::Notify>,
        dpos_activation: u64,
        epoch_interval: u64,
        metrics: crate::beacon::metrics::BeaconMetrics,
        share_dir: Option<PathBuf>,
        share_state: ShareState,
    ) -> Self {
        Self {
            namespace,
            me_key,
            sender,
            receiver,
            resolver,
            resolver_rx,
            committee_for,
            store,
            share_notify,
            dpos_activation,
            epoch_interval,
            metrics,
            share_dir,
            share_state,
            ceremonies: BTreeMap::new(),
            serve_cache: BTreeMap::new(),
            reconciled_journals: false,
            pending: BTreeMap::new(),
            last_height: 0,
            eval_logged: BTreeSet::new(),
            torn_warned: BTreeSet::new(),
        }
    }

    fn epoch_of(&self, height: u64) -> u64 {
        height.saturating_sub(self.dpos_activation) / self.epoch_interval
    }

    /// First-block height of an epoch (relative to DPoS activation).
    fn epoch_start(&self, epoch: u64) -> u64 {
        self.dpos_activation + epoch * self.epoch_interval
    }

    /// Best-effort append of the ceremony's journal records for `epoch` so a restart
    /// can `resume` (§8.11.1). No-op without a `share_dir` (in-process/test default);
    /// a write failure only warns — the in-memory ceremony stays authoritative and
    /// the unwritten tail is re-fetchable via the DKG-log recovery resolver.
    fn append_journal(&self, epoch: u64, records: Vec<JournalRecord>) {
        let Some(dir) = &self.share_dir else {
            return;
        };
        for record in records {
            if let Err(err) = share_state::append_journal(dir, epoch, &record, &self.share_state) {
                tracing::warn!(
                    epoch,
                    ?err,
                    "live DKG: failed to journal ceremony record (in-memory ceremony unaffected)"
                );
            }
        }
    }

    /// Delete a finalized/swept epoch's ceremony journal (within-window scratch).
    fn evict_journal(&self, epoch: u64) {
        if let Some(dir) = &self.share_dir {
            share_state::evict_journal(dir, epoch);
        }
    }

    /// Run until both the height-event stream and the network receiver close.
    /// `heights` carries every finalized block height (tapped from the boundary
    /// hook); the actor derives epoch transitions + the seal deadline from it.
    pub async fn run(mut self, mut heights: tokio::sync::mpsc::Receiver<u64>, mut rng: impl CryptoRngCore) {
        tracing::info!(
            activation = self.dpos_activation,
            interval = self.epoch_interval,
            "live DKG: actor started"
        );
        // The resolver inbound channel is taken out so the loop can borrow it
        // alongside `&mut self`; `None` (no resolver wired) parks the branch forever.
        let mut resolver_rx = self.resolver_rx.take();
        loop {
            tokio::select! {
                maybe_h = heights.recv() => match maybe_h {
                    Some(height) => self.on_height(height, &mut rng).await,
                    None => break,
                },
                msg = self.receiver.recv() => match msg {
                    Ok((from, buf)) => self.on_message(from, buf.as_ref(), &mut rng).await,
                    Err(_) => break,
                },
                // Serve / ingest DKG-log recovery requests from the resolver engine.
                // `recv_or_never` parks forever when no resolver is wired.
                req = recv_or_never(resolver_rx.as_mut()) => match req {
                    Some(msg) => self.on_resolver_message(msg, &mut rng).await,
                    // The resolver engine exited (its peer-set subscription / mailbox
                    // closed) → PARK the inbound branch AND clear the OUTBOUND mailbox so
                    // `fetch_missing_logs` short-circuits (`self.resolver.is_none()`) instead
                    // of computing the fetch-set + issuing no-op `send_lossy` calls every
                    // tick for the life of the process (review [313]). Do NOT break the whole
                    // loop: the height + gossip arms stay live and recovery degrades to
                    // gossip-only (the documented no-resolver behaviour). The engine is
                    // spawned once and never respawned, so the clear is terminal.
                    None => {
                        resolver_rx = None;
                        self.resolver = None;
                    }
                },
            }
        }
    }

    async fn on_height(&mut self, height: u64, rng: &mut impl CryptoRngCore) {
        // Two feeders drive this clock: the local finalized-height poller
        // (`fin + K`) and, during unified-supervisor catch-up, the LIVE upstream
        // cert frontier (so a still-catching-up newcomer deals its first epoch on
        // the live deadline). Take the max so an interleaved lagging tick can never
        // pull the deal/seal clock backward; process at the monotone height.
        self.last_height = self.last_height.max(height);
        let height = self.last_height;
        let now = self.epoch_of(height);

        // First-tick journal reconcile: now that the frozen epoch geometry is finally
        // available (the actor only runs post-`geometry_ready`), delete every boundary-
        // passed journal off disk in one scan — the SAME `epoch <= now` predicate the
        // running sweep uses, but driven off the on-disk filename so a finalize-then-
        // restart-before-boundary (which holds the epoch in NO in-memory map) still
        // reclaims its leaked journal + stale at-rest secrets (R2). One-shot.
        if !self.reconciled_journals {
            if let Some(dir) = &self.share_dir {
                share_state::reconcile_journals(dir, now);
            }
            self.reconciled_journals = true;
        }

        let mut to_send: Vec<Outgoing> = Vec::new();

        // 1. Seal any active ceremony whose collection deadline has passed. A ceremony
        //    that already sealed (or resumed player-only) has `dealing_closed()` — its
        //    dealer is gone — so `seal_dealings` would be a no-op; skip it.
        let due: Vec<u64> = self
            .ceremonies
            .iter()
            .filter(|(e, c)| {
                !c.dealing_closed()
                    && height >= self.epoch_start(**e).saturating_sub(DKG_MARGIN_BLOCKS)
            })
            .map(|(e, _)| *e)
            .collect();
        for e in due {
            if let Some(c) = self.ceremonies.get_mut(&e) {
                let step = c.seal_dealings();
                to_send.extend(step.outgoing);
                self.append_journal(e, step.journal);
            }
        }

        // 1b. Evict pending dealing buffers for epochs we will never start (a
        //     dealing-closed ceremony exists, or the epoch is now in the past) so
        //     `pending` stays O(1–2 live epochs). An absent ceremony means not-yet-
        //     started → still bufferable.
        self.pending.retain(|e, _| {
            *e > now
                && self
                    .ceremonies
                    .get(e)
                    .is_none_or(|c| !c.dealing_closed())
        });

        // 2. Finalize any SEALED ceremony whose log set has SETTLED (all-in, or the
        //    deterministic settle deadline). Also driven from `on_message`, so an
        //    all-in completed by an incoming Reveal finalizes immediately — see
        //    [`Self::drive_finalization`].
        self.drive_finalization(height, rng);

        // 2b. Evict any ceremony/`sealed` entry whose epoch boundary has passed without
        //     finalizing. An under-quorum stall still gets SEALED in step 1 but never
        //     reaches `ready()`, so `drive_finalization` never removes it — without this
        //     sweep it lingers in `ceremonies`/`sealed` and re-probes `ready()` (an n=51
        //     `observe` over cloned logs) on every height tick AND every incoming message
        //     forever. Past its own boundary (`*e <= now`) the ceremony can only ever be
        //     the option-A no-op (the boundary block already needed `PK_e` and is gone),
        //     so dropping it is safe. Runs AFTER `drive_finalization` so a ceremony that
        //     is finalizable on the boundary tick is completed first, never evicted out
        //     from under it. Mirrors the `pending.retain` sweep above.
        let swept: Vec<u64> = self
            .ceremonies
            .keys()
            .copied()
            .chain(self.serve_cache.keys().copied())
            .filter(|e| *e <= now)
            .collect();
        for e in &swept {
            self.evict_journal(*e);
        }
        self.ceremonies.retain(|e, _| *e > now);
        // Reclaim the finalized-epoch serve cache for any epoch past its boundary — the
        // SAME `*e > now` lifetime predicate as the journal evict above and the first-
        // tick reconcile, so the three drivers can never disagree (the cache is a
        // subset-copy of the journal, never outliving it).
        self.serve_cache.retain(|e, _| *e > now);

        // 3. Start the NEXT epoch's DKG ceremony. Retried on EVERY tick (not just
        //    once at the epoch transition): committee[E+1] is committed on-chain
        //    sometime DURING epoch E, which can land AFTER the actor (driven by
        //    lagging finalized heights) first enters E — a single-shot check at the
        //    transition would see the committee still unchanged, carry forward, and
        //    NEVER deal, so the E+1 boundary block wedges (no PK_{E+1}). maybe_start
        //    is idempotent (no-op once the ceremony is in flight, already computed,
        //    the committee is unchanged, or this node is not a member), so retrying
        //    until committee[E+1] is visible+changed is safe.
        self.maybe_start(now + 1, rng, &mut to_send);

        self.broadcast_all(to_send).await;

        // 4. Re-fetch missing dealer logs for any open, shorthanded ceremony (a
        //    restarted/late node that lost peer logs) via the DKG-log recovery
        //    resolver — gated on the open window. The resolver owns retry / multi-peer
        //    fallback / rate-limiting / blocked-peer eviction, so this just hands it
        //    the missing `{epoch, dealer}` keys (deduplicated by the resolver) each
        //    tick; targeting aims at the known committee roster (the holders).
        self.fetch_missing_logs(height).await;
    }

    /// Finalize every SEALED ceremony whose collected log set has SETTLED, memoizing
    /// `(PK_E, share)` into the shared [`CeremonyStore`] over a DETERMINISTIC canonical
    /// log set.
    ///
    /// Event-driven, not clock-polled. A ceremony's log set grows ONLY on our own
    /// `seal_dealings` or an incoming `Reveal` (`DkgCeremony::handle` → `Logs::record`),
    /// so this runs from exactly those two events: `on_height` after sealing, and
    /// `on_message` after handling a peer message. No timer.
    ///
    /// DETERMINISM is the load-bearing property. `select` is already canonical (the
    /// `required_commitments` lowest-keyed valid dealers — a total order every node
    /// shares), so `finalize` yields the byte-identical `PK_E` PROVIDED every honest
    /// node selects over the IDENTICAL log set. The settle gate (in the filter below)
    /// enforces that: finalize fires only once the set is all-in (every committee log
    /// recorded) or the height-deterministic settle deadline has passed. Without it,
    /// nodes finalizing at their FIRST selectable quorum pick DIFFERENT subsets ⇒
    /// divergent `PK_E` ⇒ the boundary "C" share-on-poly gate
    /// (`application::beacon_gate_decision`) rejects (the observed `vrf` wedge). With
    /// it, honest divergence is eliminated by construction; the n=51
    /// `dkg::seed_is_threshold_unique_at_n51` test confirms determinism at production
    /// committee size.
    ///
    /// The `dealing_closed()` gate states the seal-before-finalize contract: we
    /// finalize only AFTER the dealing phase is closed (we sealed — normally or
    /// check-failed — or resumed player-only), never mid-dealing. It is NOT gated on
    /// our own log being in the selected set: `Player::finalize` computes THIS node's
    /// share purely as a player from its `view` (the received dealings) + the canonical
    /// `select`ed dealers' logs (`dkg.rs::Player::finalize`/`Logs::select` — `select` is
    /// a pure function of the recorded `logs`, independent of which node selects), so a
    /// node whose own log is ABSENT (a pre-seal crash that resumed player-only, or a
    /// torn-own-seal resume) still recovers its share over the n−f survivors. Gating on
    /// `own_log_recorded` instead would PERMANENTLY block such a node — no peer holds its
    /// never-broadcast log to re-fetch — which is the liveness slash this feature exists
    /// to prevent. A check-failed-own-seal log is invalid and is excluded from the
    /// selected set by `select`'s validity filter regardless of the gate, so the node
    /// finalizes as a player either way (the gate cannot make that "more correct").
    /// Idempotent: `finalize` consumes the ceremony, so a later trigger for the
    /// same epoch is a no-op. `< required_commitments` valid logs ever settling ⇒
    /// `ready` stays false ⇒ never finalized ⇒ the natural option-A stall (the residual
    /// LIVENESS-only failure, paired with `dkg_ceremony_fail_total` below; a
    /// forged/divergent `PK_E` is independently caught by the Stage-2 certify hook
    /// `beacon::certify`, which σ-verifies the recovered seed and Nullifies on
    /// mismatch). The Byzantine log-equivocation case (a dealer signing conflicting
    /// logs) is the still-deferred consensus-pinned-QUAL residual
    /// (`dpos_beacon_share_reshare`). The actor is single-threaded (`run`'s `select!`),
    /// so there is no concurrent mutation of `ceremonies`.
    fn drive_finalization(&mut self, height: u64, rng: &mut impl CryptoRngCore) {
        // `ready` probes non-destructively (Logs clone); `finalize` then consumes the
        // fulfilled ceremony. Both run STILL DURING the margin window — before the
        // epoch's boundary block is proposed/verified — so the verify-path C gate can
        // read the share.
        let ready: Vec<u64> = self
            .ceremonies
            .iter()
            .filter(|(e, c)| {
                if !c.dealing_closed() || !c.can_finalize() || !c.ready(rng) {
                    return false;
                }
                // Finalize only over a SETTLED set so every honest node selects the
                // IDENTICAL log set (→ identical canonical `select` → identical PK_E →
                // the C gate passes): either ALL-IN (every committee log recorded —
                // the common case, the instant the last Reveal lands) or the
                // height-deterministic SETTLE deadline past the seal (the shorthanded
                // fallback — an absent dealer never reveals, so the n−f survivors
                // finalize over the settled present set instead of waiting forever).
                let target = **e;
                let seal_deadline = self.epoch_start(target).saturating_sub(DKG_MARGIN_BLOCKS);
                // Cheap height check first: when the deterministic settle deadline has
                // passed we finalize regardless of all-in, so skip the `committee_for`
                // EVM read (the common settle-fallback path).
                if height >= seal_deadline + DKG_SETTLE_BLOCKS {
                    return true;
                }
                let n = (self.committee_for)(target).map_or(0, |s| s.len());
                n > 0 && c.recorded_log_count() == n
            })
            .map(|(e, _)| *e)
            .collect();
        for e in ready {
            // Capture-then-commit on the CEREMONY ITSELF: borrow it in place and remove
            // it ONLY after `finalize` returns `Ok`. A `finalize→Err` (a transient
            // `MissingPlayerDealing`: the resolver completed a log set whose private
            // dealings a freshly-resumed node's `view` lags) must NOT destroy the
            // ceremony — pre-resolver such a state simply stalled; consuming + removing
            // it first would forfeit the share AND drop its servable logs. On `Err` the
            // ceremony stays in `ceremonies` (its `can_finalize()` now false, so the gate
            // stops re-pulling it) and keeps serving its recorded logs until the boundary.
            let Some(c) = self.ceremonies.get_mut(&e) else {
                continue;
            };
            match c.finalize(rng) {
                Ok((out, share)) => {
                    // finalize succeeded — NOW commit: take the recorded logs to seed the
                    // serve_cache and drop the consumed ceremony from the map. Taking the
                    // logs only here means a finalize-Err never leaves a non-finalized
                    // epoch's partial logs orphaned (they ride the still-present ceremony).
                    let logs = self
                        .ceremonies
                        .remove(&e)
                        .expect("ceremony present (just borrowed)")
                        .take_signed_logs();
                    // Eager serve seed (the no-restart hot path never reads disk): the
                    // recovery `Producer` keeps serving this finalized epoch's logs
                    // O(1) until the boundary sweep. A strict subset-copy of the journal.
                    self.serve_cache.insert(e, Arc::new(logs));
                    // Item A: persist (PK_E, share) to disk BEFORE the in-memory
                    // insert (which moves the pair), so a mid-epoch restart reloads
                    // it instead of carry-forwarding the wrong key and stalling.
                    // Best-effort — the in-memory store is authoritative for the
                    // running process, so a write failure only warns.
                    if let Some(dir) = &self.share_dir {
                        if let Err(err) =
                            share_state::persist(dir, e, &out, &share, &self.share_state)
                        {
                            tracing::warn!(
                                epoch = e,
                                ?err,
                                "live DKG: failed to persist share to disk (in-memory store unaffected)"
                            );
                        }
                    }
                    if let Ok(mut store) = self.store.write() {
                        store.insert(e, (out, share));
                    }
                    // The journal is NOT evicted here. A node that finalized but has
                    // not yet crossed the boundary keeps its journal (and its
                    // `serve_cache` copy) so it can still serve a late-restarting peer —
                    // both reclaimed in the past-boundary sweep (`on_height` step 2b),
                    // bounded scratch.
                    // Edge-trigger the boundary-entry waiter: the share is now visible
                    // in the store, so a racing `enter(e)` wakes immediately rather than
                    // polling. `notify_one` stores a permit when no waiter is armed, so a
                    // share that lands between the consumer's reconcile and its re-arm is
                    // not lost (single consumer: `EpochManager::run`).
                    self.share_notify.notify_one();
                    self.metrics.dkg_ceremony_ok.inc();
                    tracing::info!(epoch = e, "live DKG: PK_epoch + share computed + stored");
                }
                Err(err) => {
                    self.metrics.dkg_ceremony_fail.inc();
                    tracing::warn!(
                        epoch = e,
                        ?err,
                        "live DKG: finalize failed after ready-probe — beacon stalls for this epoch"
                    );
                }
            }
        }
    }

    /// Start a ceremony for `target` (run during the just-entered epoch) when the
    /// committee actually changes; an unchanged committee carries the key forward
    /// (no ceremony — Phase 5 reuses the prior epoch's `BeaconKey`).
    fn maybe_start(&mut self, target: u64, rng: &mut impl CryptoRngCore, out: &mut Vec<Outgoing>) {
        // Skip if a ceremony for this epoch is in flight OR already computed (the
        // per-tick retry would otherwise re-deal an epoch whose ceremony finished
        // and was removed from `ceremonies`).
        if target == 0 || self.ceremonies.contains_key(&target) {
            return;
        }
        if self
            .store
            .read()
            .ok()
            .is_some_and(|s| s.contains_key(&target))
        {
            return;
        }
        // A Torn verdict is PERMANENT for this epoch (we sit out — re-dealing would
        // self-equivocate, §8.11.1). Short-circuit BEFORE the committee reads + the
        // `load_journal` disk read (full file read + per-record AEAD attempts) so we don't
        // re-derive the same sit-out every height tick for the rest of the epoch (review
        // [634]). `torn_warned` is exactly the set of epochs we have already sat out (it is
        // inserted only on the `Torn` arm below). `target` is our OWN epoch (`now+1`),
        // bounded — never wire-controlled.
        if self.torn_warned.contains(&target) {
            return;
        }
        let cur = (self.committee_for)(target - 1);
        let next = (self.committee_for)(target);
        let me = self.me_key.public_key();
        // One-shot diagnostic: log when committee[target] FIRST becomes readable,
        // with the deal decision inputs — pinpoints start vs carry-forward vs
        // not-member vs committee-never-readable without per-tick spam.
        if next.is_some() && self.eval_logged.insert(target) {
            tracing::info!(
                target,
                cur_n = cur.as_ref().map(|c| c.len()),
                next_n = next.as_ref().map(|c| c.len()),
                change = (cur.as_ref() != next.as_ref()),
                me_member = next.as_ref().is_some_and(|n| n.iter().any(|p| *p == me)),
                "live DKG: committee[target] readable — maybe_start eval"
            );
        }
        let (Some(cur), Some(next)) = (cur, next) else {
            return;
        };
        // Deterministic epoch-2 bootstrap: committee[2] always deals (during epoch
        // 1) even when unchanged, so a long-stable initial committee still seeds the
        // beacon. Every other epoch carries the key forward on an unchanged committee.
        if next == cur && target != DETERMINISTIC_BOOTSTRAP_EPOCH {
            return; // carry-forward
        }
        // Model B: only a MEMBER of committee[target] deals to itself. A node that
        // is in committee[target-1] but not committee[target] does not deal.
        if !next.iter().any(|p| *p == me) {
            return;
        }
        // Decide RESUME vs START vs SIT-OUT from the journal tri-state, loading it
        // ONCE. A genuine first run (`NoFile`) deals; a present journal RESUMES
        // player-only (no divergent re-deal, §8.11.1); a present-but-damaged journal
        // (`Torn`) means we already participated in this epoch's ceremony, so dealing
        // fresh would self-equivocate — we SIT OUT instead.
        let started = match self.load_journal(target) {
            JournalLoad::NoFile => self.start_fresh(target, next, rng, out),
            JournalLoad::Present(records) => self.resume_from_journal(target, next, records, out),
            JournalLoad::Torn => {
                // One-shot warn (this runs every tick — no ceremony is inserted to
                // short-circuit `maybe_start` — so an unconditional warn would flood
                // the log for the whole epoch).
                if self.torn_warned.insert(target) {
                    tracing::warn!(
                        epoch = target,
                        "live DKG: ceremony journal present but unreadable/torn — sitting out this \
                         epoch (we already participated; re-dealing would self-equivocate)"
                    );
                }
                false
            }
        };
        if !started {
            // Sit-out (Torn / failed resume): drop any dealings that raced ahead of a
            // start that will now never happen, so they don't linger un-acked until
            // the past-boundary sweep. We are sitting this epoch out either way.
            self.pending.remove(&target);
            return;
        }
        // Drain any dealings that raced ahead of our start (the start-race): replay
        // them through `handle` NOW, before any seal, so every dealer we heard from is
        // acked. Order-independent (`try_ack` fires only once both halves are
        // buffered). The acks the replay emits are collected into `out` and broadcast
        // by the caller, so a dealer that previously got ≤ quorum−1 acks now seals
        // `Ok`, not `TooManyReveals`. The newly-accepted dealings are journaled too.
        if let Some(buffered) = self.pending.remove(&target) {
            let mut drained = Vec::new();
            let c = self.ceremonies.get_mut(&target).expect("just started");
            for (from, body) in buffered {
                let step = c.handle(from, body);
                out.extend(step.outgoing);
                drained.extend(step.journal);
            }
            self.append_journal(target, drained);
        }
    }

    /// Load the per-epoch ceremony journal for `target` ([`JournalLoad::NoFile`]
    /// without a `share_dir`, the in-process/test default). The tri-state lets
    /// `maybe_start` tell a genuine first run (deal) from a present-but-damaged
    /// journal (sit out, never re-deal).
    fn load_journal(&self, target: u64) -> JournalLoad {
        let Some(dir) = &self.share_dir else {
            return JournalLoad::NoFile;
        };
        let max = NonZeroU32::new(fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as u32)
            .expect("MAX_COMMITTEE_SIZE > 0");
        share_state::load_journal(dir, target, &self.share_state, max)
    }

    /// Start a fresh ceremony for `target`, journaling its initial records. Returns
    /// whether a ceremony is now live.
    fn start_fresh(
        &mut self,
        target: u64,
        next: Set<PeerPubkey>,
        rng: &mut impl CryptoRngCore,
        out: &mut Vec<Outgoing>,
    ) -> bool {
        match DkgCeremony::start(rng, &self.namespace, target, next, self.me_key.clone()) {
            Ok((ceremony, step)) => {
                self.ceremonies.insert(target, ceremony);
                out.extend(step.outgoing);
                self.append_journal(target, step.journal);
                tracing::info!(epoch = target, "live DKG: ceremony started");
                true
            }
            Err(e) => {
                tracing::warn!(epoch = target, ?e, "live DKG: ceremony start failed");
                false
            }
        }
    }

    /// Resume `target`'s ceremony from its journaled `records` (mid-window restart),
    /// PLAYER-ONLY — never re-dealing (§8.11.1). A `MissingPlayerDealing` (truncated
    /// journal dropped a publicly-acked dealing) is a graceful sit-out for this epoch,
    /// never a crash. Returns whether a ceremony is now live.
    fn resume_from_journal(
        &mut self,
        target: u64,
        next: Set<PeerPubkey>,
        records: Vec<JournalRecord>,
        out: &mut Vec<Outgoing>,
    ) -> bool {
        match DkgCeremony::resume(&self.namespace, target, next, self.me_key.clone(), records) {
            Ok(resumed) => {
                // Seal-state is intrinsic to the resumed ceremony (dealer retired; our
                // own log in `recorded` iff we sealed) — nothing to track separately.
                let own_log_recorded = resumed.ceremony.own_log_recorded(&self.me_key.public_key());
                self.ceremonies.insert(target, resumed.ceremony);
                out.extend(resumed.outgoing);
                tracing::info!(
                    epoch = target,
                    own_log_recorded,
                    "live DKG: ceremony resumed from journal"
                );
                true
            }
            Err(e) => {
                tracing::warn!(
                    epoch = target,
                    ?e,
                    "live DKG: resume from journal failed — sitting out this epoch"
                );
                false
            }
        }
    }

    /// Whether an incoming DKG message should be BUFFERED when no ceremony for its
    /// epoch exists yet (the start-race), rather than dropped. Only DEALINGS
    /// (`Commitment`/`Share`) for a near-future, not-yet-sealed, not-yet-finalized
    /// epoch qualify — acks/reveals are meaningless without a live ceremony to feed,
    /// and `last_height` bounds the future window so far-future / garbage epochs
    /// cannot accumulate (a DoS guard); stale buffers are also evicted each tick.
    fn is_bufferable(&self, epoch: u64, body: &DkgBody) -> bool {
        if !matches!(body, DkgBody::Commitment(_) | DkgBody::Share(_)) {
            return false;
        }
        // Don't buffer a dealing for a ceremony whose dealing phase is already closed
        // (sealed or resumed player-only) — it can no longer be drained into a dealer.
        if self
            .ceremonies
            .get(&epoch)
            .is_some_and(|c| c.dealing_closed())
        {
            return false;
        }
        let now = self.epoch_of(self.last_height);
        if epoch <= now || epoch > now + 2 {
            return false; // already started / past, or too far in the future
        }
        self.store.read().map_or(true, |s| !s.contains_key(&epoch))
    }

    async fn on_message(&mut self, from: PeerPubkey, buf: &[u8], rng: &mut impl CryptoRngCore) {
        // Decode bounded by MAX_COMMITTEE_SIZE (upper bound; exact n not needed).
        let max = NonZeroU32::new(fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as u32)
            .expect("MAX_COMMITTEE_SIZE > 0");
        let mut wire = buf;
        let BeaconMessage::Dkg(payload) = match BeaconMessage::read(&mut wire) {
            Ok(m) => m,
            Err(_) => return,
        };
        let mut body = payload.as_ref();
        let msg = match DkgMsg::read_cfg(&mut body, &max) {
            Ok(m) => m,
            Err(_) => return,
        };
        let epoch = msg.ceremony_epoch;
        let body = msg.body;
        // Epoch-tag filter. An active ceremony processes the message directly. A
        // DEALING for an epoch we have not started yet is a start-race victim —
        // buffer it (drained by `maybe_start`) rather than DROP it (a dropped
        // dealing leaves that dealer un-acked ⇒ `TooManyReveals` ⇒ `DkgFailed`).
        // Acks/Reveals with no live ceremony are still dropped (nothing to feed;
        // a re-sealed Reveal re-arrives once we are live via the long window). DKG-log
        // RECOVERY is no longer a gossip body — it rides the `commonware_resolver::p2p`
        // engine (see `on_resolver_message` / `fetch_missing_logs`).
        if let Some(c) = self.ceremonies.get_mut(&epoch) {
            let step = c.handle(from, body);
            let recorded_log = step.recorded_a_log();
            self.append_journal(epoch, step.journal);
            self.broadcast_all(step.outgoing).await;
            // A newly-recorded Reveal may have just made a sealed ceremony all-in —
            // finalize NOW, event-driven, over the settled set. ONLY a recorded log can
            // change finalizability, so skip the `observe` batch-BLS for ack-only /
            // dealing-only steps (review [806]); the height-driven settle-deadline
            // finalize in `on_height` still covers the time-based path. See
            // [`Self::drive_finalization`].
            if recorded_log {
                self.drive_finalization(self.last_height, rng);
            }
        } else if self.is_bufferable(epoch, &body) {
            let buf = self.pending.entry(epoch).or_default();
            if buf.len() < fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as usize * 2 {
                buf.push((from, body));
            }
        }
    }

    /// `fetch_targeted` the missing dealer logs of every open, shorthanded ceremony
    /// via the DKG-log recovery resolver (§8.11.1). Gated on the OPEN window: stops
    /// at the settle deadline (after which finalize runs over the settled set or the
    /// ceremony stalls→evicts). The resolver dedupes in-flight keys, so re-issuing the
    /// missing set each tick is idempotent; targeting aims at the known committee
    /// roster (the log holders, in `latest.primary` via the registry-union tracker).
    /// No-op without a wired resolver (in-process/test default).
    ///
    /// Runs in `on_height` AFTER finalize + the past-boundary sweep, so `self.ceremonies`
    /// already reflects every drop; it then `retain`s the resolver's in-flight fetches to
    /// exactly the keys it (re)issues this tick — CANCELLING the fetches of any epoch that
    /// finalized or was swept (incl. the unsatisfiable `{e,me}` of a pre-seal node that has
    /// since finalized as a player), so the resolver stops re-issuing dead keys every
    /// `fetch_retry_timeout` for the life of the process (the slow request leak).
    async fn fetch_missing_logs(&mut self, height: u64) {
        if self.resolver.is_none() {
            return;
        }
        // Snapshot the (key, targets) requests first — borrowing `self.ceremonies` and
        // `self.committee_for` immutably — then borrow `self.resolver` (mutably) to
        // issue the fetches, so the two borrows never overlap.
        let mut requests: Vec<(DkgLogKey, NonEmptyVec<PeerPubkey>)> = Vec::new();
        // Live-ceremony epochs whose committee we could NOT read THIS tick (the transient
        // `committee_for→None` EVM race, same root as [965]). Their keys don't enter
        // `wanted` below, so WITHOUT preserving them the `retain` would CANCEL their
        // in-flight recovery fetches on a single bad read — resetting accumulated resolver
        // progress and risking a shareless stall if the read flaps (review [893]). A
        // genuinely dead epoch (finalized/swept) is absent from `self.ceremonies`, so it is
        // in NEITHER set → still cancelled, preserving the stale-tail prune.
        let mut unreadable: BTreeSet<u64> = BTreeSet::new();
        for (e, c) in &self.ceremonies {
            let Some(roster) = (self.committee_for)(*e) else {
                unreadable.insert(*e);
                continue;
            };
            let n = roster.len();
            // Once we hold every committee log there is nothing left to fetch.
            if n == 0 || c.recorded_log_count() >= n {
                continue;
            }
            // Keep fetching right up to the epoch boundary — `resume`/`finalize` stay
            // usable to the boundary, so a node that restarts in the last few blocks
            // (after the settle deadline, still pre-boundary) must still be able to
            // re-fetch its missing logs. The boundary sweep then drops the ceremony.
            if height >= self.epoch_start(*e) {
                continue; // past the boundary — the ceremony is about to be swept
            }
            // Target each fetch at the roster (the known holders). `fetch_targeted`
            // narrows within `latest.primary`; a committee member's logs are served
            // from any peer that holds them. The holders are in `latest.primary`
            // during E-1: committee[E] ⊆ the Active registry that the beacon plane's
            // `EpochTransition` tracks (registry ∪ committee[E]) on the SAME
            // `OracleHandle` the resolver's `Provider` reads, and the E-1→E boundary
            // `track(E)` re-includes committee[E] explicitly (STEP-0 reachability).
            let Some(targets) = NonEmptyVec::try_from(roster.iter().cloned().collect::<Vec<_>>()).ok()
            else {
                continue;
            };
            let recorded = c.recorded_dealers();
            for dealer in roster.iter() {
                // A dealer whose log we already hold is served, not fetched. No `me`
                // special-case: a torn-own-seal node re-fetches its OWN log like any
                // missing dealer (the peers that recorded its broadcast serve it),
                // re-passing the finalize gate once `me ∈ recorded`. A genuine pre-seal
                // node issues ONE unsatisfiable `{e, me}` fetch per tick — deduped
                // in-flight by the resolver, 16/s-capped, timing out as "no data", not
                // re-blocked. Harmless.
                if recorded.contains(dealer) {
                    continue;
                }
                requests.push((
                    DkgLogKey {
                        epoch: *e,
                        dealer: dealer.clone(),
                    },
                    targets.clone(),
                ));
            }
        }
        // The keys we still WANT in flight after this tick = exactly the ones just
        // (re)issued. Drop everything else from the resolver so a finalized/swept epoch's
        // fetches (and the unsatisfiable `{e,me}` of a node that has since finalized) stop
        // retrying forever. `retain` needs an owned `'static` predicate, so move a snapshot
        // set in. Re-issued keys are deduped by the resolver (in-flight), so this is purely
        // a prune of the stale tail.
        let wanted: BTreeSet<DkgLogKey> = requests.iter().map(|(k, _)| k.clone()).collect();
        let resolver = self.resolver.as_mut().expect("checked Some above");
        resolver
            .retain(move |key| wanted.contains(key) || unreadable.contains(&key.epoch))
            .await;
        for (key, targets) in requests {
            resolver.fetch_targeted(key, targets).await;
        }
    }

    /// Handle one inbound resolver message — serve a `Produce` (a peer is fetching a
    /// dealer log from us) or ingest a `Deliver` (we fetched a dealer log). Runs in
    /// the single-threaded loop so it can touch ceremony state directly.
    async fn on_resolver_message(&mut self, msg: LogMessage, rng: &mut impl CryptoRngCore) {
        match msg {
            LogMessage::Produce { key, response } => {
                // DROP the responder (no send) when we don't hold the log → the
                // resolver sends an empty "no data" response → the requester retries
                // another peer. `serve_log` serves from the live ceremony / serve_cache
                // / a cold journal parse, so a finalized-but-pre-boundary node (incl.
                // one that just restarted) can still serve.
                if let Some(bytes) = self.serve_log(&key) {
                    let _ = response.send(bytes);
                }
            }
            LogMessage::Deliver {
                key,
                value,
                response,
            } => {
                let valid = self.ingest_log(&key, value, rng).await;
                let _ = response.send(valid);
            }
        }
    }

    /// Serve the encoded `SignedDealerLog` for `{epoch, dealer}`. Sources, in order:
    /// the live ceremony's recorded `signed_logs`; the bounded `serve_cache` (a
    /// finalized-but-pre-boundary epoch, O(1), no disk); and — on a cold-cache miss
    /// after a restart — a ONE-TIME parse of the durable journal (the source of truth),
    /// re-`check`ed and cached so subsequent serves are O(1). A cache HIT does no
    /// per-request BLS `check`; only the cold miss parses + re-`check`s.
    ///
    /// `serve_cache` holds ONLY non-empty POSITIVE maps (an epoch this node finalized, or
    /// re-parsed from a present journal to ≥1 valid log). An unservable cold miss
    /// (absent/torn journal, transiently-unreadable committee) caches NOTHING and returns
    /// `None`: a transient `committee_for→None` can never poison a finalized epoch's serve
    /// (it re-parses correctly once the committee is readable, review [965]), and an
    /// attacker-controlled `key.epoch > now` (no journal file → empty) can never accumulate
    /// an entry (review [954]). Returns `None` when no source holds the log → the resolver
    /// sends an empty "no data" response → the requester retries another peer.
    fn serve_log(&mut self, key: &DkgLogKey) -> Option<Bytes> {
        if let Some(signed) = self
            .ceremonies
            .get(&key.epoch)
            .and_then(|c| c.signed_log(&key.dealer))
        {
            return Some(signed.encode());
        }
        if let Some(logs) = self.serve_cache.get(&key.epoch) {
            return logs.get(&key.dealer).map(|s| s.encode());
        }
        // Cold miss: parse the epoch's durable journal ONCE, re-`check`. Cache ONLY a
        // non-empty (positive) result; an empty/unservable result is NOT cached — no
        // negative entries (that was the [965] poison / [954] unbounded-growth root). The
        // rare residual (a genuinely present-but-Torn journal for one of our OWN served
        // epochs) re-parses per request, bounded by the resolver quota + the
        // finalize→boundary window (reconcile then deletes the file → cheap `NoFile`), and
        // is never attacker-inducible (an attacker cannot create a Torn file on our disk).
        let logs = self.cold_load_serve_cache(key.epoch);
        let bytes = logs.get(&key.dealer).map(|s| s.encode());
        if !logs.is_empty() {
            self.serve_cache.insert(key.epoch, logs);
        }
        bytes
    }

    /// Parse + re-`check` `epoch`'s journal into a serve map on a `serve_cache` cold
    /// miss (the post-restart path). An absent/torn journal or transiently-unreadable
    /// committee yields an EMPTY map (a serve declines un-verifiable logs, the resolver
    /// retries elsewhere); the caller caches the result ONLY if non-empty, so an empty map
    /// is never memoized — no negative entries (review [965]/[954]). A boundary-passed
    /// epoch already had its journal reconciled/evicted, so `load_journal` returns
    /// `NoFile` → empty.
    fn cold_load_serve_cache(&self, epoch: u64) -> Arc<BTreeMap<PeerPubkey, DealerReveal>> {
        let JournalLoad::Present(records) = self.load_journal(epoch) else {
            return Arc::new(BTreeMap::new());
        };
        #[cfg(test)]
        COLD_PARSE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let map = (self.committee_for)(epoch)
            .and_then(|committee| {
                checked_serve_map(&self.namespace, epoch, committee, records).ok()
            })
            .unwrap_or_default();
        Arc::new(map)
    }

    /// Ingest a `SignedDealerLog` delivered by the resolver for `{epoch, dealer}`:
    /// decode + re-`check` + record via the ceremony's peer-Reveal path, journal it,
    /// then drive finalize (a recovered log may complete the set). Returns the
    /// resolver `deliver` verdict — a TWO-VALUED API (`true` = clear the fetch + stop;
    /// `false` = block this peer + `add_retry` the key elsewhere; `resolver engine.rs`):
    /// - `true` — the log `check`-verified AND was signed by the REQUESTED `key.dealer`
    ///   (the fetch for `{epoch, dealer}` is now genuinely satisfied) or is an honest
    ///   duplicate; OR there is no live ceremony for this epoch (already finalized/swept
    ///   — the fetch is genuinely moot, so let it clear rather than block an honest peer).
    /// - `false` — a genuine forgery (`check` fails), a valid log for the WRONG dealer
    ///   (a peer answering a targeted fetch for D with D'), OR an UNDECODABLE delivery.
    ///   An undecode must NOT return `true`: `true` marks the fetch SATISFIED (clears it),
    ///   so one peer serving garbage for `key` would permanently kill `key`'s recovery
    ///   with no log recorded. `false` keeps the fetch alive (`add_retry` → another peer).
    ///   The per-peer block `false` also incurs is an unavoidable side-effect of the
    ///   resolver's two-valued deliver API (there is no "no-data, retry, don't block"
    ///   verdict on the deliver path — that only exists when the SERVER returns no data);
    ///   it is bounded + acceptable because a committee peer serving undecodable bytes for
    ///   an EXPLICIT `{epoch,dealer}` fetch is anomalous, and keeping `key` recoverable
    ///   outweighs not-blocking one such peer.
    async fn ingest_log(
        &mut self,
        key: &DkgLogKey,
        value: Bytes,
        rng: &mut impl CryptoRngCore,
    ) -> bool {
        let max = NonZeroU32::new(fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as u32)
            .expect("MAX_COMMITTEE_SIZE > 0");
        let signed = match DealerReveal::read_cfg(&mut value.as_ref(), &max) {
            Ok(s) => s,
            // Undecodable bytes → `false`: do NOT clear the fetch (which `true` would do,
            // letting a garbage-serving peer kill `key`'s recovery). `false` retries `key`
            // at another peer; the per-peer block is the unavoidable cost of the resolver's
            // two-valued deliver API. NOT recorded.
            Err(_) => return false,
        };
        let Some(c) = self.ceremonies.get_mut(&key.epoch) else {
            // No live ceremony for this epoch (already finalized/swept, or never
            // started). The fetch is genuinely moot → `true` clears it (don't block an
            // honest peer for a key WE no longer need).
            return true;
        };
        // Bind the delivered log to the REQUESTED `key.dealer`: a forgery or a valid
        // log for a different dealer both return `false` (block + re-fetch `key`).
        let (accepted, journal) = c.ingest_signed_log(&key.dealer, signed);
        if accepted {
            self.append_journal(key.epoch, journal);
            self.drive_finalization(self.last_height, rng);
        }
        accepted
    }

    /// Send each outgoing ceremony message over BEACON_CHANNEL (broadcast or direct).
    async fn broadcast_all(&mut self, msgs: Vec<Outgoing>) {
        for o in msgs {
            let wire = BeaconMessage::Dkg(o.msg.encode()).encode();
            let recipients = match o.target {
                Target::Broadcast => Recipients::All,
                Target::Direct(pk) => Recipients::One(pk),
            };
            // Best-effort: a dropped DKG message is recovered by the long window /
            // reveal mechanism; never block consensus on a send failure.
            let _ = self.sender.send(recipients, wire, false).await;
        }
    }
}

#[cfg(test)]
mod clock_tests {
    use super::*;
    use commonware_p2p::{
        simulated::{Config as SimConfig, Link, Network, Oracle},
        Manager as _,
    };
    use commonware_math::algebra::Random as _;
    use commonware_runtime::{deterministic, Clock as _, Metrics as _, Runner as _, Spawner as _};
    use commonware_utils::NZUsize;
    use rand_08::{rngs::StdRng, SeedableRng as _};
    use std::time::Duration;

    type SimContext = deterministic::Context;

    /// A no-op DKG-log resolver for the clock tests: they exercise the gossip /
    /// finalize machinery, not the recovery-fetch path (that is covered by the
    /// resolver/ingest unit tests). Picks the concrete `R` type the actor's third
    /// generic needs; every method is inert.
    #[derive(Clone)]
    struct NoopResolver;
    impl commonware_resolver::Resolver for NoopResolver {
        type Key = DkgLogKey;
        type PublicKey = PeerPubkey;
        async fn fetch(&mut self, _: Self::Key) {}
        async fn fetch_all(&mut self, _: Vec<Self::Key>) {}
        async fn fetch_targeted(
            &mut self,
            _: Self::Key,
            _: commonware_utils::vec::NonEmptyVec<Self::PublicKey>,
        ) {
        }
        async fn fetch_all_targeted(
            &mut self,
            _: Vec<(Self::Key, commonware_utils::vec::NonEmptyVec<Self::PublicKey>)>,
        ) {
        }
        async fn cancel(&mut self, _: Self::Key) {}
        async fn clear(&mut self) {}
        async fn retain(&mut self, _: impl Fn(&Self::Key) -> bool + Send + 'static) {}
    }

    /// A resolver mock that records its in-flight fetch set so a test can assert
    /// `fetch_missing_logs` CANCELS dead fetches (`retain`) — exercising the [804]
    /// uncancelled-fetch-leak fix. `fetch_targeted` inserts the key; `retain` prunes the
    /// set by the predicate; `cancel`/`clear` mirror the trait.
    #[derive(Clone, Default)]
    struct RecordingResolver {
        in_flight: Arc<std::sync::Mutex<BTreeSet<DkgLogKey>>>,
    }
    impl commonware_resolver::Resolver for RecordingResolver {
        type Key = DkgLogKey;
        type PublicKey = PeerPubkey;
        async fn fetch(&mut self, key: Self::Key) {
            self.in_flight.lock().unwrap().insert(key);
        }
        async fn fetch_all(&mut self, keys: Vec<Self::Key>) {
            self.in_flight.lock().unwrap().extend(keys);
        }
        async fn fetch_targeted(
            &mut self,
            key: Self::Key,
            _: commonware_utils::vec::NonEmptyVec<Self::PublicKey>,
        ) {
            self.in_flight.lock().unwrap().insert(key);
        }
        async fn fetch_all_targeted(
            &mut self,
            requests: Vec<(Self::Key, commonware_utils::vec::NonEmptyVec<Self::PublicKey>)>,
        ) {
            self.in_flight
                .lock()
                .unwrap()
                .extend(requests.into_iter().map(|(k, _)| k));
        }
        async fn cancel(&mut self, key: Self::Key) {
            self.in_flight.lock().unwrap().remove(&key);
        }
        async fn clear(&mut self) {
            self.in_flight.lock().unwrap().clear();
        }
        async fn retain(&mut self, predicate: impl Fn(&Self::Key) -> bool + Send + 'static) {
            self.in_flight.lock().unwrap().retain(|k| predicate(k));
        }
    }

    const SEAL_DEADLINE: u64 = INTERVAL * DETERMINISTIC_BOOTSTRAP_EPOCH - DKG_MARGIN_BLOCKS; // 10
    const BOUNDARY: u64 = INTERVAL * DETERMINISTIC_BOOTSTRAP_EPOCH; // 20 = epoch_start(2)
    const INTERVAL: u64 = 10;
    const ACTIVATION: u64 = 0;
    /// Result-final lag (the EL-finalized clock trails the ordering clock by this).
    const K: u64 = crate::K;

    /// Spawn one [`DkgActor`] over the simulated network and return its height sink.
    /// The actor deals `committee[2]` (the deterministic bootstrap epoch) against the
    /// other dealers; the stable committee makes every epoch carry-forward EXCEPT 2.
    async fn spawn_dealer(
        ctx: &SimContext,
        oracle: &Oracle<PeerPubkey, SimContext>,
        me: Ed25519PrivateKey,
        committee: Set<PeerPubkey>,
        store: CeremonyStore,
        share_notify: Arc<tokio::sync::Notify>,
        interval: u64,
    ) -> tokio::sync::mpsc::Sender<u64> {
        spawn_dealer_at(ctx, oracle, me, committee, store, share_notify, interval, None, 7).await
    }

    /// `spawn_dealer` with an explicit on-disk `share_dir` (so the journal/share
    /// persist) + an rng seed (so a re-spawn after a restart uses fresh randomness,
    /// proving the resume does NOT depend on a deterministic re-deal).
    #[allow(clippy::too_many_arguments)]
    async fn spawn_dealer_at(
        ctx: &SimContext,
        oracle: &Oracle<PeerPubkey, SimContext>,
        me: Ed25519PrivateKey,
        committee: Set<PeerPubkey>,
        store: CeremonyStore,
        share_notify: Arc<tokio::sync::Notify>,
        interval: u64,
        share_dir: Option<PathBuf>,
        rng_seed: u64,
    ) -> tokio::sync::mpsc::Sender<u64> {
        let pk = me.public_key();
        let (sender, receiver) = oracle
            .control(pk.clone())
            .register(
                fluentbase_p2p::constants::BEACON_CHANNEL,
                fluentbase_p2p::constants::BEACON_QUOTA,
            )
            .await
            .expect("register BEACON_CHANNEL");
        let committee_for: CommitteeFor = {
            let set = committee.clone();
            Arc::new(move |_epoch: u64| Some(set.clone()))
        };
        let actor = DkgActor::new(
            b"FLUENT_DPOS_V1_clocktest".to_vec(),
            me,
            sender,
            receiver,
            None::<NoopResolver>,
            None,
            committee_for,
            store,
            share_notify,
            ACTIVATION,
            interval,
            crate::beacon::metrics::BeaconMetrics::default(),
            share_dir,
            ShareState::Plaintext,
        );
        let (height_tx, height_rx) = tokio::sync::mpsc::channel::<u64>(256);
        let rng = StdRng::seed_from_u64(rng_seed);
        drop(
            ctx.with_label("dealer")
                .spawn(move |_c| async move { actor.run(height_rx, rng).await }),
        );
        height_tx
    }

    /// `spawn_dealer_at`, but wires a REAL `commonware_resolver::p2p::Engine` (not the
    /// `NoopResolver`): registers a SECOND channel `BEACON_RESOLVER_CHANNEL` for the
    /// engine, bridges it to the actor via the `LogHandler` + an mpsc of `LogMessage`,
    /// and passes the engine `Mailbox` as the actor's `R`. This exercises the actual
    /// `fetch_missing_logs`/`serve_log`/`ingest_log` round-trip over the sim network,
    /// which `NoopResolver` cannot. Returns the height sink.
    #[allow(clippy::too_many_arguments)]
    async fn spawn_dealer_resolved(
        ctx: &SimContext,
        oracle: &Oracle<PeerPubkey, SimContext>,
        me: Ed25519PrivateKey,
        committee: Set<PeerPubkey>,
        store: CeremonyStore,
        share_notify: Arc<tokio::sync::Notify>,
        interval: u64,
        share_dir: Option<PathBuf>,
        rng_seed: u64,
    ) -> tokio::sync::mpsc::Sender<u64> {
        let pk = me.public_key();
        let (sender, receiver) = oracle
            .control(pk.clone())
            .register(
                fluentbase_p2p::constants::BEACON_CHANNEL,
                fluentbase_p2p::constants::BEACON_QUOTA,
            )
            .await
            .expect("register BEACON_CHANNEL");
        let (res_sender, res_receiver) = oracle
            .control(pk.clone())
            .register(
                fluentbase_p2p::constants::BEACON_RESOLVER_CHANNEL,
                fluentbase_p2p::constants::BEACON_RESOLVER_QUOTA,
            )
            .await
            .expect("register BEACON_RESOLVER_CHANNEL");

        let (log_tx, log_rx) =
            tokio::sync::mpsc::channel::<crate::beacon::log_resolver::LogMessage>(256);
        let handler = crate::beacon::log_resolver::LogHandler::new(log_tx);
        // Each engine registers commonware metrics under its context label; all nodes
        // share one `ctx`, so the label MUST be unique per spawn (else a duplicate-
        // metric panic). One process-global counter suffices for the tests.
        let engine_idx = {
            use std::sync::atomic::{AtomicU64, Ordering};
            static ENGINE_SEQ: AtomicU64 = AtomicU64::new(0);
            ENGINE_SEQ.fetch_add(1, Ordering::Relaxed)
        };
        let (engine, mailbox) = commonware_resolver::p2p::Engine::new(
            ctx.with_label(&format!("beacon_log_resolver_{engine_idx}")),
            commonware_resolver::p2p::Config {
                peer_provider: oracle.manager(),
                blocker: oracle.control(pk.clone()),
                consumer: handler.clone(),
                producer: handler,
                mailbox_size: 256,
                me: Some(pk.clone()),
                initial: Duration::from_millis(100),
                timeout: Duration::from_secs(5),
                fetch_retry_timeout: Duration::from_millis(500),
                priority_requests: false,
                priority_responses: false,
            },
        );
        drop(engine.start((res_sender, res_receiver)));

        let committee_for: CommitteeFor = {
            let set = committee.clone();
            Arc::new(move |_epoch: u64| Some(set.clone()))
        };
        let actor = DkgActor::new(
            b"FLUENT_DPOS_V1_clocktest".to_vec(),
            me,
            sender,
            receiver,
            Some(mailbox),
            Some(log_rx),
            committee_for,
            store,
            share_notify,
            ACTIVATION,
            interval,
            crate::beacon::metrics::BeaconMetrics::default(),
            share_dir,
            ShareState::Plaintext,
        );
        let (height_tx, height_rx) = tokio::sync::mpsc::channel::<u64>(256);
        let rng = StdRng::seed_from_u64(rng_seed);
        drop(
            ctx.with_label("dealer_resolved")
                .spawn(move |_c| async move { actor.run(height_rx, rng).await }),
        );
        height_tx
    }

    /// Run the 4-dealer committee[2] DKG over the simulated network, driving the
    /// height clock with `lag` (0 = ordering clock; K = EL-finalized clock). Feeds
    /// one height per virtual tick up to and including `feed_to`; returns whether the
    /// victim memoized `(PK_2, share)` by then.
    async fn dkg_seeded_by(ctx: SimContext, lag: u64, feed_to: u64) -> bool {
        // freeze_at == feed_to ⇒ feed the whole range; no late starters.
        dkg_seeded_with_freeze(ctx, lag, feed_to, feed_to, INTERVAL, 0, 0).await
    }

    /// The 4-dealer committee[2] DKG over the sim network, returning whether the victim
    /// (node 0) memoized `(PK_2, share)`. Knobs:
    /// - `lag`: subtract from every fed height (0 = ordering clock, K = EL-finalized).
    /// - `freeze_at`: STOP feeding heights past this (still ticking) — models the
    ///   boundary stall; a victim seeding past it proves event-driven finalize.
    /// - `interval`: epoch length (a larger value gives a real dealing window).
    /// - `late_count`/`late_lag`: the first `late_count` nodes start `late_lag` ticks
    ///   LATE, so peers' dealings reach them BEFORE their own `maybe_start` — the
    ///   start-race. Those dealings must be BUFFERED (drained on start) and acked, not
    ///   dropped (a drop leaves the dealer un-acked ⇒ `TooManyReveals` ⇒ `DkgFailed`).
    async fn dkg_seeded_with_freeze(
        ctx: SimContext,
        lag: u64,
        feed_to: u64,
        freeze_at: u64,
        interval: u64,
        late_count: usize,
        late_lag: u64,
    ) -> bool {
        let oracle: Oracle<PeerPubkey, SimContext> = {
            let (network, oracle) = Network::new(
                ctx.with_label("sim_net"),
                SimConfig {
                    max_size: 1024 * 1024,
                    disconnect_on_block: false,
                    tracked_peer_sets: NZUsize!(4),
                },
            );
            network.start();
            oracle
        };

        let mut rng = StdRng::seed_from_u64(1);
        let keys: Vec<Ed25519PrivateKey> =
            (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        oracle.manager().track(0, committee.clone()).await;
        for a in &keys {
            for b in &keys {
                if a.public_key() != b.public_key() {
                    oracle
                        .add_link(
                            a.public_key(),
                            b.public_key(),
                            Link {
                                latency: Duration::from_millis(0),
                                jitter: Duration::from_millis(0),
                                success_rate: 1.0,
                            },
                        )
                        .await
                        .expect("link");
                }
            }
        }

        let victim_store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
        let mut sinks = Vec::new();
        for (i, k) in keys.iter().enumerate() {
            let store = if i == 0 {
                victim_store.clone()
            } else {
                Arc::new(RwLock::new(BTreeMap::new()))
            };
            let notify = Arc::new(tokio::sync::Notify::new());
            sinks.push(
                spawn_dealer(&ctx, &oracle, k.clone(), committee.clone(), store, notify, interval)
                    .await,
            );
        }

        for h in 0..=feed_to {
            for (i, s) in sinks.iter().enumerate() {
                // The first `late_count` nodes start LATE (their height feed lags by
                // `late_lag`), so peers' dealings arrive before their `maybe_start`.
                let node_h = if i < late_count {
                    h.saturating_sub(late_lag)
                } else {
                    h
                };
                // Past `freeze_at` we stop feeding (the boundary-stall freeze) but keep
                // ticking, so the sim delivers in-flight Reveals.
                if node_h <= freeze_at {
                    let _ = s.send(node_h.saturating_sub(lag)).await;
                }
            }
            ctx.sleep(Duration::from_millis(50)).await;
        }

        let seeded = victim_store
            .read()
            .map(|s| s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
            .unwrap_or(false);
        seeded
    }

    /// The ordering clock seals `committee[2]`'s ceremony at `SEAL_DEADLINE` and
    /// finalizes within a couple of post-seal ticks — well before the epoch-2 boundary.
    /// The EL-finalized clock (lagged by `K`) reaches the seal deadline `K` ticks later,
    /// so by `SEAL_DEADLINE + 2` it has NOT yet memoized the share: the lag silently
    /// eats `K` blocks of the `DKG_MARGIN_BLOCKS` window. This is the wedge (Problem A):
    /// feeding the actor an `ordering−K` clock against an ordering-unit seal deadline.
    #[test]
    fn ordering_clock_seeds_within_margin_lagged_clock_slips_k() {
        const { assert!(DKG_MARGIN_BLOCKS >= K, "test geometry assumes MARGIN >= K") };
        let assert_by = SEAL_DEADLINE + 2;
        const { assert!(SEAL_DEADLINE + 2 < BOUNDARY, "assert point must precede the boundary") };

        let runtime = deterministic::Runner::default();
        let ordering_seeded =
            runtime.start(|ctx| async move { dkg_seeded_by(ctx, 0, assert_by).await });
        assert!(
            ordering_seeded,
            "ordering clock must memoize (PK_2, share) by SEAL_DEADLINE+2 (within margin)"
        );

        let runtime = deterministic::Runner::default();
        let lagged_seeded =
            runtime.start(|ctx| async move { dkg_seeded_by(ctx, K, assert_by).await });
        assert!(
            !lagged_seeded,
            "EL-finalized (ordering−K) clock must NOT have memoized the share yet — \
             the K-lag eats the margin (reproduces the wedge)"
        );
    }

    /// Regression for the boundary-stall deadlock. Feed heights only up to the seal
    /// deadline — every dealer seals and broadcasts its `Reveal` — then FREEZE the
    /// feed (no further `on_height` ticks, modelling reth-finalized frozen at the
    /// unfinalizable boundary block) while the sim keeps delivering those Reveals.
    /// The victim must STILL memoize `(PK_2, share)`, driven by `on_message` →
    /// `drive_finalization`. PRE-fix (finalize only inside `on_height`) the frozen
    /// feed starves it and it never seeds; POST-fix the Reveal event finalizes it.
    #[test]
    fn frozen_feed_seeds_via_reveal_event() {
        // The ceremony ENTERS at epoch_start(1) and SEALS on the next tick (the seal
        // step runs before `maybe_start` inserts it, so the seal lands one tick after
        // entry). Freeze right after that seal tick — every dealer has sealed and
        // broadcast its Reveal — then tick on with the feed frozen so the Reveals are
        // delivered purely over the network: finalize must then come from `on_message`,
        // not `on_height` (which is frozen). This is the boundary-stall the fix targets.
        const SEAL_TICK: u64 = SEAL_DEADLINE + 1;
        const TAIL: u64 = 6;
        const { assert!(SEAL_TICK + TAIL < BOUNDARY, "stay within the margin window") };
        let runtime = deterministic::Runner::default();
        let seeded = runtime.start(|ctx| async move {
            dkg_seeded_with_freeze(ctx, 0, SEAL_TICK + TAIL, SEAL_TICK, INTERVAL, 0, 0).await
        });
        assert!(
            seeded,
            "share must finalize via the Reveal event with the height feed frozen at \
             the seal deadline — the boundary-stall deadlock fix"
        );
    }

    /// Start-race regression (Fix 1). With a real dealing window (interval 30, margin
    /// 10 ⇒ the ceremony enters at epoch_start(1)=30 and seals at the deadline 50, a
    /// 20-tick window), the first TWO nodes start 2 ticks LATE, so each receives the
    /// two EARLY dealers' `Commitment`+`Share` BEFORE its own `maybe_start`. PRE-fix
    /// those are dropped ⇒ the early dealers collect only 2 acks ⇒ seal
    /// `TooManyReveals` ⇒ `select` rejects 2 of 4 ⇒ `< quorum(3)` ⇒ `DkgFailed` forever
    /// (the docker wedge). POST-fix they are BUFFERED and drained on start, every dealer
    /// is acked, and the victim seeds.
    #[test]
    fn start_race_buffers_early_dealings() {
        const INTERVAL_LONG: u64 = 30;
        let runtime = deterministic::Runner::default();
        let seeded = runtime.start(|ctx| async move {
            dkg_seeded_with_freeze(ctx, 0, 56, 56, INTERVAL_LONG, 2, 2).await
        });
        assert!(
            seeded,
            "early peer dealings that race ahead of `maybe_start` must be buffered + \
             drained (not dropped) so every dealer is acked and the victim seeds"
        );
    }

    /// Leak regression (H2). A ceremony whose committee has a quorum it never reaches
    /// (here only the victim deals; its three peers never reveal) is SEALED at the
    /// deadline but `ready()` stays false, so `drive_finalization` never removes it.
    /// Once the clock crosses the epoch boundary the per-tick sweep must evict it from
    /// `ceremonies`/`sealed` — otherwise it lingers and re-probes `ready()` forever.
    /// Drives `on_height` directly (not `run`) to inspect the actor's internal state.
    #[test]
    fn stalled_ceremony_is_evicted_past_boundary() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle: Oracle<PeerPubkey, SimContext> = {
                let (network, oracle) = Network::new(
                    ctx.with_label("sim_net"),
                    SimConfig {
                        max_size: 1024 * 1024,
                        disconnect_on_block: false,
                        tracked_peer_sets: NZUsize!(4),
                    },
                );
                network.start();
                oracle
            };
            let mut rng = StdRng::seed_from_u64(3);
            let keys: Vec<Ed25519PrivateKey> =
                (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
            // committee n=4 ⇒ quorum 3; only the victim runs ⇒ 1 valid log < quorum ⇒ stall.
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;
            let me = keys[0].clone();
            let pk = me.public_key();
            let (sender, receiver) = oracle
                .control(pk.clone())
                .register(
                    fluentbase_p2p::constants::BEACON_CHANNEL,
                    fluentbase_p2p::constants::BEACON_QUOTA,
                )
                .await
                .expect("register BEACON_CHANNEL");
            let committee_for: CommitteeFor = {
                let set = committee.clone();
                Arc::new(move |_epoch: u64| Some(set.clone()))
            };
            let store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut actor = DkgActor::new(
                b"FLUENT_DPOS_V1_leaktest".to_vec(),
                me,
                sender,
                receiver,
                None::<NoopResolver>,
                None,
                committee_for,
                store,
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                None,
                ShareState::Plaintext,
            );
            let mut arng = StdRng::seed_from_u64(9);

            // Through the seal deadline: committee[2] enters (height 10), seals (11),
            // then stalls (1 valid log < quorum 3, so `ready()` never holds).
            for h in 0..=(SEAL_DEADLINE + 2) {
                actor.on_height(h, &mut arng).await;
            }
            assert!(
                actor
                    .ceremonies
                    .get(&DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .is_some_and(|c| c.dealing_closed()),
                "precondition: committee[2] must be sealed-but-stalled before the boundary"
            );

            // Cross the epoch-2 boundary: the sweep must evict the stalled entry.
            for h in (SEAL_DEADLINE + 3)..=(BOUNDARY + 1) {
                actor.on_height(h, &mut arng).await;
            }
            assert!(
                actor.ceremonies.is_empty(),
                "stalled ceremony must be evicted once its boundary passes"
            );
        });
    }

    fn fresh_share_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "beacon-dkg-restart-{tag}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Mid-window restart recovery. The victim (node-0) runs committee[2]'s DKG with a
    /// PERSISTENT `share_dir`, seals + journals, then is DROPPED (its task aborted via
    /// re-register) and REBUILT with FRESH in-memory maps + a FRESH store but the SAME
    /// `share_dir`. The rebuilt actor must `resume` from the journal and memoize
    /// `(PK_2, share)` before the boundary — proving the journal+resume path recovers a
    /// mid-window restart (the §8.11.1 durability gap fix). PRE-fix the rebuilt actor
    /// loses its partial progress, never re-reaches quorum, and stays shareless.
    #[test]
    fn restart_midwindow_recovers_via_journal() {
        const RESTART_AT: u64 = SEAL_DEADLINE + 2; // after node-0's seal (deadline+1)
        const FEED_TO: u64 = BOUNDARY - 1; // stay within the margin window
        const { assert!(RESTART_AT > SEAL_DEADLINE && FEED_TO < BOUNDARY) };

        let share_dir = fresh_share_dir("recover");
        let runtime = deterministic::Runner::default();
        let dir = share_dir.clone();
        let seeded = runtime.start(|ctx| async move {
            let oracle: Oracle<PeerPubkey, SimContext> = {
                let (network, oracle) = Network::new(
                    ctx.with_label("sim_net"),
                    SimConfig {
                        max_size: 1024 * 1024,
                        disconnect_on_block: false,
                        tracked_peer_sets: NZUsize!(4),
                    },
                );
                network.start();
                oracle
            };
            let mut rng = StdRng::seed_from_u64(1);
            let keys: Vec<Ed25519PrivateKey> =
                (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;
            for a in &keys {
                for b in &keys {
                    if a.public_key() != b.public_key() {
                        oracle
                            .add_link(
                                a.public_key(),
                                b.public_key(),
                                Link {
                                    latency: Duration::from_millis(0),
                                    jitter: Duration::from_millis(0),
                                    success_rate: 1.0,
                                },
                            )
                            .await
                            .expect("link");
                    }
                }
            }

            // Node-0 (victim): persistent share_dir. Peers: in-memory.
            let victim_store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut sinks = Vec::new();
            for (i, k) in keys.iter().enumerate() {
                let store = if i == 0 {
                    victim_store.clone()
                } else {
                    Arc::new(RwLock::new(BTreeMap::new()))
                };
                let dir_i = if i == 0 { Some(dir.clone()) } else { None };
                sinks.push(
                    spawn_dealer_at(
                        &ctx,
                        &oracle,
                        k.clone(),
                        committee.clone(),
                        store,
                        Arc::new(tokio::sync::Notify::new()),
                        INTERVAL,
                        dir_i,
                        7,
                    )
                    .await,
                );
            }

            // One peer (node-3) runs LATE so node-0 is SHORTHANDED when it seals (it
            // holds only the 3-log dealer-quorum, NOT all-in) — so it does NOT finalize
            // before the restart. That is the genuine mid-window state the fix targets:
            // a sealed-but-not-finalized ceremony whose partial progress must survive
            // the restart (via the journal) and complete (via the journal + pull).
            const LATE: u64 = 4;
            let feed_round = |h: u64| {
                let node3_h = h.saturating_sub(LATE);
                (h, node3_h)
            };

            // Feed up to node-0's seal so it journals its ceremony progress.
            for h in 0..=RESTART_AT {
                let (h0, h3) = feed_round(h);
                for (i, s) in sinks.iter().enumerate() {
                    let _ = s.send(if i == 3 { h3 } else { h0 }).await;
                }
                ctx.sleep(Duration::from_millis(50)).await;
            }

            // RESTART node-0: drop its height sink (aborting the old task on next
            // recv-close) and re-spawn a FRESH actor over the SAME share_dir with a
            // FRESH store (proving resume re-populates it) + a DIFFERENT rng seed
            // (proving recovery does not rely on a deterministic re-deal). The
            // re-register OVERWRITES node-0's channel, aborting the old receiver.
            drop(sinks.remove(0));
            let restarted_store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let new_sink = spawn_dealer_at(
                &ctx,
                &oracle,
                keys[0].clone(),
                committee.clone(),
                restarted_store.clone(),
                Arc::new(tokio::sync::Notify::new()),
                INTERVAL,
                Some(dir.clone()),
                99,
            )
            .await;
            sinks.insert(0, new_sink);

            // Tick on (within the window) so the rebuilt actor resumes from the journal,
            // pulls any missing logs, and finalizes over the settled set.
            for h in (RESTART_AT + 1)..=FEED_TO {
                let (h0, h3) = feed_round(h);
                for (i, s) in sinks.iter().enumerate() {
                    let _ = s.send(if i == 3 { h3 } else { h0 }).await;
                }
                ctx.sleep(Duration::from_millis(50)).await;
            }

            restarted_store
                .read()
                .map(|s| s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
                .unwrap_or(false)
        });
        let _ = std::fs::remove_dir_all(&share_dir);
        assert!(
            seeded,
            "a mid-window restart must resume from the on-disk journal and memoize \
             (PK_2, share) before the boundary"
        );
    }

    /// Resolver ingest (`Consumer::deliver` half): VALID delivered logs converge a
    /// shorthanded ceremony to `ready` (a selectable quorum) and return `deliver→true`;
    /// a forged log fails `check` → `deliver→false` (block the peer) and does not
    /// poison the set; a no-live-ceremony (wrong-epoch) delivery is honest (`true`) and
    /// touches nothing.
    #[test]
    fn resolver_ingest_converges_rejects_forged_and_drops_wrong_epoch() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle: Oracle<PeerPubkey, SimContext> = {
                let (network, oracle) = Network::new(
                    ctx.with_label("sim_net"),
                    SimConfig {
                        max_size: 1024 * 1024,
                        disconnect_on_block: false,
                        tracked_peer_sets: NZUsize!(4),
                    },
                );
                network.start();
                oracle
            };
            let mut rng = StdRng::seed_from_u64(2);
            let keys: Vec<Ed25519PrivateKey> =
                (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;

            // Mint the full committee's SIGNED logs by running a parallel ceremony
            // (the same namespace/epoch/committee ⇒ the same `Info`, so each peer's
            // sealed log `check`s against node-0's actor ceremony).
            let ns = b"FLUENT_DPOS_V1_clocktest";
            let mut cers: BTreeMap<PeerPubkey, DkgCeremony> = BTreeMap::new();
            let mut queue: Vec<(PeerPubkey, Outgoing)> = Vec::new();
            for k in &keys {
                let (cer, step) =
                    DkgCeremony::start(&mut rng, ns, 2, committee.clone(), k.clone()).expect("start");
                let from = k.public_key();
                queue.extend(step.outgoing.into_iter().map(|o| (from.clone(), o)));
                cers.insert(from, cer);
            }
            while let Some((from, o)) = queue.pop() {
                match o.target {
                    Target::Broadcast => {
                        let tos: Vec<PeerPubkey> =
                            cers.keys().filter(|p| **p != from).cloned().collect();
                        for to in tos {
                            let more =
                                cers.get_mut(&to).unwrap().handle(from.clone(), o.msg.body.clone());
                            queue.extend(more.outgoing.into_iter().map(|m| (to.clone(), m)));
                        }
                    }
                    Target::Direct(to) => {
                        if let Some(c) = cers.get_mut(&to) {
                            let more = c.handle(from.clone(), o.msg.body.clone());
                            queue.extend(more.outgoing.into_iter().map(|m| (to.clone(), m)));
                        }
                    }
                }
            }
            let peer_logs: Vec<DealerReveal> = keys[1..]
                .iter()
                .map(|k| {
                    let signed = cers
                        .get_mut(&k.public_key())
                        .unwrap()
                        .seal_dealings()
                        .outgoing
                        .into_iter()
                        .find_map(|o| match o.msg.body {
                            DkgBody::Reveal(s) => Some(*s),
                            _ => None,
                        })
                        .expect("a sealed reveal");
                    signed
                })
                .collect();

            // Build node-0's actor and drive it to deal + seal its OWN ceremony (1 log).
            let me = keys[0].clone();
            let (sender, receiver) = oracle
                .control(me.public_key())
                .register(
                    fluentbase_p2p::constants::BEACON_CHANNEL,
                    fluentbase_p2p::constants::BEACON_QUOTA,
                )
                .await
                .expect("register");
            let committee_for: CommitteeFor = {
                let set = committee.clone();
                Arc::new(move |_e: u64| Some(set.clone()))
            };
            let mut actor: DkgActor<_, _, NoopResolver> = DkgActor::new(
                ns.to_vec(),
                me,
                sender,
                receiver,
                None,
                None,
                committee_for,
                Arc::new(RwLock::new(BTreeMap::new())),
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                None,
                ShareState::Plaintext,
            );
            // Drive node-0 to START its epoch-2 ceremony but NOT seal it (so
            // `drive_finalization`'s `sealed` guard never finalizes+evicts under us —
            // node-0's own `view` lacks the peers' private dealings, so a real finalize
            // would `MissingPlayerDealing` anyway; this test isolates the recovery
            // ingest path, the resolver `Consumer::deliver` half).
            let mut arng = StdRng::seed_from_u64(9);
            for h in 0..=SEAL_DEADLINE {
                actor.on_height(h, &mut arng).await;
            }
            assert!(
                !actor.ceremonies[&DETERMINISTIC_BOOTSTRAP_EPOCH].dealing_closed(),
                "precondition: node-0's ceremony is started but unsealed"
            );
            assert_eq!(
                actor.ceremonies[&DETERMINISTIC_BOOTSTRAP_EPOCH].recorded_log_count(),
                0,
                "before recovery node-0 has recorded no logs (unsealed ⇒ no own log yet)"
            );

            // `peer_logs[i]` is `keys[i+1]`'s sealed log; build the matching
            // `{epoch, dealer}` key per log (`ingest_log` BINDS the delivered log to
            // the requested `key.dealer`). `dealer0` = the first peer dealer.
            let dealer0 = keys[1].public_key();
            let valid_key0 = DkgLogKey {
                epoch: DETERMINISTIC_BOOTSTRAP_EPOCH,
                dealer: dealer0.clone(),
            };

            // A wrong-epoch delivery (no live ceremony for epoch 3) is honest — it
            // returns `true` (don't block the peer) and touches nothing.
            let wrong_epoch_key = DkgLogKey {
                epoch: 3,
                dealer: dealer0.clone(),
            };
            let accepted = actor
                .ingest_log(
                    &wrong_epoch_key,
                    Bytes::from(peer_logs[0].encode().to_vec()),
                    &mut arng,
                )
                .await;
            assert!(accepted, "a no-live-ceremony delivery is honest (deliver→true)");
            assert_eq!(
                actor.ceremonies[&DETERMINISTIC_BOOTSTRAP_EPOCH].recorded_log_count(),
                0,
                "a wrong-epoch delivery must not touch the epoch-2 ceremony"
            );

            // A forged (tampered-signature) log fails `check` → `deliver→false` (the
            // resolver blocks the lying peer) → not recorded.
            let mut forged = peer_logs[0].encode().to_vec();
            *forged.last_mut().unwrap() ^= 0xFF;
            let forged_valid = actor
                .ingest_log(&valid_key0, Bytes::from(forged), &mut arng)
                .await;
            assert!(!forged_valid, "a forged log is check-rejected (deliver→false)");
            assert_eq!(
                actor.ceremonies[&DETERMINISTIC_BOOTSTRAP_EPOCH].recorded_log_count(),
                0,
                "a forged log does not poison the set"
            );

            // [967] UNDECODABLE bytes → `deliver→false` (NOT `true`): `true` would CLEAR
            // the fetch (mark it satisfied), letting one garbage-serving peer permanently
            // kill `key`'s recovery with no log recorded. `false` keeps the fetch alive
            // (the resolver `add_retry`s another peer). Not recorded either way.
            let undecodable = actor
                .ingest_log(&valid_key0, Bytes::from_static(&[0xFF, 0x00, 0x13]), &mut arng)
                .await;
            assert!(
                !undecodable,
                "an undecodable delivery must NOT clear the fetch (deliver→false, retry elsewhere)"
            );
            assert_eq!(
                actor.ceremonies[&DETERMINISTIC_BOOTSTRAP_EPOCH].recorded_log_count(),
                0,
                "an undecodable delivery records nothing"
            );

            // DEALER BINDING: a VALID log for a DIFFERENT dealer (keys[2]) delivered
            // for a fetch of dealer0 (keys[1]) must NOT satisfy the fetch — it returns
            // `deliver→false` (block + re-fetch the requested key) and is not recorded.
            let mismatched = actor
                .ingest_log(
                    &valid_key0,
                    Bytes::from(peer_logs[1].encode().to_vec()),
                    &mut arng,
                )
                .await;
            assert!(
                !mismatched,
                "a valid log for the WRONG dealer must not satisfy the fetch (deliver→false)"
            );
            assert_eq!(
                actor.ceremonies[&DETERMINISTIC_BOOTSTRAP_EPOCH].recorded_log_count(),
                0,
                "a wrong-dealer log is not recorded under the fetched key"
            );

            // Each valid delivery under its OWN dealer key (3 peer logs == dealer-quorum
            // at n=4) is recorded (`deliver→true`) and converges the shorthanded
            // ceremony to a selectable quorum (`ready`).
            let quorum = peer_logs.len();
            for (i, signed) in peer_logs.iter().enumerate() {
                let key = DkgLogKey {
                    epoch: DETERMINISTIC_BOOTSTRAP_EPOCH,
                    dealer: keys[i + 1].public_key(),
                };
                let ok = actor
                    .ingest_log(&key, Bytes::from(signed.encode().to_vec()), &mut arng)
                    .await;
                assert!(ok, "a valid log for its own dealer is recorded (deliver→true)");
            }
            let c = &actor.ceremonies[&DETERMINISTIC_BOOTSTRAP_EPOCH];
            assert_eq!(
                c.recorded_log_count(),
                quorum,
                "every valid delivered log is recorded"
            );
            assert!(
                c.ready(&mut arng),
                "a dealer-quorum of valid logs is selectable (ready) after recovery"
            );
        });
    }

    /// Positive-only serve cache ([965]/[954]) — a cold-miss serve for an UNSERVABLE epoch
    /// (absent / Torn journal) caches NOTHING: it returns `None` and leaves `serve_cache`
    /// untouched. So a Byzantine peer's distinct far-future `key.epoch`s (no journal → empty)
    /// can never accumulate negative entries → `serve_cache` stays bounded ([954]).
    #[test]
    fn unservable_cold_miss_not_cached() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle: Oracle<PeerPubkey, SimContext> = {
                let (network, oracle) = Network::new(
                    ctx.with_label("sim_net"),
                    SimConfig {
                        max_size: 1024 * 1024,
                        disconnect_on_block: false,
                        tracked_peer_sets: NZUsize!(4),
                    },
                );
                network.start();
                oracle
            };
            let mut rng = StdRng::seed_from_u64(8);
            let keys: Vec<Ed25519PrivateKey> =
                (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;

            // A present-but-TORN journal for epoch 2 (1 byte → JournalLoad::Torn).
            let dir = fresh_share_dir("neg-cache");
            std::fs::create_dir_all(&dir).expect("mkdir");
            std::fs::write(journal_path(&dir, 2), [0x01]).expect("write torn journal");

            let mut actor =
                standalone_actor(&oracle, keys[0].clone(), committee, Some(dir.clone())).await;
            let key = DkgLogKey {
                epoch: 2,
                dealer: keys[1].public_key(),
            };
            assert!(actor.serve_log(&key).is_none(), "a Torn journal serves no log");
            assert!(
                !actor.serve_cache.contains_key(&2),
                "an unservable cold miss caches NOTHING — no negative entry ([965]/[954])"
            );
            // [954] bound: many distinct attacker-controlled far-future epochs (no journal)
            // never grow the cache.
            for e in 1_000u64..1_050 {
                assert!(actor.serve_log(&DkgLogKey { epoch: e, dealer: keys[1].public_key() }).is_none());
            }
            assert!(
                actor.serve_cache.is_empty(),
                "future-epoch cold misses accumulate no entries — serve_cache is bounded ([954])"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// [965] regression — a TRANSIENT `committee_for→None` (the documented EVM read race) on a
    /// PRESENT journal must NOT poison the epoch's serve. The empty cold-load is not cached, so
    /// once the committee becomes readable the very next serve re-parses and serves correctly.
    #[test]
    fn transient_committee_none_does_not_poison_serve() {
        let _guard = COLD_PARSE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle: Oracle<PeerPubkey, SimContext> = {
                let (network, oracle) = Network::new(
                    ctx.with_label("sim_net"),
                    SimConfig {
                        max_size: 1024 * 1024,
                        disconnect_on_block: false,
                        tracked_peer_sets: NZUsize!(4),
                    },
                );
                network.start();
                oracle
            };
            let mut rng = StdRng::seed_from_u64(9);
            let keys: Vec<Ed25519PrivateKey> =
                (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;

            // A PRESENT, valid epoch-2 journal (full committee logs).
            let dir = fresh_share_dir("transient-none");
            std::fs::create_dir_all(&dir).expect("mkdir");
            let logs = mint_committee_logs_at(&keys, &committee, 5, 2);
            let me_pk = keys[0].public_key();
            for (pk, signed) in &logs {
                let rec = if *pk == me_pk {
                    JournalRecord::OwnSeal(Box::new(signed.clone()))
                } else {
                    JournalRecord::PeerLog(Box::new(signed.clone()))
                };
                share_state::append_journal(&dir, 2, &rec, &ShareState::Plaintext)
                    .expect("append journal");
            }

            // `committee_for` returns None until `readable` flips — modelling the transient
            // EVM read race at the moment of the first serve.
            let readable = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let committee_for: CommitteeFor = {
                let set = committee.clone();
                let readable = readable.clone();
                Arc::new(move |_e: u64| {
                    readable
                        .load(std::sync::atomic::Ordering::Relaxed)
                        .then(|| set.clone())
                })
            };

            let mut actor =
                standalone_actor_cf(&oracle, keys[0].clone(), committee_for, Some(dir.clone()))
                    .await;
            let key = DkgLogKey {
                epoch: 2,
                dealer: keys[1].public_key(),
            };

            // Committee unreadable → empty cold-load → None, and CRUCIALLY not cached.
            assert!(actor.serve_log(&key).is_none(), "transient None serves no log");
            assert!(
                !actor.serve_cache.contains_key(&2),
                "the transient-None empty result is NOT cached → no permanent poison ([965])"
            );

            // Committee now readable → the SAME serve re-parses and serves the log.
            readable.store(true, std::sync::atomic::Ordering::Relaxed);
            assert!(
                actor.serve_log(&key).is_some(),
                "once the committee is readable the epoch serves correctly — never poisoned ([965])"
            );
            assert!(
                actor.serve_cache.get(&2).is_some_and(|m| !m.is_empty()),
                "the now-servable epoch is cached POSITIVELY"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// [804] uncancelled-fetch leak — `fetch_missing_logs` CANCELS the resolver's
    /// in-flight fetches for an epoch that no longer has an open ceremony (finalized or
    /// swept), so the resolver stops re-issuing dead `{epoch,dealer}` keys forever. An
    /// open shorthanded ceremony issues its missing-dealer fetches; once the ceremony
    /// leaves `ceremonies`, the next `fetch_missing_logs` `retain`s them away.
    #[test]
    fn fetch_missing_logs_cancels_dead_fetches() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle: Oracle<PeerPubkey, SimContext> = {
                let (network, oracle) = Network::new(
                    ctx.with_label("sim_net"),
                    SimConfig {
                        max_size: 1024 * 1024,
                        disconnect_on_block: false,
                        tracked_peer_sets: NZUsize!(4),
                    },
                );
                network.start();
                oracle
            };
            let mut rng = StdRng::seed_from_u64(4);
            let keys: Vec<Ed25519PrivateKey> =
                (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;
            let me = keys[0].clone();
            let (sender, receiver) = oracle
                .control(me.public_key())
                .register(
                    fluentbase_p2p::constants::BEACON_CHANNEL,
                    fluentbase_p2p::constants::BEACON_QUOTA,
                )
                .await
                .expect("register");
            let committee_for: CommitteeFor = {
                let set = committee.clone();
                Arc::new(move |_e: u64| Some(set.clone()))
            };
            let resolver = RecordingResolver::default();
            let in_flight = resolver.in_flight.clone();
            let mut actor = DkgActor::new(
                b"FLUENT_DPOS_V1_clocktest".to_vec(),
                me,
                sender,
                receiver,
                Some(resolver),
                None,
                committee_for,
                Arc::new(RwLock::new(BTreeMap::new())),
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                None,
                ShareState::Plaintext,
            );

            // Inject an OPEN shorthanded ceremony (node-0 started but no peer logs) so
            // `fetch_missing_logs` issues fetches for the 3 missing peer dealers.
            let mut srng = StdRng::seed_from_u64(4);
            let (cer, _step) = DkgCeremony::start(
                &mut srng,
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                keys[0].clone(),
            )
            .expect("start");
            actor.ceremonies.insert(DETERMINISTIC_BOOTSTRAP_EPOCH, cer);
            // Within the open window (height < epoch_start(2)): fetches are issued.
            actor.fetch_missing_logs(SEAL_DEADLINE).await;
            assert!(
                !in_flight.lock().unwrap().is_empty(),
                "an open shorthanded ceremony issues missing-dealer fetches"
            );

            // Finalize/sweep the ceremony (remove it), then re-run fetch_missing_logs:
            // with no open ceremony, `retain` must CANCEL every now-dead fetch.
            actor.ceremonies.clear();
            actor.fetch_missing_logs(SEAL_DEADLINE).await;
            assert!(
                in_flight.lock().unwrap().is_empty(),
                "fetches for an epoch with no open ceremony are cancelled (the [804] leak fix)"
            );
        });
    }

    /// [893] regression — a TRANSIENT `committee_for→None` (the EVM read race) for a LIVE
    /// ceremony must NOT cancel its in-flight recovery fetches. The retain predicate keeps a
    /// key whose epoch is a live ceremony we merely failed to read this tick, so a flapping
    /// committee read can't reset accumulated resolver progress (contrast the [804] case
    /// above: a genuinely dead/swept epoch — absent from `ceremonies` — is still cancelled).
    #[test]
    fn transient_committee_none_does_not_cancel_live_fetches() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle: Oracle<PeerPubkey, SimContext> = {
                let (network, oracle) = Network::new(
                    ctx.with_label("sim_net"),
                    SimConfig {
                        max_size: 1024 * 1024,
                        disconnect_on_block: false,
                        tracked_peer_sets: NZUsize!(4),
                    },
                );
                network.start();
                oracle
            };
            let mut rng = StdRng::seed_from_u64(4);
            let keys: Vec<Ed25519PrivateKey> =
                (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;
            let me = keys[0].clone();
            let (sender, receiver) = oracle
                .control(me.public_key())
                .register(
                    fluentbase_p2p::constants::BEACON_CHANNEL,
                    fluentbase_p2p::constants::BEACON_QUOTA,
                )
                .await
                .expect("register");
            // `committee_for` returns `Some` until `readable` flips — modelling the transient
            // EVM read race on the SECOND tick while the ceremony is still live.
            let readable = Arc::new(std::sync::atomic::AtomicBool::new(true));
            let committee_for: CommitteeFor = {
                let set = committee.clone();
                let readable = readable.clone();
                Arc::new(move |_e: u64| {
                    readable
                        .load(std::sync::atomic::Ordering::Relaxed)
                        .then(|| set.clone())
                })
            };
            let resolver = RecordingResolver::default();
            let in_flight = resolver.in_flight.clone();
            let mut actor = DkgActor::new(
                b"FLUENT_DPOS_V1_clocktest".to_vec(),
                me,
                sender,
                receiver,
                Some(resolver),
                None,
                committee_for,
                Arc::new(RwLock::new(BTreeMap::new())),
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                None,
                ShareState::Plaintext,
            );

            let mut srng = StdRng::seed_from_u64(4);
            let (cer, _step) = DkgCeremony::start(
                &mut srng,
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                keys[0].clone(),
            )
            .expect("start");
            actor.ceremonies.insert(DETERMINISTIC_BOOTSTRAP_EPOCH, cer);

            // Committee readable → fetches issued.
            actor.fetch_missing_logs(SEAL_DEADLINE).await;
            assert!(
                !in_flight.lock().unwrap().is_empty(),
                "an open shorthanded ceremony issues missing-dealer fetches"
            );

            // Transient committee read failure while the ceremony is STILL live: the
            // in-flight fetches must be PRESERVED, not cancelled ([893]).
            readable.store(false, std::sync::atomic::Ordering::Relaxed);
            actor.fetch_missing_logs(SEAL_DEADLINE).await;
            assert!(
                !in_flight.lock().unwrap().is_empty(),
                "a transient committee_for->None for a LIVE ceremony preserves its in-flight fetches ([893])"
            );
        });
    }

    /// Serve-after-finalize. A node that FINALIZED its ceremony but has NOT yet crossed
    /// the epoch boundary must still serve a peer's recorded log from the eager
    /// `serve_cache` (no journal read, no `check`), so a late-restarting peer can
    /// recover it — the all-live-holders-evicted residual. The past-boundary sweep then
    /// reclaims the cache.
    #[test]
    fn serves_finalized_logs_until_boundary_then_evicts() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle: Oracle<PeerPubkey, SimContext> = {
                let (network, oracle) = Network::new(
                    ctx.with_label("sim_net"),
                    SimConfig {
                        max_size: 1024 * 1024,
                        disconnect_on_block: false,
                        tracked_peer_sets: NZUsize!(4),
                    },
                );
                network.start();
                oracle
            };
            let mut rng = StdRng::seed_from_u64(5);
            let keys: Vec<Ed25519PrivateKey> =
                (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;
            for a in &keys {
                for b in &keys {
                    if a.public_key() != b.public_key() {
                        oracle
                            .add_link(
                                a.public_key(),
                                b.public_key(),
                                Link {
                                    latency: Duration::from_millis(0),
                                    jitter: Duration::from_millis(0),
                                    success_rate: 1.0,
                                },
                            )
                            .await
                            .expect("link");
                    }
                }
            }

            // Run the full 4-dealer committee[2] DKG so node-0 finalizes ALL-IN.
            let victim_store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut sinks = Vec::new();
            for (i, k) in keys.iter().enumerate() {
                let store = if i == 0 {
                    victim_store.clone()
                } else {
                    Arc::new(RwLock::new(BTreeMap::new()))
                };
                sinks.push(
                    spawn_dealer(
                        &ctx,
                        &oracle,
                        k.clone(),
                        committee.clone(),
                        store,
                        Arc::new(tokio::sync::Notify::new()),
                        INTERVAL,
                    )
                    .await,
                );
            }
            // Feed up to just before the boundary so node-0 finalizes but is NOT swept.
            for h in 0..=(BOUNDARY - 1) {
                for s in &sinks {
                    let _ = s.send(h).await;
                }
                ctx.sleep(Duration::from_millis(50)).await;
            }
            assert!(
                victim_store
                    .read()
                    .map(|s| s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
                    .unwrap_or(false),
                "precondition: node-0 finalized its epoch-2 share before the boundary"
            );

            // Build a STANDALONE actor (no network drive) and replay the same
            // finalize-then-serve invariant deterministically on its own state: deal +
            // seal + ingest all peer logs → finalize → the finalized logs are served.
            // (We assert the serve index directly on a constructed actor to avoid
            // depending on the spawned task's internal map.)
            let ns = b"FLUENT_DPOS_V1_clocktest";
            let mut cers: BTreeMap<PeerPubkey, DkgCeremony> = BTreeMap::new();
            let mut queue: Vec<(PeerPubkey, Outgoing)> = Vec::new();
            let mut srng = StdRng::seed_from_u64(5);
            for k in &keys {
                let (cer, step) = DkgCeremony::start(&mut srng, ns, 2, committee.clone(), k.clone())
                    .expect("start");
                let from = k.public_key();
                queue.extend(step.outgoing.into_iter().map(|o| (from.clone(), o)));
                cers.insert(from, cer);
            }
            while let Some((from, o)) = queue.pop() {
                match o.target {
                    Target::Broadcast => {
                        let tos: Vec<PeerPubkey> =
                            cers.keys().filter(|p| **p != from).cloned().collect();
                        for to in tos {
                            let more =
                                cers.get_mut(&to).unwrap().handle(from.clone(), o.msg.body.clone());
                            queue.extend(more.outgoing.into_iter().map(|m| (to.clone(), m)));
                        }
                    }
                    Target::Direct(to) => {
                        if let Some(c) = cers.get_mut(&to) {
                            let more = c.handle(from.clone(), o.msg.body.clone());
                            queue.extend(more.outgoing.into_iter().map(|m| (to.clone(), m)));
                        }
                    }
                }
            }
            for k in &keys {
                let step = cers.get_mut(&k.public_key()).unwrap().seal_dealings();
                queue.extend(step.outgoing.into_iter().map(|o| (k.public_key(), o)));
            }
            while let Some((from, o)) = queue.pop() {
                if let Target::Broadcast = o.target {
                    let tos: Vec<PeerPubkey> =
                        cers.keys().filter(|p| **p != from).cloned().collect();
                    for to in tos {
                        let more =
                            cers.get_mut(&to).unwrap().handle(from.clone(), o.msg.body.clone());
                        queue.extend(more.outgoing.into_iter().map(|m| (to.clone(), m)));
                    }
                }
            }
            let me = keys[0].clone();
            let (sender, receiver) = oracle
                .control(me.public_key())
                .register(
                    fluentbase_p2p::constants::BEACON_CHANNEL,
                    fluentbase_p2p::constants::BEACON_QUOTA,
                )
                .await
                .expect("register");
            let committee_for: CommitteeFor = {
                let set = committee.clone();
                Arc::new(move |_e: u64| Some(set.clone()))
            };
            let mut actor: DkgActor<_, _, NoopResolver> = DkgActor::new(
                ns.to_vec(),
                me,
                sender,
                receiver,
                None,
                None,
                committee_for,
                Arc::new(RwLock::new(BTreeMap::new())),
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                None,
                ShareState::Plaintext,
            );
            // Inject node-0's fully-recorded ceremony (already sealed in the queue
            // drive ⇒ `me ∈ recorded`, so the derived finalize gate passes), then drive
            // finalize (pre-boundary). After finalize node-0 holds no live ceremony but
            // DOES hold the `serve_cache` copy for the epoch.
            actor
                .ceremonies
                .insert(DETERMINISTIC_BOOTSTRAP_EPOCH, cers.remove(&keys[0].public_key()).unwrap());
            let mut arng = StdRng::seed_from_u64(9);
            actor.drive_finalization(BOUNDARY - 1, &mut arng);
            assert!(
                !actor.ceremonies.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH),
                "the finalized ceremony left `ceremonies`"
            );

            // SERVE-AFTER-FINALIZE: a peer's log is still served from the serve cache.
            let peer = keys[1].public_key();
            let key = DkgLogKey {
                epoch: DETERMINISTIC_BOOTSTRAP_EPOCH,
                dealer: peer.clone(),
            };
            assert!(
                actor.serve_log(&key).is_some(),
                "a finalized-but-pre-boundary node still serves a peer log (the residual close)"
            );

            // PAST-BOUNDARY SWEEP: a height past the epoch-2 boundary reclaims the
            // index → the log is no longer served.
            actor.on_height(BOUNDARY + 1, &mut arng).await;
            assert!(
                actor.serve_log(&key).is_none(),
                "the finalized-log serve index is reclaimed once the boundary passes"
            );
        });
    }

    /// Path of `epoch`'s on-disk DKG journal under `dir` (the test's view of the
    /// durable artifact, to assert the post-restart reconcile actually deleted it).
    fn journal_path(dir: &std::path::Path, epoch: u64) -> PathBuf {
        dir.join(format!("beacon-dkgjournal-e{epoch}.bin"))
    }

    /// Serializes the tests that touch the process-global `COLD_PARSE_COUNT` (the two that
    /// read it + the transient-None test that increments it via a re-parse) so a parallel
    /// run cannot interleave their parse counts.
    static COLD_PARSE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Build a STANDALONE actor (no network drive) over `committee` with the given
    /// `share_dir` and a COLD `serve_cache`, mirroring the production construction.
    async fn standalone_actor(
        oracle: &Oracle<PeerPubkey, SimContext>,
        me: Ed25519PrivateKey,
        committee: Set<PeerPubkey>,
        share_dir: Option<PathBuf>,
    ) -> DkgActor<
        commonware_p2p::simulated::Sender<PeerPubkey, SimContext>,
        commonware_p2p::simulated::Receiver<PeerPubkey>,
        NoopResolver,
    > {
        let committee_for: CommitteeFor = {
            let set = committee.clone();
            Arc::new(move |_e: u64| Some(set.clone()))
        };
        standalone_actor_cf(oracle, me, committee_for, share_dir).await
    }

    /// Like [`standalone_actor`] but takes an explicit `committee_for` closure, so a test
    /// can model a TRANSIENT `committee_for→None` EVM read race (review [965]).
    async fn standalone_actor_cf(
        oracle: &Oracle<PeerPubkey, SimContext>,
        me: Ed25519PrivateKey,
        committee_for: CommitteeFor,
        share_dir: Option<PathBuf>,
    ) -> DkgActor<
        commonware_p2p::simulated::Sender<PeerPubkey, SimContext>,
        commonware_p2p::simulated::Receiver<PeerPubkey>,
        NoopResolver,
    > {
        let (sender, receiver) = oracle
            .control(me.public_key())
            .register(
                fluentbase_p2p::constants::BEACON_CHANNEL,
                fluentbase_p2p::constants::BEACON_QUOTA,
            )
            .await
            .expect("register");
        DkgActor::new(
            b"FLUENT_DPOS_V1_clocktest".to_vec(),
            me,
            sender,
            receiver,
            None,
            None,
            committee_for,
            Arc::new(RwLock::new(BTreeMap::new())),
            Arc::new(tokio::sync::Notify::new()),
            ACTIVATION,
            INTERVAL,
            crate::beacon::metrics::BeaconMetrics::default(),
            share_dir,
            ShareState::Plaintext,
        )
    }

    /// Post-restart serve from a COLD cache (R1) + post-restart journal eviction (R2) +
    /// the cold-cache fetch-burst bound (e). All three are standalone (no network): a
    /// fresh actor whose `serve_cache` is empty but whose epoch-2 journal is present on
    /// disk must serve from the journal (cold-miss parse, R1); a first `on_height` past
    /// the boundary must reconcile-delete the journal so the serve then returns `None`
    /// (R2); and M ≫ K serve calls across K cold epochs must parse exactly K times (e).
    #[test]
    fn post_restart_serve_evict_and_burst_bound() {
        let _guard = COLD_PARSE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle: Oracle<PeerPubkey, SimContext> = {
                let (network, oracle) = Network::new(
                    ctx.with_label("sim_net"),
                    SimConfig {
                        max_size: 1024 * 1024,
                        disconnect_on_block: false,
                        tracked_peer_sets: NZUsize!(4),
                    },
                );
                network.start();
                oracle
            };
            let mut rng = StdRng::seed_from_u64(5);
            let keys: Vec<Ed25519PrivateKey> =
                (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;

            let dir = fresh_share_dir("cold-serve");
            std::fs::create_dir_all(&dir).expect("mkdir");
            // Write epoch-2's full committee logs to the on-disk journal (the durable
            // source a restart re-reads). Tag node-0's own log as OwnSeal, peers as
            // PeerLog — exactly what the live actor would have journaled pre-finalize.
            let logs = mint_committee_logs_at(&keys, &committee, 5, 2);
            let me_pk = keys[0].public_key();
            for (pk, signed) in &logs {
                let rec = if *pk == me_pk {
                    JournalRecord::OwnSeal(Box::new(signed.clone()))
                } else {
                    JournalRecord::PeerLog(Box::new(signed.clone()))
                };
                share_state::append_journal(&dir, 2, &rec, &ShareState::Plaintext)
                    .expect("append journal");
            }

            COLD_PARSE_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
            let mut actor =
                standalone_actor(&oracle, keys[0].clone(), committee.clone(), Some(dir.clone()))
                    .await;

            // (c) R1: cold cache + present journal ⇒ the cold-miss parse serves a peer's
            // log. An in-memory serve index that a restart wipes returns `None` here.
            let peer_key = DkgLogKey {
                epoch: 2,
                dealer: keys[1].public_key(),
            };
            assert!(
                actor.serve_log(&peer_key).is_some(),
                "a restarted node serves a finalized epoch's log from the journal-backed cache (R1)"
            );
            assert_eq!(
                COLD_PARSE_COUNT.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "the first serve cold-parses the journal exactly once"
            );

            // (e) burst bound: a second serve for the SAME epoch (different dealer) is a
            // cache hit — NO additional parse.
            let peer_key2 = DkgLogKey {
                epoch: 2,
                dealer: keys[2].public_key(),
            };
            assert!(actor.serve_log(&peer_key2).is_some());
            assert!(actor.serve_log(&peer_key).is_some());
            assert_eq!(
                COLD_PARSE_COUNT.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "repeated serves within a cold-loaded epoch add zero parses (one-per-epoch bound)"
            );

            // (d) R2: a first `on_height` landing PAST the epoch-2 boundary runs the
            // first-tick reconcile, deleting the journal; the serve then returns `None`.
            let mut arng = StdRng::seed_from_u64(9);
            actor.on_height(BOUNDARY + 1, &mut arng).await;
            assert!(
                !journal_path(&dir, 2).exists(),
                "the boundary-passed journal is reconcile-deleted after a restart (R2)"
            );
            assert!(
                actor.serve_log(&peer_key).is_none(),
                "a boundary-passed epoch is no longer served (journal gone + cache evicted)"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// (e) — multi-epoch cold-cache fetch-burst stays bounded at ONE parse per epoch.
    /// Construct an actor with a COLD cache and journals for K distinct finalized epochs
    /// (all `> now`, so the first-tick reconcile keeps them), then issue M ≫ K serve
    /// calls spread across those epochs and assert the parse count == K.
    #[test]
    fn multi_epoch_cold_burst_parses_once_per_epoch() {
        let _guard = COLD_PARSE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle: Oracle<PeerPubkey, SimContext> = {
                let (network, oracle) = Network::new(
                    ctx.with_label("sim_net"),
                    SimConfig {
                        max_size: 1024 * 1024,
                        disconnect_on_block: false,
                        tracked_peer_sets: NZUsize!(4),
                    },
                );
                network.start();
                oracle
            };
            let mut rng = StdRng::seed_from_u64(13);
            let keys: Vec<Ed25519PrivateKey> =
                (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;

            let dir = fresh_share_dir("multi-cold");
            std::fs::create_dir_all(&dir).expect("mkdir");
            // The seal `Info` embeds the epoch, so a log minted at epoch E only `check`s
            // under the epoch-E `Info` — mint each of K distinct finalized epochs at its
            // own epoch (so `checked_serve_map` passes). The actor is never ticked, so
            // `now=epoch_of(0)=0` and all journals stay `> now` (no reconcile).
            const K_EPOCHS: u64 = 3;
            let mut peer0_keys = Vec::new();
            for e in 2..2 + K_EPOCHS {
                let logs = mint_committee_logs_at(&keys, &committee, 13 + e, e);
                for (_pk, signed) in &logs {
                    let rec = JournalRecord::PeerLog(Box::new(signed.clone()));
                    share_state::append_journal(&dir, e, &rec, &ShareState::Plaintext)
                        .expect("append");
                }
                peer0_keys.push(DkgLogKey {
                    epoch: e,
                    dealer: keys[1].public_key(),
                });
            }

            COLD_PARSE_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
            let mut actor =
                standalone_actor(&oracle, keys[0].clone(), committee.clone(), Some(dir.clone()))
                    .await;

            // M = 10 * K serve calls spread across the K epochs (repeated within each).
            for _round in 0..10 {
                for key in &peer0_keys {
                    assert!(
                        actor.serve_log(key).is_some(),
                        "each cold epoch's log is servable from its journal"
                    );
                }
            }
            assert_eq!(
                COLD_PARSE_COUNT.load(std::sync::atomic::Ordering::Relaxed),
                K_EPOCHS,
                "M ≫ K serve calls cold-parse exactly K times (one per epoch — the DoS bound)"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// End-to-end mid-window restart over a REAL wired resolver (the T1 gap). n=7 so
    /// `f=2`, dealer-quorum `n−f=5`: the restarted victim resumes with only 4 logs
    /// (< quorum), and — because the gossip reveals already fired one-shot — the ONLY
    /// path back to quorum 5 is the resolver fetch. Asserts the victim memoizes
    /// `(PK_2, share)` ONLY after the fetch lands.
    ///
    /// Knobs:
    /// - `corrupt_own_seal`: flip a byte in the victim's `OwnSeal` journal frame so it
    ///   resumes `me ∉ recorded` (R3/R3b — it must re-fetch its OWN log AND finalize).
    /// - `use_resolver`: spawn the restarted victim WITH the real engine (recovers) or
    ///   with `NoopResolver` (the contrast — must NOT seed, proving the resolver, not
    ///   resume+settle, closes the gap).
    async fn run_resolver_restart(
        ctx: SimContext,
        corrupt_own_seal: bool,
        use_resolver: bool,
    ) -> bool {
        const N: usize = 7;
        const RESTART_AT: u64 = SEAL_DEADLINE + 2;
        const FEED_TO: u64 = BOUNDARY - 1;

        let oracle: Oracle<PeerPubkey, SimContext> = {
            let (network, oracle) = Network::new(
                ctx.with_label("sim_net"),
                SimConfig {
                    max_size: 1024 * 1024,
                    disconnect_on_block: false,
                    tracked_peer_sets: NZUsize!(4),
                },
            );
            network.start();
            oracle
        };
        let mut rng = StdRng::seed_from_u64(1);
        let keys: Vec<Ed25519PrivateKey> =
            (0..N).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        oracle.manager().track(0, committee.clone()).await;

        let link = || Link {
            latency: Duration::from_millis(0),
            jitter: Duration::from_millis(0),
            success_rate: 1.0,
        };
        // Full mesh among the 6 HOLDERS (nodes 1..6) so each records all 7 logs and can
        // serve any of them. Node-0 (victim) is linked ONLY to {1,2,3} pre-restart, so
        // it records its own + 3 peer logs = 4 < quorum 5. Nodes {4,5,6}'s gossip
        // reveals never reach it (no link), and won't re-arrive (one-shot).
        let mut links: Vec<(usize, usize)> = Vec::new();
        for i in 1..N {
            for j in 1..N {
                if i != j {
                    links.push((i, j));
                }
            }
        }
        for j in [1usize, 2, 3] {
            links.push((0, j));
            links.push((j, 0));
        }
        for (a, b) in links {
            oracle
                .add_link(keys[a].public_key(), keys[b].public_key(), link())
                .await
                .expect("link");
        }

        let restarted_store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
        let dir = fresh_share_dir(if corrupt_own_seal { "res-torn" } else { "res-plain" });
        let mut sinks = Vec::new();
        for (i, k) in keys.iter().enumerate() {
            let store = Arc::new(RwLock::new(BTreeMap::new()));
            let dir_i = if i == 0 { Some(dir.clone()) } else { None };
            sinks.push(
                spawn_dealer_resolved(
                    &ctx,
                    &oracle,
                    k.clone(),
                    committee.clone(),
                    store,
                    Arc::new(tokio::sync::Notify::new()),
                    INTERVAL,
                    dir_i,
                    7,
                )
                .await,
            );
        }

        // Feed up to the victim's seal so it journals its (partial) ceremony progress.
        for h in 0..=RESTART_AT {
            for s in &sinks {
                let _ = s.send(h).await;
            }
            ctx.sleep(Duration::from_millis(50)).await;
        }

        // RESTART node-0: abort the old task (re-register overwrites its channels) and
        // re-spawn a FRESH actor over the SAME share_dir + a FRESH store.
        drop(sinks.remove(0));
        if corrupt_own_seal {
            corrupt_own_seal_record(&dir, 2);
        }
        // For the resolver case, ADD links node-0 ↔ {4,5,6} so the resolver can reach
        // the holders of the missing dealer logs (gossip won't re-deliver them).
        if use_resolver {
            for j in [4usize, 5, 6] {
                oracle.add_link(keys[0].public_key(), keys[j].public_key(), link())
                    .await
                    .expect("link");
                oracle.add_link(keys[j].public_key(), keys[0].public_key(), link())
                    .await
                    .expect("link");
            }
        }
        let new_sink = if use_resolver {
            spawn_dealer_resolved(
                &ctx,
                &oracle,
                keys[0].clone(),
                committee.clone(),
                restarted_store.clone(),
                Arc::new(tokio::sync::Notify::new()),
                INTERVAL,
                Some(dir.clone()),
                99,
            )
            .await
        } else {
            spawn_dealer_at(
                &ctx,
                &oracle,
                keys[0].clone(),
                committee.clone(),
                restarted_store.clone(),
                Arc::new(tokio::sync::Notify::new()),
                INTERVAL,
                Some(dir.clone()),
                99,
            )
            .await
        };
        sinks.insert(0, new_sink);

        // Tick on (within the window) so the rebuilt actor resumes, fetches its missing
        // logs via the resolver, and finalizes over the settled set. The resolver fetch
        // is multi-round (request → `initial` delay → serve → deliver), so after the
        // last in-window height we re-tick the SAME frontier (still pre-boundary) a few
        // times — each `on_height` re-issues the still-missing keys + re-runs
        // `drive_finalization` — giving the resolver wall-time to converge.
        for h in (RESTART_AT + 1)..=FEED_TO {
            for s in &sinks {
                let _ = s.send(h).await;
            }
            ctx.sleep(Duration::from_millis(100)).await;
        }
        for _ in 0..20 {
            for s in &sinks {
                let _ = s.send(FEED_TO).await;
            }
            ctx.sleep(Duration::from_millis(100)).await;
        }

        let seeded = restarted_store
            .read()
            .map(|s| s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
            .unwrap_or(false);
        let _ = std::fs::remove_dir_all(&dir);
        seeded
    }

    /// Targeted corruption of the victim's `OwnSeal` journal record: walk the length-
    /// prefixed records, find the one whose `rec_tag == REC_OWN_SEAL`, and flip a byte
    /// in the TAIL of its `SignedDealerLog` body (the signature region). The record
    /// still DECODES structurally (framing + length prefix intact, so subsequent
    /// `PeerLog` records survive the load), but its `check` now FAILS — so the node
    /// resumes `me ∉ recorded` while still holding its peer logs. This is the exact
    /// torn-own-seal state that forces an OWN-log re-fetch (R3) + a derived-gate
    /// finalize (R3b). Frame: `u32_be(len) ‖ tag(1=plaintext) ‖ rec_tag(1) ‖ body`.
    fn corrupt_own_seal_record(dir: &std::path::Path, epoch: u64) {
        const REC_OWN_SEAL: u8 = 1;
        let path = journal_path(dir, epoch);
        let mut bytes = std::fs::read(&path).expect("read journal");
        let mut off = 0usize;
        let mut corrupted = false;
        while off + 4 <= bytes.len() {
            let len = u32::from_be_bytes(bytes[off..off + 4].try_into().unwrap()) as usize;
            let body_start = off + 4;
            let body_end = body_start + len;
            assert!(body_end <= bytes.len(), "journal record overruns file");
            // body = tag(plaintext=0) ‖ rec_tag ‖ signed-log bytes.
            let rec_tag = bytes[body_start + 1];
            if rec_tag == REC_OWN_SEAL {
                // Flip a byte in the last quarter of the signed-log body (the sig tail),
                // leaving the length prefix + tags intact so the record still parses.
                let sig_idx = body_start + 2 + len.saturating_sub(2) * 3 / 4;
                bytes[sig_idx] ^= 0xFF;
                corrupted = true;
                break;
            }
            off = body_end;
        }
        assert!(corrupted, "an OwnSeal record must exist in the victim's journal");
        std::fs::write(&path, bytes).expect("rewrite corrupted journal");
    }

    /// (a) fetch-gated finalize: the restarted victim memoizes its share ONLY via the
    /// resolver fetch (resume gives 4 < quorum 5), AND the `NoopResolver` contrast does
    /// NOT seed — proving the resolver, not resume+settle, closes the gap.
    #[test]
    fn restart_midwindow_recovers_via_resolver() {
        let runtime = deterministic::Runner::timed(Duration::from_secs(60));
        let seeded =
            runtime.start(|ctx| async move { run_resolver_restart(ctx, false, true).await });
        assert!(
            seeded,
            "(a) a shorthanded restarted victim reaches quorum + finalizes via the resolver fetch"
        );

        let runtime = deterministic::Runner::timed(Duration::from_secs(60));
        let seeded_noop =
            runtime.start(|ctx| async move { run_resolver_restart(ctx, false, false).await });
        assert!(
            !seeded_noop,
            "(a-contrast) with NO resolver the victim stays at 4 < quorum 5 and never seeds"
        );
    }

    /// (b) torn-own-seal must RE-FETCH its own log AND FINALIZE (R3/R3b). The victim's
    /// `OwnSeal` frame is corrupted so it resumes `me ∉ recorded`; the derived finalize
    /// gate (`own_log_recorded`) must flip true once the resolver re-fetches its OWN
    /// log. This FAILS on the pre-fix code TWO ways: the unconditional `me`-skip blocks
    /// the own-log fetch, and even with it the `self.sealed`-based gate (never set on a
    /// torn-own-seal resume) blocks the finalize. It passes only by derivation.
    #[test]
    fn torn_own_seal_refetches_own_log_and_finalizes() {
        let runtime = deterministic::Runner::timed(Duration::from_secs(60));
        let seeded =
            runtime.start(|ctx| async move { run_resolver_restart(ctx, true, true).await });
        assert!(
            seeded,
            "(b) a torn-own-seal victim re-fetches its OWN log via the resolver and FINALIZES \
             (the seal-state derivation, not a flag)"
        );
    }

    /// `mint_committee_logs` at an explicit `epoch` (the seal `Info` embeds the epoch,
    /// so a log minted at epoch E only `check`s under the epoch-E `Info`).
    fn mint_committee_logs_at(
        keys: &[Ed25519PrivateKey],
        committee: &Set<PeerPubkey>,
        seed: u64,
        epoch: u64,
    ) -> Vec<(PeerPubkey, DealerReveal)> {
        let ns = b"FLUENT_DPOS_V1_clocktest";
        let mut rng = StdRng::seed_from_u64(seed);
        let mut cers: BTreeMap<PeerPubkey, DkgCeremony> = BTreeMap::new();
        let mut queue: Vec<(PeerPubkey, Outgoing)> = Vec::new();
        for k in keys {
            let (cer, step) =
                DkgCeremony::start(&mut rng, ns, epoch, committee.clone(), k.clone()).expect("start");
            let from = k.public_key();
            queue.extend(step.outgoing.into_iter().map(|o| (from.clone(), o)));
            cers.insert(from, cer);
        }
        while let Some((from, o)) = queue.pop() {
            match o.target {
                Target::Broadcast => {
                    let tos: Vec<PeerPubkey> =
                        cers.keys().filter(|p| **p != from).cloned().collect();
                    for to in tos {
                        let more =
                            cers.get_mut(&to).unwrap().handle(from.clone(), o.msg.body.clone());
                        queue.extend(more.outgoing.into_iter().map(|m| (to.clone(), m)));
                    }
                }
                Target::Direct(to) => {
                    if let Some(c) = cers.get_mut(&to) {
                        let more = c.handle(from.clone(), o.msg.body.clone());
                        queue.extend(more.outgoing.into_iter().map(|m| (to.clone(), m)));
                    }
                }
            }
        }
        keys.iter()
            .map(|k| {
                let pk = k.public_key();
                let signed = cers
                    .get_mut(&pk)
                    .unwrap()
                    .seal_dealings()
                    .outgoing
                    .into_iter()
                    .find_map(|o| match o.msg.body {
                        DkgBody::Reveal(s) => Some(*s),
                        _ => None,
                    })
                    .expect("a sealed reveal");
                (pk, signed)
            })
            .collect()
    }

    /// Drive a 4-party committee[2] DKG at the ceremony level and capture node-0's
    /// journal records WITHOUT sealing node-0 — the exact on-disk state of a member that
    /// crashed AFTER acking every peer (full `Player.view`) but BEFORE its own
    /// `seal_dealings`. The peers DO seal + reveal, so node-0 records their `PeerLog`s;
    /// node-0 has NO `OwnSeal` and its own log is never broadcast (no peer holds it).
    /// Returns `(committee, key0, journal0)` — the inputs a pre-seal `resume` rebuilds
    /// node-0 from.
    fn node0_pre_seal_journal(
        seed: u64,
    ) -> (Set<PeerPubkey>, Ed25519PrivateKey, Vec<JournalRecord>) {
        let ns = b"FLUENT_DPOS_V1_clocktest";
        let mut rng = StdRng::seed_from_u64(seed);
        let keys: Vec<Ed25519PrivateKey> =
            (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let pk0 = keys[0].public_key();

        let mut cers: BTreeMap<PeerPubkey, DkgCeremony> = BTreeMap::new();
        let mut queue: Vec<(PeerPubkey, Outgoing)> = Vec::new();
        let mut journal0: Vec<JournalRecord> = Vec::new();
        for k in &keys {
            let (cer, step) =
                DkgCeremony::start(&mut rng, ns, 2, committee.clone(), k.clone()).expect("start");
            let from = k.public_key();
            if from == pk0 {
                journal0.extend(step.journal);
            }
            queue.extend(step.outgoing.into_iter().map(|o| (from.clone(), o)));
            cers.insert(from, cer);
        }
        let drive = |cers: &mut BTreeMap<PeerPubkey, DkgCeremony>,
                     queue: &mut Vec<(PeerPubkey, Outgoing)>,
                     journal0: &mut Vec<JournalRecord>| {
            while let Some((from, o)) = queue.pop() {
                match o.target {
                    Target::Broadcast => {
                        let tos: Vec<PeerPubkey> =
                            cers.keys().filter(|p| **p != from).cloned().collect();
                        for to in tos {
                            let more =
                                cers.get_mut(&to).unwrap().handle(from.clone(), o.msg.body.clone());
                            if to == pk0 {
                                journal0.extend(more.journal);
                            }
                            queue.extend(more.outgoing.into_iter().map(|m| (to.clone(), m)));
                        }
                    }
                    Target::Direct(to) => {
                        if let Some(c) = cers.get_mut(&to) {
                            let more = c.handle(from.clone(), o.msg.body.clone());
                            if to == pk0 {
                                journal0.extend(more.journal);
                            }
                            queue.extend(more.outgoing.into_iter().map(|m| (to.clone(), m)));
                        }
                    }
                }
            }
        };
        drive(&mut cers, &mut queue, &mut journal0);
        // The PEERS (not node-0) seal + broadcast their reveals; node-0 records their
        // `PeerLog`s but never seals its own (the pre-seal crash).
        for k in &keys[1..] {
            let step = cers.get_mut(&k.public_key()).unwrap().seal_dealings();
            queue.extend(step.outgoing.into_iter().map(|o| (k.public_key(), o)));
        }
        drive(&mut cers, &mut queue, &mut journal0);
        assert!(
            !journal0
                .iter()
                .any(|r| matches!(r, JournalRecord::OwnSeal(_))),
            "node-0 must have NO OwnSeal (it crashed before sealing)"
        );
        (committee, keys[0].clone(), journal0)
    }

    /// [1] regression — a PRE-SEAL crash player-only node FINALIZES. node-0 crashed
    /// after acking every dealer (full `view`) but before its own seal, so it resumes
    /// `me ∉ recorded` and NO peer holds its never-broadcast log to re-fetch. The
    /// finalize gate is now `dealing_closed()`, NOT `own_log_recorded(me)`:
    /// `Player::finalize` recovers node-0's share purely as a player from its `view` +
    /// the n−f survivors' selected logs, so it must memoize `(PK_2, share)`. Under the
    /// old `own_log_recorded` gate it would be blocked FOREVER (shareless → liveness
    /// slash — the failure this gate change prevents).
    #[test]
    fn pre_seal_player_only_finalizes_without_own_log() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle: Oracle<PeerPubkey, SimContext> = {
                let (network, oracle) = Network::new(
                    ctx.with_label("sim_net"),
                    SimConfig {
                        max_size: 1024 * 1024,
                        disconnect_on_block: false,
                        tracked_peer_sets: NZUsize!(4),
                    },
                );
                network.start();
                oracle
            };
            let (committee, key0, pre_seal) = node0_pre_seal_journal(21);
            oracle.manager().track(0, committee.clone()).await;
            let me0 = key0.public_key();

            // Resume node-0 from the pre-seal journal: full view, dealer retired, own log
            // absent. (This is the `resume_before_seal_is_absentee_not_redeal` state.)
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                pre_seal,
            )
            .expect("pre-seal resume");
            assert!(
                resumed.ceremony.dealing_closed(),
                "pre-seal resume retires the dealer"
            );
            assert!(
                !resumed.ceremony.own_log_recorded(&me0),
                "pre-seal resume has NO own log recorded (the case the old gate blocked)"
            );

            let store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut actor = standalone_actor(&oracle, key0, committee, None).await;
            actor.store = store.clone();
            actor
                .ceremonies
                .insert(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);

            // Past the settle deadline so the deterministic-settle fallback fires over the
            // n−f survivors (node-0's own log is genuinely absent + unrecoverable).
            let mut arng = StdRng::seed_from_u64(9);
            actor.drive_finalization(SEAL_DEADLINE + DKG_SETTLE_BLOCKS + 1, &mut arng);
            assert!(
                store
                    .read()
                    .map(|s| s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
                    .unwrap_or(false),
                "a pre-seal player-only node finalizes its share from `view` + survivors \
                 (gate = dealing_closed, not own_log_recorded)"
            );
        });
    }

    /// [1] negative — a STILL-DEALING ceremony (dealer alive) does NOT finalize early.
    /// `dealing_closed()` is false before `seal_dealings`, so even a ceremony that has
    /// somehow collected a ready quorum must wait for the seal (the seal-before-finalize
    /// contract the gate now states via the durable dealer-taken state).
    #[test]
    fn dealing_open_ceremony_does_not_finalize_early() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle: Oracle<PeerPubkey, SimContext> = {
                let (network, oracle) = Network::new(
                    ctx.with_label("sim_net"),
                    SimConfig {
                        max_size: 1024 * 1024,
                        disconnect_on_block: false,
                        tracked_peer_sets: NZUsize!(4),
                    },
                );
                network.start();
                oracle
            };
            let mut rng = StdRng::seed_from_u64(7);
            let keys: Vec<Ed25519PrivateKey> =
                (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;

            // A freshly STARTED ceremony (dealer still Some) — never sealed.
            let mut srng = StdRng::seed_from_u64(7);
            let (cer, _step) = DkgCeremony::start(
                &mut srng,
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                keys[0].clone(),
            )
            .expect("start");
            assert!(
                !cer.dealing_closed(),
                "a started-but-unsealed ceremony is NOT dealing-closed"
            );

            let store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut actor = standalone_actor(&oracle, keys[0].clone(), committee, None).await;
            actor.store = store.clone();
            actor.ceremonies.insert(DETERMINISTIC_BOOTSTRAP_EPOCH, cer);
            let mut arng = StdRng::seed_from_u64(9);
            actor.drive_finalization(BOUNDARY - 1, &mut arng);
            assert!(
                store.read().map(|s| s.is_empty()).unwrap_or(false),
                "a dealing-open ceremony must NOT finalize before its seal"
            );
        });
    }

    /// [2] regression — a `finalize→Err` does NOT destroy the ceremony / forfeit the
    /// share. We construct the exact post-resume `MissingPlayerDealing` state the task
    /// flags: node-0 resumes holding ONLY its own self-dealing (its `view` lacks every
    /// peer's dealing), then the resolver delivers the 3 peer logs (each ACKING node-0).
    /// `select` then picks a quorum of peers whose private dealings node-0's `view`
    /// lacks → `Player::finalize` returns `MissingPlayerDealing`. The ceremony MUST
    /// survive in `actor.ceremonies` (no store entry, `can_finalize()` now false so the
    /// gate stops re-pulling it). Pre-fix `drive_finalization` removed + consumed the
    /// ceremony BEFORE finalize, so the Err destroyed it (`dkg_ceremony_fail` + share
    /// forfeited).
    #[test]
    fn finalize_err_retains_ceremony_not_destroys() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle: Oracle<PeerPubkey, SimContext> = {
                let (network, oracle) = Network::new(
                    ctx.with_label("sim_net"),
                    SimConfig {
                        max_size: 1024 * 1024,
                        disconnect_on_block: false,
                        tracked_peer_sets: NZUsize!(4),
                    },
                );
                network.start();
                oracle
            };
            let (committee, key0, self_only_journal, peer_logs) = node0_self_only_resume(33);
            oracle.manager().track(0, committee.clone()).await;

            // Resume holding only our self-dealing: view = {me}, no peer logs yet, so
            // resume's ack-vs-view check passes (no peer log to conflict).
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                self_only_journal,
            )
            .expect("self-only resume");

            let store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut actor = standalone_actor(&oracle, key0, committee, None).await;
            actor.store = store.clone();
            actor
                .ceremonies
                .insert(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);

            // Deliver the 3 peer logs via the resolver-ingest path (each acks node-0).
            // Now `recorded` has all 4 dealers, `ready()` (observe) returns Ok — but
            // node-0's `view` lacks the peers' private dealings.
            let mut arng = StdRng::seed_from_u64(9);
            for (dealer, signed) in peer_logs {
                let key = DkgLogKey {
                    epoch: DETERMINISTIC_BOOTSTRAP_EPOCH,
                    dealer,
                };
                // ingest_log drives finalize internally; the FIRST one that completes the
                // quorum trips MissingPlayerDealing. It must NOT destroy the ceremony.
                let _ = actor.ingest_log(&key, signed.encode(), &mut arng).await;
            }
            actor.drive_finalization(SEAL_DEADLINE + DKG_SETTLE_BLOCKS + 1, &mut arng);

            assert!(
                store
                    .read()
                    .map(|s| !s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
                    .unwrap_or(false),
                "finalize-Err (MissingPlayerDealing) stores NO share"
            );
            let c = actor
                .ceremonies
                .get(&DETERMINISTIC_BOOTSTRAP_EPOCH)
                .expect("the ceremony must SURVIVE a finalize-Err (not be destroyed)");
            assert!(
                !c.can_finalize(),
                "after a finalize-Err the consumed-player ceremony is no longer re-pulled \
                 into a destructive finalize, yet remains in the map to keep serving"
            );
        });
    }

    /// Drive a 4-party committee[2] DKG and return node-0's SELF-ONLY resume journal
    /// (only its own `ReceivedDealing` — its `view` will lack every peer's dealing) plus
    /// the 3 sealed peer logs (each acking node-0). Resuming from the self-only journal
    /// then ingesting the peer logs reproduces the post-resume `MissingPlayerDealing`
    /// state (a delivered log acks us but our `view` lacks its dealing).
    #[allow(clippy::type_complexity)]
    fn node0_self_only_resume(
        seed: u64,
    ) -> (
        Set<PeerPubkey>,
        Ed25519PrivateKey,
        Vec<JournalRecord>,
        Vec<(PeerPubkey, DealerReveal)>,
    ) {
        let (committee, key0, full_journal) = node0_pre_seal_journal_full_sealed(seed);
        let me0 = key0.public_key();
        let info = info_for_test(&committee);
        // `JournalRecord` is intentionally not `Clone` (it holds secret `DealerPrivMsg`),
        // so partition the OWNED journal in one pass: keep our own self-dealing (resume
        // builds view = {me}; with no peer logs present, resume succeeds), and split out
        // the peer logs to deliver post-resume.
        let mut self_only: Vec<JournalRecord> = Vec::new();
        let mut peer_logs: Vec<(PeerPubkey, DealerReveal)> = Vec::new();
        for r in full_journal {
            match r {
                JournalRecord::ReceivedDealing(ref d, _, _) if *d == me0 => self_only.push(r),
                JournalRecord::PeerLog(signed) => {
                    if let Some((pk, _)) = (*signed).clone().check(&info) {
                        if pk != me0 {
                            peer_logs.push((pk, *signed));
                        }
                    }
                }
                _ => {}
            }
        }
        (committee, key0, self_only, peer_logs)
    }

    /// Like [`node0_pre_seal_journal`] but node-0 ALSO seals (a complete sealed journal:
    /// ReceivedDealings + OwnSeal + every PeerLog) — the base the [2] torn-view journals
    /// are derived from.
    fn node0_pre_seal_journal_full_sealed(
        seed: u64,
    ) -> (Set<PeerPubkey>, Ed25519PrivateKey, Vec<JournalRecord>) {
        let ns = b"FLUENT_DPOS_V1_clocktest";
        let mut rng = StdRng::seed_from_u64(seed);
        let keys: Vec<Ed25519PrivateKey> =
            (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let pk0 = keys[0].public_key();
        let mut cers: BTreeMap<PeerPubkey, DkgCeremony> = BTreeMap::new();
        let mut queue: Vec<(PeerPubkey, Outgoing)> = Vec::new();
        let mut journal0: Vec<JournalRecord> = Vec::new();
        for k in &keys {
            let (cer, step) =
                DkgCeremony::start(&mut rng, ns, 2, committee.clone(), k.clone()).expect("start");
            let from = k.public_key();
            if from == pk0 {
                journal0.extend(step.journal);
            }
            queue.extend(step.outgoing.into_iter().map(|o| (from.clone(), o)));
            cers.insert(from, cer);
        }
        let drive = |cers: &mut BTreeMap<PeerPubkey, DkgCeremony>,
                     queue: &mut Vec<(PeerPubkey, Outgoing)>,
                     journal0: &mut Vec<JournalRecord>| {
            while let Some((from, o)) = queue.pop() {
                match o.target {
                    Target::Broadcast => {
                        let tos: Vec<PeerPubkey> =
                            cers.keys().filter(|p| **p != from).cloned().collect();
                        for to in tos {
                            let more =
                                cers.get_mut(&to).unwrap().handle(from.clone(), o.msg.body.clone());
                            if to == pk0 {
                                journal0.extend(more.journal);
                            }
                            queue.extend(more.outgoing.into_iter().map(|m| (to.clone(), m)));
                        }
                    }
                    Target::Direct(to) => {
                        if let Some(c) = cers.get_mut(&to) {
                            let more = c.handle(from.clone(), o.msg.body.clone());
                            if to == pk0 {
                                journal0.extend(more.journal);
                            }
                            queue.extend(more.outgoing.into_iter().map(|m| (to.clone(), m)));
                        }
                    }
                }
            }
        };
        drive(&mut cers, &mut queue, &mut journal0);
        for k in &keys {
            let step = cers.get_mut(&k.public_key()).unwrap().seal_dealings();
            if k.public_key() == pk0 {
                journal0.extend(step.journal);
            }
            queue.extend(step.outgoing.into_iter().map(|o| (k.public_key(), o)));
        }
        drive(&mut cers, &mut queue, &mut journal0);
        (committee, keys[0].clone(), journal0)
    }

    /// The epoch-2 ceremony `Info` over `committee` — to re-`check` a journaled
    /// `SignedDealerLog`'s dealer in the test helpers.
    fn info_for_test(
        committee: &Set<PeerPubkey>,
    ) -> commonware_cryptography::bls12381::dkg::Info<
        commonware_cryptography::bls12381::primitives::variant::MinSig,
        PeerPubkey,
    > {
        use commonware_cryptography::bls12381::{dkg::Info, primitives::sharing::Mode};
        use commonware_utils::N3f1;
        Info::<_, PeerPubkey>::new::<N3f1>(
            b"FLUENT_DPOS_V1_clocktest",
            DETERMINISTIC_BOOTSTRAP_EPOCH,
            None,
            Mode::NonZeroCounter,
            committee.clone(),
            committee.clone(),
        )
        .expect("info")
    }
}
