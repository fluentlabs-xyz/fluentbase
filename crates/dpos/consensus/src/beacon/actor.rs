//! Networked live-DKG actor: wraps [`DkgCeremony`] and drives committee[E]'s
//! self-DKG over `BEACON_CHANNEL` during epoch E-1.
//!
//! One ceremony per epoch, no Muxer: each `DkgMsg` carries its `ceremony_epoch`.
//! A dealing (`Commitment`/`Share`) for a near-future epoch that arrives before this
//! node started its own ceremony for it is buffered (`pending`) and drained when the
//! ceremony starts, so the start-race does not drop it; any other message not for an
//! active ceremony is dropped. Ceremonies for E and E+1 are temporally disjoint (the
//! collection window spans ~all of E-1), so at most a couple are in flight.
//!
//! Lifecycle, driven by the finalized-height stream + chain committee reads:
//! - entering epoch E-1 (the chain says E mints a key — the `changed` bit, forced at
//!   `DETERMINISTIC_BOOTSTRAP_EPOCH` — and this node ∈ committee[E]) →
//!   `DkgCeremony::start`, broadcast commitment + send shares;
//! - finalized height reaches `epoch_start(E) - DKG_MARGIN_BLOCKS` → `seal_dealings`
//!   (broadcast the signed log);
//! - the epoch-key agreement plane certifies a dealer-log set for E and hands it
//!   back as an artifact; once every body it names is held and a quorum is
//!   selectable within it (probed event-driven on each recording, our seal or an
//!   incoming `Reveal`, via [`DkgActor::drive_finalization`]) →
//!   `DkgCeremony::finalize_over_pinned` → memoize the share into the per-epoch
//!   [`CeremonyStore`] and fire `share_notify`.
//!
//! The agreed set is the only finalize input, so every honest node selects over the
//! identical set ⇒ identical `PK_E`. The actor never finalizes before sealing or
//! over an under-quorum set (`pinned_ready` gates it); an epoch whose set never
//! reaches quorum gets no `CeremonyStore` entry and its beacon stalls, rather than
//! crashing.
//!
//! Mid-window restart: ceremony progress is journaled to `beacon-dkgjournal-e<E>.bin`;
//! on restart [`DkgActor::recover`] resumes via `DkgCeremony::resume` — a pre-seal
//! restart re-derives the seeded dealer (`dealer_seed_rng`, byte-identical
//! commitment) and keeps distributing, while an at/after-deadline restart is
//! player-only and never re-seals. Missing peer logs are re-fetched through the
//! DKG-log recovery resolver (`fetch_missing_logs`/`on_resolver_message`).
//!
//! The phase of an epoch is one value of [`EpochState`], held per target epoch in
//! [`DkgActor::epochs`], read by matching that value and never by combining fields;
//! every transition is a `set_state` / `enter` in this file.

use crate::beacon::{
    artifact::{value_digest, ChangedAt},
    ceremony::{
        log_hash, recompute_scoped, CeremonyOutput, DealerEquivocation, DkgCeremony, LogId,
        Outgoing, Step, Target,
    },
    confirmations::{ConfirmTrigger, Confirmations},
    dkg_agree::{AgreedArtifact, ConfirmPool, DkgProposal, PinnedDerive, PinnedLogs, ShareConfirm},
    dkg_msg::{DealerReveal, DkgBody, DkgMsg},
    log_resolver::{DkgLogKey, LogMessage},
    log_store::DealerLogStore,
    metrics::StallReason,
    outcome::{validate_share_on_poly, DkgOutcome},
    share_state::{self, ConflictMarker, JournalLoad, JournalRecord, ShareState},
    wire::BeaconMessage,
    JOURNAL_RETENTION_EPOCHS,
};
use crate::{epocher::OriginEpocher, sync_metrics::PlaneClock, SCHEME_RETENTION_EPOCHS};
use alloy_primitives::B256;
use bytes::Bytes;
use commonware_codec::{Encode as _, Read as _, ReadExt as _};
use commonware_consensus::types::{Epoch, Epocher as _, Height};
use commonware_cryptography::{
    bls12381::{dkg::Error as DkgError, primitives::group::Share},
    ed25519::PrivateKey as Ed25519PrivateKey,
    Signer as _,
};
use commonware_p2p::{Receiver, Recipients, Sender};
use commonware_resolver::Resolver;
use commonware_utils::{ordered::Set, vec::NonEmptyVec};
use fluentbase_bls::PeerPubkey;
use rand_core::CryptoRngCore;
use std::{
    collections::{BTreeMap, BTreeSet},
    num::{NonZeroU32, NonZeroU64},
    path::PathBuf,
    sync::{Arc, RwLock},
};

/// Blocks of slack before the epoch-E boundary at which dealing collection closes
/// and dealers seal (broadcast their signed logs) — the echo-settle tail, pinned
/// off the on-chain `epochBlockInterval`.
///
/// This is the whole budget the epoch key has: the value to be agreed does not
/// exist anywhere in the network before the seal, so `epoch_start(E) -
/// DKG_MARGIN_BLOCKS` is the earliest instant the agreement plane can start and
/// `epoch_start(E)` is when the boundary block needs the key.
///
/// Do not raise it without a measured `T_agree` overrun that demands it: a wider
/// window also lengthens the wait of every epoch-waiting case. Requires
/// `epochBlockInterval > DKG_MARGIN_BLOCKS` for a positive deal window.
pub(crate) const DKG_MARGIN_BLOCKS: u64 = 20;

/// The `channel` label every beacon ingress refusal is counted under
/// (`dpos_ingress_dropped_total`).
pub(crate) const BEACON_CHANNEL_LABEL: &str = "beacon";

/// How far above this actor's own epoch clock a frame's ceremony epoch may lie
/// and still be worth a look: the ingress window is `[now, now + 2]`
/// ([`within_ingress_window`]), and every ingress rule on this actor reads it
/// from here — [`DkgActor::epoch_is_actionable`] (the cost gate before the
/// decode), [`DkgActor::is_bufferable`] (the start-race buffer, which adds its
/// own `epoch > now`) and [`DkgActor::on_confirm`] (the entry-bar window).
///
/// The window is about epochs, not senders: who may speak on the channel is
/// classified by the channel's pre-decode `GatedReceiver`, and which seat a
/// sender holds in the frame's epoch is bound by the consumer.
///
/// The two epochs above `now` are what it has to cover: the ceremony this actor
/// may still start (`now + 1`, via `recover`) and the one a peer one epoch ahead
/// is already dealing for (`now + 2`). A peer two epochs ahead would mean this
/// clock lags the network by two whole epochs — the cold-start case, not a
/// steady-state race — and nothing a frame for `now + 3` carries is lost by
/// refusing it: a dealing is re-sent every pre-seal tick
/// ([`DkgCeremony::retransmit`]), a reveal is re-fetched by its pinned hash
/// (`fetch_missing_logs`), and a confirmation's entry bar is consumed at the
/// peers' agreement, two epochs ahead of this clock.
pub(crate) const INGRESS_LOOKAHEAD_EPOCHS: u64 = 2;

/// The shared ingress window: one predicate, so the rules that read it cannot
/// drift apart.
pub(crate) fn within_ingress_window(now: u64, epoch: u64) -> bool {
    (now..=now.saturating_add(INGRESS_LOOKAHEAD_EPOCHS)).contains(&epoch)
}

/// The epoch the beacon goes live at, deterministically: `committee[2]` runs its
/// DKG during epoch 1 even if unchanged from `committee[1]`, so a long-stable
/// initial committee still seeds the beacon (on-change-only activation would leave
/// it seedless indefinitely). Epoch 1 stays seedless (`order.digest()`); on-change
/// re-DKG and carry-forward apply thereafter.
pub const DETERMINISTIC_BOOTSTRAP_EPOCH: u64 = 2;

/// What the artifact store holds for an epoch, as this actor reads it: the
/// certified payload it serves (`held` — the pinned dealer-log set and the group
/// `Output`) and the value digest of the second quorum-certified value the store
/// saw, if it saw one (`ArtifactStore::note_divergent`, persisted under the beacon
/// directory) — the `Conflict` witness.
pub struct StoredArtifact {
    pub held: DkgProposal,
    pub divergent: Option<B256>,
}

/// Reads the agreement plane's artifact store for an epoch, threaded into the
/// actor as a read handle rather than a cross-actor push channel. Returns `Some`
/// only where an artifact for that exact epoch is held — a change epoch whose
/// agreement this node has the certified result of; `None` for a carry-forward
/// epoch (no fresh DKG) or a store miss (retried next tick).
///
/// The store owns "the epoch's artifact": the actor reads it every height tick for
/// every decided epoch ([`DkgActor::reconcile_with_store`]), so a push lost on the
/// way to the actor is still applied and a second value the store noted is still a
/// `Conflict`. Keyed by epoch rather than read from E's boundary block, because the
/// heal exists for a member that could not enter E, whose own epoch produces no
/// first block. Synchronous: the store is a RAM map behind a lock, and the handle
/// is read from synchronous paths (`recover`, `apply_artifact`) that cannot await.
pub type AgreedOutcomeAt = Arc<dyn Fn(u64) -> Option<StoredArtifact> + Send + Sync>;

/// Fire-and-forget request for the agreed artifact of an epoch this node needs and
/// does not hold.
///
/// Not a resolver: this actor's resolver is narrowed to [`DkgLogKey`] by
/// `LogFetcher`, and an artifact key has no business in that key space. Not a
/// future either — `ArtifactPull::pull` sleeps on a per-epoch throttle and then
/// waits out a timeout, and awaiting that inside `drive_acquisition` would stall
/// `on_height`, which drives every live ceremony. The callee spawns and this
/// returns immediately.
pub type PullArtifact = Arc<dyn Fn(u64) + Send + Sync>;

/// Per-epoch state of an in-flight demote-heal recompute: the pinned `Output` (the
/// `validate_share_on_poly` self-check target), the artifact's pinned set mapped
/// onto the committee (`dealer → hash`, the recompute's selection scope, the same
/// input the live finalize scopes to) and the pinned bodies this node still needs
/// to fetch (`want`, by exact `(dealer, hash)`, drained as the resolver delivers
/// them). Bounded: it is the payload of [`Acquire::Logs`], which is built only for
/// epochs inside the retention window and leaves with the epoch's `Keyed` /
/// `Unrecoverable` transition or the sweep.
struct RecomputeState {
    outcome: DkgOutcome,
    pinned: BTreeMap<PeerPubkey, B256>,
    want: BTreeSet<LogId>,
    /// The value digest of the artifact this heal is scoped to — what a second
    /// artifact for the epoch is compared against ([`EpochState::Conflict`]).
    digest: B256,
    /// Whether the recompute has run over the current inputs. `recompute_scoped` is
    /// deterministic on them, so one run per change is enough: a body landing in the
    /// journal (`ingest_recompute_log`) re-arms it, and so does a persist failure
    /// (the disk, not the inputs, failed).
    attempted: bool,
}

/// One artifact-sourced pinned set: the finalize input (`pinned`), the polynomial
/// every adopted share must lie on (`group_key`, the artifact's own — the local
/// ceremony's output is never the gate), and the value digest a second artifact for
/// the epoch is compared against (`value_digest`: the set and the key, not the
/// certificate's `confirms` metadata).
struct AgreedSet {
    pinned: BTreeMap<u8, B256>,
    group_key: DkgOutcome,
    digest: B256,
}

impl AgreedSet {
    fn of(proposal: &DkgProposal) -> Self {
        Self {
            pinned: proposal.logs.iter().copied().collect(),
            group_key: proposal.group_key.clone(),
            digest: value_digest(proposal),
        }
    }
}

/// What an [`EpochState::Acquiring`] epoch is waiting on; the arms resolve
/// differently when the artifact arrives ([`DkgActor::on_artifact`]).
enum Acquire {
    /// The instance certified a set whose body never arrived (`dkg_agree_body_lost`),
    /// or the epoch is being recovered past its boundary: the sealed ceremony waits
    /// for a peer's copy of the artifact (`pull_artifact`, retried every tick).
    ArtifactForCeremony(Box<DkgCeremony>),
    /// A member with no ceremony to resume and no artifact (a held share, or the
    /// post-seal absentee): [`DkgActor::key_held_share`] decides on its arrival.
    ArtifactForShare,
    /// A non-member of a mint epoch, which needs the epoch's `PK_E` to verify its
    /// certificates.
    ArtifactForKey,
    /// The pinned bodies the journal recompute still lacks (the demote-heal, and
    /// the retry behind a finalize `Err` / a refused adoption).
    Logs(Box<RecomputeState>),
}

/// The phase of one target epoch `E`. Absent from [`DkgActor::epochs`] means idle:
/// nothing decided yet (the committee or the `changed` bit unreadable, or the epoch
/// outside the window this actor looks at), re-asked on the next tick.
///
/// Every digest held here is the artifact's value digest (`value_digest`).
enum EpochState {
    /// Not a phase: the slot's value is out for a by-value transition
    /// ([`DkgActor::take_state`]) and comes back with [`DkgActor::set_state`] or
    /// [`DkgActor::put_back`] before the input that took it returns
    /// ([`DkgActor::debug_assert_settled`]). A slot a reader finds in it is a bug:
    /// it reads as nothing — no ceremony, no digest, no artifact wanted.
    InTransition,
    /// The dealer is live (start, or a pre-seal resume). `agreed` holds an
    /// artifact that arrived before this node's own seal (its clock lags the
    /// network); it becomes the finalize scope the instant the dealing closes.
    Dealing {
        ceremony: DkgCeremony,
        agreed: Option<AgreedSet>,
    },
    /// Dealing closed (sealed, or resumed player-only); no artifact yet. The
    /// agreement plane is asked for an instance from here.
    Sealed {
        ceremony: DkgCeremony,
    },
    /// The artifact is held: `finalize_over_pinned` runs over `set` as soon as
    /// every pinned body is held and a quorum is selectable within it.
    Agreed {
        ceremony: DkgCeremony,
        set: AgreedSet,
    },
    Acquiring(Acquire),
    /// This node's share for `E` is in `store`, it lies on the held artifact's
    /// polynomial (checked on every way in: the live finalize, the journal
    /// recompute, and a share file reloaded over an artifact), and the artifact is
    /// `digest`.
    Keyed {
        digest: B256,
    },
    /// No share obligation: a carry-forward epoch (`digest: None`, the key in force
    /// is an earlier mint's) or a mint epoch this node is not a member of and holds
    /// the artifact for.
    KeyOnly {
        digest: Option<B256>,
    },
    /// Terminal. The share is provably not derivable here: the journal acks a
    /// dealing this node no longer holds (`MissingPlayerDealing`), or the ceremony
    /// cannot be rebuilt/started over the committed roster. `key` is the epoch's
    /// artifact once held: a share-less member still verifies with `PK_E`.
    Unrecoverable {
        key: Option<B256>,
    },
    /// Terminal. Two different quorum-certified artifacts for one epoch reached this
    /// actor (`held` first) — ≥ 2q−n Byzantine signers. The epoch's signing stops
    /// here: the share leaves `store` and its file, and the verdict is persisted
    /// (`share_state::persist_conflict`, read back by `recover`) so it outlives the
    /// process. `key` is the artifact the store holds for the epoch, if any — a node
    /// in `Conflict` still verifies the epoch's certificates and needs `PK_E` for
    /// that. A `Conflict` for which the store holds no artifact acquires one from
    /// peers like every other keyless terminal; the signing stays stopped whatever
    /// arrives.
    Conflict {
        held: B256,
        second: B256,
        key: Option<B256>,
    },
}

impl EpochState {
    fn name(&self) -> &'static str {
        match self {
            Self::InTransition => "in_transition",
            Self::Dealing { .. } => "dealing",
            Self::Sealed { .. } => "sealed",
            Self::Agreed { .. } => "agreed",
            Self::Acquiring(Acquire::ArtifactForCeremony(_)) => "acquiring_artifact_for_ceremony",
            Self::Acquiring(Acquire::ArtifactForShare) => "acquiring_artifact_for_share",
            Self::Acquiring(Acquire::ArtifactForKey) => "acquiring_artifact_for_key",
            Self::Acquiring(Acquire::Logs(_)) => "acquiring_logs",
            Self::Keyed { .. } => "keyed",
            Self::KeyOnly { .. } => "key_only",
            Self::Unrecoverable { .. } => "unrecoverable",
            Self::Conflict { .. } => "conflict",
        }
    }

    fn ceremony(&self) -> Option<&DkgCeremony> {
        match self {
            Self::Dealing { ceremony, .. }
            | Self::Sealed { ceremony }
            | Self::Agreed { ceremony, .. } => Some(ceremony),
            Self::Acquiring(Acquire::ArtifactForCeremony(ceremony)) => Some(ceremony),
            _ => None,
        }
    }

    fn ceremony_mut(&mut self) -> Option<&mut DkgCeremony> {
        match self {
            Self::Dealing { ceremony, .. }
            | Self::Sealed { ceremony }
            | Self::Agreed { ceremony, .. } => Some(ceremony),
            Self::Acquiring(Acquire::ArtifactForCeremony(ceremony)) => Some(ceremony),
            _ => None,
        }
    }

    /// The digest of the artifact this phase already stands on, if any — what a
    /// second artifact for the epoch is compared against.
    fn held_digest(&self) -> Option<B256> {
        match self {
            Self::Dealing {
                agreed: Some(set), ..
            }
            | Self::Agreed { set, .. } => Some(set.digest),
            Self::Acquiring(Acquire::Logs(st)) => Some(st.digest),
            Self::Keyed { digest } => Some(*digest),
            Self::KeyOnly { digest } => *digest,
            Self::Unrecoverable { key } => *key,
            Self::Conflict { held, .. } => Some(*held),
            _ => None,
        }
    }

    fn needs_artifact(&self) -> bool {
        matches!(
            self,
            Self::Acquiring(
                Acquire::ArtifactForCeremony(_)
                    | Acquire::ArtifactForShare
                    | Acquire::ArtifactForKey
            ) | Self::Unrecoverable { key: None }
                | Self::Conflict { key: None, .. }
        )
    }
}

/// Everything this actor keeps per target epoch: the phase, the `Stalled{reason}`
/// latches raised on it (one line and one gauge step per `(epoch, reason)`, dropped
/// with the transition that leaves the condition behind — [`carries`]), the
/// dealer-equivocation evidence proven for it (data beside the phase, never a phase:
/// a two-log dealer costs that dealer a gossip ban, not this epoch its signing), and
/// whether the plane has accepted the agreement announcement (the one-shot log
/// line).
struct EpochSlot {
    state: EpochState,
    stalled: BTreeSet<StallReason>,
    evidence: BTreeMap<PeerPubkey, DealerEquivocation>,
    announced: bool,
    /// The instance's body-lost signal arrived while this node was still `Dealing`
    /// (its clock lags the network). Nothing can act on it before the seal — there
    /// is no sealed ceremony to acquire for — so it is kept here and applied at the
    /// seal: `Dealing{agreed: None}` seals into `Acquiring(ArtifactForCeremony)` and
    /// pulls at once, instead of `Sealed` waiting for the boundary.
    body_lost: bool,
}

impl EpochSlot {
    fn new(state: EpochState) -> Self {
        Self {
            state,
            stalled: BTreeSet::new(),
            evidence: BTreeMap::new(),
            announced: false,
            body_lost: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AdoptRefusal {
    OffPolynomial,
    PersistFailed,
}

impl AdoptRefusal {
    fn stall(self) -> StallReason {
        match self {
            Self::OffPolynomial => StallReason::OffPolynomial,
            Self::PersistFailed => StallReason::PersistFailed,
        }
    }
}

/// Whether `state` still carries the condition `reason` was raised for — the rule
/// [`DkgActor::set_state`] drops a latch by, so a gauge never counts a condition the
/// epoch has moved past.
fn carries(state: &EpochState, reason: StallReason) -> bool {
    match reason {
        StallReason::QuorumMissing | StallReason::BodyMissing => matches!(
            state,
            EpochState::Agreed { .. }
                | EpochState::Dealing {
                    agreed: Some(_),
                    ..
                }
        ),
        StallReason::BodyLost => {
            matches!(
                state,
                EpochState::Acquiring(Acquire::ArtifactForCeremony(_))
            )
        }
        StallReason::NoArtifact => state.needs_artifact(),
        StallReason::PersistFailed | StallReason::OffPolynomial | StallReason::HealFailed => {
            matches!(state, EpochState::Acquiring(Acquire::Logs(_)))
        }
        StallReason::Unrecoverable => matches!(state, EpochState::Unrecoverable { .. }),
        StallReason::Conflict => matches!(state, EpochState::Conflict { .. }),
    }
}

/// This node's secret share per epoch it minted at, memoized by the actor during
/// the post-seal margin window, before the boundary block is proposed. Read by the
/// oracle on the vote path; an epoch this node is not a member of gets no entry.
///
/// The map holds the share alone, not the group output: the polynomial that pairs
/// with a share is the one the epoch's certified artifact carries, read through
/// [`KeyIndex`](crate::beacon::artifact::KeyIndex) at the same minting epoch, and a
/// share that does not lie on that polynomial is refused before it is stored
/// ([`DkgActor::adopt_share`]).
pub type CeremonyStore = Arc<RwLock<BTreeMap<u64, Share>>>;

/// `epoch → (committee idx → content hash of the recorded `SignedDealerLog`)`,
/// written by the [`DkgActor`] as it records a valid log; read by the epoch-key
/// agreement plane, which proposes over it, and by the share-confirmations that
/// state it. `idx` is the position in the agreed on-chain `committee[epoch]`
/// (`u8`, `n ≤ MAX_COMMITTEE_SIZE`).
pub type DkgLogIndex = Arc<RwLock<BTreeMap<u64, BTreeMap<u8, B256>>>>;

/// The retain floor for the shared insert-only [`CeremonyStore`]: the greatest mint
/// epoch `<= now - window`, i.e. the mint in force for the oldest cert inside the
/// scheme-retention window. Every entry `>= floor` must be kept — a reader resolves
/// the share at the minting epoch the chain names, which on a stable committee is an
/// older mint, so a size cap would demote a legitimate signer — while entries below
/// the floor are superseded mints no cert in the window can select. `0` (no mint old
/// enough to be a floor) retains everything: a stable committee prunes nothing,
/// churn bounds growth to the trailing window.
///
/// Correctness depends on an invariant this function cannot see, written down here:
/// above `DETERMINISTIC_BOOTSTRAP_EPOCH`, a mint in the store implies `dkgQual[e]`
/// is set. A mint under a clear bit puts the floor above the mint the chain names,
/// and the prune deletes the key this node serves — `NoUsableMint` forever, with no
/// recompute path and no artifact carrying a share. The contract sets that bit from
/// `committee[target] != committee[target−1]` in `commitEpochCommittee`, the same
/// comparison the node makes over the same committed arrays; rosters read at
/// different states, or a bit redefined as "the DKG qualified", breaks it.
fn ceremony_retain_floor(keys: impl Iterator<Item = u64>, now: u64, window: u64) -> u64 {
    let cutoff = now.saturating_sub(window);
    keys.filter(|&k| k <= cutoff).max().unwrap_or(0)
}

/// Resolves `committee[epoch]` (the Commonware-ordered peer set) at the plane's
/// current state read — the ceremony roster and its `idx → pubkey` mapping. `None`
/// means this node cannot read the roster yet; callers leave the epoch undecided and
/// re-ask.
pub type CommitteeFor = Arc<dyn Fn(u64) -> Option<Set<PeerPubkey>> + Send + Sync>;

/// The artifact's pinned set (`idx → hash`, `idx` = position in `committee`) mapped
/// to `dealer → hash`. An `idx` with no position in `committee` is skipped, so every
/// node drops the same seats: nothing can be asked for a seat no roster has.
fn pinned_by_dealer(
    committee: &Set<PeerPubkey>,
    pinned: &BTreeMap<u8, B256>,
) -> BTreeMap<PeerPubkey, B256> {
    pinned
        .iter()
        .filter_map(|(idx, hash)| Some((committee.iter().nth(*idx as usize)?.clone(), *hash)))
        .collect()
}
/// The epochs this actor decides on its clock `now`, ascending: the trailing
/// retention window `[max(BOOTSTRAP, now − R), now]` it may still owe a key or a
/// heal for, then the one target it may still deal for, `now + 1`. Never further
/// ahead — an artifact for `now + 2` means a peer's clock is running ahead, and
/// dealing that epoch here would deal an epoch early over a roster this node has not
/// reached. Below `BOOTSTRAP` the beacon is seedless; `now + 1` is not clamped by the
/// bootstrap floor, so at `now = 0` epoch 1 is decided.
fn decidable_epochs(now: u64) -> impl Iterator<Item = u64> {
    let lo = now
        .saturating_sub(JOURNAL_RETENTION_EPOCHS)
        .max(DETERMINISTIC_BOOTSTRAP_EPOCH);
    (lo..=now).chain(std::iter::once(now.saturating_add(1)))
}

/// The dealers a ceremony step acks (`Target::Direct(dealer)` + `Ack`), read before
/// the step's journal is spent: a failed journal write withholds from exactly these.
fn acked_dealers(step: &Step) -> Vec<PeerPubkey> {
    step.outgoing
        .iter()
        .filter_map(|o| match (&o.target, &o.msg.body) {
            (Target::Direct(dealer), DkgBody::Ack(_)) => Some(dealer.clone()),
            _ => None,
        })
        .collect()
}

/// `recv()` on a plane edge that parks on a closed channel instead of answering
/// `None` on every poll: a closed edge is the sender's task gone, a state the loop
/// has nothing to do about, not an event to spin on.
async fn recv_or_park<T>(rx: &mut tokio::sync::mpsc::Receiver<T>) -> T {
    match rx.recv().await {
        Some(v) => v,
        None => std::future::pending().await,
    }
}

/// One question from the epoch-key agreement instance: does a candidate pinned
/// dealer-log set yield a group key here, and if not, why not.
///
/// The bodies the answer needs are owned by [`DkgActor`], so the question crosses a
/// channel; `epoch` rides the request because one actor serves the agreement
/// instances of every target epoch.
pub struct PinnedRequest {
    pub epoch: u64,
    pub pinned: BTreeMap<u8, B256>,
    pub(crate) response: tokio::sync::oneshot::Sender<PinnedDerive>,
}

/// The agreement instance's handle on the ceremony bodies [`DkgActor`] owns: the
/// production [`PinnedLogs`] implementor, bound to one target epoch because one
/// agreement instance agrees one epoch's set.
///
/// Every failure to get an answer — the actor gone, the reply dropped — is
/// [`PinnedDerive::Unavailable`], never [`PinnedDerive::Unusable`]: `Unusable` is
/// the one arm the agreement may turn into a nullified view for the whole network.
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

/// The two start-race dealings one sender can contribute, latest-wins. Only
/// `Commitment`/`Share` are ever bufferable (`is_bufferable`), so two `Option`s
/// cover a sender exactly.
#[derive(Default)]
struct PendingDealings {
    commitment: Option<DkgBody>,
    share: Option<DkgBody>,
}

/// The actor's edges — every channel, handle and directory the beacon plane connects
/// a [`DkgActor`] by, all of them required: the actor's behaviour is a function of
/// its inputs, never of which edges happened to be wired. Production builds the whole
/// of it in `plane.rs`; tests build it from `Wiring::standalone()`.
pub struct Wiring<R> {
    /// Mailbox to the beacon-plane DKG-log recovery resolver: a shorthanded ceremony
    /// `fetch_targeted`s its missing dealer logs through it.
    pub resolver: R,
    /// Inbound `Produce`/`Deliver` requests from the resolver engine
    /// (`log_resolver::LogHandler`). A closed channel is that engine's death, and
    /// [`DkgActor::run`] stops on it: the engine is a supervised child of the plane,
    /// so there is no gossip-only life to degrade into.
    pub resolver_rx: tokio::sync::mpsc::Receiver<LogMessage>,
    /// The chain's frozen `changed` bit (on-chain `dkgQual[epoch]`) — the only input
    /// of the ceremony-start decision. It is `committee[target] != committee[target−1]`,
    /// written by the contract in the same call that writes the committee, so reading it
    /// is reading the contract's own answer rather than comparing two local roster
    /// reads. `None` (unreadable) leaves the epoch undecided for the tick; the
    /// next tick re-asks.
    pub changed: ChangedAt,
    /// Directory for on-disk persistence of the live-DKG per-epoch shares, journals
    /// and conflict markers. The plane passes `<datadir>/beacon/` and reloads it once
    /// at startup.
    pub share_dir: PathBuf,
    /// The registered clock pair whose DKG half this actor publishes, off the
    /// monotone clamp in [`DkgActor::on_height`].
    pub plane_clock: PlaneClock,
    /// Read handle for the agreed artifact's payload (a pull, not a push channel),
    /// the artifact input of [`DkgActor::recover`] and of every tick's
    /// `reconcile_with_store`.
    pub outcome_at: AgreedOutcomeAt,
    /// Asks a peer for the agreed artifact of an epoch this node needs and does not
    /// hold. Driven by every `needs_artifact()` slot on each height tick
    /// ([`DkgActor::drive_acquisition`]) — the member without a share (body lost, or
    /// past the boundary), the member with a share and no artifact, the non-member
    /// that needs `PK_E` to verify, and a `Conflict` with no artifact in the store.
    /// Nothing else asks for the live epoch: the epoch manager's repair sweep excludes
    /// `epoch >= frontier`.
    pub pull_artifact: PullArtifact,
    /// The shared `epoch -> idx -> keccak256(SignedDealerLog)` index this actor
    /// publishes for the agreement plane to propose over and for share-confirmations
    /// to state. The same handle on both sides — this actor writes it
    /// (`publish_recorded_logs`, which owns the per-`(dealer, hash)` durability gate)
    /// and [`Confirmations`] reads it — so a confirmation can never name a set the
    /// proposal path would not.
    pub recorded_dkg_logs: DkgLogIndex,
    /// The share-confirmation pool the epoch-key agreement's entry bar counts. It
    /// carries the signing namespace, so the actor and the agreement instances must
    /// be handed the same pool or they cannot agree on it.
    pub confirms: ConfirmPool,
    /// Inbound pinned-set questions from the epoch-key agreement instances
    /// ([`PinnedMailbox`]).
    pub pinned_rx: tokio::sync::mpsc::Receiver<PinnedRequest>,
    /// Announcement sink for the epoch-key agreement plane's spawn edge: a target
    /// epoch whose ceremony has closed its dealing here, the earliest point at which
    /// this node has a dealer-log set worth agreeing. The plane owns the sub-channel
    /// registration and the instance; this actor owns the only state that knows when
    /// the edge happened.
    ///
    /// The edge is `dealing_closed()`, not the seal call: a node that restarted at or
    /// after the seal deadline resumes player-only and never seals, and it still has
    /// to run the agreement for the epoch.
    pub agreement_tx: tokio::sync::mpsc::Sender<u64>,
    /// The actor's epoch clock as the agreement launcher sees it: the epoch the height
    /// clock is in (`epoch_of(height)`, after the monotone clamp) — the launcher's
    /// cutoff, below which an instance is aborted and the journal-partition band is
    /// swept. Published from every height tick but only on change
    /// (`send_if_modified`): the epoch moves once per interval, and that edge is this
    /// clock's whole wake set; a per-tick publish would be a per-block edge carrying
    /// per-epoch work. The fork-safety latch is not carried here: the launcher waits
    /// on the latch's own edge (`SafetyHalt::engaged_edge`).
    pub epoch_clock: tokio::sync::watch::Sender<u64>,
    /// Agreed artifacts arriving from the plane — this node's own instance, or a
    /// peer's artifact that already verified against `committee[epoch]`.
    ///
    /// Precondition: every artifact on this channel has been checked against the
    /// target epoch's committee. Both producers check it (the instance only delivers
    /// a value its own quorum certified; the pull seam verifies before it stores), and
    /// this actor cannot re-check it — it reads peer identities, never the BLS
    /// committee the certificate is verified under.
    pub artifacts_rx: tokio::sync::mpsc::Receiver<AgreedArtifact>,
    /// Target epochs whose agreement instance certified a payload it could not
    /// resolve the body of: a `Sealed` epoch moves to
    /// `Acquiring(ArtifactForCeremony)` and pulls at once, while a still-`Dealing`
    /// epoch remembers it for its seal.
    pub body_lost_rx: tokio::sync::mpsc::Receiver<u64>,
    /// What a fixture-built wiring owns besides its edges — the far ends of its
    /// parked channels and its scratch directory — carried into the actor so they
    /// live exactly as long as it does. `None` from production, always `Some` from
    /// [`Wiring::inert`].
    #[cfg(test)]
    pub fixture: Option<Fixture>,
}

/// The half of a fixture wiring that is not an edge of the actor but must outlive its
/// construction, held in the actor ([`DkgActor::new`] moves it there) so nothing is
/// leaked and nothing is closed early: the parked plane channels' senders and the
/// announcement receiver, since a closed `resolver_rx` stops the actor and a closed
/// plane channel parks its arm, plus the scratch `share_dir`, removed on drop.
#[cfg(test)]
pub struct Fixture {
    scratch: PathBuf,
    _resolver_tx: tokio::sync::mpsc::Sender<LogMessage>,
    _pinned_tx: tokio::sync::mpsc::Sender<PinnedRequest>,
    _artifacts_tx: tokio::sync::mpsc::Sender<AgreedArtifact>,
    _body_lost_tx: tokio::sync::mpsc::Sender<u64>,
    _agreement_rx: tokio::sync::mpsc::Receiver<u64>,
}

#[cfg(test)]
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

/// The networked DKG actor. Generic over the p2p sender/receiver (the spawn site
/// passes the `BEACON_CHANNEL` halves) and over the DKG-log recovery resolver `R`:
/// the plane's `LogFetcher` over its `commonware_resolver::p2p::Mailbox` in
/// production, a no-op in unit tests.
pub struct DkgActor<Se, Re, R> {
    namespace: Vec<u8>,
    me_key: Ed25519PrivateKey,
    sender: Se,
    receiver: Re,
    resolver: R,
    resolver_rx: tokio::sync::mpsc::Receiver<LogMessage>,
    /// The deal/qual/serve roster reader: resolves `committee[epoch]` as the ceremony
    /// participants — under the 2-epoch warm-up it is frozen a full epoch before its
    /// DKG runs — who deals, whose logs are served and recomputed, and the roster
    /// `recover` builds the ceremony over.
    committee_for: CommitteeFor,
    changed: ChangedAt,
    store: CeremonyStore,
    /// Edge-trigger fired (`notify_one`) whenever this node's share set changes — a
    /// share adopted into `store` or dropped from it. The plane's event bridge is its
    /// only waiter; it republishes the edge as `BeaconEvent::ParticipationChanged`,
    /// and the epoch manager re-reads the share on that wake instead of polling.
    share_notify: Arc<tokio::sync::Notify>,
    /// Frozen `(dposActivationBlock, epochBlockInterval)` — the immutable epoch
    /// geometry, handed in as plain values once the plane has frozen it, so the actor
    /// never re-reads the chain and no codeless/genesis fallback can race this path.
    /// Held as the one epoch↔height authority rather than two raw numbers, so the
    /// deal/seal schedule cannot drift from every other epoch→height computation.
    epocher: OriginEpocher,
    metrics: crate::beacon::metrics::BeaconMetrics,
    share_dir: PathBuf,
    /// At-rest framing for the persisted shares: [`ShareState::Encrypted`] (the
    /// HKDF-derived seal key) on a keystore-mode validator, [`ShareState::Plaintext`]
    /// otherwise, built from the `Option<ShareSealKey>` the plane derives at launch
    /// (gated on `--dpos.bls-keystore-path`). Shared with [`Self::log_store`], which
    /// re-parses the journals this framing writes — one instance, so the encrypted
    /// arm's seal key is never duplicated in memory.
    share_state: Arc<ShareState>,
    /// The state machine: one [`EpochSlot`] per target epoch this actor has decided
    /// anything about, on the one retention window [`Self::sweep_epoch_state`]
    /// applies. The live ceremony, the agreed set, the in-flight heal, the sit-out and
    /// unrecoverable verdicts, the report-once marks and the evidence pairs are all
    /// variants or fields of the slot, so the phase of an epoch is one `match`.
    epochs: BTreeMap<u64, EpochSlot>,
    /// The cached and durable tiers of the dealer-log serve: a bounded, positive-only
    /// copy of the recorded logs of a finalized-but-not-yet-past-boundary epoch, plus
    /// the one-time journal parse behind it. Seeded eagerly at finalize and lazily on
    /// a cold `serve_log` miss after a restart, aged out at the boundary sweep on the
    /// same window as the journal.
    ///
    /// The live ceremony tier is not here — `serve_log` asks the epoch's slot first
    /// and only then falls through.
    log_store: DealerLogStore,
    /// Whether the one-shot startup journal reconcile has run. Driven on the actor's
    /// first `on_height` tick, where the frozen epoch geometry is finally available:
    /// it deletes every boundary-passed journal off disk — reclaiming orphaned
    /// journals and stale at-rest secrets a finalize-then-restart-before-boundary
    /// left, which the running sweep cannot, since the restart wiped the in-memory
    /// keys it scans.
    reconciled_journals: bool,
    /// Dealings (`Commitment`/`Share`) that arrived for an epoch before this node
    /// started its own ceremony for it — the start-race. Drained into the ceremony by
    /// `recover` before any seal, so a peer dealing that raced ahead of our start is
    /// never silently dropped (which would leave that dealer un-acked ⇒
    /// `TooManyReveals` ⇒ `DkgFailed`). Only the next 1–2 epochs are bufferable
    /// (`is_bufferable`) and stale epochs are evicted each height tick; per sender the
    /// bound is one slot (≤1 Commitment + ≤1 Share, latest-wins), so one Byzantine
    /// peer cannot evict honest dealings.
    pending: BTreeMap<u64, BTreeMap<PeerPubkey, PendingDealings>>,
    /// Last finalized height seen on the `on_height` stream — the chain time the
    /// event-driven `on_message` finalize uses for its deterministic-settle gate.
    ///
    /// `None` until the first tick is drained, and that is a distinct state, not a
    /// zero: the actor is constructed before the height poller's buffered tick is
    /// read, and `epoch_of(0)` would say "the chain is in epoch 0" about a chain that
    /// may be anywhere. Every deal/seal deadline reads it through [`Self::height_now`],
    /// whose `0` floor only delays an action; [`Self::on_confirm`] reads the field
    /// itself, because a `0` floor there would refuse every confirmation that beat the
    /// first tick, permanently — they are never re-issued.
    last_height: Option<u64>,
    /// Logs (by `(dealer, hash)`) whose journal record failed to land, per target
    /// epoch. Excluded from [`Self::publish_recorded_logs`] — this node holds the
    /// bytes in memory but cannot back the claim across a restart — retried from
    /// memory (not re-fetched) on every publish edge, and cleared on success.
    nondurable_logs: BTreeMap<u64, BTreeSet<LogId>>,
    /// `ReceivedDealing` records whose journal write failed, per target epoch. Their
    /// ack was withheld on the same edge and stays withheld; what the retry buys is
    /// the resume — a restart rebuilds `Player.view` from the journal, and a dealing
    /// that never landed there is recovered only through the dealer's reveal. Retried
    /// on the same edge as [`Self::nondurable_logs`], from the record itself: unlike a
    /// log, a dealing is not re-derivable from the ceremony.
    nondurable_dealings: BTreeMap<u64, Vec<JournalRecord>>,
    plane_clock: PlaneClock,
    outcome_at: AgreedOutcomeAt,
    pull_artifact: PullArtifact,
    recorded_dkg_logs: DkgLogIndex,
    /// This node's share-confirmation accounting: the pool the epoch-key agreement's
    /// entry bar counts, and the memory of what width this node has already put on
    /// the wire per target epoch. Ceremony-free — it reads the shared
    /// `recorded_dkg_logs` index, never the epoch slots — so the whole claimed-width
    /// policy lives in [`Confirmations`].
    confirmations: Confirmations,
    pinned_rx: tokio::sync::mpsc::Receiver<PinnedRequest>,
    agreement_tx: tokio::sync::mpsc::Sender<u64>,
    epoch_clock: tokio::sync::watch::Sender<u64>,
    artifacts_rx: tokio::sync::mpsc::Receiver<AgreedArtifact>,
    body_lost_rx: tokio::sync::mpsc::Receiver<u64>,
    /// The `Output` of every share this actor adopted, for tests only.
    ///
    /// The shared [`CeremonyStore`] keeps the share alone and the polynomial's owner
    /// is the epoch's artifact, but one test needs the output of the ceremony this
    /// node ran and cannot read it anywhere else: `Output::revealed()` — did the
    /// peers reveal this node's point, the withheld-ack rule. `#[cfg(test)]` rather
    /// than a field with a production reader, so a production line that reaches for
    /// it does not compile.
    #[cfg(test)]
    adopted_outcomes: Arc<RwLock<BTreeMap<u64, CeremonyOutput>>>,
    /// A simulated death inside [`Self::stop_signing`], between its two durable steps
    /// (the marker written, the share file not yet evicted) — the crash the marker-first
    /// order exists for. `true` makes `stop_signing` return right after the marker.
    #[cfg(test)]
    die_between_verdict_and_eviction: bool,
    #[cfg(test)]
    _fixture: Option<Fixture>,
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
        committee_for: CommitteeFor,
        store: CeremonyStore,
        share_notify: Arc<tokio::sync::Notify>,
        dpos_activation: u64,
        epoch_interval: u64,
        metrics: crate::beacon::metrics::BeaconMetrics,
        share_state: ShareState,
        wiring: Wiring<R>,
    ) -> Self {
        let Wiring {
            resolver,
            resolver_rx,
            changed,
            share_dir,
            plane_clock,
            outcome_at,
            pull_artifact,
            recorded_dkg_logs,
            confirms,
            pinned_rx,
            agreement_tx,
            epoch_clock,
            artifacts_rx,
            body_lost_rx,
            #[cfg(test)]
            fixture,
        } = wiring;
        let share_state = Arc::new(share_state);
        let log_store = DealerLogStore::new(
            namespace.clone(),
            committee_for.clone(),
            Some(share_dir.clone()),
            share_state.clone(),
        );
        let mut confirmations = Confirmations::new(me_key.clone(), committee_for.clone());
        confirmations.set_recorded(recorded_dkg_logs.clone());
        confirmations.set_pool(confirms);
        Self {
            namespace,
            me_key,
            sender,
            receiver,
            resolver,
            resolver_rx,
            committee_for,
            changed,
            store,
            share_notify,
            // Production callers reject a zero interval; the expect fails at
            // construction rather than mid-run.
            epocher: OriginEpocher::new(
                dpos_activation,
                NonZeroU64::new(epoch_interval).expect("epochBlockInterval must be > 0"),
            ),
            metrics,
            share_dir,
            share_state,
            log_store,
            epochs: BTreeMap::new(),
            reconciled_journals: false,
            pending: BTreeMap::new(),
            last_height: None,
            nondurable_logs: BTreeMap::new(),
            nondurable_dealings: BTreeMap::new(),
            plane_clock,
            outcome_at,
            pull_artifact,
            recorded_dkg_logs,
            confirmations,
            pinned_rx,
            agreement_tx,
            epoch_clock,
            artifacts_rx,
            body_lost_rx,
            #[cfg(test)]
            adopted_outcomes: Arc::new(RwLock::new(BTreeMap::new())),
            #[cfg(test)]
            die_between_verdict_and_eviction: false,
            #[cfg(test)]
            _fixture: fixture,
        }
    }

    /// Adopt `epoch`'s recorded dealer-log set as if a certified artifact had named
    /// it, with the group key the ceremony derives over that set — a test that only
    /// needs a ceremony to reach `finalize_over_pinned` skips the agreement.
    #[cfg(test)]
    fn pin_recorded_as_agreed(&mut self, epoch: u64, rng: &mut impl CryptoRngCore) {
        let committee = (self.committee_for)(epoch).expect("a committee for the pinned epoch");
        let c = self.ceremony(epoch).expect("a ceremony to pin over");
        let logs: Vec<(u8, B256)> = committee
            .iter()
            .enumerate()
            .filter_map(|(idx, pk)| {
                c.signed_log_hash(pk)
                    .map(|hash| (u8::try_from(idx).expect("committee fits a u8"), hash))
            })
            .collect();
        let PinnedDerive::Derived(group_key) =
            c.derive_pinned(rng, &committee, &logs.iter().copied().collect())
        else {
            panic!("the recorded set must derive a key");
        };
        let proposal = DkgProposal {
            target_epoch: epoch,
            logs,
            group_key: *group_key,
            confirms: Vec::new(),
        };
        self.apply_artifact(epoch, &proposal);
    }

    /// Pre-activation heights have no relative epoch and answer 0.
    fn epoch_of(&self, height: u64) -> u64 {
        self.epocher
            .containing(Height::new(height))
            .map_or(0, |info| info.epoch().get())
    }

    /// First-block height of an epoch (relative to DPoS activation), `u64::MAX` on
    /// `epoch * interval` overflow. Unreachable at any real epoch: readers only
    /// compare a real height against it or subtract a margin from it, both of which
    /// degrade to "not yet".
    fn epoch_start(&self, epoch: u64) -> u64 {
        self.epocher
            .first(Epoch::new(epoch))
            .map_or(u64::MAX, |h| h.get())
    }

    /// Append `epoch`'s journal records, so a restart can `resume` and a post-boundary
    /// member can recompute its share. Each record is fsync-durable on a successful
    /// return; a write failure warns and yields `false`. The durable-before-ack gate
    /// uses that: an `Ack` whose `ReceivedDealing` did not land durably is withheld,
    /// so a QUAL log can never record an ack this node cannot back after a restart.
    ///
    /// A caller that can retry gets the failed records back from
    /// [`Self::journal_failures`]; attributing a failed log is the caller's job, from
    /// the `(dealer, hash)` the ceremony authenticated.
    #[must_use]
    fn append_journal(&self, epoch: u64, records: Vec<JournalRecord>) -> bool {
        self.journal_failures(epoch, records).is_empty()
    }

    /// [`Self::append_journal`], handing back the records whose write did not land,
    /// in order, so a caller that can retry them from memory does.
    #[must_use]
    fn journal_failures(&self, epoch: u64, records: Vec<JournalRecord>) -> Vec<JournalRecord> {
        let mut failed = Vec::new();
        for record in records {
            if let Err(err) =
                share_state::append_journal(&self.share_dir, epoch, &record, &self.share_state)
            {
                tracing::warn!(
                    epoch,
                    ?err,
                    "live DKG: failed to journal ceremony record (in-memory ceremony \
                     unaffected; dependent ack withheld — the dealer reveals our point)"
                );
                failed.push(record);
            }
        }
        failed
    }

    fn evict_journal(&self, epoch: u64) {
        share_state::evict_journal(&self.share_dir, epoch);
    }

    /// Delete a swept epoch's `Conflict` marker: an epoch past the window has no slot
    /// to be terminal in.
    fn evict_conflict(&self, epoch: u64) {
        share_state::evict_conflict(&self.share_dir, epoch);
    }

    /// Run until the clock, the network receiver or the resolver channel closes.
    /// `clock` is the marshal's ordering tip; the actor derives epoch transitions and
    /// the seal deadline from it.
    pub async fn run(
        mut self,
        mut clock: tokio::sync::watch::Receiver<u64>,
        mut rng: impl CryptoRngCore,
    ) {
        tracing::info!(epocher = ?self.epocher, "live DKG: actor started");
        loop {
            tokio::select! {
                // A watch, not a stream: `changed()` wakes once per unseen publish and
                // `borrow_and_update` takes the newest value, so a burst of tips costs
                // one `on_height` at the highest of them.
                changed = clock.changed() => match changed {
                    Ok(()) => {
                        let height = *clock.borrow_and_update();
                        self.on_height(height, &mut rng).await
                    }
                    Err(_) => break,
                },
                msg = self.receiver.recv() => match msg {
                    Ok((from, buf)) => self.on_message(from, buf.as_ref(), &mut rng).await,
                    Err(_) => break,
                },
                req = self.resolver_rx.recv() => match req {
                    Some(msg) => self.on_resolver_message(msg, &mut rng).await,
                    // The resolver engine exited. It is a supervised child of the beacon
                    // plane and the node goes down with it, so there is no gossip-only
                    // life to degrade into: `run` returning is what the supervisor sees.
                    None => {
                        tracing::error!(
                            target: "dpos::beacon",
                            "live DKG: the dealer-log resolver engine exited — its inbound \
                             channel closed; the resolver is a supervised child of the \
                             beacon plane and the node stops with it, so the actor stops here"
                        );
                        break;
                    }
                },
                // Answer an agreement instance's question about a candidate pinned set:
                // a closed channel means every instance is gone, so the branch parks.
                req = recv_or_park(&mut self.pinned_rx) => {
                    let verdict = self.derive_pinned(&req, &mut rng);
                    drop(req.response.send(verdict));
                },
                // The write-back edge, independent of the height stream: with the chain
                // halted at `epoch_start(E+1)` the height clock stops, and the artifact
                // is what starts the epoch's key moving again.
                artifact = recv_or_park(&mut self.artifacts_rx) => {
                    self.on_artifact(artifact, &mut rng).await;
                },
                // The instance's other verdict: a certified body it could not resolve.
                // The heal is a pull from peers, started here and not at the boundary.
                epoch = recv_or_park(&mut self.body_lost_rx) => self.on_body_lost(epoch),
            }
        }
    }

    /// The phase of `epoch`, if decided.
    fn state(&self, epoch: u64) -> Option<&EpochState> {
        self.epochs.get(&epoch).map(|slot| &slot.state)
    }

    /// The live ceremony of `epoch`, in whichever phase carries one.
    fn ceremony(&self, epoch: u64) -> Option<&DkgCeremony> {
        self.state(epoch).and_then(EpochState::ceremony)
    }

    fn ceremony_mut(&mut self, epoch: u64) -> Option<&mut DkgCeremony> {
        self.epochs
            .get_mut(&epoch)
            .and_then(|slot| slot.state.ceremony_mut())
    }

    fn ceremonies(&self) -> impl Iterator<Item = (u64, &DkgCeremony)> {
        self.epochs
            .iter()
            .filter_map(|(e, slot)| slot.state.ceremony().map(|c| (*e, c)))
    }

    /// Decide `epoch` from a test-built ceremony: `Dealing` while its dealer is live,
    /// `Sealed` once it has closed, as `recover` does from a resumed journal.
    #[cfg(test)]
    fn insert_ceremony(&mut self, epoch: u64, ceremony: DkgCeremony) {
        let state = if ceremony.dealing_closed() {
            EpochState::Sealed { ceremony }
        } else {
            EpochState::Dealing {
                ceremony,
                agreed: None,
            }
        };
        self.epochs.insert(epoch, EpochSlot::new(state));
    }

    /// The phase's name, for assertions on the state machine.
    #[cfg(test)]
    fn phase(&self, epoch: u64) -> Option<&'static str> {
        self.state(epoch).map(EpochState::name)
    }

    /// The `Stalled{reason}` latches raised on `epoch`.
    #[cfg(test)]
    fn stalls(&self, epoch: u64) -> BTreeSet<StallReason> {
        self.epochs
            .get(&epoch)
            .map(|slot| slot.stalled.clone())
            .unwrap_or_default()
    }

    /// The equivocation evidence held for `epoch`.
    #[cfg(test)]
    fn evidence(&self, epoch: u64) -> BTreeMap<PeerPubkey, DealerEquivocation> {
        self.epochs
            .get(&epoch)
            .map(|slot| slot.evidence.clone())
            .unwrap_or_default()
    }

    /// Decide `epoch`: insert its slot, log it once, and raise the latch a terminal
    /// carries. A slot that already stands is a transition instead (`decide` guards on
    /// `contains_key`, so no production caller hits that): its latches and gauges are
    /// kept by [`Self::set_state`].
    fn enter(&mut self, epoch: u64, state: EpochState) {
        let latch = match &state {
            EpochState::Unrecoverable { .. } => Some(StallReason::Unrecoverable),
            EpochState::Conflict { .. } => Some(StallReason::Conflict),
            _ => None,
        };
        if self.epochs.contains_key(&epoch) {
            self.set_state(epoch, state);
        } else {
            tracing::info!(
                target: "dpos::beacon",
                epoch,
                state = state.name(),
                height = self.height_now(),
                "live DKG: epoch decided"
            );
            self.epochs.insert(epoch, EpochSlot::new(state));
        }
        // A resumed ceremony carries its journaled evidence pairs; the slot's copy is
        // what outlives the ceremony.
        if let Some(slot) = self.epochs.get_mut(&epoch) {
            if let Some(c) = slot.state.ceremony() {
                let pairs: Vec<_> = c
                    .equivocations()
                    .iter()
                    .map(|(d, p)| (d.clone(), *p))
                    .collect();
                slot.evidence.extend(pairs);
            }
        }
        if let Some(reason) = latch {
            self.stall(epoch, reason);
        }
    }

    /// Move a decided `epoch` to `state`, dropping the latches the new phase no longer
    /// carries ([`carries`]) with their gauge step, and the plane's announcement mark
    /// with `Sealed` / `Agreed`.
    fn set_state(&mut self, epoch: u64, state: EpochState) {
        let Some(slot) = self.epochs.get_mut(&epoch) else {
            return;
        };
        tracing::debug!(
            target: "dpos::beacon",
            epoch,
            from = slot.state.name(),
            to = state.name(),
            "live DKG: epoch transition"
        );
        slot.state = state;
        if !matches!(
            slot.state,
            EpochState::Sealed { .. } | EpochState::Agreed { .. }
        ) {
            slot.announced = false;
        }
        let left: Vec<StallReason> = slot
            .stalled
            .iter()
            .copied()
            .filter(|reason| !carries(&slot.state, *reason))
            .collect();
        for reason in left {
            slot.stalled.remove(&reason);
            self.metrics.stall_cleared(reason);
        }
    }

    /// Take `epoch`'s phase out for a by-value transition; the caller must put a
    /// phase back before its input returns. The slot keeps its latches and evidence
    /// in between; [`EpochState::InTransition`] stands in meanwhile and
    /// [`Self::debug_assert_settled`] checks it gone.
    fn take_state(&mut self, epoch: u64) -> Option<EpochState> {
        self.epochs
            .get_mut(&epoch)
            .map(|slot| std::mem::replace(&mut slot.state, EpochState::InTransition))
    }

    /// Put a taken phase back unchanged, for a plan that found a phase other than the
    /// one it selected. Not a transition: no line, no latch change.
    fn put_back(&mut self, epoch: u64, state: EpochState) {
        if let Some(slot) = self.epochs.get_mut(&epoch) {
            slot.state = state;
        }
    }

    /// No slot is left mid-transition once an input has been handled.
    fn debug_assert_settled(&self) {
        debug_assert!(
            !self
                .epochs
                .values()
                .any(|slot| matches!(slot.state, EpochState::InTransition)),
            "an epoch slot was taken and never put back"
        );
    }

    fn stored(&self, epoch: u64) -> Option<StoredArtifact> {
        (self.outcome_at)(epoch)
    }

    /// Raise the `Stalled{reason}` latch on `epoch` — one line per `(epoch, reason)`,
    /// ERROR when only peer action can move the epoch (`QuorumMissing`, `Conflict`)
    /// and WARN otherwise.
    fn stall(&mut self, epoch: u64, reason: StallReason) {
        let Some(slot) = self.epochs.get_mut(&epoch) else {
            return;
        };
        if !slot.stalled.insert(reason) {
            return;
        }
        let state = slot.state.name();
        self.metrics.stalled(reason);
        match reason {
            StallReason::QuorumMissing | StallReason::Conflict => tracing::error!(
                target: "dpos::beacon",
                epoch,
                state,
                reason = reason.as_str(),
                "live DKG: epoch stalled"
            ),
            _ => tracing::warn!(
                target: "dpos::beacon",
                epoch,
                state,
                reason = reason.as_str(),
                "live DKG: epoch stalled"
            ),
        }
    }

    /// Drop one latch whose condition passed without a phase change; a phase-leaving
    /// latch is dropped by `set_state` ([`carries`]).
    fn clear_stall(&mut self, epoch: u64, reason: StallReason) {
        let Some(slot) = self.epochs.get_mut(&epoch) else {
            return;
        };
        if slot.stalled.remove(&reason) {
            self.metrics.stall_cleared(reason);
        }
    }

    /// The instance for `epoch` certified a payload whose body never arrived. A
    /// `Sealed` epoch moves to acquiring and pulls at once, then on every tick
    /// (`drive_acquisition`); a still-`Dealing` epoch (a lagging clock) keeps the
    /// signal for its seal. Any other phase holds the artifact or never had a ceremony.
    fn on_body_lost(&mut self, epoch: u64) {
        if let Some(EpochSlot {
            state: EpochState::Dealing { .. },
            body_lost,
            ..
        }) = self.epochs.get_mut(&epoch)
        {
            *body_lost = true;
            tracing::info!(
                target: "dpos::beacon",
                epoch,
                "live DKG: body-lost signal while still dealing — kept for the seal"
            );
            return;
        }
        let Some(EpochState::Sealed { .. }) = self.state(epoch) else {
            tracing::debug!(
                target: "dpos::beacon",
                epoch,
                state = self.state(epoch).map(EpochState::name),
                "live DKG: body-lost signal for an epoch not waiting on its instance"
            );
            return;
        };
        let ceremony = match self.take_state(epoch) {
            Some(EpochState::Sealed { ceremony }) => ceremony,
            Some(other) => {
                self.put_back(epoch, other);
                return;
            }
            None => return,
        };
        self.set_state(
            epoch,
            EpochState::Acquiring(Acquire::ArtifactForCeremony(Box::new(ceremony))),
        );
        self.stall(epoch, StallReason::BodyLost);
        (self.pull_artifact)(epoch);
        self.debug_assert_settled();
    }

    /// What this node can say about a candidate pinned dealer-log set for
    /// `req.epoch` — the production side of [`PinnedLogs`].
    ///
    /// An unreadable roster and an absent ceremony are properties of this node, so
    /// both answer [`PinnedDerive::Unavailable`] and the caller parks; a rejection may
    /// rest on [`PinnedDerive::Unusable`] alone, the one arm that is a property of the
    /// set every honest node reaches alike.
    ///
    /// A ceremony that already finalized has been consumed, so this answers
    /// `Unavailable` from then on — contract-correct and free, since a node that
    /// finalized has the key it was voting to agree.
    fn derive_pinned(&self, req: &PinnedRequest, rng: &mut impl CryptoRngCore) -> PinnedDerive {
        let Some(committee) = (self.committee_for)(req.epoch) else {
            return PinnedDerive::Unavailable;
        };
        let Some(ceremony) = self.ceremony(req.epoch) else {
            return PinnedDerive::Unavailable;
        };
        ceremony.derive_pinned(rng, &committee, &req.pinned)
    }

    /// Take an agreed artifact as this epoch's pinned dealer-log set and drive the
    /// write-back to the point where the existing rails carry it: the artifact is the
    /// only source of the set, so no second finalize path can pick a different one
    /// (`finalize_over_pinned` → store insert → `share_notify` → the epoch manager).
    ///
    /// The finalize is driven here and the missing pinned bodies fetched here, both of
    /// which `on_height` would otherwise do: with the chain halted neither would run
    /// again.
    async fn on_artifact(&mut self, artifact: AgreedArtifact, rng: &mut impl CryptoRngCore) {
        let epoch = artifact.0.target_epoch;
        if !self.epochs.contains_key(&epoch) {
            // No clock before the first tick, so `decide` cannot place the epoch against
            // its seal deadline (a resume could re-deal an epoch already past it); the
            // store already holds the artifact, and the first tick's `recover` reads it.
            if self.last_height.is_none() {
                tracing::debug!(
                    target: "dpos::beacon",
                    epoch,
                    "live DKG: artifact before the first height tick; recovered from the \
                     store on the first tick"
                );
                return;
            }
            let mut out = Vec::new();
            self.decide(epoch, &mut out).await;
            self.broadcast_all(out).await;
        }
        // An epoch outside `decide`'s window (or still undecidable) has no slot to take
        // the set; the store holds the artifact and `reconcile_with_store` applies it on
        // the tick the epoch is decided.
        if self.apply_artifact(epoch, &artifact.0) {
            self.drive_finalization(rng);
            self.fetch_missing_logs().await;
        }
        self.debug_assert_settled();
    }

    /// The artifact edge of the transition table, on a decided epoch. Returns
    /// whether a ceremony now stands on the set (the caller finalizes).
    ///
    /// A phase already standing on a value ignores a re-delivery of it; a different
    /// value is `Conflict` — both passed the committee-quorum check, so two of them means
    /// ≥ 2q−n Byzantine signers and the epoch's signing stops. The phase's own value is
    /// compared first, then the store's (first-wins and quorum-checked), so a hand-off
    /// this actor never received still bars the second value.
    fn apply_artifact(&mut self, epoch: u64, proposal: &DkgProposal) -> bool {
        // One guard for both intake rails (the channel and the store read): an artifact
        // naming no dealer logs is nothing to finalize over. A certified value cannot be
        // empty (`verify` rejects a set below the quorum); kept so the rails cannot diverge.
        if proposal.logs.is_empty() {
            tracing::warn!(
                target: "dpos::beacon",
                epoch,
                "live DKG: an agreed artifact names no dealer logs; nothing to finalize over"
            );
            return false;
        }
        let digest = value_digest(proposal);
        if let Some(EpochState::Conflict { held, second, key }) = self.state(epoch) {
            let (held, second, key) = (*held, *second, *key);
            if digest != held && digest != second {
                tracing::warn!(
                    target: "dpos::beacon",
                    epoch,
                    third = %digest,
                    "live DKG: a third quorum-certified artifact for a conflicted epoch"
                );
            }
            // A `Conflict` with no key yet takes the first artifact to arrive as the key
            // to verify with; the signing stays stopped — only `key` changes.
            if key.is_none() {
                tracing::info!(
                    target: "dpos::beacon",
                    epoch,
                    key = %digest,
                    "live DKG: a conflicted epoch acquired an artifact to verify with; \
                     its signing stays stopped"
                );
                self.set_state(
                    epoch,
                    EpochState::Conflict {
                        held,
                        second,
                        key: Some(digest),
                    },
                );
            }
            return false;
        }
        if let Some(held) = self.state(epoch).and_then(EpochState::held_digest) {
            if held != digest {
                self.conflict(epoch, held, digest);
            }
            return false;
        }
        if let Some(stored) = self.stored(epoch) {
            let held = value_digest(&stored.held);
            if held != digest {
                self.conflict(epoch, held, digest);
                return false;
            }
        }
        let Some(state) = self.take_state(epoch) else {
            return false;
        };
        let (next, finalizable, stalled) = match state {
            EpochState::Dealing {
                ceremony,
                agreed: None,
            } => (
                EpochState::Dealing {
                    ceremony,
                    agreed: Some(AgreedSet::of(proposal)),
                },
                false,
                None,
            ),
            EpochState::Sealed { ceremony } => (
                EpochState::Agreed {
                    ceremony,
                    set: AgreedSet::of(proposal),
                },
                true,
                None,
            ),
            EpochState::Acquiring(Acquire::ArtifactForCeremony(ceremony)) => (
                EpochState::Agreed {
                    ceremony: *ceremony,
                    set: AgreedSet::of(proposal),
                },
                true,
                None,
            ),
            // The held share is keyed only if it lies on the artifact's polynomial. An
            // unreadable committee keeps the phase: the store holds the artifact and the
            // next tick re-applies it.
            EpochState::Acquiring(Acquire::ArtifactForShare) => match (self.committee_for)(epoch) {
                Some(committee) => {
                    let (next, stalled) =
                        self.key_held_share(epoch, &committee, AgreedSet::of(proposal));
                    (next, false, stalled)
                }
                None => (
                    EpochState::Acquiring(Acquire::ArtifactForShare),
                    false,
                    None,
                ),
            },
            EpochState::Acquiring(Acquire::ArtifactForKey) => (
                EpochState::KeyOnly {
                    digest: Some(digest),
                },
                false,
                None,
            ),
            // An unrecoverable member still verifies with the key.
            EpochState::Unrecoverable { key: None } => {
                (EpochState::Unrecoverable { key: Some(digest) }, false, None)
            }
            // A carry-forward `KeyOnly` (no digest) has nothing to pin.
            other => (other, false, None),
        };
        if finalizable {
            tracing::info!(
                target: "dpos::beacon",
                epoch,
                pinned = proposal.logs.len(),
                height = self.height_now(),
                "live DKG: adopting the agreed dealer-log set as this epoch's pinned set"
            );
        }
        self.set_state(epoch, next);
        if let Some(reason) = stalled {
            self.stall(epoch, reason);
        }
        finalizable
    }

    /// Key `epoch` over a share this node already holds (reloaded from its share file,
    /// or kept through a partial success) and the artifact just learned for it: `Keyed`
    /// iff the share lies on the artifact's polynomial at this node's index — the gate
    /// `adopt_share` applies on every other way in. A share that fails it leaves the
    /// store and its file (a restart must not re-key it), and the epoch heals over its
    /// retained journal (`Acquiring(Logs)`) with the caller raising
    /// `Stalled{OffPolynomial}`; a share no longer held is the same heal without the stall.
    fn key_held_share(
        &mut self,
        epoch: u64,
        committee: &Set<PeerPubkey>,
        set: AgreedSet,
    ) -> (EpochState, Option<StallReason>) {
        let share = self
            .store
            .read()
            .ok()
            .and_then(|store| store.get(&epoch).cloned());
        match share {
            Some(share) if self.share_on_artifact(epoch, committee, &set.group_key, &share) => {
                (EpochState::Keyed { digest: set.digest }, None)
            }
            Some(_) => {
                self.drop_share(epoch);
                (
                    EpochState::Acquiring(Acquire::Logs(Box::new(
                        self.heal_over(epoch, committee, set),
                    ))),
                    Some(StallReason::OffPolynomial),
                )
            }
            None => (
                EpochState::Acquiring(Acquire::Logs(Box::new(
                    self.heal_over(epoch, committee, set),
                ))),
                None,
            ),
        }
    }

    /// Does `share` lie on the certified `outcome`'s polynomial at this node's index
    /// ([`validate_share_on_poly`])? Counted and said (ERROR) on a refusal; the caller
    /// decides what the refusal moves.
    fn share_on_artifact(
        &mut self,
        epoch: u64,
        committee: &Set<PeerPubkey>,
        outcome: &DkgOutcome,
        share: &Share,
    ) -> bool {
        if validate_share_on_poly(outcome, committee, &self.me_key.public_key(), share) {
            return true;
        }
        self.metrics.dkg_share_off_polynomial.inc();
        tracing::error!(
            target: "dpos::beacon",
            epoch,
            share_index = %share.index,
            players = outcome.players().len(),
            committee = committee.len(),
            "live DKG: REFUSING a share that does not lie on its own epoch's \
             polynomial at this node's index — nothing is stored and the epoch \
             stays verify-only (adopting it would produce seed partials every \
             honest peer rejects, subtracting this node from the quorum silently)"
        );
        false
    }

    /// Take this node's share for `epoch` out of both places it lives: the shared
    /// `store` (dropping it demotes this node to verify-only at the epoch manager's next
    /// reconcile, which the notify wakes) and its file (a restart must not reload it).
    /// Returns whether `store` held one.
    fn drop_share(&mut self, epoch: u64) -> bool {
        let mut dropped = false;
        if let Ok(mut store) = self.store.write() {
            dropped = store.remove(&epoch).is_some();
        }
        if dropped {
            self.share_notify.notify_one();
        }
        share_state::evict_share(&self.share_dir, epoch);
        dropped
    }

    /// Two different quorum-certified artifacts for one epoch: the epoch is `Conflict`
    /// and its signing stops ([`Self::stop_signing`]); the ceremony, if any, is dropped
    /// with its recorded logs left servable, and nothing is reported on-chain.
    fn conflict(&mut self, epoch: u64, held: B256, second: B256) {
        let Some(state) = self.take_state(epoch) else {
            return;
        };
        if let Some(logs) = match state {
            EpochState::Dealing { mut ceremony, .. }
            | EpochState::Sealed { mut ceremony }
            | EpochState::Agreed { mut ceremony, .. } => Some(ceremony.take_signed_logs()),
            EpochState::Acquiring(Acquire::ArtifactForCeremony(mut ceremony)) => {
                Some(ceremony.take_signed_logs())
            }
            _ => None,
        } {
            self.log_store.seed(epoch, logs);
        }
        self.stop_signing(epoch, held, second);
        // `held` is a value this actor stood on or read from the store, and both
        // producers insert before they hand off, so the store holds a key to verify
        // with; what a restart finds is `recover`'s question.
        self.set_state(
            epoch,
            EpochState::Conflict {
                held,
                second,
                key: Some(held),
            },
        );
        self.stall(epoch, StallReason::Conflict);
    }

    /// The durable half of `Conflict(E)`, in the one order safe under a death at any
    /// point: the verdict is on disk (`share_state::persist_conflict`, skipped if a
    /// marker is already there — the store writes one as it notes the second value)
    /// before the share leaves the store and its file. `recover` reads that marker
    /// first, so a death in between restarts as `Conflict` with the share evicted,
    /// never re-deriving the share over the journal for an epoch this node stopped
    /// signing. A marker that cannot be written is counted and said (ERROR) and the
    /// share is dropped regardless: the verdict holds here and the store's witness
    /// re-judges the epoch on a restart.
    fn stop_signing(&mut self, epoch: u64, held: B256, second: B256) {
        let durable = match share_state::load_conflict(&self.share_dir, epoch) {
            Some(_) => true,
            None => match share_state::persist_conflict(&self.share_dir, epoch, &held, &second) {
                Ok(()) => true,
                Err(err) => {
                    self.metrics.dkg_conflict_marker_failed.inc();
                    tracing::error!(
                        target: "dpos::beacon",
                        epoch,
                        ?err,
                        "live DKG: could not write the conflict marker; the verdict holds \
                         in this process and a restart re-judges the epoch from the store"
                    );
                    false
                }
            },
        };
        #[cfg(test)]
        if self.die_between_verdict_and_eviction {
            return;
        }
        let share_dropped = self.drop_share(epoch);
        self.metrics.dkg_artifact_conflict.inc();
        tracing::error!(
            target: "dpos::beacon",
            epoch,
            held = %held,
            second = %second,
            share_dropped,
            durable,
            "live DKG: TWO different quorum-certified artifacts for one epoch — the epoch's \
             signing is stopped on this node (≥ 2q−n signers certified both)"
        );
    }

    /// Announce every target whose dealing has closed to the agreement plane.
    ///
    /// Repeated on every tick, and the plane deduplicates: a one-shot announcement
    /// would be lost whenever the launcher cannot act on it yet — an unreadable
    /// `committee[epoch]`, a sub-channel registration that lost a race. Only the log
    /// line is one-shot.
    async fn announce_agreement_targets(&mut self) {
        let due: Vec<u64> = self
            .epochs
            .iter()
            .filter(|(_, slot)| {
                matches!(
                    slot.state,
                    EpochState::Sealed { .. } | EpochState::Agreed { .. }
                )
            })
            .map(|(e, _)| *e)
            .collect();
        for epoch in due {
            match self.agreement_tx.try_send(epoch) {
                Ok(()) => {
                    let Some(slot) = self.epochs.get_mut(&epoch) else {
                        continue;
                    };
                    if !slot.announced {
                        slot.announced = true;
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

    /// Adopt `(outcome, share)` for `epoch`: self-check first, then persist, then the
    /// in-memory insert, the waiter last.
    ///
    /// The self-check ([`validate_share_on_poly`], this node's local fork-safety gate)
    /// runs here rather than at a call site, so an adoption through this function
    /// cannot skip it: `committee` is a required parameter. A share off the epoch's
    /// polynomial is never adopted with a warning: honest peers reject its partials, so
    /// adopting one subtracts this node from the quorum while it believes it signs, and
    /// the epoch stays verify-only instead. Both callers already hold the epoch's
    /// committee, so an unreadable one stops the plan before it reaches here.
    ///
    /// A failed persist is the second refusal, for the same reason: signing with a
    /// share no restart can reload commits this node to partials it cannot reproduce.
    /// The retry is a transition, not a timer: the caller moves the epoch to
    /// `Acquiring(Logs)` and the next tick re-derives the share from the retained
    /// journal, which is kept precisely because eviction follows a successful adopt.
    ///
    /// Returns why the share was refused, so the caller raises the matching
    /// `Stalled{reason}` instead of inferring it from the store.
    ///
    /// `persist` must run before the in-memory insert, or a mid-epoch restart reloads
    /// no share and carry-forwards a wrong key. The order is enforced by ownership,
    /// not by care — `insert` moves the pair while `persist` borrows it, so a swap is
    /// a use-after-move — which is also why both stay owned parameters; by reference
    /// the order is back with whoever edits this next.
    ///
    /// The waiter wake-up is last because it is the only step other tasks can observe.
    /// `notify_one` stores a permit when no waiter is armed, so a share landing between
    /// the consumer's reconcile and its re-arm is not lost (`EpochManager::run` is the
    /// only waiter).
    fn adopt_share(
        &mut self,
        epoch: u64,
        committee: &Set<PeerPubkey>,
        outcome: CeremonyOutput,
        share: Share,
    ) -> Result<(), AdoptRefusal> {
        if !self.share_on_artifact(epoch, committee, &outcome, &share) {
            return Err(AdoptRefusal::OffPolynomial);
        }
        if let Err(err) = share_state::persist(&self.share_dir, epoch, &share, &self.share_state) {
            self.metrics.dkg_share_persist_failed.inc();
            tracing::error!(
                target: "dpos::beacon",
                epoch,
                ?err,
                "live DKG: REFUSING the share because its disk write failed — the \
                 epoch stays verify-only and the write is retried from the retained \
                 journal on the next height tick (adopting it would sign now and be \
                 mute after a restart)"
            );
            return Err(AdoptRefusal::PersistFailed);
        }
        #[cfg(test)]
        if let Ok(mut seen) = self.adopted_outcomes.write() {
            seen.insert(epoch, outcome.clone());
        }
        // The store insert is this node's qualified verdict for the epoch; there is no
        // separate certificate artifact.
        if let Ok(mut store) = self.store.write() {
            store.insert(epoch, share);
        }
        self.share_notify.notify_one();
        self.metrics.dkg_ceremony_ok.inc();
        Ok(())
    }

    /// Age every per-epoch map out on one window, then reclaim the journals of the
    /// epochs that left.
    ///
    /// Without this sweep an epoch whose ceremony sealed but never reached a quorum
    /// lingers in `epochs` forever: `drive_finalization` never moves it on.
    ///
    /// One window for every phase, because it is the exact bound of usefulness. The
    /// only finalize left runs over an artifact-sourced pinned set, which ages out on
    /// this same window: a ceremony kept past it could never be finalized, and one
    /// kept short of it is the halt the window exists to prevent — an agreement that
    /// has not converged when the chain enters its target still asks this actor for
    /// the bodies (`derive_pinned`), and a swept ceremony answers `Unavailable`
    /// forever, parking every `verify` and leaving `build_proposal` with nothing to
    /// pin. That instance is aborted only once the epoch clock enters `target + 1`
    /// (`dkg_engine::prune_agreements`, on the clock this tick publishes), a full
    /// epoch inside this window. A halted chain sweeps nothing at all: the caller runs
    /// only on a verified finalization.
    ///
    /// The journal and the serve store ride the same window, because a demoted member
    /// (or a peer it serves) can still recompute E's share while E is
    /// committee-relevant. [`DealerLogStore::retain`] ages its own map out and hands
    /// back the epochs it dropped, which the actor cannot enumerate itself; the
    /// journal reclaim stays here because `share_dir` is the actor's.
    ///
    /// This is the only place the window is applied: a new per-epoch fact opts in by
    /// living in the slot, and the stall gauges step down with the slots that carried
    /// them.
    fn sweep_epoch_state(&mut self, now: u64) {
        let floor = now.saturating_sub(JOURNAL_RETENTION_EPOCHS);
        let retained = |e: u64| e + JOURNAL_RETENTION_EPOCHS >= now;
        let mut evictable: Vec<u64> = self.log_store.retain(floor);
        let aged: Vec<u64> = self
            .epochs
            .keys()
            .copied()
            .filter(|e| !retained(*e))
            .collect();
        for e in aged {
            if let Some(slot) = self.epochs.remove(&e) {
                for reason in slot.stalled {
                    self.metrics.stall_cleared(reason);
                }
            }
            evictable.push(e);
        }
        for e in &evictable {
            self.evict_journal(*e);
            self.evict_conflict(*e);
        }
        // Share-confirmations are per-target scratch: useful only while that target's
        // agreement can still run, so they age on the same predicate as its slot.
        self.confirmations.retain(floor);
        // A non-durable log is retryable only while its ceremony holds the bytes, so
        // these sets cannot outlive the ceremonies they name.
        self.nondurable_logs.retain(|e, _| retained(*e));
        self.nondurable_dealings.retain(|e, _| retained(*e));
        if let Ok(mut m) = self.recorded_dkg_logs.write() {
            m.retain(|e, _| retained(*e));
        }

        // Retain every mint at or above the floor `ceremony_retain_floor` derives; a
        // size cap could drop one still in force for the oldest cert in the window.
        if let Ok(mut store) = self.store.write() {
            let floor =
                ceremony_retain_floor(store.keys().copied(), now, SCHEME_RETENTION_EPOCHS as u64);
            if floor > 0 {
                store.retain(|mint, _| *mint >= floor);
            }
        }
    }

    async fn on_height(&mut self, height: u64, rng: &mut impl CryptoRngCore) {
        // One feeder drives this clock (the marshal's ordering tip, which only ever
        // rises), so the clamp is a guard for hand-driven tests, not a merge.
        let height = self.height_now().max(height);
        self.last_height = Some(height);
        // Gauged here, off the clamp, not by the watch's writer: this is the clock the
        // ceremony geometry runs on, and an actor that stopped draining shows up as the
        // ordering gauge pulling away from this one.
        self.plane_clock.record_dkg_clock(height);
        let now = self.epoch_of(height);

        // First tick only, and driven off the on-disk filename rather than the in-memory
        // maps: a finalize-then-restart-before-boundary holds the epoch in no map, and
        // its leaked journal and stale at-rest secrets still have to be reclaimed.
        if !self.reconciled_journals {
            share_state::reconcile_journals(&self.share_dir, now);
            self.reconciled_journals = true;
        }

        let mut to_send: Vec<Outgoing> = Vec::new();

        // The window is re-decided on every tick while undecided, not once at the epoch
        // transition: committee[E+1] is committed on-chain sometime during E and can land
        // after the actor (driven by lagging finalized heights) first enters E, so a
        // single-shot check would see it unreadable and never deal, wedging the E+1
        // boundary block. A decided epoch is never re-decided, so the retry is idempotent.
        let started = self.decide_window(now, &mut to_send).await;

        let due: Vec<u64> = self
            .epochs
            .iter()
            .filter(|(e, slot)| {
                matches!(slot.state, EpochState::Dealing { .. })
                    && height >= self.epoch_start(**e).saturating_sub(DKG_MARGIN_BLOCKS)
            })
            .map(|(e, _)| *e)
            .collect();
        for e in due {
            let (mut ceremony, agreed) = match self.take_state(e) {
                Some(EpochState::Dealing { ceremony, agreed }) => (ceremony, agreed),
                Some(other) => {
                    self.put_back(e, other);
                    continue;
                }
                None => continue,
            };
            let step = ceremony.seal_dealings();
            to_send.extend(step.outgoing);
            // The seal broadcast carries no ack and the log is re-fetchable via the
            // resolver, so a failed journal write only warns.
            let _ = self.append_journal(e, step.journal);
            let body_lost = self.epochs.get(&e).is_some_and(|slot| slot.body_lost);
            let (next, stalled) = match agreed {
                Some(set) => {
                    tracing::info!(
                        target: "dpos::beacon",
                        epoch = e,
                        pinned = set.pinned.len(),
                        height,
                        "live DKG: adopting the agreed dealer-log set as this epoch's pinned set"
                    );
                    (EpochState::Agreed { ceremony, set }, None)
                }
                // A body-lost signal that arrived while dealing is honoured at the seal:
                // the ceremony goes straight to acquiring, and this tick's
                // `drive_acquisition` pulls for it.
                None if body_lost => (
                    EpochState::Acquiring(Acquire::ArtifactForCeremony(Box::new(ceremony))),
                    Some(StallReason::BodyLost),
                ),
                None => (EpochState::Sealed { ceremony }, None),
            };
            self.set_state(e, next);
            if let Some(reason) = stalled {
                self.stall(e, reason);
            }
        }

        // An epoch past its dealing — decided, or in the past — can never start, so its
        // buffers go; an undecided epoch can still start, so its buffer stays.
        self.pending.retain(|e, _| {
            *e > now
                && matches!(
                    self.epochs.get(e),
                    None | Some(EpochSlot {
                        state: EpochState::Dealing { .. },
                        ..
                    })
                )
        });

        // Also driven from `on_message` and the artifact intake, so a Reveal that
        // completes the pinned set finalizes without waiting for a tick.
        self.drive_finalization(rng);

        // Runs after `drive_finalization`, which publishes the set being confirmed; only
        // a target whose body-checked set grew since the last confirmation is minted.
        to_send.extend(self.confirmations.mint(ConfirmTrigger::AnyGrowth));

        // Ask the plane for an instance for every target whose dealing has closed,
        // after `drive_finalization` — so an epoch finalized on this tick is not
        // announced at all.
        self.announce_agreement_targets().await;

        // Runs after `drive_finalization` too, so a ceremony finalizable on the boundary
        // tick is completed first and never evicted out from under it.
        self.sweep_epoch_state(now);

        // The announcement and this cutoff ride two channels, so the launcher may see
        // them in either order: a target at or above `now` is never touched by this
        // cutoff, and an instance for an epoch the clock has already passed is retired
        // on the launcher's next clock edge.
        self.epoch_clock.send_if_modified(|v| {
            if *v != now {
                *v = now;
                true
            } else {
                false
            }
        });

        // Re-send each un-acked dealing on every pre-seal tick until acks drain, bounded
        // because each ceremony's `unsent` shrinks as acks land. A ceremony started this
        // tick is skipped: its initial send is already in `to_send`.
        for (e, c) in self.ceremonies() {
            if !started.contains(&e) {
                to_send.extend(c.retransmit());
            }
        }

        self.broadcast_all(to_send).await;

        // Ask peers for the artifact each `Acquiring` epoch lacks — the member without a
        // share, the member with a share and no artifact, the non-member that needs
        // `PK_E` — and attempt the journal recompute for each one holding every pinned
        // body. Before the log fetch, so the tick's two network legs stay together.
        self.drive_acquisition(now, rng);

        // Hand the resolver the missing `{epoch, dealer, hash}` keys for any open,
        // shorthanded ceremony each tick; it owns retry, multi-peer fallback,
        // rate-limiting and dedup, so this stays a thin call.
        self.fetch_missing_logs().await;
        self.debug_assert_settled();
    }

    /// Re-attempt the journal write for every log this node holds but could not make
    /// durable. Rides the publish edge, no timer; an epoch whose ceremony has been
    /// swept has nothing left to re-journal, and the sweep drops its entry.
    fn retry_nondurable_journals(&mut self) {
        // The dealing records are held here, so each entry is one append and only the
        // still-failed ones go back in the queue.
        let dealings = std::mem::take(&mut self.nondurable_dealings);
        for (epoch, records) in dealings {
            if self.ceremony(epoch).is_none() {
                continue;
            }
            let failed = self.journal_failures(epoch, records);
            if !failed.is_empty() {
                self.nondurable_dealings.insert(epoch, failed);
            }
        }
        if self.nondurable_logs.is_empty() {
            return;
        }
        let pending: Vec<(u64, Vec<LogId>)> = self
            .nondurable_logs
            .iter()
            .map(|(e, set)| (*e, set.iter().cloned().collect()))
            .collect();
        for (epoch, ids) in pending {
            for id in ids {
                // The record that backs this log — an equivocation pair's second half,
                // else a `PeerLog` — so a retry never downgrades a pair to a lone log.
                let Some(record) = self.ceremony(epoch).and_then(|c| c.journal_record_for(&id))
                else {
                    continue;
                };
                if self.append_journal(epoch, vec![record]) {
                    if let Some(set) = self.nondurable_logs.get_mut(&epoch) {
                        set.remove(&id);
                    }
                }
            }
        }
        self.nondurable_logs.retain(|_, set| !set.is_empty());
    }

    /// Publish each live ceremony's recorded dealer-log hashes (`idx` = the dealer's
    /// position in the agreed `committee[epoch]`) into the shared `recorded_dkg_logs`,
    /// the index the agreement plane proposes from and a share-confirmation states.
    /// Monotone and idempotent: a seat once published is never replaced.
    fn publish_recorded_logs(&mut self) {
        // Retry the failed journal writes first, so a log that lands here is
        // publishable on this same pass.
        self.retry_nondurable_journals();
        let Ok(mut map) = self.recorded_dkg_logs.write() else {
            return;
        };
        let mut grew = false;
        for (e, c) in self.ceremonies() {
            let Some(committee) = (self.committee_for)(e) else {
                continue;
            };
            let nondurable = self.nondurable_logs.get(&e);
            for (idx, pk) in committee.iter().enumerate() {
                let Some(hash) = c.signed_log_hash(pk) else {
                    continue;
                };
                // Not claimed: this node could not back the hash after a restart, and
                // both the agreement plane and `Confirmations::mint` speak from this index.
                if nondurable.is_some_and(|set| set.contains(&(pk.clone(), hash))) {
                    continue;
                }
                // First-wins per seat enforced here, not merely relied on from
                // `signed_log_hash`'s stability: consumers read the index as append-only,
                // so a published seat is never overwritten.
                if let std::collections::btree_map::Entry::Vacant(seat) =
                    map.entry(e).or_default().entry(idx as u8)
                {
                    seat.insert(hash);
                    grew = true;
                }
            }
        }
        drop(map);
        // This is the only edge that grows the index, and the pool carries its wakeup
        // along with its own (`ConfirmPool::subscribe`): a parked leader whose last
        // missing input was a dealer log is woken here rather than sleeping out its view.
        if grew {
            if let Some(pool) = self.confirmations.pool() {
                pool.note_inputs_grew();
            }
        }
    }

    /// Record a peer's share-confirmation, or drop it.
    ///
    /// The pool re-verifies the signature against `committee[target_epoch][idx]`, so a
    /// relayed confirmation is as good as a directly-sent one; the seat check here is
    /// this consumer's, and a sender outside that roster is refused as `no_seat`. The
    /// unsigned envelope epoch must agree with the signed one, or a confirmation could
    /// slip past a receive-side epoch filter it does not bind.
    ///
    /// The window below is `[now, now + 2]` and nothing else, while
    /// [`Self::epoch_is_actionable`] also admits an epoch whose ceremony is still
    /// running past its own epoch — ceremonies are swept on a retention window, not at
    /// the boundary. A ceremony still open below `now` therefore gets its DKG frames
    /// through and its confirmations refused: the safe direction, since an epoch already
    /// entered has no agreement left to count them. Deliberate; do not align the two.
    fn on_confirm(&mut self, envelope_epoch: u64, from: &PeerPubkey, confirm: ShareConfirm) {
        let Some(pool) = self.confirmations.pool() else {
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
        // The entry bar counts only an epoch whose agreement is live or about to be
        // (`[now, now + 2]`), so outside it a confirmation is unusable — refused here
        // rather than after a committee resolve it would waste.
        //
        // There is no window before the first height tick, and a `0` floor must not
        // invent one: it would put the window at `[0, 2]` and permanently refuse every
        // confirmation on a chain past epoch 2 (`Confirmations::mint` is edge-triggered
        // on width growth, so a dropped confirmation is never re-issued). With no clock
        // the seat check below is the whole bound.
        if let Some(height) = self.last_height {
            if !within_ingress_window(self.epoch_of(height), confirm.target_epoch) {
                self.refuse(from, Some(confirm.target_epoch), "confirm_window");
                return;
            }
        }
        let Some(roster) = (self.committee_for)(confirm.target_epoch) else {
            return;
        };
        if roster.position(from).is_none() {
            self.refuse(from, Some(confirm.target_epoch), "no_seat");
            return;
        }
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

    /// Record an equivocation a ceremony step has just proved: the pair is kept in the
    /// epoch's slot as evidence, outliving the ceremony until the sweep. `durable` says
    /// whether its journal record landed — an unbacked pair is still proven here (the
    /// ban and the RAM copy hold), but a restart would lose it until the nondurable
    /// retry re-appends it.
    fn note_equivocation(&mut self, epoch: u64, dealer: Option<&PeerPubkey>, durable: bool) {
        let Some(dealer) = dealer else {
            return;
        };
        let Some(pair) = self
            .ceremony(epoch)
            .and_then(|c| c.equivocation(dealer).copied())
        else {
            return;
        };
        let Some(slot) = self.epochs.get_mut(&epoch) else {
            return;
        };
        slot.evidence.insert(dealer.clone(), pair);
        self.metrics.dkg_dealer_equivocation.inc();
        tracing::warn!(
            target: "dpos::beacon",
            epoch,
            %dealer,
            first = %pair.first,
            second = %pair.second,
            evidence = if durable { "journaled" } else { "nondurable" },
            "live DKG: dealer signed TWO distinct valid logs for this epoch — pair kept as \
             evidence, dealer locally banned from gossip for the epoch; the ceremony \
             finalizes over whichever of its logs the agreement pinned"
        );
    }

    /// Finalize every `Agreed` epoch whose pinned set is fully held and quorum-ready.
    ///
    /// The gate is the closed dealing, never this node's own log: a node whose log was
    /// never broadcast still recovers its share, and gating on the own log would block it
    /// forever — no peer holds that log to re-fetch.
    fn drive_finalization(&mut self, rng: &mut impl CryptoRngCore) {
        // The one publication edge: seal, Reveal and resolver ingest all funnel through
        // here, so a held log becomes claimable here and nowhere else.
        self.publish_recorded_logs();
        // Probe and finalize both run before the epoch's boundary block is
        // proposed/verified, so the verify-path share gate can read the share.
        // `(epoch, all_held, unmappable_pinned)`, collected before the reporting loop so
        // no `&mut self` call runs under the `epochs` borrow.
        let mut deferrals: Vec<(u64, bool, usize)> = Vec::new();
        let plans: Vec<(u64, Set<PeerPubkey>)> = self
            .epochs
            .iter()
            .filter_map(|(e, slot)| {
                // `Agreed` is the only phase holding both a closed dealing (sealed, or
                // resumed player-only) and the set — the seal-before-finalize gate. It
                // needs no deadline of its own: the certificate fixes the set, so waiting
                // for bodies can only enable the selection.
                let EpochState::Agreed { ceremony: c, set } = &slot.state else {
                    return None;
                };
                let target = *e;
                let committee = (self.committee_for)(target)?;
                let n = committee.len();
                // `all_held && ready` is the whole gate: a missing pinned body means wait
                // for the fetch, never finalize over a subset, and never below quorum.
                let (ready, all_held) = c.pinned_ready(rng, &committee, &set.pinned);
                if !(all_held && ready) {
                    // An index with no committee position can never be satisfied (the
                    // ceremony skips it, `scoped_pinned_logs`), so a non-zero count means
                    // the pinned set and this node's committee disagree.
                    let unmappable = set.pinned.keys().filter(|i| **i as usize >= n).count();
                    deferrals.push((target, all_held, unmappable));
                    return None;
                }
                Some((target, committee))
            })
            .collect();
        for (epoch, all_held, unmappable) in deferrals {
            if unmappable > 0 {
                self.metrics.dkg_pinned_idx_out_of_range.inc();
            }
            // `QuorumMissing` is a property of the agreed set, readable only with every
            // body in hand: below `all_held`, `ready` says nothing about the set, so that
            // latch is neither raised nor dropped. `BodyMissing` clears on the tick the
            // fetch lands — a held body is never lost.
            let reason = if all_held {
                self.clear_stall(epoch, StallReason::BodyMissing);
                StallReason::QuorumMissing
            } else {
                StallReason::BodyMissing
            };
            // The deferred counter and its error line ride the latch (once per epoch and
            // reason); the out-of-range counter is per tick.
            if self
                .epochs
                .get(&epoch)
                .is_some_and(|slot| !slot.stalled.contains(&reason))
            {
                self.metrics.dkg_finalize_deferred.inc();
                if unmappable > 0 {
                    tracing::error!(
                        target: "dpos::beacon",
                        epoch,
                        reason = reason.as_str(),
                        unmappable,
                        "DKG finalize deferred over the agreed pinned set and that set names \
                         indices outside the committed committee — the pinned set and this \
                         node's committee disagree"
                    );
                }
            }
            self.stall(epoch, reason);
        }
        for (e, committee) in plans {
            let (mut ceremony, set) = match self.take_state(e) {
                Some(EpochState::Agreed { ceremony, set }) => (ceremony, set),
                Some(other) => {
                    self.put_back(e, other);
                    continue;
                }
                None => continue,
            };
            let finalized = ceremony.finalize_over_pinned(rng, &committee, &set.pinned);
            // Seed the serve store now rather than at the boundary: the no-restart hot
            // path never reads the journal, and a peer recovering this epoch is served
            // from here. The journal itself stays until the sweep.
            self.log_store.seed(e, ceremony.take_signed_logs());
            let (next, stalled) = match finalized {
                Ok((_out, share)) => {
                    // The gate is the artifact's polynomial, not the local `_out`: only
                    // the former is what the network certified.
                    match self.adopt_share(e, &committee, set.group_key.clone(), share) {
                        Ok(()) => {
                            tracing::info!(
                                epoch = e,
                                height = self.height_now(),
                                "live DKG: PK_epoch + share computed + stored"
                            );
                            (EpochState::Keyed { digest: set.digest }, None)
                        }
                        Err(refusal) => (
                            EpochState::Acquiring(Acquire::Logs(Box::new(
                                self.heal_over(e, &committee, set),
                            ))),
                            Some(refusal.stall()),
                        ),
                    }
                }
                Err(DkgError::MissingPlayerDealing) => {
                    // This node acked a dealing it no longer holds; the dealer does not
                    // reveal an acked point and a sealed log cannot be re-opened, so no
                    // fetch can produce the missing input. Terminal — a share-less member
                    // is a safe verifier.
                    self.metrics.dkg_ceremony_fail.inc();
                    self.metrics.dkg_share_unrecoverable.inc();
                    tracing::warn!(
                        epoch = e,
                        "live DKG: share for this epoch is UNRECOVERABLE — this node \
                         acknowledged a dealing it no longer holds (a lost or replaced \
                         share directory). It sits the epoch out as a verifier; the next \
                         epoch's ceremony is unaffected."
                    );
                    (
                        EpochState::Unrecoverable {
                            key: Some(set.digest),
                        },
                        Some(StallReason::Unrecoverable),
                    )
                }
                Err(err) => {
                    self.metrics.dkg_ceremony_fail.inc();
                    tracing::warn!(
                        epoch = e,
                        ?err,
                        "live DKG: finalize failed after ready-probe — re-selecting the \
                         retained journal over the pinned set on the next tick"
                    );
                    (
                        EpochState::Acquiring(Acquire::Logs(Box::new(
                            self.heal_over(e, &committee, set),
                        ))),
                        None,
                    )
                }
            };
            // Phase first, latch second: a latch names a condition of the phase the epoch
            // is in, and `stall` logs that phase.
            self.set_state(e, next);
            if let Some(reason) = stalled {
                self.stall(e, reason);
            }
        }
    }

    /// The heal behind a failed finalize or a refused adoption: the artifact's pinned
    /// set mapped onto the committee (the scope the recompute selects over, as the live
    /// finalize does), its `Output` as the self-check target, and the pinned bodies the
    /// retained journal lacks (`want`, fetched by exact `(dealer, hash)`).
    fn heal_over(&self, epoch: u64, committee: &Set<PeerPubkey>, set: AgreedSet) -> RecomputeState {
        let pinned = pinned_by_dealer(committee, &set.pinned);
        // Only the exact pinned `(dealer, hash)` counts as held — a journaled other body
        // of the same dealer does not.
        let held = self.log_store.parse_journal(epoch);
        let want: BTreeSet<LogId> = pinned
            .iter()
            .map(|(d, h)| (d.clone(), *h))
            .filter(|id| !held.contains_key(id))
            .collect();
        tracing::info!(
            epoch,
            want = want.len(),
            pinned = pinned.len(),
            "live DKG: demoted committee member detected — starting share recompute-heal"
        );
        RecomputeState {
            outcome: set.group_key,
            pinned,
            want,
            digest: set.digest,
            attempted: false,
        }
    }

    /// The post-seal absentee's entry: heal over the pinned set, or wait for the artifact first.
    fn heal_or_acquire(
        &self,
        epoch: u64,
        committee: &Set<PeerPubkey>,
        agreed: Option<AgreedSet>,
    ) -> EpochState {
        match agreed {
            Some(set) => EpochState::Acquiring(Acquire::Logs(Box::new(
                self.heal_over(epoch, committee, set),
            ))),
            None => EpochState::Acquiring(Acquire::ArtifactForShare),
        }
    }

    /// Decide every undecided epoch in this actor's window ([`decidable_epochs`]) on one
    /// tick; returns the epochs that entered `Dealing`. `out` collects only what the
    /// target sends — a window epoch is at or past its boundary and sends nothing.
    async fn decide_window(&mut self, now: u64, out: &mut Vec<Outgoing>) -> BTreeSet<u64> {
        let mut started = BTreeSet::new();
        let mut dropped = Vec::new();
        let target = now.saturating_add(1);
        for e in decidable_epochs(now) {
            let sink = if e == target { &mut *out } else { &mut dropped };
            if self.decide(e, sink).await {
                started.insert(e);
            }
        }
        debug_assert!(dropped.is_empty(), "a past-boundary resume must not send");
        started
    }

    /// Whether `epoch` is one `decide` looks at on this actor's clock
    /// ([`decidable_epochs`]); false before the first height tick, where there is no clock.
    fn decidable(&self, epoch: u64) -> bool {
        let Some(height) = self.last_height else {
            return false;
        };
        decidable_epochs(self.epoch_of(height)).any(|e| e == epoch)
    }

    /// Decide `epoch` if undecided; returns whether it entered `Dealing` now, in which
    /// case the dealings that raced ahead of the start are drained below.
    async fn decide(&mut self, epoch: u64, out: &mut Vec<Outgoing>) -> bool {
        if self.epochs.contains_key(&epoch) || !self.decidable(epoch) {
            return false;
        }
        let Some((state, outgoing, stalled)) = self.recover(epoch).await else {
            return false; // undecided: re-asked next tick
        };
        out.extend(outgoing);
        let dealing = matches!(state, EpochState::Dealing { .. });
        self.enter(epoch, state);
        if let Some(reason) = stalled {
            self.stall(epoch, reason);
        }
        if !dealing {
            // Not dealing: drop dealings buffered for a start that will never happen, so
            // they do not linger un-acked until the sweep.
            self.pending.remove(&epoch);
            return false;
        }
        // The start-race drain: replay now, before any seal, so every dealer we heard
        // from is acked. Order-independent — `try_ack` fires only once both halves are
        // buffered.
        if let Some(buffered) = self.pending.remove(&epoch) {
            // Buffered before this epoch's ceremony existed — on epoch alone, or on a seat
            // check the roster could not answer yet — so the consuming ceremony asks the
            // same admission question the live dispatch asks.
            let mut admitted: Vec<(PeerPubkey, DkgBody)> = Vec::new();
            for (from, dealings) in buffered {
                let halves: Vec<DkgBody> = [dealings.commitment, dealings.share]
                    .into_iter()
                    .flatten()
                    .collect();
                let refusal = halves
                    .first()
                    .and_then(|body| self.ceremony_refusal(epoch, &from, body));
                if let Some(reason) = refusal {
                    self.refuse(&from, Some(epoch), reason);
                    continue;
                }
                admitted.extend(halves.into_iter().map(|body| (from.clone(), body)));
            }
            // Collected first so the borrow of the ceremony ends before the journal writes:
            // each step's acks go out only if its own `ReceivedDealing` write landed.
            let steps: Vec<Step> = {
                let c = self.ceremony_mut(epoch).expect("just entered Dealing");
                admitted
                    .into_iter()
                    .map(|(from, body)| c.handle(from, body))
                    .collect()
            };
            for step in steps {
                let acked = acked_dealers(&step);
                if self.journal_or_defer(epoch, step.journal, &acked) {
                    out.extend(step.outgoing);
                }
            }
        }
        true
    }

    /// The consumer's admission of a ceremony frame from `from` for `epoch` — the
    /// one answer both ingress paths give, the live dispatch ([`Self::on_message`])
    /// and the start-race drain ([`Self::decide`]): `None` admits; `Some` is the
    /// reason for the shared ingress counter.
    ///
    /// `no_seat`: a ceremony keys a dealing and an ack by the sender, and commonware
    /// answers a stranger with a silent `None` / `UnknownPlayer`, so the seat is
    /// refused out loud on the shared counter instead.
    ///
    /// `equivocator`: this epoch holds a proven pair for the dealer, so none of its
    /// `Commitment`/`Share`/`Ack` may be consumed — a player that had not acked yet
    /// would take the dealing of the second polynomial and end up with a share off
    /// the one the network pinned. Gossip logs are refused in the ceremony
    /// (`DkgCeremony::handle`); the dealings are refused here, where the sender is
    /// bound to its seat. The evidence is the slot's, so it outlives the ceremony
    /// and a resumed one carries the ban from the tick it is restored (`enter`).
    fn ceremony_refusal(
        &self,
        epoch: u64,
        from: &PeerPubkey,
        body: &DkgBody,
    ) -> Option<&'static str> {
        if self.ceremony(epoch).is_some_and(|c| !c.has_seat(from)) {
            return Some("no_seat");
        }
        if matches!(
            body,
            DkgBody::Commitment(_) | DkgBody::Share(_) | DkgBody::Ack(_)
        ) && self
            .epochs
            .get(&epoch)
            .is_some_and(|slot| slot.evidence.contains_key(from))
        {
            return Some("equivocator");
        }
        None
    }

    /// The one write-before-ack form both ingress paths use: journal a step's
    /// `records`, and on any failure withhold the step's acks (`acked`) and queue
    /// every `ReceivedDealing` among them for the retry
    /// (`retry_nondurable_journals`). Returns whether the write was durable — the
    /// caller broadcasts the step's outgoing only then, and names a failed log by
    /// the id the ceremony authenticated.
    fn journal_or_defer(
        &mut self,
        epoch: u64,
        records: Vec<JournalRecord>,
        acked: &[PeerPubkey],
    ) -> bool {
        let failed = self.journal_failures(epoch, records);
        if failed.is_empty() {
            return true;
        }
        let dealings: Vec<JournalRecord> = failed
            .into_iter()
            .filter(|r| matches!(r, JournalRecord::ReceivedDealing(..)))
            .collect();
        if !dealings.is_empty() {
            self.nondurable_dealings
                .entry(epoch)
                .or_default()
                .extend(dealings);
        }
        self.withhold_acks(epoch, acked);
        false
    }

    /// Forget a failed step's acks: the next retransmit must not re-emit one from the
    /// ceremony's cache as already durable.
    fn withhold_acks(&mut self, epoch: u64, acked: &[PeerPubkey]) {
        if let Some(c) = self.ceremony_mut(epoch) {
            for dealer in acked {
                c.withhold_ack(dealer);
            }
        }
    }

    /// Turn what this node holds for `epoch` into its phase: the share file, the
    /// ceremony journal, the agreed artifact and the clock against the seal deadline,
    /// plus the chain's `changed` bit and this node's seat in `committee[epoch]`.
    /// `None` ⇒ undecidable yet (an unreadable committee or bit), re-asked next tick.
    ///
    /// The journal decides a member's epoch with no share held: `Present` resumes —
    /// the seeded dealer re-derived only before the seal deadline, player-only at or
    /// after it, preferring each dealer's pinned body when the artifact is already
    /// held. `Torn` and `NoFile` follow one rule by timing: before the deadline
    /// nothing was ever broadcast, so re-deriving the deterministic dealer is safe
    /// (a torn file is removed first); at or after it this node may already have
    /// sealed, so it heals as a player over the pinned bodies and never re-deals. A
    /// replay `Err` is `Unrecoverable` — terminal, not a per-tick retry — and a
    /// resumed epoch at or past its boundary with no artifact is acquiring rather
    /// than sealed: its agreement ran without this node.
    ///
    /// The third element is the latch the decided phase is raised with once its slot
    /// stands — `OffPolynomial` for a refused reloaded share.
    async fn recover(
        &mut self,
        epoch: u64,
    ) -> Option<(EpochState, Vec<Outgoing>, Option<StallReason>)> {
        let quiet = |state: EpochState| Some((state, Vec::new(), None));
        let stored = self.stored(epoch);
        if let Some(marker) = share_state::load_conflict(&self.share_dir, epoch) {
            // The verdict outlives the process: a share the eviction missed must not
            // re-key the epoch.
            self.drop_share(epoch);
            let (held, second) = match marker {
                ConflictMarker::Pair(held, second) => (held, second),
                // A marker is written only by a verdict, so a damaged one is still the
                // verdict — fail closed, with no pair to name.
                ConflictMarker::Malformed => {
                    tracing::error!(
                        target: "dpos::beacon",
                        epoch,
                        "live DKG: the conflict marker on disk is malformed — the epoch stays \
                         Conflict (fail-closed) with no known pair"
                    );
                    (B256::ZERO, B256::ZERO)
                }
            };
            // The key is whatever the store holds: the marker is fsync'd ahead of the
            // artifact's write-behind, so it can outlive the artifact across a restart.
            let key = stored.as_ref().map(|s| value_digest(&s.held));
            return quiet(EpochState::Conflict { held, second, key });
        }
        if let Some((held, divergent)) = stored
            .as_ref()
            .and_then(|s| Some((value_digest(&s.held), s.divergent?)))
        {
            // Two certified values in the store with no slot ever standing (a lost
            // hand-off, or a restart in between): the verdict is made durable here.
            self.stop_signing(epoch, held, divergent);
            return quiet(EpochState::Conflict {
                held,
                second: divergent,
                key: Some(held),
            });
        }
        let artifact = stored.map(|s| s.held);
        let share_held = self
            .store
            .read()
            .ok()
            .is_some_and(|s| s.contains_key(&epoch));
        if share_held {
            return match artifact {
                Some(proposal) => {
                    let committee = (self.committee_for)(epoch)?;
                    let (state, stalled) =
                        self.key_held_share(epoch, &committee, AgreedSet::of(&proposal));
                    Some((state, Vec::new(), stalled))
                }
                None => quiet(EpochState::Acquiring(Acquire::ArtifactForShare)),
            };
        }
        // The mint decision is the chain's `changed` bit, never a roster comparison;
        // the committee below is read for the ceremony itself — who deals to whom.
        let mints = self.mints_at(epoch)?;
        if !mints {
            return quiet(EpochState::KeyOnly { digest: None });
        }
        let committee = (self.committee_for)(epoch)?;
        let me = self.me_key.public_key();
        // Only a member of `committee[epoch]` deals; a seat in `committee[epoch - 1]`
        // does not qualify.
        if !committee.iter().any(|p| *p == me) {
            return quiet(match artifact {
                Some(proposal) => EpochState::KeyOnly {
                    digest: Some(value_digest(&proposal)),
                },
                None => EpochState::Acquiring(Acquire::ArtifactForKey),
            });
        }
        // A node seals only at or after the deadline: below it nothing was ever
        // broadcast, so re-deriving the seeded dealer is safe; at or after it a torn
        // or absent journal cannot prove we did not already seal, so we never re-seal.
        let past_seal =
            self.height_now() >= self.epoch_start(epoch).saturating_sub(DKG_MARGIN_BLOCKS);
        let past_boundary = self.last_height.is_some_and(|h| epoch <= self.epoch_of(h));
        let key = artifact.as_ref().map(value_digest);
        let agreed = artifact.as_ref().map(AgreedSet::of);
        let (ceremony, outgoing) = match self.load_journal(epoch) {
            JournalLoad::Present(records) => {
                match self.resume_from_journal(epoch, committee, records, !past_seal, &agreed) {
                    Some(resumed) => resumed,
                    None => return quiet(EpochState::Unrecoverable { key }),
                }
            }
            JournalLoad::Torn if past_seal => {
                tracing::warn!(
                    epoch,
                    "live DKG: ceremony journal present but unreadable/torn at or after the \
                     seal deadline — never re-dealing (we may already have sealed); healing \
                     the share as a player over the pinned bodies"
                );
                self.evict_journal(epoch);
                return quiet(self.heal_or_acquire(epoch, &committee, agreed));
            }
            JournalLoad::NoFile if past_seal => {
                tracing::warn!(
                    epoch,
                    "live DKG: no ceremony journal at or after the seal deadline — never \
                     re-dealing (a lost journal cannot prove we never sealed); healing the \
                     share as a player over the pinned bodies"
                );
                return quiet(self.heal_or_acquire(epoch, &committee, agreed));
            }
            JournalLoad::Torn => {
                tracing::warn!(
                    epoch,
                    "live DKG: torn ceremony journal before the seal deadline — nothing was \
                     broadcast, re-deriving the seeded dealer over a fresh journal"
                );
                self.evict_journal(epoch);
                match self.start_fresh(epoch, committee) {
                    Some(started) => started,
                    None => return quiet(EpochState::Unrecoverable { key }),
                }
            }
            JournalLoad::NoFile => match self.start_fresh(epoch, committee) {
                Some(started) => started,
                None => return quiet(EpochState::Unrecoverable { key }),
            },
        };
        let state = match (ceremony.dealing_closed(), agreed, past_boundary) {
            (false, agreed, _) => EpochState::Dealing { ceremony, agreed },
            (true, Some(set), _) => EpochState::Agreed { ceremony, set },
            (true, None, false) => EpochState::Sealed { ceremony },
            (true, None, true) => {
                EpochState::Acquiring(Acquire::ArtifactForCeremony(Box::new(ceremony)))
            }
        };
        // No dealer is left to re-ack past the boundary, so a resumed epoch sends nothing.
        let outgoing = if past_boundary { Vec::new() } else { outgoing };
        Some((state, outgoing, None))
    }

    fn load_journal(&self, target: u64) -> JournalLoad {
        let max = NonZeroU32::new(fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as u32)
            .expect("MAX_COMMITTEE_SIZE > 0");
        share_state::load_journal(&self.share_dir, target, &self.share_state, max)
    }

    /// Start a fresh ceremony for `target`, journaling its initial records. `None`
    /// when the ceremony cannot be built over the committed roster — deterministic on
    /// the same inputs, so the caller records `Unrecoverable` rather than retrying.
    fn start_fresh(
        &mut self,
        target: u64,
        next: Set<PeerPubkey>,
    ) -> Option<(DkgCeremony, Vec<Outgoing>)> {
        match DkgCeremony::start(&self.namespace, target, next, self.me_key.clone()) {
            Ok((ceremony, step)) => {
                // The outgoing is our broadcast commitment + private shares, not an
                // ack, so it is not gated on this write.
                let _ = self.append_journal(target, step.journal);
                tracing::info!(epoch = target, "live DKG: ceremony started");
                Some((ceremony, step.outgoing))
            }
            Err(e) => {
                tracing::warn!(epoch = target, ?e, "live DKG: ceremony start failed");
                None
            }
        }
    }

    /// Resume `target`'s ceremony from its journaled `records`. `reconstruct_dealer`
    /// is the pre-seal flag — set, it re-derives the seeded dealer and keeps
    /// distributing; at or after the deadline it is false and the resume is
    /// player-only, never re-sealing. With the artifact already held (`agreed`), the
    /// rebuilt player stands on each dealer's pinned body, the first-recorded one
    /// otherwise. Any replay `Err` yields `None` — the caller records
    /// `Unrecoverable`, never a crash and never a per-tick retry.
    fn resume_from_journal(
        &mut self,
        target: u64,
        next: Set<PeerPubkey>,
        records: Vec<JournalRecord>,
        reconstruct_dealer: bool,
        agreed: &Option<AgreedSet>,
    ) -> Option<(DkgCeremony, Vec<Outgoing>)> {
        let preferred = agreed
            .as_ref()
            .map(|set| pinned_by_dealer(&next, &set.pinned))
            .unwrap_or_default();
        match DkgCeremony::resume(
            &self.namespace,
            target,
            next,
            self.me_key.clone(),
            records,
            reconstruct_dealer,
            &preferred,
        ) {
            Ok(resumed) => {
                let own_log_recorded = resumed.ceremony.own_log_recorded(&self.me_key.public_key());
                tracing::info!(
                    epoch = target,
                    own_log_recorded,
                    "live DKG: ceremony resumed from journal"
                );
                Some((resumed.ceremony, resumed.outgoing))
            }
            Err(e) => {
                // Any replay `Err` is terminal — one WARN, one count, no per-tick retry.
                self.metrics.dkg_share_unrecoverable.inc();
                tracing::warn!(
                    epoch = target,
                    ?e,
                    "live DKG: resume from journal failed — the share is UNRECOVERABLE here \
                     (a lost or replaced share directory); this node sits the epoch out as a \
                     verifier"
                );
                None
            }
        }
    }

    /// Whether an incoming DKG message should be buffered when no ceremony for its
    /// epoch exists yet (the start-race) rather than dropped: only a `Commitment` or
    /// `Share` for an undecided, near-future epoch qualifies — an ack or a reveal has
    /// no ceremony to feed, and the ingress window bounds how many garbage epochs can
    /// accumulate. Stale buffers are evicted on each tick.
    ///
    /// This is the epoch half of the admission only; the seat half — is the sender in
    /// `committee[epoch]` — belongs to the caller ([`Self::on_message`], the buffer
    /// branch): asked against that record when it is readable, deferred to the drain
    /// in [`Self::decide`] when it is not.
    fn is_bufferable(&self, epoch: u64, body: &DkgBody) -> bool {
        if !matches!(body, DkgBody::Commitment(_) | DkgBody::Share(_)) {
            return false;
        }
        // Only an undecided epoch can still start the dealer that drains the buffer;
        // a decided one dispatches the frame, or is closed, `Keyed`, or never deals.
        if self.epochs.contains_key(&epoch) {
            return false;
        }
        let now = self.epoch_of(self.height_now());
        // This buffer's own rule on top of the shared window: a dealing for `now` or
        // below belongs to a ceremony already started or past.
        if epoch <= now || !within_ingress_window(now, epoch) {
            return false;
        }
        true
    }

    /// The `0`-floored clock the seal deadline and the ingress window read: the last
    /// drained height, or `0` before the first tick.
    ///
    /// `0` is the floor a deadline wants — a clock that has not started answers "not
    /// yet", which only delays an action to the first tick. A window must not read it
    /// that way, so `on_confirm` reads `last_height` itself and can tell "epoch 0"
    /// from "no clock".
    fn height_now(&self) -> u64 {
        self.last_height.unwrap_or(0)
    }

    /// Whether `epoch` is one this actor could act on at all: a ceremony it is already
    /// running, or one inside the shared ingress window of its clock. Asked before the
    /// body decode so an arbitrary epoch on the wire costs no committee resolve.
    ///
    /// A cost gate, deliberately a superset of what the dispatch does: it admits
    /// `[now, now+2]`, while [`Self::is_bufferable`] takes only `[now+1, now+2]` and
    /// the ceremony dispatch takes only a live ceremony. A frame it lets through is
    /// refused a few lines later by the check that owns the decision.
    fn epoch_is_actionable(&self, epoch: u64) -> bool {
        if self.ceremony(epoch).is_some() {
            return true;
        }
        within_ingress_window(self.epoch_of(self.height_now()), epoch)
    }

    /// The one place the actor refuses a beacon frame: one count on the channel's
    /// ingress metric, one `debug` line naming who sent what for which epoch and why,
    /// with this actor's own clock beside it (`None` before the first height tick).
    ///
    /// The reasons, and where each is decided:
    ///  * `undecodable` — [`Self::on_message`]: not a beacon frame
    ///    (`BeaconMessage::read`), too short to carry an epoch (the eight-byte peek),
    ///    or a body that does not decode as a `DkgMsg`; `epoch` is `None` when the
    ///    frame never yielded one;
    ///  * `epoch` — [`Self::epoch_is_actionable`], before the body decode;
    ///  * `confirm_window` — [`Self::on_confirm`], the entry-bar window;
    ///  * `no_seat` — no seat for the sender in the frame's epoch: the live ceremony's
    ///    roster ([`DkgCeremony::has_seat`]) at the dispatch in [`Self::on_message`],
    ///    `committee[epoch]` at the start-race buffer there when that record is
    ///    readable, the ceremony's roster at the drain in [`Self::decide`], or
    ///    `committee[target_epoch]` in [`Self::on_confirm`].
    ///
    /// What is not counted here is not a refusal: an ack or a reveal with no live
    /// ceremony to feed, a confirmation for an epoch whose committee this node cannot
    /// read yet (the node's own state, not the frame's), and a confirmation no wider
    /// than the one already held for its seat.
    ///
    /// The sender-tier refusals (`untracked`, `secondary`) are the channel's
    /// pre-decode `GatedReceiver`'s, on the same metric, and never reach here.
    fn refuse(&self, from: &PeerPubkey, epoch: Option<u64>, reason: &'static str) {
        let now = self.last_height.map(|h| self.epoch_of(h));
        tracing::debug!(
            target: "dpos::beacon",
            %from,
            ?epoch,
            ?now,
            reason,
            "DKG frame refused at ingress"
        );
        crate::dpos::record_ingress_drop(BEACON_CHANNEL_LABEL, reason);
    }

    async fn on_message(&mut self, from: PeerPubkey, buf: &[u8], rng: &mut impl CryptoRngCore) {
        // An upper bound is enough: the exact committee size is not known at decode time.
        let max = NonZeroU32::new(fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as u32)
            .expect("MAX_COMMITTEE_SIZE > 0");
        let mut wire = buf;
        // A frame that is not a beacon frame is a refusal too: it is counted like any other.
        let payload = match BeaconMessage::read(&mut wire) {
            Ok(BeaconMessage::Dkg(p)) => p,
            Err(_) => {
                self.refuse(&from, None, "undecodable");
                return;
            }
        };
        // The epoch rides first on the wire: reading it before the body keeps a frame for an
        // epoch this actor cannot act on away from the polynomial and signed-log decoders.
        let mut header = payload.as_ref();
        let Ok(epoch) = u64::read_cfg(&mut header, &()) else {
            self.refuse(&from, None, "undecodable");
            return;
        };
        if !self.epoch_is_actionable(epoch) {
            self.refuse(&from, Some(epoch), "epoch");
            return;
        }
        let mut body = payload.as_ref();
        let msg = match DkgMsg::read_cfg(&mut body, &max) {
            Ok(m) => m,
            Err(_) => {
                self.refuse(&from, Some(epoch), "undecodable");
                return;
            }
        };
        debug_assert_eq!(
            msg.ceremony_epoch, epoch,
            "header peek must match the decode"
        );
        let epoch = msg.ceremony_epoch;
        let body = msg.body;
        // A confirmation must be recorded even when no ceremony is live for the epoch (this
        // node may have finalized already), so it is intercepted before dispatch and buffering.
        if let DkgBody::Confirm(confirm) = body {
            self.on_confirm(epoch, &from, confirm);
            return;
        }
        // Epoch-tag filter: an active ceremony consumes the frame, a dealing for an epoch this
        // node has not started is buffered rather than dropped (a dropped dealing leaves that
        // dealer un-acked), and an ack or reveal with no live ceremony has nothing to feed.
        if let Some(reason) = self.ceremony_refusal(epoch, &from, &body) {
            self.refuse(&from, Some(epoch), reason);
            return;
        }
        if let Some(c) = self.ceremony_mut(epoch) {
            let step = c.handle(from, body);
            let recorded_log = step.recorded_a_log();
            let acked = acked_dealers(&step);
            // An ack must not go out before its journal write lands: after a restart this
            // node could not back it, an unhealable `MissingPlayerDealing`. An empty journal
            // is trivially durable, and a withheld ack still leaves our point in the
            // dealer's own log, which the recompute heals from.
            let durable = self.journal_or_defer(epoch, step.journal, &acked);
            self.note_equivocation(epoch, step.equivocation.as_ref(), durable);
            if !durable {
                // Named by the id the ceremony authenticated, not by `from`: a peer may relay
                // another dealer's valid Reveal, and blaming the sender would suppress an
                // honest log while leaving the real dealer's claim standing.
                if let Some(id) = step.recorded_log {
                    self.nondurable_logs.entry(epoch).or_default().insert(id);
                }
            }
            if durable {
                self.broadcast_all(step.outgoing).await;
            }
            // Only a recorded log can change finalizability, so finalize on this edge and
            // skip the batch BLS for ack-only / dealing-only steps; the height tick still
            // covers the time-based path.
            if recorded_log {
                // Both claims made from here — the hash published into `recorded_dkg_logs`
                // and the confirmation minted from it — are durability-gated upstream in
                // `publish_recorded_logs`. The ack gate above is separate: it asks whether
                // our own `Player.view` is recoverable, and a withheld ack is not retried.
                self.drive_finalization(rng);
                // A recorded log widens what this node can confirm, and the entry bar counts
                // confirmations that cover the proposed set: the two widths a leader cannot
                // wait a block for go out on this edge, the rest ride the height tick.
                let minted = self.confirmations.mint(ConfirmTrigger::Decisive);
                self.broadcast_all(minted).await;
            }
        } else if self.is_bufferable(epoch, &body) {
            // The seat question, asked as early as it can be: when `committee[epoch]` is
            // readable, a sender with no seat in it is refused here and occupies no slot;
            // when it is not (the start race), the dealing is buffered on its epoch and the
            // drain in `decide` asks the ceremony's roster instead.
            if (self.committee_for)(epoch).is_some_and(|roster| roster.position(&from).is_none()) {
                self.refuse(&from, Some(epoch), "no_seat");
                return;
            }
            // Per-sender latest-wins: a peer can only overwrite its own slot (at most one
            // Commitment + one Share), so a Byzantine peer cannot evict honest dealings.
            let slot = self
                .pending
                .entry(epoch)
                .or_default()
                .entry(from)
                .or_default();
            match body {
                DkgBody::Commitment(_) => slot.commitment = Some(body),
                DkgBody::Share(_) => slot.share = Some(body),
                // Unreachable: `is_bufferable` admits only Commitment/Share.
                _ => {}
            }
        }
        self.debug_assert_settled();
    }

    /// `fetch_targeted` every pinned body a ceremony lacks, by exact
    /// `(epoch, dealer, hash)`. The pinned set — the artifact's `idx`-to-`hash` map onto
    /// `committee[epoch]` — is the only source of what to ask for: a dealer whose held log
    /// is a different body than the pinned one is fetched exactly like one this node holds
    /// nothing of. Before an artifact there is nothing to ask for by hash: a ceremony still
    /// collecting relies on the gossip `Reveal`s, and the agreement fetches a proposal's own
    /// bodies itself.
    ///
    /// Runs in `on_height` after finalize and the past-boundary sweep, so `self.epochs`
    /// already reflects every drop; it then `retain`s the resolver's in-flight fetches to
    /// the keys it (re)issues this tick, cancelling any epoch that finalized or was swept
    /// so the resolver stops retrying dead keys. Re-issuing is idempotent (the resolver
    /// dedupes in-flight keys) and targets the committee roster.
    async fn fetch_missing_logs(&mut self) {
        // Two phases so the immutable borrows of `epochs` and `committee_for` end before
        // `self.resolver` is borrowed mutably.
        let mut requests: Vec<(DkgLogKey, NonEmptyVec<PeerPubkey>)> = Vec::new();
        // Epochs whose committee read failed this tick: preserving their keys keeps one bad
        // read from cancelling live in-flight fetches. A dead epoch is in neither set, so it
        // is still cancelled.
        let mut unreadable: BTreeSet<u64> = BTreeSet::new();
        for (e, slot) in &self.epochs {
            // Nothing pinned has nothing to name; a heal already names its `want` (`pinned(E)`
            // minus `held`), so a body no peer holds ages out with the epoch instead of
            // storming. The slot's life in the map is the retention window and nothing
            // narrower: a second clock would stop a retained ceremony from asking for the
            // bodies its finalize needs.
            let missing: Vec<LogId> = match &slot.state {
                // A set that arrived while dealing is fetched here too: the post-seal
                // finalize needs the bodies, and a lagging clock is no reason to wait.
                EpochState::Agreed { ceremony, set }
                | EpochState::Dealing {
                    ceremony,
                    agreed: Some(set),
                } => {
                    let Some(roster) = (self.committee_for)(*e) else {
                        unreadable.insert(*e);
                        continue;
                    };
                    // Exact `(dealer, hash)` only, with no `me` special case: a node whose own
                    // seal was torn re-fetches its own pinned log, and the peers that recorded
                    // its broadcast serve it.
                    pinned_by_dealer(&roster, &set.pinned)
                        .into_iter()
                        .filter(|id| !ceremony.holds(id))
                        .collect()
                }
                EpochState::Acquiring(Acquire::Logs(st)) => st.want.iter().cloned().collect(),
                _ => continue,
            };
            if missing.is_empty() {
                continue;
            }
            let Some(roster) = (self.committee_for)(*e) else {
                unreadable.insert(*e);
                continue;
            };
            // Target the whole roster: `fetch_targeted` only reaches peers in
            // `latest.primary`, which carries `committee[E]` while the epoch is E-1.
            let Some(targets) =
                NonEmptyVec::try_from(roster.iter().cloned().collect::<Vec<_>>()).ok()
            else {
                continue;
            };
            for (dealer, hash) in missing {
                requests.push((
                    DkgLogKey {
                        epoch: *e,
                        dealer,
                        hash,
                    },
                    targets.clone(),
                ));
            }
        }

        // Only the keys (re)issued this tick stay in flight; everything else is dropped so a
        // finalized or swept epoch's fetches stop retrying. The predicate must be owned.
        let wanted: BTreeSet<DkgLogKey> = requests.iter().map(|(k, _)| k.clone()).collect();
        self.resolver
            .retain(move |key| wanted.contains(key) || unreadable.contains(&key.epoch))
            .await;
        for (key, targets) in requests {
            self.resolver.fetch_targeted(key, targets).await;
        }
    }

    /// Drive every `Acquiring` epoch, once per `on_height` tick: reconcile with the
    /// artifact store first (the store owns the fact, and a lost hand-off over a bounded
    /// channel is not a lost artifact), then ask peers for the artifact every phase that
    /// needs one ([`PullArtifact`]), then attempt the journal recompute for the heals
    /// holding all their pinned bodies ([`Self::try_recompute`]).
    ///
    /// The pull short-circuits on a local hit, throttles to one network attempt per epoch
    /// per `PULL_MIN_INTERVAL`, and de-duplicates its spawns per epoch, so the call on
    /// every tick is cheap.
    ///
    /// An epoch at or past its boundary that is still acquiring raises `Stalled{NoArtifact}`
    /// once; before the boundary, waiting is the normal shape of an epoch whose agreement
    /// has not converged.
    ///
    /// A member's live ceremony never pulls — its instance delivers the artifact — because
    /// pulling would spend the `BEACON_RESOLVER_CHANNEL` budget the same ceremony's
    /// dealer-log fetches need.
    fn drive_acquisition(&mut self, now: u64, rng: &mut impl CryptoRngCore) {
        if self.reconcile_with_store() {
            self.drive_finalization(rng);
        }
        // At or past its boundary a `Sealed` epoch has no instance left to join: the value is
        // at the peers now, so the pull is the heal.
        let entered: Vec<u64> = self
            .epochs
            .iter()
            .filter(|(e, slot)| **e <= now && matches!(slot.state, EpochState::Sealed { .. }))
            .map(|(e, _)| *e)
            .collect();
        for e in entered {
            let ceremony = match self.take_state(e) {
                Some(EpochState::Sealed { ceremony }) => ceremony,
                Some(other) => {
                    self.put_back(e, other);
                    continue;
                }
                None => continue,
            };
            self.set_state(
                e,
                EpochState::Acquiring(Acquire::ArtifactForCeremony(Box::new(ceremony))),
            );
        }
        let acquiring: Vec<u64> = self
            .epochs
            .iter()
            .filter(|(_, slot)| slot.state.needs_artifact())
            .map(|(e, _)| *e)
            .collect();
        for e in acquiring {
            if e <= now {
                self.stall(e, StallReason::NoArtifact);
            }
            (self.pull_artifact)(e);
        }
        self.try_recompute(rng);
    }

    /// Bring every decided epoch up to what the artifact store holds for it — the store is
    /// the one owner of "the epoch's artifact" (first-wins, quorum-checked before it stores),
    /// and the actor's own intake is a bounded channel whose loss must not lose the fact: a
    /// phase standing on no artifact takes the store's through the ordinary artifact edge
    /// ([`Self::apply_artifact`]), and a phase standing on a different value — or a store that
    /// noted a divergent second value (`ArtifactStore::note_divergent`) — is `Conflict`.
    ///
    /// Returns whether a ceremony now stands on a set it did not before (the caller
    /// finalizes).
    fn reconcile_with_store(&mut self) -> bool {
        let decided: Vec<u64> = self
            .epochs
            .iter()
            .filter(|(_, slot)| !matches!(slot.state, EpochState::Conflict { key: Some(_), .. }))
            .map(|(e, _)| *e)
            .collect();
        let mut finalizable = false;
        for e in decided {
            let Some(stored) = self.stored(e) else {
                continue;
            };
            // A `Conflict` without a key takes the store's artifact as its key through the
            // artifact edge; the value is never compared, its verdict is already final.
            if matches!(self.state(e), Some(EpochState::Conflict { .. })) {
                self.apply_artifact(e, &stored.held);
                continue;
            }
            let held = value_digest(&stored.held);
            match self.state(e).and_then(EpochState::held_digest) {
                None => finalizable |= self.apply_artifact(e, &stored.held),
                Some(own) if own != held => {
                    self.conflict(e, own, held);
                    continue;
                }
                Some(_) => {}
            }
            if let Some(second) = stored.divergent {
                if let Some(own) = self.state(e).and_then(EpochState::held_digest) {
                    if own != second {
                        self.conflict(e, own, second);
                    }
                }
            }
        }
        finalizable
    }

    /// Whether the network re-minted the beacon key at `epoch`: the chain's frozen
    /// `changed[epoch]` bit, plus the bootstrap exception. The one predicate the deal
    /// decision and the acquisition branch read, so they cannot disagree.
    ///
    /// `None` where the bit is unreadable: an undecided read is neither a mint nor a
    /// carry-forward, and the next tick re-asks.
    fn mints_at(&self, epoch: u64) -> Option<bool> {
        if epoch == DETERMINISTIC_BOOTSTRAP_EPOCH {
            return Some(true);
        }
        (self.changed)(epoch)
    }

    /// Arm or disarm the journal recompute of an `Acquiring(Logs)` epoch
    /// (`RecomputeState::attempted`): armed once its inputs are loaded, re-armed
    /// by a body landing in the journal or by a persist failure.
    fn set_recompute_attempted(&mut self, epoch: u64, attempted: bool) {
        if let Some(EpochSlot {
            state: EpochState::Acquiring(Acquire::Logs(st)),
            ..
        }) = self.epochs.get_mut(&epoch)
        {
            st.attempted = attempted;
        }
    }

    /// Attempt the scoped share recompute for each `Acquiring(Logs)` epoch that holds
    /// every pinned body (`want` empty) and has not already run over those inputs.
    fn try_recompute(&mut self, rng: &mut impl CryptoRngCore) {
        let ready: Vec<u64> = self
            .epochs
            .iter()
            .filter(|(_, slot)| {
                matches!(&slot.state, EpochState::Acquiring(Acquire::Logs(st))
                    if st.want.is_empty() && !st.attempted)
            })
            .map(|(e, _)| *e)
            .collect();
        for e in ready {
            let Some(committee) = (self.committee_for)(e) else {
                continue;
            };
            let Some(EpochSlot {
                state: EpochState::Acquiring(Acquire::Logs(st)),
                ..
            }) = self.epochs.get_mut(&e)
            else {
                continue;
            };
            let pinned = st.pinned.clone();
            let outcome = st.outcome.clone();
            let digest = st.digest;
            let records = match self.load_journal(e) {
                JournalLoad::Present(r) => r,
                // NoFile/Torn: the load is not an attempt — `attempted` stays false, so
                // the next tick re-reads the file.
                _ => {
                    self.stall(e, StallReason::HealFailed);
                    continue;
                }
            };
            // Once per set of inputs, and only once they are loaded: the recompute is a
            // pure function of the journal and the pinned set, so a re-run could only
            // re-derive the same refusal. A journal body landing (`ingest_recompute_log`)
            // or a failed persist below re-arms it.
            self.set_recompute_attempted(e, true);
            let recomputed = recompute_scoped(
                rng,
                &self.namespace,
                e,
                committee.clone(),
                self.me_key.clone(),
                &pinned,
                records,
            );
            let share = match recomputed {
                Ok((_out, share)) => share,
                Err(DkgError::MissingPlayerDealing) => {
                    // Terminal, not pending: the journal acks a dealing this node no
                    // longer holds, so every retry re-derives this error.
                    tracing::warn!(
                        epoch = e,
                        "live DKG: share for this epoch is UNRECOVERABLE — this node \
                         acknowledged a dealing it no longer holds (a lost or replaced \
                         share directory). It sits the epoch out as a verifier; the next \
                         epoch's ceremony is unaffected."
                    );
                    self.metrics.dkg_share_unrecoverable.inc();
                    self.set_state(e, EpochState::Unrecoverable { key: Some(digest) });
                    self.stall(e, StallReason::Unrecoverable);
                    continue;
                }
                Err(err) => {
                    // Retryable in principle (a body the journal does not hold yet can
                    // change the result), and latched as `Stalled{HealFailed}`: with
                    // `want` empty no fetch is issued, so the latch is the only signal.
                    tracing::warn!(
                        epoch = e,
                        ?err,
                        "live DKG: journal recompute over the pinned set failed — waiting \
                         for the journal to change"
                    );
                    self.stall(e, StallReason::HealFailed);
                    continue;
                }
            };
            // The recomputed output is dropped: the pinned artifact outcome is the
            // canonical one the gate checks the share against.
            match self.adopt_share(e, &committee, outcome, share) {
                Ok(()) => {}
                Err(AdoptRefusal::OffPolynomial) => {
                    // Every pinned body is held (`want` empty), so no input is left to
                    // change and no retry can heal this epoch.
                    self.metrics.dkg_share_unrecoverable.inc();
                    tracing::warn!(
                        epoch = e,
                        "live DKG: the share recomputed over every pinned body is off the \
                         certified polynomial — UNRECOVERABLE here; this node sits the \
                         epoch out as a verifier"
                    );
                    self.set_state(e, EpochState::Unrecoverable { key: Some(digest) });
                    self.stall(e, StallReason::Unrecoverable);
                    continue;
                }
                Err(AdoptRefusal::PersistFailed) => {
                    // The disk failed, not the inputs: re-arm so the next tick retries.
                    self.stall(e, StallReason::PersistFailed);
                    self.set_recompute_attempted(e, false);
                    continue;
                }
            }
            // The eviction must stay below the adopt: the share file and the journal are
            // disjoint, so a crash in between would leave the node holding neither and
            // refetching every pinned body. Warm the serve cache first, from the journal
            // this is about to delete.
            self.log_store.warm_from_journal(e);
            self.evict_journal(e);
            self.set_state(e, EpochState::Keyed { digest });
            tracing::info!(
                epoch = e,
                "live DKG: demoted committee member recomputed its share from the retained \
                 journal — share stored, promoting to Signer"
            );
        }
    }

    async fn on_resolver_message(&mut self, msg: LogMessage, rng: &mut impl CryptoRngCore) {
        match msg {
            LogMessage::Produce { key, response } => {
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
                if valid {
                    // A recorded log just widened the set with no height tick behind it:
                    // `AnyGrowth` carries every width, including the intermediate ones
                    // `Decisive` leaves to a tick a halted chain will not deliver. Every
                    // member mints its own — the entry bar counts each one.
                    let minted = self.confirmations.mint(ConfirmTrigger::AnyGrowth);
                    self.broadcast_all(minted).await;
                }
            }
        }
        self.debug_assert_settled();
    }

    /// Serve the body held under exactly `{epoch, dealer, hash}` — never a dealer's
    /// other log. The live ceremony's recorded `signed_logs` first (an epoch still
    /// collecting is in memory only), then the [`DealerLogStore`] tiers.
    ///
    /// `None` when no tier holds it; the caller then drops the responder, the resolver
    /// answers "no data", and the requester retries another peer.
    fn serve_log(&mut self, key: &DkgLogKey) -> Option<Bytes> {
        let id: LogId = (key.dealer.clone(), key.hash);
        if let Some(signed) = self.ceremony(key.epoch).and_then(|c| c.signed_log(&id)) {
            return Some(signed.encode());
        }
        self.log_store.get(key.epoch, &id)
    }

    /// Ingest a `SignedDealerLog` delivered by the resolver for `{epoch, dealer, hash}`:
    /// decode, re-`check`, record (live ceremony or the `Acquiring(Logs)` heal), journal,
    /// and drive finalize.
    ///
    /// Returns the resolver's two-valued `deliver` verdict: `true` clears the fetch — the
    /// requested body, an honest duplicate, or an epoch that no longer needs it; `false`
    /// blocks this peer and retries the key elsewhere — a forgery, a valid log that is
    /// not the one asked for, or an undecodable delivery.
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
            // Undecodable bytes → `false`: `true` would clear the fetch, letting one
            // garbage-serving peer kill `key`'s recovery. `false` retries the key at
            // another peer, at the cost of blocking this one.
            Err(_) => return false,
        };
        if let Some(c) = self.ceremony_mut(key.epoch) {
            let (accepted, step) = c.ingest_signed_log(&(key.dealer.clone(), key.hash), signed);
            if accepted {
                let durable = self.append_journal(key.epoch, step.journal);
                // A two-log dealer is proven here: the fetched body is the second half of
                // the pair whenever the other one is already recorded.
                self.note_equivocation(key.epoch, step.equivocation.as_ref(), durable);
                if !durable {
                    if let Some(id) = step.recorded_log {
                        self.nondurable_logs
                            .entry(key.epoch)
                            .or_default()
                            .insert(id);
                    }
                }
                self.drive_finalization(rng);
            }
            return accepted;
        }
        // Past the live ceremony: an epoch that is still healing takes the body into its
        // retained journal, everything else clears the fetch.
        if matches!(
            self.state(key.epoch),
            Some(EpochState::Acquiring(Acquire::Logs(_)))
        ) {
            return self.ingest_recompute_log(key, signed, rng);
        }
        true
    }

    /// Ingest a resolver-delivered log for an `Acquiring(Logs)` epoch: `check` it against
    /// the pinned `Info` and, iff it is exactly the requested body, journal it, drop the
    /// id from `want` and attempt the recompute. Same verdict as [`Self::ingest_log`],
    /// plus `false` on an unreadable committee (a transient race — keep the fetch alive).
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
            Some((pk, _)) if pk == key.dealer && log_hash(&signed) == key.hash => {
                let durable =
                    self.append_journal(key.epoch, vec![JournalRecord::PeerLog(Box::new(signed))]);
                // Mark the body satisfied only once it is a durable part of the journal
                // the recompute reads: on a failed write `want` keeps it for a retry.
                if durable {
                    if let Some(EpochSlot {
                        state: EpochState::Acquiring(Acquire::Logs(st)),
                        ..
                    }) = self.epochs.get_mut(&key.epoch)
                    {
                        st.want.remove(&(key.dealer.clone(), key.hash));
                    }
                    self.set_recompute_attempted(key.epoch, false);
                    self.try_recompute(rng);
                }
                true
            }
            Some(_) => false, // a valid log, but not the body fetched
            None => false,    // a forgery
        }
    }

    async fn broadcast_all(&mut self, msgs: Vec<Outgoing>) {
        for o in msgs {
            let wire = BeaconMessage::Dkg(o.msg.encode()).encode();
            let recipients = match o.target {
                Target::Broadcast => Recipients::All,
                Target::Direct(pk) => Recipients::One(pk),
            };
            // Best-effort: a dropped dealing is re-sent by the dealer's per-tick
            // retransmit and a dropped ack by the player's ack-cache re-emit, so a send
            // failure never blocks consensus.
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
    use commonware_utils::{Faults as _, N3f1, NZUsize};
    use rand_08::{rngs::StdRng, SeedableRng as _};
    use std::time::Duration;

    type SimContext = deterministic::Context;

    /// The chain id the test artifacts' agreement namespace is derived under: the pull
    /// seam re-derives the namespace from it, so it has to agree with the signer.
    const AGREEMENT_CHAIN_ID: u64 = 20_994;

    /// No-op DKG-log resolver: the clock tests exercise the gossip/finalize path,
    /// not recovery fetch (covered by the resolver/ingest unit tests).
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

    /// The `changed` bit of a fixture that has no chain: unreadable for every epoch.
    /// One shared closure, so `standalone_actor_at` can recognise its own default with
    /// `Arc::ptr_eq` and leave a bit a test set itself alone.
    fn unreadable_bit() -> ChangedAt {
        static BIT: std::sync::OnceLock<ChangedAt> = std::sync::OnceLock::new();
        BIT.get_or_init(|| Arc::new(|_epoch: u64| None)).clone()
    }

    impl<R> Wiring<R> {
        /// Every edge inert: parked receivers (their senders live in [`Fixture`], so
        /// `run` sees a plane that never speaks rather than one that died), a no-op
        /// pull, an empty store, an unreadable `changed` bit, and a scratch directory
        /// removed with the actor. A test overrides the fields it exercises.
        fn inert(resolver: R) -> Self {
            let (resolver_tx, resolver_rx) = tokio::sync::mpsc::channel::<LogMessage>(1);
            let (pinned_tx, pinned_rx) = tokio::sync::mpsc::channel::<PinnedRequest>(1);
            let (artifacts_tx, artifacts_rx) = tokio::sync::mpsc::channel::<AgreedArtifact>(1);
            let (body_lost_tx, body_lost_rx) = tokio::sync::mpsc::channel::<u64>(1);
            let (agreement_tx, agreement_rx) = tokio::sync::mpsc::channel::<u64>(1);
            // No receiver: a watch with none is a publish nobody reads, never an error.
            let (epoch_clock, _) = tokio::sync::watch::channel(0u64);
            let scratch = fresh_share_dir("standalone");
            Self {
                resolver,
                resolver_rx,
                changed: unreadable_bit(),
                share_dir: scratch.clone(),
                plane_clock: PlaneClock::default(),
                outcome_at: Arc::new(|_epoch: u64| None),
                pull_artifact: Arc::new(|_epoch: u64| {}),
                recorded_dkg_logs: Arc::new(RwLock::new(BTreeMap::new())),
                confirms: ConfirmPool::new(b"FLUENT_TEST_STANDALONE"),
                pinned_rx,
                agreement_tx,
                epoch_clock,
                artifacts_rx,
                body_lost_rx,
                fixture: Some(Fixture {
                    scratch,
                    _resolver_tx: resolver_tx,
                    _pinned_tx: pinned_tx,
                    _artifacts_tx: artifacts_tx,
                    _body_lost_tx: body_lost_tx,
                    _agreement_rx: agreement_rx,
                }),
            }
        }
    }

    impl Wiring<NoopResolver> {
        /// [`Wiring::inert`] over the no-op resolver — the standalone default.
        fn standalone() -> Self {
            Self::inert(NoopResolver)
        }
    }

    /// Records its in-flight fetch set so tests can assert `fetch_missing_logs`
    /// cancels dead fetches through `retain`.
    #[derive(Clone, Default)]
    struct RecordingResolver {
        in_flight: Arc<std::sync::Mutex<BTreeSet<DkgLogKey>>>,
    }

    /// The one-value-per-epoch memory a set of [`spawn_stub_agreement`]s share.
    type Certified = Arc<std::sync::Mutex<BTreeMap<u64, (Vec<(u8, B256)>, DkgOutcome)>>>;
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

    const SEAL_DEADLINE: u64 = INTERVAL * DETERMINISTIC_BOOTSTRAP_EPOCH - DKG_MARGIN_BLOCKS; // 40
    const BOUNDARY: u64 = INTERVAL * DETERMINISTIC_BOOTSTRAP_EPOCH; // 60 = epoch_start(2)
    /// A positive deal window (`INTERVAL - DKG_MARGIN_BLOCKS = 10` ticks): the
    /// epoch-2 ceremony starts at epoch-1 start (30) and seals at the deadline (40).
    /// The zero-width geometry (`INTERVAL = DKG_MARGIN_BLOCKS`) never deals and is
    /// pinned by `at_a_zero_width_deal_window_a_missing_journal_at_the_deadline_heals_not_deals`;
    /// every `SEAL_DEADLINE`/`BOUNDARY`-relative drive below assumes this geometry.
    /// Production uses a much larger interval; this is test geometry, not a protocol value.
    const INTERVAL: u64 = 30;
    const ACTIVATION: u64 = 0;
    /// Result-final lag (the EL-finalized clock trails the ordering clock by this).
    const K: u64 = crate::K;

    /// Spawn one [`DkgActor`] over the simulated network and return its height sink.
    /// The fixed committee makes `DETERMINISTIC_BOOTSTRAP_EPOCH` the only DKG; every
    /// other epoch is a carry-forward.
    async fn spawn_dealer(
        ctx: &SimContext,
        oracle: &Oracle<PeerPubkey, SimContext>,
        me: Ed25519PrivateKey,
        committee: Set<PeerPubkey>,
        store: CeremonyStore,
        share_notify: Arc<tokio::sync::Notify>,
        interval: u64,
    ) -> tokio::sync::watch::Sender<u64> {
        spawn_dealer_at_sender(
            ctx,
            oracle,
            me,
            committee,
            store,
            share_notify,
            interval,
            None,
            7,
            Arc::new(RwLock::new(BTreeMap::new())),
        )
        .await
    }

    /// A stand-in for the epoch-key agreement plane on the actor's real seams: it
    /// takes the dealing-closed announcement and, once a quorum of dealer-log hashes
    /// is in `recorded`, hands that exact set back as an agreed artifact. Agreeing one
    /// set across the committee, quorum certificate and all, is covered in
    /// [`crate::beacon::dkg_agree`] / [`crate::beacon::dkg_engine`].
    ///
    /// It polls for the quorum on its own clock rather than on the next announcement,
    /// so a node whose height feed has frozen still gets its epoch agreed; one target
    /// at a time is enough; no test here runs two.
    ///
    /// `recorded` is the caller's choice. A per-node index is faithful (the real
    /// leader also proposes from its own), but this stub has one leader and no
    /// certification, so it stalls where the real plane nullifies the view and lets
    /// the next leader propose; a test about a node that claims less than it holds
    /// must hand the whole committee a single index.
    ///
    /// `certified` is the one thing the real plane has that a per-node stub does not:
    /// one value per epoch across the committee. Stubs sharing a map deliver the first
    /// set any of them certified, so a node holding less than a quorum still receives
    /// the set its peers agreed; a fresh map per spawn is the per-node behaviour.
    fn spawn_stub_agreement(
        ctx: &SimContext,
        recorded: DkgLogIndex,
        n: usize,
        certified: Certified,
    ) -> (
        tokio::sync::mpsc::Sender<u64>,
        tokio::sync::mpsc::Receiver<AgreedArtifact>,
        tokio::sync::mpsc::Receiver<PinnedRequest>,
    ) {
        let (announce_tx, mut announce_rx) = tokio::sync::mpsc::channel::<u64>(16);
        let (artifact_tx, artifact_rx) = tokio::sync::mpsc::channel::<AgreedArtifact>(16);
        // The key is derived through the actor's own pinned-set seam, as an instance's
        // `verify` does, so the artifact carries the polynomial the share must lie on.
        let (pinned_tx, pinned_rx) = tokio::sync::mpsc::channel::<PinnedRequest>(16);
        let quorum = <commonware_utils::N3f1 as commonware_utils::Faults>::quorum(n) as usize;
        drop(ctx.with_label("stub_agreement").spawn(move |c| async move {
            let mut agreed: BTreeSet<u64> = BTreeSet::new();
            while let Some(epoch) = announce_rx.recv().await {
                if agreed.contains(&epoch) {
                    continue;
                }
                // A set and key certified by any node are what every later node is
                // handed: a restarted node holds too few bodies to derive the key itself.
                let (held, group_key) = loop {
                    if let Some(set) = certified.lock().expect("certified").get(&epoch) {
                        break set.clone();
                    }
                    let held: Vec<(u8, B256)> = recorded
                        .read()
                        .ok()
                        .and_then(|m| m.get(&epoch).cloned())
                        .map(|m| m.into_iter().collect())
                        .unwrap_or_default();
                    if held.len() >= quorum {
                        match PinnedMailbox::new(epoch, pinned_tx.clone())
                            .derive(held.iter().copied().collect())
                            .await
                        {
                            PinnedDerive::Derived(key) => {
                                break certified
                                    .lock()
                                    .expect("certified")
                                    .entry(epoch)
                                    .or_insert((held, *key))
                                    .clone();
                            }
                            PinnedDerive::Unavailable => return,
                            _ => {}
                        }
                    }
                    c.sleep(Duration::from_millis(10)).await;
                };
                agreed.insert(epoch);
                if artifact_tx
                    .send(agreed_artifact_keyed(epoch, held, group_key))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }));
        (announce_tx, artifact_rx, pinned_rx)
    }

    /// [`spawn_dealer`] with an explicit `share_dir` (so the journal/share persist)
    /// and rng seed: a restart must resume from disk rather than from a deterministic
    /// re-deal. Returns only the height sink; [`spawn_dealer_at`] also hands back the
    /// adopted `Output`, which only `run_reveal_check` needs.
    #[allow(clippy::too_many_arguments)]
    async fn spawn_dealer_at_sender(
        ctx: &SimContext,
        oracle: &Oracle<PeerPubkey, SimContext>,
        me: Ed25519PrivateKey,
        committee: Set<PeerPubkey>,
        store: CeremonyStore,
        share_notify: Arc<tokio::sync::Notify>,
        interval: u64,
        share_dir: Option<PathBuf>,
        rng_seed: u64,
        recorded: DkgLogIndex,
    ) -> tokio::sync::watch::Sender<u64> {
        spawn_dealer_at(
            ctx,
            oracle,
            me,
            committee,
            store,
            share_notify,
            interval,
            share_dir,
            rng_seed,
            recorded,
        )
        .await
        .0
    }

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
        recorded: DkgLogIndex,
    ) -> (
        tokio::sync::watch::Sender<u64>,
        Arc<RwLock<BTreeMap<u64, CeremonyOutput>>>,
        ConfirmPool,
    ) {
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
        let (announce_tx, artifact_rx, pinned_rx) =
            spawn_stub_agreement(ctx, recorded.clone(), committee.len(), Certified::default());
        let mut wiring = Wiring::standalone();
        if let Some(dir) = share_dir {
            wiring.share_dir = dir;
        }
        wiring.recorded_dkg_logs = recorded;
        wiring.agreement_tx = announce_tx;
        wiring.artifacts_rx = artifact_rx;
        wiring.pinned_rx = pinned_rx;
        // The contract's rule over this fixture's single committee: unchanged for
        // every epoch, so the only ceremony start is the unconditional bootstrap mint.
        wiring.changed = Arc::new(|_epoch: u64| Some(false));
        // The pool this dealer counts peers' `ShareConfirm`s into; returned so
        // `run_reveal_check` can assert a peer's confirmation reached it.
        let confirms = wiring.confirms.clone();
        let actor = DkgActor::new(
            b"FLUENT_DPOS_V1_clocktest".to_vec(),
            me,
            sender,
            receiver,
            committee_for,
            store,
            share_notify,
            ACTIVATION,
            interval,
            crate::beacon::metrics::BeaconMetrics::default(),
            ShareState::Plaintext,
            wiring,
        );
        let adopted = actor.adopted_outcomes.clone();
        let (height_tx, height_rx) = tokio::sync::watch::channel(0u64);
        let rng = StdRng::seed_from_u64(rng_seed);
        drop(
            ctx.with_label("dealer")
                .spawn(move |_c| async move { actor.run(height_rx, rng).await }),
        );
        (height_tx, adopted, confirms)
    }

    /// [`spawn_dealer_at`] over a real `commonware_resolver::p2p::Engine` instead of
    /// the `NoopResolver`: it exercises the
    /// `fetch_missing_logs`/`serve_log`/`ingest_log` round-trip over the sim network.
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
        certified: Certified,
    ) -> tokio::sync::watch::Sender<u64> {
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
        // Engines register their metrics under the context label and all nodes share
        // one `ctx`, so the label must be unique per spawn or the registry panics.
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
        let recorded: DkgLogIndex = Arc::new(RwLock::new(BTreeMap::new()));
        let (announce_tx, artifact_rx, pinned_rx) =
            spawn_stub_agreement(ctx, recorded.clone(), committee.len(), certified);
        let actor = DkgActor::new(
            b"FLUENT_DPOS_V1_clocktest".to_vec(),
            me,
            sender,
            receiver,
            committee_for,
            store,
            share_notify,
            ACTIVATION,
            interval,
            crate::beacon::metrics::BeaconMetrics::default(),
            ShareState::Plaintext,
            {
                let mut wiring = Wiring::inert(mailbox);
                wiring.resolver_rx = log_rx;
                if let Some(dir) = share_dir {
                    wiring.share_dir = dir;
                }
                wiring.recorded_dkg_logs = recorded;
                wiring.agreement_tx = announce_tx;
                wiring.artifacts_rx = artifact_rx;
                wiring.pinned_rx = pinned_rx;
                wiring.changed = Arc::new(|_epoch: u64| Some(false));
                wiring
            },
        );
        let (height_tx, height_rx) = tokio::sync::watch::channel(0u64);
        let rng = StdRng::seed_from_u64(rng_seed);
        drop(
            ctx.with_label("dealer_resolved")
                .spawn(move |_c| async move { actor.run(height_rx, rng).await }),
        );
        height_tx
    }

    /// The 4-dealer bootstrap DKG over the sim network, driven with `lag` (0 =
    /// ordering clock, `K` = EL-finalized) up to `feed_to`; returns whether the victim
    /// memoized `(PK_2, share)`.
    async fn dkg_seeded_by(ctx: SimContext, lag: u64, feed_to: u64) -> bool {
        // freeze_at == feed_to ⇒ feed the whole range; no late starters.
        dkg_seeded_with_freeze(ctx, lag, feed_to, feed_to, INTERVAL, 0, 0).await
    }

    /// The 4-dealer bootstrap DKG over the sim network; returns whether the victim
    /// (node 0) memoized `(PK_2, share)`. `lag` is subtracted from every fed height
    /// (0 = ordering clock, `K` = EL-finalized). `freeze_at` stops the feed while the
    /// sim keeps ticking, modelling the boundary stall. The first `late_count` nodes
    /// start `late_lag` ticks late, so peers' dealings arrive before their own start
    /// and must be buffered and acked rather than dropped (a drop fails the ceremony).
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
                let node_h = if i < late_count {
                    h.saturating_sub(late_lag)
                } else {
                    h
                };
                // Past `freeze_at` the feed stops but the ticking continues; the ticking
                // is what delivers the in-flight Reveals.
                if node_h <= freeze_at {
                    s.send_replace(node_h.saturating_sub(lag));
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

    /// The ordering clock seals committee[2]'s ceremony at `SEAL_DEADLINE` and
    /// finalizes within a couple of ticks; the EL-finalized clock (lagged by `K`)
    /// reaches the deadline `K` ticks later, so by `SEAL_DEADLINE + 2` it has not
    /// memoized the share: the lag eats `K` blocks of the `DKG_MARGIN_BLOCKS` window.
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

    /// Feed heights only to the seal deadline, so every dealer seals and broadcasts its
    /// `Reveal`, then freeze the feed (reth-finalized stuck at the unfinalizable
    /// boundary block) while the sim keeps delivering them: the victim must still
    /// memoize `(PK_2, share)`, driven by `on_message` -> `drive_finalization`.
    #[test]
    fn frozen_feed_seeds_via_reveal_event() {
        // One tick past the deadline, so the seal has landed before the feed freezes.
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

    /// Start-race: with a real dealing window (interval 30, margin 20, the ceremony
    /// enters at 30 and seals at the deadline 40) the first two nodes start 2 ticks
    /// late, so each receives the early dealers' `Commitment`+`Share` before its own
    /// start; those must be buffered and drained on start, or the early dealers go
    /// un-acked and the ceremony never reaches quorum.
    #[test]
    fn start_race_buffers_early_dealings() {
        const INTERVAL_LONG: u64 = 30;
        let runtime = deterministic::Runner::default();
        let seeded = runtime.start(|ctx| async move {
            dkg_seeded_with_freeze(ctx, 0, 56, 56, INTERVAL_LONG, 2, 2).await
        });
        assert!(
            seeded,
            "early peer dealings that race ahead of the start must be buffered + \
             drained (not dropped) so every dealer is acked and the victim seeds"
        );
    }

    /// A ceremony whose committee never reaches a quorum (only the victim deals here)
    /// is sealed at the deadline but never finalized, so `drive_finalization` never
    /// removes it. It must survive the epoch boundary, where an agreement that has not
    /// converged still asks this actor for the bodies, and be evicted once the epoch
    /// ages out of the retention window. Drives `on_height` directly.
    #[test]
    fn stalled_ceremony_is_evicted_once_its_epoch_ages_out() {
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
            // n=4 means quorum 3, and only the victim runs, so the log never reaches it.
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
                committee_for,
                store,
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                ShareState::Plaintext,
                Wiring::standalone(),
            );
            let mut arng = StdRng::seed_from_u64(9);

            // Through the seal deadline: committee[2] enters at the epoch-1 start (30)
            // and seals at the deadline (40), then stalls (one valid log < quorum 3).
            for h in 0..=(SEAL_DEADLINE + 2) {
                actor.on_height(h, &mut arng).await;
            }
            assert!(
                actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .is_some_and(|c| c.dealing_closed()),
                "precondition: committee[2] must be sealed-but-stalled before the boundary"
            );

            // Crossing the epoch boundary must not sweep it: an agreement for epoch 2
            // can still be running there.
            for h in (SEAL_DEADLINE + 3)..=(BOUNDARY + 1) {
                actor.on_height(h, &mut arng).await;
            }
            assert!(
                actor.ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH).is_some(),
                "the chain entering the target epoch must not sweep its ceremony"
            );

            let aged_out =
                INTERVAL * (DETERMINISTIC_BOOTSTRAP_EPOCH + JOURNAL_RETENTION_EPOCHS + 1);
            for h in (BOUNDARY + 2)..=aged_out {
                actor.on_height(h, &mut arng).await;
            }
            assert!(
                actor.ceremonies().next().is_none(),
                "stalled ceremony must be evicted once its epoch ages out of the window"
            );
        });
    }

    /// The start-race buffer is bounded per sender: a peer flooding its own
    /// `Commitment` fills at most its single slot (latest-wins), so it can never evict
    /// another sender's buffered dealing. Drives `on_message` directly.
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
                committee_for,
                store,
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                ShareState::Plaintext,
                Wiring::standalone(),
            );
            let mut arng = StdRng::seed_from_u64(13);

            // A real commitment for epoch 1, inside the buffer window while no height
            // has been drained; the body is not verified before buffering, so the same
            // bytes stand in for any sender; only the `from` key selects the slot.
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

            for _ in 0..5 {
                actor.on_message(a.clone(), wire.as_ref(), &mut arng).await;
            }
            assert_eq!(
                actor.pending.get(&1).map(|m| m.len()),
                Some(1),
                "5 copies of sender A's commitment occupy exactly ONE slot (latest-wins)"
            );

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

    /// Mid-window restart: the victim (node 0) runs the bootstrap DKG with a
    /// persistent `share_dir`, seals and journals, then is dropped (its task aborted by
    /// the re-register) and rebuilt with fresh in-memory state and the same `share_dir`.
    /// It must `resume` from the journal and memoize `(PK_2, share)` before the boundary.
    #[test]
    fn restart_midwindow_recovers_via_journal() {
        const RESTART_AT: u64 = SEAL_DEADLINE + 2; // after node-0's seal
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
                    spawn_dealer_at_sender(
                        &ctx,
                        &oracle,
                        k.clone(),
                        committee.clone(),
                        store,
                        Arc::new(tokio::sync::Notify::new()),
                        INTERVAL,
                        dir_i,
                        7,
                        Arc::new(RwLock::new(BTreeMap::new())),
                    )
                    .await,
                );
            }

            // Node-3 lags by LATE, so node-0 seals while still missing its log: the
            // partial-progress state the journal must carry across the restart.
            const LATE: u64 = 4;
            let feed_round = |h: u64| {
                let node3_h = h.saturating_sub(LATE);
                (h, node3_h)
            };

            for h in 0..=RESTART_AT {
                let (h0, h3) = feed_round(h);
                for (i, s) in sinks.iter().enumerate() {
                    s.send_replace(if i == 3 { h3 } else { h0 });
                }
                ctx.sleep(Duration::from_millis(50)).await;
            }

            // Fresh store and rng seed: recovery is journal-driven, not a deterministic re-deal.
            drop(sinks.remove(0));
            let restarted_store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let new_sink = spawn_dealer_at_sender(
                &ctx,
                &oracle,
                keys[0].clone(),
                committee.clone(),
                restarted_store.clone(),
                Arc::new(tokio::sync::Notify::new()),
                INTERVAL,
                Some(dir.clone()),
                99,
                Arc::new(RwLock::new(BTreeMap::new())),
            )
            .await;
            sinks.insert(0, new_sink);

            for h in (RESTART_AT + 1)..=FEED_TO {
                let (h0, h3) = feed_round(h);
                for (i, s) in sinks.iter().enumerate() {
                    s.send_replace(if i == 3 { h3 } else { h0 });
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

    /// Resolver ingest: valid logs converge a shorthanded ceremony to a selectable quorum;
    /// forged/undecodable/wrong-dealer logs are rejected; a wrong-epoch one is honest (`true`).
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

            // Same namespace/epoch/committee ⇒ the sealed logs `check` against node-0's ceremony.
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
                committee_for,
                Arc::new(RwLock::new(BTreeMap::new())),
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                ShareState::Plaintext,
                Wiring::standalone(),
            );
            // Unsealed: `ingest_log`'s finalize would consume the ceremony mid-test.
            let mut arng = StdRng::seed_from_u64(9);
            for h in 0..SEAL_DEADLINE {
                actor.on_height(h, &mut arng).await;
            }
            assert!(
                !actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .expect("ceremony")
                    .dealing_closed(),
                "precondition: node-0's ceremony is started but unsealed"
            );
            assert_eq!(
                actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .expect("ceremony")
                    .recorded_log_count(),
                0,
                "before recovery node-0 has recorded no logs (unsealed ⇒ no own log yet)"
            );

            // `ingest_log` binds the delivered log to the requested `{epoch, dealer, hash}`.
            let dealer0 = keys[1].public_key();
            let valid_key0 = DkgLogKey {
                epoch: DETERMINISTIC_BOOTSTRAP_EPOCH,
                dealer: dealer0.clone(),
                hash: log_hash(&peer_logs[0]),
            };

            let wrong_epoch_key = DkgLogKey {
                epoch: 3,
                dealer: dealer0.clone(),
                hash: log_hash(&peer_logs[0]),
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
                actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .expect("ceremony")
                    .recorded_log_count(),
                0,
                "a wrong-epoch delivery must not touch the epoch-2 ceremony"
            );

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
                actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .expect("ceremony")
                    .recorded_log_count(),
                0,
                "a forged log does not poison the set"
            );

            // Undecodable must be `false`: `true` would clear the fetch, killing `key`'s recovery.
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
                actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .expect("ceremony")
                    .recorded_log_count(),
                0,
                "an undecodable delivery records nothing"
            );

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
                actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .expect("ceremony")
                    .recorded_log_count(),
                0,
                "a wrong-dealer log is not recorded under the fetched key"
            );

            let quorum = peer_logs.len();
            for (i, signed) in peer_logs.iter().enumerate() {
                let key = DkgLogKey {
                    epoch: DETERMINISTIC_BOOTSTRAP_EPOCH,
                    dealer: keys[i + 1].public_key(),
                    hash: log_hash(signed),
                };
                let ok = actor
                    .ingest_log(&key, Bytes::from(signed.encode().to_vec()), &mut arng)
                    .await;
                assert!(
                    ok,
                    "a valid log for its own dealer is recorded (deliver→true)"
                );
            }
            let c = actor
                .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                .expect("ceremony");
            assert_eq!(
                c.recorded_log_count(),
                quorum,
                "every valid delivered log is recorded"
            );
            let pinned: BTreeMap<u8, B256> = committee
                .iter()
                .enumerate()
                .filter_map(|(idx, pk)| c.signed_log_hash(pk).map(|h| (idx as u8, h)))
                .collect();
            assert_eq!(
                c.pinned_ready(&mut arng, &committee, &pinned),
                (true, true),
                "a dealer-quorum of valid logs is selectable after recovery"
            );
        });
    }

    /// A cold-miss serve for an unservable epoch (absent or Torn journal) returns `None` and
    /// caches nothing, so attacker-named far-future epochs cannot grow the serve cache.
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

            // A one-byte file parses as a torn journal (`JournalLoad::Torn`).
            let dir = fresh_share_dir("neg-cache");
            std::fs::create_dir_all(&dir).expect("mkdir");
            std::fs::write(journal_path(&dir, 2), [0x01]).expect("write torn journal");

            let mut actor =
                standalone_actor(&oracle, keys[0].clone(), committee, Some(dir.clone())).await;
            let key = DkgLogKey {
                epoch: 2,
                dealer: keys[1].public_key(),
                hash: B256::repeat_byte(0x22),
            };
            assert!(
                actor.serve_log(&key).is_none(),
                "a Torn journal serves no log"
            );
            assert!(
                actor.log_store.cached(2).is_none(),
                "an unservable cold miss caches NOTHING — no negative entry ([965]/[954])"
            );
            for e in 1_000u64..1_050 {
                assert!(actor
                    .serve_log(&DkgLogKey {
                        epoch: e,
                        dealer: keys[1].public_key(),
                        hash: B256::repeat_byte(0x22),
                    })
                    .is_none());
            }
            assert!(
                actor.log_store.cache_is_empty(),
                "future-epoch cold misses accumulate no entries — the cache is bounded ([954])"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// A transient `committee_for`→`None` on a present journal must not poison the epoch's serve:
    /// the empty cold-load is not cached, so the next serve re-parses and serves correctly.
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
            let (dealer, signed) = logs
                .iter()
                .find(|(pk, _)| *pk == keys[1].public_key())
                .expect("keys[1] sealed");
            let key = DkgLogKey {
                epoch: 2,
                dealer: dealer.clone(),
                hash: log_hash(signed),
            };

            assert!(
                actor.serve_log(&key).is_none(),
                "transient None serves no log"
            );
            assert!(
                actor.log_store.cached(2).is_none(),
                "the transient-None empty result is NOT cached → no permanent poison ([965])"
            );

            readable.store(true, std::sync::atomic::Ordering::Relaxed);
            assert!(
                actor.serve_log(&key).is_some(),
                "once the committee is readable the epoch serves correctly — never poisoned ([965])"
            );
            assert!(
                actor.log_store.cached(2).is_some_and(|m| !m.is_empty()),
                "the now-servable epoch is cached POSITIVELY"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// `fetch_missing_logs` cancels the resolver's in-flight fetches for an epoch that is no
    /// longer live (finalized or swept), so its dead keys stop being re-issued.
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
                committee_for,
                Arc::new(RwLock::new(BTreeMap::new())),
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                ShareState::Plaintext,
                Wiring::inert(resolver),
            );

            let (cer, _step) = DkgCeremony::start(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                keys[0].clone(),
            )
            .expect("start");
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, cer);
            let pinned: BTreeMap<u8, B256> = committee
                .iter()
                .enumerate()
                .filter(|(_, pk)| **pk != keys[0].public_key())
                .map(|(i, _)| (i as u8, B256::repeat_byte(0x40 + i as u8)))
                .collect();
            actor.apply_artifact(
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                &agreed_artifact(
                    DETERMINISTIC_BOOTSTRAP_EPOCH,
                    pinned.iter().map(|(i, h)| (*i, *h)).collect(),
                )
                .0,
            );
            actor.fetch_missing_logs().await;
            let expected: BTreeSet<DkgLogKey> = pinned
                .iter()
                .map(|(i, h)| DkgLogKey {
                    epoch: DETERMINISTIC_BOOTSTRAP_EPOCH,
                    dealer: committee.iter().nth(*i as usize).expect("seat").clone(),
                    hash: *h,
                })
                .collect();
            assert_eq!(
                *in_flight.lock().unwrap(),
                expected,
                "an open ceremony with an agreed set fetches exactly the pinned bodies it lacks"
            );

            actor.epochs.clear();
            actor.fetch_missing_logs().await;
            assert!(
                in_flight.lock().unwrap().is_empty(),
                "fetches for an epoch with no open ceremony are cancelled (the [804] leak fix)"
            );
        });
    }

    /// A transient `committee_for`→`None` for a live ceremony must not cancel its in-flight
    /// recovery fetches: `retain` keeps keys of a live epoch we merely failed to read.
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
                committee_for,
                Arc::new(RwLock::new(BTreeMap::new())),
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                ShareState::Plaintext,
                Wiring::inert(resolver),
            );

            let (cer, _step) = DkgCeremony::start(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                keys[0].clone(),
            )
            .expect("start");
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, cer);
            let pinned: BTreeMap<u8, B256> = committee
                .iter()
                .enumerate()
                .filter(|(_, pk)| **pk != keys[0].public_key())
                .map(|(i, _)| (i as u8, B256::repeat_byte(0x40 + i as u8)))
                .collect();
            actor.apply_artifact(
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                &agreed_artifact(DETERMINISTIC_BOOTSTRAP_EPOCH, pinned.into_iter().collect()).0,
            );

            actor.fetch_missing_logs().await;
            assert!(
                !in_flight.lock().unwrap().is_empty(),
                "an open ceremony with an agreed set issues fetches for the pinned bodies it lacks"
            );

            readable.store(false, std::sync::atomic::Ordering::Relaxed);
            actor.fetch_missing_logs().await;
            assert!(
                !in_flight.lock().unwrap().is_empty(),
                "a transient committee_for->None for a LIVE ceremony preserves its in-flight fetches ([893])"
            );
        });
    }

    /// A finalized-but-pre-boundary node still serves a peer's log from the eager serve store
    /// (no journal read, no `check`) so a late-restarting peer can recover it.
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
            for h in 0..=(BOUNDARY - 1) {
                for s in &sinks {
                    s.send_replace(h);
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

            // Standalone: assert the serve index directly, not via the spawned task's map.
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
                committee_for,
                Arc::new(RwLock::new(BTreeMap::new())),
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                ShareState::Plaintext,
                Wiring::standalone(),
            );
            // Node-0's ceremony is fully recorded, so the derived finalize gate passes.
            actor.insert_ceremony(
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                cers.remove(&keys[0].public_key()).unwrap(),
            );
            let mut arng = StdRng::seed_from_u64(9);
            let peer = keys[1].public_key();
            let peer_hash = actor.ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH).expect("ceremony")
                .signed_log_hash(&peer)
                .expect("the peer's log is recorded");
            actor.pin_recorded_as_agreed(DETERMINISTIC_BOOTSTRAP_EPOCH, &mut arng);
            actor.drive_finalization(&mut arng);
            assert!(
                actor.ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH).is_none(),
                "the finalized ceremony left `ceremonies`"
            );

            let key = DkgLogKey {
                epoch: DETERMINISTIC_BOOTSTRAP_EPOCH,
                dealer: peer.clone(),
                hash: peer_hash,
            };
            assert!(
                actor.serve_log(&key).is_some(),
                "a finalized-but-pre-boundary node still serves a peer log (the residual close)"
            );

            // Within JOURNAL_RETENTION_EPOCHS past the boundary the logs stay servable: a demoted
            // peer may still need to recompute its share from them.
            actor.on_height(BOUNDARY + 1, &mut arng).await;
            assert!(
                actor.serve_log(&key).is_some(),
                "a finalized epoch's logs stay servable within the retention window past the boundary"
            );

            let past_window =
                INTERVAL * (DETERMINISTIC_BOOTSTRAP_EPOCH + JOURNAL_RETENTION_EPOCHS + 1);
            actor.on_height(past_window, &mut arng).await;
            assert!(
                actor.serve_log(&key).is_none(),
                "the finalized-log serve index is reclaimed once the epoch ages out of the retention window"
            );
        });
    }

    /// A dealer whose held body differs from the pinned one is refetched by the pinned
    /// `(epoch, dealer, hash)` key, not by the dealer; a body delivered under another hash is
    /// refused, one under the pinned hash proves the equivocation and completes the set.
    #[test]
    fn a_held_body_that_is_not_the_pinned_one_is_refetched_by_the_pinned_hash() {
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
            let mut rng = StdRng::seed_from_u64(0x2002);
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;
            let ns = b"FLUENT_DPOS_V1_clocktest";
            let (me0, d1) = (keys[0].public_key(), keys[1].public_key());

            // A second `check`-valid log over the same `Info`: an independent
            // polynomial with no acks.
            let log2: DealerReveal = {
                use commonware_cryptography::bls12381::dkg::Dealer;
                let (d, _, _) = Dealer::<_, Ed25519PrivateKey>::start::<N3f1>(
                    StdRng::seed_from_u64(0x5EC0),
                    info_for_test(&committee),
                    keys[1].clone(),
                    None,
                )
                .expect("second dealer");
                d.finalize::<N3f1>()
            };
            let h2 = log_hash(&log2);

            let mut cers: BTreeMap<PeerPubkey, DkgCeremony> = BTreeMap::new();
            let mut queue: Vec<(PeerPubkey, Outgoing)> = Vec::new();
            for k in &keys {
                let (cer, step) =
                    DkgCeremony::start(ns, 2, committee.clone(), k.clone()).expect("start");
                let from = k.public_key();
                queue.extend(step.outgoing.into_iter().map(|o| (from.clone(), o)));
                cers.insert(from, cer);
            }
            let deliver = |cers: &mut BTreeMap<PeerPubkey, DkgCeremony>,
                           queue: &mut Vec<(PeerPubkey, Outgoing)>| {
                while let Some((from, o)) = queue.pop() {
                    let tos: Vec<PeerPubkey> = match &o.target {
                        Target::Broadcast => cers.keys().filter(|p| **p != from).cloned().collect(),
                        Target::Direct(to) => vec![to.clone()],
                    };
                    for to in tos {
                        let body = match &o.msg.body {
                            DkgBody::Reveal(_) if from == d1 && to == me0 => {
                                DkgBody::Reveal(Box::new(log2.clone()))
                            }
                            body => body.clone(),
                        };
                        let more = cers.get_mut(&to).unwrap().handle(from.clone(), body);
                        queue.extend(more.outgoing.into_iter().map(|m| (to.clone(), m)));
                    }
                }
            };
            deliver(&mut cers, &mut queue);
            for k in &keys {
                let step = cers.get_mut(&k.public_key()).unwrap().seal_dealings();
                queue.extend(step.outgoing.into_iter().map(|o| (k.public_key(), o)));
            }
            deliver(&mut cers, &mut queue);
            let h1 = cers[&keys[2].public_key()]
                .signed_log_hash(&d1)
                .expect("node 2 recorded dealer 1's sealed log");
            let log1 = cers[&keys[2].public_key()]
                .signed_log(&(d1.clone(), h1))
                .expect("held")
                .clone();
            assert_ne!(h1, h2);
            assert_eq!(
                cers[&me0].signed_log_hash(&d1),
                Some(h2),
                "node 0 holds the OTHER body"
            );
            assert!(!cers[&me0].holds(&(d1.clone(), h1)));
            let pinned: BTreeMap<u8, B256> = committee
                .iter()
                .enumerate()
                .map(|(i, pk)| {
                    (
                        i as u8,
                        cers[&keys[2].public_key()]
                            .signed_log_hash(pk)
                            .expect("recorded"),
                    )
                })
                .collect();
            let seat1 = committee.iter().position(|pk| *pk == d1).expect("seat") as u8;
            assert_eq!(pinned[&seat1], h1);

            let (sender, receiver) = oracle
                .control(me0.clone())
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
                ns.to_vec(),
                keys[0].clone(),
                sender,
                receiver,
                committee_for,
                Arc::new(RwLock::new(BTreeMap::new())),
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                ShareState::Plaintext,
                Wiring::inert(resolver),
            );
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, cers.remove(&me0).unwrap());
            let mut arng = StdRng::seed_from_u64(9);
            // Derived over the pinned set by node 2, which holds every pinned body.
            let pinned_logs: Vec<(u8, B256)> = pinned.iter().map(|(i, h)| (*i, *h)).collect();
            let key = key_over(
                &cers[&keys[2].public_key()],
                &committee,
                &pinned_logs,
                &mut arng,
            );
            actor
                .on_artifact(
                    agreed_artifact_keyed(DETERMINISTIC_BOOTSTRAP_EPOCH, pinned_logs, key),
                    &mut arng,
                )
                .await;
            assert!(
                actor.ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH).is_some(),
                "the pinned body at dealer 1's seat is not held, so nothing finalized yet"
            );

            let wanted = DkgLogKey {
                epoch: DETERMINISTIC_BOOTSTRAP_EPOCH,
                dealer: d1.clone(),
                hash: h1,
            };
            assert_eq!(
                *in_flight.lock().unwrap(),
                BTreeSet::from([wanted.clone()]),
                "a dealer whose held body is not the pinned one is fetched by the pinned hash"
            );

            let key_h2 = DkgLogKey {
                hash: h2,
                ..wanted.clone()
            };
            let key_zero = DkgLogKey {
                hash: B256::ZERO,
                ..wanted.clone()
            };
            assert!(
                actor.serve_log(&key_h2).is_some(),
                "the held body is served under its hash"
            );
            assert!(
                actor.serve_log(&wanted).is_none(),
                "a body this node does not hold is not served, dealer match or not"
            );
            assert!(
                actor.serve_log(&key_zero).is_none(),
                "a zero hash names no body — there is no wildcard"
            );

            assert!(
                !actor.ingest_log(&key_h2, log1.encode(), &mut arng).await,
                "a valid log that is not the body asked for does not satisfy the fetch"
            );
            assert_eq!(actor.metrics.dkg_dealer_equivocation.get(), 0);
            assert!(actor.ingest_log(&wanted, log1.encode(), &mut arng).await);
            assert_eq!(
                actor.metrics.dkg_dealer_equivocation.get(),
                1,
                "the victim is where both bodies meet: the equivocation is proven on the refetch"
            );
            assert!(
                actor.ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH).is_none(),
                "every pinned body held ⇒ finalized"
            );
            assert!(
                actor
                    .store
                    .read()
                    .unwrap()
                    .contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH),
                "the victim holds its share"
            );
            assert_eq!(actor.metrics.dkg_ceremony_ok.get(), 1);
            assert!(actor.serve_log(&wanted).is_some() && actor.serve_log(&key_h2).is_some());
            actor.fetch_missing_logs().await;
            assert!(
                in_flight.lock().unwrap().is_empty(),
                "nothing left to fetch once the pinned set is held"
            );

            assert_eq!(
                actor
                    .evidence(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .get(&d1)
                    .copied(),
                Some(DealerEquivocation {
                    first: h2,
                    second: h1
                }),
                "the pair is held by the actor after the ceremony is gone"
            );
            actor
                .on_height(
                    INTERVAL * (DETERMINISTIC_BOOTSTRAP_EPOCH + JOURNAL_RETENTION_EPOCHS + 1),
                    &mut arng,
                )
                .await;
            assert!(
                actor.evidence(DETERMINISTIC_BOOTSTRAP_EPOCH).is_empty(),
                "the pair ages out with the epoch's other maps at the sweep"
            );
        });
    }

    /// A journal carrying a `DealerEquivocation` record resumes into a ceremony that
    /// holds the pair, and the actor copies it out on the resume edge — so the evidence
    /// that outlives the ceremony exists on a restarted node too.
    #[test]
    fn an_evidence_pair_comes_back_with_the_journal_on_a_restart() {
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
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(57);
            oracle.manager().track(0, committee.clone()).await;
            let keys: Vec<Ed25519PrivateKey> = {
                let mut rng = StdRng::seed_from_u64(57);
                (0..4)
                    .map(|_| Ed25519PrivateKey::random(&mut rng))
                    .collect()
            };
            assert_eq!(keys[0].public_key(), key0.public_key());
            let d1 = keys[1].public_key();
            let info = info_for_test(&committee);
            let log1 = journal
                .iter()
                .find_map(|r| match r {
                    JournalRecord::PeerLog(l)
                        if l.clone().check(&info).is_some_and(|(pk, _)| pk == d1) =>
                    {
                        Some((**l).clone())
                    }
                    _ => None,
                })
                .expect("dealer 1's log is journaled");
            let log2: DealerReveal = {
                use commonware_cryptography::bls12381::dkg::Dealer;
                let (d, _, _) = Dealer::<_, Ed25519PrivateKey>::start::<N3f1>(
                    StdRng::seed_from_u64(0x5EC1),
                    info.clone(),
                    keys[1].clone(),
                    None,
                )
                .expect("second dealer");
                d.finalize::<N3f1>()
            };
            let (h1, h2) = (log_hash(&log1), log_hash(&log2));
            assert_ne!(h1, h2);

            let dir = fresh_share_dir("evidence-restart");
            std::fs::create_dir_all(&dir).expect("mkdir");
            for r in journal {
                share_state::append_journal(&dir, 2, &r, &ShareState::Plaintext).expect("append");
            }
            share_state::append_journal(
                &dir,
                2,
                &JournalRecord::DealerEquivocation(Box::new(log1), Box::new(log2)),
                &ShareState::Plaintext,
            )
            .expect("append evidence");

            let mut actor =
                standalone_actor(&oracle, key0, committee.clone(), Some(dir.clone())).await;
            assert!(
                actor.evidence(DETERMINISTIC_BOOTSTRAP_EPOCH).is_empty(),
                "a fresh actor holds no evidence"
            );
            // Past the seal deadline `recover` resumes the journal player-only.
            let mut arng = StdRng::seed_from_u64(9);
            actor.on_height(SEAL_DEADLINE + 1, &mut arng).await;
            assert!(
                actor.ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH).is_some(),
                "the journal resumed the ceremony"
            );
            assert_eq!(
                actor
                    .evidence(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .get(&d1)
                    .copied(),
                Some(DealerEquivocation {
                    first: h1,
                    second: h2
                }),
                "the replayed pair is copied out of the resumed ceremony"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// Every node-local gap answers `Unavailable`, which parks the agreement's `verify`:
    /// `Unusable` is the one arm that can nullify a view for the whole network, so it must
    /// never stand in for a gap this node merely cannot answer.
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

            let readable =
                standalone_actor(&oracle, keys[0].clone(), committee.clone(), None).await;
            assert!(
                matches!(
                    readable.derive_pinned(&request, &mut arng),
                    PinnedDerive::Unavailable
                ),
                "no ceremony for the epoch must park the verify, not nullify the view"
            );

            // An unreadable roster: the transient EVM read race.
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

    /// Path of `epoch`'s on-disk DKG journal under `dir`.
    fn journal_path(dir: &std::path::Path, epoch: u64) -> PathBuf {
        dir.join(format!("beacon-dkgjournal-e{epoch}.bin"))
    }

    use crate::beacon::log_store::COLD_PARSE_COUNT;

    /// Serializes the tests whose `serve`/`recover` paths cold-load the journal and so touch
    /// the process-global `COLD_PARSE_COUNT`, which two of them assert on exactly.
    static COLD_PARSE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// A standalone actor (no network drive) over `committee` with the given `share_dir` and
    /// a cold (empty-cache) serve store.
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
        standalone_actor_wired(oracle, me, committee, share_dir, Wiring::standalone()).await
    }

    /// [`standalone_actor`] over the edges a test wires itself (`wiring`).
    async fn standalone_actor_wired<R: Resolver<Key = DkgLogKey, PublicKey = PeerPubkey>>(
        oracle: &Oracle<PeerPubkey, SimContext>,
        me: Ed25519PrivateKey,
        committee: Set<PeerPubkey>,
        share_dir: Option<PathBuf>,
        wiring: Wiring<R>,
    ) -> DkgActor<
        commonware_p2p::simulated::Sender<PeerPubkey, SimContext>,
        commonware_p2p::simulated::Receiver<PeerPubkey>,
        R,
    > {
        let committee_for: CommitteeFor = {
            let set = committee.clone();
            Arc::new(move |_e: u64| Some(set.clone()))
        };
        standalone_actor_at(oracle, me, committee_for, share_dir, INTERVAL, wiring).await
    }

    /// [`standalone_actor`] over an explicit `committee_for` closure, so a test can model a
    /// transient `committee_for → None` EVM read race.
    async fn standalone_actor_cf(
        oracle: &Oracle<PeerPubkey, SimContext>,
        me: Ed25519PrivateKey,
        cf: CommitteeFor,
        share_dir: Option<PathBuf>,
    ) -> DkgActor<
        commonware_p2p::simulated::Sender<PeerPubkey, SimContext>,
        commonware_p2p::simulated::Receiver<PeerPubkey>,
        NoopResolver,
    > {
        standalone_actor_at(oracle, me, cf, share_dir, INTERVAL, Wiring::standalone()).await
    }

    /// [`standalone_actor_cf`] over an explicit epoch `interval`, the one knob a test of the
    /// deal-window geometry turns (`INTERVAL` is a positive window, `DKG_MARGIN_BLOCKS` a
    /// zero-width one), and the test's own `wiring`.
    ///
    /// A `share_dir` the test names replaces the fixture's scratch one, and a `wiring` still
    /// carrying the fixture's unreadable `changed` gets the rule the contract applies —
    /// `changed[e] = committee[e] != committee[e-1]` over `cf` — since a fixture with no
    /// chain has no bit of its own. A bit the test set itself stands.
    async fn standalone_actor_at<R: Resolver<Key = DkgLogKey, PublicKey = PeerPubkey>>(
        oracle: &Oracle<PeerPubkey, SimContext>,
        me: Ed25519PrivateKey,
        cf: CommitteeFor,
        share_dir: Option<PathBuf>,
        interval: u64,
        mut wiring: Wiring<R>,
    ) -> DkgActor<
        commonware_p2p::simulated::Sender<PeerPubkey, SimContext>,
        commonware_p2p::simulated::Receiver<PeerPubkey>,
        R,
    > {
        let (sender, receiver) = oracle
            .control(me.public_key())
            .register(
                fluentbase_p2p::constants::BEACON_CHANNEL,
                fluentbase_p2p::constants::BEACON_QUOTA,
            )
            .await
            .expect("register");
        if let Some(dir) = share_dir {
            wiring.share_dir = dir;
        }
        // The fixture's default bit only: a bit the test wired itself stands.
        if Arc::ptr_eq(&wiring.changed, &unreadable_bit()) {
            wiring.changed = {
                let reads = cf.clone();
                Arc::new(move |epoch: u64| {
                    let prev = epoch.checked_sub(1)?;
                    Some(reads(epoch)? != reads(prev)?)
                })
            };
        }
        DkgActor::new(
            b"FLUENT_DPOS_V1_clocktest".to_vec(),
            me,
            sender,
            receiver,
            cf,
            Arc::new(RwLock::new(BTreeMap::new())),
            Arc::new(tokio::sync::Notify::new()),
            ACTIVATION,
            interval,
            crate::beacon::metrics::BeaconMetrics::default(),
            ShareState::Plaintext,
            wiring,
        )
    }

    /// The beacon-channel ingress refusals under `reason`, drained: the debugging snapshot
    /// resets what it reads, so each call answers "since the last call".
    fn beacon_refusals(snap: &metrics_util::debugging::Snapshotter, reason: &str) -> u64 {
        use metrics_util::debugging::DebugValue;
        snap.snapshot()
            .into_vec()
            .into_iter()
            .filter_map(|(k, .., v)| {
                let key = k.key();
                let DebugValue::Counter(value) = v else {
                    return None;
                };
                (key.name() == crate::dpos::INGRESS_DROPPED_TOTAL
                    && key
                        .labels()
                        .any(|l| l.key() == "channel" && l.value() == BEACON_CHANNEL_LABEL)
                    && key
                        .labels()
                        .any(|l| l.key() == "reason" && l.value() == reason))
                .then_some(value)
            })
            .sum()
    }

    /// A confirmation from a sender with no seat costs exactly one roster read and is counted
    /// `no_seat` with nothing downstream; one naming an out-of-window epoch is refused `epoch`
    /// before any read, and a member's is admitted on its own single read.
    #[test]
    fn a_confirmation_from_a_sender_with_no_seat_costs_one_roster_read_and_is_counted() {
        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
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
                let mut rng = StdRng::seed_from_u64(0x4301);
                let keys: Vec<Ed25519PrivateKey> = (0..5)
                    .map(|_| Ed25519PrivateKey::random(&mut rng))
                    .collect();
                // keys[4] is the outsider: registered on the plane, in no committee.
                let committee = Set::from_iter_dedup(keys[..4].iter().map(|k| k.public_key()));
                let asked: Arc<std::sync::Mutex<Vec<u64>>> =
                    Arc::new(std::sync::Mutex::new(Vec::new()));
                let committee_for: CommitteeFor = {
                    let set = committee.clone();
                    let asked = asked.clone();
                    Arc::new(move |epoch: u64| {
                        asked.lock().unwrap().push(epoch);
                        Some(set.clone())
                    })
                };
                let pool = crate::beacon::dkg_agree::ConfirmPool::new(b"FLUENT_TEST_INGRESS");
                let mut wiring = Wiring::standalone();
                wiring.confirms = pool.clone();
                let mut actor = standalone_actor_at(
                    &oracle,
                    keys[0].clone(),
                    committee_for.clone(),
                    None,
                    INTERVAL,
                    wiring,
                )
                .await;
                let mut arng = StdRng::seed_from_u64(0x4302);
                // Epoch 5's first height: `now` = 5, so the actionable window is [5, 7].
                actor.on_height(INTERVAL * 5, &mut arng).await;
                assert_eq!(actor.epoch_of(actor.height_now()), 5);
                asked.lock().unwrap().clear();
                let _ = beacon_refusals(&snap, "epoch");
                let _ = beacon_refusals(&snap, "no_seat");

                let frame = |signer: &Ed25519PrivateKey, epoch: u64| -> Vec<u8> {
                    let confirm = ShareConfirm::sign(
                        pool.namespace(),
                        signer,
                        0,
                        epoch,
                        vec![(0, B256::repeat_byte(0xAA))],
                    );
                    BeaconMessage::Dkg(
                        DkgMsg {
                            ceremony_epoch: epoch,
                            body: DkgBody::Confirm(confirm),
                        }
                        .encode(),
                    )
                    .encode()
                    .to_vec()
                };

                let far = frame(&keys[1], 1_000_000_000);
                actor
                    .on_message(keys[1].public_key(), &far, &mut arng)
                    .await;
                assert!(
                    asked.lock().unwrap().is_empty(),
                    "an out-of-window epoch bought a committee record: {:?}",
                    asked.lock().unwrap()
                );
                assert_eq!(beacon_refusals(&snap, "epoch"), 1);
                assert_eq!(beacon_refusals(&snap, "no_seat"), 0);

                let outsider = frame(&keys[4], 6);
                actor
                    .on_message(keys[4].public_key(), &outsider, &mut arng)
                    .await;
                assert_eq!(
                    *asked.lock().unwrap(),
                    vec![6],
                    "a seatless sender's confirmation costs exactly the roster read"
                );
                assert!(
                    actor.pending.is_empty(),
                    "a seatless sender's frame must not occupy ceremony state"
                );
                assert_eq!(
                    beacon_refusals(&snap, "no_seat"),
                    1,
                    "the consumer's refusal must be COUNTED, not silent"
                );
                asked.lock().unwrap().clear();

                let member = frame(&keys[1], 6);
                actor
                    .on_message(keys[1].public_key(), &member, &mut arng)
                    .await;
                assert_eq!(
                    *asked.lock().unwrap(),
                    vec![6],
                    "a member's confirmation is resolved once, for the confirmation itself"
                );
                assert_eq!(beacon_refusals(&snap, "no_seat"), 0);
                assert_eq!(beacon_refusals(&snap, "epoch"), 0);
            });
        });
    }

    /// A ceremony frame from a sender with no seat is refused `no_seat` at all three places
    /// one is taken in: the start-race buffer once `committee[2]` is readable, the drain of
    /// what was buffered before it was, and the live dispatch before `handle`; the same frame
    /// from a seated dealer is handled.
    ///
    /// One predicate covers both frame kinds because dealers == players == `committee[epoch]`,
    /// so "no seat" is the same answer for an ack and a dealing.
    #[test]
    fn a_ceremony_frame_from_a_sender_with_no_seat_is_refused_at_the_buffer_the_drain_and_the_dispatch(
    ) {
        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
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
                let mut rng = StdRng::seed_from_u64(0x4303);
                let keys: Vec<Ed25519PrivateKey> = (0..5)
                    .map(|_| Ed25519PrivateKey::random(&mut rng))
                    .collect();
                let committee = Set::from_iter_dedup(keys[..4].iter().map(|k| k.public_key()));
                // `committee[2]` unreadable until `readable` flips: the start-race's own shape.
                let readable = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let committee_for: CommitteeFor = {
                    let set = committee.clone();
                    let readable = readable.clone();
                    Arc::new(move |_epoch: u64| {
                        readable
                            .load(std::sync::atomic::Ordering::SeqCst)
                            .then(|| set.clone())
                    })
                };
                let mut actor =
                    standalone_actor_cf(&oracle, keys[0].clone(), committee_for, None).await;
                let mut arng = StdRng::seed_from_u64(0x4304);
                let ns = b"FLUENT_DPOS_V1_clocktest";
                let stranger = keys[4].public_key();
                let dealer = keys[1].public_key();
                let _ = beacon_refusals(&snap, "no_seat");

                // A decodable dealing for epoch 2 from `keys[1]`: the body is never verified
                // before buffering or the seat check, so it stands in for any sender's.
                let commitment: DkgBody = {
                    let (_cer, step) =
                        DkgCeremony::start(ns, 2, committee.clone(), keys[1].clone())
                            .expect("start");
                    step.outgoing
                        .into_iter()
                        .find_map(|o| match o.msg.body {
                            b @ DkgBody::Commitment(_) => Some(b),
                            _ => None,
                        })
                        .expect("a commitment dealing")
                };
                let wire = |body: DkgBody| -> Vec<u8> {
                    BeaconMessage::Dkg(
                        DkgMsg {
                            ceremony_epoch: 2,
                            body,
                        }
                        .encode(),
                    )
                    .encode()
                    .to_vec()
                };
                let commitment_wire = wire(commitment);

                // No height tick yet: `now` = 0, so a dealing for epoch 2 is bufferable, and
                // with no record to ask both the stranger's and the dealer's are buffered.
                actor
                    .on_message(stranger.clone(), &commitment_wire, &mut arng)
                    .await;
                actor
                    .on_message(dealer.clone(), &commitment_wire, &mut arng)
                    .await;
                assert_eq!(
                    actor.pending.get(&2).map(|m| m.len()),
                    Some(2),
                    "with no record readable there is no roster to ask: both are buffered"
                );
                assert_eq!(
                    beacon_refusals(&snap, "no_seat"),
                    0,
                    "nothing is refused for a seat while the record is unreadable"
                );

                readable.store(true, std::sync::atomic::Ordering::SeqCst);
                actor
                    .on_message(stranger.clone(), &commitment_wire, &mut arng)
                    .await;
                assert_eq!(
                    beacon_refusals(&snap, "no_seat"),
                    1,
                    "a seatless dealing for a readable record must be refused at the buffer"
                );
                assert_eq!(
                    actor.pending.get(&2).map(|m| m.len()),
                    Some(2),
                    "a refused dealing must not add or replace a `pending` slot"
                );
                assert!(
                    actor.ceremonies().next().is_none(),
                    "no ceremony yet — the refusal was the buffer's, not the dispatch's"
                );

                // Epoch 1's first height: `recover(2)` starts the bootstrap ceremony and
                // drains the buffer into it.
                actor.on_height(INTERVAL, &mut arng).await;
                assert!(
                    actor.ceremony(2).is_some(),
                    "the bootstrap ceremony must have started"
                );
                assert!(!actor.pending.contains_key(&2), "the buffer was drained");
                assert_eq!(
                    beacon_refusals(&snap, "no_seat"),
                    1,
                    "the stranger's buffered dealing must be refused at the drain, once"
                );

                let ack: DkgBody = {
                    let (mut cer, _) =
                        DkgCeremony::start(ns, 2, committee.clone(), keys[2].clone())
                            .expect("start");
                    let (_cer1, step1) =
                        DkgCeremony::start(ns, 2, committee.clone(), keys[1].clone())
                            .expect("start");
                    let mut out = Vec::new();
                    for o in step1.outgoing {
                        let keep = match &o.target {
                            Target::Broadcast => true,
                            Target::Direct(pk) => *pk == keys[2].public_key(),
                        };
                        if keep {
                            out.extend(cer.handle(keys[1].public_key(), o.msg.body).outgoing);
                        }
                    }
                    out.into_iter()
                        .find_map(|o| match o.msg.body {
                            b @ DkgBody::Ack(_) => Some(b),
                            _ => None,
                        })
                        .expect("an ack")
                };
                actor
                    .on_message(stranger.clone(), &wire(ack), &mut arng)
                    .await;
                assert_eq!(beacon_refusals(&snap, "no_seat"), 1, "a stranger's ack");
                actor
                    .on_message(stranger.clone(), &commitment_wire, &mut arng)
                    .await;
                assert_eq!(
                    beacon_refusals(&snap, "no_seat"),
                    1,
                    "a stranger's commitment on a live ceremony"
                );
                actor
                    .on_message(dealer.clone(), &commitment_wire, &mut arng)
                    .await;
                assert_eq!(
                    beacon_refusals(&snap, "no_seat"),
                    0,
                    "a seated dealer's frame is consumed, never refused for a seat"
                );
            });
        });
    }

    /// A frame this actor cannot decode is refused and counted `undecodable` at each of its
    /// three decode steps: an unknown wire tag, a frame too short to carry the eight-byte
    /// epoch, and an unknown body tag behind an actionable epoch.
    ///
    /// The epoch peek runs before the body decode, so a body that would fail behind an
    /// out-of-window epoch is refused `epoch`, never `undecodable`.
    #[test]
    fn an_undecodable_frame_is_refused_and_counted_at_each_of_the_three_decode_steps() {
        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
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
                let mut rng = StdRng::seed_from_u64(0x5307);
                let keys: Vec<Ed25519PrivateKey> = (0..4)
                    .map(|_| Ed25519PrivateKey::random(&mut rng))
                    .collect();
                let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
                let mut actor = standalone_actor(&oracle, keys[0].clone(), committee, None).await;
                let mut arng = StdRng::seed_from_u64(0x5308);
                // Epoch 5's first height: `now` = 5, so the window is [5, 7].
                actor.on_height(INTERVAL * 5, &mut arng).await;
                assert_eq!(actor.epoch_of(actor.height_now()), 5);
                let _ = beacon_refusals(&snap, "undecodable");
                let _ = beacon_refusals(&snap, "epoch");
                let sender = keys[1].public_key();

                actor.on_message(sender.clone(), &[0xFF], &mut arng).await;
                assert_eq!(beacon_refusals(&snap, "undecodable"), 1, "unknown wire tag");

                let short = BeaconMessage::Dkg(Bytes::from_static(&[1, 2, 3])).encode();
                actor.on_message(sender.clone(), &short, &mut arng).await;
                assert_eq!(beacon_refusals(&snap, "undecodable"), 1, "no epoch to peek");

                let bad_body = {
                    let mut payload = 6u64.encode().to_vec();
                    payload.push(0xFF); // no such body tag
                    BeaconMessage::Dkg(Bytes::from(payload)).encode()
                };
                actor.on_message(sender.clone(), &bad_body, &mut arng).await;
                assert_eq!(beacon_refusals(&snap, "undecodable"), 1, "unknown body tag");
                assert_eq!(beacon_refusals(&snap, "epoch"), 0);

                let far_bad_body = {
                    let mut payload = 1_000_000_000u64.encode().to_vec();
                    payload.push(0xFF);
                    BeaconMessage::Dkg(Bytes::from(payload)).encode()
                };
                actor
                    .on_message(sender.clone(), &far_bad_body, &mut arng)
                    .await;
                assert_eq!(beacon_refusals(&snap, "epoch"), 1);
                assert_eq!(
                    beacon_refusals(&snap, "undecodable"),
                    0,
                    "the epoch peek runs before the body decode"
                );
                assert!(
                    actor.pending.is_empty(),
                    "nothing undecodable may occupy ceremony state"
                );
            });
        });
    }

    /// One ingress window read at its three sites, with the clock at `now` = 5:
    /// `epoch_is_actionable` admits 5..=7, `is_bufferable` takes 6 and 7 (its own
    /// `epoch > now` refuses 5), and `on_confirm` reads the window before the
    /// committee, so a confirmation for 8 costs no read.
    #[test]
    fn the_ingress_window_is_one_rule_at_three_sites() {
        assert!(within_ingress_window(5, 5) && within_ingress_window(5, 7));
        assert!(!within_ingress_window(5, 4) && !within_ingress_window(5, 8));
        assert!(
            within_ingress_window(u64::MAX, u64::MAX),
            "the top of the clock saturates rather than wraps"
        );

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
            let mut rng = StdRng::seed_from_u64(0x5303);
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            let asked: Arc<std::sync::Mutex<Vec<u64>>> =
                Arc::new(std::sync::Mutex::new(Vec::new()));
            let committee_for: CommitteeFor = {
                let set = committee.clone();
                let asked = asked.clone();
                Arc::new(move |epoch: u64| {
                    asked.lock().unwrap().push(epoch);
                    Some(set.clone())
                })
            };
            let pool = crate::beacon::dkg_agree::ConfirmPool::new(b"FLUENT_TEST_WINDOW");
            let mut wiring = Wiring::standalone();
            wiring.confirms = pool.clone();
            let mut actor = standalone_actor_at(
                &oracle,
                keys[0].clone(),
                committee_for,
                None,
                INTERVAL,
                wiring,
            )
            .await;
            // Leave the epochs ahead undecided (their `changed` bit unreadable):
            // a decided epoch never buffers.
            actor.changed = Arc::new(|epoch: u64| (epoch <= 5).then_some(false));
            let mut arng = StdRng::seed_from_u64(0x5304);
            actor.on_height(INTERVAL * 5, &mut arng).await;
            assert_eq!(actor.epoch_of(actor.height_now()), 5);

            assert!(actor.epoch_is_actionable(5) && actor.epoch_is_actionable(7));
            assert!(!actor.epoch_is_actionable(4) && !actor.epoch_is_actionable(8));

            // The start-race buffer adds its own `epoch > now` to the window, and
            // never verifies a body before buffering, so any dealing stands in.
            let dealing: DkgBody = {
                let (_cer, step) = DkgCeremony::start(
                    b"FLUENT_DPOS_V1_window",
                    6,
                    committee.clone(),
                    keys[1].clone(),
                )
                .expect("start");
                step.outgoing
                    .into_iter()
                    .find_map(|o| match o.msg.body {
                        b @ DkgBody::Commitment(_) => Some(b),
                        _ => None,
                    })
                    .expect("a commitment dealing")
            };
            assert!(
                !actor.is_bufferable(5, &dealing),
                "a dealing for `now` is past"
            );
            assert!(actor.is_bufferable(6, &dealing) && actor.is_bufferable(7, &dealing));
            assert!(!actor.is_bufferable(8, &dealing));

            asked.lock().unwrap().clear();
            let confirm = |epoch: u64| {
                ShareConfirm::sign(
                    pool.namespace(),
                    &keys[1],
                    1,
                    epoch,
                    vec![(0, B256::repeat_byte(0xAB))],
                )
            };
            actor.on_confirm(8, &keys[1].public_key(), confirm(8));
            assert!(
                asked.lock().unwrap().is_empty(),
                "a confirmation for `now + 3` bought a committee read: {:?}",
                asked.lock().unwrap()
            );
            actor.on_confirm(7, &keys[1].public_key(), confirm(7));
            assert_eq!(
                *asked.lock().unwrap(),
                vec![7],
                "a confirmation for `now + 2` is resolved once, for itself"
            );
        });
    }

    /// The resolver engine's exit stops the actor: when the engine's `LogMessage`
    /// sender goes, `run` returns instead of degrading to gossip-only. The height
    /// sink and the gossip channel stay open here, so only the resolver arm can
    /// end the loop.
    #[test]
    fn the_resolver_engines_exit_stops_the_actor() {
        let runtime = deterministic::Runner::timed(Duration::from_secs(60));
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
            let mut rng = StdRng::seed_from_u64(0x5305);
            let me = Ed25519PrivateKey::random(&mut rng);
            let committee = Set::from_iter_dedup([me.public_key()]);
            let committee_for: CommitteeFor = Arc::new(move |_e: u64| Some(committee.clone()));
            let (sender, receiver) = oracle
                .control(me.public_key())
                .register(
                    fluentbase_p2p::constants::BEACON_CHANNEL,
                    fluentbase_p2p::constants::BEACON_QUOTA,
                )
                .await
                .expect("register");
            let (resolver_tx, resolver_rx) = tokio::sync::mpsc::channel::<LogMessage>(4);
            let actor = DkgActor::new(
                b"FLUENT_DPOS_V1_clocktest".to_vec(),
                me,
                sender,
                receiver,
                committee_for,
                Arc::new(RwLock::new(BTreeMap::new())),
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                ShareState::Plaintext,
                {
                    let mut wiring = Wiring::inert(NoopResolver);
                    wiring.resolver_rx = resolver_rx;
                    wiring
                },
            );
            let (height_tx, height_rx) = tokio::sync::watch::channel(0u64);
            let run = ctx.with_label("actor").spawn(move |_| async move {
                actor.run(height_rx, StdRng::seed_from_u64(0x5306)).await
            });
            // Let the actor task come up before the resolver's sender goes.
            ctx.sleep(Duration::from_millis(50)).await;
            drop(resolver_tx);
            tokio::select! {
                res = run => res.expect("the actor task must return, not fail"),
                _ = ctx.sleep(Duration::from_secs(5)) => panic!(
                    "the actor kept running after its resolver engine exited — the \
                     gossip-only degradation is back"
                ),
            }
            // Dropped only now, so neither the height sink nor the gossip receiver
            // could have been what ended the loop.
            drop(height_tx);
        });
    }

    /// `dpos_dkg_clock_height` is the actor's clamped clock, not the last height
    /// handed in: the lower height arrives last and must not move it.
    #[test]
    fn the_dkg_clock_gauge_is_the_actors_clamp_not_the_last_height_handed_in() {
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
            let mut rng = StdRng::seed_from_u64(0x1168);
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            let clock = crate::sync_metrics::PlaneClock::default();
            let mut wiring = Wiring::standalone();
            wiring.plane_clock = clock.clone();
            let mut actor =
                standalone_actor_wired(&oracle, keys[0].clone(), committee, None, wiring).await;
            let mut arng = StdRng::seed_from_u64(0x1169);

            assert_eq!(clock.snapshot().2, -1, "no half has reported yet");

            actor.on_height(1000, &mut arng).await;
            assert_eq!(clock.snapshot().1, 1000);

            actor.on_height(303, &mut arng).await;
            assert_eq!(actor.last_height, Some(1000));
            assert_eq!(clock.snapshot().1, 1000);

            clock.record_ordering_tip(1002);
            assert_eq!(clock.snapshot().2, 2, "both halves reported ⇒ a real lag");
        });
    }

    /// The recording edge gossips only the two widths a leader cannot wait a block
    /// for: the first at or above the quorum, and the one completing the committee.
    /// Every intermediate width is superseded within the same `Reveal` burst
    /// (`ConfirmPool::record` keeps the widest a peer sees) and rides the next tick.
    #[test]
    fn only_decisive_widths_gossip_on_the_recording_edge() {
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
            // n = 7 ⇒ quorum 5: widths 6 and 7 both clear it, only 7 completes
            // the committee.
            const N: u8 = 7;
            const QUORUM: usize = 5;
            let mut rng = StdRng::seed_from_u64(0x9C);
            let keys: Vec<Ed25519PrivateKey> = (0..N)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            assert_eq!(N3f1::quorum(committee.len()) as usize, QUORUM);
            const TARGET: u64 = 6;
            let logs: Vec<(u8, B256)> = (0..N).map(|i| (i, B256::repeat_byte(0x40 + i))).collect();

            let pool = ConfirmPool::new(b"FLUENT_TEST_TRIGGER");
            let recorded: DkgLogIndex = Arc::new(RwLock::new(BTreeMap::new()));
            let mut wiring = Wiring::standalone();
            wiring.recorded_dkg_logs = recorded.clone();
            wiring.confirms = pool.clone();
            let mut actor =
                standalone_actor_wired(&oracle, keys[0].clone(), committee.clone(), None, wiring)
                    .await;
            let record = |width: usize| {
                recorded
                    .write()
                    .unwrap()
                    .insert(TARGET, logs[..width].iter().copied().collect());
            };

            record(QUORUM - 1);
            assert!(
                actor
                    .confirmations
                    .mint(ConfirmTrigger::Decisive)
                    .is_empty(),
                "below the quorum no proposal could count it"
            );

            record(QUORUM);
            assert_eq!(
                actor.confirmations.mint(ConfirmTrigger::Decisive).len(),
                1,
                "the first width at the quorum must reach peers on the recording edge"
            );

            record(QUORUM + 1);
            assert!(
                actor
                    .confirmations
                    .mint(ConfirmTrigger::Decisive)
                    .is_empty(),
                "an intermediate width is superseded within the burst"
            );
            assert_eq!(
                pool.covering(TARGET, &logs[..QUORUM + 1]).len(),
                0,
                "precondition: nothing on the wire covers the wider set yet"
            );

            // A dealer set that stalls at 6: the height tick is the backstop that
            // must carry it.
            let minted = actor.confirmations.mint(ConfirmTrigger::AnyGrowth);
            assert_eq!(minted.len(), 1, "the tick must flush the widest set");
            let DkgBody::Confirm(flushed) = &minted[0].msg.body else {
                panic!("the minted message is not a confirmation");
            };
            assert_eq!(flushed.recorded, logs[..QUORUM + 1].to_vec());
            assert_eq!(pool.covering(TARGET, &logs[..QUORUM + 1]).len(), 1);

            // The complete set does not wait for a tick: nothing can supersede it.
            record(usize::from(N));
            let minted = actor.confirmations.mint(ConfirmTrigger::Decisive);
            assert_eq!(minted.len(), 1, "a completed committee is decisive");
            let DkgBody::Confirm(full) = &minted[0].msg.body else {
                panic!("the minted message is not a confirmation");
            };
            assert_eq!(full.recorded, logs);
            assert_eq!(pool.covering(TARGET, &logs).len(), 1);
        });
    }

    /// The claimed-width rule (`previous >= confirmed.len()`) pinned through the
    /// actor's real recording path: an unchanged set mints nothing, the next genuine
    /// widening mints again, and a confirmation is taken only from the member in the
    /// seat it names.
    #[test]
    fn share_confirmations_are_minted_on_growth_and_taken_only_from_their_signer() {
        // n = 4 ⇒ quorum 3, so the two-log step below is genuinely sub-quorum.
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
                (0..4u8).map(|i| (i, B256::repeat_byte(0x80 + i))).collect();

            let pool = ConfirmPool::new(b"FLUENT_TEST_ACTOR");
            let recorded: DkgLogIndex = Arc::new(RwLock::new(BTreeMap::new()));
            let mut wiring = Wiring::standalone();
            wiring.recorded_dkg_logs = recorded.clone();
            wiring.confirms = pool.clone();
            let mut actor =
                standalone_actor_wired(&oracle, keys[0].clone(), committee.clone(), None, wiring)
                    .await;
            // Put the epoch clock inside `[now, now + 2]` for TARGET: `on_confirm`
            // refuses outside that window before resolving a committee, and driving
            // it directly keeps `on_height`'s own side effects out of the test.
            actor.last_height = Some(INTERVAL * (TARGET - 1));
            assert_eq!(actor.epoch_of(actor.height_now()), TARGET - 1);

            // Nothing recorded is nothing to confirm: a confirmation of an empty set
            // would be a member claiming a coverage it does not have.
            assert!(actor
                .confirmations
                .mint(ConfirmTrigger::AnyGrowth)
                .is_empty());

            // Below the quorum nothing is minted either: a proposal pins at least a
            // quorum, so no proposal that will ever exist could count it.
            recorded
                .write()
                .unwrap()
                .insert(TARGET, logs[..2].iter().copied().collect());
            assert!(actor
                .confirmations
                .mint(ConfirmTrigger::AnyGrowth)
                .is_empty());

            recorded
                .write()
                .unwrap()
                .insert(TARGET, logs[..3].iter().copied().collect());
            let minted = actor.confirmations.mint(ConfirmTrigger::AnyGrowth);
            assert_eq!(minted.len(), 1, "a set at the quorum must be confirmed");
            let DkgBody::Confirm(mine) = &minted[0].msg.body else {
                panic!("the minted message is not a confirmation");
            };
            assert_eq!(minted[0].msg.ceremony_epoch, TARGET);
            assert_eq!(mine.target_epoch, TARGET);
            assert_eq!(mine.idx, seat(&keys[0]));
            assert_eq!(mine.recorded, logs[..3].to_vec());
            assert!(mine.verify(pool.namespace(), &keys[0].public_key()));
            assert_eq!(pool.covering(TARGET, &logs[..3]).len(), 1);

            // Unchanged set, no re-mint: the statement changes only when a log is
            // recorded.
            assert!(actor
                .confirmations
                .mint(ConfirmTrigger::AnyGrowth)
                .is_empty());

            recorded
                .write()
                .unwrap()
                .insert(TARGET, logs.iter().copied().collect());
            assert_eq!(actor.confirmations.mint(ConfirmTrigger::AnyGrowth).len(), 1);
            assert_eq!(pool.covering(TARGET, &logs).len(), 1);

            let peer = ShareConfirm::sign(
                pool.namespace(),
                &keys[1],
                seat(&keys[1]),
                TARGET,
                logs.clone(),
            );
            actor.on_confirm(TARGET, &keys[1].public_key(), peer);
            assert_eq!(pool.covering(TARGET, &logs).len(), 2);

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

            // Neither does one whose unsigned framing disagrees with its signed
            // epoch: the framing is the half an attacker can rewrite.
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

            // Per-target scratch: the confirmations outlive the chain entering the
            // target (its agreement can still run) and go at the retention floor.
            let mut arng = StdRng::seed_from_u64(0x3D);
            actor.on_height(INTERVAL * (TARGET + 1), &mut arng).await;
            assert_eq!(
                pool.covering(TARGET, &logs).len(),
                3,
                "the confirmations went before the agreement they are counted by could"
            );
            actor
                .on_height(
                    INTERVAL * (TARGET + JOURNAL_RETENTION_EPOCHS + 1),
                    &mut arng,
                )
                .await;
            assert!(
                pool.covering(TARGET, &logs).is_empty(),
                "the confirmation pool outlived the epoch it was about"
            );
        });
    }

    /// A confirmation that beats the actor's first height tick is counted, and the
    /// window binds the moment a tick lands.
    ///
    /// `last_height` is `None` until the first tick is drained and the actor may
    /// serve a peer's frame first: reading that state as height 0 would put the
    /// window at `[0, 2]` and refuse every confirmation on a chain past epoch 2 —
    /// permanently, since `Confirmations::mint` is edge-triggered on width growth
    /// and never re-issues a full-width statement.
    #[test]
    fn a_confirmation_that_beats_the_first_height_tick_is_counted() {
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
            let mut rng = StdRng::seed_from_u64(0x4501);
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
            // Far outside `[0, 2]`: the epoch a running chain would be in when this
            // node's actor comes up.
            const TARGET: u64 = 40;
            let logs: Vec<(u8, B256)> =
                (0..4u8).map(|i| (i, B256::repeat_byte(0x90 + i))).collect();

            let pool = ConfirmPool::new(b"FLUENT_TEST_FIRST_TICK");
            let mut wiring = Wiring::standalone();
            wiring.confirms = pool.clone();
            let mut actor =
                standalone_actor_wired(&oracle, keys[0].clone(), committee.clone(), None, wiring)
                    .await;
            assert_eq!(
                actor.last_height, None,
                "the clock must not have started — that IS the case under test"
            );

            let confirm = |k: &Ed25519PrivateKey, epoch: u64| {
                ShareConfirm::sign(pool.namespace(), k, seat(k), epoch, logs.clone())
            };
            actor.on_confirm(TARGET, &keys[1].public_key(), confirm(&keys[1], TARGET));
            assert_eq!(
                pool.covering(TARGET, &logs).len(),
                1,
                "a confirmation that arrived before the first height tick was dropped, \
                 and nothing re-sends it"
            );

            // One tick and the window binds: epoch 5's first height, with TARGET 40
            // far outside `[5, 7]`.
            let mut arng = StdRng::seed_from_u64(0x4502);
            actor.on_height(INTERVAL * 5, &mut arng).await;
            assert_eq!(actor.last_height, Some(INTERVAL * 5));
            actor.on_confirm(TARGET, &keys[2].public_key(), confirm(&keys[2], TARGET));
            assert_eq!(
                pool.covering(TARGET, &logs).len(),
                1,
                "with a clock, `[now, now+2]` must still refuse an out-of-window epoch"
            );

            // An epoch inside the window still lands, so the refusal above is the
            // window and not the tick.
            let in_window = 6u64;
            actor.on_confirm(
                in_window,
                &keys[2].public_key(),
                confirm(&keys[2], in_window),
            );
            assert_eq!(
                pool.covering(in_window, &logs).len(),
                1,
                "an in-window confirmation must still be counted after the first tick"
            );
        });
    }

    /// The clock the launcher prunes on, as this actor publishes it:
    /// `send_if_modified` publishes on the epoch edge only, so a healthy epoch costs
    /// the launcher one wake-up.
    #[test]
    fn the_agreement_clock_moves_on_the_epoch_edge_and_never_per_tick() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let mut rng = StdRng::seed_from_u64(0x5C);
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;

            let (clock_tx, mut clock_rx) = tokio::sync::watch::channel(0u64);
            let mut wiring = Wiring::standalone();
            wiring.epoch_clock = clock_tx;
            let mut actor =
                standalone_actor_wired(&oracle, keys[0].clone(), committee, None, wiring).await;
            let mut arng = StdRng::seed_from_u64(0x5D);

            actor.on_height(INTERVAL * 2, &mut arng).await;
            assert!(clock_rx.has_changed().expect("sender alive"));
            assert_eq!(*clock_rx.borrow_and_update(), 2);
            actor.on_height(INTERVAL * 2 + 1, &mut arng).await;
            actor.on_height(INTERVAL * 2 + 2, &mut arng).await;
            assert!(
                !clock_rx.has_changed().expect("sender alive"),
                "a tick inside the epoch must not wake the launcher"
            );
            actor.on_height(INTERVAL * 3, &mut arng).await;
            assert!(clock_rx.has_changed().expect("sender alive"));
            assert_eq!(*clock_rx.borrow_and_update(), 3);
            actor.on_height(INTERVAL * 3 + 1, &mut arng).await;
            assert!(
                !clock_rx.has_changed().expect("sender alive"),
                "a tick inside the epoch must not wake the launcher"
            );
        });
    }

    /// The agreement plane keeps agreeing after the chain has entered the target
    /// epoch, so the ceremony it derives against must outlive that boundary: a swept
    /// ceremony turns `derive_pinned` into a permanent `Unavailable`, which parks
    /// every `verify` and leaves `build_proposal` with nothing to pin. The instance
    /// is aborted only once this actor's epoch clock enters `target + 1`.
    #[test]
    fn a_ceremony_outlives_the_chain_entering_its_epoch() {
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
            let incoming = Set::from_iter_dedup(keys[..6].iter().map(|k| k.public_key()));
            let outgoing = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, outgoing.clone()).await;
            let committee_for: CommitteeFor = Arc::new(move |e| {
                Some(if e == TARGET - 1 {
                    outgoing.clone()
                } else {
                    incoming.clone()
                })
            });

            let (_pinned_tx, pinned_rx) = tokio::sync::mpsc::channel(4);
            let mut wiring = Wiring::standalone();
            wiring.pinned_rx = pinned_rx;
            let mut actor = standalone_actor_at(
                &oracle,
                keys[0].clone(),
                committee_for,
                None,
                INTERVAL,
                wiring,
            )
            .await;

            // The clock is in `TARGET − 1`: the epoch this actor deals for is `now + 1`.
            actor.last_height = Some(INTERVAL * (TARGET - 1));
            let mut out = Vec::new();
            actor.decide(TARGET, &mut out).await;
            assert!(actor.ceremony(TARGET).is_some(), "no ceremony to keep");

            let ask = |epoch: u64| {
                let (response, _reply) = tokio::sync::oneshot::channel();
                PinnedRequest {
                    epoch,
                    pinned: (0..4u8).map(|i| (i, B256::repeat_byte(0x60 + i))).collect(),
                    response,
                }
            };
            let mut arng = StdRng::seed_from_u64(0x5B);

            actor.on_height(INTERVAL * (TARGET + 1), &mut arng).await;
            assert!(
                actor.ceremony(TARGET).is_some(),
                "the ceremony was swept while its agreement could still be running"
            );
            assert!(
                matches!(
                    actor.derive_pinned(&ask(TARGET), &mut arng),
                    PinnedDerive::Missing(_)
                ),
                "the plane must still answer for a target the chain has passed"
            );

            actor
                .on_height(
                    INTERVAL * (TARGET + crate::beacon::JOURNAL_RETENTION_EPOCHS + 1),
                    &mut arng,
                )
                .await;
            assert!(
                actor.ceremony(TARGET).is_none(),
                "the retention window outlived the epoch it was for"
            );
            assert!(matches!(
                actor.derive_pinned(&ask(TARGET), &mut arng),
                PinnedDerive::Unavailable
            ));
        });
    }

    /// A shrink must start a ceremony: the fixture's `changed` bit is the contract
    /// rule `committee[e] != committee[e − 1]`, and here the two rosters differ only
    /// by removal.
    #[test]
    fn a_shrink_still_deals() {
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
            let all_keys: Vec<Ed25519PrivateKey> = (0..10)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let me = all_keys[0].clone();
            let outgoing = Set::from_iter_dedup(all_keys.iter().map(|k| k.public_key()));
            let incoming = Set::from_iter_dedup(all_keys[..8].iter().map(|k| k.public_key()));
            oracle.manager().track(0, outgoing.clone()).await;

            const TARGET: u64 = 5; // past the bootstrap epoch, which always mints
            let committee_for: CommitteeFor = Arc::new(move |e| {
                Some(if e == TARGET - 1 {
                    outgoing.clone()
                } else {
                    incoming.clone()
                })
            });

            let mut actor = standalone_actor_cf(&oracle, me, committee_for, None).await;
            // The clock is in `TARGET − 1`: the epoch this actor deals for is `now + 1`.
            actor.last_height = Some(INTERVAL * (TARGET - 1));
            let mut out = Vec::new();
            actor.decide(TARGET, &mut out).await;
            assert!(
                actor.ceremony(TARGET).is_some(),
                "a shrink MUST deal: committee[t−1] = 10 ≠ committee[t] = 8"
            );
        });
    }

    /// Carry-forward control: a genuine no-change epoch
    /// (`committee[t−1] == committee[t]`) starts no ceremony — the key carries
    /// forward.
    #[test]
    fn no_change_carries_forward() {
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

            const TARGET: u64 = 5; // past the bootstrap epoch, which always mints
            let same = set.clone();
            let committee_for: CommitteeFor = Arc::new(move |_e| Some(same.clone()));

            let mut actor = standalone_actor_cf(&oracle, me, committee_for, None).await;
            let mut out = Vec::new();
            actor.decide(TARGET, &mut out).await;
            assert!(
                actor.ceremony(TARGET).is_none(),
                "no-change (committee[t−1] == committee[t]) ⇒ carry-forward, no ceremony"
            );
        });
    }

    /// The epoch slots — every per-epoch fact, the terminal verdicts included — are
    /// bounded by the retention window, except that a verdict for a target the actor
    /// can still reach must survive the sweep: dropping it would re-open a settled
    /// verdict and re-run its heal.
    #[tokio::test]
    async fn the_epoch_slots_ride_the_retention_window() {
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
            let mut rng = StdRng::seed_from_u64(0x9D);
            let keys: Vec<Ed25519PrivateKey> = (0..6)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let me = keys[0].clone();
            let set = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, set.clone()).await;

            let same = set.clone();
            let committee_for: CommitteeFor = Arc::new(move |_e| Some(same.clone()));
            let mut actor = standalone_actor_cf(&oracle, me, committee_for, None).await;
            let mut arng = StdRng::seed_from_u64(0x9E);

            // The walk must reach past the retention window, or nothing is below the
            // floor and the pruning assertions hold vacuously — so its length is
            // derived from the window.
            let last: u64 = crate::beacon::JOURNAL_RETENTION_EPOCHS + 3;
            for e in 1..=last {
                actor.on_height(INTERVAL * e, &mut arng).await;
            }

            let now = actor.epoch_of(INTERVAL * last);
            // The only target `recover` can still be asked to deal for.
            let reachable = now + 1;
            assert_eq!(
                actor.phase(reachable),
                Some("key_only"),
                "the walk did not decide its target — this test proves nothing about pruning"
            );
            assert!(actor.epochs.len() > JOURNAL_RETENTION_EPOCHS as usize);

            // Seed the load-bearing phase by hand — a standalone actor with no share
            // dir cannot produce an `Unrecoverable` — on both sides of the floor, the
            // aged-out one derived from the window.
            let aged_out = now - JOURNAL_RETENTION_EPOCHS - 1;
            actor.epochs.remove(&reachable);
            actor.enter(reachable, EpochState::Unrecoverable { key: None });
            actor.enter(aged_out, EpochState::Unrecoverable { key: None });
            assert_eq!(actor.metrics.stalled_gauge(StallReason::Unrecoverable), 2);

            // One more tick: the sweep runs at the same `now` that feeds `recover`.
            actor.on_height(INTERVAL * last, &mut arng).await;
            assert_eq!(
                actor.metrics.stalled_gauge(StallReason::Unrecoverable),
                1,
                "the aged-out slot's latch steps the gauge down with it"
            );

            // SAFETY: a slot the actor can still reach as a target must survive. Dropping
            // it would re-open a settled verdict and re-run its heal from scratch.
            assert_eq!(
                actor.phase(reachable),
                Some("unrecoverable"),
                "the sweep dropped the verdict for a target recover can still reach — \
                 the epoch would be re-decided and its heal re-run"
            );

            let floor = now.saturating_sub(JOURNAL_RETENTION_EPOCHS);
            assert!(
                actor.phase(aged_out).is_none(),
                "an aged-out slot was retained — the map grows for the life of the process"
            );
            for e in actor.epochs.keys() {
                assert!(
                    *e >= floor,
                    "epoch {e} is below the retention floor {floor} but was kept"
                );
            }
        });
    }

    /// A restarted actor serves epoch 2's logs from the on-disk journal — one cold parse
    /// for the epoch however many serves land — and a first `on_height` past the
    /// retention window deletes the journal so the serve then returns `None`.
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

            let log_of = |i: usize| -> DkgLogKey {
                let (pk, signed) = logs
                    .iter()
                    .find(|(pk, _)| *pk == keys[i].public_key())
                    .expect("sealed");
                DkgLogKey {
                    epoch: 2,
                    dealer: pk.clone(),
                    hash: log_hash(signed),
                }
            };
            let peer_key = log_of(1);
            assert!(
                actor.serve_log(&peer_key).is_some(),
                "a restarted node serves a finalized epoch's log from the journal-backed cache (R1)"
            );
            assert_eq!(
                COLD_PARSE_COUNT.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "the first serve cold-parses the journal exactly once"
            );

            let peer_key2 = log_of(2);
            assert!(actor.serve_log(&peer_key2).is_some());
            assert!(actor.serve_log(&peer_key).is_some());
            assert_eq!(
                COLD_PARSE_COUNT.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "repeated serves within a cold-loaded epoch add zero parses (one-per-epoch bound)"
            );

            // Past the retention window the first-tick reconcile deletes the aged-out
            // journal; inside it the journal is kept for the recompute heal.
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

    /// M ≫ K serve calls spread across K cold epochs cold-parse the journal exactly once
    /// per epoch.
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
            // The actor is never ticked, so the first-tick reconcile never runs and the
            // journals stay on disk for the serves below.
            const K_EPOCHS: u64 = 3;
            let mut peer0_keys = Vec::new();
            for e in 2..2 + K_EPOCHS {
                let logs = mint_committee_logs_at(&keys, &committee, e);
                for (_pk, signed) in &logs {
                    let rec = JournalRecord::PeerLog(Box::new(signed.clone()));
                    share_state::append_journal(&dir, e, &rec, &ShareState::Plaintext)
                        .expect("append");
                }
                let (pk, signed) = logs
                    .iter()
                    .find(|(pk, _)| *pk == keys[1].public_key())
                    .expect("keys[1] sealed");
                peer0_keys.push(DkgLogKey {
                    epoch: e,
                    dealer: pk.clone(),
                    hash: log_hash(signed),
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

    /// Mid-window restart of node 0 over a real wired resolver (n=7, f=2, dealer quorum
    /// 5): the victim resumes with 4 logs and, since the gossip reveals already fired
    /// one-shot, only the resolver can bring it back to quorum. Returns whether the
    /// restarted victim seeded a share.
    ///
    /// `corrupt_own_seal` flips a byte in the victim's `OwnSeal` frame so it resumes
    /// `me ∉ recorded` and must re-fetch its own log; `use_resolver` spawns the victim
    /// with the real engine or with `NoopResolver`, the contrast that must not seed.
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
        // Full mesh among holders 1..6, so each holds the others' logs and can serve
        // them. Victim 0 links only to {1,2,3} and so records 4 < quorum 5; the reveals
        // of {4,5,6} never reach it and do not re-arrive (one-shot).
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
        // The holders share one `Certified`, so the restarted victim is handed the set
        // their stub certified and fetches the bodies it names. Node 0's pre-restart
        // instance gets its own map, whose stub never reaches quorum (4 < 5) — no delivery.
        let certified = Certified::default();
        let mut sinks = Vec::new();
        for (i, k) in keys.iter().enumerate() {
            let store = Arc::new(RwLock::new(BTreeMap::new()));
            let dir_i = if i == 0 { Some(dir.clone()) } else { None };
            let certified_i = if i == 0 {
                Certified::default()
            } else {
                certified.clone()
            };
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
                    certified_i,
                )
                .await,
            );
        }

        // Past the seal, so the victim has journaled its partial progress before the restart.
        for h in 0..=RESTART_AT {
            for s in &sinks {
                s.send_replace(h);
            }
            ctx.sleep(Duration::from_millis(50)).await;
        }

        drop(sinks.remove(0));
        if corrupt_own_seal {
            corrupt_own_seal_record(&dir, 2);
        }
        // The resolver needs links to the holders of the missing logs; gossip will not
        // re-deliver them.
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
                certified.clone(),
            )
            .await
        } else {
            spawn_dealer_at_sender(
                &ctx,
                &oracle,
                keys[0].clone(),
                committee.clone(),
                restarted_store.clone(),
                Arc::new(tokio::sync::Notify::new()),
                INTERVAL,
                Some(dir.clone()),
                99,
                Arc::new(RwLock::new(BTreeMap::new())),
            )
            .await
        };
        sinks.insert(0, new_sink);

        // The resolver fetch is multi-round (request → `initial` delay → serve →
        // deliver), so the last in-window frontier (still pre-boundary) is re-ticked:
        // each `on_height` re-issues the missing keys and re-runs `drive_finalization`.
        for h in (RESTART_AT + 1)..=FEED_TO {
            for s in &sinks {
                s.send_replace(h);
            }
            ctx.sleep(Duration::from_millis(100)).await;
        }
        for _ in 0..20 {
            for s in &sinks {
                s.send_replace(FEED_TO);
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

    /// Flips a byte in the signature tail of the victim's `OwnSeal` record, leaving the
    /// framing and length prefix intact so later records still load: the record decodes
    /// but its `check` fails, and the node resumes `me ∉ recorded` holding its peer logs.
    /// Frame: `u32_be(len) ‖ tag(1=plaintext) ‖ rec_tag(1) ‖ signed-log`.
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
            let rec_tag = bytes[body_start + 1];
            if rec_tag == REC_OWN_SEAL {
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

    /// The restarted victim seeds only through the resolver fetch (resume gives 4 <
    /// quorum 5); with `NoopResolver` it must not seed.
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

    /// A victim whose `OwnSeal` frame is corrupted resumes `me ∉ recorded`: its own log
    /// is fetched like any other missing pinned body, and it still finalizes.
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

    /// Mints every committee member's sealed log at `epoch`: a log only `check`s under
    /// its own epoch's `Info`. Deterministic in `keys` — the dealer polynomial is seeded
    /// from the key and epoch (`ceremony::dealer_seed_rng`).
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

    /// Node 0's actor over a live bootstrap-epoch ceremony that has ingested every
    /// committee log, the `victims` logs ingested while the share dir was a file: those
    /// appends failed, so the bytes are held with nothing backing them across a restart.
    /// A caller that wants the retry leg repairs `actor.share_dir` first.
    struct NondurableLogs {
        actor: DkgActor<
            commonware_p2p::simulated::Sender<PeerPubkey, SimContext>,
            commonware_p2p::simulated::Receiver<PeerPubkey>,
            NoopResolver,
        >,
        committee: Set<PeerPubkey>,
        victims: BTreeSet<PeerPubkey>,
        /// The victims' logs by `(dealer, hash)` — the shape `nondurable_logs` names.
        victim_ids: BTreeSet<LogId>,
        recorded: DkgLogIndex,
        pool: ConfirmPool,
        me: PeerPubkey,
        good_dir: PathBuf,
    }

    impl NondurableLogs {
        /// The one dealer whose write failed, for the cases that break exactly one.
        fn sole_victim(&self) -> PeerPubkey {
            let mut it = self.victims.iter();
            let only = it.next().expect("a victim").clone();
            assert!(
                it.next().is_none(),
                "this fixture broke more than one write"
            );
            only
        }

        /// Committee positions (the `idx` the index and every `ShareConfirm` are keyed
        /// by) whose journal record is durable.
        fn durable_seats(&self) -> Vec<u8> {
            self.committee
                .iter()
                .enumerate()
                .filter(|(_, pk)| !self.victims.contains(*pk))
                .map(|(i, _)| i as u8)
                .collect()
        }

        fn my_seat(&self) -> u8 {
            self.committee
                .iter()
                .position(|pk| *pk == self.me)
                .expect("a member") as u8
        }

        /// `(seat, keccak256(log))` for `seats`, read from the ceremony rather than the
        /// index under test: the ceremony holds every log, claimable or not.
        fn seat_hashes(&self, seats: &[u8]) -> Vec<(u8, B256)> {
            let c = self
                .actor
                .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                .expect("ceremony");
            seats
                .iter()
                .map(|i| {
                    let pk = self.committee.iter().nth(*i as usize).expect("seat");
                    (*i, c.signed_log_hash(pk).expect("recorded log"))
                })
                .collect()
        }

        /// Dealers whose `PeerLog` the on-disk journal holds, read as every other
        /// consumer reads it: `check`ed against the epoch's `Info`.
        fn journaled_dealers(&self) -> BTreeSet<PeerPubkey> {
            let max = NonZeroU32::new(fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as u32)
                .expect("MAX_COMMITTEE_SIZE > 0");
            let JournalLoad::Present(records) = share_state::load_journal(
                &self.good_dir,
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                &ShareState::Plaintext,
                max,
            ) else {
                return BTreeSet::new();
            };
            let info = crate::beacon::ceremony::info_for(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                self.committee.clone(),
            )
            .expect("info");
            records
                .into_iter()
                .filter_map(|r| match r {
                    JournalRecord::PeerLog(signed) => signed.check(&info).map(|(pk, _)| pk),
                    _ => None,
                })
                .collect()
        }
    }

    async fn nondurable_logs(ctx: SimContext, victims_at: &[usize]) -> NondurableLogs {
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
        let mut rng = StdRng::seed_from_u64(0xD1);
        let keys: Vec<Ed25519PrivateKey> = (0..4)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let logs = mint_committee_logs_at(&keys, &committee, DETERMINISTIC_BOOTSTRAP_EPOCH);
        let victims: BTreeSet<PeerPubkey> = victims_at.iter().map(|i| logs[*i].0.clone()).collect();
        let victim_ids: BTreeSet<LogId> = victims_at
            .iter()
            .map(|i| (logs[*i].0.clone(), log_hash(&logs[*i].1)))
            .collect();

        let good_dir = fresh_share_dir("nondurable-good");
        let bad_dir = fresh_share_dir("nondurable-bad");
        std::fs::write(&bad_dir, b"not a dir").expect("write file");

        let recorded: DkgLogIndex = Arc::new(RwLock::new(BTreeMap::new()));
        let pool = ConfirmPool::new(b"FLUENT_TEST_NONDURABLE");
        let mut wiring = Wiring::standalone();
        wiring.recorded_dkg_logs = recorded.clone();
        wiring.confirms = pool.clone();
        let mut actor = standalone_actor_wired(
            &oracle,
            keys[0].clone(),
            committee.clone(),
            Some(good_dir.clone()),
            wiring,
        )
        .await;
        let (cer, _step) = DkgCeremony::start(
            b"FLUENT_DPOS_V1_clocktest",
            DETERMINISTIC_BOOTSTRAP_EPOCH,
            committee.clone(),
            keys[0].clone(),
        )
        .expect("start");
        actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, cer);

        // Victims last: a later durable ingest runs the retry leg against the good dir
        // and would land the victim's record before the assert.
        let order = (0..logs.len())
            .filter(|i| !victims_at.contains(i))
            .chain(victims_at.iter().copied());
        for i in order {
            if victims_at.contains(&i) {
                actor.share_dir = bad_dir.clone();
            }
            let (dealer, signed) = &logs[i];
            let key = DkgLogKey {
                epoch: DETERMINISTIC_BOOTSTRAP_EPOCH,
                dealer: dealer.clone(),
                hash: log_hash(signed),
            };
            assert!(
                actor.ingest_log(&key, signed.encode(), &mut rng).await,
                "every minted log is valid for its own dealer"
            );
        }
        NondurableLogs {
            actor,
            committee,
            victims,
            victim_ids,
            recorded,
            pool,
            me: keys[0].public_key(),
            good_dir,
        }
    }

    /// A dealer log this node holds in memory but could not journal is not claimed: it is
    /// absent from the shared `recorded_dkg_logs` index the agreement plane proposes from
    /// and from the `ShareConfirm` minted off it, while every dealer whose record landed
    /// stays claimed — a restart would lose the held bytes.
    #[test]
    fn a_nondurable_dealer_log_is_neither_indexed_nor_confirmed() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let mut f = nondurable_logs(ctx, &[1]).await;
            let victim = f.sole_victim();
            let victim_seat = f
                .committee
                .iter()
                .position(|pk| *pk == victim)
                .expect("the victim sits in the committee") as u8;

            assert_eq!(
                f.actor.nondurable_logs.get(&DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some(&f.victim_ids),
                "the fault landed on EXACTLY the victim's log: one failed append, named"
            );
            assert!(
                !f.journaled_dealers().contains(&victim),
                "and it really is absent from the journal a restart would replay"
            );
            assert_eq!(
                f.actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .expect("ceremony")
                    .recorded_log_count(),
                4,
                "the ceremony still HOLDS all four logs in memory — the gate is about \
                 what may be claimed, not about what is recorded"
            );

            let indexed: Vec<u8> = f.recorded.read().unwrap()[&DETERMINISTIC_BOOTSTRAP_EPOCH]
                .keys()
                .copied()
                .collect();
            assert_eq!(indexed, f.durable_seats());
            assert!(!indexed.contains(&victim_seat));

            let minted = f.actor.confirmations.mint(ConfirmTrigger::AnyGrowth);
            assert_eq!(minted.len(), 1, "three durable logs is the quorum at n=4");
            let DkgBody::Confirm(confirm) = &minted[0].msg.body else {
                panic!("the minted message is not a confirmation");
            };
            let confirmed: Vec<u8> = confirm.recorded.iter().map(|(i, _)| *i).collect();
            assert_eq!(confirmed, f.durable_seats());

            // `covers` tests the signed bodies, so this answers whether the node signed
            // for a log it cannot back, independently of the index read above.
            let all_four = f.seat_hashes(&[0, 1, 2, 3]);
            let durable = f.seat_hashes(&f.durable_seats());
            assert!(
                f.pool
                    .covering(DETERMINISTIC_BOOTSTRAP_EPOCH, &all_four)
                    .is_empty(),
                "no confirmation on the wire covers a set containing the unbacked log"
            );
            assert!(
                f.pool
                    .covering(DETERMINISTIC_BOOTSTRAP_EPOCH, &durable)
                    .iter()
                    .any(|c| c.idx == f.my_seat()),
                "while this node's own confirmation does cover the three it can back"
            );
        });
    }

    /// With the share dir unwritable for every ingest the node backs nothing, so it
    /// claims nothing: the published index for the target stays empty rather than
    /// carrying a set a restart would lose. Every log is still held in ceremony memory.
    #[test]
    fn a_node_that_can_journal_nothing_claims_nothing() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let mut f = nondurable_logs(ctx, &[0, 1, 2, 3]).await;

            assert_eq!(
                f.actor.nondurable_logs[&DETERMINISTIC_BOOTSTRAP_EPOCH].len(),
                4,
                "every append failed and every one is named"
            );
            assert!(f.journaled_dealers().is_empty());
            assert_eq!(
                f.actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .expect("ceremony")
                    .recorded_log_count(),
                4,
                "the bytes are all still held — only the CLAIM is withheld"
            );
            assert!(
                f.durable_seats().is_empty(),
                "precondition for the assert below"
            );

            let published = f
                .recorded
                .read()
                .unwrap()
                .get(&DETERMINISTIC_BOOTSTRAP_EPOCH)
                .is_none_or(BTreeMap::is_empty);
            assert!(published, "an index entry would be a claim nothing backs");
            assert!(
                f.actor
                    .confirmations
                    .mint(ConfirmTrigger::AnyGrowth)
                    .is_empty(),
                "and a node claiming nothing signs no confirmation"
            );
        });
    }

    /// `publish_recorded_logs` re-attempts every failed write before deciding what may be
    /// claimed, so the first publish after the dir is writable lands the record and
    /// widens the claim from memory, with no re-fetch.
    #[test]
    fn a_retried_journal_write_makes_its_log_claimable_again() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let mut f = nondurable_logs(ctx, &[1]).await;
            let victim = f.sole_victim();
            let seats: Vec<u8> = (0..4).collect();

            f.actor.share_dir = f.good_dir.clone();
            f.actor.publish_recorded_logs();

            assert!(
                f.actor.nondurable_logs.is_empty(),
                "the retry cleared the entry rather than leaving an empty set behind"
            );
            assert!(
                f.journaled_dealers().contains(&victim),
                "the record the first attempt lost is now on disk"
            );
            let indexed: Vec<u8> = f.recorded.read().unwrap()[&DETERMINISTIC_BOOTSTRAP_EPOCH]
                .keys()
                .copied()
                .collect();
            assert_eq!(indexed, seats, "and the claim widened on the same edge");

            let minted = f.actor.confirmations.mint(ConfirmTrigger::AnyGrowth);
            let DkgBody::Confirm(confirm) = &minted[0].msg.body else {
                panic!("the minted message is not a confirmation");
            };
            let confirmed: Vec<u8> = confirm.recorded.iter().map(|(i, _)| *i).collect();
            assert_eq!(confirmed, seats);
        });
    }

    /// A recovery delivery widens this node's confirmation with no height tick behind
    /// it: the entry bar counts confirmations covering the proposed set, and a halted
    /// chain has no next tick. The delivery path mints on `AnyGrowth`, not `Decisive`.
    ///
    /// No `on_height` is called anywhere in this test: the tick is the mechanism under
    /// test, by its absence.
    #[test]
    fn a_resolver_delivery_mints_the_wider_confirmation_with_no_height_tick() {
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
            let mut rng = StdRng::seed_from_u64(0xD3);
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            const TARGET: u64 = DETERMINISTIC_BOOTSTRAP_EPOCH;
            let quorum = N3f1::quorum(committee.len()) as usize;
            assert_eq!(quorum, 3, "n = 4 ⇒ the 4th log is the first width above it");
            let logs = mint_committee_logs_at(&keys, &committee, TARGET);

            let recorded: DkgLogIndex = Arc::new(RwLock::new(BTreeMap::new()));
            let pool = ConfirmPool::new(b"FLUENT_TEST_MINT_EDGE");
            let mut wiring = Wiring::standalone();
            wiring.recorded_dkg_logs = recorded.clone();
            wiring.confirms = pool.clone();
            let mut actor =
                standalone_actor_wired(&oracle, keys[0].clone(), committee.clone(), None, wiring)
                    .await;
            let (cer, _step) = DkgCeremony::start(
                b"FLUENT_DPOS_V1_clocktest",
                TARGET,
                committee.clone(),
                keys[0].clone(),
            )
            .expect("start");
            actor.insert_ceremony(TARGET, cer);

            for (dealer, signed) in logs.iter().take(quorum) {
                let key = DkgLogKey {
                    epoch: TARGET,
                    dealer: dealer.clone(),
                    hash: log_hash(signed),
                };
                assert!(actor.ingest_log(&key, signed.encode(), &mut rng).await);
            }
            assert_eq!(recorded.read().unwrap()[&TARGET].len(), quorum);

            assert_eq!(actor.confirmations.mint(ConfirmTrigger::AnyGrowth).len(), 1);
            assert_eq!(actor.confirmations.claimed_width(TARGET), Some(quorum));
            let narrow: Vec<(u8, B256)> = {
                let c = actor.ceremony(TARGET).expect("ceremony");
                committee
                    .iter()
                    .enumerate()
                    .filter_map(|(i, pk)| c.signed_log_hash(pk).map(|h| (i as u8, h)))
                    .collect()
            };
            assert_eq!(narrow.len(), quorum);
            assert_eq!(pool.covering(TARGET, &narrow).len(), 1);

            // The remaining log arrives by recovery, not gossip, with no tick behind it.
            let (dealer, signed) = &logs[quorum];
            let (response, verdict) = tokio::sync::oneshot::channel();
            actor
                .on_resolver_message(
                    LogMessage::Deliver {
                        key: DkgLogKey {
                            epoch: TARGET,
                            dealer: dealer.clone(),
                            hash: log_hash(signed),
                        },
                        value: signed.encode(),
                        response,
                    },
                    &mut rng,
                )
                .await;
            assert!(verdict.await.expect("a verdict"), "the log is valid");

            let wide: Vec<(u8, B256)> = {
                let c = actor.ceremony(TARGET).expect("ceremony");
                committee
                    .iter()
                    .enumerate()
                    .filter_map(|(i, pk)| c.signed_log_hash(pk).map(|h| (i as u8, h)))
                    .collect()
            };
            assert_eq!(
                recorded.read().unwrap()[&TARGET].len(),
                quorum + 1,
                "the delivery widened the index"
            );
            assert_eq!(wide.len(), quorum + 1);
            let me_seat = committee
                .iter()
                .position(|pk| *pk == keys[0].public_key())
                .expect("a member") as u8;
            assert!(
                pool.covering(TARGET, &wide)
                    .iter()
                    .any(|c| c.idx == me_seat),
                "and put this node's confirmation AT the new width on the wire — \
                 without it the widening is a set no leader can count"
            );
        });
    }

    /// A 4-party committee[2] DKG driven at the ceremony level, returning node-0's journal
    /// from the crash state it models: every peer sealed and revealed, so their `PeerLog`s
    /// are recorded, but node-0 never called `seal_dealings` — it has no `OwnSeal`, and no
    /// peer ever received its log.
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

    /// A pre-deadline resume reconstructs a live dealer, so the finalize gate stays shut
    /// until the seal at the deadline: `on_height` seals it, records node-0's own log, and
    /// the epoch finalizes with that log in its set.
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
            // Drop one peer log so only node-0's own freshly-sealed log can complete the quorum.
            let idx = journal
                .iter()
                .position(|r| matches!(r, JournalRecord::PeerLog(_)))
                .expect("a peer log to drop");
            journal.remove(idx);
            oracle.manager().track(0, committee.clone()).await;
            let me0 = key0.public_key();

            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                true,
                &BTreeMap::new(),
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
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);

            // A `Dealing` epoch is never a finalize input: the gate holds until the seal.
            let mut arng = StdRng::seed_from_u64(9);
            actor.drive_finalization(&mut arng);
            assert!(
                store.read().map(|s| s.is_empty()).unwrap_or(false),
                "a reconstructed live-dealer ceremony does not finalize before its seal"
            );
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("dealing"));

            actor.on_height(SEAL_DEADLINE, &mut arng).await;
            let c = actor
                .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                .expect("ceremony still present (shorthanded — not yet finalized)");
            assert!(
                c.dealing_closed(),
                "on_height step 1 sealed the reconstructed dealer"
            );
            assert!(
                c.own_log_recorded(&me0),
                "the seal recorded node-0's OWN freshly-sealed log (finalizes WITH its own log)"
            );

            actor.pin_recorded_as_agreed(DETERMINISTIC_BOOTSTRAP_EPOCH, &mut arng);
            actor.on_height(SEAL_DEADLINE + 1, &mut arng).await;
            assert!(
                store
                    .read()
                    .map(|s| s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
                    .unwrap_or(false),
                "the reconstructed dealer seals via on_height step 1 and finalizes"
            );
        });
    }

    /// A share whose disk write fails is not adopted — the node stays verify-only for the
    /// epoch: `dpos_dkg_share_persist_failed_total` moves, `dkg_ceremony_ok_total` does not,
    /// no share reaches the store, and the epoch re-selects the journal as `Acquiring(Logs)`
    /// with `Stalled{PersistFailed}`.
    #[test]
    fn a_share_whose_persist_fails_is_refused_and_the_node_stays_verify_only() {
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
            let idx = journal
                .iter()
                .position(|r| matches!(r, JournalRecord::PeerLog(_)))
                .expect("a peer log to drop");
            journal.remove(idx);
            oracle.manager().track(0, committee.clone()).await;

            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                true,
                &BTreeMap::new(),
            )
            .expect("pre-deadline resume");

            // A share dir that is a file: `persist`'s `create_dir_all` fails and nothing else does.
            let bad_dir = fresh_share_dir("persist-refused");
            std::fs::create_dir_all(bad_dir.parent().expect("a parent")).expect("parent");
            std::fs::write(&bad_dir, b"not a dir").expect("write file");

            let store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut actor = standalone_actor(&oracle, key0, committee, Some(bad_dir.clone())).await;
            actor.store = store.clone();
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);

            let mut arng = StdRng::seed_from_u64(9);
            actor.on_height(SEAL_DEADLINE, &mut arng).await;
            actor.pin_recorded_as_agreed(DETERMINISTIC_BOOTSTRAP_EPOCH, &mut arng);
            actor.on_height(SEAL_DEADLINE + 1, &mut arng).await;

            assert!(
                store
                    .read()
                    .map(|s| !s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
                    .unwrap_or(false),
                "a share whose file could not be written must NOT reach the shared \
                 store: the node would sign now and be mute after a restart"
            );
            assert_eq!(
                actor.metrics.dkg_share_persist_failed.get(),
                1,
                "the refusal is counted — otherwise it is indistinguishable from a \
                 ceremony that never finalized"
            );
            assert_eq!(
                actor.metrics.dkg_ceremony_ok.get(),
                0,
                "and the promote-driving signal must NOT fire for a refused share"
            );
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("acquiring_logs"),
                "a refused adoption re-selects the retained journal over the pinned set"
            );
            assert!(
                actor
                    .stalls(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .contains(&StallReason::PersistFailed),
                "the refusal raises Stalled{{PersistFailed}} (§5.4)"
            );
            let _ = std::fs::remove_file(&bad_dir);
        });
    }

    /// A post-deadline resume whose journal lacks a valid `OwnSeal` (torn tail) stays
    /// player-only: it re-emits peer acks and recovers its share as a player, never
    /// re-sealing a possibly-divergent second log.
    #[test]
    fn torn_own_seal_post_deadline_refetches_not_reseals() {
        // A pre-seal journal resumed at/after the deadline models the torn-own-seal state.
        let (committee, key0, pre_seal) = node0_pre_seal_journal(29);
        let me0 = key0.public_key();

        let resumed = DkgCeremony::resume(
            b"FLUENT_DPOS_V1_clocktest",
            DETERMINISTIC_BOOTSTRAP_EPOCH,
            committee,
            key0,
            pre_seal,
            false, // reconstruct_dealer = false (resumed at/after the seal deadline)
            &BTreeMap::new(),
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

    /// A still-dealing ceremony holds a fully-held, selectable quorum over the agreed set
    /// and still does not finalize: the gate is the seal, not the logs.
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
            // Pre-seal resume with its dealer reconstructed: every peer log is recorded,
            // so only the seal gate can stop the finalize.
            let (committee, key0, journal) = node0_pre_seal_journal(7);
            oracle.manager().track(0, committee.clone()).await;
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                true,
                &BTreeMap::new(),
            )
            .expect("pre-deadline resume");
            assert!(
                !resumed.ceremony.dealing_closed(),
                "a pre-seal resume keeps a LIVE dealer"
            );

            let store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut actor = standalone_actor(&oracle, key0, committee.clone(), None).await;
            actor.store = store.clone();
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);
            let mut arng = StdRng::seed_from_u64(9);
            let pinned: BTreeMap<u8, B256> =
                pinned_logs_of(&actor, DETERMINISTIC_BOOTSTRAP_EPOCH, &committee)
                    .into_iter()
                    .collect();
            actor.pin_recorded_as_agreed(DETERMINISTIC_BOOTSTRAP_EPOCH, &mut arng);
            let c = actor
                .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                .expect("ceremony");
            assert_eq!(
                c.pinned_ready(&mut arng, &committee, &pinned),
                (true, true),
                "precondition: the agreed set is a fully-held, selectable quorum"
            );

            actor.drive_finalization(&mut arng);
            assert!(
                store.read().map(|s| s.is_empty()).unwrap_or(false),
                "a dealing-open ceremony must NOT finalize before its seal"
            );
        });
    }

    /// A resume that holds only its own self-dealing, then receives the peers' logs, makes
    /// `finalize` fail with `MissingPlayerDealing`: a quorum is selectable but its private
    /// dealings are missing from our `view`. The failure is terminal (`Unrecoverable` +
    /// `Stalled{Unrecoverable}`, no share) and the recorded logs stay servable to peers.
    #[test]
    fn finalize_err_missing_player_dealing_is_unrecoverable_and_still_serves() {
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

            // Only our self-dealing is journaled, so `Player::resume`'s integrity check passes.
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                self_only_journal,
                // Player-only (post-deadline): `dealing_closed()`, so the ceremony can reach
                // the finalize this test exercises.
                false,
                &BTreeMap::new(),
            )
            .expect("self-only resume");

            let store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut actor = standalone_actor(&oracle, key0, committee, None).await;
            actor.store = store.clone();
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);

            // The delivered peer logs are all recorded — yet our `view` lacks their dealings.
            let mut arng = StdRng::seed_from_u64(9);
            for (dealer, signed) in &peer_logs {
                let key = DkgLogKey {
                    epoch: DETERMINISTIC_BOOTSTRAP_EPOCH,
                    dealer: dealer.clone(),
                    hash: log_hash(signed),
                };
                let _ = actor.ingest_log(&key, signed.encode(), &mut arng).await;
            }
            let served: Vec<DkgLogKey> = peer_logs
                .iter()
                .map(|(dealer, signed)| DkgLogKey {
                    epoch: DETERMINISTIC_BOOTSTRAP_EPOCH,
                    dealer: dealer.clone(),
                    hash: log_hash(signed),
                })
                .collect();
            actor.pin_recorded_as_agreed(DETERMINISTIC_BOOTSTRAP_EPOCH, &mut arng);
            actor.drive_finalization(&mut arng);

            assert!(
                store
                    .read()
                    .map(|s| !s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
                    .unwrap_or(false),
                "finalize-Err (MissingPlayerDealing) stores NO share"
            );
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("unrecoverable"),
                "MissingPlayerDealing is the one finalize error no fetch can answer"
            );
            assert!(
                actor
                    .stalls(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .contains(&StallReason::Unrecoverable),
                "the terminal raises Stalled{{Unrecoverable}}"
            );
            for key in &served {
                assert!(
                    actor.serve_log(key).is_some(),
                    "the recorded logs outlive the consumed ceremony in the serve store"
                );
            }
        });
    }

    /// [`node0_pre_seal_journal_full_sealed`] split into node-0's self-only journal (its own
    /// `ReceivedDealing`, so the resumed `view` lacks every peer dealing) and the 3 sealed
    /// peer logs, delivered after the resume.
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
        // `JournalRecord` is not `Clone` (it holds secret `DealerPrivMsg`), so the owned
        // journal is partitioned in one pass.
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

    /// Like [`node0_pre_seal_journal`], but node-0 also seals: the journal holds every
    /// `ReceivedDealing`, its `OwnSeal` and every `PeerLog`.
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

    /// An epoch's journal and serve-store entry survive the boundary until the epoch ages
    /// out of the retention window (`e + JOURNAL_RETENTION_EPOCHS < now`).
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
            let (dealer, signed) = logs
                .iter()
                .find(|(pk, _)| *pk == keys[1].public_key())
                .expect("keys[1] sealed");
            let key = DkgLogKey {
                epoch: 2,
                dealer: dealer.clone(),
                hash: log_hash(signed),
            };
            assert!(
                actor.serve_log(&key).is_some(),
                "precondition: epoch-2 log servable"
            );

            let mut arng = StdRng::seed_from_u64(9);
            actor.on_height(BOUNDARY + 1, &mut arng).await; // now = 2
            assert!(
                actor.serve_log(&key).is_some(),
                "journal + serve store are RETAINED across the boundary within the window"
            );
            assert!(
                journal_path(&dir, 2).exists(),
                "the journal file survives the boundary within the retention window"
            );

            let past = INTERVAL * (DETERMINISTIC_BOOTSTRAP_EPOCH + JOURNAL_RETENTION_EPOCHS + 1);
            actor.on_height(past, &mut arng).await; // now = 11
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

    /// A demote-heal in `Acquiring(Logs)` keeps issuing a targeted fetch for the pinned
    /// `dealers()` body it lacks — the fetch is driven by the slot, not by a height gate.
    #[test]
    fn a_heal_fetches_missing_dealer_past_boundary() {
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
            // Stands in for the artifact's pinned body: the heal asks for that dealer by hash.
            let missing_hash = B256::repeat_byte(0x5A);

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
                committee_for,
                Arc::new(RwLock::new(BTreeMap::new())),
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                ShareState::Plaintext,
                Wiring::inert(resolver),
            );
            actor.enter(
                2,
                EpochState::Acquiring(Acquire::Logs(Box::new(RecomputeState {
                    outcome,
                    pinned: BTreeMap::from([(missing.clone(), missing_hash)]),
                    want: BTreeSet::from([(missing.clone(), missing_hash)]),
                    digest: B256::ZERO,
                    attempted: false,
                }))),
            );

            // `fetch_missing_logs` has no height gate — the slot alone keeps the fetch alive.
            actor.fetch_missing_logs().await;
            assert!(
                in_flight.lock().unwrap().contains(&DkgLogKey {
                    epoch: 2,
                    dealer: missing,
                    hash: missing_hash,
                }),
                "the heal keeps fetching the missing pinned body PAST the boundary, by hash"
            );
        });
    }

    /// The core heal: a demoted committee[2] member (no share, one pinned dealer's log
    /// missing) detects the demote in its own `on_height` loop from the pinned artifact
    /// alone, fetches the missing log, recomputes its share scoped to `dealers()`, verifies
    /// it against the pinned Output and adopts it, reproducing its canonical share.
    #[test]
    fn demoted_member_recomputes_share_and_heals() {
        // The heal's journal parse bumps the process-global `COLD_PARSE_COUNT` that the
        // burst-bound tests reset and assert on.
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
            // node-0's journal for epoch 2, plus an identical copy to derive the pinned
            // Output and its canonical share.
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
                &BTreeMap::new(),
            )
            .expect("resume");
            let pinned_canon: BTreeMap<u8, B256> = committee
                .iter()
                .enumerate()
                .filter_map(|(idx, pk)| canon.ceremony.signed_log_hash(pk).map(|h| (idx as u8, h)))
                .collect();
            let (outcome, canonical_share) = canon
                .ceremony
                .finalize_over_pinned(&mut frng, &committee, &pinned_canon)
                .expect("finalize");
            let outcome_bytes = crate::beacon::outcome::encode_outcome(&outcome);
            let dealers: BTreeSet<PeerPubkey> = outcome.dealers().iter().cloned().collect();

            // Write the journal minus one pinned-dealer peer log — the demote — and hold
            // that body back for delivery through the resolver.
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
                                held_back = Some((pk, *signed));
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
            // Mock artifact reader: epoch 2's payload, `None` elsewhere — what `recover(2)`
            // reads on the first tick.
            actor.outcome_at = artifact_reader(
                2,
                pinned_canon.iter().map(|(i, h)| (*i, *h)).collect(),
                &outcome_bytes,
            );
            let mut arng = StdRng::seed_from_u64(9);

            // now == 2: `recover(2)` resumes the journal player-only over the pinned bodies
            // and holds no share.
            actor.on_height(BOUNDARY, &mut arng).await;
            assert_eq!(
                actor.phase(2),
                Some("agreed"),
                "recover(2) resumed the journal against the held artifact"
            );
            assert!(
                actor.stalls(2).contains(&StallReason::BodyMissing),
                "the finalize waits on the held-back pinned body, and says so"
            );
            assert!(
                !actor
                    .ceremony(2)
                    .expect("ceremony")
                    .holds(&(held_dealer.clone(), log_hash(&held_log))),
                "the held-back pinned body is the one still missing"
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

            let key = DkgLogKey {
                epoch: 2,
                dealer: held_dealer.clone(),
                hash: log_hash(&held_log),
            };
            let accepted = actor.ingest_log(&key, held_log.encode(), &mut arng).await;
            assert!(accepted, "the delivered pinned-dealer log is accepted");

            let stored = actor
                .store
                .read()
                .unwrap()
                .get(&2)
                .map(|s| s.encode().to_vec());
            assert_eq!(
                stored,
                Some(canonical_share.encode().to_vec()),
                "the demoted member recomputed its CANONICAL share from the retained journal"
            );
            assert_eq!(
                actor.phase(2),
                Some("keyed"),
                "the epoch is Keyed once the share is adopted"
            );
            assert_eq!(
                actor.metrics.dkg_ceremony_ok.get(),
                1,
                "the heal fires dkg_ceremony_ok (the promote-driving signal)"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// The actor asks for the live epoch's artifact on every tick for as long as it lacks
    /// one: a share is not a key, so holding the share does not end the ask.
    #[test]
    fn the_live_epoch_artifact_is_asked_for_every_tick_until_it_is_held() {
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
            let mut rng = StdRng::seed_from_u64(0x1166);
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;
            let (_outcome, shares) = crate::beacon::dkg_oracle::run_local_dkg(
                &mut rng,
                b"FLUENT_DPOS_V1_clocktest",
                2,
                &keys,
                &keys,
            )
            .expect("dkg");
            let me = keys[0].public_key();

            let dir = fresh_share_dir("live-epoch-pull");
            std::fs::create_dir_all(&dir).expect("mkdir");
            let mut actor = standalone_actor(
                &oracle,
                keys[0].clone(),
                committee.clone(),
                Some(dir.clone()),
            )
            .await;
            // No artifact yet: at this seam that is indistinguishable from a carry-forward
            // epoch, which is why the ask is unconditional.
            let outcome_at: AgreedOutcomeAt = Arc::new(|_epoch: u64| None);
            actor.outcome_at = outcome_at;
            let asked: Arc<std::sync::Mutex<Vec<u64>>> = Arc::default();
            actor.pull_artifact = {
                let asked = asked.clone();
                Arc::new(move |epoch: u64| asked.lock().expect("asked").push(epoch))
            };

            let mut arng = StdRng::seed_from_u64(9);
            actor.on_height(BOUNDARY, &mut arng).await; // now = 2
            assert_eq!(
                *asked.lock().expect("asked"),
                vec![2],
                "the live epoch this member holds no share for is asked for"
            );
            actor.on_height(BOUNDARY + 1, &mut arng).await;
            assert_eq!(
                *asked.lock().expect("asked"),
                vec![2, 2],
                "still asked while the epoch stays share-less"
            );

            // The share lands, but a share is not a key: `PK_E` comes from the artifact, so
            // the ask continues until the artifact arrives.
            actor
                .store
                .write()
                .expect("store")
                .insert(2, shares.get(&me).expect("our share").clone());
            actor.on_height(BOUNDARY + 2, &mut arng).await;
            assert_eq!(
                *asked.lock().expect("asked"),
                vec![2, 2, 2],
                "a held share does NOT end the ask: without the artifact the epoch has no \
                 key at all, and this loop is the only leg that asks for a member's"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// A restart that kept the epoch's share file but not its artifact, in a live epoch,
    /// asks peers for the artifact and keys when it arrives. No other leg covers that
    /// state: non-member acquisition excludes members, the repair sweep takes only past
    /// epochs, and the cert-inlet's `ensure_key` spends the network-free `PinEffort::Local`.
    #[test]
    fn a_restarted_member_with_a_share_and_no_artifact_asks_for_it_and_keys_on_arrival() {
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
            let mut rng = StdRng::seed_from_u64(0x51A);
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;
            let (outcome, shares) = crate::beacon::dkg_oracle::run_local_dkg(
                &mut rng,
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                &keys,
                &keys,
            )
            .expect("dkg");
            let me = keys[0].public_key();
            let my_share = shares.get(&me).expect("our share").clone();

            let dir = fresh_share_dir("partial-success");
            share_state::persist(
                &dir,
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                &my_share,
                &ShareState::Plaintext,
            )
            .expect("persist the share");
            let reloaded = share_state::load_all(&dir, &ShareState::Plaintext);
            assert_eq!(
                reloaded.len(),
                1,
                "the share really came back off the disk — the restart half of the fixture"
            );
            let artifacts = crate::beacon::artifact::ArtifactStore::new();

            let mut actor = standalone_actor(
                &oracle,
                keys[0].clone(),
                committee.clone(),
                Some(dir.clone()),
            )
            .await;
            for (epoch, share) in reloaded {
                actor.store.write().expect("store").insert(epoch, share);
            }
            actor.outcome_at = store_reader(&artifacts);
            let asked: Arc<std::sync::Mutex<Vec<u64>>> = Arc::default();
            actor.pull_artifact = {
                let asked = asked.clone();
                Arc::new(move |epoch: u64| asked.lock().expect("asked").push(epoch))
            };

            let key_index = crate::beacon::artifact::key_index_over(artifacts.clone(), &[]);
            assert!(
                actor
                    .store
                    .read()
                    .expect("store")
                    .contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH),
                "the restart kept the share"
            );
            assert!(
                key_index.key_at(DETERMINISTIC_BOOTSTRAP_EPOCH).is_none(),
                "and holds NO key for that epoch: the artifact is its only owner"
            );

            let mut arng = StdRng::seed_from_u64(0x51B);
            actor.on_height(BOUNDARY, &mut arng).await; // now = 2, a LIVE epoch
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("acquiring_artifact_for_share"),
                "recover(2): share held, no artifact ⇒ the partial-success phase"
            );
            assert_eq!(
                *asked.lock().expect("asked"),
                vec![DETERMINISTIC_BOOTSTRAP_EPOCH],
                "a member holding the share and no artifact ASKS peers for the artifact \
                 (§5.4 `Acquiring{{artifact}}`), instead of sitting verify-only until the \
                 next epoch boundary"
            );

            let served =
                crate::beacon::artifact::artifact_with_key(DETERMINISTIC_BOOTSTRAP_EPOCH, outcome);
            assert!(
                artifacts
                    .insert(DETERMINISTIC_BOOTSTRAP_EPOCH, served.clone())
                    .is_ok(),
                "the arriving artifact is filed"
            );
            actor.on_artifact(served, &mut arng).await;
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("keyed"),
                "the held share and the arrived artifact make the epoch Keyed"
            );
            assert_eq!(
                key_index
                    .sharing_at(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .map(|(minted_at, _)| minted_at),
                Some(DETERMINISTIC_BOOTSTRAP_EPOCH),
                "KEYED: the polynomial resolves off the arrived artifact"
            );
            assert!(
                actor
                    .store
                    .read()
                    .expect("store")
                    .contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH),
                "and it pairs with the share the restart kept — the epoch signs again"
            );

            actor.on_height(BOUNDARY + 1, &mut arng).await;
            assert_eq!(
                *asked.lock().expect("asked"),
                vec![DETERMINISTIC_BOOTSTRAP_EPOCH],
                "the ask stops the moment the artifact is local: one fetch, not one per tick"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// An epoch past the retention window is neither asked for nor healed — its journal
    /// is reclaimed, so a retry could not recompute anything from it.
    #[test]
    fn an_aged_out_epoch_is_neither_asked_for_nor_healed() {
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
            let mut rng = StdRng::seed_from_u64(0x1167);
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
            // `DkgOutcome` is not `Clone`, so the actor's copy is re-parsed from the wire encoding.
            let outcome_bytes = crate::beacon::outcome::encode_outcome(&outcome);
            let missing = outcome.dealers().iter().next().cloned().expect("a dealer");
            let missing_hash = B256::repeat_byte(0x5A);

            let dir = fresh_share_dir("aged-out-heal");
            std::fs::create_dir_all(&dir).expect("mkdir");
            let me_pk = keys[0].public_key();
            for (pk, signed) in &mint_committee_logs_at(&keys, &committee, 2) {
                let rec = if *pk == me_pk {
                    JournalRecord::OwnSeal(Box::new(signed.clone()))
                } else {
                    JournalRecord::PeerLog(Box::new(signed.clone()))
                };
                share_state::append_journal(&dir, 2, &rec, &ShareState::Plaintext)
                    .expect("append journal");
            }

            let (sender, receiver) = oracle
                .control(me_pk.clone())
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
                keys[0].clone(),
                sender,
                receiver,
                committee_for,
                Arc::new(RwLock::new(BTreeMap::new())),
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                ShareState::Plaintext,
                {
                    let mut wiring = Wiring::inert(resolver);
                    wiring.share_dir = dir.clone();
                    wiring
                },
            );
            actor.outcome_at = artifact_reader(2, vec![(0, missing_hash)], &outcome_bytes);
            // Every epoch mints here; an unchanged committee carries forward and asks for nothing.
            actor.changed = Arc::new(|_epoch: u64| Some(true));
            let asked: Arc<std::sync::Mutex<Vec<u64>>> = Arc::default();
            actor.pull_artifact = {
                let asked = asked.clone();
                Arc::new(move |epoch: u64| asked.lock().expect("asked").push(epoch))
            };
            actor.enter(
                2,
                EpochState::Acquiring(Acquire::Logs(Box::new(RecomputeState {
                    outcome: crate::beacon::outcome::parse_outcome(&outcome_bytes)
                        .expect("re-parse"),
                    pinned: BTreeMap::from([(missing.clone(), missing_hash)]),
                    want: BTreeSet::from([(missing.clone(), missing_hash)]),
                    digest: B256::ZERO,
                    attempted: false,
                }))),
            );
            assert!(
                journal_path(&dir, 2).exists(),
                "precondition: the heal inputs"
            );

            // The height is one past the retention edge, derived from `JOURNAL_RETENTION_EPOCHS`.
            let mut arng = StdRng::seed_from_u64(9);
            actor
                .on_height(
                    INTERVAL * (2 + crate::beacon::JOURNAL_RETENTION_EPOCHS + 1),
                    &mut arng,
                )
                .await;

            assert!(
                actor.phase(2).is_none(),
                "the heal is dropped once its epoch leaves the retention window"
            );
            assert!(
                !journal_path(&dir, 2).exists(),
                "and its inputs are reclaimed, which is why retrying could not work"
            );
            assert!(
                actor.store.read().expect("store").get(&2).is_none(),
                "no share was adopted for the expired epoch"
            );
            assert!(
                !in_flight
                    .lock()
                    .expect("in flight")
                    .iter()
                    .any(|k| k.epoch == 2),
                "no dealer log is fetched for an epoch that can no longer be recomputed"
            );
            // Every in-window epoch is a mint this member never dealt for, so all are asked for.
            let now = 2 + crate::beacon::JOURNAL_RETENTION_EPOCHS + 1;
            let lo = now
                .saturating_sub(crate::beacon::JOURNAL_RETENTION_EPOCHS)
                .max(DETERMINISTIC_BOOTSTRAP_EPOCH);
            let expected: Vec<u64> = (lo..=now).filter(|e| *e != 2).collect();
            assert_eq!(
                *asked.lock().expect("asked"),
                expected,
                "nothing is asked for the expired epoch — and the in-window epochs ARE \
                 asked for, so the absence above is not vacuous"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// A node that acked a dealer's point and then lost it — the ack is withheld until
    /// the journal write is durable — can never be sent that point again: the dealer
    /// does not reveal to a peer whose ack it holds, and a sealed log cannot be
    /// re-opened, so the epoch is verdicted `Unrecoverable` once and never retried.
    ///
    /// The verdict is not a panic: a share-less member is safe as a verifier and sits
    /// the epoch out.
    #[test]
    fn an_unrecoverable_share_is_verdicted_once_and_never_retried() {
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
            let (committee, key0, full_journal) = node0_pre_seal_journal_full_sealed(61);
            let (_c2, _k2, journal_for_outcome) = node0_pre_seal_journal_full_sealed(61);
            oracle.manager().track(0, committee.clone()).await;
            let me0 = key0.public_key();

            // The agreed outcome is derived from an identical undamaged journal.
            let mut frng = StdRng::seed_from_u64(1);
            let mut canon = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal_for_outcome,
                false,
                &BTreeMap::new(),
            )
            .expect("resume");
            let pinned_canon: BTreeMap<u8, B256> = committee
                .iter()
                .enumerate()
                .filter_map(|(idx, pk)| canon.ceremony.signed_log_hash(pk).map(|h| (idx as u8, h)))
                .collect();
            let (outcome, _share) = canon
                .ceremony
                .finalize_over_pinned(&mut frng, &committee, &pinned_canon)
                .expect("finalize");
            let outcome_bytes = crate::beacon::outcome::encode_outcome(&outcome);

            // The damage: every dealer log is on disk, so `want` is empty and the
            // recompute runs — but a dealing those logs show as acked is gone.
            let dir = fresh_share_dir("unrecoverable-share");
            std::fs::create_dir_all(&dir).expect("mkdir");
            let mut dropped = false;
            for r in full_journal {
                if !dropped {
                    if let JournalRecord::ReceivedDealing(dealer, _, _) = &r {
                        if *dealer != me0 {
                            dropped = true;
                            continue;
                        }
                    }
                }
                share_state::append_journal(&dir, 2, &r, &ShareState::Plaintext).expect("append");
            }
            assert!(dropped, "precondition: an acked peer dealing was destroyed");

            let (sender, receiver) = oracle
                .control(me0.clone())
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
                key0.clone(),
                sender,
                receiver,
                committee_for,
                Arc::new(RwLock::new(BTreeMap::new())),
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                ShareState::Plaintext,
                {
                    let mut wiring = Wiring::inert(resolver);
                    wiring.share_dir = dir.clone();
                    wiring
                },
            );
            actor.outcome_at = artifact_reader(
                2,
                pinned_canon.iter().map(|(i, h)| (*i, *h)).collect(),
                &outcome_bytes,
            );
            let asked: Arc<std::sync::Mutex<Vec<u64>>> = Arc::default();
            actor.pull_artifact = {
                let asked = asked.clone();
                Arc::new(move |epoch: u64| asked.lock().expect("asked").push(epoch))
            };

            let mut arng = StdRng::seed_from_u64(9);
            // `BOUNDARY` puts the clock at epoch 2: `recover(2)` replays the journal
            // against the held artifact and trips the missing acked dealing.
            actor.on_height(BOUNDARY, &mut arng).await;

            assert_eq!(
                actor.phase(2),
                Some("unrecoverable"),
                "the verdict is the epoch's phase, so nothing is driven for it again"
            );
            assert!(
                actor.stalls(2).contains(&StallReason::Unrecoverable),
                "the terminal raises Stalled{{Unrecoverable}}"
            );
            assert_eq!(
                actor.metrics.dkg_share_unrecoverable.get(),
                1,
                "the WARN and this counter are the same statement — one per epoch"
            );
            assert!(
                actor.store.read().expect("store").get(&2).is_none(),
                "no share was adopted: the node sits the epoch out as a verifier"
            );

            actor.on_height(BOUNDARY + 1, &mut arng).await;
            assert_eq!(
                actor.metrics.dkg_share_unrecoverable.get(),
                1,
                "warned once, not once per height tick"
            );
            assert_eq!(actor.phase(2), Some("unrecoverable"));
            assert!(
                in_flight.lock().expect("in flight").is_empty(),
                "no dealer log is fetched for an epoch whose share cannot be reassembled"
            );
            assert!(
                asked.lock().expect("asked").is_empty(),
                "and no artifact is asked for either — the artifact is not what is missing"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// With every `append_journal` failing, node-0 withholds its ack to each dealer, so
    /// every dealer reveals node-0's point in its log — a log can never record an ack
    /// this node cannot back with a durable view. Node-0 still finalizes its share over
    /// those reveals. Control: a working share dir → the acks land → not revealed.
    ///
    /// Only the journal path is made unwritable. Breaking the whole share dir would also
    /// fail `share_state::persist`, which refuses the share, so `seeded` would be false
    /// for the wrong reason.
    #[test]
    fn acked_dealing_withheld_on_append_failure_still_recoverable() {
        let runtime = deterministic::Runner::default();
        let (seeded_fail, revealed_fail) = runtime.start(|ctx| async move {
            let bad = fresh_share_dir("append-fail");
            std::fs::create_dir_all(&bad).expect("share dir");
            std::fs::create_dir_all(bad.join(format!(
                "beacon-dkgjournal-e{DETERMINISTIC_BOOTSTRAP_EPOCH}.bin"
            )))
            .expect("journal path as a directory");
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

    /// Drives the 4-dealer bootstrap DKG over the sim with node-0 as the victim, using
    /// `victim_dir` as its share dir, and feeds heights until it finalizes; returns
    /// `(seeded, revealed)` read from node-0's adopted `Output`.
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
        // One shared index: node-0 publishes only what it journaled durably, and the
        // stub agreement waits for a quorum of records without any leader rotation to
        // move past a node holding too few.
        let recorded: DkgLogIndex = Arc::new(RwLock::new(BTreeMap::new()));
        let mut sinks = Vec::new();
        let mut victim_adopted = None;
        let mut victim_confirms = None;
        for (i, k) in keys.iter().enumerate() {
            let store = if i == 0 {
                victim_store.clone()
            } else {
                Arc::new(RwLock::new(BTreeMap::new()))
            };
            let dir = if i == 0 { victim_dir.clone() } else { None };
            let (sink, adopted, confirms) = spawn_dealer_at(
                &ctx,
                &oracle,
                k.clone(),
                committee.clone(),
                store,
                Arc::new(tokio::sync::Notify::new()),
                INTERVAL,
                dir,
                7,
                recorded.clone(),
            )
            .await;
            if i == 0 {
                victim_adopted = Some(adopted);
                victim_confirms = Some(confirms);
            }
            sinks.push(sink);
        }
        let victim_adopted = victim_adopted.expect("node 0 was spawned");
        let victim_confirms = victim_confirms.expect("node 0 was spawned");
        for h in 0..=(BOUNDARY - 1) {
            for s in &sinks {
                s.send_replace(h);
            }
            ctx.sleep(Duration::from_millis(50)).await;
        }
        // Pinned as a fact rather than assumed: the dealers wire a `ConfirmPool` and the
        // shared index, so the confirmation traffic asserted below really crosses the network.
        let me0_seat = committee.position(&me0).expect("node 0 seats") as u8;
        assert!(
            victim_confirms
                .covering(DETERMINISTIC_BOOTSTRAP_EPOCH, &[])
                .iter()
                .any(|c| c.idx != me0_seat),
            "a peer's ShareConfirm reached node-0's pool over the sim network"
        );
        // The share says it finalized; the `Output` says whether the peers revealed this
        // node's point.
        let seeded = victim_store
            .read()
            .map(|s| s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
            .unwrap_or(false);
        let revealed = victim_adopted
            .read()
            .ok()
            .and_then(|a| {
                a.get(&DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .map(|o| o.revealed().iter().any(|p| *p == me0))
            })
            .unwrap_or(false);
        (seeded, revealed)
    }

    /// The bootstrap-epoch ceremony `Info` over `committee`, for the test helpers that
    /// check or build dealer logs.
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

    /// A mock [`AgreedOutcomeAt`] answering `epoch` with the pinned `logs` and the
    /// `Output` encoded in `outcome_bytes`, and `None` for every other epoch — the
    /// artifact read `recover(E)` makes on a restart.
    fn artifact_reader(epoch: u64, logs: Vec<(u8, B256)>, outcome_bytes: &[u8]) -> AgreedOutcomeAt {
        let proposal = crate::beacon::dkg_agree::DkgProposal {
            target_epoch: epoch,
            logs,
            group_key: crate::beacon::outcome::parse_outcome(outcome_bytes).expect("outcome"),
            confirms: Vec::new(),
        };
        Arc::new(move |e: u64| {
            (e == epoch).then(|| StoredArtifact {
                held: proposal.clone(),
                divergent: None,
            })
        })
    }

    /// The production [`AgreedOutcomeAt`] over an artifact store — the closure
    /// `beacon::build` wires: the held payload plus the divergent second value.
    fn store_reader(store: &crate::beacon::artifact::ArtifactStore) -> AgreedOutcomeAt {
        let store = store.clone();
        Arc::new(move |epoch: u64| {
            store.view(epoch).map(|(held, divergent)| StoredArtifact {
                held: held.0.clone(),
                divergent,
            })
        })
    }

    /// A quorum-certified artifact over `logs`, built the way the agreement plane does.
    /// The write-back reads only the target epoch and the pinned set, and the certificate
    /// is verified before hand-over, so the committee here is a fresh set whose only job
    /// is to make a real `Finalization` constructible.
    fn agreed_artifact(target_epoch: u64, logs: Vec<(u8, B256)>) -> AgreedArtifact {
        agreed_artifact_with_committee(target_epoch, logs).0
    }

    /// [`agreed_artifact`] carrying `group_key` — the polynomial the actor's share must
    /// lie on, derived by the certifier over exactly `logs`.
    fn agreed_artifact_keyed(
        target_epoch: u64,
        logs: Vec<(u8, B256)>,
        group_key: DkgOutcome,
    ) -> AgreedArtifact {
        certify(target_epoch, logs, Some(group_key)).0
    }

    /// [`agreed_artifact`] plus the committee its certificate was signed by, which a
    /// pull-seam bridge needs to verify that artifact.
    fn agreed_artifact_with_committee(
        target_epoch: u64,
        logs: Vec<(u8, B256)>,
    ) -> (AgreedArtifact, fluentbase_bls::EpochCommittee) {
        certify(target_epoch, logs, None)
    }

    /// Certifies `(logs, group_key)` for `target_epoch` under a fresh 4-member committee;
    /// a random `deal` stands in for a key the test does not care about.
    fn certify(
        target_epoch: u64,
        logs: Vec<(u8, B256)>,
        group_key: Option<DkgOutcome>,
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
        let group_key = group_key.unwrap_or_else(|| {
            deal::<MinSig, PeerPubkey, N3f1>(
                &mut rng,
                Mode::NonZeroCounter,
                Set::from_iter_dedup(peers.iter().map(|p| p.public_key())),
            )
            .expect("deal")
            .0
        });

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
                let signer =
                    build_signer(&ns, bimap.clone(), kp, target_epoch, None).expect("member");
                Finalize::sign(&signer, payload.clone()).expect("sign")
            })
            .collect();
        let certificate = Finalization::from_finalizes(
            &build_verifier(&ns, bimap.clone(), target_epoch, None),
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

    /// The group key a certifier derives over `pinned` from `ceremony` — the polynomial
    /// an artifact naming that set carries. Panics unless every pinned body is held there
    /// and a quorum is selectable within them.
    fn key_over(
        ceremony: &DkgCeremony,
        committee: &Set<PeerPubkey>,
        pinned: &[(u8, B256)],
        rng: &mut impl CryptoRngCore,
    ) -> DkgOutcome {
        match ceremony.derive_pinned(rng, committee, &pinned.iter().copied().collect()) {
            PinnedDerive::Derived(key) => *key,
            other => panic!("the pinned set must derive a key here, got {other:?}"),
        }
    }

    /// The artifact this actor's own instance would certify for `epoch`: every
    /// dealer log it holds, and the key derived over exactly those.
    fn artifact_over(
        actor: &DkgActor<
            impl Sender<PublicKey = PeerPubkey>,
            impl Receiver<PublicKey = PeerPubkey>,
            impl Resolver<Key = DkgLogKey, PublicKey = PeerPubkey>,
        >,
        epoch: u64,
        committee: &Set<PeerPubkey>,
        rng: &mut impl CryptoRngCore,
    ) -> AgreedArtifact {
        let logs = pinned_logs_of(actor, epoch, committee);
        let key = key_over(
            actor.ceremony(epoch).expect("ceremony"),
            committee,
            &logs,
            rng,
        );
        agreed_artifact_keyed(epoch, logs, key)
    }

    /// The pinned set an artifact for this ceremony would name: every dealer log the
    /// actor holds, at its committee index.
    fn pinned_logs_of(
        actor: &DkgActor<
            impl Sender<PublicKey = PeerPubkey>,
            impl Receiver<PublicKey = PeerPubkey>,
            impl Resolver<Key = DkgLogKey, PublicKey = PeerPubkey>,
        >,
        epoch: u64,
        committee: &Set<PeerPubkey>,
    ) -> Vec<(u8, B256)> {
        let c = actor.ceremony(epoch).expect("ceremony");
        committee
            .iter()
            .enumerate()
            .filter_map(|(i, pk)| c.signed_log_hash(pk).map(|h| (i as u8, h)))
            .collect()
    }

    /// The halted-chain recovery, the reason the agreement plane exists: the chain stopped
    /// at `epoch_start(E+1)`, so the finalized dealer-log set never arrives and no height
    /// tick will come. Delivering the agreed artifact alone must produce
    /// `(PK_{E+1}, share)`.
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
                &BTreeMap::new(),
            )
            .expect("post-deadline resume");

            let store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let share_notify = Arc::new(tokio::sync::Notify::new());
            let mut actor = standalone_actor(&oracle, key0, committee.clone(), None).await;
            actor.store = store.clone();
            actor.share_notify = share_notify.clone();
            // Only a certified set may mint, so an epoch without its artifact waits.
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);

            let mut rng = StdRng::seed_from_u64(11);
            actor.on_height(BOUNDARY - 1, &mut rng).await;
            assert!(
                store.read().map(|s| s.is_empty()).unwrap_or(false),
                "with every body held and no agreed set, the epoch cannot mint — \
                 the halt"
            );

            let artifact =
                artifact_over(&actor, DETERMINISTIC_BOOTSTRAP_EPOCH, &committee, &mut rng);
            assert_eq!(artifact.0.logs.len(), 4, "node-0 holds every dealer log");
            // No further height tick from here on: the artifact edge is the only thing that runs.
            actor.on_artifact(artifact, &mut rng).await;

            assert!(
                store
                    .read()
                    .map(|s| s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
                    .unwrap_or(false),
                "the artifact's dealer-log set is the pinned set; the existing \
                 finalize rails do the rest"
            );
            // The E+1 share-gate resolves the share at the mint the chain names: this store entry.
            assert!(
                store
                    .read()
                    .map(|s| s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
                    .unwrap_or(false),
                "the E+1 share-gate must pass with no block of E+1 in existence"
            );
            // The respawn edge is `share_notify`: its permit survives having had no waiter armed.
            assert!(
                futures::FutureExt::now_or_never(Box::pin(share_notify.notified())).is_some(),
                "the write-back must fire the edge the epoch manager respawns on"
            );
            // The write-back is complete, so the agreed set no longer pins a ceremony open.
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("keyed"),
                "a completed write-back lands the epoch in `Keyed`"
            );
        });
    }

    /// A node that only ever pulled the agreed set must still end holding the epoch's
    /// share, not just a verifiable key: its own instance died and its store holds no share.
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
                &BTreeMap::new(),
            )
            .expect("post-deadline resume");

            let store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut actor = standalone_actor(&oracle, key0, committee.clone(), None).await;
            actor.store = store.clone();
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);

            let mut rng = StdRng::seed_from_u64(12);
            actor.on_height(BOUNDARY - 1, &mut rng).await;
            assert!(
                store.read().map(|s| s.is_empty()).unwrap_or(false),
                "no local instance decided, so nothing may mint yet"
            );

            let logs = pinned_logs_of(&actor, DETERMINISTIC_BOOTSTRAP_EPOCH, &committee);
            let key = key_over(
                actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .expect("ceremony"),
                &committee,
                &logs,
                &mut rng,
            );
            let (artifact, artifact_committee) =
                certify(DETERMINISTIC_BOOTSTRAP_EPOCH, logs, Some(key));
            let committee_source: crate::beacon::artifact::CommitteeSource = Arc::new(move |e| {
                (e == DETERMINISTIC_BOOTSTRAP_EPOCH).then(|| artifact_committee.clone())
            });

            // The peer that decided without this node.
            let served = crate::beacon::artifact::ArtifactStore::new();
            assert!(served
                .insert(DETERMINISTIC_BOOTSTRAP_EPOCH, artifact)
                .is_ok());
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

            // In the node the write-back sits between the bridge and here; the test
            // drives the actor's `artifacts_rx` arm by hand.
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

    /// The artifact edge issues the recovery fetch itself: a member missing a pinned body
    /// cannot finalize over the agreed set, and with the chain halted `on_height` never
    /// fetches again — including past the target's own boundary.
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
            // Node-0 holds no log of its own (it never sealed); the pinned set below
            // names all four seats by arbitrary hash, so none of those bodies is held.
            let (committee, key0, journal) = node0_pre_seal_journal(71);
            oracle.manager().track(0, committee.clone()).await;
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                false,
                &BTreeMap::new(),
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
                committee_for,
                store.clone(),
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                ShareState::Plaintext,
                Wiring::inert(resolver),
            );
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);
            // The chain is past the target's boundary — where the height-driven fetch
            // gives up — and the height clock has stopped there.
            actor.last_height = Some(BOUNDARY);

            // The agreed set pins every seat to a hash this node holds no body for.
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

    /// A certified set mints the instant its bodies are held, on no clock at all: a
    /// quorum certificate states outright what a deadline was there to make true. The
    /// pinned set here is a strict subset of the committee (node-0 never sealed its log).
    #[test]
    fn a_certified_subset_finalizes_on_no_clock() {
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
                &BTreeMap::new(),
            )
            .expect("post-deadline resume");

            let store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut actor = standalone_actor(&oracle, key0, committee.clone(), None).await;
            actor.store = store.clone();
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);
            // The clock sits at the seal deadline, a whole margin short of the boundary.
            actor.last_height = Some(SEAL_DEADLINE);

            let logs = pinned_logs_of(&actor, DETERMINISTIC_BOOTSTRAP_EPOCH, &committee);
            assert!(
                logs.len() < committee.len(),
                "the pinned set must be a strict subset or the fast path decides this"
            );
            let mut rng = StdRng::seed_from_u64(13);
            let key = key_over(
                actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .expect("ceremony"),
                &committee,
                &logs,
                &mut rng,
            );
            actor
                .on_artifact(
                    agreed_artifact_keyed(DETERMINISTIC_BOOTSTRAP_EPOCH, logs, key),
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

    /// The plane's spawn edge: the actor announces a target once its dealing has closed,
    /// never while the dealer is still adding to the set — the agreed value would be stale
    /// by construction.
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
            let mut wiring = Wiring::standalone();
            wiring.agreement_tx = requests_tx;
            let mut actor =
                standalone_actor_wired(&oracle, keys[0].clone(), committee.clone(), None, wiring)
                    .await;
            let (cer, _step) = DkgCeremony::start(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee,
                keys[0].clone(),
            )
            .expect("start");
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, cer);

            actor.on_height(SEAL_DEADLINE - 1, &mut rng).await;
            assert!(
                requests_rx.try_recv().is_err(),
                "a ceremony that is still dealing must not start an agreement"
            );

            // The tick at the seal deadline seals the ceremony, which closes the dealing.
            actor.on_height(SEAL_DEADLINE, &mut rng).await;
            assert_eq!(
                requests_rx.try_recv().ok(),
                Some(DETERMINISTIC_BOOTSTRAP_EPOCH),
                "the dealing-closed edge asks the plane for this target's instance"
            );
        });
    }

    /// A restart between adopting an artifact and finalizing over it: the durable store
    /// holds the artifact the lost channel would have delivered, so the first tick's
    /// `recover(E)` resumes the epoch `Agreed` and the body still missing finalizes it.
    #[test]
    fn a_restart_reads_the_stored_artifact_back_into_the_actor() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(97);
            // The same fixture again, for the key the instance derived over the full
            // pinned set before the crash.
            let (_c2, _k2, full_journal) = node0_pre_seal_journal_full_sealed(97);
            oracle.manager().track(0, committee.clone()).await;

            // The on-disk state a crash leaves behind: the journal minus the one peer log
            // the recovery fetch had not delivered, and no share file.
            let dir = fresh_share_dir("artifact-read-back");
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

            // The pinned set the instance certified before the crash — every seat, the
            // withheld body at the missing dealer's — and the key over it.
            let (pinned, missing_dealer, group_key) = {
                let full = DkgCeremony::resume(
                    b"FLUENT_DPOS_V1_clocktest",
                    DETERMINISTIC_BOOTSTRAP_EPOCH,
                    committee.clone(),
                    key0.clone(),
                    full_journal,
                    false,
                    &BTreeMap::new(),
                )
                .expect("resume")
                .ceremony;
                let withheld_hash = log_hash(&withheld);
                let mut missing: Option<PeerPubkey> = None;
                let pinned: Vec<(u8, B256)> = committee
                    .iter()
                    .enumerate()
                    .map(|(i, pk)| {
                        let hash = full.signed_log_hash(pk).expect("every log is held");
                        if hash == withheld_hash {
                            missing = Some(pk.clone());
                        }
                        (i as u8, hash)
                    })
                    .collect();
                let mut rng = StdRng::seed_from_u64(0xB007);
                let key = key_over(&full, &committee, &pinned, &mut rng);
                (pinned, missing.expect("one peer log was withheld"), key)
            };
            let artifacts = crate::beacon::artifact::ArtifactStore::new();
            assert!(artifacts
                .insert(
                    DETERMINISTIC_BOOTSTRAP_EPOCH,
                    agreed_artifact_keyed(DETERMINISTIC_BOOTSTRAP_EPOCH, pinned, group_key)
                )
                .is_ok());

            let ceremony_store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut wiring = Wiring::standalone();
            wiring.outcome_at = store_reader(&artifacts);
            let mut actor =
                standalone_actor_wired(&oracle, key0, committee.clone(), Some(dir.clone()), wiring)
                    .await;
            actor.store = ceremony_store.clone();
            // The first height tick resumes the ceremony off the journal, still inside
            // epoch E (so nothing has swept it), and reads the stored artifact.
            let mut rng = StdRng::seed_from_u64(0xB008);
            actor.on_height(BOUNDARY - 1, &mut rng).await;
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("agreed"),
                "the stored artifact is the pinned set on the restart's first tick"
            );
            assert!(
                ceremony_store.read().map(|s| s.is_empty()).unwrap_or(false),
                "one pinned body is still missing, so the finalize waits"
            );

            let key = DkgLogKey {
                epoch: DETERMINISTIC_BOOTSTRAP_EPOCH,
                dealer: missing_dealer,
                hash: log_hash(&withheld),
            };
            assert!(
                actor.ingest_log(&key, withheld.encode(), &mut rng).await,
                "the withheld log is a valid one for the dealer it was fetched from"
            );
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("keyed"));
            assert!(
                ceremony_store
                    .read()
                    .map(|s| s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
                    .unwrap_or(false),
                "the read-back artifact is the pinned set; the existing rails do the rest"
            );
            // The per-tick re-read of the same value changes nothing.
            actor.on_height(BOUNDARY, &mut rng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("keyed"));
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// A standalone actor over `committee` with `share_dir`, its clock at `height` and no
    /// ceremony slots — the restart shape every `recover(E)` cell starts from.
    async fn restarted_actor(
        oracle: &Oracle<PeerPubkey, SimContext>,
        me: Ed25519PrivateKey,
        committee: Set<PeerPubkey>,
        share_dir: PathBuf,
        height: u64,
    ) -> DkgActor<
        commonware_p2p::simulated::Sender<PeerPubkey, SimContext>,
        commonware_p2p::simulated::Receiver<PeerPubkey>,
        NoopResolver,
    > {
        let mut actor = standalone_actor(oracle, me, committee, Some(share_dir)).await;
        actor.last_height = Some(height);
        actor
    }

    /// A present-but-unreadable journal: `load_journal` answers `Torn` for a non-empty
    /// file whose first record fails to decode.
    fn write_torn_journal(dir: &std::path::Path, epoch: u64) {
        std::fs::create_dir_all(dir).expect("mkdir");
        std::fs::write(
            journal_path(dir, epoch),
            [0, 0, 0, 4, 0xFF, 0xFF, 0xFF, 0xFF],
        )
        .expect("write torn journal");
    }

    fn sim_oracle(ctx: &SimContext) -> Oracle<PeerPubkey, SimContext> {
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
    }

    /// No journal before the seal: a genuine first run deals, and the seeded dealer keeps
    /// the start idempotent after a datadir loss.
    #[test]
    fn recover_no_journal_before_the_seal_deals_fresh() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, _journal) = node0_pre_seal_journal_full_sealed(101);
            oracle.manager().track(0, committee.clone()).await;
            let dir = fresh_share_dir("recover-nofile-pre");
            let mut actor =
                restarted_actor(&oracle, key0, committee, dir.clone(), SEAL_DEADLINE - 1).await;
            let mut out = Vec::new();
            assert!(actor.decide(DETERMINISTIC_BOOTSTRAP_EPOCH, &mut out).await);
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("dealing"));
            assert!(!out.is_empty(), "a fresh start sends its dealings");
            assert!(
                journal_path(&dir, DETERMINISTIC_BOOTSTRAP_EPOCH).exists(),
                "the fresh start journals its own dealing"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    #[test]
    fn recover_no_journal_at_the_seal_deadline_waits_for_the_artifact_to_heal() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, _journal) = node0_pre_seal_journal_full_sealed(102);
            oracle.manager().track(0, committee.clone()).await;
            let dir = fresh_share_dir("recover-nofile-post");
            let mut actor =
                restarted_actor(&oracle, key0, committee, dir.clone(), SEAL_DEADLINE).await;
            let asked: Arc<std::sync::Mutex<Vec<u64>>> = Arc::default();
            actor.pull_artifact = {
                let asked = asked.clone();
                Arc::new(move |epoch: u64| asked.lock().expect("asked").push(epoch))
            };
            let mut out = Vec::new();
            assert!(!actor.decide(DETERMINISTIC_BOOTSTRAP_EPOCH, &mut out).await);
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("acquiring_artifact_for_share")
            );
            assert!(out.is_empty(), "nothing is dealt or re-sealed");
            assert!(actor.ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH).is_none());
            assert!(
                !journal_path(&dir, DETERMINISTIC_BOOTSTRAP_EPOCH).exists(),
                "no journal is written for an epoch this node did not deal for"
            );
            assert!(actor.stalls(DETERMINISTIC_BOOTSTRAP_EPOCH).is_empty());
            let mut arng = StdRng::seed_from_u64(1);
            actor.on_height(BOUNDARY, &mut arng).await;
            actor.on_height(BOUNDARY + 1, &mut arng).await;
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("acquiring_artifact_for_share")
            );
            assert_eq!(
                actor.stalls(DETERMINISTIC_BOOTSTRAP_EPOCH),
                BTreeSet::from([StallReason::NoArtifact])
            );
            assert!(
                !asked.lock().expect("asked").is_empty(),
                "the absentee asks for the artifact its heal is scoped to"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// An epoch-2 ceremony node 0 was absent from: peers 1..3 sealed, the artifact is theirs.
    struct AbsentEpoch2 {
        committee: Set<PeerPubkey>,
        key0: Ed25519PrivateKey,
        logs: Vec<(PeerPubkey, DealerReveal)>,
        pinned: Vec<(u8, B256)>,
        outcome_bytes: Vec<u8>,
    }

    /// Node 0 never acks, so every sealed log reveals its point.
    fn node0_absent_epoch2_artifact(seed: u64) -> AbsentEpoch2 {
        use commonware_cryptography::bls12381::dkg::{observe, DealerLogSummary, Logs};
        let mut rng = StdRng::seed_from_u64(seed);
        let keys: Vec<Ed25519PrivateKey> = (0..4)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let me0 = keys[0].public_key();
        let logs = mint_committee_logs_at(&keys[1..], &committee, DETERMINISTIC_BOOTSTRAP_EPOCH);
        let info = info_for_test(&committee);
        let mut recorded = Logs::<
            commonware_cryptography::bls12381::primitives::variant::MinSig,
            PeerPubkey,
            N3f1,
        >::new(info.clone());
        let mut pinned = Vec::new();
        for (dealer, signed) in &logs {
            let (pk, log) = signed.clone().check(&info).expect("a sealed log checks");
            assert_eq!(pk, *dealer);
            match log.summary() {
                DealerLogSummary::Ok { reveals, .. } => assert!(
                    reveals.iter().any(|p| *p == me0),
                    "the fixture is not an absentee's: a dealer holds node 0's ack"
                ),
                DealerLogSummary::TooManyReveals => panic!("one reveal is within f"),
            }
            let idx = committee
                .iter()
                .position(|p| p == dealer)
                .expect("a committee seat");
            pinned.push((u8::try_from(idx).expect("fits"), log_hash(signed)));
            recorded.record(pk, log);
        }
        let outcome = observe::<_, _, N3f1, commonware_cryptography::ed25519::Batch>(
            &mut rng,
            recorded,
            &commonware_parallel::Sequential,
        )
        .expect("the three logs are a dealer quorum of four");
        let outcome_bytes = crate::beacon::outcome::encode_outcome(&outcome);
        AbsentEpoch2 {
            committee,
            key0: keys[0].clone(),
            logs,
            pinned,
            outcome_bytes,
        }
    }

    #[test]
    fn an_absentee_restarted_after_the_seal_heals_its_share_from_the_reveals() {
        let _guard = COLD_PARSE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let AbsentEpoch2 {
                committee,
                key0,
                logs,
                pinned,
                outcome_bytes,
            } = node0_absent_epoch2_artifact(140);
            oracle.manager().track(0, committee.clone()).await;
            let me0 = key0.public_key();
            let dir = fresh_share_dir("absentee-heal");
            let mut actor =
                restarted_actor(&oracle, key0, committee.clone(), dir.clone(), SEAL_DEADLINE).await;
            actor.outcome_at = artifact_reader(
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                pinned.clone(),
                &outcome_bytes,
            );
            let mut out = Vec::new();
            assert!(!actor.decide(DETERMINISTIC_BOOTSTRAP_EPOCH, &mut out).await);
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("acquiring_logs")
            );
            assert!(out.is_empty(), "nothing is dealt or re-sealed");
            assert!(actor.ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH).is_none());
            let want = match actor.state(DETERMINISTIC_BOOTSTRAP_EPOCH) {
                Some(EpochState::Acquiring(Acquire::Logs(st))) => st.want.clone(),
                other => panic!("expected the heal, got {:?}", other.map(EpochState::name)),
            };
            assert_eq!(
                want.len(),
                pinned.len(),
                "an empty journal holds none of the pinned bodies: every one is wanted"
            );
            assert!(
                !journal_path(&dir, DETERMINISTIC_BOOTSTRAP_EPOCH).exists(),
                "deciding writes nothing"
            );

            let mut arng = StdRng::seed_from_u64(7);
            for (i, (dealer, signed)) in logs.iter().enumerate() {
                let key = DkgLogKey {
                    epoch: DETERMINISTIC_BOOTSTRAP_EPOCH,
                    dealer: dealer.clone(),
                    hash: log_hash(signed),
                };
                assert!(want.contains(&(dealer.clone(), key.hash)));
                assert!(
                    actor.ingest_log(&key, signed.encode(), &mut arng).await,
                    "the pinned body is accepted"
                );
                if i + 1 < logs.len() {
                    assert_eq!(
                        actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                        Some("acquiring_logs"),
                        "no recompute before every pinned body is held"
                    );
                }
            }
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("keyed"));
            let share = actor
                .store
                .read()
                .expect("store")
                .get(&DETERMINISTIC_BOOTSTRAP_EPOCH)
                .cloned()
                .expect("the absentee holds a share for the epoch");
            let outcome = crate::beacon::outcome::parse_outcome(&outcome_bytes).expect("outcome");
            assert!(
                validate_share_on_poly(&outcome, &committee, &me0, &share),
                "the recomputed share lies on the certified polynomial at node 0's index"
            );
            assert_eq!(actor.metrics.dkg_ceremony_ok.get(), 1);
            assert_eq!(actor.metrics.dkg_share_unrecoverable.get(), 0);
            assert!(actor.stalls(DETERMINISTIC_BOOTSTRAP_EPOCH).is_empty());
            assert!(
                !journal_path(&dir, DETERMINISTIC_BOOTSTRAP_EPOCH).exists(),
                "the journal the heal wrote is superseded by the share file"
            );
            actor.on_height(BOUNDARY, &mut arng).await;
            actor.on_height(BOUNDARY + 1, &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("keyed"));
            assert_eq!(actor.metrics.dkg_ceremony_ok.get(), 1);
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// The dealers hold node 0's acks, so no pinned body reveals its point.
    #[test]
    fn an_absentee_heal_over_an_acked_dealing_is_unrecoverable_not_a_loop() {
        let _guard = COLD_PARSE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(141);
            oracle.manager().track(0, committee.clone()).await;
            let info = info_for_test(&committee);
            let logs: Vec<(PeerPubkey, DealerReveal)> = journal
                .iter()
                .filter_map(|r| match r {
                    JournalRecord::OwnSeal(signed) | JournalRecord::PeerLog(signed) => {
                        let (pk, _) = (**signed).clone().check(&info).expect("checks");
                        Some((pk, (**signed).clone()))
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(logs.len(), 4, "every member sealed");
            let pinned: Vec<(u8, B256)> = logs
                .iter()
                .map(|(dealer, signed)| {
                    let idx = committee.iter().position(|p| p == dealer).expect("seat");
                    (u8::try_from(idx).expect("fits"), log_hash(signed))
                })
                .collect();
            let mut frng = StdRng::seed_from_u64(2);
            let mut canon = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                false,
                &BTreeMap::new(),
            )
            .expect("resume");
            let (outcome, _share) = canon
                .ceremony
                .finalize_over_pinned(
                    &mut frng,
                    &committee,
                    &pinned.iter().copied().collect::<BTreeMap<u8, B256>>(),
                )
                .expect("the canonical finalize");
            let outcome_bytes = crate::beacon::outcome::encode_outcome(&outcome);

            let dir = fresh_share_dir("absentee-acked");
            let resolver = RecordingResolver::default();
            let in_flight = resolver.in_flight.clone();
            let committee_for: CommitteeFor = {
                let set = committee.clone();
                Arc::new(move |_e: u64| Some(set.clone()))
            };
            let mut actor = standalone_actor_at(
                &oracle,
                key0,
                committee_for,
                Some(dir.clone()),
                INTERVAL,
                Wiring::inert(resolver),
            )
            .await;
            actor.last_height = Some(SEAL_DEADLINE);
            actor.outcome_at = artifact_reader(
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                pinned.clone(),
                &outcome_bytes,
            );
            let mut out = Vec::new();
            assert!(!actor.decide(DETERMINISTIC_BOOTSTRAP_EPOCH, &mut out).await);
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("acquiring_logs")
            );
            assert!(out.is_empty(), "nothing is dealt or re-sealed");

            let mut arng = StdRng::seed_from_u64(7);
            for (dealer, signed) in &logs {
                let key = DkgLogKey {
                    epoch: DETERMINISTIC_BOOTSTRAP_EPOCH,
                    dealer: dealer.clone(),
                    hash: log_hash(signed),
                };
                assert!(actor.ingest_log(&key, signed.encode(), &mut arng).await);
            }
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("unrecoverable")
            );
            assert_eq!(actor.metrics.dkg_share_unrecoverable.get(), 1);
            assert_eq!(actor.metrics.dkg_ceremony_ok.get(), 0);
            assert!(actor.store.read().expect("store").is_empty());
            assert_eq!(
                actor.stalls(DETERMINISTIC_BOOTSTRAP_EPOCH),
                BTreeSet::from([StallReason::Unrecoverable])
            );

            actor.on_height(BOUNDARY, &mut arng).await;
            actor.on_height(BOUNDARY + 1, &mut arng).await;
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("unrecoverable")
            );
            assert_eq!(actor.metrics.dkg_share_unrecoverable.get(), 1);
            assert!(
                !in_flight
                    .lock()
                    .expect("in flight")
                    .iter()
                    .any(|k| k.epoch == DETERMINISTIC_BOOTSTRAP_EPOCH),
                "no dealer log is fetched for an unrecoverable epoch"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// Torn journal before the seal: a node only ever seals at or after the deadline, so
    /// nothing was broadcast and the deterministic dealer re-deals over a fresh journal.
    #[test]
    fn recover_torn_journal_before_the_seal_redeals_over_a_fresh_journal() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, _journal) = node0_pre_seal_journal_full_sealed(103);
            oracle.manager().track(0, committee.clone()).await;
            let dir = fresh_share_dir("recover-torn-pre");
            write_torn_journal(&dir, DETERMINISTIC_BOOTSTRAP_EPOCH);
            let mut actor =
                restarted_actor(&oracle, key0, committee, dir.clone(), SEAL_DEADLINE - 1).await;
            assert!(matches!(
                actor.load_journal(DETERMINISTIC_BOOTSTRAP_EPOCH),
                JournalLoad::Torn
            ));
            let mut out = Vec::new();
            assert!(actor.decide(DETERMINISTIC_BOOTSTRAP_EPOCH, &mut out).await);
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("dealing"));
            assert!(
                matches!(
                    actor.load_journal(DETERMINISTIC_BOOTSTRAP_EPOCH),
                    JournalLoad::Present(_)
                ),
                "the torn file is replaced by a fresh, readable journal"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// A torn file stays `Torn` whatever is appended after it, so it must be evicted
    /// before the heal.
    #[test]
    fn recover_torn_journal_at_the_seal_deadline_evicts_it_and_heals_as_a_player() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let AbsentEpoch2 {
                committee,
                key0,
                pinned,
                outcome_bytes,
                ..
            } = node0_absent_epoch2_artifact(104);
            oracle.manager().track(0, committee.clone()).await;
            let dir = fresh_share_dir("recover-torn-post");
            write_torn_journal(&dir, DETERMINISTIC_BOOTSTRAP_EPOCH);
            let mut actor =
                restarted_actor(&oracle, key0, committee, dir.clone(), SEAL_DEADLINE).await;
            actor.outcome_at = artifact_reader(
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                pinned.clone(),
                &outcome_bytes,
            );
            let mut out = Vec::new();
            assert!(!actor.decide(DETERMINISTIC_BOOTSTRAP_EPOCH, &mut out).await);
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("acquiring_logs")
            );
            assert!(out.is_empty(), "nothing is dealt or re-sealed");
            assert!(
                matches!(
                    actor.load_journal(DETERMINISTIC_BOOTSTRAP_EPOCH),
                    JournalLoad::NoFile
                ),
                "the torn file is removed: the fetched bodies must land in a loadable journal"
            );
            let want = match actor.state(DETERMINISTIC_BOOTSTRAP_EPOCH) {
                Some(EpochState::Acquiring(Acquire::Logs(st))) => st.want.clone(),
                other => panic!("expected the heal, got {:?}", other.map(EpochState::name)),
            };
            assert_eq!(
                want.len(),
                pinned.len(),
                "a torn journal holds nothing: every pinned body is wanted"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// The journal-present restart cells: `Present, h < seal` resumes with a
    /// reconstructed dealer (`Dealing`); `Present, h ≥ seal` resumes player-only
    /// (`Sealed`) for an epoch not yet entered; an epoch already entered resumes
    /// into `Acquiring(ArtifactForCeremony)` — its agreement ran without this node,
    /// so the artifact is at the peers.
    #[test]
    fn recover_present_journal_by_the_clock() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal(105);
            oracle.manager().track(0, committee.clone()).await;
            let dir = fresh_share_dir("recover-present");
            std::fs::create_dir_all(&dir).expect("mkdir");
            for record in &journal {
                share_state::append_journal(
                    &dir,
                    DETERMINISTIC_BOOTSTRAP_EPOCH,
                    record,
                    &ShareState::Plaintext,
                )
                .expect("append");
            }
            for (height, expected, sends) in [
                (SEAL_DEADLINE - 1, "dealing", true),
                (SEAL_DEADLINE, "sealed", true),
                (BOUNDARY, "acquiring_artifact_for_ceremony", false),
            ] {
                let mut actor = restarted_actor(
                    &oracle,
                    key0.clone(),
                    committee.clone(),
                    dir.clone(),
                    height,
                )
                .await;
                let mut out = Vec::new();
                actor.decide(DETERMINISTIC_BOOTSTRAP_EPOCH, &mut out).await;
                assert_eq!(
                    actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                    Some(expected),
                    "at height {height}"
                );
                assert_eq!(
                    !out.is_empty(),
                    sends,
                    "a resume before the boundary re-emits its acks; past it, nothing"
                );
            }
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// The restart cells without a ceremony: a held share with the artifact is
    /// `Keyed`; a carry-forward epoch is `KeyOnly`; a non-member of a mint epoch is
    /// `KeyOnly` with the artifact and `Acquiring(ArtifactForKey)` without; an epoch
    /// whose committee cannot be read is not decided at all.
    #[test]
    fn recover_decides_the_share_key_and_membership_cells() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let mut rng = StdRng::seed_from_u64(106);
            let keys: Vec<Ed25519PrivateKey> = (0..5)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            // node 0 is a member of `members`, not of `others`.
            let members = Set::from_iter_dedup(keys[..4].iter().map(|k| k.public_key()));
            let others = Set::from_iter_dedup(keys[1..].iter().map(|k| k.public_key()));
            oracle.manager().track(0, members.clone()).await;
            let (outcome, shares) = crate::beacon::dkg_oracle::run_local_dkg(
                &mut rng,
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                &keys[..4],
                &keys[..4],
            )
            .expect("dkg");
            let outcome_bytes = crate::beacon::outcome::encode_outcome(&outcome);
            let logs: Vec<(u8, B256)> =
                (0..4u8).map(|i| (i, B256::repeat_byte(0x70 + i))).collect();

            let committee_for: CommitteeFor = {
                let (members, others) = (members.clone(), others.clone());
                Arc::new(move |e: u64| match e {
                    2 | 3 => Some(members.clone()),
                    4 => Some(others.clone()),
                    _ => None,
                })
            };
            let dir = fresh_share_dir("recover-cells");
            let mut actor =
                standalone_actor_cf(&oracle, keys[0].clone(), committee_for, Some(dir.clone()))
                    .await;
            // `now` = 4, so every epoch below is in `decide`'s window `[2, 5]`.
            actor.last_height = Some(INTERVAL * 4);
            // Epoch 3 carries forward; 2 and 4 mint.
            actor.changed = Arc::new(|e: u64| match e {
                2 | 4 => Some(true),
                3 => Some(false),
                _ => None,
            });
            let mut out = Vec::new();

            actor
                .store
                .write()
                .expect("store")
                .insert(2, shares[&keys[0].public_key()].clone());
            actor.outcome_at = artifact_reader(2, logs.clone(), &outcome_bytes);
            actor.decide(2, &mut out).await;
            assert_eq!(actor.phase(2), Some("keyed"));

            actor.decide(3, &mut out).await;
            assert_eq!(actor.phase(3), Some("key_only"));

            actor.decide(4, &mut out).await;
            assert_eq!(actor.phase(4), Some("acquiring_artifact_for_key"));
            let mut arng = StdRng::seed_from_u64(7);
            actor
                .on_artifact(agreed_artifact(4, logs.clone()), &mut arng)
                .await;
            assert_eq!(actor.phase(4), Some("key_only"));

            assert!(!actor.decide(5, &mut out).await);
            assert!(actor.phase(5).is_none());
            assert!(out.is_empty(), "none of these cells sends anything");
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// A sealed ceremony whose agreed set holds every pinned body and still has no
    /// selectable quorum is `Stalled{QuorumMissing}`: the ERROR line and the deferred
    /// counter fire once for the epoch, the latch stays, and the epoch is still
    /// `Agreed` (a reveal or a wider set could still complete it).
    #[test]
    fn a_below_quorum_agreed_set_stalls_the_epoch_once_with_quorum_missing() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(107);
            oracle.manager().track(0, committee.clone()).await;
            let me0 = key0.public_key();
            // Keep only node-0's own sealed log: every other dealer's body is absent.
            let own_only: Vec<JournalRecord> = journal
                .into_iter()
                .filter(|r| !matches!(r, JournalRecord::PeerLog(_)))
                .collect();
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                own_only,
                false,
                &BTreeMap::new(),
            )
            .expect("resume");
            let mut actor = standalone_actor(&oracle, key0, committee.clone(), None).await;
            actor.last_height = Some(SEAL_DEADLINE + 1);
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);
            // The one-dealer set holds every body this node has, but a quorum needs 3.
            let seat0 = committee.iter().position(|pk| *pk == me0).expect("seat") as u8;
            let own_hash = actor
                .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                .expect("ceremony")
                .signed_log_hash(&me0)
                .expect("own log");
            let mut arng = StdRng::seed_from_u64(9);
            actor
                .on_artifact(
                    agreed_artifact(DETERMINISTIC_BOOTSTRAP_EPOCH, vec![(seat0, own_hash)]),
                    &mut arng,
                )
                .await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("agreed"));
            assert!(actor
                .stalls(DETERMINISTIC_BOOTSTRAP_EPOCH)
                .contains(&StallReason::QuorumMissing));
            assert_eq!(actor.metrics.dkg_finalize_deferred.get(), 1);
            actor.drive_finalization(&mut arng);
            actor.on_height(SEAL_DEADLINE + 2, &mut arng).await;
            assert_eq!(actor.metrics.dkg_finalize_deferred.get(), 1);
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("agreed"));
            assert!(
                actor.store.read().expect("store").is_empty(),
                "below the quorum nothing is ever minted"
            );
        });
    }

    /// The adoption gate is the artifact's polynomial on the live path. The artifact
    /// pins the full set but carries a key derived over a different set (the three
    /// highest-seated dealers); the ceremony finalizes over the pinned four and its
    /// share lies on the four-dealer polynomial — off the certified one — so it is
    /// refused (`dkg_share_off_polynomial`), nothing is stored, and the epoch heals
    /// over the retained journal (`Acquiring(Logs)`).
    #[test]
    fn a_live_share_off_the_artifacts_polynomial_is_refused_not_adopted() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(108);
            oracle.manager().track(0, committee.clone()).await;
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                false,
                &BTreeMap::new(),
            )
            .expect("resume");
            let mut actor = standalone_actor(&oracle, key0, committee.clone(), None).await;
            actor.last_height = Some(SEAL_DEADLINE + 1);
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);
            let full = pinned_logs_of(&actor, DETERMINISTIC_BOOTSTRAP_EPOCH, &committee);
            assert_eq!(full.len(), 4);
            let mut arng = StdRng::seed_from_u64(9);
            let c = actor
                .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                .expect("ceremony");
            // `select` takes the lowest-keyed quorum, so the three-dealer set has
            // to omit the lowest seat to select differently from the full set.
            let key_over_three = key_over(c, &committee, &full[1..], &mut arng);
            let key_over_four = key_over(c, &committee, &full, &mut arng);
            assert_ne!(
                key_over_three.public().public(),
                key_over_four.public().public(),
                "the two selections must yield different polynomials or the test is vacuous"
            );
            actor
                .on_artifact(
                    agreed_artifact_keyed(DETERMINISTIC_BOOTSTRAP_EPOCH, full, key_over_three),
                    &mut arng,
                )
                .await;
            assert_eq!(actor.metrics.dkg_share_off_polynomial.get(), 1);
            assert_eq!(actor.metrics.dkg_ceremony_ok.get(), 0);
            assert!(actor.store.read().expect("store").is_empty());
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("acquiring_logs")
            );
        });
    }

    /// Two different quorum-certified artifacts for one epoch stop the epoch's
    /// signing: the share leaves the store, the phase is terminal, and
    /// `Stalled{Conflict}` is raised. A re-delivery of the held value is not a
    /// conflict.
    #[test]
    fn two_different_certified_artifacts_put_the_epoch_in_conflict() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(109);
            oracle.manager().track(0, committee.clone()).await;
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                false,
                &BTreeMap::new(),
            )
            .expect("resume");
            let mut actor = standalone_actor(&oracle, key0, committee.clone(), None).await;
            actor.last_height = Some(SEAL_DEADLINE + 1);
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);
            let mut arng = StdRng::seed_from_u64(9);
            let first = artifact_over(&actor, DETERMINISTIC_BOOTSTRAP_EPOCH, &committee, &mut arng);
            actor.on_artifact(first.clone(), &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("keyed"));

            actor.on_artifact(first.clone(), &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("keyed"));
            assert_eq!(actor.metrics.dkg_artifact_conflict.get(), 0);

            let mut three = first.0.logs.clone();
            three.pop();
            let second = agreed_artifact(DETERMINISTIC_BOOTSTRAP_EPOCH, three);
            assert_ne!(second.0.digest(), first.0.digest());
            actor.on_artifact(second.clone(), &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("conflict"));
            assert!(
                actor.store.read().expect("store").is_empty(),
                "the epoch's signing is stopped: the share leaves the store"
            );
            assert!(actor
                .stalls(DETERMINISTIC_BOOTSTRAP_EPOCH)
                .contains(&StallReason::Conflict));
            assert_eq!(actor.metrics.dkg_artifact_conflict.get(), 1);
            actor.on_artifact(first, &mut arng).await;
            actor.on_artifact(second, &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("conflict"));
            assert_eq!(actor.metrics.dkg_artifact_conflict.get(), 1);
        });
    }

    /// `Conflict` is reachable only through the artifact seam's quorum check: a
    /// second artifact whose certificate does not verify is rejected at the bridge
    /// (`deliver → false`), never handed to the write-back, and the actor stays
    /// `Keyed`.
    #[test]
    fn a_forged_second_artifact_never_reaches_the_actor() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(110);
            oracle.manager().track(0, committee.clone()).await;
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                false,
                &BTreeMap::new(),
            )
            .expect("resume");
            let mut actor = standalone_actor(&oracle, key0, committee.clone(), None).await;
            actor.last_height = Some(SEAL_DEADLINE + 1);
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);
            let mut arng = StdRng::seed_from_u64(9);
            let logs = pinned_logs_of(&actor, DETERMINISTIC_BOOTSTRAP_EPOCH, &committee);
            let key = key_over(
                actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .expect("ceremony"),
                &committee,
                &logs,
                &mut arng,
            );
            let (held, artifact_committee) =
                certify(DETERMINISTIC_BOOTSTRAP_EPOCH, logs.clone(), Some(key));
            actor.on_artifact(held.clone(), &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("keyed"));

            let committee_source: crate::beacon::artifact::CommitteeSource = Arc::new(move |e| {
                (e == DETERMINISTIC_BOOTSTRAP_EPOCH).then(|| artifact_committee.clone())
            });
            let store = crate::beacon::artifact::ArtifactStore::new();
            assert!(store
                .insert(DETERMINISTIC_BOOTSTRAP_EPOCH, held.clone())
                .is_ok());
            let (adopt_tx, mut adopt_rx) = tokio::sync::mpsc::channel(4);
            let bridge = crate::beacon::artifact::ArtifactBridge::new(
                AGREEMENT_CHAIN_ID,
                store,
                committee_source,
                adopt_tx,
                crate::beacon::metrics::BeaconMetrics::default(),
            );

            // Forged: the certificate verifies (it is signed by `certify`'s committee)
            // but does not cover the payload that was altered after signing.
            let mut three = logs.clone();
            three.pop();
            let mut forged = agreed_artifact(DETERMINISTIC_BOOTSTRAP_EPOCH, three.clone());
            forged.0.logs.pop();
            let served = crate::beacon::artifact::ArtifactResponse::Have(Box::new(forged)).encode();
            assert!(
                !bridge.deliver(DETERMINISTIC_BOOTSTRAP_EPOCH, &served),
                "a payload its certificate does not cover is rejected"
            );
            assert!(
                adopt_rx.try_recv().is_err(),
                "nothing unverified is handed to the write-back"
            );
            actor.on_artifact(held.clone(), &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("keyed"));
            assert_eq!(actor.metrics.dkg_artifact_conflict.get(), 0);

            // Contrast: the same divergent set under a certificate that verifies is
            // handed over and makes the epoch `Conflict`.
            let divergent = agreed_artifact(DETERMINISTIC_BOOTSTRAP_EPOCH, three);
            let served =
                crate::beacon::artifact::ArtifactResponse::Have(Box::new(divergent)).encode();
            assert!(bridge.deliver(DETERMINISTIC_BOOTSTRAP_EPOCH, &served));
            let handed = adopt_rx
                .try_recv()
                .expect("a verified divergent artifact reaches the write-back");
            actor.on_artifact(handed, &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("conflict"));
            assert_eq!(actor.metrics.dkg_artifact_conflict.get(), 1);
        });
    }

    /// The instance's body-lost verdict moves a `Sealed` epoch to acquiring the
    /// artifact from peers at once — asked on the signal and on every tick after —
    /// with `Stalled{BodyLost}` raised; the artifact arriving takes it through
    /// `Agreed` to `Keyed`. Nothing waits for the boundary.
    #[test]
    fn a_lost_body_moves_a_sealed_epoch_to_acquiring_and_asks_at_once() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(111);
            oracle.manager().track(0, committee.clone()).await;
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                false,
                &BTreeMap::new(),
            )
            .expect("resume");
            let mut actor = standalone_actor(&oracle, key0, committee.clone(), None).await;
            actor.last_height = Some(SEAL_DEADLINE + 1);
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);
            let asked: Arc<std::sync::Mutex<Vec<u64>>> = Arc::default();
            actor.pull_artifact = {
                let asked = asked.clone();
                Arc::new(move |epoch: u64| asked.lock().expect("asked").push(epoch))
            };
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("sealed"));

            actor.on_body_lost(DETERMINISTIC_BOOTSTRAP_EPOCH);
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("acquiring_artifact_for_ceremony")
            );
            assert_eq!(
                *asked.lock().expect("asked"),
                vec![DETERMINISTIC_BOOTSTRAP_EPOCH]
            );
            assert!(actor
                .stalls(DETERMINISTIC_BOOTSTRAP_EPOCH)
                .contains(&StallReason::BodyLost));
            let mut arng = StdRng::seed_from_u64(9);
            actor.on_height(SEAL_DEADLINE + 2, &mut arng).await;
            assert_eq!(
                asked.lock().expect("asked").len(),
                2,
                "asked again on the next tick, before the boundary"
            );
            // A second signal for an epoch no longer waiting on its instance is a no-op.
            actor.on_body_lost(DETERMINISTIC_BOOTSTRAP_EPOCH);
            assert_eq!(asked.lock().expect("asked").len(), 2);

            let artifact =
                artifact_over(&actor, DETERMINISTIC_BOOTSTRAP_EPOCH, &committee, &mut arng);
            actor.on_artifact(artifact, &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("keyed"));
            assert!(
                actor.stalls(DETERMINISTIC_BOOTSTRAP_EPOCH).is_empty(),
                "Keyed clears the epoch's latches"
            );
            actor.on_height(SEAL_DEADLINE + 3, &mut arng).await;
            assert_eq!(
                asked.lock().expect("asked").len(),
                2,
                "keyed: nothing more is asked"
            );
        });
    }

    /// Two independent ceremonies over one 4-member committee: `(outcome_a,
    /// shares_a)` and `outcome_b`, with `pk_a ≠ pk_b` asserted — a share of `a`
    /// is off `b`'s polynomial by construction, never by luck.
    fn two_polynomials(
        seed: u64,
    ) -> (
        Vec<Ed25519PrivateKey>,
        Set<PeerPubkey>,
        DkgOutcome,
        BTreeMap<PeerPubkey, Share>,
        DkgOutcome,
    ) {
        let mut rng = StdRng::seed_from_u64(seed);
        let keys: Vec<Ed25519PrivateKey> = (0..4)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let ns = b"FLUENT_DPOS_V1_clocktest";
        let (outcome_a, shares_a) =
            crate::beacon::dkg_oracle::run_local_dkg(&mut rng, ns, 2, &keys, &keys).expect("dkg a");
        let (outcome_b, _) =
            crate::beacon::dkg_oracle::run_local_dkg(&mut rng, ns, 2, &keys, &keys).expect("dkg b");
        assert_ne!(
            outcome_a.public().public(),
            outcome_b.public().public(),
            "two ceremonies, two polynomials — or the refusal below is vacuous"
        );
        (keys, committee, outcome_a, shares_a, outcome_b)
    }

    /// A member restarts holding a share written over artifact X, and the artifact
    /// that then arrives from a peer is X′ — a different polynomial. The share is
    /// not keyed over it: it is refused exactly as the live path refuses one
    /// (`dkg_share_off_polynomial`, `Stalled{OffPolynomial}`), it leaves the store
    /// and its file, and the epoch heals over its journal (`Acquiring(Logs)`).
    #[test]
    fn a_held_share_off_the_arriving_artifacts_polynomial_is_refused_not_keyed() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (keys, committee, outcome_a, shares_a, outcome_b) = two_polynomials(0x3B01);
            oracle.manager().track(0, committee.clone()).await;
            let me = keys[0].public_key();
            let dir = fresh_share_dir("held-share-off-poly");
            share_state::persist(
                &dir,
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                &shares_a[&me],
                &ShareState::Plaintext,
            )
            .expect("persist");
            let mut actor = standalone_actor(
                &oracle,
                keys[0].clone(),
                committee.clone(),
                Some(dir.clone()),
            )
            .await;
            for (epoch, share) in share_state::load_all(&dir, &ShareState::Plaintext) {
                actor.store.write().expect("store").insert(epoch, share);
            }
            let mut arng = StdRng::seed_from_u64(1);
            actor.on_height(BOUNDARY, &mut arng).await;
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("acquiring_artifact_for_share")
            );

            // X′: the same four dealer seats, a different polynomial.
            let served = crate::beacon::artifact::artifact_with_key(
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                outcome_b,
            );
            actor.on_artifact(served, &mut arng).await;
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("acquiring_logs"),
                "refused into the journal heal, never Keyed"
            );
            assert_eq!(actor.metrics.dkg_share_off_polynomial.get(), 1);
            assert!(actor
                .stalls(DETERMINISTIC_BOOTSTRAP_EPOCH)
                .contains(&StallReason::OffPolynomial));
            assert!(
                actor.store.read().expect("store").is_empty(),
                "the share over X leaves the store"
            );
            assert!(
                share_state::load_all(&dir, &ShareState::Plaintext).is_empty(),
                "and its file: a restart must not reload a share over another value"
            );
            let _ = outcome_a;
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// The restart cell: `recover(E)` with a share over X in the store and artifact
    /// X′ on disk refuses the share the same way — `(share, artifact) ⇒ Keyed` holds
    /// only for a share on the artifact's polynomial.
    #[test]
    fn recover_refuses_a_reloaded_share_off_the_stored_artifacts_polynomial() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (keys, committee, _outcome_a, shares_a, outcome_b) = two_polynomials(0x3B02);
            oracle.manager().track(0, committee.clone()).await;
            let me = keys[0].public_key();
            let dir = fresh_share_dir("recover-share-off-poly");
            let mut actor = standalone_actor(
                &oracle,
                keys[0].clone(),
                committee.clone(),
                Some(dir.clone()),
            )
            .await;
            actor.last_height = Some(BOUNDARY);
            actor
                .store
                .write()
                .expect("store")
                .insert(DETERMINISTIC_BOOTSTRAP_EPOCH, shares_a[&me].clone());
            let logs: Vec<(u8, B256)> =
                (0..4u8).map(|i| (i, B256::repeat_byte(0x70 + i))).collect();
            actor.outcome_at = artifact_reader(
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                logs,
                &crate::beacon::outcome::encode_outcome(&outcome_b),
            );
            let mut out = Vec::new();
            assert!(!actor.decide(DETERMINISTIC_BOOTSTRAP_EPOCH, &mut out).await);
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("acquiring_logs")
            );
            assert_eq!(actor.metrics.dkg_share_off_polynomial.get(), 1);
            assert!(actor
                .stalls(DETERMINISTIC_BOOTSTRAP_EPOCH)
                .contains(&StallReason::OffPolynomial));
            assert!(actor.store.read().expect("store").is_empty());
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// `Conflict` is durable: the verdict evicts the share file and writes a marker
    /// beside it, so a restart (`load_all` + `recover`) comes back `Conflict` with
    /// `Stalled{Conflict}` — never `Keyed` off a reloaded share.
    #[test]
    fn a_conflict_survives_a_restart() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(112);
            oracle.manager().track(0, committee.clone()).await;
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                false,
                &BTreeMap::new(),
            )
            .expect("resume");
            let dir = fresh_share_dir("conflict-restart");
            let mut actor =
                standalone_actor(&oracle, key0.clone(), committee.clone(), Some(dir.clone())).await;
            actor.last_height = Some(SEAL_DEADLINE + 1);
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);
            let mut arng = StdRng::seed_from_u64(9);
            let first = artifact_over(&actor, DETERMINISTIC_BOOTSTRAP_EPOCH, &committee, &mut arng);
            actor.on_artifact(first.clone(), &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("keyed"));
            let share = actor
                .store
                .read()
                .expect("store")
                .get(&DETERMINISTIC_BOOTSTRAP_EPOCH)
                .cloned()
                .expect("the keyed share");
            assert_eq!(
                share_state::load_all(&dir, &ShareState::Plaintext).len(),
                1,
                "the share file is on disk before the verdict"
            );
            let mut three = first.0.logs.clone();
            three.pop();
            let second = agreed_artifact(DETERMINISTIC_BOOTSTRAP_EPOCH, three);
            actor.on_artifact(second.clone(), &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("conflict"));
            assert!(
                share_state::load_all(&dir, &ShareState::Plaintext).is_empty(),
                "the durable share leaves with the terminal"
            );
            assert_eq!(
                share_state::load_conflict(&dir, DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some(ConflictMarker::Pair(
                    crate::beacon::artifact::value_digest(&first.0),
                    crate::beacon::artifact::value_digest(&second.0)
                )),
                "the verdict is on disk"
            );
            // The share file outlives the verdict here (an `evict_share` that failed),
            // so the share is put back beside the marker before the restart below:
            // the marker is what `recover` reads first.
            share_state::persist(
                &dir,
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                &share,
                &ShareState::Plaintext,
            )
            .expect("re-create the share file");
            assert_eq!(
                share_state::load_all(&dir, &ShareState::Plaintext).len(),
                1,
                "the share file is back beside the marker"
            );

            let mut restarted = standalone_actor(&oracle, key0, committee, Some(dir.clone())).await;
            for (epoch, share) in share_state::load_all(&dir, &ShareState::Plaintext) {
                restarted.store.write().expect("store").insert(epoch, share);
            }
            assert!(
                !restarted.store.read().expect("store").is_empty(),
                "the reloaded share is in RAM before the verdict is re-read"
            );
            restarted.last_height = Some(BOUNDARY);
            let mut out = Vec::new();
            assert!(
                !restarted
                    .decide(DETERMINISTIC_BOOTSTRAP_EPOCH, &mut out)
                    .await
            );
            assert_eq!(
                restarted.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("conflict")
            );
            assert!(restarted
                .stalls(DETERMINISTIC_BOOTSTRAP_EPOCH)
                .contains(&StallReason::Conflict));
            assert!(
                restarted.store.read().expect("store").is_empty(),
                "the marker is read FIRST: the reloaded share leaves `store`"
            );
            assert!(
                share_state::load_all(&dir, &ShareState::Plaintext).is_empty(),
                "and its file is evicted again"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// A bridge over `store` whose hand-off channel to the actor is full: what it
    /// delivers lands in the store and nowhere else.
    fn bridge_with_a_full_hand_off(
        store: crate::beacon::artifact::ArtifactStore,
        committee: fluentbase_bls::EpochCommittee,
    ) -> (
        crate::beacon::artifact::ArtifactBridge,
        tokio::sync::mpsc::Receiver<AgreedArtifact>,
    ) {
        let committee_source: crate::beacon::artifact::CommitteeSource =
            Arc::new(move |e| (e == DETERMINISTIC_BOOTSTRAP_EPOCH).then(|| committee.clone()));
        let (adopt_tx, adopt_rx) = tokio::sync::mpsc::channel(1);
        adopt_tx
            .try_send(agreed_artifact(99, vec![(0, B256::ZERO)]))
            .expect("fill the one slot");
        let bridge = crate::beacon::artifact::ArtifactBridge::new(
            AGREEMENT_CHAIN_ID,
            store,
            committee_source,
            adopt_tx,
            crate::beacon::metrics::BeaconMetrics::default(),
        );
        (bridge, adopt_rx)
    }

    /// The store is the owner of the fact: a pulled artifact whose hand-off to the
    /// actor is lost (the write-back mailbox is full) still keys the epoch on the next
    /// height tick, read from the store — through the same `Agreed → Keyed` edge,
    /// without the network.
    #[test]
    fn a_lost_artifact_hand_off_is_healed_from_the_store_on_the_next_tick() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(113);
            oracle.manager().track(0, committee.clone()).await;
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                false,
                &BTreeMap::new(),
            )
            .expect("resume");
            let mut actor = standalone_actor(&oracle, key0, committee.clone(), None).await;
            actor.last_height = Some(SEAL_DEADLINE + 1);
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);
            let store = crate::beacon::artifact::ArtifactStore::new();
            actor.outcome_at = store_reader(&store);
            let mut arng = StdRng::seed_from_u64(9);
            let logs = pinned_logs_of(&actor, DETERMINISTIC_BOOTSTRAP_EPOCH, &committee);
            let key = key_over(
                actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .expect("ceremony"),
                &committee,
                &logs,
                &mut arng,
            );
            let (artifact, artifact_committee) =
                certify(DETERMINISTIC_BOOTSTRAP_EPOCH, logs, Some(key));
            let (bridge, mut adopt_rx) =
                bridge_with_a_full_hand_off(store.clone(), artifact_committee);

            let served =
                crate::beacon::artifact::ArtifactResponse::Have(Box::new(artifact.clone()))
                    .encode();
            assert!(bridge.deliver(DETERMINISTIC_BOOTSTRAP_EPOCH, &served));
            assert!(
                store.has(DETERMINISTIC_BOOTSTRAP_EPOCH),
                "the store holds it"
            );
            assert_eq!(
                adopt_rx.try_recv().expect("the filler").0.target_epoch,
                99,
                "the hand-off was lost: only the filler was ever queued"
            );
            assert!(adopt_rx.try_recv().is_err());
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("sealed"));

            actor.on_height(SEAL_DEADLINE + 2, &mut arng).await;
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("keyed"),
                "the fact the store owns reaches the actor on the tick"
            );
            assert!(actor
                .store
                .read()
                .expect("store")
                .contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH));
            assert_eq!(actor.metrics.dkg_artifact_conflict.get(), 0);
        });
    }

    /// The store holds A and the actor stands on nothing yet; a certified B ≠ A
    /// pushed to the actor is `Conflict` against the store's value — the actor never
    /// finalizes over B while the node serves A as `PK_E`.
    #[test]
    fn a_second_value_against_the_stores_is_a_conflict_even_when_the_actor_holds_none() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(114);
            oracle.manager().track(0, committee.clone()).await;
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                false,
                &BTreeMap::new(),
            )
            .expect("resume");
            let mut actor = standalone_actor(&oracle, key0, committee.clone(), None).await;
            actor.last_height = Some(SEAL_DEADLINE + 1);
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);
            let store = crate::beacon::artifact::ArtifactStore::new();
            actor.outcome_at = store_reader(&store);
            let mut arng = StdRng::seed_from_u64(9);
            let a = artifact_over(&actor, DETERMINISTIC_BOOTSTRAP_EPOCH, &committee, &mut arng);
            assert!(store
                .insert(DETERMINISTIC_BOOTSTRAP_EPOCH, a.clone())
                .is_ok());
            let mut three = a.0.logs.clone();
            three.pop();
            let b = agreed_artifact(DETERMINISTIC_BOOTSTRAP_EPOCH, three);
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("sealed"));

            actor.on_artifact(b.clone(), &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("conflict"));
            assert!(matches!(
                actor.state(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some(EpochState::Conflict { held, second, .. })
                    if *held == crate::beacon::artifact::value_digest(&a.0)
                        && *second == crate::beacon::artifact::value_digest(&b.0)
            ));
            assert!(actor.store.read().expect("store").is_empty());
            assert_eq!(actor.metrics.dkg_artifact_conflict.get(), 1);
        });
    }

    /// The divergent value noted by the store: the bridge's hand-off of a second
    /// certified value is lost, and the epoch is still `Conflict` on the next tick,
    /// off the store's note.
    #[test]
    fn a_divergent_value_noted_by_the_store_reaches_conflict_without_the_hand_off() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(115);
            oracle.manager().track(0, committee.clone()).await;
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                false,
                &BTreeMap::new(),
            )
            .expect("resume");
            let mut actor = standalone_actor(&oracle, key0, committee.clone(), None).await;
            actor.last_height = Some(SEAL_DEADLINE + 1);
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);
            let store = crate::beacon::artifact::ArtifactStore::new();
            actor.outcome_at = store_reader(&store);
            let mut arng = StdRng::seed_from_u64(9);
            let logs = pinned_logs_of(&actor, DETERMINISTIC_BOOTSTRAP_EPOCH, &committee);
            let key = key_over(
                actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .expect("ceremony"),
                &committee,
                &logs,
                &mut arng,
            );
            let (a, artifact_committee) =
                certify(DETERMINISTIC_BOOTSTRAP_EPOCH, logs.clone(), Some(key));
            assert!(store
                .insert(DETERMINISTIC_BOOTSTRAP_EPOCH, a.clone())
                .is_ok());
            actor.on_artifact(a.clone(), &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("keyed"));

            let (bridge, _adopt_rx) =
                bridge_with_a_full_hand_off(store.clone(), artifact_committee);
            let mut three = logs;
            three.pop();
            let b = agreed_artifact(DETERMINISTIC_BOOTSTRAP_EPOCH, three);
            let served =
                crate::beacon::artifact::ArtifactResponse::Have(Box::new(b.clone())).encode();
            assert!(bridge.deliver(DETERMINISTIC_BOOTSTRAP_EPOCH, &served));
            assert!(
                store
                    .view(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .is_some_and(|(_, d)| d.is_some()),
                "the store noted the second value"
            );
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("keyed"));

            actor.on_height(SEAL_DEADLINE + 2, &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("conflict"));
            assert_eq!(actor.metrics.dkg_artifact_conflict.get(), 1);
        });
    }

    /// The store is the durable owner of the conflict witness: a second certified
    /// value it notes (a lost hand-off, no actor tick yet) is on disk the instant it
    /// is noted, so a restart before the actor's tick — fresh actor, fresh store RAM,
    /// the share file and the held artifact reloaded — decides the epoch `Conflict`,
    /// never `Keyed` off the reloaded share.
    #[test]
    fn a_second_value_noted_by_the_store_is_conflict_on_a_restart_before_the_tick() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(117);
            oracle.manager().track(0, committee.clone()).await;
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                false,
                &BTreeMap::new(),
            )
            .expect("resume");
            let dir = fresh_share_dir("store-witness-restart");
            let mut actor =
                standalone_actor(&oracle, key0.clone(), committee.clone(), Some(dir.clone())).await;
            actor.last_height = Some(SEAL_DEADLINE + 1);
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);
            let store =
                crate::beacon::artifact::ArtifactStore::new().with_conflict_dir(dir.clone());
            actor.outcome_at = store_reader(&store);
            let mut arng = StdRng::seed_from_u64(9);
            let logs = pinned_logs_of(&actor, DETERMINISTIC_BOOTSTRAP_EPOCH, &committee);
            let key = key_over(
                actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .expect("ceremony"),
                &committee,
                &logs,
                &mut arng,
            );
            let (a, artifact_committee) =
                certify(DETERMINISTIC_BOOTSTRAP_EPOCH, logs.clone(), Some(key));
            assert!(store
                .insert(DETERMINISTIC_BOOTSTRAP_EPOCH, a.clone())
                .is_ok());
            actor.on_artifact(a.clone(), &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("keyed"));
            assert_eq!(
                share_state::load_all(&dir, &ShareState::Plaintext).len(),
                1,
                "the share file is on disk"
            );

            // The second value reaches the store only (the hand-off is lost); the actor
            // never ticks again before the death.
            let (bridge, _adopt_rx) =
                bridge_with_a_full_hand_off(store.clone(), artifact_committee);
            let mut three = logs;
            three.pop();
            let b = agreed_artifact(DETERMINISTIC_BOOTSTRAP_EPOCH, three);
            let served =
                crate::beacon::artifact::ArtifactResponse::Have(Box::new(b.clone())).encode();
            assert!(bridge.deliver(DETERMINISTIC_BOOTSTRAP_EPOCH, &served));
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("keyed"));
            let (vd_a, vd_b) = (
                crate::beacon::artifact::value_digest(&a.0),
                crate::beacon::artifact::value_digest(&b.0),
            );
            assert_eq!(
                share_state::load_conflict(&dir, DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some(ConflictMarker::Pair(vd_a, vd_b)),
                "the store wrote the witness to disk as it noted the value"
            );
            drop(actor);

            // Restart before any tick: a fresh store with the artifact put back and the
            // marker reloaded, a fresh actor reloading the share file.
            let store = crate::beacon::artifact::ArtifactStore::new();
            assert!(store.insert(DETERMINISTIC_BOOTSTRAP_EPOCH, a).is_ok());
            let store = store.with_conflict_dir(dir.clone());
            assert_eq!(
                store.view(DETERMINISTIC_BOOTSTRAP_EPOCH).expect("held").1,
                Some(vd_b),
                "the reopened store knows the witness"
            );
            let mut restarted = standalone_actor(&oracle, key0, committee, Some(dir.clone())).await;
            for (epoch, share) in share_state::load_all(&dir, &ShareState::Plaintext) {
                restarted.store.write().expect("store").insert(epoch, share);
            }
            assert!(!restarted.store.read().expect("store").is_empty());
            restarted.outcome_at = store_reader(&store);
            restarted.last_height = Some(BOUNDARY);
            let mut out = Vec::new();
            assert!(
                !restarted
                    .decide(DETERMINISTIC_BOOTSTRAP_EPOCH, &mut out)
                    .await
            );
            assert!(
                matches!(
                    restarted.state(DETERMINISTIC_BOOTSTRAP_EPOCH),
                    Some(EpochState::Conflict { held, second, .. }) if *held == vd_a && *second == vd_b
                ),
                "restart before the tick: Conflict, not Keyed off the reloaded share \
                 (got {:?})",
                restarted.phase(DETERMINISTIC_BOOTSTRAP_EPOCH)
            );
            assert!(restarted
                .stalls(DETERMINISTIC_BOOTSTRAP_EPOCH)
                .contains(&StallReason::Conflict));
            assert!(restarted.store.read().expect("store").is_empty());
            assert!(share_state::load_all(&dir, &ShareState::Plaintext).is_empty());
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// The verdict's two durable steps are ordered marker-then-share, so a death
    /// between them leaves the marker beside a share file that outlived it: the
    /// restart reads the marker first, evicts the share and stands in `Conflict`.
    #[test]
    fn a_death_between_the_verdict_and_the_share_eviction_restarts_as_conflict() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(118);
            oracle.manager().track(0, committee.clone()).await;
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                false,
                &BTreeMap::new(),
            )
            .expect("resume");
            let dir = fresh_share_dir("conflict-death-between");
            let mut actor =
                standalone_actor(&oracle, key0.clone(), committee.clone(), Some(dir.clone())).await;
            actor.last_height = Some(SEAL_DEADLINE + 1);
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);
            let mut arng = StdRng::seed_from_u64(9);
            let first = artifact_over(&actor, DETERMINISTIC_BOOTSTRAP_EPOCH, &committee, &mut arng);
            actor.on_artifact(first.clone(), &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("keyed"));
            assert_eq!(share_state::load_all(&dir, &ShareState::Plaintext).len(), 1);

            actor.die_between_verdict_and_eviction = true;
            let mut three = first.0.logs.clone();
            three.pop();
            let second = agreed_artifact(DETERMINISTIC_BOOTSTRAP_EPOCH, three);
            actor.on_artifact(second.clone(), &mut arng).await;
            let (vd_first, vd_second) = (
                crate::beacon::artifact::value_digest(&first.0),
                crate::beacon::artifact::value_digest(&second.0),
            );
            assert_eq!(
                share_state::load_conflict(&dir, DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some(ConflictMarker::Pair(vd_first, vd_second)),
                "the marker is the FIRST durable step of the verdict"
            );
            assert_eq!(
                share_state::load_all(&dir, &ShareState::Plaintext).len(),
                1,
                "the share file outlived the death"
            );
            assert!(actor
                .store
                .read()
                .expect("store")
                .contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH));
            drop(actor);

            let mut restarted = standalone_actor(&oracle, key0, committee, Some(dir.clone())).await;
            for (epoch, share) in share_state::load_all(&dir, &ShareState::Plaintext) {
                restarted.store.write().expect("store").insert(epoch, share);
            }
            restarted.last_height = Some(BOUNDARY);
            let mut out = Vec::new();
            assert!(
                !restarted
                    .decide(DETERMINISTIC_BOOTSTRAP_EPOCH, &mut out)
                    .await
            );
            assert!(matches!(
                restarted.state(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some(EpochState::Conflict { held, second, .. })
                    if *held == vd_first && *second == vd_second
            ));
            assert!(
                restarted.store.read().expect("store").is_empty(),
                "the share the death left behind leaves with the verdict"
            );
            assert!(
                share_state::load_all(&dir, &ShareState::Plaintext).is_empty(),
                "and its file is evicted on the restart"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// A heal over a torn journal parks visibly (`Stalled{HealFailed}`, one gauge
    /// step) and spends no attempt: a failed load is not an attempt, so the next
    /// tick re-reads the file and the recompute keys the epoch once it is back.
    #[test]
    fn a_heal_over_a_torn_journal_stalls_visibly_and_runs_once_the_journal_is_back() {
        let _guard = COLD_PARSE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, full_journal) = node0_pre_seal_journal_full_sealed(119);
            let (_c2, _k2, journal_for_outcome) = node0_pre_seal_journal_full_sealed(119);
            oracle.manager().track(0, committee.clone()).await;
            let mut frng = StdRng::seed_from_u64(1);
            let mut canon = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal_for_outcome,
                false,
                &BTreeMap::new(),
            )
            .expect("resume");
            let pinned_canon: BTreeMap<u8, B256> = committee
                .iter()
                .enumerate()
                .filter_map(|(idx, pk)| canon.ceremony.signed_log_hash(pk).map(|h| (idx as u8, h)))
                .collect();
            let (outcome, canonical_share) = canon
                .ceremony
                .finalize_over_pinned(&mut frng, &committee, &pinned_canon)
                .expect("finalize");
            let outcome_bytes = crate::beacon::outcome::encode_outcome(&outcome);
            let dir = fresh_share_dir("heal-torn-journal");
            std::fs::create_dir_all(&dir).expect("mkdir");
            for r in full_journal {
                share_state::append_journal(&dir, 2, &r, &ShareState::Plaintext).expect("append");
            }
            let journal_file = std::fs::read_dir(&dir)
                .expect("dir")
                .flatten()
                .map(|e| e.path())
                .find(|p| p.to_string_lossy().contains("dkgjournal"))
                .expect("the journal file");
            let good = std::fs::read(&journal_file).expect("journal bytes");

            let mut actor =
                standalone_actor(&oracle, key0.clone(), committee.clone(), Some(dir.clone())).await;
            actor.outcome_at = artifact_reader(
                2,
                pinned_canon.iter().map(|(i, h)| (*i, *h)).collect(),
                &outcome_bytes,
            );
            actor.last_height = Some(BOUNDARY);
            let set = AgreedSet::of(&actor.stored(2).expect("the artifact").held);
            let heal = actor.heal_over(2, &committee, set);
            assert!(heal.want.is_empty(), "every pinned body is journaled");
            assert!(!heal.attempted);
            actor.epochs.insert(
                2,
                EpochSlot::new(EpochState::Acquiring(Acquire::Logs(Box::new(heal)))),
            );
            let mut arng = StdRng::seed_from_u64(9);

            // Torn under the heal: non-empty, first record unreadable.
            std::fs::write(&journal_file, [0xff, 0xff, 0xff, 0xff, 1, 2, 3]).expect("tear");
            actor.on_height(BOUNDARY + 1, &mut arng).await;
            assert_eq!(actor.phase(2), Some("acquiring_logs"));
            assert!(
                actor.stalls(2).contains(&StallReason::HealFailed),
                "the park is said and gauged"
            );
            assert_eq!(actor.metrics.stalled_gauge(StallReason::HealFailed), 1);
            assert_eq!(actor.metrics.dkg_ceremony_ok.get(), 0);
            actor.on_height(BOUNDARY + 2, &mut arng).await;
            assert_eq!(actor.phase(2), Some("acquiring_logs"));
            assert_eq!(
                actor.metrics.stalled_gauge(StallReason::HealFailed),
                1,
                "latched: one gauge step for the condition, however many ticks"
            );

            std::fs::write(&journal_file, &good).expect("restore");
            actor.on_height(BOUNDARY + 3, &mut arng).await;
            assert_eq!(
                actor.phase(2),
                Some("keyed"),
                "the unreadable journal did not spend the one attempt"
            );
            assert_eq!(actor.metrics.stalled_gauge(StallReason::HealFailed), 0);
            assert_eq!(actor.metrics.dkg_ceremony_ok.get(), 1);
            assert_eq!(
                actor.store.read().expect("store").get(&2),
                Some(&canonical_share),
                "the recompute over the pinned set is the canonical share"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// A conflict marker that does not read (not 64 bytes) is still the verdict —
    /// only a verdict ever writes one — so the epoch is `Conflict` with no known
    /// pair, and the file is left in place for the operator.
    #[test]
    fn a_malformed_conflict_marker_is_still_a_verdict() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, _journal) = node0_pre_seal_journal_full_sealed(120);
            oracle.manager().track(0, committee.clone()).await;
            let dir = fresh_share_dir("malformed-marker");
            share_state::persist_conflict(
                &dir,
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                &B256::repeat_byte(1),
                &B256::repeat_byte(2),
            )
            .expect("a marker");
            let marker = std::fs::read_dir(&dir)
                .expect("dir")
                .flatten()
                .map(|e| e.path())
                .find(|p| p.to_string_lossy().contains("conflict"))
                .expect("the marker file");
            std::fs::write(&marker, b"short").expect("damage it");
            let mut actor = standalone_actor(&oracle, key0, committee, Some(dir.clone())).await;
            actor.last_height = Some(BOUNDARY);
            let mut out = Vec::new();
            assert!(!actor.decide(DETERMINISTIC_BOOTSTRAP_EPOCH, &mut out).await);
            assert!(matches!(
                actor.state(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some(EpochState::Conflict { held, second, .. })
                    if *held == B256::ZERO && *second == B256::ZERO
            ));
            assert!(actor
                .stalls(DETERMINISTIC_BOOTSTRAP_EPOCH)
                .contains(&StallReason::Conflict));
            assert!(marker.exists(), "left in place, never deleted");
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// `enter` on a slot that already stands is a transition, so the latches it
    /// holds are kept — or dropped with their gauge step — by the same rule every
    /// transition uses, never dropped un-counted.
    #[test]
    fn entering_a_standing_slot_keeps_its_latches_counted() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, _journal) = node0_pre_seal_journal_full_sealed(121);
            oracle.manager().track(0, committee.clone()).await;
            let mut actor = standalone_actor(&oracle, key0, committee, None).await;
            actor.enter(3, EpochState::Unrecoverable { key: None });
            assert_eq!(actor.metrics.stalled_gauge(StallReason::Unrecoverable), 1);
            actor.enter(
                3,
                EpochState::Unrecoverable {
                    key: Some(B256::repeat_byte(7)),
                },
            );
            assert_eq!(
                actor.metrics.stalled_gauge(StallReason::Unrecoverable),
                1,
                "the standing latch is kept, not re-raised on a fresh slot"
            );
            assert_eq!(actor.stalls(3).len(), 1);
            actor.enter(3, EpochState::KeyOnly { digest: None });
            assert_eq!(
                actor.metrics.stalled_gauge(StallReason::Unrecoverable),
                0,
                "a phase that does not carry the latch drops it with its gauge step"
            );
            assert!(actor.stalls(3).is_empty());
        });
    }

    /// `Conflict` is judged by value: a second certificate over the same pinned set
    /// and key, differing only in the confirmation metadata a leader attached
    /// (`confirms`, part of `DkgProposal::digest`), is the held value again —
    /// first-wins, not a conflict.
    #[test]
    fn two_certificates_over_one_value_are_one_artifact_not_a_conflict() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(116);
            oracle.manager().track(0, committee.clone()).await;
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                false,
                &BTreeMap::new(),
            )
            .expect("resume");
            let mut actor = standalone_actor(&oracle, key0.clone(), committee.clone(), None).await;
            actor.last_height = Some(SEAL_DEADLINE + 1);
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);
            let mut arng = StdRng::seed_from_u64(9);
            let first = artifact_over(&actor, DETERMINISTIC_BOOTSTRAP_EPOCH, &committee, &mut arng);
            actor.on_artifact(first.clone(), &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("keyed"));

            let mut same_value = first.clone();
            same_value.0.confirms = vec![ShareConfirm::sign(
                b"FLUENT_DPOS_V1_clocktest",
                &key0,
                0,
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                first.0.logs.clone(),
            )];
            assert_ne!(
                same_value.0.digest(),
                first.0.digest(),
                "the certificates certify different PAYLOADS"
            );
            assert_eq!(
                crate::beacon::artifact::value_digest(&same_value.0),
                crate::beacon::artifact::value_digest(&first.0),
                "over one VALUE"
            );
            actor.on_artifact(same_value, &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("keyed"));
            assert_eq!(actor.metrics.dkg_artifact_conflict.get(), 0);
            assert!(actor
                .store
                .read()
                .expect("store")
                .contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH));
        });
    }

    /// A live share refused as off the certified polynomial raises
    /// `Stalled{OffPolynomial}` once and heals over the journal once: the recompute
    /// over every pinned body yields the same off-polynomial share — `Unrecoverable`,
    /// not a per-tick error or a re-run of the crypto until the sweep.
    #[test]
    fn a_refused_live_share_heals_once_then_is_unrecoverable_not_a_per_tick_error() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(117);
            oracle.manager().track(0, committee.clone()).await;
            let dir = fresh_share_dir("off-poly-once");
            std::fs::create_dir_all(&dir).expect("mkdir");
            for record in &journal {
                share_state::append_journal(
                    &dir,
                    DETERMINISTIC_BOOTSTRAP_EPOCH,
                    record,
                    &ShareState::Plaintext,
                )
                .expect("append");
            }
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                journal,
                false,
                &BTreeMap::new(),
            )
            .expect("resume");
            let mut actor =
                standalone_actor(&oracle, key0, committee.clone(), Some(dir.clone())).await;
            actor.last_height = Some(SEAL_DEADLINE + 1);
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);
            let full = pinned_logs_of(&actor, DETERMINISTIC_BOOTSTRAP_EPOCH, &committee);
            let mut arng = StdRng::seed_from_u64(9);
            let key_over_three = key_over(
                actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .expect("ceremony"),
                &committee,
                &full[1..],
                &mut arng,
            );
            actor
                .on_artifact(
                    agreed_artifact_keyed(DETERMINISTIC_BOOTSTRAP_EPOCH, full, key_over_three),
                    &mut arng,
                )
                .await;
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("acquiring_logs")
            );
            assert_eq!(actor.metrics.dkg_share_off_polynomial.get(), 1);
            assert!(actor
                .stalls(DETERMINISTIC_BOOTSTRAP_EPOCH)
                .contains(&StallReason::OffPolynomial));

            actor.on_height(SEAL_DEADLINE + 2, &mut arng).await;
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("unrecoverable")
            );
            assert_eq!(actor.metrics.dkg_share_off_polynomial.get(), 2);
            assert_eq!(actor.metrics.dkg_share_unrecoverable.get(), 1);
            assert_eq!(
                actor.stalls(DETERMINISTIC_BOOTSTRAP_EPOCH),
                BTreeSet::from([StallReason::Unrecoverable]),
                "the heal's latch leaves with the heal; the terminal carries its own"
            );
            actor.on_height(SEAL_DEADLINE + 3, &mut arng).await;
            actor.on_height(BOUNDARY, &mut arng).await;
            assert_eq!(actor.metrics.dkg_share_off_polynomial.get(), 2);
            assert_eq!(actor.metrics.dkg_share_unrecoverable.get(), 1);
            assert!(actor.store.read().expect("store").is_empty());
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// A zero-width deal window (`interval = DKG_MARGIN_BLOCKS`): epoch 2's seal
    /// deadline is epoch 1's first height, so the first tick at which epoch 2 is
    /// decidable is already at the deadline and heals rather than deals.
    #[test]
    fn at_a_zero_width_deal_window_a_missing_journal_at_the_deadline_heals_not_deals() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, _journal) = node0_pre_seal_journal_full_sealed(118);
            oracle.manager().track(0, committee.clone()).await;
            let dir = fresh_share_dir("zero-window");
            let committee_for: CommitteeFor = {
                let set = committee.clone();
                Arc::new(move |_e: u64| Some(set.clone()))
            };
            let mut actor = standalone_actor_at(
                &oracle,
                key0,
                committee_for,
                Some(dir.clone()),
                DKG_MARGIN_BLOCKS,
                Wiring::standalone(),
            )
            .await;
            let deadline = DKG_MARGIN_BLOCKS * DETERMINISTIC_BOOTSTRAP_EPOCH - DKG_MARGIN_BLOCKS;
            assert_eq!(
                actor.epoch_of(deadline),
                DETERMINISTIC_BOOTSTRAP_EPOCH - 1,
                "the deadline is the first height of the dealing epoch: a zero-width window"
            );
            let mut arng = StdRng::seed_from_u64(1);
            actor.on_height(deadline, &mut arng).await;
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("acquiring_artifact_for_share")
            );
            assert!(actor.ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH).is_none());
            assert!(
                !journal_path(&dir, DETERMINISTIC_BOOTSTRAP_EPOCH).exists(),
                "nothing was dealt"
            );
            assert!(
                actor.stalls(DETERMINISTIC_BOOTSTRAP_EPOCH).is_empty(),
                "before the boundary the wait for the artifact carries no latch"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// `resume(.., preferred)` stands the rebuilt player on the pinned body of a
    /// two-log dealer, not the first-recorded one. The observable is
    /// `Player::resume`'s integrity check: the first log acks this node for a dealing
    /// the journal no longer holds (`MissingPlayerDealing`), the second reveals this
    /// node instead — so the resume fails on the first body, succeeds on the pinned.
    #[test]
    fn a_resume_stands_on_the_pinned_body_of_a_two_log_dealer() {
        use commonware_cryptography::bls12381::dkg::Dealer;
        use commonware_utils::N3f1;
        const SEED: u64 = 119;
        let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(SEED);
        // The fixture's peer keys, re-derived from its seed (it hands back node-0's).
        let mut rng = StdRng::seed_from_u64(SEED);
        let keys: Vec<Ed25519PrivateKey> = (0..4)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        assert_eq!(keys[0].public_key(), key0.public_key());
        let d1 = keys[1].public_key();
        let info = info_for_test(&committee);

        // The dealer's sealed log (acks node 0), and a second valid log of its
        // own that acks nobody (reveals every player, node 0 included).
        let first = journal
            .iter()
            .find_map(|r| match r {
                JournalRecord::PeerLog(log)
                    if log.clone().check(&info).map(|(pk, _)| pk) == Some(d1.clone()) =>
                {
                    Some((**log).clone())
                }
                _ => None,
            })
            .expect("the dealer's sealed log");
        let (second_dealer, _, _) =
            Dealer::<
                commonware_cryptography::bls12381::primitives::variant::MinSig,
                Ed25519PrivateKey,
            >::start::<N3f1>(StdRng::seed_from_u64(0x5EC0), info, keys[1].clone(), None)
            .expect("second dealer");
        let second = second_dealer.finalize::<N3f1>();
        let (h1, h2) = (log_hash(&first), log_hash(&second));
        assert_ne!(h1, h2);

        // The journal as a restart finds it: the dealer's dealing to node 0 is gone,
        // and `pair` makes the dealer's two bodies its evidence pair, first-recorded
        // first, or leaves its first body alone — each shape is cut from a fresh copy
        // because `JournalRecord` is not `Clone`.
        let records = |pair: bool| -> Vec<JournalRecord> {
            node0_pre_seal_journal_full_sealed(SEED)
                .2
                .into_iter()
                .filter(|r| !matches!(r, JournalRecord::ReceivedDealing(d, ..) if *d == d1))
                .map(|r| match r {
                    JournalRecord::PeerLog(log) if pair && log_hash(&log) == h1 => {
                        JournalRecord::DealerEquivocation(log, Box::new(second.clone()))
                    }
                    other => other,
                })
                .collect()
        };
        let resume = |pair: bool, preferred: BTreeMap<PeerPubkey, B256>| {
            DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                records(pair),
                false,
                &preferred,
            )
        };
        assert!(
            matches!(
                resume(true, BTreeMap::new()),
                Err(DkgError::MissingPlayerDealing)
            ),
            "first-recorded: the dealer's ack of node 0 has no dealing behind it"
        );
        let resumed = resume(true, BTreeMap::from([(d1.clone(), h2)])).expect("pinned: the reveal");
        assert!(
            resumed.ceremony.holds(&(d1.clone(), h1)) && resumed.ceremony.holds(&(d1.clone(), h2))
        );
        assert!(
            matches!(
                resume(false, BTreeMap::from([(d1.clone(), h2)])),
                Err(DkgError::MissingPlayerDealing)
            ),
            "a preferred hash the journal does not hold falls back to first-recorded"
        );
        let _ = journal;
    }

    /// An artifact for an epoch outside `decide`'s window
    /// (`[max(2, now - JOURNAL_RETENTION_EPOCHS), now + 1]`) decides nothing and
    /// starts no ceremony — the store keeps it for the tick the epoch enters the
    /// window; one for `now + 1` is decided.
    #[test]
    fn an_artifact_beyond_the_decide_window_starts_nothing() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, _journal) = node0_pre_seal_journal_full_sealed(120);
            oracle.manager().track(0, committee.clone()).await;
            let mut actor = standalone_actor(&oracle, key0, committee, None).await;
            // A committee that changes at every epoch, so every epoch mints.
            actor.changed = Arc::new(|_e: u64| Some(true));
            actor.last_height = Some(BOUNDARY); // now = 2
            let logs: Vec<(u8, B256)> =
                (0..4u8).map(|i| (i, B256::repeat_byte(0x70 + i))).collect();
            let mut arng = StdRng::seed_from_u64(1);
            for far in [4u64, 5, 11] {
                actor
                    .on_artifact(agreed_artifact(far, logs.clone()), &mut arng)
                    .await;
                assert!(
                    actor.phase(far).is_none(),
                    "epoch {far} is beyond the window"
                );
                assert!(actor.ceremony(far).is_none());
            }
            actor.on_artifact(agreed_artifact(3, logs), &mut arng).await;
            assert_eq!(
                actor.phase(3),
                Some("dealing"),
                "now + 1 is the target this actor deals for"
            );
        });
    }

    /// The verdict's marker is fsync'd before the artifact write-behind lands, so a
    /// death in between restarts with the marker and an empty store: `recover` reads
    /// the marker first (`Conflict`) and, with nothing in the store, still pulls the
    /// epoch's artifact on the tick (`needs_artifact`) and takes it as its `key` — no
    /// share appears, the signing stays stopped.
    #[test]
    fn a_conflict_restarted_without_the_artifact_acquires_a_key_and_no_share() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let mut rng = StdRng::seed_from_u64(0xF001);
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;
            let (a, b) = (B256::repeat_byte(0xA1), B256::repeat_byte(0xB2));

            let dir = fresh_share_dir("conflict-keyless");
            std::fs::create_dir_all(&dir).expect("mkdir");
            share_state::persist_conflict(&dir, DETERMINISTIC_BOOTSTRAP_EPOCH, &a, &b)
                .expect("marker");
            let artifacts = crate::beacon::artifact::ArtifactStore::new();
            let asked: Arc<std::sync::Mutex<Vec<u64>>> = Arc::default();
            let mut wiring = Wiring::standalone();
            wiring.outcome_at = store_reader(&artifacts);
            wiring.pull_artifact = {
                let asked = asked.clone();
                Arc::new(move |epoch: u64| asked.lock().expect("asked").push(epoch))
            };
            let mut actor = standalone_actor_wired(
                &oracle,
                keys[0].clone(),
                committee.clone(),
                Some(dir.clone()),
                wiring,
            )
            .await;

            let mut arng = StdRng::seed_from_u64(9);
            actor.on_height(BOUNDARY, &mut arng).await;
            assert!(matches!(
                actor.state(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some(EpochState::Conflict { held, second, key: None })
                    if *held == a && *second == b
            ));
            assert_eq!(
                *asked.lock().expect("asked"),
                vec![DETERMINISTIC_BOOTSTRAP_EPOCH],
                "a keyless Conflict asks peers for the epoch's artifact"
            );
            let stalls = actor.stalls(DETERMINISTIC_BOOTSTRAP_EPOCH);
            assert!(stalls.contains(&StallReason::Conflict));
            assert!(stalls.contains(&StallReason::NoArtifact));

            // The pull seam files the value into the store; the hand-off to the actor's
            // channel may be lost, so the tick reads it back.
            let artifact = agreed_artifact(
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                vec![(0, B256::repeat_byte(0x11)), (1, B256::repeat_byte(0x22))],
            );
            let vd = crate::beacon::artifact::value_digest(&artifact.0);
            assert!(artifacts
                .insert(DETERMINISTIC_BOOTSTRAP_EPOCH, artifact)
                .is_ok());
            actor.on_height(BOUNDARY + 1, &mut arng).await;
            assert!(matches!(
                actor.state(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some(EpochState::Conflict { held, second, key: Some(key) })
                    if *held == a && *second == b && *key == vd
            ));
            assert_eq!(
                asked.lock().expect("asked").len(),
                1,
                "keyed: nothing more is asked"
            );
            let stalls = actor.stalls(DETERMINISTIC_BOOTSTRAP_EPOCH);
            assert!(
                stalls.contains(&StallReason::Conflict),
                "the verdict stands"
            );
            assert!(
                !stalls.contains(&StallReason::NoArtifact),
                "the artifact latch leaves with the key"
            );
            assert!(
                actor.store.read().expect("store").is_empty()
                    && share_state::load_all(&dir, &ShareState::Plaintext).is_empty(),
                "no share appears: the signing stays stopped"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// The local ban of a proven equivocator covers its dealings, not only its logs:
    /// with the evidence pair held for dealer 1, a `Commitment` from dealer 1 is
    /// refused at the consumer and counted (`equivocator`), an honest dealer's is not.
    /// Without the ban, a player that has not acked yet takes the dealer's second
    /// polynomial and its share lies off the pinned one.
    #[test]
    fn a_dealing_from_a_proven_equivocator_is_refused_and_counted() {
        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            let runtime = deterministic::Runner::default();
            runtime.start(|ctx| async move {
                let oracle = sim_oracle(&ctx);
                let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(57);
                oracle.manager().track(0, committee.clone()).await;
                let keys: Vec<Ed25519PrivateKey> = {
                    let mut rng = StdRng::seed_from_u64(57);
                    (0..4)
                        .map(|_| Ed25519PrivateKey::random(&mut rng))
                        .collect()
                };
                assert_eq!(keys[0].public_key(), key0.public_key());
                let d1 = keys[1].public_key();
                let info = info_for_test(&committee);
                let log1 = journal
                    .iter()
                    .find_map(|r| match r {
                        JournalRecord::PeerLog(l)
                            if l.clone().check(&info).is_some_and(|(pk, _)| pk == d1) =>
                        {
                            Some((**l).clone())
                        }
                        _ => None,
                    })
                    .expect("dealer 1's log is journaled");
                let log2: DealerReveal = {
                    use commonware_cryptography::bls12381::dkg::Dealer;
                    let (d, _, _) = Dealer::<_, Ed25519PrivateKey>::start::<N3f1>(
                        StdRng::seed_from_u64(0x5EC2),
                        info.clone(),
                        keys[1].clone(),
                        None,
                    )
                    .expect("second dealer");
                    d.finalize::<N3f1>()
                };
                assert_ne!(log_hash(&log1), log_hash(&log2));
                let dir = fresh_share_dir("equivocator-dealing");
                std::fs::create_dir_all(&dir).expect("mkdir");
                for r in journal {
                    share_state::append_journal(&dir, 2, &r, &ShareState::Plaintext)
                        .expect("append");
                }
                share_state::append_journal(
                    &dir,
                    2,
                    &JournalRecord::DealerEquivocation(Box::new(log1), Box::new(log2)),
                    &ShareState::Plaintext,
                )
                .expect("append evidence");
                let mut actor =
                    standalone_actor(&oracle, key0, committee.clone(), Some(dir.clone())).await;
                let mut arng = StdRng::seed_from_u64(9);
                actor.on_height(SEAL_DEADLINE + 1, &mut arng).await;
                assert!(actor
                    .evidence(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .contains_key(&d1));
                let _ = beacon_refusals(&snap, "equivocator");

                let ns = b"FLUENT_DPOS_V1_clocktest";
                let commitment = |dealer: &Ed25519PrivateKey| -> Bytes {
                    let (_cer, step) = DkgCeremony::start(ns, 2, committee.clone(), dealer.clone())
                        .expect("start");
                    let body = step
                        .outgoing
                        .into_iter()
                        .find_map(|o| match o.msg.body {
                            b @ DkgBody::Commitment(_) => Some(b),
                            _ => None,
                        })
                        .expect("a commitment dealing");
                    BeaconMessage::Dkg(
                        DkgMsg {
                            ceremony_epoch: 2,
                            body,
                        }
                        .encode(),
                    )
                    .encode()
                };
                actor
                    .on_message(d1.clone(), &commitment(&keys[1]), &mut arng)
                    .await;
                assert_eq!(
                    beacon_refusals(&snap, "equivocator"),
                    1,
                    "the proven equivocator's dealing is refused and counted"
                );
                actor
                    .on_message(keys[2].public_key(), &commitment(&keys[2]), &mut arng)
                    .await;
                assert_eq!(
                    beacon_refusals(&snap, "equivocator"),
                    0,
                    "an honest dealer's frame is not"
                );
                let _ = std::fs::remove_dir_all(&dir);
            });
        });
    }

    /// The equivocator ban also holds where a buffered dealing is consumed, not only
    /// on the live dispatch: after a restart over a journal holding
    /// `DealerEquivocation` for dealer 1, dealer 1's and an honest dealer's
    /// `Commitment` both arrive before the first tick and are buffered (no slot yet),
    /// and the tick that resumes `Dealing` and drains the buffer refuses dealer 1's.
    #[test]
    fn a_buffered_dealing_from_a_proven_equivocator_is_refused_at_the_drain() {
        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            let runtime = deterministic::Runner::default();
            runtime.start(|ctx| async move {
                let oracle = sim_oracle(&ctx);
                // Pre-seal journal (peers sealed, node-0 did not): a resume below
                // the seal deadline is `Dealing`, the one phase with a drain.
                let (committee, key0, journal) = node0_pre_seal_journal(58);
                oracle.manager().track(0, committee.clone()).await;
                let keys: Vec<Ed25519PrivateKey> = {
                    let mut rng = StdRng::seed_from_u64(58);
                    (0..4)
                        .map(|_| Ed25519PrivateKey::random(&mut rng))
                        .collect()
                };
                assert_eq!(keys[0].public_key(), key0.public_key());
                let d1 = keys[1].public_key();
                let info = info_for_test(&committee);
                let log1 = journal
                    .iter()
                    .find_map(|r| match r {
                        JournalRecord::PeerLog(l)
                            if l.clone().check(&info).is_some_and(|(pk, _)| pk == d1) =>
                        {
                            Some((**l).clone())
                        }
                        _ => None,
                    })
                    .expect("dealer 1's log is journaled");
                let log2: DealerReveal = {
                    use commonware_cryptography::bls12381::dkg::Dealer;
                    let (d, _, _) = Dealer::<_, Ed25519PrivateKey>::start::<N3f1>(
                        StdRng::seed_from_u64(0x5EC3),
                        info.clone(),
                        keys[1].clone(),
                        None,
                    )
                    .expect("second dealer");
                    d.finalize::<N3f1>()
                };
                assert_ne!(log_hash(&log1), log_hash(&log2));
                let dir = fresh_share_dir("equivocator-drain");
                std::fs::create_dir_all(&dir).expect("mkdir");
                for r in journal {
                    share_state::append_journal(&dir, 2, &r, &ShareState::Plaintext)
                        .expect("append");
                }
                share_state::append_journal(
                    &dir,
                    2,
                    &JournalRecord::DealerEquivocation(Box::new(log1), Box::new(log2)),
                    &ShareState::Plaintext,
                )
                .expect("append evidence");
                let mut actor =
                    standalone_actor(&oracle, key0, committee.clone(), Some(dir.clone())).await;
                let mut arng = StdRng::seed_from_u64(9);
                let _ = beacon_refusals(&snap, "equivocator");
                let _ = beacon_refusals(&snap, "no_seat");

                let ns = b"FLUENT_DPOS_V1_clocktest";
                let commitment = |dealer: &Ed25519PrivateKey| -> Bytes {
                    let (_cer, step) = DkgCeremony::start(ns, 2, committee.clone(), dealer.clone())
                        .expect("start");
                    let body = step
                        .outgoing
                        .into_iter()
                        .find_map(|o| match o.msg.body {
                            b @ DkgBody::Commitment(_) => Some(b),
                            _ => None,
                        })
                        .expect("a commitment dealing");
                    BeaconMessage::Dkg(
                        DkgMsg {
                            ceremony_epoch: 2,
                            body,
                        }
                        .encode(),
                    )
                    .encode()
                };
                // No tick yet, so no slot: both dealings buffer and nothing is refused
                // here — the evidence is on disk, not in a slot.
                actor
                    .on_message(d1.clone(), &commitment(&keys[1]), &mut arng)
                    .await;
                actor
                    .on_message(keys[2].public_key(), &commitment(&keys[2]), &mut arng)
                    .await;
                assert_eq!(
                    actor
                        .pending
                        .get(&DETERMINISTIC_BOOTSTRAP_EPOCH)
                        .map(|m| m.len()),
                    Some(2),
                    "both dealings raced ahead of the start and are buffered"
                );
                assert_eq!(beacon_refusals(&snap, "equivocator"), 0);

                actor.on_height(SEAL_DEADLINE - 1, &mut arng).await;
                assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("dealing"));
                assert!(actor
                    .evidence(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .contains_key(&d1));
                assert!(
                    !actor.pending.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH),
                    "the buffer was drained"
                );
                assert_eq!(
                    beacon_refusals(&snap, "equivocator"),
                    1,
                    "the proven equivocator's buffered dealing is refused at the drain, once — \
                     the honest dealer's is not"
                );
                assert_eq!(
                    beacon_refusals(&snap, "no_seat"),
                    0,
                    "both senders sit in the committee"
                );
                let _ = std::fs::remove_dir_all(&dir);
            });
        });
    }

    /// A body-lost signal while this node is still dealing is kept in the slot and
    /// applied at the seal: that tick enters `Acquiring(ArtifactForCeremony)`, stalls
    /// `BodyLost` and asks a peer for the artifact.
    #[test]
    fn a_body_lost_signal_while_dealing_is_applied_at_the_seal() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let mut rng = StdRng::seed_from_u64(0xF002);
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;
            let asked: Arc<std::sync::Mutex<Vec<u64>>> = Arc::default();
            let mut wiring = Wiring::standalone();
            wiring.pull_artifact = {
                let asked = asked.clone();
                Arc::new(move |epoch: u64| asked.lock().expect("asked").push(epoch))
            };
            let mut actor =
                standalone_actor_wired(&oracle, keys[0].clone(), committee, None, wiring).await;
            let mut arng = StdRng::seed_from_u64(9);
            actor.on_height(SEAL_DEADLINE - 1, &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("dealing"));

            actor.on_body_lost(DETERMINISTIC_BOOTSTRAP_EPOCH);
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("dealing"),
                "nothing to acquire for before the seal"
            );
            assert!(asked.lock().expect("asked").is_empty());

            actor.on_height(SEAL_DEADLINE, &mut arng).await;
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("acquiring_artifact_for_ceremony")
            );
            assert!(actor
                .stalls(DETERMINISTIC_BOOTSTRAP_EPOCH)
                .contains(&StallReason::BodyLost));
            assert_eq!(
                *asked.lock().expect("asked"),
                vec![DETERMINISTIC_BOOTSTRAP_EPOCH],
                "asked on the seal tick, before the boundary"
            );
        });
    }

    /// `BodyMissing` and `QuorumMissing` are two latches of `Agreed`, each on its own
    /// condition: a missing pinned body stalls `BodyMissing` and says nothing about the
    /// quorum, and once that body lands the latch leaves and `QuorumMissing` stands.
    #[test]
    fn the_body_missing_latch_leaves_when_every_pinned_body_is_held() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(131);
            oracle.manager().track(0, committee.clone()).await;
            let me0 = key0.public_key();
            // Only the own log is passed to the ceremony; one withheld peer log lands later.
            let mut withheld: Option<DealerReveal> = None;
            let own_only: Vec<JournalRecord> = journal
                .into_iter()
                .filter_map(|r| match r {
                    JournalRecord::PeerLog(l) => {
                        if withheld.is_none() {
                            withheld = Some(*l);
                        }
                        None
                    }
                    r => Some(r),
                })
                .collect();
            let withheld = withheld.expect("a peer log");
            let resumed = DkgCeremony::resume(
                b"FLUENT_DPOS_V1_clocktest",
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                key0.clone(),
                own_only,
                false,
                &BTreeMap::new(),
            )
            .expect("resume");
            let mut actor = standalone_actor(&oracle, key0, committee.clone(), None).await;
            actor.last_height = Some(SEAL_DEADLINE + 1);
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);
            let info = info_for_test(&committee);
            let (dealer, _) = withheld.clone().check(&info).expect("a valid peer log");
            let seat =
                |pk: &PeerPubkey| committee.iter().position(|p| p == pk).expect("seat") as u8;
            let own_hash = actor
                .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                .expect("ceremony")
                .signed_log_hash(&me0)
                .expect("own log");
            let mut arng = StdRng::seed_from_u64(9);
            // Two bodies pinned (below the 3-dealer quorum), one of them not held.
            actor
                .on_artifact(
                    agreed_artifact(
                        DETERMINISTIC_BOOTSTRAP_EPOCH,
                        vec![(seat(&me0), own_hash), (seat(&dealer), log_hash(&withheld))],
                    ),
                    &mut arng,
                )
                .await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("agreed"));
            assert_eq!(
                actor.stalls(DETERMINISTIC_BOOTSTRAP_EPOCH),
                BTreeSet::from([StallReason::BodyMissing])
            );
            assert_eq!(actor.metrics.stalled_gauge(StallReason::BodyMissing), 1);

            let key = DkgLogKey {
                epoch: DETERMINISTIC_BOOTSTRAP_EPOCH,
                dealer,
                hash: log_hash(&withheld),
            };
            assert!(actor.ingest_log(&key, withheld.encode(), &mut arng).await);
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("agreed"));
            assert_eq!(
                actor.stalls(DETERMINISTIC_BOOTSTRAP_EPOCH),
                BTreeSet::from([StallReason::QuorumMissing]),
                "the condition that passed leaves; the one that holds stands"
            );
            assert_eq!(actor.metrics.stalled_gauge(StallReason::BodyMissing), 0);
            assert_eq!(actor.metrics.stalled_gauge(StallReason::QuorumMissing), 1);
        });
    }

    /// A `ReceivedDealing` whose journal write failed is not lost with the withheld ack:
    /// it is queued and re-appended on the next publish edge until the write lands.
    #[test]
    fn a_dealing_whose_journal_write_failed_is_retried_on_the_next_edge() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let mut rng = StdRng::seed_from_u64(0xD808);
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;
            let good_dir = fresh_share_dir("dealing-retry-good");
            let bad_dir = fresh_share_dir("dealing-retry-bad");
            std::fs::write(&bad_dir, b"not a dir").expect("write file");
            let mut actor = standalone_actor(
                &oracle,
                keys[0].clone(),
                committee.clone(),
                Some(good_dir.clone()),
            )
            .await;
            let mut arng = StdRng::seed_from_u64(9);
            actor.on_height(SEAL_DEADLINE - 1, &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("dealing"));

            let ns = b"FLUENT_DPOS_V1_clocktest";
            let me0 = keys[0].public_key();
            let d1 = keys[1].public_key();
            let (_cer, step) = DkgCeremony::start(
                ns,
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                keys[1].clone(),
            )
            .expect("start");
            let frames: Vec<Bytes> = step
                .outgoing
                .into_iter()
                .filter(|o| match (&o.target, &o.msg.body) {
                    (Target::Broadcast, DkgBody::Commitment(_)) => true,
                    (Target::Direct(to), DkgBody::Share(_)) => *to == me0,
                    _ => false,
                })
                .map(|o| BeaconMessage::Dkg(o.msg.encode()).encode())
                .collect();
            assert_eq!(frames.len(), 2, "the commitment and my share");
            actor.share_dir = bad_dir.clone();
            for frame in &frames {
                actor.on_message(d1.clone(), frame, &mut arng).await;
            }
            assert_eq!(
                actor
                    .nondurable_dealings
                    .get(&DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .map(Vec::len),
                Some(1),
                "the failed ReceivedDealing is queued for a retry"
            );
            assert!(
                !journaled_dealings(&good_dir).contains(&d1),
                "and it really is absent from the journal"
            );

            actor.share_dir = good_dir.clone();
            actor.on_height(SEAL_DEADLINE - 1, &mut arng).await;
            assert!(
                actor.nondurable_dealings.is_empty(),
                "the retry landed and the queue is drained"
            );
            assert!(
                journaled_dealings(&good_dir).contains(&d1),
                "the peer's dealing is in the journal a restart replays"
            );
            let _ = std::fs::remove_dir_all(&good_dir);
            let _ = std::fs::remove_file(&bad_dir);
        });
    }

    /// The dealers of the bootstrap epoch's journaled `ReceivedDealing` records in `dir`,
    /// the list a restart's resume rebuilds `Player.view` from.
    fn journaled_dealings(dir: &std::path::Path) -> Vec<PeerPubkey> {
        let max = NonZeroU32::new(8).expect("nz");
        match share_state::load_journal(
            dir,
            DETERMINISTIC_BOOTSTRAP_EPOCH,
            &ShareState::Plaintext,
            max,
        ) {
            JournalLoad::Present(records) => records
                .into_iter()
                .filter_map(|r| match r {
                    JournalRecord::ReceivedDealing(d, _, _) => Some(d),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The start-race drain queues a failed `ReceivedDealing` as live dispatch does
    /// (`journal_or_defer`): a dealing buffered before the first tick is drained against
    /// a broken directory and retried on the next edge, once the directory is back.
    #[test]
    fn a_buffered_dealing_whose_drain_write_failed_is_retried_on_the_next_edge() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let mut rng = StdRng::seed_from_u64(0xD809);
            let keys: Vec<Ed25519PrivateKey> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;
            let good_dir = fresh_share_dir("drain-retry-good");
            let bad_dir = fresh_share_dir("drain-retry-bad");
            std::fs::write(&bad_dir, b"not a dir").expect("write file");
            let mut actor = standalone_actor(
                &oracle,
                keys[0].clone(),
                committee.clone(),
                Some(good_dir.clone()),
            )
            .await;
            let mut arng = StdRng::seed_from_u64(9);

            let ns = b"FLUENT_DPOS_V1_clocktest";
            let me0 = keys[0].public_key();
            let d1 = keys[1].public_key();
            let (_cer, step) = DkgCeremony::start(
                ns,
                DETERMINISTIC_BOOTSTRAP_EPOCH,
                committee.clone(),
                keys[1].clone(),
            )
            .expect("start");
            let frames: Vec<Bytes> = step
                .outgoing
                .into_iter()
                .filter(|o| match (&o.target, &o.msg.body) {
                    (Target::Broadcast, DkgBody::Commitment(_)) => true,
                    (Target::Direct(to), DkgBody::Share(_)) => *to == me0,
                    _ => false,
                })
                .map(|o| BeaconMessage::Dkg(o.msg.encode()).encode())
                .collect();
            assert_eq!(frames.len(), 2, "the commitment and my share");
            for frame in &frames {
                actor.on_message(d1.clone(), frame, &mut arng).await;
            }
            assert_eq!(
                actor
                    .pending
                    .get(&DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .map(|m| m.len()),
                Some(1),
                "dealer 1's dealing raced ahead of the start and is buffered"
            );

            actor.share_dir = bad_dir.clone();
            actor.on_height(SEAL_DEADLINE - 1, &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("dealing"));
            assert!(
                !actor.pending.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH),
                "the buffer was drained"
            );
            assert_eq!(
                actor
                    .nondurable_dealings
                    .get(&DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .map(Vec::len),
                Some(1),
                "the drained dealing's failed ReceivedDealing is queued for a retry"
            );

            actor.share_dir = good_dir.clone();
            actor.on_height(SEAL_DEADLINE - 1, &mut arng).await;
            assert!(
                actor.nondurable_dealings.is_empty(),
                "the retry landed and the queue is drained"
            );
            assert!(
                journaled_dealings(&good_dir).contains(&d1),
                "the peer's dealing is in the journal a restart replays"
            );
            let _ = std::fs::remove_dir_all(&good_dir);
            let _ = std::fs::remove_file(&bad_dir);
        });
    }
}

#[cfg(test)]
mod retain_floor_tests {
    use super::{ceremony_retain_floor, ChangedAt, CommitteeFor};
    use commonware_cryptography::ed25519::PublicKey as PeerPubkey;
    use commonware_utils::ordered::Set;
    use std::sync::Arc;

    const WINDOW: u64 = 8;

    #[test]
    fn single_old_mint_on_stable_committee_is_retained() {
        // The lone mint of a stable committee is still in force at now=1000: it is its own floor.
        assert_eq!(ceremony_retain_floor([3].into_iter(), 1000, WINDOW), 3);
    }

    #[test]
    fn churned_mints_keep_only_from_the_in_force_floor() {
        // now=20, cutoff=12: the greatest mint at or below the cutoff is 10, so 3 and 7 are
        // pruned and 10, 13, 19 are retained.
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
        // No mint is old enough to be a floor: 0 is returned and nothing is pruned.
        assert_eq!(ceremony_retain_floor([15, 18].into_iter(), 20, WINDOW), 0);
        assert_eq!(ceremony_retain_floor(std::iter::empty(), 20, WINDOW), 0);
    }

    /// A roster reader that answers differently for the same epoch cannot move the start
    /// decision, which reads the chain's `changed` bit; the roster only says who deals.
    #[test]
    fn a_straddling_roster_reader_cannot_move_the_start_decision() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let a = distinct_peer_set(0);
        let b = distinct_peer_set(1);

        let calls = Arc::new(AtomicUsize::new(0));
        let (sa, sb) = (a.clone(), b.clone());
        let c = calls.clone();
        let straddling: CommitteeFor = Arc::new(move |_epoch| {
            Some(if c.fetch_add(1, Ordering::SeqCst) == 0 {
                sa.clone()
            } else {
                sb.clone()
            })
        });
        assert_ne!(
            straddling(7),
            straddling(7),
            "premise: this reader really does straddle — the same epoch answers differently"
        );

        let changed: ChangedAt = Arc::new(|epoch| Some(epoch == 7));
        assert_eq!(
            changed(7),
            Some(true),
            "the contract recorded a change at 7"
        );
        assert_eq!(
            changed(8),
            Some(false),
            "and none at 8 — which is the carry-forward the old comparison could get \
             wrong under a straddle"
        );
    }

    fn distinct_peer_set(seed: u8) -> Set<PeerPubkey> {
        use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
        use commonware_math::algebra::Random as _;
        use rand_core::SeedableRng as _;
        let mut rng = rand_08::rngs::StdRng::seed_from_u64(0xC0FFEE + seed as u64);
        Set::from_iter_dedup((0..4).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()))
    }
}
