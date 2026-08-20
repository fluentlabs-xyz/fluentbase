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
//!   [`CeremonyStore`] + fire `share_notify`. The epoch manager's share-gate
//!   reads the entry to decide whether this node may run `E`'s engine, and
//!   Phase 5's finalized-boundary swap reads both for the per-epoch signing
//!   slot + `commitEpochBeaconKey`.
//!
//! The actor never finalizes over a locally-selected Q before sealing, and never
//! over an under-quorum log set (`ready` gates it). <quorum valid logs → no store
//! entry → the beacon naturally stalls for that epoch (option A), not a crash.
//!
//! Mid-window restart durability (§8.11.1): ceremony progress is journaled to
//! `beacon-dkgjournal-e<E>.bin`; on restart `maybe_start` RESUMES via
//! `DkgCeremony::resume` — a PRE-seal restart (before the seal deadline) RE-DERIVES the
//! SEEDED dealer (`dealer_seed_rng`, byte-identical commitment) and keeps distributing;
//! an at/after-deadline restart is player-only + never re-seals — and re-fetches missing
//! peer logs via the DKG-log recovery resolver (`fetch_missing_logs`/`on_resolver_message`,
//! the `commonware_resolver::p2p` engine on `BEACON_RESOLVER_CHANNEL`), so a routine
//! restart no longer leaves the member shareless + liveness-slashed.

use crate::beacon::{
    artifact::encode_artifact,
    ceremony::{
        checked_serve_map, recompute_scoped, CeremonyOutput, DkgCeremony, Outgoing, Step, Target,
    },
    dkg_agree::{
        AgreedArtifact, AgreementHold, AgreementTargets, ConfirmPool, PinnedDerive, PinnedLogs,
        ShareConfirm,
    },
    dkg_msg::{DealerReveal, DkgBody, DkgMsg},
    log_resolver::{DkgLogKey, LogMessage},
    outcome::{validate_share_on_poly, DkgOutcome},
    share_state::{self, JournalLoad, JournalRecord, ShareState},
    wire::BeaconMessage,
};
use crate::{epocher::OriginEpocher, outer::SCHEME_RETENTION_EPOCHS};
use alloy_primitives::B256;
use bytes::Bytes;
use commonware_codec::{Encode as _, Read as _, ReadExt as _};
use commonware_consensus::types::{Epoch, Epocher as _, Height};
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
    future::Future,
    num::{NonZeroU32, NonZeroU64},
    path::PathBuf,
    pin::Pin,
    sync::{Arc, RwLock},
};

/// Blocks of slack before the epoch-E boundary at which dealing collection closes
/// and dealers seal (broadcast their signed logs) — the echo-settle tail. Pinned
/// off the on-chain `epochBlockInterval`, not an absolute window (see Q4).
///
/// This is the WHOLE budget the epoch key has: the value to be agreed does not
/// exist anywhere in the network before the seal, so `B − MARGIN` is the earliest
/// instant the agreement plane can start and `B` is when the boundary block needs
/// the key. 20 blocks at the 1 blk/s target is 20 s.
///
/// It stays 20 on a measurement rather than on comfort. The first live run
/// (2026-08-19, docker stand, `case growth`, two committee changes at
/// `epochBlockInterval = 32`) measured `T_agree` = 30.07 s and 30.09 s — exactly
/// one `LEADER_TIMEOUT`, with `view = 2` both times — which overran this window by
/// ~11 s and cost a `verify-only` demotion at each boundary. The cause was a
/// leader that evaluated its proposal once and parked for the rest of its view,
/// NOT a window that was too narrow: the inputs land in milliseconds on a LAN, and
/// every member agreed a byte-identical set within a 1-3 ms span once one leader
/// proposed. `DkgAgree::build_proposal` now re-reads on the growth edge, which puts
/// a view-1 decision at roughly three network delays and leaves this window with
/// more than an order of magnitude of headroom. Widening it instead would have
/// hidden the 30 s and slowed every epoch-waiting smoke case by the same factor.
///
/// So: do NOT raise this to buy time for the agreement without first re-measuring
/// `T_agree` — a raise that is not answering a measured overrun is buying nothing,
/// and it costs a proportionally longer `epochBlockInterval` everywhere. The one
/// case that still overruns is a view-1 leader that is down; that pays one
/// `LEADER_TIMEOUT` and lands in the measured ~11 s verify-only window, which
/// recovers on its own and is the accepted BFT residual, not a halt.
///
/// v41 shifted 10→16 (circular-finality fix); AMENDMENT 5 shifted 16→20 so the
/// dealer-log hashes — which then rode the OrderBlock and had to finalize by
/// `K`-lag — had an includable window. That carrier is gone (the field left
/// `OrderBlock`, and the set is agreed off-chain), so 20 now stands only on the
/// budget above.
///
/// Requires `epochBlockInterval > DKG_MARGIN_BLOCKS` for a positive deal window
/// (devnet `I=32` ⇒ deal window `I−20 = 12`; the production target is ~1200).
pub(crate) const DKG_MARGIN_BLOCKS: u64 = 20;

/// Blocks AFTER the seal deadline to keep collecting peer logs before finalizing
/// over whatever valid set has settled — the deterministic fallback when a dealer is
/// genuinely absent (the all-present case finalizes earlier, the instant the last
/// `Reveal` lands). Bounds the canonical-set wait so every honest node selects over
/// the IDENTICAL log set at the identical height ⇒ identical `PK_E` ⇒ the C gate
/// passes. MUST stay `< DKG_MARGIN_BLOCKS` so finalize lands before the boundary.
///
/// The 4→8 shift dates from AMENDMENT 5, when the finalize input was the
/// finalized dealer-log HASH set carried by blocks, so a hash sealed at `B−20`
/// had to be INCLUDED in a block AND that block FINALIZED (`K`-lag) by
/// `H_settle = B−(MARGIN−SETTLE) = B−12`. That carrier is gone: the set is agreed
/// off-chain, a certified artifact settles its epoch the moment it lands, and this
/// deadline now governs ONLY the legacy no-plane arm of `drive_finalization`
/// (in-process and test wiring). It stays 8 because nothing measures it any more —
/// with a plane wired the finalize path never reaches it, so retuning it would be
/// tuning a number no live run exercises. `H_settle = B−12` is unchanged from the
/// pre-AM5 schedule (`MARGIN−SETTLE = 20−8 = 12`).
pub(crate) const DKG_SETTLE_BLOCKS: u64 = 8;
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

/// Trailing epochs past its own boundary for which a finalized/stalled epoch's DKG
/// journal (own `ReceivedDealing` views AND the shared QUAL logs) + `serve_cache`
/// are RETAINED — the recompute-heal window (§8.11.1). A demoted `committee[E]`
/// member (or a peer it serves) recomputes E's share from these while E is still
/// committee-relevant, instead of being swept the instant `now == E` and lingering a
/// verify-only observer until the next committee change.
///
/// Default `1` = "current epoch + 1 trailing". This is the BELTED default, NOT
/// "already sufficient": for the warm-member trigger (already caught up, mesh flapped)
/// the heal completes within 1 window; for the cold-sync refill/promote triggers the
/// heal DEPENDS on catch-up (EL sync + mesh reconnect + log refetch) finishing before
/// the target epoch's journal ages out of this window — those are ALSO fronted by the
/// harness warm-gate. Derivation for a wider window:
/// `JOURNAL_RETENTION_EPOCHS ≥ ceil(worst_case_catchup_seconds / epoch_seconds) + 1`,
/// `epoch_seconds ≈ EPOCH_INTERVAL × 1 s` (1 blk/s). Operational monitor:
/// `epoch_engine_demoted_no_polynomial` persisting > `JOURNAL_RETENTION_EPOCHS` epochs
/// for one identity = a demote whose logs aged out before catch-up → widen the window.
/// Size cost ≈ window × (~430 KiB QUAL set at n=51 + the per-dealer secret view bodies)
/// per retained epoch. Under-retention is SAFE: a member that finds the logs evicted
/// simply keeps fetching / stays a verify-only observer — it never adopts a wrong share
/// (the recompute self-check gates that), so widening only trades disk for heal reach.
pub(crate) const JOURNAL_RETENTION_EPOCHS: u64 = 1;

/// Reads the agreed per-epoch DKG `Output` for an EPOCH out of the agreement plane's
/// artifact store, threaded into the actor as a READ handle (NOT a cross-actor push
/// channel). Returns `Some(outcome)` only where an artifact for that exact epoch is
/// held — i.e. a CHANGE epoch whose agreement this node has the certified result of;
/// `None` for a carry-forward epoch (no fresh DKG ⇒ never a demote to recompute) OR a
/// store miss (retry next tick). `None` (no reader wired, the in-process/test default)
/// makes the recompute-heal inert — the resolver/gossip paths still run.
///
/// It used to read the boundary block at `epoch_start(E)`, which was a chicken-and-egg:
/// the heal exists for a member that could not enter `E`, and `E`'s own first block is
/// exactly what such a member's epoch does not produce. An epoch-keyed artifact,
/// certified before `E` starts, dissolves that.
pub type AgreedOutcomeAt =
    Arc<dyn Fn(u64) -> Pin<Box<dyn Future<Output = Option<DkgOutcome>> + Send>> + Send + Sync>;

/// Per-epoch state of an in-flight demote-heal recompute: the pinned `Output` (read
/// once from the boundary block — supplies the `dealers()` scope AND the
/// `validate_share_on_poly` self-check target) and the pinned `dealers()` logs this
/// node still needs to fetch (`want`, drained as the resolver delivers them). Bounded:
/// inserted ONLY for a demoted `committee[E]` member-epoch within the retention window,
/// removed on recompute-success or age-out.
struct RecomputeState {
    outcome: DkgOutcome,
    want: BTreeSet<PeerPubkey>,
}

/// How `drive_finalization` will finalize a ready ceremony: over the LEGACY local
/// settled set (no agreement plane wired — the in-process/test default) or over the
/// AGREED dealer-log set from the plane's artifact (committee + `idx→hash`).
enum FinalizePlan {
    Legacy,
    Pinned(Set<PeerPubkey>, BTreeMap<u8, B256>),
}

/// Test-only counter of cold-cache journal parses, so the fetch-burst-bound test can
/// assert "one parse per epoch, not per request" (the DoS-is-one-shot property Option C
/// relies on). Incremented in `cold_load_serve_cache` on each disk parse.
#[cfg(test)]
pub(crate) static COLD_PARSE_COUNT: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// The agreed DKG result for an epoch this node is a MEMBER of: the group output
/// (`PK_E` + public polynomial) and this node's secret share, memoized by the
/// actor during the post-seal margin window — BEFORE the epoch-E boundary block
/// is proposed/verified. Read by the per-epoch engine's signing material resolver
/// and by Phase 5's signing-slot swap at the finalized boundary. Non-members never
/// get an entry (⇒ observer ⇒ withhold).
pub type CeremonyStore = Arc<RwLock<BTreeMap<u64, (CeremonyOutput, Share)>>>;

/// Dealer-log hash index: `epoch → (committee idx → content hash of the recorded
/// `SignedDealerLog`)`. Written by the [`DkgActor`] as it records a valid dealer
/// log; read by the epoch-key agreement plane, which proposes over it, and by the
/// share-confirmations that state it.
///
/// `Arc`-shared like [`CeremonyStore`]; `idx` is the position in the agreed
/// on-chain `committee[epoch]` (`u8`, `n ≤ MAX_COMMITTEE_SIZE`).
pub type DkgLogIndex = Arc<RwLock<BTreeMap<u64, BTreeMap<u8, B256>>>>;

/// The retain floor for the SHARED insert-only [`CeremonyStore`]: the greatest mint
/// epoch still `<= now - window` — i.e. the mint IN FORCE for the OLDEST cert inside
/// the scheme-retention window. Every entry with key `>= floor` must be kept: a reader
/// resolves `PK_E` via `store.read().range(..=E).next_back()` (`dpos.rs`), so an
/// older-but-still-in-force mint on a STABLE committee is load-bearing and a plain
/// `pop_first`/size-cap would demote a legitimate signer. Entries strictly below the
/// floor are superseded mints no cert in the window can select. Returns `0` (retain
/// everything) when no mint is old enough to be a floor — so a stable committee (its
/// lone mint is never `<= now - window` until it ages fully out) prunes nothing, while
/// churn bounds growth to the trailing window.
fn ceremony_retain_floor(keys: impl Iterator<Item = u64>, now: u64, window: u64) -> u64 {
    let cutoff = now.saturating_sub(window);
    keys.filter(|&k| k <= cutoff).max().unwrap_or(0)
}

/// Resolves committee[epoch] (the Commonware-ordered peer set) at a finalized
/// state hash — provided by the launch site over the staking reader.
pub type CommitteeFor = Arc<dyn Fn(u64) -> Option<Set<PeerPubkey>> + Send + Sync>;

/// `recv()` on an optional mpsc receiver, or park forever when it is `None` — so the
/// resolver and agreement branches of the actor's `select!` are inert on a node with
/// neither wired (in-process / test default) without a second loop shape.
async fn recv_or_never<T>(rx: Option<&mut tokio::sync::mpsc::Receiver<T>>) -> Option<T> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

/// One question from the epoch-key agreement instance: does a candidate pinned
/// dealer-log set yield a group key here, and if not, why not.
///
/// The bodies the answer needs are owned single-threaded by [`DkgActor`], so the
/// question crosses a channel exactly like [`LogMessage`] does. `epoch` rides the
/// request rather than being implied by the channel: one actor serves the
/// agreement instances of every target epoch it is dealing for.
pub struct PinnedRequest {
    pub epoch: u64,
    pub pinned: BTreeMap<u8, B256>,
    pub(crate) response: tokio::sync::oneshot::Sender<PinnedDerive>,
}

/// The agreement instance's handle on the ceremony bodies [`DkgActor`] owns — the
/// production [`PinnedLogs`] implementor, bound to one target epoch because one
/// agreement instance agrees one epoch's set.
///
/// Every way this handle can fail to get an answer — the actor gone, the reply
/// dropped — is [`PinnedDerive::Unavailable`], never [`PinnedDerive::Unusable`]:
/// those are properties of THIS node, and `Unusable` is the one arm the agreement
/// may turn into a nullified view for the whole network.
#[derive(Clone)]
pub(crate) struct PinnedMailbox {
    target_epoch: u64,
    requests: tokio::sync::mpsc::Sender<PinnedRequest>,
}

impl PinnedMailbox {
    pub const fn new(
        target_epoch: u64,
        requests: tokio::sync::mpsc::Sender<PinnedRequest>,
    ) -> Self {
        Self {
            target_epoch,
            requests,
        }
    }
}

impl PinnedLogs for PinnedMailbox {
    async fn derive(&self, pinned: BTreeMap<u8, B256>) -> PinnedDerive {
        let (response, reply) = tokio::sync::oneshot::channel();
        if self
            .requests
            .send(PinnedRequest {
                epoch: self.target_epoch,
                pinned,
                response,
            })
            .await
            .is_err()
        {
            return PinnedDerive::Unavailable;
        }
        reply.await.unwrap_or(PinnedDerive::Unavailable)
    }
}

/// The two start-race dealings a single sender can contribute, latest-wins. Only
/// `Commitment`/`Share` are ever bufferable (`is_bufferable`), so two `Option`s
/// cover a sender exactly.
#[derive(Default)]
struct PendingDealings {
    commitment: Option<DkgBody>,
    share: Option<DkgBody>,
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
    /// The DEALING/QUAL/SERVE roster reader — resolves `committee[epoch]` as the
    /// CEREMONY participants (in production the committed-slot reader, since under the
    /// 2-epoch warm-up `committee[epoch]` is frozen a full epoch before its DKG runs):
    /// who deals, whose qual partials are roster-bound, whose logs are served/
    /// recomputed, and the `next` committee in `maybe_start`.
    committee_for: CommitteeFor,
    /// The `cur`-side roster reader for `maybe_start`'s change-test
    /// (`committee(target−1)` vs `committee(target)`). Defaults to `committee_for`
    /// (the same committed slot in production — both sides read the immutable
    /// committed committee); a test may override it via `with_active_committee`
    /// to exercise a divergent-reader change-detect.
    active_committee_for: CommitteeFor,
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
    /// geometry is frozen (see `build_beacon_plane`). Held as the ONE epoch↔height
    /// authority rather than as two raw numbers, so this actor's deal/seal
    /// schedule cannot drift from every other epoch→height computation.
    epocher: OriginEpocher,
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
    /// `(epoch, reason)` pairs already reported for a past-deadline finalize deferral,
    /// so the warn + counter fire ONCE per epoch per reason instead of on every height
    /// tick. Cleared with the ceremony at the boundary sweep.
    deferred_reported: BTreeSet<(u64, &'static str)>,
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
    /// bufferable (`is_bufferable`) and stale epochs are evicted each height tick.
    /// PER-SENDER bounded (Rule R, N2): one Byzantine peer occupies at most its own
    /// slot (≤1 Commitment + ≤1 Share, latest-wins), so it cannot evict honest
    /// dealings from the shared per-epoch buffer. Size-bounded by the committee
    /// roster (the only authenticated senders on `BEACON_CHANNEL`).
    pending: BTreeMap<u64, BTreeMap<PeerPubkey, PendingDealings>>,
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
    /// READ handle for the agreed epoch outcome (a pull, not a push channel), used by
    /// the actor's OWN `on_height` self-detect of a demoted `committee[E]` member to
    /// scope + self-check a share recompute. `None` ⇒ no reader wired
    /// (in-process/test default) ⇒ the recompute-heal is inert.
    outcome_at: Option<AgreedOutcomeAt>,
    /// In-flight demote-heal recomputes, keyed by the demoted committee epoch E. The
    /// actor SELF-DETECTS the demote (`me ∈ committee[E]` ∧ `store` lacks `E` ∧ an
    /// agreed outcome exists for `E`) from its `on_height` loop — no
    /// `epoch_manager`→actor signal — and drives the resolver to fetch the pinned
    /// `dealers()` logs, then recomputes + self-checks + adopts the share (firing
    /// `share_notify` so the existing promote edge re-runs). Bounded to the retention
    /// window; entries removed on success or age-out. Also RETAINS its epoch's journal
    /// (the sweep keeps a journal while its epoch is a `recompute_pending` key).
    recompute_pending: BTreeMap<u64, RecomputeState>,
    /// The dealer-log hash index this actor PUBLISHES (idx→hash of each recorded
    /// log) for the agreement plane to propose over and for share-confirmations to
    /// state. `None` ⇒ unwired (in-process/test default). Wired at the beacon-plane
    /// spawn site alongside the shared `CeremonyStore`.
    recorded_dkg_logs: Option<DkgLogIndex>,
    /// The share-confirmation pool the epoch-key agreement's entry bar counts:
    /// written here (this node's own confirmations and every peer's that verifies),
    /// read by the agreement instance's `propose`. Left unset the actor neither
    /// mints nor accepts confirmations and the branch is inert, exactly like
    /// `recorded_dkg_logs`. Wired at the beacon-plane spawn site, which hands the
    /// SAME pool to every agreement instance — the namespace confirmations are
    /// signed under lives in it.
    share_confirms: Option<ConfirmPool>,
    /// `target epoch -> the size of the body-checked set this node last confirmed`.
    /// A confirmation is re-minted only when that set GROWS, which is a faithful
    /// change detector because `record_checked_log` is first-wins and irreversible,
    /// so the set never shrinks and never changes an entry in place.
    confirmed_len: BTreeMap<u64, usize>,
    /// Inbound pinned-set questions from the epoch-key agreement instances
    /// ([`PinnedMailbox`]). `None` ⇒ no agreement plane wired (in-process/test
    /// default) ⇒ the branch parks forever and nothing asks.
    pinned_rx: Option<tokio::sync::mpsc::Receiver<PinnedRequest>>,
    /// Target epochs with a LIVE agreement instance, exempt from the height-driven
    /// ceremony/log sweep below. Empty ⇒ no agreement plane wired ⇒ the sweep is
    /// exactly the height-driven one it has always been.
    agreement_targets: AgreementTargets,
    /// Announcement sink for the epoch-key agreement plane's spawn edge: a target
    /// epoch whose ceremony has CLOSED ITS DEALING here, which is the earliest
    /// point at which this node has a dealer-log set worth agreeing. The plane
    /// (node crate) owns the sub-channel registration and the instance itself;
    /// this actor owns the only state that knows when the edge happened.
    ///
    /// Not the `seal_dealings` call specifically: a node that restarted at or
    /// after the seal deadline resumes PLAYER-ONLY and never seals, and it still
    /// has to run the agreement for the epoch. `dealing_closed()` covers both.
    agreement_tx: Option<tokio::sync::mpsc::Sender<u64>>,
    /// Targets already announced on `agreement_tx`, so the per-tick re-probe does
    /// not re-announce. Only a target the sink ACCEPTED is recorded, so a full
    /// channel is retried on the next tick rather than silently dropped.
    agreement_announced: BTreeSet<u64>,
    /// Agreed artifacts arriving from the plane — this node's own instance, or a
    /// peer's artifact that already verified against `committee[epoch]`.
    ///
    /// PRECONDITION: every artifact on this channel has been checked against the
    /// target epoch's committee. Both producers do it (the instance only ever
    /// delivers a value its own quorum certified; the pull seam verifies before
    /// it stores), and this actor cannot re-check it — it reads peer identities,
    /// never the BLS committee the certificate is verified under.
    artifacts_rx: Option<tokio::sync::mpsc::Receiver<AgreedArtifact>>,
    /// Pinned dealer-log sets taken from an agreed artifact, keyed by target
    /// epoch — the ONE source [`Self::drive_finalization`] runs
    /// `finalize_over_pinned` over. It needs no settle deadline: the deadline
    /// exists only to make every honest node select over an identical set, which a
    /// quorum certificate states outright.
    ///
    /// The [`AgreementHold`] rides along because the write-back is not
    /// instantaneous — missing bodies are fetched first — and the height-driven
    /// sweep would otherwise drop the very ceremony the finalize runs on the
    /// moment the chain enters the target epoch.
    agreed_pinned: BTreeMap<u64, AgreedSet>,
}

/// One artifact-sourced pinned set, and the ceremony-retention hold that keeps it
/// usable. See [`DkgActor::agreed_pinned`].
struct AgreedSet {
    pinned: BTreeMap<u8, B256>,
    /// The encoded artifact this set came out of, kept so the finalize can write it
    /// down BESIDE the share it produced. A node whose share file carries the
    /// artifact restarts holding the value its instance agreed instead of having to
    /// go looking for it again.
    encoded_artifact: Vec<u8>,
    _hold: AgreementHold,
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
        outcome_at: Option<AgreedOutcomeAt>,
    ) -> Self {
        Self {
            namespace,
            me_key,
            sender,
            receiver,
            resolver,
            resolver_rx,
            active_committee_for: committee_for.clone(),
            committee_for,
            store,
            share_notify,
            // Fail LOUD and at construction on a zero interval, which the previous
            // raw-field form turned into a div-by-zero at the first `epoch_of`.
            // Every production caller already `ensure!`s it non-zero.
            epocher: OriginEpocher::new(
                dpos_activation,
                NonZeroU64::new(epoch_interval).expect("epochBlockInterval must be > 0"),
            ),
            metrics,
            share_dir,
            share_state,
            ceremonies: BTreeMap::new(),
            deferred_reported: BTreeSet::new(),
            serve_cache: BTreeMap::new(),
            reconciled_journals: false,
            pending: BTreeMap::new(),
            last_height: 0,
            eval_logged: BTreeSet::new(),
            torn_warned: BTreeSet::new(),
            outcome_at,
            recompute_pending: BTreeMap::new(),
            recorded_dkg_logs: None,
            share_confirms: None,
            confirmed_len: BTreeMap::new(),
            pinned_rx: None,
            agreement_targets: AgreementTargets::default(),
            agreement_tx: None,
            agreement_announced: BTreeSet::new(),
            artifacts_rx: None,
            agreed_pinned: BTreeMap::new(),
        }
    }

    /// Serve the epoch-key agreement plane's pinned-set questions off this actor's
    /// ceremony state. Left unset nothing asks and the branch is inert; wired at the
    /// beacon-plane spawn site with the sending half handed to each agreement
    /// instance as a [`PinnedMailbox`].
    ///
    /// `targets` arrives with the receiver and not on a method of its own,
    /// deliberately: serving the questions without exempting their epochs from the
    /// height-driven sweep is the shape in which the plane answers `Unavailable`
    /// forever the moment the chain reaches the target, and nothing in the wiring
    /// would say so. The same registry must be given to every instance
    /// ([`crate::beacon::dkg_engine::AgreementConfig::targets`]).
    pub fn with_pinned_requests(
        mut self,
        pinned_rx: tokio::sync::mpsc::Receiver<PinnedRequest>,
        targets: AgreementTargets,
    ) -> Self {
        self.pinned_rx = Some(pinned_rx);
        self.agreement_targets = targets;
        self
    }

    /// Join the epoch-key agreement plane: announce every target whose dealing has
    /// closed on `agreement_tx`, and take agreed artifacts back on `artifacts_rx`.
    ///
    /// The two halves arrive together because neither is useful alone. Announcing
    /// without an intake starts instances whose output nothing consumes; an intake
    /// without an announcement waits for artifacts no instance was ever started to
    /// produce. Left unwired both branches are inert and this actor behaves
    /// exactly as it did before the plane existed.
    ///
    /// Requires [`Self::with_pinned_requests`] to be wired with the SAME
    /// [`AgreementTargets`]: the write-back takes its own hold on that registry to
    /// keep a ceremony alive across the fetch-then-finalize the artifact starts.
    pub fn with_agreement_plane(
        mut self,
        agreement_tx: tokio::sync::mpsc::Sender<u64>,
        artifacts_rx: tokio::sync::mpsc::Receiver<AgreedArtifact>,
    ) -> Self {
        self.agreement_tx = Some(agreement_tx);
        self.artifacts_rx = Some(artifacts_rx);
        self
    }

    /// Publish each live ceremony's body-checked dealer-log hashes into a shared
    /// `epoch -> idx -> keccak256(SignedDealerLog)` index.
    ///
    /// It used to feed the consensus propose path, which carried the set in
    /// `OrderBlock.dkg_logs`; the block no longer carries it and the index's
    /// readers are now local — the agreement plane proposes from it, and
    /// share-confirmations state it. Wired at the beacon-plane spawn site (node
    /// crate) alongside the shared `CeremonyStore`.
    pub fn with_recorded_logs(mut self, recorded: DkgLogIndex) -> Self {
        self.recorded_dkg_logs = Some(recorded);
        self
    }

    /// Enable share-confirmations: this node mints and gossips its own for every
    /// target epoch whose body-checked dealer-log set grows, and records every peer
    /// confirmation that verifies against the epoch's committee.
    ///
    /// The pool carries the signing namespace, so the actor and the agreement
    /// instances cannot disagree about it — hand BOTH the same pool. Requires
    /// [`Self::with_recorded_logs`] to be wired too: the confirmed set is read from
    /// `recorded_dkg_logs`, which is the same source the agreement proposes from, so
    /// a confirmation can never name a set the proposal path would not.
    pub fn with_share_confirms(mut self, confirms: ConfirmPool) -> Self {
        self.share_confirms = Some(confirms);
        self
    }

    /// Override the `cur`-side roster reader (the `cur` side of `maybe_start`'s
    /// change-test) with a distinct reader. Left unset it defaults to `committee_for`
    /// — which is what production uses (both sides read the same committed slot under
    /// the 2-epoch warm-up). A test uses this to inject a divergent `cur` reader and
    /// exercise change-detect (see [`Self::active_committee_for`]).
    #[cfg(test)]
    pub fn with_active_committee(mut self, active_committee_for: CommitteeFor) -> Self {
        self.active_committee_for = active_committee_for;
        self
    }

    /// Pre-activation heights have no relative epoch; they answer 0, as the
    /// previous saturating form did.
    fn epoch_of(&self, height: u64) -> u64 {
        self.epocher
            .containing(Height::new(height))
            .map_or(0, |info| info.epoch().get())
    }

    /// First-block height of an epoch (relative to DPoS activation). `u64::MAX`
    /// only on `epoch * interval` overflowing u64 — unreachable at any real epoch,
    /// and every reader here compares a real height against it or subtracts a
    /// margin from it, both of which degrade to "not yet" rather than misfiring.
    fn epoch_start(&self, epoch: u64) -> u64 {
        self.epocher
            .first(Epoch::new(epoch))
            .map_or(u64::MAX, |h| h.get())
    }

    /// Append the ceremony's journal records for `epoch` so a restart can `resume` AND
    /// a post-boundary member can recompute its share (§8.11.1). Each record is
    /// fsync-durable on a successful return (`share_state::append_journal` calls
    /// `sync_all`). Returns whether EVERY record was written DURABLY — the write-durably-
    /// before-ack gate (step 1f) uses this: an `Ack` paired with a `ReceivedDealing`
    /// record is broadcast ONLY when that record is durable, so a QUAL log can never
    /// record an ack this node cannot back with a durable view (which recompute would
    /// hit as an un-resurrectable `MissingPlayerDealing`). No `share_dir` (in-process/
    /// test default) ⇒ `true`: there is no on-disk journal and thus no cross-restart
    /// recompute for this node, so the in-memory view is authoritative and acking is
    /// safe. A write failure warns and returns `false` (the caller withholds the ack).
    #[must_use]
    fn append_journal(&self, epoch: u64, records: Vec<JournalRecord>) -> bool {
        let Some(dir) = &self.share_dir else {
            return true;
        };
        let mut durable = true;
        for record in records {
            if let Err(err) = share_state::append_journal(dir, epoch, &record, &self.share_state) {
                tracing::warn!(
                    epoch,
                    ?err,
                    "live DKG: failed to journal ceremony record (in-memory ceremony \
                     unaffected; dependent ack withheld — the dealer reveals our point)"
                );
                durable = false;
            }
        }
        durable
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
    pub async fn run(
        mut self,
        mut heights: tokio::sync::mpsc::Receiver<u64>,
        mut rng: impl CryptoRngCore,
    ) {
        tracing::info!(epocher = ?self.epocher, "live DKG: actor started");
        // The resolver inbound channel is taken out so the loop can borrow it
        // alongside `&mut self`; `None` (no resolver wired) parks the branch forever.
        let mut resolver_rx = self.resolver_rx.take();
        // Same reason, same shape: taken out so the agreement branch can borrow it
        // alongside `&mut self`.
        let mut pinned_rx = self.pinned_rx.take();
        // Same reason again: the artifact arm borrows the local receiver alongside
        // `&mut self`.
        let mut artifacts_rx = self.artifacts_rx.take();
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
                // Answer an epoch-key agreement instance's question about a
                // candidate pinned set. Parked forever when no agreement plane is
                // wired; a closed channel means every instance is gone, so the
                // branch parks rather than spinning on `None`.
                req = recv_or_never(pinned_rx.as_mut()) => match req {
                    Some(req) => {
                        let verdict = self.derive_pinned(&req, &mut rng);
                        drop(req.response.send(verdict));
                    }
                    None => pinned_rx = None,
                },
                // The write-back edge. It is the ONE arm that does not depend on
                // the finalized-height stream, which is the whole point: with the
                // chain halted at `epoch_start(E+1)` the height clock stops, and
                // the artifact is what starts the epoch's key moving again.
                artifact = recv_or_never(artifacts_rx.as_mut()) => match artifact {
                    Some(artifact) => self.on_artifact(artifact, &mut rng).await,
                    None => artifacts_rx = None,
                },
            }
        }
    }

    /// What this node can say about a candidate pinned dealer-log set for
    /// `req.epoch` — the production side of [`PinnedLogs`].
    ///
    /// IMPLEMENTOR CONTRACT, and it is the safety-critical part: an unreadable
    /// roster and an absent ceremony are properties of THIS node, so both answer
    /// [`PinnedDerive::Unavailable`] and the caller parks. Only
    /// [`PinnedDerive::Unusable`] — every named body held, and still no key —
    /// may become a `verify → false`, because only that is a property of the set
    /// every honest node reaches alike.
    ///
    /// A ceremony that already finalized has been consumed and removed, so this
    /// answers `Unavailable` from then on. That is the contract-correct answer and
    /// costs nothing: a node that finalized has the key it was voting to agree.
    fn derive_pinned(&self, req: &PinnedRequest, rng: &mut impl CryptoRngCore) -> PinnedDerive {
        let Some(committee) = (self.committee_for)(req.epoch) else {
            return PinnedDerive::Unavailable;
        };
        let Some(ceremony) = self.ceremonies.get(&req.epoch) else {
            return PinnedDerive::Unavailable;
        };
        ceremony.derive_pinned(rng, &committee, &req.pinned)
    }

    /// Take an agreed artifact as this epoch's pinned dealer-log set and drive the
    /// write-back to the point where the existing rails carry it.
    ///
    /// Everything downstream of the pinned set is reused verbatim:
    /// `finalize_over_pinned` → `CeremonyStore` insert → `share_notify` → the
    /// epoch manager's respawn edge. The artifact is the ONLY source of the set,
    /// so there is no second finalize path and no second store writer.
    ///
    /// Two things happen here that `on_height` would otherwise have to: the
    /// finalize is driven immediately, and the missing pinned bodies are fetched
    /// immediately. With the chain halted neither would ever run again.
    async fn on_artifact(&mut self, artifact: AgreedArtifact, rng: &mut impl CryptoRngCore) {
        let epoch = artifact.0.target_epoch;
        // Claimed FIRST, so every early return below releases it. A local instance
        // parks its ceremony-retention hold on delivery rather than letting it die
        // with its own future — its `send` returns two hops short of here — and an
        // artifact that arrived any other way (a peer pull, a restart replay)
        // parked nothing and needs a hold of its own.
        let hold = self
            .agreement_targets
            .take_handover(epoch)
            .unwrap_or_else(|| self.agreement_targets.hold(epoch));
        // Already holding this epoch's share: the ceremony is consumed, there is
        // nothing left for the artifact to unblock, and adopting the set would
        // only take a retention hold nothing would release.
        if self
            .store
            .read()
            .ok()
            .is_some_and(|m| m.contains_key(&epoch))
        {
            return;
        }
        let pinned: BTreeMap<u8, B256> = artifact.0.logs.iter().copied().collect();
        if pinned.is_empty() {
            tracing::warn!(
                target: "dpos::beacon",
                epoch,
                "live DKG: an agreed artifact names no dealer logs; nothing to finalize over"
            );
            return;
        }
        // FIRST-WINS, for the same reason `ArtifactStore::insert` is: one
        // instance certifies exactly one value per target epoch, so a second
        // artifact is either the identical set or a divergence this actor cannot
        // adjudicate — and re-pinning would swap the set out from under a
        // finalize that is already fetching bodies for it.
        if self.agreed_pinned.contains_key(&epoch) {
            return;
        }
        tracing::info!(
            target: "dpos::beacon",
            epoch,
            pinned = pinned.len(),
            height = self.last_height,
            "live DKG: adopting the agreed dealer-log set as this epoch's pinned set"
        );
        self.agreed_pinned.insert(
            epoch,
            AgreedSet {
                pinned,
                encoded_artifact: encode_artifact(&artifact),
                _hold: hold,
            },
        );
        self.drive_finalization(self.last_height, rng);
        self.fetch_missing_logs(self.last_height).await;
    }

    /// Announce every target whose dealing has closed to the agreement plane.
    ///
    /// Repeated on every tick rather than fired once, and the plane deduplicates.
    /// A one-shot announcement would be lost outright whenever the plane cannot act
    /// on it yet — an unreadable `committee[epoch]`, a sub-channel registration
    /// that lost a race — and the target would then never get an instance at all.
    /// The log line is the part that is one-shot.
    async fn announce_agreement_targets(&mut self) {
        let Some(tx) = self.agreement_tx.as_ref() else {
            return;
        };
        let due: Vec<u64> = self
            .ceremonies
            .iter()
            .filter(|(_, c)| c.dealing_closed())
            .map(|(e, _)| *e)
            .collect();
        for epoch in due {
            match tx.try_send(epoch) {
                Ok(()) => {
                    if self.agreement_announced.insert(epoch) {
                        tracing::info!(
                            target: "dpos::beacon",
                            epoch,
                            "live DKG: dealing closed — asking the plane for an epoch-key \
                             agreement instance"
                        );
                    }
                }
                Err(err) => tracing::debug!(
                    target: "dpos::beacon",
                    epoch,
                    ?err,
                    "live DKG: agreement announcement deferred to the next tick"
                ),
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
                // Our OWN seal broadcast is not ack-gated (it carries no ack; the log is
                // re-fetchable via the resolver), so a failed journal only warns.
                let _ = self.append_journal(e, step.journal);
            }
        }

        // 1b. Evict pending dealing buffers for epochs we will never start (a
        //     dealing-closed ceremony exists, or the epoch is now in the past) so
        //     `pending` stays O(1–2 live epochs). An absent ceremony means not-yet-
        //     started → still bufferable.
        self.pending
            .retain(|e, _| *e > now && self.ceremonies.get(e).is_none_or(|c| !c.dealing_closed()));

        // 2. Finalize any SEALED ceremony whose log set has SETTLED (all-in, or the
        //    deterministic settle deadline). Also driven from `on_message`, so an
        //    all-in completed by an incoming Reveal finalizes immediately — see
        //    [`Self::drive_finalization`].
        self.drive_finalization(height, rng);

        // 2a. Gossip this node's share-confirmation for any target whose body-checked
        //     set grew since the last one (a no-op when nothing grew). Runs AFTER
        //     `drive_finalization`, which is what publishes the set being confirmed.
        to_send.extend(self.mint_confirmations());

        // 2a'. Ask the plane for an agreement instance for every target whose
        //      dealing has closed. AFTER `drive_finalization`, so an epoch this
        //      tick already finalized locally is not announced at all.
        self.announce_agreement_targets().await;

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
        //
        //     TWO distinct lifetimes now (§8.11.1 recompute-heal):
        //   - the LIVE ceremony object is still dropped at its OWN boundary (`e <= now`):
        //     a ceremony past its boundary can only finalize over a NON-pinned set (an
        //     un-self-checked share), so a demoted member heals via the RETAINED journal
        //     + the scoped, self-checked recompute (`drive_recompute`), NEVER a stale
        //     live ceremony (this also keeps the stalled-ceremony eviction invariant).
        //   - the JOURNAL + serve_cache (passive, self-verifying data) are RETAINED for
        //     the recompute window `e + JOURNAL_RETENTION_EPOCHS >= now`, so a demoted
        //     member (or a peer it serves) can still recompute E's share while E is
        //     committee-relevant. A journal is evicted only once its epoch has aged out
        //     of the window — past BOTH serve_cache and recompute_pending, which own the
        //     two window lifetimes (a finalized epoch rides serve_cache; a demoted one
        //     rides recompute_pending) — so it is never reclaimed while still needed.
        let evictable: Vec<u64> = self
            .ceremonies
            .keys()
            .copied()
            .chain(self.serve_cache.keys().copied())
            .chain(self.recompute_pending.keys().copied())
            .filter(|e| e + JOURNAL_RETENTION_EPOCHS < now)
            .collect();
        for e in &evictable {
            self.evict_journal(*e);
        }
        //     A target with a LIVE agreement instance is exempt from both sweeps
        //     below, and the exemption is what decouples the plane from this clock.
        //     The plane exists to keep agreeing while the ordering chain has HALTED,
        //     so `now` reaching the target says nothing about whether its agreement
        //     is done — and a swept ceremony makes `derive_pinned` answer
        //     `Unavailable` forever, which parks every `verify` and leaves
        //     `build_proposal` with nothing to pin. The hold is released when the
        //     instance ends, including when the epoch manager aborts it below the
        //     frontier, so this is a lifetime and not a wider window.
        let agreeing = self.agreement_targets.live();
        self.ceremonies
            .retain(|e, _| *e > now || agreeing.contains(e));
        // Report-once marks die with their ceremony, so a re-entered epoch reports
        // again — and they outlive it exactly as long as the ceremony does, or a
        // held target past its boundary would re-warn on every height tick.
        self.deferred_reported
            .retain(|(e, _)| *e > now || agreeing.contains(e));
        // Share-confirmations are per-target scratch on the same lifetime: useful
        // only while that target's agreement can still run, and held past the
        // boundary for exactly as long as a live instance holds its ceremony.
        if let Some(pool) = self.share_confirms.as_ref() {
            pool.retain(|e| e > now || agreeing.contains(&e));
        }
        self.confirmed_len
            .retain(|e, _| *e > now || agreeing.contains(e));
        // The dealer-log hash index is per-E+1 scratch: an entry for epoch E+1 is
        // useful only while agreeing/finalizing during epoch E, so drop `<= now`.
        if let Some(shared) = self.recorded_dkg_logs.as_ref() {
            if let Ok(mut m) = shared.write() {
                m.retain(|e, _| *e > now || agreeing.contains(e));
            }
        }
        // Reclaim the serve cache only once its epoch ages out of the recompute window —
        // the SAME predicate as the journal evict above and the first-tick reconcile, so
        // the drivers can never disagree (the cache is a subset-copy of the journal,
        // never outliving it).
        self.serve_cache
            .retain(|e, _| e + JOURNAL_RETENTION_EPOCHS >= now);
        // Artifact-sourced pinned sets age out on the SAME window, and dropping
        // one releases its ceremony-retention hold. A set is normally removed the
        // instant its finalize succeeds; this is the bound for the case where it
        // never does — a body no peer still holds — so a stuck write-back cannot
        // pin a ceremony open for the life of the process.
        self.agreed_pinned
            .retain(|e, _| e + JOURNAL_RETENTION_EPOCHS >= now);
        // Announced marks ride the ceremony's own lifetime: a target whose
        // ceremony is gone will never be announced again, and keeping the mark
        // would silently bar a re-entered epoch from getting an instance.
        let live_ceremonies: BTreeSet<u64> = self.ceremonies.keys().copied().collect();
        self.agreement_announced
            .retain(|e| live_ceremonies.contains(e));

        // Bound the SHARED insert-only CeremonyStore (writes at `store.insert`). Readers
        // pick the in-force mint via `range(..=E).next_back()`, so a plain size-cap could
        // demote a legitimate signer on a stable committee — instead retain every mint
        // `>= floor`, the mint still in force for the oldest cert inside the scheme-
        // retention window. Stable committee ⇒ nothing pruned; churn ⇒ trailing-window
        // bound. See `ceremony_retain_floor`.
        if let Ok(mut store) = self.store.write() {
            let floor =
                ceremony_retain_floor(store.keys().copied(), now, SCHEME_RETENTION_EPOCHS as u64);
            if floor > 0 {
                store.retain(|mint, _| *mint >= floor);
            }
        }

        // 3. Start the NEXT epoch's DKG ceremony. Retried on EVERY tick (not just
        //    once at the epoch transition): committee[E+1] is committed on-chain
        //    sometime DURING epoch E, which can land AFTER the actor (driven by
        //    lagging finalized heights) first enters E — a single-shot check at the
        //    transition would see the committee still unchanged, carry forward, and
        //    NEVER deal, so the E+1 boundary block wedges (no PK_{E+1}). maybe_start
        //    is idempotent (no-op once the ceremony is in flight, already computed,
        //    the committee is unchanged, or this node is not a member), so retrying
        //    until committee[E+1] is visible+changed is safe.
        // 3c. Reliable dealing delivery (P1, dealer leg): re-send each un-acked dealing
        //     point-to-point while pre-seal, so a member that dropped our initial dealing
        //     (or whose ack we lost) still receives it and (re-)acks. Bounded — each
        //     ceremony's `unsent` shrinks to ∅ as acks land; a no-op once every ceremony
        //     has sealed or resumed player-only. Runs BEFORE `maybe_start` so a ceremony
        //     started THIS tick is not retransmitted on top of its own initial send
        //     (`maybe_start` already queues that); it retransmits on every SUBSEQUENT
        //     pre-seal tick until acks drain. The player-leg ack re-emit needs no actor
        //     change — it rides the `on_message → handle → broadcast_all` path.
        for c in self.ceremonies.values() {
            to_send.extend(c.retransmit());
        }

        self.maybe_start(now + 1, &mut to_send);

        self.broadcast_all(to_send).await;

        // 3b. Demote-heal (§8.11.1): the actor SELF-DETECTS a demoted committee[E]
        //     member lacking E's share (from its OWN committee_for/store/me_key — no
        //     epoch_manager→actor signal) and recomputes that share from the RETAINED
        //     journal, scoped to the pinned dealers() and self-checked against PK_E, then
        //     fires share_notify so the existing in-process Verifier→Signer promote edge
        //     re-runs. Inert without a marshal reader + a journal dir.
        self.drive_recompute(now, rng).await;

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
    /// Publish each live ceremony's recorded dealer-log hashes
    /// (`idx→keccak256(SignedDealerLog)`, `idx` = the dealer's position in the agreed
    /// `committee[epoch]`) into the shared `recorded_dkg_logs` — what the agreement
    /// plane proposes from and what a share-confirmation states. Monotone (recorded
    /// logs only accrue) + idempotent; no-op when unwired. Committee-read per live
    /// ceremony (`n ≤ 51`, cheap).
    fn publish_recorded_logs(&self) {
        let Some(shared) = self.recorded_dkg_logs.as_ref() else {
            return;
        };
        let Ok(mut map) = shared.write() else {
            return;
        };
        let mut grew = false;
        for (e, c) in &self.ceremonies {
            let Some(committee) = (self.committee_for)(*e) else {
                continue;
            };
            for (idx, pk) in committee.iter().enumerate() {
                if let Some(hash) = c.signed_log_hash(pk) {
                    grew |= map.entry(*e).or_default().insert(idx as u8, hash).is_none();
                }
            }
        }
        drop(map);
        // A parked agreement leader re-reads this index when it wakes, and this is
        // the only edge that grows it. The confirmation pool carries both wakeups
        // (see `ConfirmPool::subscribe`), so a leader whose last missing input was a
        // dealer log — not a confirmation — is woken here rather than sleeping out
        // its view.
        if grew {
            if let Some(pool) = self.share_confirms.as_ref() {
                pool.note_inputs_grew();
            }
        }
    }

    /// Mint and gossip this node's share-confirmation for every target epoch whose
    /// body-checked dealer-log set has GROWN since the last one.
    ///
    /// The count of members that confirm they hold a usable set is the number that
    /// decides the epoch's fate — a member without one demotes to verify-only and
    /// votes false on the change-boundary block — and it had no protocol
    /// representation at all before this. Minting is event-driven off the recording
    /// paths (every caller runs immediately after `publish_recorded_logs`) and never
    /// on a timer: the statement only changes when a log is recorded.
    ///
    /// Inert without both the pool and the shared recorded index. The set is read
    /// from that index rather than from the live ceremonies deliberately: a ceremony
    /// is CONSUMED at finalize, so a node that has just derived its share — the very
    /// node whose confirmation matters most — would otherwise fall silent.
    fn mint_confirmations(&mut self) -> Vec<Outgoing> {
        let (Some(pool), Some(recorded)) =
            (self.share_confirms.clone(), self.recorded_dkg_logs.as_ref())
        else {
            return Vec::new();
        };
        let Ok(index) = recorded.read() else {
            return Vec::new();
        };
        let held: Vec<(u64, BTreeMap<u8, B256>)> =
            index.iter().map(|(e, set)| (*e, set.clone())).collect();
        drop(index);

        let me = self.me_key.public_key();
        let mut out = Vec::new();
        for (epoch, set) in held {
            let Some(roster) = (self.committee_for)(epoch) else {
                continue;
            };
            let Some(idx) = roster
                .iter()
                .position(|pk| *pk == me)
                .and_then(|i| u8::try_from(i).ok())
            else {
                continue; // not a member of this target's committee
            };
            let confirmed: Vec<(u8, B256)> = set
                .into_iter()
                .filter(|(i, _)| (*i as usize) < roster.len())
                .collect();
            if confirmed.is_empty()
                || self
                    .confirmed_len
                    .get(&epoch)
                    .is_some_and(|last| *last >= confirmed.len())
            {
                continue;
            }
            self.confirmed_len.insert(epoch, confirmed.len());
            let confirm = ShareConfirm::sign(pool.namespace(), &self.me_key, idx, epoch, confirmed);
            let members: Vec<PeerPubkey> = roster.iter().cloned().collect();
            pool.record(&members, confirm.clone());
            out.push(Outgoing {
                target: Target::Broadcast,
                msg: DkgMsg {
                    ceremony_epoch: epoch,
                    body: DkgBody::Confirm(confirm),
                },
            });
        }
        out
    }

    /// Record a peer's share-confirmation, or drop it.
    ///
    /// The pool re-verifies the signature against `committee[target_epoch][idx]`, so
    /// a relayed confirmation is as good as a directly-sent one and the sender is
    /// only ever a diagnostic. The unsigned envelope epoch must agree with the
    /// signed one — a mismatch is either a relay bug or an attempt to slip a
    /// confirmation past a receive-side epoch filter it does not actually bind.
    fn on_confirm(&mut self, envelope_epoch: u64, from: &PeerPubkey, confirm: ShareConfirm) {
        let Some(pool) = self.share_confirms.as_ref() else {
            return;
        };
        if confirm.target_epoch != envelope_epoch {
            tracing::debug!(
                target: "dpos::beacon",
                envelope_epoch,
                signed_epoch = confirm.target_epoch,
                "share-confirmation framing disagrees with its own signed epoch"
            );
            return;
        }
        let Some(roster) = (self.committee_for)(confirm.target_epoch) else {
            return;
        };
        let members: Vec<PeerPubkey> = roster.iter().cloned().collect();
        if !pool.record(&members, confirm) {
            tracing::debug!(
                target: "dpos::beacon",
                epoch = envelope_epoch,
                %from,
                "share-confirmation not recorded (unverifiable, or narrower than the one held)"
            );
        }
    }

    fn drive_finalization(&mut self, height: u64, rng: &mut impl CryptoRngCore) {
        // Publish our recorded dealer-log hashes for the agreement plane to propose
        // from and for share-confirmations to state — the recording paths (seal /
        // Reveal / resolver ingest) all funnel through here.
        self.publish_recorded_logs();
        // `ready` probes non-destructively (Logs clone); `finalize` then consumes the
        // fulfilled ceremony. Both run STILL DURING the margin window — before the
        // epoch's boundary block is proposed/verified — so the verify-path C gate can
        // read the share.
        // Past-deadline deferrals observed this tick, reported after the borrow ends.
        // `(epoch, reason, unmappable_pinned)`.
        let mut deferrals: Vec<(u64, &'static str, usize)> = Vec::new();
        let plans: Vec<(u64, FinalizePlan)> = self
            .ceremonies
            .iter()
            .filter_map(|(e, c)| {
                if !c.dealing_closed() || !c.can_finalize() {
                    return None;
                }
                let target = *e;
                let seal_deadline = self.epoch_start(target).saturating_sub(DKG_MARGIN_BLOCKS);
                // Where the set came from decides whether a deadline is needed at
                // all. The settle deadline exists ONLY to make every honest node
                // select over an identical set; a quorum-certified artifact states
                // that outright, so an agreed set is settled the moment it lands —
                // which is also what decouples the write-back from this clock.
                let certified = self.agreed_pinned.contains_key(&target);
                let past_deadline = certified || height >= seal_deadline + DKG_SETTLE_BLOCKS;
                // The SOURCE swap, and it is the whole of the write-back: an agreed
                // artifact's set displaces the finalized-consensus one for its
                // epoch. Everything below this line is unchanged.
                let pinned_source = match self.agreed_pinned.get(&target) {
                    Some(agreed) => Some(agreed.pinned.clone()),
                    // An agreement plane is wired but this target's artifact has
                    // not landed: answer EMPTY, not `None`. Empty takes the Pinned
                    // arm's "no set yet" early return and waits; `None` would drop
                    // through to the LEGACY local-settled gate, whose whole defect
                    // is that two honest nodes can settle different sets — the
                    // divergence this plane exists to remove. The legacy arm is
                    // reachable only where no plane is wired at all.
                    None if self.agreement_tx.is_some() => Some(BTreeMap::new()),
                    None => None,
                };
                match pinned_source {
                    // Deterministic finalize: the finalize INPUT is the AGREED
                    // dealer-log HASH SET, NOT the local settled set — so every honest
                    // node selects over the IDENTICAL pinned set ⇒ identical `PK_E`
                    // (honest divergence impossible by construction).
                    Some(pinned) => {
                        if pinned.is_empty() {
                            return None; // no agreed dealer-log hashes yet
                        }
                        let committee = (self.committee_for)(target)?;
                        let n = committee.len();
                        // `all_held` = every pinned body held with a matching hash
                        // (fetch-before-finalize: a `false` means WAIT, never subset-
                        // finalize — the resolver fetches the missing pinned bytes);
                        // `ready` = a quorum is selectable within the pinned+held set.
                        let (ready, all_held) = c.pinned_ready(rng, &committee, &pinned);
                        // Fast path: the pinned set is COMPLETE (all n) and fully held.
                        let fast = pinned.len() == n && all_held && ready;
                        // Deadline: qualified iff every pinned body is held AND a quorum
                        // is selectable; else defer (below-quorum → boundary sweep;
                        // missing pinned body → keep waiting/fetching).
                        let at_deadline = past_deadline && all_held && ready;
                        if past_deadline && !at_deadline {
                            // This wait used to be completely silent, so a stall of this
                            // family was only diagnosable post-mortem from a wedged
                            // boundary. Report it, and separately count pinned indices
                            // with no committee position: those can never be satisfied
                            // (the ceremony skips them, see `scoped_pinned_logs`), so a
                            // non-zero count means the pinned set and the committee this
                            // node reads disagree.
                            let unmappable = pinned.keys().filter(|i| **i as usize >= n).count();
                            let reason = if all_held {
                                "below_quorum"
                            } else {
                                "missing_body"
                            };
                            deferrals.push((target, reason, unmappable));
                        }
                        (fast || at_deadline)
                            .then_some((target, FinalizePlan::Pinned(committee, pinned)))
                    }
                    // LEGACY local-settled-set gate (unwired in-process/test default):
                    // ALL-IN (every committee log recorded) OR the height-deterministic
                    // SETTLE deadline past the seal.
                    None => {
                        if !c.ready(rng) {
                            return None;
                        }
                        if past_deadline {
                            return Some((target, FinalizePlan::Legacy));
                        }
                        let n = (self.committee_for)(target).map_or(0, |s| s.len());
                        (n > 0 && c.recorded_log_count() == n)
                            .then_some((target, FinalizePlan::Legacy))
                    }
                }
            })
            .collect();
        for (epoch, reason, unmappable) in deferrals {
            if unmappable > 0 {
                self.metrics.dkg_pinned_idx_out_of_range.inc();
            }
            if !self.deferred_reported.insert((epoch, reason)) {
                continue; // already reported for this epoch+reason
            }
            self.metrics.dkg_finalize_deferred.inc();
            if unmappable > 0 {
                tracing::error!(
                    target: "dpos::beacon",
                    epoch,
                    reason,
                    unmappable,
                    "DKG finalize deferred past the settle deadline and the pinned set names \
                     indices outside the committed committee — the pinned set and this node's \
                     committee disagree"
                );
            } else {
                tracing::warn!(
                    target: "dpos::beacon",
                    epoch,
                    reason,
                    "DKG finalize deferred past the settle deadline"
                );
            }
        }
        for (e, plan) in plans {
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
            let result = match &plan {
                FinalizePlan::Legacy => c.finalize(rng),
                FinalizePlan::Pinned(committee, pinned) => {
                    c.finalize_over_pinned(rng, committee, pinned)
                }
            };
            match result {
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
                    // The agreed artifact rides along where this finalize ran over
                    // one, so the restart also comes back able to SERVE the key it
                    // reloaded, not only to sign with it.
                    // Best-effort — the in-memory store is authoritative for the
                    // running process, so a write failure only warns.
                    if let Some(dir) = &self.share_dir {
                        let artifact = self
                            .agreed_pinned
                            .get(&e)
                            .map(|agreed| agreed.encoded_artifact.as_slice());
                        if let Err(err) =
                            share_state::persist(dir, e, &out, &share, artifact, &self.share_state)
                        {
                            tracing::warn!(
                                epoch = e,
                                ?err,
                                "live DKG: failed to persist share to disk (in-memory store unaffected)"
                            );
                        }
                    }
                    // QUALIFY-BEFORE-COMMIT (AMENDMENT 5): "qualified(e)" is now simply
                    // this finalize — the share landing in the shared CeremonyStore IS
                    // the node's deterministic qualified verdict (read by the vote-time
                    // marker gate + the marker relayer). No separate cert artifact.
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
                    // The write-back is complete for this epoch, so its agreed set
                    // and the ceremony-retention hold that rode with it are spent.
                    self.agreed_pinned.remove(&e);
                    self.metrics.dkg_ceremony_ok.inc();
                    tracing::info!(
                        epoch = e,
                        height = self.last_height,
                        "live DKG: PK_epoch + share computed + stored"
                    );
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
    fn maybe_start(&mut self, target: u64, out: &mut Vec<Outgoing>) {
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
        // `cur` = the COMMITTED (key-owning) roster of `target−1`; `next` = the
        // COMMITTED roster of `target`. Under the 2-epoch committee warm-up
        // `committee[target]` was frozen a full epoch earlier (at its `target−2`
        // selection block), so both sides read the immutable committed slot — the
        // candidate/stash reader is gone. `active_committee_for` defaults to
        // `committee_for.clone()`, so both closures resolve the same committed set.
        let cur = (self.active_committee_for)(target - 1);
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
        // ONCE. A genuine first run (`NoFile`) deals (seeded ⇒ idempotent even after a
        // datadir loss); a present journal RESUMES — reconstructing the seeded dealer
        // pre-seal, player-only at/after the deadline (§8.11.1); a present-but-damaged
        // journal (`Torn`) means we already participated in this epoch's ceremony, so
        // dealing fresh would self-equivocate — we SIT OUT instead.
        let started = match self.load_journal(target) {
            JournalLoad::NoFile => self.start_fresh(target, next, out),
            JournalLoad::Present(records) => {
                // Timing gate (R2-L): a node only ever seals AT/AFTER the deadline, so
                // `height < deadline ⇒ we never sealed ⇒ no original log was ever
                // broadcast`, and re-deriving the seeded (idempotent) dealer + sealing
                // once later is safe. At/after the deadline we stay player-only — a torn
                // journal cannot prove we did not already seal + broadcast a possibly-
                // divergent log, so we NEVER re-seal.
                let reconstruct_dealer =
                    self.last_height < self.epoch_start(target).saturating_sub(DKG_MARGIN_BLOCKS);
                self.resume_from_journal(target, next, records, reconstruct_dealer, out)
            }
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
            // Collect each drained dealing's Step (borrowing the ceremony only for the
            // replay) so its ack broadcast can be gated on ITS OWN `ReceivedDealing`
            // write being durable (step 1f), exactly as the on_message path does.
            let mut steps: Vec<Step> = Vec::new();
            {
                let c = self.ceremonies.get_mut(&target).expect("just started");
                for (from, dealings) in buffered {
                    // Replay each present half (commitment-then-share). Order-independent
                    // (`try_ack` fires only once both halves are buffered), so `from` is
                    // re-used per replay → clone (PeerPubkey is Clone, NOT Copy).
                    for body in [dealings.commitment, dealings.share].into_iter().flatten() {
                        steps.push(c.handle(from.clone(), body));
                    }
                }
            }
            for step in steps {
                if self.append_journal(target, step.journal) {
                    out.extend(step.outgoing);
                }
            }
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
    /// whether a ceremony is now live. The dealer polynomial is SEEDED from the
    /// validator key + epoch (`ceremony::dealer_seed_rng`), so a `NoFile → start_fresh`
    /// after a datadir loss re-derives the IDENTICAL commitment — closing the former
    /// `OsRng` re-deal self-equivocation gap (§8.11.1).
    fn start_fresh(&mut self, target: u64, next: Set<PeerPubkey>, out: &mut Vec<Outgoing>) -> bool {
        match DkgCeremony::start(&self.namespace, target, next, self.me_key.clone()) {
            Ok((ceremony, step)) => {
                self.ceremonies.insert(target, ceremony);
                out.extend(step.outgoing);
                // Start records our own commitment/self-dealing; the outgoing is our
                // broadcast commitment + private shares, not an ack, so it is not gated.
                let _ = self.append_journal(target, step.journal);
                tracing::info!(epoch = target, "live DKG: ceremony started");
                true
            }
            Err(e) => {
                tracing::warn!(epoch = target, ?e, "live DKG: ceremony start failed");
                false
            }
        }
    }

    /// Resume `target`'s ceremony from its journaled `records` (mid-window restart).
    /// PRE-seal (`reconstruct_dealer`) it RE-DERIVES the seeded dealer and keeps
    /// distributing; at/after the deadline it is player-only and never re-seals
    /// (§8.11.1). A `MissingPlayerDealing` (truncated journal dropped a publicly-acked
    /// dealing) is a graceful sit-out for this epoch, never a crash. Returns whether a
    /// ceremony is now live.
    fn resume_from_journal(
        &mut self,
        target: u64,
        next: Set<PeerPubkey>,
        records: Vec<JournalRecord>,
        reconstruct_dealer: bool,
        out: &mut Vec<Outgoing>,
    ) -> bool {
        match DkgCeremony::resume(
            &self.namespace,
            target,
            next,
            self.me_key.clone(),
            records,
            reconstruct_dealer,
        ) {
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
        let payload = match BeaconMessage::read(&mut wire) {
            Ok(BeaconMessage::Dkg(p)) => p,
            Err(_) => return,
        };
        let mut body = payload.as_ref();
        let msg = match DkgMsg::read_cfg(&mut body, &max) {
            Ok(m) => m,
            Err(_) => return,
        };
        let epoch = msg.ceremony_epoch;
        let body = msg.body;
        // A share-confirmation is agreement traffic, not ceremony traffic: it is
        // consumed by the entry bar and must be recorded whether or not this node
        // has a live ceremony for the epoch (it may have finalized one already).
        // Intercepted here, before every ceremony dispatch and buffering path.
        if let DkgBody::Confirm(confirm) = body {
            self.on_confirm(epoch, &from, confirm);
            return;
        }
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
            // Step 1f (write-durably-before-ack). A `handle` step's outgoing is an `Ack`:
            // either a FIRST-receipt ack paired with a `ReceivedDealing` journal record,
            // or a RE-EMITTED cached ack with an EMPTY journal (already durable). A
            // `Reveal` records a `PeerLog` + emits nothing; an inbound `Ack` prunes our
            // `unsent` + records an `OwnDealerAck` + emits nothing. Broadcast the outgoing
            // ONLY once the (possibly empty) journal write is durable — else a QUAL log
            // could record an ack this node cannot back with a durable view (an
            // un-resurrectable `MissingPlayerDealing` the recompute-heal cannot recover).
            // An empty journal is durable trivially (the re-emit passes the gate). On a
            // write failure the ack is WITHHELD: the dealer then reveals our point in its
            // own log, which the recompute recovers just as well — safe, no liveness loss.
            let durable = self.append_journal(epoch, step.journal);
            if durable {
                self.broadcast_all(step.outgoing).await;
            }
            // A newly-recorded Reveal may have just made a sealed ceremony all-in —
            // finalize NOW, event-driven, over the settled set. ONLY a recorded log can
            // change finalizability, so skip the `observe` batch-BLS for ack-only /
            // dealing-only steps (review [806]); the height-driven settle-deadline
            // finalize in `on_height` still covers the time-based path. See
            // [`Self::drive_finalization`].
            if recorded_log {
                self.drive_finalization(self.last_height, rng);
                // A newly-recorded log widens what this node can confirm, and the
                // entry bar is counted over confirmations that COVER the proposed
                // set — so the widened statement has to reach peers on the same
                // edge, not on the next height tick.
                let minted = self.mint_confirmations();
                self.broadcast_all(minted).await;
            }
        } else if self.is_bufferable(epoch, &body) {
            // PER-SENDER, latest-wins: a sender occupies at most its own slot (≤1
            // Commitment + ≤1 Share), so a Byzantine peer cannot evict honest
            // dealings (N2). The outer + inner maps are roster-bounded, so the
            // prior `len() <` cap is redundant.
            let slot = self
                .pending
                .entry(epoch)
                .or_default()
                .entry(from)
                .or_default();
            match body {
                DkgBody::Commitment(_) => slot.commitment = Some(body),
                DkgBody::Share(_) => slot.share = Some(body),
                // `is_bufferable` admits ONLY Commitment/Share; defensive no-op.
                _ => {}
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
        // Targets a live agreement instance or an in-flight write-back is holding
        // open — see the boundary gate below.
        let held = self.agreement_targets.live();
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
            //
            // A HELD target is exempt, on the same lifetime the ceremony sweep
            // grants it: an agreement instance still running past the boundary, or
            // an agreed set still fetching the bodies its finalize needs, is
            // precisely a ceremony the sweep is NOT about to drop — and refusing to
            // fetch for it would leave the write-back waiting on a body it stopped
            // asking for.
            if height >= self.epoch_start(*e) && !held.contains(e) {
                continue; // past the boundary — the ceremony is about to be swept
            }
            // Target each fetch at the roster (the known holders). `fetch_targeted`
            // narrows within `latest.primary`; a committee member's logs are served
            // from any peer that holds them. The holders are in `latest.primary`
            // during E-1: committee[E] ⊆ the Active registry that the beacon plane's
            // `EpochTransition` tracks (registry ∪ committee[E]) on the SAME
            // `OracleHandle` the resolver's `Provider` reads, and the E-1→E boundary
            // `track(E)` re-includes committee[E] explicitly (STEP-0 reachability).
            let Some(targets) =
                NonEmptyVec::try_from(roster.iter().cloned().collect::<Vec<_>>()).ok()
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
        // Demote-heal fetches (§8.11.1): drive the pinned `dealers()` logs we still lack
        // for each recompute_pending epoch, targeting the roster. This runs PAST the
        // boundary (bounded to the retention window by recompute_pending's own lifetime),
        // which the in-window ceremony fetch above (gated at `epoch_start`) does not. The
        // target set is exactly `dealers(E) − held`, so a log NO peer holds simply backs
        // off with the epoch's age-out — never an unbounded storm.
        for (e, st) in &self.recompute_pending {
            if st.want.is_empty() {
                continue;
            }
            let Some(roster) = (self.committee_for)(*e) else {
                unreadable.insert(*e);
                continue;
            };
            let Some(targets) =
                NonEmptyVec::try_from(roster.iter().cloned().collect::<Vec<_>>()).ok()
            else {
                continue;
            };
            for dealer in &st.want {
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

    /// Demote-heal driver (§8.11.1), run each `on_height` tick AFTER the sweep. All
    /// LOCAL — no `epoch_manager`→actor signal:
    /// 1. AGE-OUT — drop `recompute_pending` entries whose epoch left the retention
    ///    window (their journals were reclaimed by the sweep); a caught-up-too-late
    ///    member stops trying and stays a SAFE verify-only observer.
    /// 2. SELF-DETECT — for each epoch E in the window with `me ∈ committee[E]`,
    ///    `E ≥ DETERMINISTIC_BOOTSTRAP_EPOCH`, `store` lacking E's share, and NOT already
    ///    pending: read E's agreed `Output` via the artifact READ handle. `Some`
    ///    ⇒ a CHANGE-epoch demote (a qualified member WOULD hold `store[E]`) ⇒ record
    ///    `recompute_pending[E] = { outcome, want: dealers()−held }`. `None` ⇒
    ///    carry-forward (no fresh DKG ⇒ not a demote) or a marshal miss ⇒ skip (retry).
    /// 3. TRY — attempt the scoped recompute for any pending epoch whose `want` is empty.
    ///
    /// Inert without BOTH an artifact reader (the agreed outcome) AND a journal dir (the
    /// recompute inputs) — the in-process/test default.
    async fn drive_recompute(&mut self, now: u64, rng: &mut impl CryptoRngCore) {
        let Some(outcome_at) = self.outcome_at.clone() else {
            return;
        };
        if self.share_dir.is_none() {
            return;
        }

        // 1. Age-out (past the retention window).
        self.recompute_pending
            .retain(|e, _| *e + JOURNAL_RETENTION_EPOCHS >= now);

        // 2. Self-detect over the small window [max(BOOTSTRAP, now−RET), now]. A future
        //    epoch (> now) is not seated/demoted yet; below BOOTSTRAP the beacon is
        //    seedless (no share obligation).
        let lo = now
            .saturating_sub(JOURNAL_RETENTION_EPOCHS)
            .max(DETERMINISTIC_BOOTSTRAP_EPOCH);
        let me = self.me_key.public_key();
        for e in lo..=now {
            if self.recompute_pending.contains_key(&e) {
                continue;
            }
            // Already holds E's share (qualified) — not demoted.
            if self.store.read().ok().is_some_and(|s| s.contains_key(&e)) {
                continue;
            }
            let Some(committee) = (self.committee_for)(e) else {
                continue;
            };
            if !committee.iter().any(|p| *p == me) {
                continue; // not a member of committee[E] — no share obligation
            }
            // Read the agreed Output for E. `Some` ⇒ a change-epoch demote to
            // recompute; `None` ⇒ carry-forward or store-miss ⇒ skip (retry next tick).
            let Some(outcome) = outcome_at(e).await else {
                continue;
            };
            // Defensive shape check: the agreed outcome must be over EXACTLY
            // committee[E]; else it is not this epoch's mint — skip.
            if outcome.players() != &committee {
                continue;
            }
            // want = pinned dealers() − the dealer logs already in our retained journal.
            let held = self.cold_load_serve_cache(e);
            let want: BTreeSet<PeerPubkey> = outcome
                .dealers()
                .iter()
                .filter(|d| !held.contains_key(*d))
                .cloned()
                .collect();
            tracing::info!(
                epoch = e,
                want = want.len(),
                dealers = outcome.dealers().len(),
                "live DKG: demoted committee member detected — starting share recompute-heal"
            );
            self.recompute_pending
                .insert(e, RecomputeState { outcome, want });
        }

        // 3. Try the ready ones.
        self.try_recompute_pending(rng);
    }

    /// Attempt the scoped share recompute for each `recompute_pending` epoch whose
    /// `want` is empty (we now hold every pinned dealer's log). Loads the retained
    /// journal, runs the `dealers()`-scoped [`recompute_scoped`], and adopts the share
    /// IFF it self-verifies against the pinned `Output` ([`validate_share_on_poly`]).
    ///
    /// On adopt: persist + store `(PK_E, share)`, seed the serve cache (so peers can
    /// still fetch this epoch's logs while it is in-window), evict the now-superseded
    /// journal, fire `share_notify` (re-runs the in-process promote edge) +
    /// `dkg_ceremony_ok`. On ANY failure (torn/short journal, self-check fail, `Err`) the
    /// entry STAYS (keep fetching) and NO share is adopted — a wrong-but-valid-looking
    /// share can never leak into consensus (the FORK-SAFETY guard).
    fn try_recompute_pending(&mut self, rng: &mut impl CryptoRngCore) {
        let ready: Vec<u64> = self
            .recompute_pending
            .iter()
            .filter(|(_, st)| st.want.is_empty())
            .map(|(e, _)| *e)
            .collect();
        for e in ready {
            let Some(committee) = (self.committee_for)(e) else {
                continue;
            };
            let dealers = match self.recompute_pending.get(&e) {
                Some(st) => st.outcome.dealers().clone(),
                None => continue,
            };
            let records = match self.load_journal(e) {
                JournalLoad::Present(r) => r,
                // NoFile/Torn: cannot recompute now — keep pending (SAFE), retry later.
                _ => continue,
            };
            let recomputed = recompute_scoped(
                rng,
                &self.namespace,
                e,
                committee.clone(),
                self.me_key.clone(),
                &dealers,
                records,
            );
            // FORK-SAFETY GATE: adopt ONLY when the recomputed share lies on the PINNED
            // poly at our index — a wrong log-subset / tampered journal fails this and is
            // never adopted (the member keeps fetching, stays safe verify-only).
            let adopt = match &recomputed {
                Ok((_out, share)) => {
                    validate_share_on_poly(&self.recompute_pending[&e].outcome, &committee, share)
                }
                Err(_) => false,
            };
            if !adopt {
                continue;
            }
            let (_out, share) = recomputed.expect("adopt gated on Ok");
            let st = self
                .recompute_pending
                .remove(&e)
                .expect("present (just read)");
            // Store the PINNED outcome (the canonical one we self-checked against) + the
            // recomputed share. Persist BEFORE the in-memory insert (which moves the pair).
            // No artifact: this path reconstructs the share from the retained journal of a
            // ceremony this node was demoted out of, and holds no certified artifact to
            // write down. A reload reads the absence as "re-agree", never as damage.
            if let Some(dir) = &self.share_dir {
                if let Err(err) =
                    share_state::persist(dir, e, &st.outcome, &share, None, &self.share_state)
                {
                    tracing::warn!(
                        epoch = e,
                        ?err,
                        "live DKG: failed to persist recomputed share (in-memory store unaffected)"
                    );
                }
            }
            if let Ok(mut store) = self.store.write() {
                store.insert(e, (st.outcome, share));
            }
            // The journal is now superseded (the share is memoized); seed the serve cache
            // so this epoch's logs stay servable to peers for the window, then reclaim it.
            let logs = self.cold_load_serve_cache(e);
            if !logs.is_empty() {
                self.serve_cache.insert(e, logs);
            }
            self.evict_journal(e);
            self.share_notify.notify_one();
            self.metrics.dkg_ceremony_ok.inc();
            tracing::info!(
                epoch = e,
                "live DKG: demoted committee member recomputed its share from the retained \
                 journal — share stored, promoting to Signer"
            );
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
        if let Some(c) = self.ceremonies.get_mut(&key.epoch) {
            // Bind the delivered log to the REQUESTED `key.dealer`: a forgery or a valid
            // log for a different dealer both return `false` (block + re-fetch `key`).
            let (accepted, journal) = c.ingest_signed_log(&key.dealer, signed);
            if accepted {
                let _ = self.append_journal(key.epoch, journal);
                self.drive_finalization(self.last_height, rng);
            }
            return accepted;
        }
        // No LIVE ceremony (swept at the boundary). If this epoch is a demote-heal
        // recompute target (§8.11.1), ingest the delivered log into its RETAINED journal
        // + `want` set and attempt the scoped recompute; otherwise the fetch is genuinely
        // moot → `true` (don't block an honest peer for a key WE no longer need).
        if self.recompute_pending.contains_key(&key.epoch) {
            return self.ingest_recompute_log(key, signed, rng);
        }
        true
    }

    /// Ingest a resolver-delivered `SignedDealerLog` for a `recompute_pending` epoch
    /// whose live ceremony was already swept: re-`check` it against the pinned `Info`
    /// and, iff it is a valid log signed by the REQUESTED `key.dealer`, JOURNAL it
    /// (retained for the window) + drop the dealer from `want`, then attempt the scoped
    /// recompute. Verdict mirrors the live-ceremony ingest: `true` = valid + correctly
    /// targeted (or already held); `false` = a forgery, a wrong-dealer answer, or an
    /// unverifiable committee read (block + re-fetch the key — never `true`, which would
    /// clear a still-needed fetch).
    fn ingest_recompute_log(
        &mut self,
        key: &DkgLogKey,
        signed: DealerReveal,
        rng: &mut impl CryptoRngCore,
    ) -> bool {
        let Some(committee) = (self.committee_for)(key.epoch) else {
            return false; // transient committee read race — keep the fetch alive
        };
        let Ok(info) = crate::beacon::ceremony::info_for(&self.namespace, key.epoch, committee)
        else {
            return false;
        };
        match signed.clone().check(&info) {
            Some((pk, _)) if pk == key.dealer => {
                let durable =
                    self.append_journal(key.epoch, vec![JournalRecord::PeerLog(Box::new(signed))]);
                // Only mark the dealer satisfied + attempt recompute once the log is a
                // DURABLE part of the journal the recompute reads; a non-durable write
                // keeps `want` (retry the fetch next tick).
                if durable {
                    if let Some(st) = self.recompute_pending.get_mut(&key.epoch) {
                        st.want.remove(&key.dealer);
                    }
                    self.try_recompute_pending(rng);
                }
                true
            }
            Some(_) => false, // a valid log, but for a different dealer than fetched
            None => false,    // a forgery
        }
    }

    /// Send each outgoing ceremony message over BEACON_CHANNEL (broadcast or direct).
    async fn broadcast_all(&mut self, msgs: Vec<Outgoing>) {
        for o in msgs {
            let wire = BeaconMessage::Dkg(o.msg.encode()).encode();
            let recipients = match o.target {
                Target::Broadcast => Recipients::All,
                Target::Direct(pk) => Recipients::One(pk),
            };
            // Best-effort: a dropped dealing is recovered by the per-tick dealer
            // retransmit (`retransmit`/step 3c) and a dropped ack by the player ack-cache
            // re-emit (`try_ack`), with the reveal mechanism as the last-resort backstop;
            // never block consensus on a send failure.
            let _ = self.sender.send(recipients, wire, false).await;
        }
    }
}

#[cfg(test)]
mod clock_tests {
    use super::*;
    use commonware_math::algebra::Random as _;
    use commonware_p2p::{
        simulated::{Config as SimConfig, Link, Network, Oracle},
        Manager as _,
    };
    use commonware_runtime::{deterministic, Clock as _, Metrics as _, Runner as _, Spawner as _};
    use commonware_utils::NZUsize;
    use rand_08::{rngs::StdRng, SeedableRng as _};
    use std::time::Duration;

    type SimContext = deterministic::Context;

    /// The chain id the test artifacts' agreement namespace is derived under. Only
    /// the pull seam re-derives it, and it has to agree with the signer.
    const AGREEMENT_CHAIN_ID: u64 = 20_994;

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
            _: Vec<(
                Self::Key,
                commonware_utils::vec::NonEmptyVec<Self::PublicKey>,
            )>,
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
            requests: Vec<(
                Self::Key,
                commonware_utils::vec::NonEmptyVec<Self::PublicKey>,
            )>,
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

    const SEAL_DEADLINE: u64 = INTERVAL * DETERMINISTIC_BOOTSTRAP_EPOCH - DKG_MARGIN_BLOCKS; // 20
    const BOUNDARY: u64 = INTERVAL * DETERMINISTIC_BOOTSTRAP_EPOCH; // 40 = epoch_start(2)
                                                                    // Pinned to DKG_MARGIN_BLOCKS (AM5 raised it 16→20) so `seal_deadline ==
                                                                    // epoch_start(1)` — the SAME relative geometry the tests were written against:
                                                                    // the epoch-2 ceremony is created by `maybe_start` at epoch-1 start and sealed
                                                                    // exactly at the seal deadline, so every SEAL_DEADLINE/BOUNDARY-relative drive
                                                                    // below keeps its meaning. (Production uses a much larger interval; this is the
                                                                    // minimal test geometry, not a protocol constraint.)
    const INTERVAL: u64 = 20;
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
        spawn_dealer_at(
            ctx,
            oracle,
            me,
            committee,
            store,
            share_notify,
            interval,
            None,
            7,
        )
        .await
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
            None,
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
            None,
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
        let keys: Vec<Ed25519PrivateKey> = (0..4)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
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
                spawn_dealer(
                    &ctx,
                    &oracle,
                    k.clone(),
                    committee.clone(),
                    store,
                    notify,
                    interval,
                )
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
        const {
            assert!(
                SEAL_DEADLINE + 2 < BOUNDARY,
                "assert point must precede the boundary"
            )
        };

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
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
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
                None,
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

    /// N2 fairness regression: the start-race buffer is bounded PER SENDER. One peer
    /// flooding its own `Commitment` occupies at most its single slot (latest-wins),
    /// so it can never evict another sender's buffered dealing from the shared
    /// per-epoch buffer. Drives `on_message` directly to inspect `actor.pending`.
    #[test]
    fn pending_buffer_is_per_sender_bounded() {
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
            let mut rng = StdRng::seed_from_u64(11);
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
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
                .expect("register BEACON_CHANNEL");
            let committee_for: CommitteeFor = {
                let set = committee.clone();
                Arc::new(move |_epoch: u64| Some(set.clone()))
            };
            let store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let ns = b"FLUENT_DPOS_V1_n2test";
            let mut actor = DkgActor::new(
                ns.to_vec(),
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
                None,
            );
            let mut arng = StdRng::seed_from_u64(13);

            // A real, decodable `Commitment` dealing tagged for epoch 1 (bufferable
            // at the default last_height=0 ⇒ now=0, so 0 < 1 ≤ now+2). The body is
            // never verified before buffering, so the same bytes stand in for any
            // sender's dealing — only the `from` key keys the per-sender slot.
            let commitment: DkgBody = {
                let (_cer, step) =
                    DkgCeremony::start(ns, 1, committee.clone(), keys[0].clone()).expect("start");
                step.outgoing
                    .into_iter()
                    .find_map(|o| match o.msg.body {
                        b @ DkgBody::Commitment(_) => Some(b),
                        _ => None,
                    })
                    .expect("a commitment dealing")
            };
            let wire = BeaconMessage::Dkg(
                DkgMsg {
                    ceremony_epoch: 1,
                    body: commitment,
                }
                .encode(),
            )
            .encode();

            let a = keys[1].public_key();
            let b = keys[2].public_key();

            // Sender A floods 5 copies → it overwrites its OWN slot (latest-wins).
            for _ in 0..5 {
                actor.on_message(a.clone(), wire.as_ref(), &mut arng).await;
            }
            assert_eq!(
                actor.pending.get(&1).map(|m| m.len()),
                Some(1),
                "5 copies of sender A's commitment occupy exactly ONE slot (latest-wins)"
            );

            // Sender B contributes ONE → it gets its OWN slot; A is NOT evicted.
            actor.on_message(b.clone(), wire.as_ref(), &mut arng).await;
            assert_eq!(
                actor.pending.get(&1).map(|m| m.len()),
                Some(2),
                "sender B gets its own slot; one peer's flood cannot starve another (N2)"
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
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
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
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
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
                    DkgCeremony::start(ns, 2, committee.clone(), k.clone()).expect("start");
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
                            let more = cers
                                .get_mut(&to)
                                .unwrap()
                                .handle(from.clone(), o.msg.body.clone());
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
                None,
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
            assert!(
                accepted,
                "a no-live-ceremony delivery is honest (deliver→true)"
            );
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
            assert!(
                !forged_valid,
                "a forged log is check-rejected (deliver→false)"
            );
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
                .ingest_log(
                    &valid_key0,
                    Bytes::from_static(&[0xFF, 0x00, 0x13]),
                    &mut arng,
                )
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
                assert!(
                    ok,
                    "a valid log for its own dealer is recorded (deliver→true)"
                );
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
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
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
            assert!(
                actor.serve_log(&key).is_none(),
                "a Torn journal serves no log"
            );
            assert!(
                !actor.serve_cache.contains_key(&2),
                "an unservable cold miss caches NOTHING — no negative entry ([965]/[954])"
            );
            // [954] bound: many distinct attacker-controlled far-future epochs (no journal)
            // never grow the cache.
            for e in 1_000u64..1_050 {
                assert!(actor
                    .serve_log(&DkgLogKey {
                        epoch: e,
                        dealer: keys[1].public_key()
                    })
                    .is_none());
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
        let _guard = COLD_PARSE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;

            // A PRESENT, valid epoch-2 journal (full committee logs).
            let dir = fresh_share_dir("transient-none");
            std::fs::create_dir_all(&dir).expect("mkdir");
            let logs = mint_committee_logs_at(&keys, &committee, 2);
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
            assert!(
                actor.serve_log(&key).is_none(),
                "transient None serves no log"
            );
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
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
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
                None,
            );

            // Inject an OPEN shorthanded ceremony (node-0 started but no peer logs) so
            // `fetch_missing_logs` issues fetches for the 3 missing peer dealers.
            let (cer, _step) = DkgCeremony::start(
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
                None,
            );

            let (cer, _step) = DkgCeremony::start(
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
            for k in &keys {
                let (cer, step) = DkgCeremony::start(ns, 2, committee.clone(), k.clone())
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
                None,
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

            // IN-WINDOW (just past the boundary): with the recompute-heal retention
            // (§8.11.1) a finalized epoch's logs stay servable for
            // JOURNAL_RETENTION_EPOCHS past the boundary — a demoted peer may still need
            // to recompute its share from them.
            actor.on_height(BOUNDARY + 1, &mut arng).await;
            assert!(
                actor.serve_log(&key).is_some(),
                "a finalized epoch's logs stay servable within the retention window past the boundary"
            );

            // PAST THE WINDOW: once the epoch ages out (now > E + JOURNAL_RETENTION_EPOCHS)
            // the serve index is reclaimed → the log is no longer served.
            let past_window =
                INTERVAL * (DETERMINISTIC_BOOTSTRAP_EPOCH + JOURNAL_RETENTION_EPOCHS + 1);
            actor.on_height(past_window, &mut arng).await;
            assert!(
                actor.serve_log(&key).is_none(),
                "the finalized-log serve index is reclaimed once the epoch ages out of the retention window"
            );
        });
    }

    /// The safety-critical half of the [`PinnedLogs`] contract on the PRODUCTION
    /// implementor: everything this node merely cannot answer is `Unavailable`,
    /// which parks the agreement's `verify`. `Unusable` is the one arm that can
    /// nullify a view for the whole network, so it must never stand in for a
    /// node-local gap.
    #[test]
    fn pinned_derive_answers_unavailable_for_every_node_local_gap() {
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
            let mut rng = StdRng::seed_from_u64(31);
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            let pinned: BTreeMap<u8, B256> =
                (0..3u8).map(|i| (i, B256::repeat_byte(0x50 + i))).collect();
            let (response, _reply) = tokio::sync::oneshot::channel();
            let request = PinnedRequest {
                epoch: 2,
                pinned,
                response,
            };
            let mut arng = StdRng::seed_from_u64(32);

            // A readable roster but no ceremony for the epoch — the common case for
            // a node that has not started dealing, or has already finalized.
            let readable =
                standalone_actor(&oracle, keys[0].clone(), committee.clone(), None).await;
            assert!(
                matches!(
                    readable.derive_pinned(&request, &mut arng),
                    PinnedDerive::Unavailable
                ),
                "no ceremony for the epoch must park the verify, not nullify the view"
            );

            // An unreadable roster — the documented transient EVM read race.
            let unreadable_cf: CommitteeFor = Arc::new(|_e: u64| None);
            let unreadable =
                standalone_actor_cf(&oracle, keys[1].clone(), unreadable_cf, None).await;
            assert!(
                matches!(
                    unreadable.derive_pinned(&request, &mut arng),
                    PinnedDerive::Unavailable
                ),
                "an unreadable roster must park the verify, not nullify the view"
            );
        });
    }

    /// The mailbox half of the same contract: an actor that is gone, and a reply
    /// that never comes, are both `Unavailable`.
    #[test]
    fn pinned_mailbox_answers_unavailable_when_the_actor_cannot_reply() {
        let runtime = deterministic::Runner::default();
        runtime.start(|_| async move {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let mailbox = PinnedMailbox::new(4, tx);
            drop(rx);
            assert!(
                matches!(
                    mailbox.derive(BTreeMap::new()).await,
                    PinnedDerive::Unavailable
                ),
                "a departed actor must park the verify"
            );

            let (tx, mut rx) = tokio::sync::mpsc::channel(1);
            let mailbox = PinnedMailbox::new(4, tx);
            let asked = mailbox.derive(BTreeMap::new());
            let served = async {
                let request = rx.recv().await.expect("request");
                assert_eq!(request.epoch, 4, "the mailbox names its own target epoch");
                drop(request.response); // the actor took the question and answered nothing
            };
            let (verdict, ()) = tokio::join!(asked, served);
            assert!(
                matches!(verdict, PinnedDerive::Unavailable),
                "a dropped reply must park the verify"
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
            None,
        )
    }

    /// The share-confirmation leg end to end on the actor: this node mints one when
    /// its body-checked set GROWS and only then, a peer's genuine confirmation is
    /// recorded, and everything else is dropped.
    ///
    /// The count of members that confirm they hold a usable set is the number that
    /// decides the epoch's fate, and it had no protocol representation at all before
    /// this. What it must never become is a number anyone can inflate: a seat's
    /// confirmation has to be signed by the member sitting in that seat, over that
    /// set, for that epoch.
    #[test]
    fn share_confirmations_are_minted_on_growth_and_taken_only_from_their_signer() {
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
            let mut rng = StdRng::seed_from_u64(0x3C);
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            let seat = |k: &Ed25519PrivateKey| {
                committee
                    .iter()
                    .position(|pk| *pk == k.public_key())
                    .expect("a committee seat") as u8
            };
            const TARGET: u64 = 6;
            let logs: Vec<(u8, B256)> =
                (0..3u8).map(|i| (i, B256::repeat_byte(0x80 + i))).collect();

            let pool = ConfirmPool::new(b"FLUENT_TEST_ACTOR");
            let recorded: DkgLogIndex = Arc::new(RwLock::new(BTreeMap::new()));
            let mut actor = standalone_actor(&oracle, keys[0].clone(), committee.clone(), None)
                .await
                .with_recorded_logs(recorded.clone())
                .with_share_confirms(pool.clone());

            // Nothing recorded is nothing to confirm: a confirmation of an empty set
            // would be a member claiming a coverage it does not have.
            assert!(actor.mint_confirmations().is_empty());

            recorded
                .write()
                .unwrap()
                .insert(TARGET, logs[..2].iter().copied().collect());
            let minted = actor.mint_confirmations();
            assert_eq!(minted.len(), 1, "a grown set must be confirmed");
            let DkgBody::Confirm(mine) = &minted[0].msg.body else {
                panic!("the minted message is not a confirmation");
            };
            assert_eq!(minted[0].msg.ceremony_epoch, TARGET);
            assert_eq!(mine.target_epoch, TARGET);
            assert_eq!(mine.idx, seat(&keys[0]));
            assert_eq!(mine.recorded, logs[..2].to_vec());
            assert!(mine.verify(pool.namespace(), &keys[0].public_key()));
            assert_eq!(pool.covering(TARGET, &logs[..2]).len(), 1);

            // Unchanged set, no re-mint: the statement only changes when a log is
            // recorded, so re-gossiping it every tick would be noise.
            assert!(actor.mint_confirmations().is_empty());

            // Grown again: a new, wider statement, and the pool keeps the wider one.
            recorded
                .write()
                .unwrap()
                .insert(TARGET, logs.iter().copied().collect());
            assert_eq!(actor.mint_confirmations().len(), 1);
            assert_eq!(pool.covering(TARGET, &logs).len(), 1);

            // A peer's genuine confirmation counts.
            let peer = ShareConfirm::sign(
                pool.namespace(),
                &keys[1],
                seat(&keys[1]),
                TARGET,
                logs.clone(),
            );
            actor.on_confirm(TARGET, &keys[1].public_key(), peer);
            assert_eq!(pool.covering(TARGET, &logs).len(), 2);

            // A confirmation for seat 2 that seat 2 did not sign does not.
            let outsider = Ed25519PrivateKey::random(&mut rng);
            let forged = ShareConfirm::sign(
                pool.namespace(),
                &outsider,
                seat(&keys[2]),
                TARGET,
                logs.clone(),
            );
            actor.on_confirm(TARGET, &keys[2].public_key(), forged);
            assert_eq!(
                pool.covering(TARGET, &logs).len(),
                2,
                "a seat's confirmation must be signed by the member in that seat"
            );

            // Nor does one whose unsigned framing disagrees with its signed epoch —
            // the framing is the half an attacker can rewrite.
            let genuine = ShareConfirm::sign(
                pool.namespace(),
                &keys[2],
                seat(&keys[2]),
                TARGET,
                logs.clone(),
            );
            actor.on_confirm(TARGET + 1, &keys[2].public_key(), genuine.clone());
            assert_eq!(pool.covering(TARGET, &logs).len(), 2);
            actor.on_confirm(TARGET, &keys[2].public_key(), genuine);
            assert_eq!(pool.covering(TARGET, &logs).len(), 3);

            // Per-target scratch: once the chain is past the target and no agreement
            // holds it, the confirmations go with the ceremony they describe.
            let mut arng = StdRng::seed_from_u64(0x3D);
            actor.on_height(INTERVAL * (TARGET + 1), &mut arng).await;
            assert!(
                pool.covering(TARGET, &logs).is_empty(),
                "the confirmation pool outlived the epoch it was about"
            );
        });
    }

    /// The plane's whole purpose is to keep agreeing while the ordering chain has
    /// HALTED, so nothing it needs may be retained on the finalized-height clock.
    /// The sweep here drops a ceremony the moment the chain enters its epoch; for
    /// the target of a RUNNING agreement instance that turns `derive_pinned` into a
    /// permanent `Unavailable` — every `verify` parks, `build_proposal` has nothing
    /// to pin, and a node that entered `E+1` before its plane converged could never
    /// agree `E+1` again.
    #[test]
    fn a_held_agreement_target_survives_the_chain_passing_its_epoch() {
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
            let mut rng = StdRng::seed_from_u64(0x5A);
            let keys: Vec<Ed25519PrivateKey> = (0..8)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            const TARGET: u64 = 5;
            let candidate = Set::from_iter_dedup(keys[..6].iter().map(|k| k.public_key()));
            let committed = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committed.clone()).await;
            let cand = candidate.clone();
            let candidate_for: CommitteeFor = Arc::new(move |_e| Some(cand.clone()));
            let committed_for: CommitteeFor =
                Arc::new(move |e| (e == TARGET - 1).then(|| committed.clone()));

            let targets = AgreementTargets::default();
            let (_pinned_tx, pinned_rx) = tokio::sync::mpsc::channel(4);
            let mut actor = standalone_actor_cf(&oracle, keys[0].clone(), candidate_for, None)
                .await
                .with_active_committee(committed_for)
                .with_pinned_requests(pinned_rx, targets.clone());

            let mut out = Vec::new();
            actor.maybe_start(TARGET, &mut out);
            assert!(
                actor.ceremonies.contains_key(&TARGET),
                "no ceremony to keep"
            );

            let ask = |epoch: u64| {
                let (response, _reply) = tokio::sync::oneshot::channel();
                PinnedRequest {
                    epoch,
                    pinned: (0..4u8).map(|i| (i, B256::repeat_byte(0x60 + i))).collect(),
                    response,
                }
            };
            let mut arng = StdRng::seed_from_u64(0x5B);

            // The chain reaches the target epoch and runs a whole epoch past it,
            // while the agreement for it is still running.
            let hold = targets.hold(TARGET);
            actor.on_height(INTERVAL * (TARGET + 1), &mut arng).await;
            assert!(
                actor.ceremonies.contains_key(&TARGET),
                "a held target's ceremony was swept by the height clock"
            );
            assert!(
                matches!(
                    actor.derive_pinned(&ask(TARGET), &mut arng),
                    PinnedDerive::Missing(_)
                ),
                "the plane can no longer answer for a target the chain has passed"
            );

            // The instance ends (delivered, or aborted below the frontier): the hold
            // goes with it and the ordinary sweep applies again.
            drop(hold);
            actor.on_height(INTERVAL * (TARGET + 2), &mut arng).await;
            assert!(
                !actor.ceremonies.contains_key(&TARGET),
                "the exemption outlived the instance that earned it"
            );
            assert!(matches!(
                actor.derive_pinned(&ask(TARGET), &mut arng),
                PinnedDerive::Unavailable
            ));
        });
    }

    /// A delivering instance hands its retention hold to this arm rather than
    /// letting it die with its own future — its `send` returns two hops short of
    /// here — so ONLY this arm can release it. It therefore has to claim the parked
    /// hold on every path, including the ones that adopt nothing: a hold it took
    /// for itself instead would leave the parked one owner-less and the ceremony it
    /// covers retained for the life of the process.
    #[test]
    fn on_artifact_releases_the_parked_hold_even_when_it_adopts_nothing() {
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
            let mut rng = StdRng::seed_from_u64(0x6D);
            const TARGET: u64 = 5;
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;
            let mut actor =
                standalone_actor(&oracle, keys[0].clone(), committee.clone(), None).await;

            let targets = actor.agreement_targets.clone();
            targets.hand_over(targets.hold(TARGET));
            assert!(
                targets.live().contains(&TARGET),
                "the instance's hold must survive the instance"
            );

            // Names no dealer logs, so there is nothing to pin and `on_artifact`
            // returns before it adopts anything.
            actor
                .on_artifact(agreed_artifact(TARGET, Vec::new()), &mut rng)
                .await;
            assert!(
                targets.live().is_empty(),
                "the parked hold outlived the arm that was supposed to release it"
            );
        });
    }

    /// v42 shrink regression: `maybe_start`'s change-test compares the COMMITTED roster
    /// of `target−1` (the live key-owning set) against the CANDIDATE roster of `target`
    /// (the pending set). A SHRINK whose candidate STASH is stable across consecutive
    /// epochs (`candidate(t−1) == candidate(t)`) must STILL start the ceremony — reading
    /// `cur` from the CANDIDATE reader made it look unchanged ⇒ no ceremony ⇒ `getDkgQual`
    /// stays empty ⇒ infinite deferral (v42: 10→8 shrink, zero dealing on every node for
    /// 3 boundaries). Distinct readers: committed(t−1)=10, candidate(t)=8 (stable).
    #[test]
    fn maybe_start_shrink_stable_candidate_stash_still_deals() {
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
            let mut rng = StdRng::seed_from_u64(0x42);
            let committed_keys: Vec<Ed25519PrivateKey> = (0..10)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            // Candidate = the first 8 (a genuine SHRINK from the committed 10).
            let me = committed_keys[0].clone();
            let committed_set = Set::from_iter_dedup(committed_keys.iter().map(|k| k.public_key()));
            let candidate_set =
                Set::from_iter_dedup(committed_keys[..8].iter().map(|k| k.public_key()));
            oracle.manager().track(0, committed_set.clone()).await;

            const TARGET: u64 = 5; // non-bootstrap
                                   // Candidate reader: the STABLE 8-set for EVERY epoch (the stash never changed).
            let cand = candidate_set.clone();
            let candidate_for: CommitteeFor = Arc::new(move |_e| Some(cand.clone()));
            // Committed reader: the 10-set at `target−1` (the live key holders).
            let committed_for: CommitteeFor =
                Arc::new(move |e| (e == TARGET - 1).then(|| committed_set.clone()));

            let mut actor = standalone_actor_cf(&oracle, me, candidate_for, None)
                .await
                .with_active_committee(committed_for);
            let mut out = Vec::new();
            actor.maybe_start(TARGET, &mut out);
            assert!(
                actor.ceremonies.contains_key(&TARGET),
                "shrink with a stable candidate stash MUST deal: committed(t−1)=10 ≠ candidate(t)=8"
            );
        });
    }

    /// Carry-forward control: a genuine NO-CHANGE epoch (`committed(t−1) == candidate(t)`)
    /// must NOT start a ceremony — the key carries forward. Guards the change-test from
    /// firing spuriously after the v42 two-reader split.
    #[test]
    fn maybe_start_no_change_carries_forward() {
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
            let mut rng = StdRng::seed_from_u64(0x77);
            let keys: Vec<Ed25519PrivateKey> = (0..6)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let me = keys[0].clone();
            let set = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, set.clone()).await;

            const TARGET: u64 = 5; // non-bootstrap
            let cand = set.clone();
            let candidate_for: CommitteeFor = Arc::new(move |_e| Some(cand.clone()));
            let comm = set.clone();
            let committed_for: CommitteeFor = Arc::new(move |_e| Some(comm.clone()));

            let mut actor = standalone_actor_cf(&oracle, me, candidate_for, None)
                .await
                .with_active_committee(committed_for);
            let mut out = Vec::new();
            actor.maybe_start(TARGET, &mut out);
            assert!(
                !actor.ceremonies.contains_key(&TARGET),
                "no-change (committed(t−1) == candidate(t)) ⇒ carry-forward, no ceremony"
            );
        });
    }

    /// Post-restart serve from a COLD cache (R1) + post-restart journal eviction (R2) +
    /// the cold-cache fetch-burst bound (e). All three are standalone (no network): a
    /// fresh actor whose `serve_cache` is empty but whose epoch-2 journal is present on
    /// disk must serve from the journal (cold-miss parse, R1); a first `on_height` past
    /// the boundary must reconcile-delete the journal so the serve then returns `None`
    /// (R2); and M ≫ K serve calls across K cold epochs must parse exactly K times (e).
    #[test]
    fn post_restart_serve_evict_and_burst_bound() {
        let _guard = COLD_PARSE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;

            let dir = fresh_share_dir("cold-serve");
            std::fs::create_dir_all(&dir).expect("mkdir");
            // Write epoch-2's full committee logs to the on-disk journal (the durable
            // source a restart re-reads). Tag node-0's own log as OwnSeal, peers as
            // PeerLog — exactly what the live actor would have journaled pre-finalize.
            let logs = mint_committee_logs_at(&keys, &committee, 2);
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
            let mut actor = standalone_actor(
                &oracle,
                keys[0].clone(),
                committee.clone(),
                Some(dir.clone()),
            )
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

            // (d) R2: a first `on_height` landing PAST the epoch-2 RETENTION WINDOW
            // (§8.11.1) runs the first-tick reconcile, deleting the aged-out journal; the
            // serve then returns `None`. (Within the window the journal is retained for
            // the demote-heal recompute — the earlier serve above still worked.)
            let mut arng = StdRng::seed_from_u64(9);
            let past_window =
                INTERVAL * (DETERMINISTIC_BOOTSTRAP_EPOCH + JOURNAL_RETENTION_EPOCHS + 1);
            actor.on_height(past_window, &mut arng).await;
            assert!(
                !journal_path(&dir, 2).exists(),
                "the aged-out journal is reconcile-deleted after a restart (R2)"
            );
            assert!(
                actor.serve_log(&peer_key).is_none(),
                "an aged-out epoch is no longer served (journal gone + cache evicted)"
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
        let _guard = COLD_PARSE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
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
                let logs = mint_committee_logs_at(&keys, &committee, e);
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
            let mut actor = standalone_actor(
                &oracle,
                keys[0].clone(),
                committee.clone(),
                Some(dir.clone()),
            )
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
        let keys: Vec<Ed25519PrivateKey> = (0..N)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
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
        let dir = fresh_share_dir(if corrupt_own_seal {
            "res-torn"
        } else {
            "res-plain"
        });
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
                oracle
                    .add_link(keys[0].public_key(), keys[j].public_key(), link())
                    .await
                    .expect("link");
                oracle
                    .add_link(keys[j].public_key(), keys[0].public_key(), link())
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
        assert!(
            corrupted,
            "an OwnSeal record must exist in the victim's journal"
        );
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

    /// Mint every committee member's sealed log at an explicit `epoch` (the seal `Info`
    /// embeds the epoch, so a log minted at epoch E only `check`s under the epoch-E
    /// `Info`). Deterministic given `keys` — the dealer polynomial is seeded from each
    /// key + epoch (`ceremony::dealer_seed_rng`), no external rng needed.
    fn mint_committee_logs_at(
        keys: &[Ed25519PrivateKey],
        committee: &Set<PeerPubkey>,
        epoch: u64,
    ) -> Vec<(PeerPubkey, DealerReveal)> {
        let ns = b"FLUENT_DPOS_V1_clocktest";
        let mut cers: BTreeMap<PeerPubkey, DkgCeremony> = BTreeMap::new();
        let mut queue: Vec<(PeerPubkey, Outgoing)> = Vec::new();
        for k in keys {
            let (cer, step) =
                DkgCeremony::start(ns, epoch, committee.clone(), k.clone()).expect("start");
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
                        let more = cers
                            .get_mut(&to)
                            .unwrap()
                            .handle(from.clone(), o.msg.body.clone());
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
        let keys: Vec<Ed25519PrivateKey> = (0..4)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let pk0 = keys[0].public_key();

        let mut cers: BTreeMap<PeerPubkey, DkgCeremony> = BTreeMap::new();
        let mut queue: Vec<(PeerPubkey, Outgoing)> = Vec::new();
        let mut journal0: Vec<JournalRecord> = Vec::new();
        for k in &keys {
            let (cer, step) =
                DkgCeremony::start(ns, 2, committee.clone(), k.clone()).expect("start");
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
                            let more = cers
                                .get_mut(&to)
                                .unwrap()
                                .handle(from.clone(), o.msg.body.clone());
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

    /// A PRE-deadline resume RECONSTRUCTS a live dealer, so it must NOT finalize until it
    /// SEALS at the deadline via `on_height` step 1 — then it finalizes WITH its own
    /// freshly-sealed log. This inverts the old player-only premise: the finalize gate
    /// stays `dealing_closed()`, and the reconstructed dealer keeps that gate shut until
    /// the seal (a live dealer whose log recovers the share, not a shareless sit-out).
    #[test]
    fn pre_deadline_resume_seals_via_on_height_then_finalizes() {
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
            let (committee, key0, mut journal) = node0_pre_seal_journal(21);
            // Drop ONE peer log so node-0 seals SHORTHANDED (own + 2 peers = the 3-log
            // quorum), forcing its OWN freshly-sealed log into the finalized set AND
            // keeping it under the all-in count so the seal + finalize are decoupled.
            let idx = journal
                .iter()
                .position(|r| matches!(r, JournalRecord::PeerLog(_)))
                .expect("a peer log to drop");
            journal.remove(idx);
            oracle.manager().track(0, committee.clone()).await;
            let me0 = key0.public_key();

            // Pre-deadline ⇒ reconstruct the seeded dealer (LIVE). Own log NOT yet recorded.
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                true,
            )
            .expect("pre-deadline resume");
            assert!(
                !resumed.ceremony.dealing_closed(),
                "a pre-deadline resume keeps a LIVE dealer (reconstructed)"
            );
            assert!(
                !resumed.ceremony.own_log_recorded(&me0),
                "own log is not recorded until it seals at the deadline"
            );

            let store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut actor = standalone_actor(&oracle, key0, committee, None).await;
            actor.store = store.clone();
            actor
                .ceremonies
                .insert(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);

            // Before the seal deadline the live-dealer ceremony must NOT finalize (the
            // `dealing_closed` gate stays shut).
            let mut arng = StdRng::seed_from_u64(9);
            actor.drive_finalization(SEAL_DEADLINE - 1, &mut arng);
            assert!(
                store.read().map(|s| s.is_empty()).unwrap_or(false),
                "a reconstructed live-dealer ceremony does not finalize before its seal"
            );

            // `on_height` step 1 seals it at the deadline; shorthanded ⇒ it does NOT
            // finalize on this tick, so we can observe that the seal recorded its OWN log.
            actor.on_height(SEAL_DEADLINE, &mut arng).await;
            let c = actor
                .ceremonies
                .get(&DETERMINISTIC_BOOTSTRAP_EPOCH)
                .expect("ceremony still present (shorthanded — not yet finalized)");
            assert!(
                c.dealing_closed(),
                "on_height step 1 sealed the reconstructed dealer"
            );
            assert!(
                c.own_log_recorded(&me0),
                "the seal recorded node-0's OWN freshly-sealed log (finalizes WITH its own log)"
            );

            // At the settle deadline it finalizes over the settled 3-log quorum.
            actor
                .on_height(SEAL_DEADLINE + DKG_SETTLE_BLOCKS + 1, &mut arng)
                .await;
            assert!(
                store
                    .read()
                    .map(|s| s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
                    .unwrap_or(false),
                "the reconstructed dealer seals via on_height step 1 and finalizes"
            );
        });
    }

    /// R2-L guard: a POST-deadline resume whose journal LACKS a valid OwnSeal (a torn
    /// tail / corrupt own-seal) must stay PLAYER-ONLY — it must NOT reconstruct a dealer
    /// and re-seal a possibly-divergent second log. It re-emits peer acks + recovers its
    /// share as a player; the only outgoing are Acks (no Reveal = no re-seal, no
    /// Commitment/Share = no re-deal).
    #[test]
    fn torn_own_seal_post_deadline_refetches_not_reseals() {
        // A pre-seal journal (no OwnSeal) resumed AT/AFTER the seal deadline models the
        // torn-own-seal state (a torn tail dropped the OwnSeal).
        let (committee, key0, pre_seal) = node0_pre_seal_journal(29);
        let me0 = key0.public_key();

        let resumed = DkgCeremony::resume(
            b"FLUENT_DPOS_V1_clocktest",
            DETERMINISTIC_BOOTSTRAP_EPOCH,
            committee,
            key0,
            pre_seal,
            false, // reconstruct_dealer = false (resumed at/after the seal deadline)
        )
        .expect("post-deadline resume");

        assert!(
            resumed.ceremony.dealing_closed(),
            "a post-deadline torn-own-seal resume is PLAYER-ONLY (no live dealer — never re-seals)"
        );
        assert!(
            !resumed.ceremony.own_log_recorded(&me0),
            "no valid OwnSeal ⇒ our own log is NOT recorded (never re-derived)"
        );
        assert!(
            resumed.ceremony.retransmit().is_empty(),
            "player-only ⇒ no dealer ⇒ retransmit is a no-op (nothing re-dealt)"
        );
        assert!(
            resumed
                .outgoing
                .iter()
                .all(|o| matches!(o.msg.body, DkgBody::Ack(_))),
            "the only outgoing are re-emitted peer acks — NO Reveal (no re-seal) and NO \
             Commitment/Share (no re-deal)"
        );
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
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;

            // A freshly STARTED ceremony (dealer still Some) — never sealed.
            let (cer, _step) = DkgCeremony::start(
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
                // Player-only (post-deadline): `dealing_closed()` so the ceremony reaches
                // the destructive finalize this test exercises (a reconstructed live
                // dealer would keep the finalize gate shut instead).
                false,
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
        let keys: Vec<Ed25519PrivateKey> = (0..4)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let pk0 = keys[0].public_key();
        let mut cers: BTreeMap<PeerPubkey, DkgCeremony> = BTreeMap::new();
        let mut queue: Vec<(PeerPubkey, Outgoing)> = Vec::new();
        let mut journal0: Vec<JournalRecord> = Vec::new();
        for k in &keys {
            let (cer, step) =
                DkgCeremony::start(ns, 2, committee.clone(), k.clone()).expect("start");
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
                            let more = cers
                                .get_mut(&to)
                                .unwrap()
                                .handle(from.clone(), o.msg.body.clone());
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

    /// P1 1a (RED→GREEN): a finalized epoch's journal + `serve_cache` SURVIVE the epoch
    /// boundary for the recompute-heal retention window, and are evicted only once the
    /// epoch ages OUT (`now > E + JOURNAL_RETENTION_EPOCHS`). Pre-fix the boundary sweep
    /// deleted them the instant `now == E`, leaving a caught-up member nothing to
    /// recompute from.
    #[test]
    fn journal_survives_epoch_boundary_for_retained_window() {
        // serve_log cold-loads → touches the process-global COLD_PARSE_COUNT.
        let _guard = COLD_PARSE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;

            // node-0's on-disk journal for epoch 2 (full committee logs).
            let dir = fresh_share_dir("retain-window");
            std::fs::create_dir_all(&dir).expect("mkdir");
            let logs = mint_committee_logs_at(&keys, &committee, 2);
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

            let mut actor =
                standalone_actor(&oracle, keys[0].clone(), committee, Some(dir.clone())).await;
            let key = DkgLogKey {
                epoch: 2,
                dealer: keys[1].public_key(),
            };
            assert!(
                actor.serve_log(&key).is_some(),
                "precondition: epoch-2 log servable"
            );

            // Cross the boundary but stay IN the retention window (now == E == 2).
            let mut arng = StdRng::seed_from_u64(9);
            actor.on_height(BOUNDARY + 1, &mut arng).await; // now = 2
            assert!(
                actor.serve_log(&key).is_some(),
                "journal + serve_cache are RETAINED across the boundary within the window"
            );
            assert!(
                journal_path(&dir, 2).exists(),
                "the journal file survives the boundary within the retention window"
            );

            // Age OUT (now > E + JOURNAL_RETENTION_EPOCHS).
            let past = INTERVAL * (DETERMINISTIC_BOOTSTRAP_EPOCH + JOURNAL_RETENTION_EPOCHS + 1);
            actor.on_height(past, &mut arng).await; // now = 4
            assert!(
                actor.serve_log(&key).is_none(),
                "evicted once the epoch ages out of the retention window"
            );
            assert!(
                !journal_path(&dir, 2).exists(),
                "journal reclaimed once the epoch ages out of the retention window"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// P1 1c (RED→GREEN): a demote-heal in progress keeps issuing a TARGETED fetch for a
    /// pinned `dealers()` log it lacks even PAST the epoch boundary (the in-window
    /// ceremony fetch stops at `epoch_start`; `recompute_pending` does not). Pre-fix the
    /// only fetch driver was `self.ceremonies`, gated off past the boundary.
    #[test]
    fn recompute_pending_fetches_missing_dealer_past_boundary() {
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
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;
            let (outcome, _shares) = crate::beacon::dkg_oracle::run_local_dkg(
                &mut rng,
                b"FLUENT_DPOS_V1_clocktest",
                2,
                &keys,
                &keys,
            )
            .expect("dkg");
            let missing = outcome.dealers().iter().next().cloned().expect("a dealer");

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
                None,
            );
            // A demote-heal already in flight for epoch 2, still wanting `missing`.
            actor.recompute_pending.insert(
                2,
                RecomputeState {
                    outcome,
                    want: BTreeSet::from([missing.clone()]),
                },
            );

            // Drive the fetch at a height PAST the epoch-2 boundary (epoch_start(2)=20).
            actor.fetch_missing_logs(BOUNDARY + 5).await;
            assert!(
                in_flight.lock().unwrap().contains(&DkgLogKey {
                    epoch: 2,
                    dealer: missing,
                }),
                "recompute_pending keeps fetching the missing pinned dealer PAST the boundary"
            );
        });
    }

    /// P2 2a (RED→GREEN) — the CORE heal. node-0 is a demoted committee[2] member (no
    /// share; a peer dealer's log missing from its journal). The actor SELF-DETECTS the
    /// demote from its OWN `on_height` loop (via a mock `outcome_at` returning the pinned
    /// Output — no epoch_manager signal), fetches the missing pinned-`dealers()` log
    /// (delivered here via the resolver ingest), recomputes its share scoped to
    /// `dealers()`, self-verifies it against the pinned Output, and adopts it —
    /// reproducing its CANONICAL share and firing the promote path (dkg_ceremony_ok).
    #[test]
    fn demoted_member_recomputes_share_and_heals() {
        // drive_recompute cold-loads the journal → touches the process-global
        // COLD_PARSE_COUNT the burst-bound tests assert on.
        let _guard = COLD_PARSE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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
            // node-0's real journal for epoch 2 + an identical copy to derive the pinned
            // Output + node-0's CANONICAL share.
            let (committee, key0, full_journal) = node0_pre_seal_journal_full_sealed(41);
            let (_c2, _k2, journal_for_outcome) = node0_pre_seal_journal_full_sealed(41);
            oracle.manager().track(0, committee.clone()).await;
            let me0 = key0.public_key();

            let mut frng = StdRng::seed_from_u64(1);
            let mut canon = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal_for_outcome,
                false, // player-only: derive the canonical outcome/share from a sealed journal
            )
            .expect("resume");
            let (outcome, canonical_share) = canon.ceremony.finalize(&mut frng).expect("finalize");
            let outcome_bytes = crate::beacon::outcome::encode_outcome(&outcome);
            let dealers: BTreeSet<PeerPubkey> = outcome.dealers().iter().cloned().collect();

            // Write node-0's journal to disk MINUS one pinned-dealer peer log (the demote:
            // it holds < dealers()), holding that log back to DELIVER via the resolver.
            let info = info_for_test(&committee);
            let dir = fresh_share_dir("recompute-heal");
            std::fs::create_dir_all(&dir).expect("mkdir");
            let mut held_back: Option<(PeerPubkey, DealerReveal)> = None;
            for r in full_journal {
                match r {
                    JournalRecord::PeerLog(signed) => {
                        let pk = (*signed).clone().check(&info).map(|(pk, _)| pk);
                        match pk {
                            Some(pk)
                                if pk != me0
                                    && dealers.iter().any(|d| *d == pk)
                                    && held_back.is_none() =>
                            {
                                held_back = Some((pk, *signed)); // deliver later
                            }
                            _ => share_state::append_journal(
                                &dir,
                                2,
                                &JournalRecord::PeerLog(signed),
                                &ShareState::Plaintext,
                            )
                            .expect("append"),
                        }
                    }
                    other => share_state::append_journal(&dir, 2, &other, &ShareState::Plaintext)
                        .expect("append"),
                }
            }
            let (held_dealer, held_log) = held_back.expect("a held-back pinned peer dealer");

            let mut actor =
                standalone_actor(&oracle, key0.clone(), committee.clone(), Some(dir.clone())).await;
            // Mock artifact reader: return the agreed Output for epoch 2 (a change
            // epoch), None elsewhere. Re-parse from bytes each call (Output is not Clone).
            let outcome_at: AgreedOutcomeAt = {
                let bytes = outcome_bytes.clone();
                Arc::new(move |epoch: u64| {
                    let bytes = bytes.clone();
                    Box::pin(async move {
                        (epoch == 2)
                            .then(|| crate::beacon::outcome::parse_outcome(&bytes).ok())
                            .flatten()
                    })
                })
            };
            actor.outcome_at = Some(outcome_at);

            // now == 2 (height = epoch_start(2)). The actor self-detects the demote.
            let mut arng = StdRng::seed_from_u64(9);
            actor.on_height(BOUNDARY, &mut arng).await;
            assert!(
                actor.recompute_pending.contains_key(&2),
                "the actor self-detected the demote and began recomputing"
            );
            assert!(
                actor.store.read().unwrap().get(&2).is_none(),
                "not yet healed — a pinned dealer log is still missing"
            );
            assert_eq!(
                actor.metrics.dkg_ceremony_ok.get(),
                0,
                "no share adopted before the missing log arrives"
            );

            // Deliver the held-back pinned-dealer log via the resolver ingest.
            let key = DkgLogKey {
                epoch: 2,
                dealer: held_dealer.clone(),
            };
            let accepted = actor.ingest_log(&key, held_log.encode(), &mut arng).await;
            assert!(accepted, "the delivered pinned-dealer log is accepted");

            // HEALED: store[2] holds node-0's CANONICAL share; recompute_pending cleared;
            // the promote-driving metric incremented.
            let stored = actor
                .store
                .read()
                .unwrap()
                .get(&2)
                .map(|(_o, s)| s.encode().to_vec());
            assert_eq!(
                stored,
                Some(canonical_share.encode().to_vec()),
                "the demoted member recomputed its CANONICAL share from the retained journal"
            );
            assert!(
                !actor.recompute_pending.contains_key(&2),
                "recompute_pending is cleared once the share is adopted"
            );
            assert_eq!(
                actor.metrics.dkg_ceremony_ok.get(),
                1,
                "the heal fires dkg_ceremony_ok (the promote-driving signal)"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// P3 3d (RED→GREEN, step 1f): with EVERY `append_journal` failing (a FILE stands in
    /// for the share_dir), node-0's ack to each peer dealer is WITHHELD, so every peer
    /// reveals node-0's point → node-0 lands in the pinned `Output.revealed()` set AND
    /// still finalizes its share (the "share is reconstructable" branch — recoverable via
    /// those reveals). A QUAL log can never record an ack node-0 cannot back with a
    /// durable view. Control: a WORKING dir → node-0 acks everyone → node-0 ∉ revealed.
    #[test]
    fn acked_dealing_withheld_on_append_failure_still_recoverable() {
        let runtime = deterministic::Runner::default();
        let (seeded_fail, revealed_fail) = runtime.start(|ctx| async move {
            let bad = fresh_share_dir("append-fail");
            std::fs::write(&bad, b"not a dir").expect("write file");
            run_reveal_check(ctx, Some(bad), 5).await
        });
        assert!(
            seeded_fail,
            "node-0 still finalizes its share (reconstructable via reveals) despite withheld acks"
        );
        assert!(
            revealed_fail,
            "append-failed acks are WITHHELD → every dealer reveals node-0 → node-0 ∈ revealed()"
        );

        let runtime = deterministic::Runner::default();
        let (seeded_ok, revealed_ok) = runtime.start(|ctx| async move {
            let good = fresh_share_dir("append-ok");
            run_reveal_check(ctx, Some(good), 5).await
        });
        assert!(seeded_ok, "control: node-0 finalizes");
        assert!(
            !revealed_ok,
            "control: durable acks land → node-0 is NOT revealed (the ack was broadcast)"
        );
    }

    /// Drive the 4-dealer committee[2] DKG over the sim with node-0 (the victim) using
    /// `victim_dir` as its share_dir; feed heights to finalize. Returns
    /// `(seeded, victim_in_revealed)` read from node-0's memoized Output.
    async fn run_reveal_check(
        ctx: SimContext,
        victim_dir: Option<PathBuf>,
        seed: u64,
    ) -> (bool, bool) {
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
        let mut rng = StdRng::seed_from_u64(seed);
        let keys: Vec<Ed25519PrivateKey> = (0..4)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
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
        let me0 = keys[0].public_key();
        let victim_store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
        let mut sinks = Vec::new();
        for (i, k) in keys.iter().enumerate() {
            let store = if i == 0 {
                victim_store.clone()
            } else {
                Arc::new(RwLock::new(BTreeMap::new()))
            };
            let dir = if i == 0 { victim_dir.clone() } else { None };
            sinks.push(
                spawn_dealer_at(
                    &ctx,
                    &oracle,
                    k.clone(),
                    committee.clone(),
                    store,
                    Arc::new(tokio::sync::Notify::new()),
                    INTERVAL,
                    dir,
                    7,
                )
                .await,
            );
        }
        for h in 0..=(BOUNDARY - 1) {
            for s in &sinks {
                let _ = s.send(h).await;
            }
            ctx.sleep(Duration::from_millis(50)).await;
        }
        let guard = victim_store.read().unwrap();
        match guard.get(&DETERMINISTIC_BOOTSTRAP_EPOCH) {
            Some((outcome, _share)) => (true, outcome.revealed().iter().any(|p| *p == me0)),
            None => (false, false),
        }
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

    /// A quorum-certified artifact over `logs`, built the way the agreement plane
    /// builds one. The write-back reads only the target epoch and the pinned set —
    /// the certificate is verified by the plane's own instance and by the pull
    /// seam before either hands one over — so the committee here is a fresh set
    /// whose only job is to make a real `Finalization` constructible.
    fn agreed_artifact(target_epoch: u64, logs: Vec<(u8, B256)>) -> AgreedArtifact {
        agreed_artifact_with_committee(target_epoch, logs).0
    }

    /// [`agreed_artifact`] plus the committee its certificate was signed by — what
    /// a pull-seam bridge needs to VERIFY the same artifact.
    fn agreed_artifact_with_committee(
        target_epoch: u64,
        logs: Vec<(u8, B256)>,
    ) -> (AgreedArtifact, fluentbase_bls::EpochCommittee) {
        use commonware_codec::DecodeExt as _;
        use commonware_consensus::{
            simplex::types::{Finalization, Finalize, Proposal},
            types::{Epoch, Round, View},
        };
        use commonware_cryptography::bls12381::{
            dkg::deal,
            primitives::{sharing::Mode, variant::MinSig},
        };
        use commonware_parallel::Sequential;
        use commonware_utils::{ordered::BiMap, N3f1, TryCollect as _};
        use fluentbase_bls::{
            beacon::dkg_namespace,
            fluent_namespace,
            keys::ValidatorBlsKeypair,
            scheme::{build_signer, build_verifier},
            BlsPubkey,
        };

        let mut rng = StdRng::seed_from_u64(0xA27);
        let peers: Vec<Ed25519PrivateKey> = (0..4)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let bls: Vec<ValidatorBlsKeypair> = (0..4)
            .map(|_| ValidatorBlsKeypair::generate(&mut rng))
            .collect();
        let bimap: BiMap<PeerPubkey, BlsPubkey> = peers
            .iter()
            .zip(bls.iter())
            .map(|(p, b)| {
                (
                    p.public_key(),
                    BlsPubkey::decode(b.public_bytes().as_slice()).expect("bls pubkey"),
                )
            })
            .try_collect()
            .expect("unique committee");
        let group_key = deal::<MinSig, PeerPubkey, N3f1>(
            &mut rng,
            Mode::NonZeroCounter,
            Set::from_iter_dedup(peers.iter().map(|p| p.public_key())),
        )
        .expect("deal")
        .0;

        let proposal = crate::beacon::dkg_agree::DkgProposal {
            target_epoch,
            logs,
            group_key,
            confirms: Vec::new(),
        };
        let ns = dkg_namespace(&fluent_namespace(AGREEMENT_CHAIN_ID));
        let round = Round::new(Epoch::new(target_epoch), View::new(1));
        let payload = Proposal::new(round, View::new(0), proposal.digest());
        let finalizes: Vec<_> = bls
            .iter()
            .take(3)
            .map(|kp| {
                let signer = build_signer(&ns, bimap.clone(), kp, None).expect("member");
                Finalize::sign(&signer, payload.clone()).expect("sign")
            })
            .collect();
        let certificate = Finalization::from_finalizes(
            &build_verifier(&ns, bimap.clone(), None, None),
            finalizes.iter(),
            &Sequential,
        )
        .expect("quorum");
        (
            (proposal, certificate),
            fluentbase_bls::EpochCommittee {
                epoch: target_epoch,
                bimap,
            },
        )
    }

    /// The pinned set an artifact for this ceremony would name: every dealer log
    /// node-0 holds, at its committee index.
    fn pinned_logs_of(
        actor: &DkgActor<
            impl Sender<PublicKey = PeerPubkey>,
            impl Receiver<PublicKey = PeerPubkey>,
            impl Resolver<Key = DkgLogKey, PublicKey = PeerPubkey>,
        >,
        epoch: u64,
        committee: &Set<PeerPubkey>,
    ) -> Vec<(u8, B256)> {
        let c = actor.ceremonies.get(&epoch).expect("ceremony");
        committee
            .iter()
            .enumerate()
            .filter_map(|(i, pk)| c.signed_log_hash(pk).map(|h| (i as u8, h)))
            .collect()
    }

    /// THE halted-chain recovery, which is the reason the agreement plane exists.
    ///
    /// The chain has stopped at `epoch_start(E+1)`: every block of `E` is
    /// finalized, no block of `E+1` exists and none can be produced, so the
    /// finalized-consensus dealer-log set that AM5 finalizes over NEVER arrives and
    /// the height clock never ticks again. Delivering the agreed artifact must be
    /// enough on its own to produce `(PK_{E+1}, share)` — and the two things the
    /// `E+1` spawn actually consumes are asserted directly: the share-gate resolver
    /// answers `Key` for `E+1`, and `share_notify` carries the permit the epoch
    /// manager's respawn edge wakes on.
    #[test]
    fn an_agreed_artifact_recovers_the_epoch_with_the_chain_halted() {
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
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(63);
            oracle.manager().track(0, committee.clone()).await;
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                false,
            )
            .expect("post-deadline resume");

            let store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let share_notify = Arc::new(tokio::sync::Notify::new());
            let mut actor = standalone_actor(&oracle, key0, committee.clone(), None).await;
            actor.store = store.clone();
            actor.share_notify = share_notify.clone();
            // The agreement plane is WIRED, which is what bars the legacy
            // local-settled finalize: with a plane running, only its certified set
            // may mint, so an epoch whose artifact has not landed waits instead of
            // finalizing over whatever this node happens to hold.
            let (agree_tx, _agree_rx) = tokio::sync::mpsc::channel(4);
            let (_artifacts_tx, artifacts_rx) = tokio::sync::mpsc::channel(4);
            actor = actor
                .with_recorded_logs(Arc::new(RwLock::new(BTreeMap::new())))
                .with_agreement_plane(agree_tx, artifacts_rx);
            actor
                .ceremonies
                .insert(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);

            // The last block of epoch E. Past the settle deadline, every body held,
            // and still nothing to finalize over — this is the wedge.
            let mut rng = StdRng::seed_from_u64(11);
            actor.on_height(BOUNDARY - 1, &mut rng).await;
            assert!(
                store.read().map(|s| s.is_empty()).unwrap_or(false),
                "past the settle deadline with every body held and no agreed set, \
                 the epoch cannot mint — the halt"
            );

            let logs = pinned_logs_of(&actor, DETERMINISTIC_BOOTSTRAP_EPOCH, &committee);
            assert_eq!(logs.len(), 4, "node-0 holds every dealer log");
            // No further height tick from here on: the artifact edge is the only
            // thing that runs.
            actor
                .on_artifact(
                    agreed_artifact(DETERMINISTIC_BOOTSTRAP_EPOCH, logs),
                    &mut rng,
                )
                .await;

            assert!(
                store
                    .read()
                    .map(|s| s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
                    .unwrap_or(false),
                "the artifact's dealer-log set is the pinned set; the existing \
                 finalize rails do the rest"
            );
            // Blocker 2 of the write-back: the share-gate wants a LOCAL share at
            // E+1, and this is the resolver the epoch manager asks.
            let dkg_qual: crate::beacon::carry::DkgQualFor =
                Arc::new(|e: u64| Some(e == DETERMINISTIC_BOOTSTRAP_EPOCH));
            let resolve = crate::dpos::beacon_share_resolver(
                store.clone(),
                dkg_qual,
                b"ns".to_vec(),
                crate::beacon::keys::BeaconKeys::new(),
            );
            assert!(
                matches!(
                    resolve(DETERMINISTIC_BOOTSTRAP_EPOCH),
                    crate::epoch_manager::BeaconResolve::Key(_)
                ),
                "the E+1 share-gate must pass with no block of E+1 in existence"
            );
            // Blocker 1: the respawn edge is `share_notify`, and its permit
            // survives having had no waiter armed when it fired.
            assert!(
                futures::FutureExt::now_or_never(Box::pin(share_notify.notified())).is_some(),
                "the write-back must fire the edge the epoch manager respawns on"
            );
            // The write-back is complete, so its retention hold is released rather
            // than pinning the ceremony open for the life of the process.
            assert!(
                actor.agreement_targets.live().is_empty(),
                "a completed write-back releases its ceremony-retention hold"
            );
        });
    }

    /// An artifact this node only ever PULLED has to end in a finalized share, not
    /// just a verifiable key.
    ///
    /// The reachable shape: this member's own agreement instance died mid-agreement
    /// while the rest of `committee[E+1]` decided without it. Its store is empty for
    /// the epoch, so the startup replay finds nothing; its relaunched instance draws
    /// no votes because every peer's launcher already holds the target in `started`.
    /// The pull is its ONLY source — and the store the pull files into is read by
    /// the `PK_epoch` rungs and the serve path, none of which reach `on_artifact`.
    /// Without the seam's push into the write-back the node verifies the epoch key
    /// and stays permanently shareless for the epoch it was elected to sign in.
    #[test]
    fn an_artifact_that_arrives_only_by_pull_still_mints_the_share() {
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
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(64);
            oracle.manager().track(0, committee.clone()).await;
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                false,
            )
            .expect("post-deadline resume");

            let store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut actor = standalone_actor(&oracle, key0, committee.clone(), None).await;
            actor.store = store.clone();
            let (agree_tx, _agree_rx) = tokio::sync::mpsc::channel(4);
            let (_artifacts_tx, artifacts_rx) = tokio::sync::mpsc::channel(4);
            actor = actor
                .with_recorded_logs(Arc::new(RwLock::new(BTreeMap::new())))
                .with_agreement_plane(agree_tx, artifacts_rx);
            actor
                .ceremonies
                .insert(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);

            let mut rng = StdRng::seed_from_u64(12);
            actor.on_height(BOUNDARY - 1, &mut rng).await;
            assert!(
                store.read().map(|s| s.is_empty()).unwrap_or(false),
                "no local instance decided, so nothing may mint yet"
            );

            let logs = pinned_logs_of(&actor, DETERMINISTIC_BOOTSTRAP_EPOCH, &committee);
            let (artifact, artifact_committee) =
                agreed_artifact_with_committee(DETERMINISTIC_BOOTSTRAP_EPOCH, logs);
            let committee_source: crate::beacon::artifact::CommitteeSource = Arc::new(move |e| {
                (e == DETERMINISTIC_BOOTSTRAP_EPOCH).then(|| artifact_committee.clone())
            });

            // The peer that decided without this node.
            let served = crate::beacon::artifact::ArtifactStore::new();
            assert!(served.insert(DETERMINISTIC_BOOTSTRAP_EPOCH, artifact));
            let (peer_adopt_tx, _peer_adopt_rx) = tokio::sync::mpsc::channel(4);
            let serving = crate::beacon::artifact::ArtifactBridge::new(
                AGREEMENT_CHAIN_ID,
                served,
                committee_source.clone(),
                peer_adopt_tx,
                crate::beacon::metrics::BeaconMetrics::default(),
            );

            // This node: an empty store and the write-back's own inbound channel.
            let (adopt_tx, mut adopt_rx) = tokio::sync::mpsc::channel(4);
            let fetching = crate::beacon::artifact::ArtifactBridge::new(
                AGREEMENT_CHAIN_ID,
                crate::beacon::artifact::ArtifactStore::new(),
                committee_source,
                adopt_tx,
                crate::beacon::metrics::BeaconMetrics::default(),
            );
            assert!(fetching.deliver(
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                serving.produce(DETERMINISTIC_BOOTSTRAP_EPOCH).as_ref()
            ));

            // The actor's own `artifacts_rx` arm, driven by hand: in the node the
            // write-back sits between the two and republishes `PK_epoch` on the way.
            let pulled = adopt_rx
                .try_recv()
                .expect("a pulled artifact must reach the agreement write-back");
            actor.on_artifact(pulled, &mut rng).await;

            assert!(
                store
                    .read()
                    .map(|s| s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
                    .unwrap_or(false),
                "a pull-only artifact must leave this node holding the epoch's share"
            );
        });
    }

    /// The artifact edge drives the RECOVERY too, not just the finalize.
    ///
    /// A member missing a pinned body cannot finalize over the agreed set, and with
    /// the chain halted `on_height` will never fetch it again. The arrival edge has
    /// to issue the fetch itself, and it must keep issuing it past the target's own
    /// boundary — the height gate that normally stops there is exactly the coupling
    /// this phase removes.
    #[test]
    fn the_artifact_edge_fetches_the_pinned_bodies_it_lacks() {
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
            // Node-0 holds only its own log and one peer's, so two of the four
            // pinned bodies are missing.
            let (committee, key0, journal) = node0_pre_seal_journal(71);
            oracle.manager().track(0, committee.clone()).await;
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                false,
            )
            .expect("post-deadline resume");

            let (sender, receiver) = oracle
                .control(key0.public_key())
                .register(
                    fluentbase_p2p::constants::BEACON_CHANNEL,
                    fluentbase_p2p::constants::BEACON_QUOTA,
                )
                .await
                .expect("register");
            let resolver = RecordingResolver::default();
            let in_flight = resolver.in_flight.clone();
            let committee_for: CommitteeFor = {
                let set = committee.clone();
                Arc::new(move |_e: u64| Some(set.clone()))
            };
            let store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut actor = DkgActor::new(
                b"FLUENT_DPOS_V1_clocktest".to_vec(),
                key0,
                sender,
                receiver,
                Some(resolver),
                None,
                committee_for,
                store.clone(),
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                None,
                ShareState::Plaintext,
                None,
            );
            actor
                .ceremonies
                .insert(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);
            // The chain is PAST the target's boundary — where the height-driven
            // fetch gives up — and the height clock has stopped there.
            actor.last_height = BOUNDARY;

            // The agreed set names every seat, including the two whose bodies this
            // node does not hold.
            let logs: Vec<(u8, B256)> = committee
                .iter()
                .enumerate()
                .map(|(i, _)| (i as u8, B256::repeat_byte(0x40 + i as u8)))
                .collect();
            let mut rng = StdRng::seed_from_u64(12);
            actor
                .on_artifact(
                    agreed_artifact(DETERMINISTIC_BOOTSTRAP_EPOCH, logs),
                    &mut rng,
                )
                .await;

            assert!(
                store.read().map(|s| s.is_empty()).unwrap_or(false),
                "a set whose bodies are missing must WAIT, never subset-finalize"
            );
            assert!(
                !in_flight.lock().unwrap().is_empty(),
                "the arrival edge must drive the recovery fetch the halted height \
                 clock no longer does"
            );
        });
    }

    /// A certified set needs no settle deadline, and that is not an optimisation.
    ///
    /// The deadline exists ONLY to make every honest node select over an identical
    /// set; a quorum certificate states that outright. Keeping the deadline would
    /// re-couple the write-back to the finalized-height clock — the exact coupling
    /// this phase removes — so a node whose clock has stopped short of it would sit
    /// on an agreed key it could have minted.
    ///
    /// The pinned set here is a strict SUBSET of the committee (node-0 never sealed
    /// its own log), so the complete-set fast path cannot fire and the deadline is
    /// the only other way through.
    #[test]
    fn a_certified_subset_finalizes_without_the_settle_deadline() {
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
            let (committee, key0, journal) = node0_pre_seal_journal(83);
            oracle.manager().track(0, committee.clone()).await;
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                false,
            )
            .expect("post-deadline resume");

            let store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut actor = standalone_actor(&oracle, key0, committee.clone(), None).await;
            actor.store = store.clone();
            actor
                .ceremonies
                .insert(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);
            // Sealed, but the settle deadline is still `DKG_SETTLE_BLOCKS` away.
            actor.last_height = SEAL_DEADLINE;

            let logs = pinned_logs_of(&actor, DETERMINISTIC_BOOTSTRAP_EPOCH, &committee);
            assert!(
                logs.len() < committee.len(),
                "the pinned set must be a strict subset or the fast path decides this"
            );
            let mut rng = StdRng::seed_from_u64(13);
            actor
                .on_artifact(
                    agreed_artifact(DETERMINISTIC_BOOTSTRAP_EPOCH, logs),
                    &mut rng,
                )
                .await;
            assert!(
                store
                    .read()
                    .map(|s| s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
                    .unwrap_or(false),
                "a quorum-certified set is settled the moment it lands"
            );
        });
    }

    /// The plane's spawn edge. A ceremony whose dealing has CLOSED is the earliest
    /// point at which this node has a dealer-log set worth agreeing, and it is the
    /// actor that knows when that happened — so it announces, and the plane starts
    /// the instance.
    ///
    /// A ceremony that is still DEALING is not announced: proposing a set the dealer
    /// is still adding to would agree a value that is stale by construction.
    #[test]
    fn the_dealing_closed_edge_asks_the_plane_for_an_instance() {
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
            let mut rng = StdRng::seed_from_u64(0x5EA1);
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;
            let (requests_tx, mut requests_rx) = tokio::sync::mpsc::channel(4);
            let (_artifacts_tx, artifacts_rx) = tokio::sync::mpsc::channel(1);
            let mut actor = standalone_actor(&oracle, keys[0].clone(), committee.clone(), None)
                .await
                .with_agreement_plane(requests_tx, artifacts_rx);
            let (cer, _step) = DkgCeremony::start(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee,
                keys[0].clone(),
            )
            .expect("start");
            actor.ceremonies.insert(DETERMINISTIC_BOOTSTRAP_EPOCH, cer);

            // Still dealing: nothing to agree yet.
            actor.on_height(SEAL_DEADLINE - 1, &mut rng).await;
            assert!(
                requests_rx.try_recv().is_err(),
                "a ceremony that is still dealing must not start an agreement"
            );

            // `on_height` step 1 seals it, which closes the dealing.
            actor.on_height(SEAL_DEADLINE, &mut rng).await;
            assert_eq!(
                requests_rx.try_recv().ok(),
                Some(DETERMINISTIC_BOOTSTRAP_EPOCH),
                "the dealing-closed edge asks the plane for this target's instance"
            );
        });
    }

    /// The restart between adopting an artifact and finalizing over it.
    ///
    /// A member of `committee[E+1]` adopts the agreed set, waits on the one pinned
    /// body its fetch has not delivered yet, and goes down. Its own instance wrote
    /// the artifact to the durable store on the edge that produced it, so the value
    /// is on this node's disk — but the only producer this actor's artifact arm ever
    /// had is a LIVE instance, and no peer will start a second one (their launchers
    /// already hold `E+1` in `started`). So the missing body arriving after the
    /// restart completes nothing: the set to finalize over went down with the
    /// process. [`restart_replay`] is what puts it back, through the same arm.
    #[test]
    fn a_restart_replays_the_stored_artifact_back_into_the_actor() {
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
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(97);
            oracle.manager().track(0, committee.clone()).await;

            // The on-disk state a crash leaves behind: the ceremony journal minus the
            // one peer log the recovery fetch had not delivered, and no share file.
            let dir = fresh_share_dir("artifact-replay");
            std::fs::create_dir_all(&dir).expect("mkdir");
            let mut withheld: Option<DealerReveal> = None;
            for record in journal {
                match record {
                    JournalRecord::PeerLog(log) if withheld.is_none() => withheld = Some(*log),
                    record => {
                        share_state::append_journal(
                            &dir,
                            DETERMINISTIC_BOOTSTRAP_EPOCH,
                            &record,
                            &ShareState::Plaintext,
                        )
                        .expect("append journal");
                    }
                }
            }
            let withheld = withheld.expect("the sealed journal carries peer logs");

            let ceremony_store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut actor =
                standalone_actor(&oracle, key0, committee.clone(), Some(dir.clone())).await;
            actor.store = ceremony_store.clone();
            // A wired plane is what bars the legacy local-settled finalize: only a
            // certified set may mint, so the restarted node waits on a set it no
            // longer has instead of settling one of its own.
            let (agree_tx, _agree_rx) = tokio::sync::mpsc::channel(4);
            let (_artifacts_tx, artifacts_rx) = tokio::sync::mpsc::channel(4);
            actor = actor
                .with_recorded_logs(Arc::new(RwLock::new(BTreeMap::new())))
                .with_agreement_plane(agree_tx, artifacts_rx);

            // The restart proper: the first height tick resumes the ceremony off the
            // journal, still inside epoch E (so nothing has swept it).
            let mut rng = StdRng::seed_from_u64(0xB007);
            actor.on_height(BOUNDARY - 1, &mut rng).await;
            let resumed = actor
                .ceremonies
                .get(&DETERMINISTIC_BOOTSTRAP_EPOCH)
                .expect("the journal resumes the ceremony");
            let mut missing_dealer: Option<PeerPubkey> = None;
            let pinned: Vec<(u8, B256)> = committee
                .iter()
                .enumerate()
                .map(|(i, pk)| match resumed.signed_log_hash(pk) {
                    Some(hash) => (i as u8, hash),
                    None => {
                        missing_dealer = Some(pk.clone());
                        (i as u8, alloy_primitives::keccak256(withheld.encode()))
                    }
                })
                .collect();
            let missing_dealer = missing_dealer.expect("one peer log was withheld");

            // The artifact this node's own instance certified and stored before the
            // crash. `insert` is what the instance's delivery edge does.
            let artifacts = crate::beacon::artifact::ArtifactStore::new();
            assert!(artifacts.insert(
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                agreed_artifact(DETERMINISTIC_BOOTSTRAP_EPOCH, pinned)
            ));

            // The awaited body lands. Every pinned body is now held — and it mints
            // nothing, because the pinned set itself did not survive the restart.
            let key = DkgLogKey {
                epoch: DETERMINISTIC_BOOTSTRAP_EPOCH,
                dealer: missing_dealer,
            };
            assert!(
                actor.ingest_log(&key, withheld.encode(), &mut rng).await,
                "the withheld log is a valid one for the dealer it was fetched from"
            );
            assert!(
                actor.agreed_pinned.is_empty(),
                "nothing but a live instance ever reaches the artifact arm — the wedge"
            );
            assert!(
                ceremony_store.read().map(|s| s.is_empty()).unwrap_or(false),
                "with the agreed set gone the finalize waits, holding every body it names"
            );

            // The fix: the value on this node's own disk, pushed back through the arm
            // a live instance feeds.
            let held: BTreeSet<u64> = ceremony_store
                .read()
                .map(|s| s.keys().copied().collect())
                .unwrap_or_default();
            let replay = crate::beacon::artifact::restart_replay(&artifacts, &dir, &held);
            assert_eq!(
                replay.len(),
                1,
                "an epoch with a stored artifact, a resumable journal and no share"
            );
            for artifact in replay {
                actor.on_artifact(artifact, &mut rng).await;
            }
            assert!(
                ceremony_store
                    .read()
                    .map(|s| s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
                    .unwrap_or(false),
                "the replayed artifact is the pinned set; the existing rails do the rest"
            );

            // And it is a one-shot: the share it produced is what takes the epoch back
            // out of the selection, so a later restart does not re-adopt a spent set.
            let held: BTreeSet<u64> = ceremony_store
                .read()
                .map(|s| s.keys().copied().collect())
                .unwrap_or_default();
            assert!(
                crate::beacon::artifact::restart_replay(&artifacts, &dir, &held).is_empty(),
                "a held share takes its epoch out of the replay"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }
}

#[cfg(test)]
mod retain_floor_tests {
    use super::ceremony_retain_floor;

    const WINDOW: u64 = 8;

    #[test]
    fn single_old_mint_on_stable_committee_is_retained() {
        // The load-bearing case: one mint minted long ago (@3) is STILL in force at
        // now=1000 on a stable committee — the floor is that mint itself, so it survives.
        assert_eq!(ceremony_retain_floor([3].into_iter(), 1000, WINDOW), 3);
    }

    #[test]
    fn churned_mints_keep_only_from_the_in_force_floor() {
        // now=20, cutoff=12: the greatest mint <= 12 is 10 (the mint in force for the
        // oldest still-verified cert), so 3 and 7 are prunable, 10/13/19 retained.
        assert_eq!(
            ceremony_retain_floor([3, 7, 10, 13, 19].into_iter(), 20, WINDOW),
            10
        );
    }

    #[test]
    fn mint_exactly_on_the_window_boundary_is_the_floor() {
        // A mint at exactly now-window (12) is <= cutoff, so it is the floor and is kept.
        assert_eq!(ceremony_retain_floor([12, 15].into_iter(), 20, WINDOW), 12);
    }

    #[test]
    fn no_mint_old_enough_retains_everything() {
        // Every mint is inside the window (> cutoff) ⇒ floor 0 ⇒ prune nothing.
        assert_eq!(ceremony_retain_floor([15, 18].into_iter(), 20, WINDOW), 0);
        assert_eq!(ceremony_retain_floor(std::iter::empty(), 20, WINDOW), 0);
    }
}
