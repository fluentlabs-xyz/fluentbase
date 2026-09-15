//! Networked live-DKG actor: wraps [`DkgCeremony`] and drives committee[E]'s
//! self-DKG over `BEACON_CHANNEL` during epoch E-1.
//!
//! Single-ceremony-per-epoch, NO Muxer: each `DkgMsg` carries its `ceremony_epoch`.
//! A dealing (`Commitment`/`Share`) that arrives for a near-future epoch BEFORE this
//! node started its own ceremony for it is BUFFERED (`pending`, drained by
//! `recover`) so the start-race never silently drops it; any other message not for
//! an active ceremony is dropped (epoch-tag filter). Ceremonies for E and E+1 are
//! temporally disjoint (the collection window spans ~all of E-1), so at most a couple
//! are in flight.
//!
//! Lifecycle, driven by the finalized-height stream + chain committee reads:
//! - entering epoch E-1 (committee[E] != committee[E-1] AND this node ∈
//!   committee[E]) → `DkgCeremony::start`, broadcast commitment + send shares;
//! - finalized height reaches `epoch_start(E) - DKG_MARGIN_BLOCKS` → `seal_dealings`
//!   (broadcast the signed log);
//! - the epoch-key agreement plane certifies a dealer-log set for E and hands it
//!   back as an artifact; once every body it names is held and a quorum is
//!   selectable within it (probed event-driven on each recording, our seal or an
//!   incoming `Reveal`, via [`DkgActor::drive_finalization`]) →
//!   `DkgCeremony::finalize_over_pinned` → memoize `(PK_E, share)` into the per-epoch
//!   [`CeremonyStore`] + fire `share_notify`. The epoch manager's share-gate
//!   reads the entry to decide whether this node may run `E`'s engine, and
//!   Phase 5's finalized-boundary swap reads both for the per-epoch signing
//!   slot + `commitEpochBeaconKey`.
//!
//! The agreed set is the ONLY finalize input, so every honest node selects over the
//! IDENTICAL set ⇒ identical `PK_E`. The actor never finalizes before sealing, and
//! never over an under-quorum set (`pinned_ready` gates it). <quorum valid logs → no
//! store entry → the beacon naturally stalls for that epoch (option A), not a crash.
//!
//! Mid-window restart durability (§8.11.1): ceremony progress is journaled to
//! `beacon-dkgjournal-e<E>.bin`; on restart [`DkgActor::recover`] RESUMES via
//! `DkgCeremony::resume` — a PRE-seal restart (before the seal deadline) RE-DERIVES the
//! SEEDED dealer (`dealer_seed_rng`, byte-identical commitment) and keeps distributing;
//! an at/after-deadline restart is player-only + never re-seals — and re-fetches missing
//! peer logs via the DKG-log recovery resolver (`fetch_missing_logs`/`on_resolver_message`,
//! the `commonware_resolver::p2p` engine on `BEACON_RESOLVER_CHANNEL`), so a routine
//! restart no longer leaves the member shareless + liveness-slashed.
//!
//! Every phase an epoch can be in is ONE value of [`EpochState`], held per target
//! epoch in [`DkgActor::epochs`]; the transitions are the table in
//! `.dpos-study/history/E5-BEACON-DESIGN.md` §5.2 and each is a `set_state` /
//! `enter` in this file. The phase of an epoch is read by matching that value and
//! never by combining fields (5.3 заход А1).

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

/// The `channel` label every BEACON ingress refusal is counted under
/// (`dpos_ingress_dropped_total`).
pub(crate) const BEACON_CHANNEL_LABEL: &str = "beacon";

/// How far ABOVE this actor's own epoch clock a frame's ceremony epoch may lie
/// and still be worth a look: the ingress window is `[now, now + 2]`
/// ([`within_ingress_window`]), and every ingress rule on this actor reads it
/// from here — [`DkgActor::epoch_is_actionable`] (the cost gate before the
/// decode), [`DkgActor::is_bufferable`] (the start-race buffer, which adds its
/// own `epoch > now`) and [`DkgActor::on_confirm`] (the entry-bar window).
///
/// The window is about EPOCHS, not senders. ONE classification and ONE binding,
/// not one source: who may speak on the channel at all is classified once, by
/// the channel's pre-decode `GatedReceiver` (`crate::dpos::GatedReceiver`, over
/// the peer set this node's transition last registered); which SEAT a sender
/// holds in the frame's epoch is bound once, by the consumer — the ceremony's
/// roster for a dealing / ack / reveal, `committee[target_epoch]` for a
/// confirmation. The two read different things (the transition's assembled
/// window, which skips a neighbour record it could not read; the committed
/// record itself) and can disagree for a moment at a boundary — the gate is the
/// stricter of the two, and a dealing it refuses is re-sent. The actor asks a
/// third question of neither (5.3-В, second round: the per-epoch membership
/// check this actor used to run between the two was a third answer to "who is
/// this sender", and it disagreed with both on a lagging clock).
///
/// WHY 2 AND NOT 3 (R-126, closed by this record). The two epochs above `now` are
/// the ceremony this actor may still start (`now + 1`, `recover`) and the one
/// a peer ONE epoch ahead of it is already dealing for (`now + 2`). A peer two
/// epochs ahead would mean this actor's clock — the max over its three feeders,
/// `fin + K`, the upstream cert frontier and the marshal's ordering tip
/// (`on_height`) — lags the network by two whole epochs, which is the
/// frozen-tip / cold-start case, not a steady-state race; and nothing that a
/// frame for `now + 3` carries is lost by refusing it: a dealing is re-sent every
/// pre-seal tick (`DkgCeremony::retransmit`), a reveal is re-fetched by its pinned
/// hash (`fetch_missing_logs`), and a confirmation names an entry bar the peers
/// consume at THEIR agreement — two epochs before this clock could reach it, so
/// this node's count of it decides nothing.
pub(crate) const INGRESS_LOOKAHEAD_EPOCHS: u64 = 2;

/// THE ingress window: is `epoch` within `[now, now + INGRESS_LOOKAHEAD_EPOCHS]`?
/// Pure so the three rules that read it cannot drift apart. `now` is the caller's
/// epoch clock. Two of the callers ([`DkgActor::epoch_is_actionable`],
/// [`DkgActor::is_bufferable`]) read it off [`DkgActor::height_now`], which is
/// `0` before the first height tick — a floor of `[0, 2]` that is the honest
/// answer for them, since a dealing they refuse is re-sent and nothing else
/// they gate is one-shot. [`DkgActor::on_confirm`] alone refuses to call this
/// before its clock has started (it reads `last_height` itself), because a
/// confirmation refused there is never re-issued.
pub(crate) fn within_ingress_window(now: u64, epoch: u64) -> bool {
    (now..=now.saturating_add(INGRESS_LOOKAHEAD_EPOCHS)).contains(&epoch)
}

/// The epoch the beacon goes live at, deterministically. `committee[2]` runs its
/// DKG during epoch 1 EVEN IF unchanged from `committee[1]`, so a long-stable
/// initial committee still seeds the beacon (on-change-only activation would
/// leave it seedless indefinitely). Epoch 1 stays seedless (`order.digest()`);
/// on-change re-DKG + carry-forward apply thereafter. The same constant gates the
/// `application::is_change_epoch_first_block` boundary so the two never drift.
pub const DETERMINISTIC_BOOTSTRAP_EPOCH: u64 = 2;

/// What the artifact store holds for an epoch, as this actor reads it: the
/// certified payload it serves (`held` — the pinned dealer-log set and the group
/// `Output`), and the VALUE digest of the DIVERGENT second quorum-certified
/// value if the store ever saw one (`ArtifactStore::note_divergent`, durable
/// under the beacon directory) — the `Conflict` witness.
pub struct StoredArtifact {
    pub held: DkgProposal,
    pub divergent: Option<B256>,
}

/// Reads the agreement plane's artifact store for an EPOCH, threaded into the
/// actor as a READ handle (NOT a cross-actor push channel). Returns `Some` only
/// where an artifact for that exact epoch is held — i.e. a CHANGE epoch whose
/// agreement this node has the certified result of; `None` for a carry-forward
/// epoch (no fresh DKG) OR a store miss (retry next tick).
///
/// THE STORE IS THE OWNER of "the epoch's artifact" (§5.3): the actor reads it on
/// every height tick for every decided epoch ([`DkgActor::reconcile_with_store`]),
/// so an artifact whose push to the actor was lost is still applied, and a second
/// value the store noted is still a `Conflict`. Synchronous, because the store is
/// a RAM map behind a lock and every reader of it here runs inside `on_height`.
///
/// It used to read the boundary block at `epoch_start(E)`, which was a chicken-and-egg:
/// the heal exists for a member that could not enter `E`, and `E`'s own first block is
/// exactly what such a member's epoch does not produce. An epoch-keyed artifact,
/// certified before `E` starts, dissolves that.
pub type AgreedOutcomeAt = Arc<dyn Fn(u64) -> Option<StoredArtifact> + Send + Sync>;

/// Fire-and-forget request for the agreed artifact of an epoch this node needs
/// and does not hold.
///
/// Deliberately NOT a resolver: this actor's resolver is narrowed to [`DkgLogKey`]
/// by `LogFetcher` on purpose, and an artifact key has no business in that key
/// space. Deliberately NOT a future either — `ArtifactPull::pull` sleeps on a
/// per-epoch throttle and then waits out a timeout, and awaiting that inside
/// `drive_acquisition` would stall `on_height`, which drives every live ceremony.
/// The callee spawns and this returns immediately.
pub type PullArtifact = Arc<dyn Fn(u64) + Send + Sync>;

/// Per-epoch state of an in-flight demote-heal recompute: the pinned `Output` (the
/// `validate_share_on_poly` self-check target), the artifact's pinned set mapped onto
/// the committee (`dealer → hash`, the recompute's selection scope — the SAME input
/// the live finalize scopes to) and the pinned bodies this node still needs to fetch
/// (`want`, by exact `(dealer, hash)`, drained as the resolver delivers them).
/// Bounded: it is the payload of [`Acquire::Logs`], which exists ONLY for a
/// `committee[E]` member-epoch within the retention window, and leaves with the
/// epoch's `Keyed` / `Unrecoverable` transition or the sweep.
struct RecomputeState {
    outcome: DkgOutcome,
    pinned: BTreeMap<PeerPubkey, B256>,
    want: BTreeSet<LogId>,
    /// The value digest of the artifact this heal is scoped to — what a second
    /// artifact for the epoch is compared against ([`EpochState::Conflict`]).
    digest: B256,
    /// Whether the recompute has run over the CURRENT inputs. The journal
    /// recompute is deterministic on its inputs, so it runs once per change of
    /// them — a body landing in the journal (`ingest_recompute_log`) re-arms it;
    /// nothing else does. What used to re-run the crypto and re-emit the refusal
    /// line on every height tick (R-038, DB-03).
    attempted: bool,
}

/// One artifact-sourced pinned set: the finalize INPUT (`pinned`), the polynomial
/// every adopted share must lie on (`group_key`, the artifact's own — F-02: the
/// local ceremony's output is never the gate), and the VALUE digest a second
/// artifact for the epoch is compared against (`value_digest`: the set and the
/// key, not the certificate's `confirms` metadata — two certificates over one
/// value are one value).
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

/// What an [`EpochState::Acquiring`] epoch is waiting on. Each arm is a different
/// restart / failure shape of §5.2, and they are kept apart because they resolve
/// differently on the artifact's arrival ([`DkgActor::on_artifact`]).
enum Acquire {
    /// The instance certified a set whose body never arrived (`dkg_agree_body_lost`),
    /// or the epoch is being recovered past its boundary: the sealed ceremony waits
    /// for a PEER's copy of the artifact (`pull_artifact`, retried every tick).
    ArtifactForCeremony(Box<DkgCeremony>),
    /// Partial success (§5.4): the share file landed, the artifact's durable write
    /// did not. The share is in `store`; the key of the epoch this node signs in is
    /// nowhere local until a peer serves the artifact.
    ArtifactForShare,
    /// A non-member of a mint epoch, which needs the epoch's `PK_E` to verify its
    /// certificates (I4, R-121/R-122).
    ArtifactForKey,
    /// The pinned bodies the journal recompute still lacks (the demote-heal, and
    /// the retry behind a finalize `Err` / a refused adoption).
    Logs(Box<RecomputeState>),
}

/// The phase of ONE target epoch `E` (§5.2). Absent from [`DkgActor::epochs`] means
/// `Idle`: nothing decided yet (the committee or the `changed` bit unreadable, or the
/// epoch outside the window this actor looks at), re-asked on the next tick.
///
/// `Finalizing` is not a resting state — `finalize_over_pinned` is synchronous, so
/// an `Agreed` epoch is `Keyed`, `Acquiring(Logs)` or `Unrecoverable` by the time
/// `drive_finalization` returns. `Unfrozen` is the plane's, before this actor
/// exists (`plane.rs`, the geometry wait).
///
/// Every digest held here is the artifact's VALUE digest (`value_digest`).
enum EpochState {
    /// Not a phase: the slot's value is out for a by-value transition
    /// ([`DkgActor::take_state`]) and comes back with [`DkgActor::set_state`]
    /// before the input that took it returns ([`DkgActor::debug_assert_settled`]).
    /// A slot a reader finds in it is a bug, and it reads as nothing: no ceremony,
    /// no digest, no artifact wanted.
    InTransition,
    /// The dealer is live (start, or a pre-seal resume). `agreed` holds an
    /// artifact that arrived BEFORE this node's own seal (its clock lags the
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
    /// polynomial (F-02 — checked on EVERY way in: the live finalize, the
    /// journal recompute, a share file reloaded over an artifact, see
    /// [`DkgActor::key_held_share`]), and the artifact is `digest`.
    Keyed {
        digest: B256,
    },
    /// No share obligation: a carry-forward epoch (`digest: None`, the key in force
    /// is an earlier mint's) or a mint epoch this node is not a member of and holds
    /// the artifact for.
    KeyOnly {
        digest: Option<B256>,
    },
    /// Terminal. A damaged or absent journal at/after the seal deadline: this node
    /// may already have sealed and broadcast a log, so it never re-deals (R-036).
    /// `key` is the epoch's artifact once held — a sat-out member still needs
    /// `PK_E` to verify the epoch's certificates (I4), so it is acquired.
    SatOut {
        key: Option<B256>,
    },
    /// Terminal. The share is provably not derivable here: the journal acks a
    /// dealing this node no longer holds (`MissingPlayerDealing`), or the ceremony
    /// cannot be rebuilt/started over the committed roster. `key` as for `SatOut`.
    Unrecoverable {
        key: Option<B256>,
    },
    /// Terminal. Two DIFFERENT quorum-certified artifacts for one epoch reached
    /// this actor (`held` first). ≥ 2q−n Byzantine signers — the epoch's signing
    /// is stopped here (the share leaves `store` AND its file, and the verdict is
    /// on disk: `share_state::persist_conflict`, read back by `recover`).
    /// `key` is the artifact the STORE holds for the epoch, if any — a node in
    /// `Conflict` still verifies the epoch's certificates and needs `PK_E` for
    /// that (I4), so a `Conflict` the store holds no artifact for (the marker
    /// landed, the artifact's write-behind did not, then a death — F-01)
    /// acquires one from peers like every other keyless terminal; the signing
    /// stays stopped whatever arrives.
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
            Self::SatOut { .. } => "sat_out",
            Self::Unrecoverable { .. } => "unrecoverable",
            Self::Conflict { .. } => "conflict",
        }
    }

    /// The live ceremony this phase carries, if any.
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
            Self::SatOut { key } | Self::Unrecoverable { key } => *key,
            Self::Conflict { held, .. } => Some(*held),
            _ => None,
        }
    }

    /// Whether this phase is waiting on the epoch's artifact from a peer.
    fn needs_artifact(&self) -> bool {
        matches!(
            self,
            Self::Acquiring(
                Acquire::ArtifactForCeremony(_)
                    | Acquire::ArtifactForShare
                    | Acquire::ArtifactForKey
            ) | Self::SatOut { key: None }
                | Self::Unrecoverable { key: None }
                | Self::Conflict { key: None, .. }
        )
    }
}

/// Everything this actor keeps per target epoch: the phase, the `Stalled{reason}`
/// latches raised on it (one WARN/ERROR and one gauge step per `(epoch, reason)`,
/// dropped with the transition that leaves the condition behind — see
/// [`carries`]), the dealer-equivocation EVIDENCE proven for it (5.3-Б —
/// data beside the phase, never a phase: a two-log dealer costs that dealer a
/// gossip ban, not this epoch its signing), and whether the plane has accepted
/// the agreement announcement (the one-shot log line).
struct EpochSlot {
    state: EpochState,
    stalled: BTreeSet<StallReason>,
    evidence: BTreeMap<PeerPubkey, DealerEquivocation>,
    announced: bool,
    /// The instance's body-lost signal arrived while this node was still
    /// `Dealing` (its clock lags the network: the instance certified before the
    /// local seal). Nothing can act on it before the seal — there is no sealed
    /// ceremony to acquire for — so it is kept here and applied AT the seal:
    /// `Dealing{agreed: None}` seals into `Acquiring(ArtifactForCeremony)` and
    /// pulls at once, instead of `Sealed` waiting for the boundary (F-02).
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

/// What [`DkgActor::adopt_share`] refused a share for.
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

/// Whether `state` still carries the condition `reason` was raised for — the
/// rule [`DkgActor::set_state`] keeps a latch by. A latch names a condition of a
/// PHASE (bodies missing from an agreed set, an artifact wanted, a heal that
/// was refused), so it leaves with the phase and a gauge never counts a
/// condition an epoch has moved past; the three terminals carry their own.
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
        StallReason::SatOut => matches!(state, EpochState::SatOut { .. }),
        StallReason::Conflict => matches!(state, EpochState::Conflict { .. }),
    }
}

/// This node's SECRET SHARE per epoch it MINTED at, memoized by the actor during
/// the post-seal margin window — BEFORE the epoch-E boundary block is
/// proposed/verified. Read by the oracle on the vote path and by Phase 5's
/// signing-slot swap at the finalized boundary. Non-members never get an entry
/// (⇒ observer ⇒ withhold).
///
/// # It holds the share and NOT the group output (П-3)
///
/// It used to hold `(CeremonyOutput, Share)`, and the output half made this map the
/// second owner of `PK_E` and of the public polynomial beside
/// [`ArtifactStore`](crate::beacon::artifact::ArtifactStore) — which is the fact the
/// row moves. The polynomial that pairs with a share here is the one the epoch's
/// certified artifact carries, read through
/// [`KeyIndex`](crate::beacon::artifact::KeyIndex) at the SAME minting epoch this
/// map is keyed by; nothing needs a local copy of it, because a share that does not
/// lie on the artifact's polynomial is refused before it is ever stored
/// ([`DkgActor::adopt_share`]).
pub type CeremonyStore = Arc<RwLock<BTreeMap<u64, Share>>>;

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
/// **CORRECTNESS DEPENDS ON AN INVARIANT THIS FUNCTION CANNOT SEE**, so it is
/// written down here: for every epoch `e > DETERMINISTIC_BOOTSTRAP_EPOCH`, a mint
/// in the `CeremonyStore` implies `dkgQual[e]` is set.
///
/// The floor is "the highest mint at or below the cutoff", and everything strictly
/// below it is pruned. That is only safe while the highest such mint IS the one in
/// force. A mint under a CLEAR bit would break it: the chain's key epoch would
/// still name an older mint, the floor would sit above that mint, and the prune
/// would delete the key the node is supposed to be serving — `NoUsableMint`
/// forever, with no recompute path (the journal outlives one epoch) and no artifact
/// that carries a share. Committee-wide, that is zero signers at the next
/// boundary.
///
/// **Why the invariant holds** (established 2026-08-21, see the task's
/// `carry_research.md`): the contract sets the bit from `committee[target] !=
/// committee[target−1]` inside `commitEpochCommittee`, and the node starts a
/// ceremony from the SAME comparison over the SAME committed arrays. Consensus
/// keys are joined LIVE by address rather than frozen per epoch, so one state hash
/// yields one answer for both sides — the node's peer-key projection cannot see a
/// change the contract's address comparison does not.
///
/// **Two things would break it, and both are known:**
/// 1. Reading the two rosters at DIFFERENT states. That was reachable until the
///    ceremony-start decision moved onto the chain's `changed` bit (Д-7); a key rotation
///    between two reads fabricated a change. Do not reintroduce a per-epoch read
///    there.
/// 2. Making the bit mean "the DKG qualified" again rather than "the committee
///    changed", or freezing consensus keys per epoch. Either revives the state
///    immediately — the first is what the v47 incident was.
///
/// The `declined` arms that modelled the state this invariant excludes lived in
/// `beacon::carry` and went with it: the whole three-verdict arbitration
/// (`Serve`/`NoUsableMint`/`ReadFailed`) collapsed into
/// [`crate::beacon::artifact::MintIndex::minted_at`]'s flat `Option`, because "the
/// chain's mint is not local" and "the chain cannot be read yet" are one answer to
/// every caller — no key here, ask again.
fn ceremony_retain_floor(keys: impl Iterator<Item = u64>, now: u64, window: u64) -> u64 {
    let cutoff = now.saturating_sub(window);
    keys.filter(|&k| k <= cutoff).max().unwrap_or(0)
}

/// Resolves committee[epoch] (the Commonware-ordered peer set) at a finalized
/// state hash — provided by the launch site over the staking reader.
pub type CommitteeFor = Arc<dyn Fn(u64) -> Option<Set<PeerPubkey>> + Send + Sync>;

/// The artifact's pinned set (`idx → hash`, `idx` = position in `committee`) mapped
/// onto the dealers it names: `dealer → hash`. An `idx` with no position in
/// `committee` is skipped — the same deterministic skip
/// `DkgCeremony::scoped_pinned_logs` applies, since nothing can be asked for a seat
/// no roster has. THE one translation from "what the network pinned" to "which body
/// of which dealer", shared by the in-window fetch and the past-boundary recompute.
fn pinned_by_dealer(
    committee: &Set<PeerPubkey>,
    pinned: &BTreeMap<u8, B256>,
) -> BTreeMap<PeerPubkey, B256> {
    pinned
        .iter()
        .filter_map(|(idx, hash)| Some((committee.iter().nth(*idx as usize)?.clone(), *hash)))
        .collect()
}
/// THE epochs this actor decides on its clock `now`, ascending: the trailing
/// retention window `[max(BOOTSTRAP, now − R), now]` it may still owe a key or a
/// heal for, then the one target it may still deal for, `now + 1`. Stated once,
/// so the tick's walk (`decide_window`) and the artifact edge's gate
/// (`decidable`, via `decide`) cannot drift apart. Never further ahead: an
/// artifact for `now + 2` or beyond is a peer's clock running ahead of this one,
/// and starting that epoch's ceremony here would deal an epoch early over a
/// roster this node has not reached yet — the store keeps the artifact, and the
/// epoch is decided when it enters the window. Below BOOTSTRAP the beacon is
/// seedless (no share obligation, no artifact). `now + 1` is NOT clamped by the
/// bootstrap floor: at `now = 0` epoch 1 is decided — seedless, `KeyOnly`.
fn decidable_epochs(now: u64) -> impl Iterator<Item = u64> {
    let lo = now
        .saturating_sub(JOURNAL_RETENTION_EPOCHS)
        .max(DETERMINISTIC_BOOTSTRAP_EPOCH);
    (lo..=now).chain(std::iter::once(now.saturating_add(1)))
}

/// The dealers a ceremony step acks (`Target::Direct(dealer)` + `Ack`), read before
/// the step's journal is spent — the set the actor withholds from when that
/// journal write fails (step 1f).
fn acked_dealers(step: &Step) -> Vec<PeerPubkey> {
    step.outgoing
        .iter()
        .filter_map(|o| match (&o.target, &o.msg.body) {
            (Target::Direct(dealer), DkgBody::Ack(_)) => Some(dealer.clone()),
            _ => None,
        })
        .collect()
}

/// `recv()` on a plane edge, PARKING on a closed channel instead of answering
/// `None` on every poll: every sender of these edges is a supervised task of the
/// plane (an agreement instance, the write-back, the launcher), so a closed edge
/// is that task gone — a state the loop has nothing to do about, not an event to
/// spin on. The next iteration re-creates the future, sees the closed channel
/// again at once and parks again: one poll per loop turn, no wake-up.
async fn recv_or_park<T>(rx: &mut tokio::sync::mpsc::Receiver<T>) -> T {
    match rx.recv().await {
        Some(v) => v,
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

/// The actor's EDGES — every channel, handle and directory the beacon plane
/// connects a [`DkgActor`] by, all of them REQUIRED. There is no `None` and no
/// inert branch: the actor's behaviour is a function of its inputs, never of
/// which edges happened to be wired (E5-23 — the `DKG_SETTLE_BLOCKS` lesson, a
/// second time). Production builds the whole of it in `plane.rs`; the crate's
/// tests build it from `Wiring::standalone()` — parked receivers, a no-op pull,
/// a reader over an empty store, a scratch directory — so a test exercises the
/// same loop shape production runs.
pub struct Wiring<R> {
    /// Mailbox to the beacon-plane DKG-log recovery resolver — a shorthanded
    /// ceremony `fetch_targeted`s its missing dealer logs through it (replacing the
    /// former best-effort `BEACON_CHANNEL` `LogRequest` gossip pull).
    pub resolver: R,
    /// Inbound `Produce`/`Deliver` requests from the resolver engine
    /// (`log_resolver::LogHandler`), served against the live ceremonies + persisted
    /// journal in the single-threaded run loop. CLOSED is the resolver engine's
    /// death, and [`DkgActor::run`] stops on it (the engine is a supervised
    /// sibling; see the arm).
    pub resolver_rx: tokio::sync::mpsc::Receiver<LogMessage>,
    /// The chain's frozen `changed` bit, the ONE input of the ceremony-start
    /// decision (Д-7, `.dpos-study/DECISIONS.md`). It replaced the roster
    /// COMPARISON `recover` used to make (`committee[target] != committee[target−1]`
    /// through a `CommitteePairFor` reader): the contract writes the bit by that
    /// exact rule in the same `commitEpochCommittee` call that writes the
    /// committee, so reading it is reading the contract's own answer — which
    /// removes a class the comparison could not, two reads a beat apart seeing a
    /// change the contract never recorded. An unreadable bit (`None` from the
    /// read) leaves the epoch undecided for the tick, which is the honest answer;
    /// an actor without a chain reader does not exist.
    pub changed: ChangedAt,
    /// Directory for on-disk persistence of the live-DKG per-epoch shares, journals
    /// and conflict markers — the always-on plane passes `<datadir>/beacon/` (see
    /// `node/dpos.rs::build_beacon_plane`), reloaded once at plane startup.
    pub share_dir: PathBuf,
    /// The registered clock pair whose DKG half this actor publishes, off the
    /// monotone clamp in [`DkgActor::on_height`] — the single point every feeder's
    /// height lands at.
    pub plane_clock: PlaneClock,
    /// READ handle for the agreed artifact's payload (a pull, not a push channel),
    /// the artifact input of [`DkgActor::recover`] and of every tick's
    /// `reconcile_with_store`.
    pub outcome_at: AgreedOutcomeAt,
    /// Asks a peer for the agreed artifact of an epoch this node needs and does not
    /// hold. See [`PullArtifact`]. Driven by every `needs_artifact()` slot on each
    /// height tick ([`DkgActor::drive_acquisition`]) — the member without a share
    /// (body lost, or past the boundary), the member with a share and no artifact,
    /// the non-member that needs `PK_E` to verify, and a `Conflict` that has no
    /// artifact in the store — and for the LIVE epoch nothing else ever asks (the
    /// epoch-manager's repair sweep excludes `epoch >= frontier` by design).
    pub pull_artifact: PullArtifact,
    /// The shared `epoch -> idx -> keccak256(SignedDealerLog)` index this actor
    /// PUBLISHES for the agreement plane to propose over and for
    /// share-confirmations to state (it used to feed the consensus propose path,
    /// which carried the set in `OrderBlock.dkg_logs`; the block no longer carries
    /// it and every reader is local). The SAME handle on both sides: this actor
    /// writes it (`publish_recorded_logs`, which owns the per-`(dealer, hash)`
    /// durability gate) and [`Confirmations`] reads it — and the SAME index the
    /// confirmations are minted from, so a confirmation can never name a set the
    /// proposal path would not.
    pub recorded_dkg_logs: DkgLogIndex,
    /// The share-confirmation pool the epoch-key agreement's entry bar counts. It
    /// carries the signing namespace, so the actor and the agreement instances
    /// cannot disagree about it — hand BOTH the same pool.
    pub confirms: ConfirmPool,
    /// Inbound pinned-set questions from the epoch-key agreement instances
    /// ([`PinnedMailbox`]).
    pub pinned_rx: tokio::sync::mpsc::Receiver<PinnedRequest>,
    /// Announcement sink for the epoch-key agreement plane's spawn edge: a target
    /// epoch whose ceremony has CLOSED ITS DEALING here, which is the earliest
    /// point at which this node has a dealer-log set worth agreeing. The plane
    /// (node crate) owns the sub-channel registration and the instance itself;
    /// this actor owns the only state that knows when the edge happened.
    ///
    /// Not the `seal_dealings` call specifically: a node that restarted at or
    /// after the seal deadline resumes PLAYER-ONLY and never seals, and it still
    /// has to run the agreement for the epoch. `dealing_closed()` covers both.
    pub agreement_tx: tokio::sync::mpsc::Sender<u64>,
    /// Agreed artifacts arriving from the plane — this node's own instance, or a
    /// peer's artifact that already verified against `committee[epoch]`.
    ///
    /// PRECONDITION: every artifact on this channel has been checked against the
    /// target epoch's committee. Both producers do it (the instance only ever
    /// delivers a value its own quorum certified; the pull seam verifies before
    /// it stores), and this actor cannot re-check it — it reads peer identities,
    /// never the BLS committee the certificate is verified under.
    pub artifacts_rx: tokio::sync::mpsc::Receiver<AgreedArtifact>,
    /// Target epochs whose agreement instance certified a payload it could not
    /// resolve the body of (`dkg_engine`, `dkg_agree_body_lost`): a `Sealed` epoch
    /// moves to `Acquiring(ArtifactForCeremony)` on it and asks peers at once
    /// (R-026); a still-`Dealing` epoch remembers it for its seal.
    pub body_lost_rx: tokio::sync::mpsc::Receiver<u64>,
    /// TEST ONLY: what a fixture-built wiring owns besides its edges — the far
    /// ends of its parked channels and its scratch directory — carried into the
    /// actor so they live exactly as long as it does. `None` from production
    /// (`plane.rs`), always `Some` from [`Wiring::inert`].
    #[cfg(test)]
    pub fixture: Option<Fixture>,
}

/// TEST ONLY: the half of a fixture wiring that is not an edge of the actor but
/// must outlive its construction — held IN the actor ([`DkgActor::new`] moves it
/// there), so nothing is leaked and nothing is closed early:
/// - the senders of the parked plane channels and the receiver of the
///   announcement channel: a `run` that sees a plane that never speaks, not one
///   that died (a closed `resolver_rx` stops the actor; a closed plane channel
///   parks its arm — `recv_or_park`);
/// - the scratch `share_dir` [`Wiring::inert`] made, removed when the actor
///   goes (a test that names its own directory is unaffected: this removes
///   only the directory it created).
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
/// passes the `BEACON_CHANNEL` halves) and over the DKG-log recovery resolver `R`
/// (the `commonware_resolver::p2p::Mailbox` in production; a no-op in unit tests).
/// Testable with mock channels.
pub struct DkgActor<Se, Re, R> {
    namespace: Vec<u8>,
    me_key: Ed25519PrivateKey,
    sender: Se,
    receiver: Re,
    /// [`Wiring::resolver`].
    resolver: R,
    /// [`Wiring::resolver_rx`].
    resolver_rx: tokio::sync::mpsc::Receiver<LogMessage>,
    /// The DEALING/QUAL/SERVE roster reader — resolves `committee[epoch]` as the
    /// CEREMONY participants (in production the committed-slot reader, since under the
    /// 2-epoch warm-up `committee[epoch]` is frozen a full epoch before its DKG runs):
    /// who deals, whose qual partials are roster-bound, whose logs are served/
    /// recomputed, and the `next` committee in `recover`.
    committee_for: CommitteeFor,
    /// [`Wiring::changed`].
    changed: ChangedAt,
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
    /// [`Wiring::share_dir`].
    share_dir: PathBuf,
    /// At-rest framing for the persisted shares: [`ShareState::Encrypted`] (the
    /// HKDF-derived seal key) on a keystore-mode validator, [`ShareState::Plaintext`]
    /// otherwise. Built from the `Option<ShareSealKey>` the plane derives at launch
    /// (gated on `--dpos.bls-keystore-path`). Shared with [`Self::log_store`], which
    /// re-parses the journals this framing writes — ONE instance, so the encrypted
    /// arm's seal key is never duplicated in memory.
    share_state: Arc<ShareState>,
    /// THE state machine: one [`EpochSlot`] per target epoch this actor has decided
    /// anything about, on the one retention window [`Self::sweep_epoch_state`]
    /// applies. Every per-epoch fact that used to be its own map — the live
    /// ceremony, the agreed set, the in-flight heal, the sit-out / unrecoverable
    /// verdicts, the report-once marks, the evidence pairs — is a variant or a
    /// field of the slot, so the phase of an epoch is one `match`.
    epochs: BTreeMap<u64, EpochSlot>,
    /// The CACHED + DURABLE tiers of the dealer-log serve: a bounded, positive-only
    /// copy of the recorded logs of a FINALIZED-but-not-yet-past-boundary epoch, plus
    /// the one-time journal parse behind it. Seeded eagerly at finalize (the no-restart
    /// path never touches disk) and lazily on a cold `serve_log` miss after a restart.
    /// Aged out at the boundary sweep on the SAME window as the journal, so a restart
    /// re-reads from disk with nothing to repopulate (R1 closed by construction).
    ///
    /// The LIVE ceremony tier is NOT in here — `serve_log` asks the epoch's slot first and
    /// only then falls through, so a ceremony's lifetime stays out of the serve store.
    log_store: DealerLogStore,
    /// Whether the one-shot startup journal reconcile has run. Driven on the actor's
    /// FIRST `on_height` tick (where the frozen epoch geometry is finally available),
    /// it deletes every boundary-passed journal off disk — reclaiming orphaned
    /// journals + stale at-rest secrets a finalize-then-restart-before-boundary left
    /// (the running sweep cannot, since the restart wiped the in-memory keys it scans).
    reconciled_journals: bool,
    /// Dealings (`Commitment`/`Share`) that arrived for an epoch BEFORE this node
    /// started its own ceremony for it — the start-race. Drained into the ceremony by
    /// `recover` before any seal, so a peer dealing that raced ahead of our start
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
    ///
    /// `None` until the FIRST tick is drained, and that is a distinct state, not a
    /// zero: the actor is constructed before the height poller's buffered tick is
    /// read (`beacon/plane.rs`), and `epoch_of(0)` would say "the chain is in epoch
    /// 0" about a chain that may be anywhere. Every deal/seal deadline below reads
    /// it through [`Self::height_now`], whose `0` floor only ever DELAYS an action
    /// (nothing is due below the first epoch boundary); the one place where the
    /// difference is load-bearing is [`Self::on_confirm`], which refuses a
    /// confirmation outside `[now, now+2]` and would otherwise drop, permanently
    /// and with no retransmit behind it, every confirmation that beat the first
    /// tick.
    last_height: Option<u64>,
    /// Logs (by `(dealer, hash)`) whose journal record failed to land, per target
    /// epoch. Excluded from [`Self::publish_recorded_logs`] — this node holds the
    /// bytes in memory but cannot back the claim across a restart. Retried from
    /// memory (not re-fetched: the bytes are already here) on every publish edge,
    /// and cleared on success.
    nondurable_logs: BTreeMap<u64, BTreeSet<LogId>>,
    /// `ReceivedDealing` records whose journal write failed, per target epoch
    /// (DB-08). Their ack was withheld on the same edge (step 1f) and stays
    /// withheld; what the retry buys is the RESUME — a restart rebuilds
    /// `Player.view` from the journal, and a dealing that never landed there is
    /// recovered only through the dealer's reveal. Retried on the same edge as
    /// [`Self::nondurable_logs`] (`retry_nondurable_journals`), from the record
    /// itself: unlike a log, a dealing is not re-derivable from the ceremony.
    nondurable_dealings: BTreeMap<u64, Vec<JournalRecord>>,
    /// [`Wiring::plane_clock`].
    plane_clock: PlaneClock,
    /// [`Wiring::outcome_at`].
    outcome_at: AgreedOutcomeAt,
    /// [`Wiring::pull_artifact`].
    pull_artifact: PullArtifact,
    /// [`Wiring::recorded_dkg_logs`].
    recorded_dkg_logs: DkgLogIndex,
    /// This node's share-confirmation accounting: the pool the epoch-key agreement's
    /// entry bar counts, and the memory of what width this node has already put on
    /// the wire per target epoch. Ceremony-free by construction — it reads the shared
    /// `recorded_dkg_logs` index, never the epoch slots — so the whole
    /// claimed-width policy lives in [`Confirmations`]. Its pool and index are
    /// [`Wiring::confirms`] and [`Wiring::recorded_dkg_logs`].
    confirmations: Confirmations,
    /// [`Wiring::pinned_rx`].
    pinned_rx: tokio::sync::mpsc::Receiver<PinnedRequest>,
    /// [`Wiring::agreement_tx`].
    agreement_tx: tokio::sync::mpsc::Sender<u64>,
    /// [`Wiring::artifacts_rx`].
    artifacts_rx: tokio::sync::mpsc::Receiver<AgreedArtifact>,
    /// [`Wiring::body_lost_rx`].
    body_lost_rx: tokio::sync::mpsc::Receiver<u64>,
    /// The `Output` of every share this actor adopted, for TESTS ONLY.
    ///
    /// It exists because the shared store no longer holds one: after П-3 the
    /// `CeremonyStore` keeps the share alone and the polynomial's owner is the
    /// epoch's artifact. Two assertions genuinely need the OUTPUT of the ceremony
    /// this node ran and cannot read it anywhere else — `Output::revealed()` (did
    /// the peers reveal this node's point, the step-1f ack rule) and the canonical
    /// `PK_E` a heal reproduced. `#[cfg(test)]` rather than a field with a
    /// production reader, so a production line that reaches for it does not compile.
    #[cfg(test)]
    adopted_outcomes: Arc<RwLock<BTreeMap<u64, CeremonyOutput>>>,
    /// TEST ONLY: a simulated death INSIDE [`Self::stop_signing`], between its two
    /// durable steps (the marker written, the share file not yet evicted) — the
    /// crash the marker-first order exists for. `true` makes `stop_signing`
    /// return right after the marker; the test then "restarts" over the directory.
    #[cfg(test)]
    die_between_verdict_and_eviction: bool,
    /// TEST ONLY: [`Wiring::fixture`], kept for the actor's life.
    #[cfg(test)]
    _fixture: Option<Fixture>,
}

impl<Se, Re, R> DkgActor<Se, Re, R>
where
    Se: Sender<PublicKey = PeerPubkey>,
    Re: Receiver<PublicKey = PeerPubkey>,
    R: Resolver<Key = DkgLogKey, PublicKey = PeerPubkey>,
{
    /// The actor's own state is built here; its every edge is `wiring`, whole.
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
            artifacts_rx,
            body_lost_rx,
            #[cfg(test)]
            fixture,
        } = wiring;
        // ONE `ShareState`, shared with the serve store: the encrypted arm carries the
        // HKDF-derived seal key, which has no business existing twice.
        let share_state = Arc::new(share_state);
        let log_store = DealerLogStore::new(
            namespace.clone(),
            committee_for.clone(),
            Some(share_dir.clone()),
            share_state.clone(),
        );
        // The SAME index handle on both sides: this actor writes it
        // (`publish_recorded_logs`, which owns the per-`(dealer, hash)` durability
        // gate) and `Confirmations` reads it; the SAME pool every agreement
        // instance holds.
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
    /// it — the ONE input the finalize path takes — with the group key the
    /// ceremony derives over that set, which is what the artifact would carry.
    ///
    /// Building a real artifact costs a quorum-signed agreement, which
    /// [`crate::beacon::dkg_agree`] and [`crate::beacon::dkg_engine`] cover directly;
    /// a test that only needs a ceremony to reach `finalize_over_pinned` wants the
    /// set, not the certificate.
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
    /// `sync_all`). Returns whether EVERY record was written DURABLY — the
    /// write-durably-before-ack gate (step 1f) uses this: an `Ack` paired
    /// with a `ReceivedDealing` record is broadcast ONLY when that record is durable, so
    /// a QUAL log can never record an ack this node cannot back with a durable view
    /// (which recompute would hit as an un-resurrectable `MissingPlayerDealing`). A
    /// write failure warns and returns `false`.
    ///
    /// It does NOT name which record failed, deliberately: the only identity that
    /// matters here is the `(dealer, hash)` a recorded log belongs to, and the ceremony
    /// already authenticated that and hands it back as [`Step::recorded_log`]. The two
    /// recording call sites attribute from there. The one record that has no such
    /// identity and is not re-derivable from the ceremony — a `ReceivedDealing` —
    /// is handed back by [`Self::journal_failures`] to the site that retries it.
    #[must_use]
    fn append_journal(&self, epoch: u64, records: Vec<JournalRecord>) -> bool {
        self.journal_failures(epoch, records).is_empty()
    }

    /// [`Self::append_journal`], handing back the records whose write did NOT land
    /// (in their order) so a caller that can retry them from memory does.
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

    /// Delete a finalized/swept epoch's ceremony journal (within-window scratch).
    fn evict_journal(&self, epoch: u64) {
        share_state::evict_journal(&self.share_dir, epoch);
    }

    /// Delete a swept epoch's `Conflict` marker: the terminal it records is an
    /// epoch's, and an epoch past the window has no slot to be terminal in.
    fn evict_conflict(&self, epoch: u64) {
        share_state::evict_conflict(&self.share_dir, epoch);
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
                req = self.resolver_rx.recv() => match req {
                    Some(msg) => self.on_resolver_message(msg, &mut rng).await,
                    // The resolver engine exited: the one holder of the sender is the
                    // `LogHandler` the plane hands to the resolver engine
                    // (`plane.rs`, `LogHandler::new(log_resolver_tx)` in `build`, handed
                    // to `open_artifact_seam`), and that engine is a supervised child of
                    // the plane (`("beacon_resolver", ..)` in the plane's
                    // `spawn_supervisor`), whose supervisor is one of the node's own
                    // supervised handles (`node/dpos.rs`, `("beacon", ..)`): the node
                    // goes down on its exit. So there is no "gossip-only" life to degrade
                    // into — what this arm used to do (clear the mailbox, keep the loop)
                    // described the microseconds between the engine's exit and the
                    // supervisor's reaction, and hid the exit behind a silent downgrade.
                    // Say it, and stop: `run` returning is what the plane's supervisor
                    // sees, and the actor's task is supervised too.
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
                // Answer an epoch-key agreement instance's question about a
                // candidate pinned set. A closed channel means every instance is
                // gone, so the branch parks (`recv_or_park`) rather than spinning.
                req = recv_or_park(&mut self.pinned_rx) => {
                    let verdict = self.derive_pinned(&req, &mut rng);
                    drop(req.response.send(verdict));
                },
                // The write-back edge. It is the ONE arm that does not depend on
                // the finalized-height stream, which is the whole point: with the
                // chain halted at `epoch_start(E+1)` the height clock stops, and
                // the artifact is what starts the epoch's key moving again.
                artifact = recv_or_park(&mut self.artifacts_rx) => {
                    self.on_artifact(artifact, &mut rng).await;
                },
                // The instance's other verdict: a certified body it could not
                // resolve. The heal is a pull from peers, started here and not at
                // the boundary (R-026).
                epoch = recv_or_park(&mut self.body_lost_rx) => self.on_body_lost(epoch),
            }
        }
    }

    // ---- the state machine's own accessors ---------------------------------

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

    /// Every epoch with a live ceremony, with it.
    fn ceremonies(&self) -> impl Iterator<Item = (u64, &DkgCeremony)> {
        self.epochs
            .iter()
            .filter_map(|(e, slot)| slot.state.ceremony().map(|c| (*e, c)))
    }

    /// TEST SUPPORT: decide `epoch` from a ceremony built by the test — `Dealing`
    /// while its dealer is live, `Sealed` once it has closed — the way `recover`
    /// would have from that ceremony's journal.
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

    /// TEST SUPPORT: the phase's name, for assertions on the state machine.
    #[cfg(test)]
    fn phase(&self, epoch: u64) -> Option<&'static str> {
        self.state(epoch).map(EpochState::name)
    }

    /// TEST SUPPORT: the `Stalled{reason}` latches raised on `epoch`.
    #[cfg(test)]
    fn stalls(&self, epoch: u64) -> BTreeSet<StallReason> {
        self.epochs
            .get(&epoch)
            .map(|slot| slot.stalled.clone())
            .unwrap_or_default()
    }

    /// TEST SUPPORT: the equivocation evidence held for `epoch`.
    #[cfg(test)]
    fn evidence(&self, epoch: u64) -> BTreeMap<PeerPubkey, DealerEquivocation> {
        self.epochs
            .get(&epoch)
            .map(|slot| slot.evidence.clone())
            .unwrap_or_default()
    }

    /// Decide `epoch`: insert its slot, say so once, and raise the latch a
    /// terminal carries. The one-shot "epoch decided" line replaces the
    /// per-epoch one-shot diagnostic mark: a slot is created exactly once. On a
    /// slot that already stands (no production caller does this — `decide`
    /// guards on `contains_key`) it is a transition, so the latches and the
    /// gauges they hold are kept by the rule every transition keeps them by
    /// ([`Self::set_state`]), never dropped un-counted.
    fn enter(&mut self, epoch: u64, state: EpochState) {
        let latch = match &state {
            EpochState::SatOut { .. } => Some(StallReason::SatOut),
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
        // A resumed ceremony's journaled evidence pairs come back with it; the
        // slot's copy is what outlives the ceremony (until the sweep).
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

    /// Move a decided `epoch` to `state`. The latches the new phase no longer
    /// carries ([`carries`]) are dropped with their gauge step — `Keyed` carries
    /// none, a terminal carries its own — so a re-stall of a reason the phase
    /// still carries stays one line, and a gauge never counts a condition the
    /// epoch has left. The plane's announcement mark leaves with the announced
    /// phase (`Sealed` / `Agreed`).
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

    /// Take `epoch`'s phase out for a by-value transition; the caller puts the
    /// next one back with [`Self::set_state`] before its input returns (the slot
    /// keeps its latches and evidence in between). What stands in meanwhile is
    /// [`EpochState::InTransition`], which reads as nothing and is checked gone
    /// at the end of every input ([`Self::debug_assert_settled`]).
    fn take_state(&mut self, epoch: u64) -> Option<EpochState> {
        self.epochs
            .get_mut(&epoch)
            .map(|slot| std::mem::replace(&mut slot.state, EpochState::InTransition))
    }

    /// Put a taken phase back UNCHANGED — the transition the caller took it for
    /// does not apply to what it found (a phase other than the one its plan
    /// selected). Not a transition: no line, no latch change.
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

    /// What the artifact store holds for `epoch`, through the read handle.
    fn stored(&self, epoch: u64) -> Option<StoredArtifact> {
        (self.outcome_at)(epoch)
    }

    /// Raise `Stalled{reason}` on `epoch` — the event of §5.2/§5.4. LATCHED per
    /// `(epoch, reason)`: one line (ERROR where the epoch cannot progress without
    /// peers acting — a quorum that is not there, a conflict — WARN otherwise), one
    /// step of the `dpos_dkg_stalled{reason}` gauge, until the epoch keys or is
    /// swept. The line is bounded, the gauge is not: an operator sees how many
    /// epochs are stalled and why without a per-tick flood.
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

    /// Drop one latch of `epoch` whose condition passed WITHIN the phase (the
    /// phase-leaving latches go through `set_state`). One gauge step down; a
    /// latch that is not raised is a no-op.
    fn clear_stall(&mut self, epoch: u64, reason: StallReason) {
        let Some(slot) = self.epochs.get_mut(&epoch) else {
            return;
        };
        if slot.stalled.remove(&reason) {
            self.metrics.stall_cleared(reason);
        }
    }

    /// The instance for `epoch` certified a payload whose body never arrived. A
    /// `Sealed` epoch keeps its ceremony and waits for a PEER's copy of the
    /// artifact, asked for at once and on every tick (`drive_acquisition`). A
    /// still-`Dealing` epoch (this node's clock lags the instance) remembers the
    /// signal in its slot and acts on it at its seal (F-02). Every other phase
    /// either already holds the artifact or never ran a ceremony, so the signal is
    /// noted and dropped.
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
        let Some(ceremony) = self.ceremony(req.epoch) else {
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
        if !self.epochs.contains_key(&epoch) {
            // Before the first height tick there is no clock to decide the epoch
            // by (a pre-seal resume would be wrong for an epoch already past its
            // deadline), and the artifact is already in the store every producer
            // inserts into before it sends — so the first tick's `recover` reads
            // it from there. A live epoch is decided here, then handed the set.
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
        // An epoch that could not be decided (undecidable yet, or outside the
        // window `decide` looks at) has no slot to take the set: the artifact is
        // in the store, and `reconcile_with_store` applies it on the tick the
        // epoch is decided.
        if self.apply_artifact(epoch, &artifact.0) {
            self.drive_finalization(rng);
            self.fetch_missing_logs().await;
        }
        self.debug_assert_settled();
    }

    /// The artifact edge of the transition table, on a decided epoch. Returns
    /// whether a ceremony now stands on the set (the caller finalizes / fetches).
    ///
    /// FIRST-WINS on an identical VALUE (`value_digest`), for the same reason
    /// `ArtifactStore::insert` is: one instance certifies exactly one value per
    /// target epoch, and re-pinning would swap the set out from under a finalize
    /// that is already fetching bodies for it. A DIFFERENT value is `Conflict` —
    /// the channel's precondition says both passed the committee-quorum check, so
    /// two of them is ≥ 2q−n Byzantine signers, and the epoch's signing stops
    /// here. The value this phase stands on is compared first; a phase standing on
    /// nothing yet is compared against the STORE's, which is first-wins and
    /// quorum-checked — so a hand-off this actor never received still bars the
    /// second value (the store is the owner of the fact).
    fn apply_artifact(&mut self, epoch: u64, proposal: &DkgProposal) -> bool {
        // ONE guard for both intake rails (the channel and the store read): an
        // artifact naming no dealer logs is nothing to finalize over. Not
        // reachable from a quorum-certified value (the agreement's entry bar and
        // `select` need a quorum of logs); kept so the two rails cannot diverge.
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
            // Both halves of the conflict are known; a re-delivery of either is
            // not a third value, and a third value changes nothing here.
            if digest != held && digest != second {
                tracing::warn!(
                    target: "dpos::beacon",
                    epoch,
                    third = %digest,
                    "live DKG: a third quorum-certified artifact for a conflicted epoch"
                );
            }
            // A `Conflict` without an artifact in the store takes the first one
            // that arrives as the key to VERIFY with (F-01): the signing stays
            // stopped — the phase does not change, only its `key`.
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
            // The held share is keyed over the artifact only if it lies on the
            // artifact's polynomial (F-02, the third way into `Keyed`). An
            // unreadable committee keeps the phase: the store holds the artifact
            // and the next tick re-applies it.
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
            // A sat-out / unrecoverable member still verifies with the key.
            EpochState::SatOut { key: None } => {
                (EpochState::SatOut { key: Some(digest) }, false, None)
            }
            EpochState::Unrecoverable { key: None } => {
                (EpochState::Unrecoverable { key: Some(digest) }, false, None)
            }
            // A phase already standing on a digest (handled above).
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

    /// Key `epoch` over a share this node already HOLDS (`store` — a share file
    /// reloaded at launch, or one kept through the §5.4 partial success) and the
    /// artifact that just became known for it: `Keyed` iff the share lies on the
    /// artifact's polynomial at this node's index (F-02, the same gate every
    /// other way into `Keyed` passes — `adopt_share`); refused otherwise, exactly
    /// as the live path refuses one: the share leaves `store` AND its file (it is
    /// a share over some OTHER value, which a restart must not re-key), the
    /// epoch heals over its retained journal (`Acquiring(Logs)`) and
    /// `Stalled{OffPolynomial}` is raised — by the caller, once the slot stands.
    /// A share that is no longer in `store` (nothing holds it) is the same heal.
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

    /// THE F-02 gate, stated once: does `share` lie on the certified `outcome`'s
    /// polynomial at this node's index ([`validate_share_on_poly`])? Counted and
    /// said (ERROR) on a refusal; the caller decides what the refusal moves.
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

    /// Take this node's share for `epoch` out of BOTH places it lives — the
    /// shared `store` (the epoch manager's next reconcile demotes this node to
    /// verify-only for it; woken) and its file (a restart must not reload it).
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

    /// Two different quorum-certified artifacts for one epoch: the epoch is
    /// `Conflict` and its signing stops ([`Self::stop_signing`]); the ceremony, if
    /// any, is dropped and its recorded logs stay servable. Nothing goes on-chain
    /// (Д-6 defer).
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
        // `held` is a value this actor stood on or read from the store, and every
        // way a value reaches this actor puts it in the store first (the channel's
        // producers insert before they hand off; `reconcile_with_store` reads it)
        // — so the store holds a key to verify with. What a restart finds is
        // `recover`'s question, not this one's.
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

    /// The DURABLE half of `Conflict(E)`, in the one order that is safe under a
    /// death at any point: FIRST the verdict is on disk beside where the share is
    /// (`share_state::persist_conflict` — unless the artifact store, which owns
    /// the witness, already wrote it as it noted the second value), THEN the
    /// share leaves `store` and its file (the epoch manager's next reconcile
    /// demotes this node to verify-only for the epoch). `recover` reads the
    /// marker back before anything else, so a death between the two steps
    /// restarts as `Conflict` with the share evicted there; the reverse order
    /// left a window with neither marker nor share, from which a restart
    /// re-derived the share over the journal and re-keyed an epoch this node had
    /// stopped signing. A marker that cannot be written is said (ERROR) and
    /// counted, and the share is dropped regardless — the verdict holds in this
    /// process, and a restart re-judges the epoch from the store's witness.
    /// Counted and said (ERROR) once, here.
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
    /// Repeated on every tick rather than fired once, and the plane deduplicates.
    /// A one-shot announcement would be lost outright whenever the plane cannot act
    /// on it yet — an unreadable `committee[epoch]`, a sub-channel registration
    /// that lost a race — and the target would then never get an instance at all.
    /// The log line is the part that is one-shot.
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

    /// Adopt `(outcome, share)` for `epoch`: SELF-CHECK first, then durable, then
    /// in-memory, waiter last.
    ///
    /// # The self-check is a property of THIS function, not of its callers (П-3)
    ///
    /// [`validate_share_on_poly`] is the local fork-safety gate: it refuses a share
    /// that does not lie on the asserted polynomial at this node's index. It used
    /// to sit at ONE of the two call sites — the recompute-heal's, where an
    /// incomplete dealer-log set really can produce a wrong-looking share — and the
    /// live finalize path ran without it on the argument that `finalize_over_pinned`
    /// computes both halves from the same inputs and cannot disagree with itself.
    ///
    /// That argument is about today's implementation, and the ratified rule
    /// (`.dpos-study/DECISIONS.md`, П-3) is about the function: the gate runs before
    /// EVERY adoption. Moving it in here is what makes forgetting it impossible —
    /// `committee` is a required parameter, so a third adoption path cannot be
    /// written that skips the check, and neither can a refactor of the two that
    /// exist.
    ///
    /// **What a failure means and what it costs.** The share is not adopted: no
    /// disk write, no store insert, no waiter wake-up, and the epoch stays
    /// verify-only — the same outcome §5.4 gives a share whose persist failed, for
    /// the stronger reason. It is never a panic and never an adoption-with-a-warning:
    /// a wrong-but-plausible share produces seed partials every honest peer rejects
    /// per-partial, so adopting one subtracts this node from the quorum while it
    /// believes it signs (`outcome.rs`, the P3-3a argument). Both callers already
    /// hold the epoch's committee, so the gate adds no new failure mode of its own —
    /// an unreadable committee still stops the plan before it reaches here.
    ///
    /// # A FAILED PERSIST IS THE SECOND REFUSAL (§5.4)
    ///
    /// It used to warn and carry on, keeping the share in RAM. That is the one
    /// failure §5.4 names as unacceptable by itself: a node signing with a share no
    /// restart can reload is "signing now, mute after a restart" (R-021), and the
    /// votes it casts in between commit the epoch to partials it cannot reproduce.
    /// The share is refused instead and the epoch stays verify-only.
    ///
    /// The retry §5.4 asks for is a transition, not a timer: the caller moves the
    /// epoch to `Acquiring(Logs)`, and `try_recompute` re-derives the share
    /// from the retained ceremony journal on the next height tick — the journal is
    /// still there precisely because the eviction follows a successful adopt.
    ///
    /// Returns WHY the share was refused, so the caller raises the matching
    /// `Stalled{reason}` instead of inferring it from the store.
    ///
    /// The ordering is the point. `persist` must run BEFORE the in-memory insert so a
    /// mid-epoch restart reloads the share instead of carry-forwarding the wrong key
    /// and stalling. Both adoption paths (a live finalize, and the demote-heal's
    /// recompute) stated that rule in a comment and then hand-wrote the sequence, so it
    /// was enforced by neither; a third caller would have hand-written it again.
    ///
    /// Here it is enforced by the compiler rather than by care: `insert` MOVES the
    /// pair and `persist` borrows it, so swapping the two is a use-after-move, not a
    /// silent loss of the share. Keep them owned parameters for that reason — taking
    /// them by reference would hand the ordering back to whoever edits this next.
    ///
    /// Waking the boundary waiter is last because it is the only step a reader can
    /// observe. `notify_one` stores a permit when no waiter is armed, so a share that
    /// lands between the consumer's reconcile and its re-arm is not lost (single
    /// consumer: `EpochManager::run`).
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
            // REFUSED, not warned-and-continued. §5.4: a share accepted in RAM
            // whose file was never written means "signing now, mute after a
            // restart" — the node casts votes carrying seed partials for an
            // epoch it will come back unable to sign in, and R-021 is that
            // asymmetry. Verify-only is the recoverable half of the trade.
            //
            // THE RETRY IS A TRANSITION, not a new timer: the caller moves the
            // epoch to `Acquiring(Logs)`, and the next height tick re-derives
            // the share from the retained journal — which `adopt_share` has not
            // evicted, precisely because the eviction follows a successful adopt.
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
        // QUALIFY-BEFORE-COMMIT (AMENDMENT 5): the share landing in the shared
        // CeremonyStore IS this node's deterministic qualified verdict (read by the
        // vote-time marker gate + the marker relayer). No separate cert artifact.
        if let Ok(mut store) = self.store.write() {
            // The SHARE alone: the polynomial's owner is the artifact (П-3), and the
            // `outcome` this function was handed exists only to gate the share
            // against it above.
            store.insert(epoch, share);
        }
        self.share_notify.notify_one();
        self.metrics.dkg_ceremony_ok.inc();
        Ok(())
    }

    /// Age every epoch slot out on ONE window, then reclaim the journals of the
    /// epochs that left.
    ///
    /// An epoch is kept while `e + JOURNAL_RETENTION_EPOCHS >= now` and dropped past
    /// it. An under-quorum stall still gets SEALED and never finalizes, so
    /// `drive_finalization` never moves it on — without this sweep it lingers in
    /// `epochs` forever.
    ///
    /// ONE window, because the window is the exact bound of usefulness on every
    /// phase: the only finalize left runs over an artifact-sourced pinned set
    /// (`Agreed`), which ages out on THIS window — so a ceremony retained past it
    /// could never be finalized anyway, and one retained short of it is the halt
    /// this window exists to prevent: an agreement that has not converged by the time
    /// the chain enters its target still asks this actor for the bodies
    /// (`derive_pinned`), and a swept ceremony answers `Unavailable` forever, which
    /// parks every `verify` and leaves `build_proposal` with nothing to pin. That
    /// agreement's instance is aborted when the epoch manager enters `target + 1`
    /// (`prune_agreements`), a full epoch inside this window. A HALTED ordering chain
    /// cannot sweep anything at all — the caller only runs on a finalized height.
    ///
    /// The journal + the serve store (passive, self-verifying data) ride the same
    /// window for the recompute-heal (§8.11.1): a demoted member (or a peer it serves)
    /// can still recompute E's share while E is committee-relevant. A journal is
    /// evicted only once its epoch has aged out — past BOTH [`DealerLogStore`] and
    /// the slot (whose `Acquiring(Logs)` phase owns the heal lifetime) — so it is
    /// never reclaimed while still needed. The store ages its own map out here
    /// ([`DealerLogStore::retain`]) and returns the epochs it dropped, since the actor
    /// cannot enumerate it; the journal reclaim stays here because `share_dir` does.
    ///
    /// This is the ONE place the window is applied. Per-epoch state lives in the
    /// slot, so a new per-epoch fact opts in by being a field of it — the class of
    /// omission that let two one-shot marks grow for the life of the process is
    /// closed by the type, not by care. The stall gauges step down with the slots
    /// that carried them.
    fn sweep_epoch_state(&mut self, now: u64) {
        // The serve store ages ITSELF out (the actor can no longer enumerate its keys)
        // and hands back the epochs it dropped, which are exactly its contribution to
        // the reclaim set: `epoch < floor` is the same predicate as
        // `epoch + JOURNAL_RETENTION_EPOCHS < now`. This is candidate ENUMERATION over
        // one shared age predicate, not a cross-map liveness check — an epoch is
        // reclaimed because it aged out, never because some other map stopped naming
        // it. The reclaim call itself stays here: `share_dir` is the actor's.
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
        // Share-confirmations and the dealer-log hash index are per-target scratch on
        // the same lifetime: useful only while that target's agreement can still run.
        // The confirmation half ages on `floor`, which is the same predicate as
        // `retained` (`e >= now - R` ⟺ `e + R >= now`), on the map that owns it.
        self.confirmations.retain(floor);
        // A non-durable log is retryable only while its ceremony holds the bytes, so
        // the set cannot outlive the ceremonies it names.
        self.nondurable_logs.retain(|e, _| retained(*e));
        self.nondurable_dealings.retain(|e, _| retained(*e));
        if let Ok(mut m) = self.recorded_dkg_logs.write() {
            m.retain(|e, _| retained(*e));
        }

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
    }

    async fn on_height(&mut self, height: u64, rng: &mut impl CryptoRngCore) {
        // Three feeders drive this clock: the local finalized-height poller
        // (`fin + K`), the LIVE upstream cert frontier (so a still-catching-up
        // newcomer deals its first epoch on the live deadline), and marshal's
        // ordering tip off `FluentApp::report` (the only one still moving once
        // execution stalls). Take the max so an interleaved lagging tick can never
        // pull the deal/seal clock backward; process at the monotone height.
        let height = self.height_now().max(height);
        self.last_height = Some(height);
        // Gauged HERE, at the single point where every feeder's height lands,
        // rather than by each feeder. The poller gauged itself and the cert inlet
        // did not, so on a validator with an upstream the gauge reported `fin + K`
        // while the actor's real clock was `max(fin + K, upstream_frontier)` — the
        // entire reported lag was spurious.
        self.plane_clock.record_dkg_clock(height);
        let now = self.epoch_of(height);

        // First-tick journal reconcile: now that the frozen epoch geometry is finally
        // available (the actor only runs post-`geometry_ready`), delete every boundary-
        // passed journal off disk in one scan — the SAME `epoch + JOURNAL_RETENTION_EPOCHS
        // < now` predicate the
        // running sweep uses, but driven off the on-disk filename so a finalize-then-
        // restart-before-boundary (which holds the epoch in NO in-memory map) still
        // reclaims its leaked journal + stale at-rest secrets (R2). One-shot.
        if !self.reconciled_journals {
            share_state::reconcile_journals(&self.share_dir, now);
            self.reconciled_journals = true;
        }

        let mut to_send: Vec<Outgoing> = Vec::new();

        // 0. Decide every epoch in the window this actor looks at that is not decided
        //    yet: the target it may still deal for (`now + 1`) and the trailing
        //    retention window it may still need a key or a heal for — ONE function,
        //    `recover`, over the share file, the journal, the artifact and the clock.
        //    A decided epoch costs nothing here; an undecided one (committee or bit
        //    unreadable) is re-asked next tick.
        let started = self.decide_window(now, &mut to_send).await;

        // 1. Seal any DEALING ceremony whose collection deadline has passed: the phase
        //    moves to `Sealed`, or straight to `Agreed` if the artifact arrived while
        //    this node was still dealing (its clock lagged the network).
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
            // Our OWN seal broadcast is not ack-gated (it carries no ack; the log is
            // re-fetchable via the resolver), so a failed journal only warns.
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
                // The body-lost signal that arrived while dealing (F-02): the
                // sealed ceremony goes straight to acquiring, as it would have on
                // the signal itself; this tick's `drive_acquisition` pulls for it.
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

        // 1b. Evict pending dealing buffers for epochs we will never start (the epoch
        //     is decided past its dealing, or is now in the past) so `pending` stays
        //     O(1–2 live epochs). An undecided epoch is not-yet-started → still
        //     bufferable.
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

        // 2. Finalize any SEALED ceremony whose agreed pinned set is fully held. Also
        //    driven from `on_message` and from the artifact intake, so a set completed
        //    by an incoming Reveal finalizes immediately — see
        //    [`Self::drive_finalization`].
        self.drive_finalization(rng);

        // 2a. Gossip this node's share-confirmation for any target whose body-checked
        //     set grew since the last one (a no-op when nothing grew). Runs AFTER
        //     `drive_finalization`, which is what publishes the set being confirmed.
        to_send.extend(self.confirmations.mint(ConfirmTrigger::AnyGrowth));

        // 2a'. Ask the plane for an agreement instance for every target whose
        //      dealing has closed. AFTER `drive_finalization`, so an epoch this
        //      tick already finalized locally is not announced at all.
        self.announce_agreement_targets().await;

        // 2b. Age every per-epoch map out on ONE window, and reclaim the journals of
        //     the epochs that left. Runs AFTER `drive_finalization` so a ceremony that
        //     is finalizable on the boundary tick is completed first, never evicted out
        //     from under it. See [`Self::sweep_epoch_state`] for the window's rationale.
        self.sweep_epoch_state(now);

        // 3. The NEXT epoch's ceremony was started in step 0 (`decide_window` →
        //    `recover(now + 1)`), retried on EVERY tick while undecided (not just
        //    once at the epoch transition): committee[E+1] is committed on-chain
        //    sometime DURING epoch E, which can land AFTER the actor (driven by
        //    lagging finalized heights) first enters E — a single-shot check at the
        //    transition would see the committee still unreadable and NEVER deal, so
        //    the E+1 boundary block wedges (no PK_{E+1}). A decided epoch is never
        //    re-decided, so the retry is idempotent.
        // 3c. Reliable dealing delivery (P1, dealer leg): re-send each un-acked dealing
        //     point-to-point while pre-seal, so a member that dropped our initial dealing
        //     (or whose ack we lost) still receives it and (re-)acks. Bounded — each
        //     ceremony's `unsent` shrinks to ∅ as acks land; a no-op once every ceremony
        //     has sealed or resumed player-only. A ceremony started THIS tick (step 0)
        //     is not retransmitted on top of its own initial send, which is already in
        //     `to_send`; it retransmits on every SUBSEQUENT pre-seal tick until acks
        //     drain. The player-leg ack re-emit needs no actor change — it rides the
        //     `on_message → handle → broadcast_all` path.
        for (e, c) in self.ceremonies() {
            if !started.contains(&e) {
                to_send.extend(c.retransmit());
            }
        }

        self.broadcast_all(to_send).await;

        // 3b. Drive every `Acquiring` epoch: ask peers for the artifact each one lacks
        //     (the member without a share, the member with a share and no artifact,
        //     the non-member that needs `PK_E` — R-121/R-122), and attempt the journal
        //     recompute (§8.11.1's demote-heal) for each one holding every pinned body.
        //     Before the log fetch so the two network legs of a tick stay in one place.
        self.drive_acquisition(now, rng);

        // 4. Re-fetch missing dealer logs for any open, shorthanded ceremony (a
        //    restarted/late node that lost peer logs) via the DKG-log recovery
        //    resolver — gated on the open window. The resolver owns retry / multi-peer
        //    fallback / rate-limiting / blocked-peer eviction, so this just hands it
        //    the missing `{epoch, dealer, hash}` keys (deduplicated by the resolver)
        //    each tick; targeting aims at the known committee roster (the holders).
        self.fetch_missing_logs().await;
        self.debug_assert_settled();
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
    /// node selects over the IDENTICAL log set. The agreed pinned set (the filter
    /// below takes no other input) enforces that: a quorum certificate states the
    /// set outright, so no node picks one for itself. Without it,
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
    /// forged/divergent `PK_E` is independently caught by the certificate verdict
    /// rule `beacon::surface::certificate_verdict`, which σ-verifies every seed a
    /// certificate carries under the epoch's attested key and refuses on
    /// mismatch). The Byzantine log-equivocation case (a dealer signing conflicting
    /// logs) is the still-deferred consensus-pinned-QUAL residual
    /// (`dpos_beacon_share_reshare`). The actor is single-threaded (`run`'s `select!`),
    /// so there is no concurrent mutation of `epochs`.
    /// Re-attempt the journal write for every log this node holds but could not make
    /// durable. Event-driven (it rides the publish edge, no timer), bounded by the size
    /// of the failed set, and a no-op in the overwhelmingly common empty case. A dealer
    /// whose ceremony has already been swept has nothing left to re-journal; the
    /// retention sweep drops its epoch's entry.
    fn retry_nondurable_journals(&mut self) {
        // The dealings first (DB-08): the record is held here, so the retry is one
        // append per record and the queue keeps what still failed.
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
                // The record that backs THIS log — the evidence pair for the second
                // half of an equivocation, a `PeerLog` otherwise — so a retry never
                // downgrades a pair to a lone log.
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

    /// Publish each live ceremony's recorded dealer-log hashes
    /// (`idx→keccak256(SignedDealerLog)`, `idx` = the dealer's position in the agreed
    /// `committee[epoch]`) into the shared `recorded_dkg_logs` — what the agreement
    /// plane proposes from and what a share-confirmation states. Monotone (recorded
    /// logs only accrue) + idempotent. Committee-read per live ceremony (`n ≤ 51`,
    /// cheap).
    fn publish_recorded_logs(&mut self) {
        // Give every previously-failed write another chance BEFORE deciding what may
        // be claimed; a log that lands here is publishable on this same edge.
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
                // A log this node holds but cannot back after a restart is NOT
                // claimed: the index is what the agreement plane proposes from, and
                // `Confirmations::mint` signs a `ShareConfirm` from this same index.
                if nondurable.is_some_and(|set| set.contains(&(pk.clone(), hash))) {
                    continue;
                }
                // First-wins per seat, IN THE CODE and not only by the stability of
                // the source: `Confirmations` and the agreement's widest-wins read the
                // index as a set that never changes an entry in place
                // (`confirmations.rs` module doc), so a seat once published is never
                // overwritten here, whatever the ceremony answers later.
                if let std::collections::btree_map::Entry::Vacant(seat) =
                    map.entry(e).or_default().entry(idx as u8)
                {
                    seat.insert(hash);
                    grew = true;
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
            if let Some(pool) = self.confirmations.pool() {
                pool.note_inputs_grew();
            }
        }
    }

    /// Record a peer's share-confirmation, or drop it.
    ///
    /// The pool re-verifies the signature against `committee[target_epoch][idx]`, so
    /// the SIGNED half of a relayed confirmation is as good as a directly-sent one.
    /// The sender is not only a diagnostic, though: this is the CONSUMER of the
    /// frame, and the consumer is where a sender is bound to a seat in the frame's
    /// epoch (5.3-В) — `from` must hold a seat in `committee[target_epoch]`, the
    /// roster this function reads anyway, or the frame is refused as `no_seat`.
    /// Nothing in the tree relays a confirmation (the only emitter signs and sends
    /// its own, `confirmations.rs`), so a member relaying another member's costs no
    /// live path; a non-member relaying one is refused. Upstream of here the
    /// channel's `GatedReceiver` has already refused a sender outside the
    /// registered peer set, and [`Self::epoch_is_actionable`] an epoch outside the
    /// window; neither asks about seats.
    ///
    /// The unsigned envelope epoch must agree with the signed one — a mismatch is
    /// either a relay bug or an attempt to slip a confirmation past a receive-side
    /// epoch filter it does not actually bind.
    ///
    /// ASYMMETRY WITH [`Self::epoch_is_actionable`], deliberate and unresolved: that
    /// gate also admits an epoch whose ceremony is still RUNNING even after the
    /// clock has moved past it (ceremonies are swept on a retention window in
    /// `on_height`, not at the boundary), while the window below is `[now, now+2]`
    /// and nothing else. A ceremony still open for an epoch below `now` therefore
    /// gets its DKG frames through and its confirmations refused. It is the safe
    /// direction — the entry bar those confirmations feed is consumed at the
    /// agreement for `now+1` and later, so a count for an epoch already entered
    /// changes no decision — but it is not a coincidence and must not be
    /// "tidied up" by widening one to match the other.
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
        // The entry bar only ever counts confirmations for an epoch whose agreement
        // is live or about to be: `[now, now + 2]`, `now` being this actor's own
        // epoch clock (`epoch_of(last_height)` — the same one `is_bufferable` and
        // `recover` run on). Outside it the confirmation is unusable, so it is
        // refused HERE rather than after a committee resolve it would waste (R-023,
        // E4-12).
        //
        // BEFORE the first height tick there is no window, and this must not invent
        // one. `last_height` is `None` until `on_height` drains its first value, and
        // the actor is spawned before that happens, so a `0` floor here would put
        // the window at `[0, 2]` and refuse every confirmation on any chain past
        // epoch 2. The cost of that is PERMANENT, unlike the dealing path's: a
        // dealer re-sends an un-acked dealing on every pre-seal tick, but
        // `Confirmations::mint` is edge-triggered on WIDTH growth
        // (`confirmations.rs`, the `previous >= confirmed.len()` memo), so a
        // full-width confirmation dropped here is never re-issued and this node's
        // entry bar undercounts a member for the whole epoch. With no clock the
        // seat check below is the whole bound — which is the same bound the window
        // would add nothing to, since an epoch this node cannot place in time is
        // one whose committee record it either holds or does not.
        if let Some(height) = self.last_height {
            if !within_ingress_window(self.epoch_of(height), confirm.target_epoch) {
                self.refuse(from, Some(confirm.target_epoch), "confirm_window");
                return;
            }
        }
        let Some(roster) = (self.committee_for)(confirm.target_epoch) else {
            return;
        };
        // The consumer's seat check (see the doc above): the ONE read this
        // function makes is the roster, and the sender must sit in it.
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

    /// Record a dealer equivocation a ceremony step just PROVED: the pair goes
    /// into the epoch's slot as EVIDENCE (outliving the ceremony, until the sweep;
    /// data beside the phase — a two-log dealer is banned from gossip, the epoch
    /// is not `Conflict`, which two quorum-certified artifacts alone reach), one
    /// WARN line and one count per `(epoch, dealer)` — the step flags it exactly
    /// once, on the pair's creation — naming both hashes so an operator can pull
    /// the two bodies out of the journal. Called AFTER the journal append, and
    /// `durable` says whether the evidence record landed: a pair whose record did
    /// not is still a proven pair (the ban and the RAM copy hold), but the line
    /// says so, because a restart would then lose it until the nondurable retry
    /// re-appends it (`retry_nondurable_journals`). Nothing is sent on-chain (Д-6
    /// defer, `DECISIONS.md`).
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

    fn drive_finalization(&mut self, rng: &mut impl CryptoRngCore) {
        // Publish our recorded dealer-log hashes for the agreement plane to propose
        // from and for share-confirmations to state — the recording paths (seal /
        // Reveal / resolver ingest) all funnel through here.
        self.publish_recorded_logs();
        // `pinned_ready` probes non-destructively (Logs clone); `finalize_over_pinned`
        // then consumes the fulfilled ceremony. Both run STILL DURING the margin
        // window — before the epoch's boundary block is proposed/verified — so the
        // verify-path C gate can read the share.
        // Deferrals observed this tick, reported after the borrow ends.
        // `(epoch, all_held, unmappable_pinned)`.
        let mut deferrals: Vec<(u64, bool, usize)> = Vec::new();
        let plans: Vec<(u64, Set<PeerPubkey>)> = self
            .epochs
            .iter()
            .filter_map(|(e, slot)| {
                // The finalize INPUT is the AGREED dealer-log HASH SET, never a
                // locally settled one — so every honest node selects over the
                // IDENTICAL pinned set ⇒ identical `PK_E` (honest divergence
                // impossible by construction). It needs no settle deadline of its
                // own: the deadline exists only to make every honest node select
                // over an identical set, which a quorum certificate states outright
                // — which is also what decouples this finalize from the height clock.
                // `Agreed` is the ONE phase that holds both a ceremony whose dealing
                // has closed and the set: a player consumed by a failed finalize has
                // already left it.
                let EpochState::Agreed { ceremony: c, set } = &slot.state else {
                    return None;
                };
                let target = *e;
                let committee = (self.committee_for)(target)?;
                let n = committee.len();
                // `all_held` = every pinned body held with a matching hash
                // (fetch-before-finalize: a `false` means WAIT, never subset-
                // finalize — the resolver fetches the missing pinned bytes);
                // `ready` = a quorum is selectable within the pinned+held set.
                let (ready, all_held) = c.pinned_ready(rng, &committee, &set.pinned);
                if !(all_held && ready) {
                    // This wait used to be completely silent, so a stall of this
                    // family was only diagnosable post-mortem from a wedged
                    // boundary. Report it, and separately count pinned indices
                    // with no committee position: those can never be satisfied
                    // (the ceremony skips them, see `scoped_pinned_logs`), so a
                    // non-zero count means the pinned set and the committee this
                    // node reads disagree.
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
            // Two latches, each on its own condition (F-03). `BodyMissing` while a
            // pinned body is not held; it leaves the tick every body is (the fetch
            // landed — a held body is never lost, so it leaves once). `QuorumMissing`
            // is a property of the pinned SET, readable only with every body in
            // hand: `ready` probes the held bodies, so below `all_held` a `false` says
            // nothing about the set, and the latch is neither raised nor dropped on
            // it. Once raised it stands until the phase leaves (`carries`): the set
            // is agreed and its bodies are all here, so nothing can change the
            // answer.
            let reason = if all_held {
                self.clear_stall(epoch, StallReason::BodyMissing);
                StallReason::QuorumMissing
            } else {
                StallReason::BodyMissing
            };
            // `Stalled{QuorumMissing}` / `Stalled{BodyMissing}`: one line and one
            // gauge step per (epoch, reason). The deferred counter and the
            // unmappable-set line keep their once-per-(epoch, reason) semantics on
            // the same latch (the unmappable COUNTER stays per tick, as it was).
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
            // Capture-then-commit on the PHASE: the ceremony is taken out of its
            // slot, `finalize` consumes its player, and the phase the epoch lands in
            // says what happened — `Keyed` on an adopted share; `Acquiring(Logs)` on
            // a finalize `Err` that a fuller journal can still answer (a transient
            // `MissingPlayerDealing` race aside, see below) or a refused adoption,
            // where the retained journal is re-selected over the pinned set on the
            // next tick (`try_recompute`); `Unrecoverable` on the one `Err`
            // no fetch can answer. In every arm the recorded logs move to the serve
            // store, so a peer recovering this epoch is served regardless.
            let (mut ceremony, set) = match self.take_state(e) {
                Some(EpochState::Agreed { ceremony, set }) => (ceremony, set),
                Some(other) => {
                    self.put_back(e, other);
                    continue;
                }
                None => continue,
            };
            let finalized = ceremony.finalize_over_pinned(rng, &committee, &set.pinned);
            // Eager serve seed (the no-restart hot path never reads disk): the
            // recovery `Producer` keeps serving this finalized epoch's logs
            // O(1) until the boundary sweep. A strict subset-copy of the journal.
            // The journal is NOT evicted here. A node that finalized but has not
            // yet crossed the boundary keeps its journal (and its serve-store
            // copy) so it can still serve a late-restarting peer — both reclaimed
            // by `sweep_epoch_state`, bounded scratch.
            self.log_store.seed(e, ceremony.take_signed_logs());
            let (next, stalled) = match finalized {
                Ok((_out, share)) => {
                    // F-02 / R-039: the gate is the ARTIFACT's polynomial, on this
                    // path exactly as on the heal. The local ceremony's `Output`
                    // is what this node computed; the artifact's is what the
                    // network certified, and a share is adopted only if it lies on
                    // the latter. (They agree whenever the ceremony selected the
                    // pinned bodies; a share off the certified polynomial is a
                    // selection over a set the network did not pin.)
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
                    // This node acknowledged a dealer's private point and no longer
                    // holds it. The dealer does not reveal a point whose ack it
                    // holds, and a sealed log cannot be re-opened, so no amount of
                    // fetching can produce the missing input — commonware's own doc
                    // calls it "not recoverable without external intervention".
                    // Terminal, and said so; a share-less member is a safe verifier.
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
            // The phase first, the latch second: a latch names a condition of the
            // phase the epoch is IN, and its line says which.
            self.set_state(e, next);
            if let Some(reason) = stalled {
                self.stall(e, reason);
            }
        }
    }

    /// The heal a failed finalize / refused adoption falls back to: the artifact's
    /// pinned set mapped onto the committee (the recompute's selection scope — the
    /// SAME input the live finalize scopes to), its `Output` (the self-check
    /// target), and the pinned bodies the retained journal does not hold yet
    /// (`want`, fetched by exact `(dealer, hash)`).
    fn heal_over(&self, epoch: u64, committee: &Set<PeerPubkey>, set: AgreedSet) -> RecomputeState {
        let pinned = pinned_by_dealer(committee, &set.pinned);
        // want = pinned bodies − the bodies already in our retained journal, by
        // exact `(dealer, hash)`: a journaled OTHER body of a pinned dealer is
        // not "held".
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

    /// Decide every undecided epoch this actor looks at on a tick: the target it
    /// may still deal for (`now + 1`) and the trailing retention window
    /// `[max(BOOTSTRAP, now − R), now]` it may still owe a key or a heal for. A
    /// future epoch (> now + 1) is not seated yet; below BOOTSTRAP the beacon is
    /// seedless (no share obligation, no artifact). Returns the epochs that
    /// entered `Dealing` on this tick.
    ///
    /// `out` collects what a fresh start / a pre-seal resume of the TARGET sends;
    /// a window epoch's resume sends nothing (it is at or past its boundary — no
    /// dealer to re-ack, and its own log, if sealed, is re-fetched by hash).
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
        // A window epoch is at or past its boundary, and `recover` sends nothing
        // for one (no dealer left to re-ack; its own log is re-fetched by hash).
        debug_assert!(dropped.is_empty(), "a past-boundary resume must not send");
        started
    }

    /// Whether `epoch` is one `decide` looks at on this actor's clock
    /// ([`decidable_epochs`]). Nothing before the first tick (no clock to place
    /// an epoch against its deadline).
    fn decidable(&self, epoch: u64) -> bool {
        let Some(height) = self.last_height else {
            return false;
        };
        decidable_epochs(self.epoch_of(height)).any(|e| e == epoch)
    }

    /// Decide `epoch` if it is undecided: one [`Self::recover`], one `enter`, and —
    /// for an epoch that starts DEALING — the drain of the dealings that raced
    /// ahead of the start. Returns whether the epoch entered `Dealing` now.
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
            // Not dealing (sat out, player-only, keyed, ...): drop any dealings that
            // raced ahead of a start that will now never happen, so they don't
            // linger un-acked until the sweep.
            self.pending.remove(&epoch);
            return false;
        }
        // Drain any dealings that raced ahead of our start (the start-race): replay
        // them through `handle` NOW, before any seal, so every dealer we heard from is
        // acked. Order-independent (`try_ack` fires only once both halves are
        // buffered). The acks the replay emits are collected into `out` and broadcast
        // by the caller, so a dealer that previously got ≤ quorum−1 acks now seals
        // `Ok`, not `TooManyReveals`. The newly-accepted dealings are journaled too.
        if let Some(buffered) = self.pending.remove(&epoch) {
            // A buffered dealing was admitted on its epoch, and on its seat only
            // if `committee[target]` was readable when it arrived (`on_message`,
            // the buffer branch); one buffered before that record was readable
            // was admitted on its epoch alone. The consumer's admission is asked
            // HERE, where the ceremony that consumes it exists — the SAME
            // question the live dispatch asks (`ceremony_refusal`: the seat, and
            // the local ban of a dealer this epoch holds evidence for, which a
            // journal replay restores before the drain runs). Both halves are
            // one sender's, so the answer is the sender's, asked once.
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
                // Replay each present half (commitment-then-share). Order-independent
                // (`try_ack` fires only once both halves are buffered), so `from` is
                // re-used per replay → clone (PeerPubkey is Clone, NOT Copy).
                admitted.extend(halves.into_iter().map(|body| (from.clone(), body)));
            }
            // Collect each drained dealing's Step (borrowing the ceremony only for the
            // replay) so its ack broadcast can be gated on ITS OWN `ReceivedDealing`
            // write being durable (step 1f), exactly as the on_message path does.
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

    /// The consumer's admission of a ceremony frame from `from` for `epoch`, the
    /// ONE answer both ingress paths give — the live dispatch (`on_message`) and
    /// the drain of the start-race buffer (`decide`): `None` admits; `Some` is the
    /// reason for the shared ingress counter.
    /// - `no_seat`: a live ceremony consumes a frame keyed by its sender — a
    ///   dealing is buffered under `from`, an ack prunes `from` from the
    ///   retransmit set — and commonware's `Player`/`Dealer` answer a stranger
    ///   with a silent `None` / `UnknownPlayer`. Say it instead, once, on the
    ///   shared counter.
    /// - `equivocator`: the local ban of 5.3-Б, extended from logs to DEALINGS
    ///   (E-10): a dealer this epoch holds equivocation evidence for gets no
    ///   `Commitment`/`Share`/`Ack` consumed. Without it a player that had not
    ///   acked yet would take the dealing of the dealer's SECOND polynomial, and
    ///   its share would then lie off the polynomial the network pinned — a share
    ///   lost for nothing. The ceremony refuses the dealer's further LOGS by
    ///   gossip (`handle`); the dealings are refused here, where the sender is
    ///   bound to its seat. Nothing goes on-chain (Д-6). The evidence is the
    ///   slot's, so a proven pair outlives the ceremony and a resumed one is
    ///   banned from the tick it is restored (`enter`).
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

    /// Step 1f's write, the ONE form both ingress paths use: journal a step's
    /// `records`; when any did not land, withhold the step's acks (`acked`) and
    /// queue every `ReceivedDealing` among them for the retry from the record
    /// (DB-08, `retry_nondurable_journals`). Returns whether the write was durable —
    /// the caller broadcasts the step's outgoing only then. A log that did not land
    /// is the caller's to name (by the id the ceremony authenticated, not by the
    /// record), and an ack record is re-made by the next retransmit.
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

    /// Step 1f, the withheld half: the acks of a step whose journal write failed are
    /// forgotten by the ceremony, so a retransmit cannot re-emit them from the
    /// cache as if their record were durable (`DkgCeremony::withhold_ack`).
    fn withhold_acks(&mut self, epoch: u64, acked: &[PeerPubkey]) {
        if let Some(c) = self.ceremony_mut(epoch) {
            for dealer in acked {
                c.withhold_ack(dealer);
            }
        }
    }

    /// `recover(E)` — THE one function that turns what this node holds for `E`
    /// into `E`'s phase (§5.2, the restart table): the share file (`store`), the
    /// ceremony journal, the agreed artifact (`outcome_at`) and the clock against
    /// the seal deadline; plus the two inputs of the deal decision, the chain's
    /// `changed` bit (Д-7) and `committee[E]`. `None` ⇒ undecidable yet (a
    /// committee or a bit this node cannot read), re-asked next tick.
    ///
    /// The rows, in the order they are decided:
    /// - a `Conflict` marker on disk, or a second value noted in the store ⇒
    ///   `Conflict` (the terminal is durable: a share file that outlived it does
    ///   not re-key the epoch);
    /// - share held, artifact held ⇒ `Keyed` iff the share lies on the artifact's
    ///   polynomial (`key_held_share`, F-02 — refused otherwise, into the journal
    ///   heal); share held, no artifact ⇒ `Acquiring(ArtifactForShare)` — the
    ///   partial success of §5.4, healed by a peer's copy;
    /// - not a mint epoch ⇒ `KeyOnly` (carry-forward: the key in force is an
    ///   earlier mint's, nothing to deal or acquire);
    /// - not a member ⇒ `KeyOnly` with the artifact, `Acquiring(ArtifactForKey)`
    ///   without (I4: the key is needed to verify the epoch's certificates);
    /// - a member with no share: the journal decides.
    ///   `Present` ⇒ resume — the seeded dealer re-derived while `h < seal(E)`,
    ///   player-only at/after it (§8.11.1) — and the phase is `Dealing` /
    ///   `Sealed`, or `Agreed` when the artifact is already held (D-10: the
    ///   resume then prefers each dealer's PINNED body over its first-recorded
    ///   one); a resume `Err` is `Unrecoverable` (memory, not a per-tick retry).
    ///   `Torn` and `NoFile` are ONE rule by timing: `h < seal(E)` ⇒ nothing was
    ///   ever broadcast (a node seals only at/after the deadline) and the dealer
    ///   is deterministic, so start fresh (a torn file is removed first, so the
    ///   fresh journal reads back clean); `h ≥ seal(E)` ⇒ `SatOut` — this node
    ///   may already have sealed and broadcast, and re-dealing would sign a
    ///   second, differently-acked log (R-036, R-072/E5-36).
    ///   A resumed epoch at or past its boundary (`E ≤ now`) that has no artifact
    ///   is `Acquiring(ArtifactForCeremony)` rather than `Sealed`: its agreement
    ///   ran without this node, so the artifact is at the peers, not in an
    ///   instance this node could still join.
    ///
    /// The third element is the latch the decided phase is raised with once its
    /// slot stands (a refused reloaded share).
    async fn recover(
        &mut self,
        epoch: u64,
    ) -> Option<(EpochState, Vec<Outgoing>, Option<StallReason>)> {
        let quiet = |state: EpochState| Some((state, Vec::new(), None));
        let stored = self.stored(epoch);
        if let Some(marker) = share_state::load_conflict(&self.share_dir, epoch) {
            // The terminal outlives the process. A share file the eviction missed
            // (or a death between the marker and the eviction) is not re-keyed:
            // it leaves `store` and its file here as it left on the verdict.
            self.drop_share(epoch);
            let (held, second) = match marker {
                ConflictMarker::Pair(held, second) => (held, second),
                // FAIL-CLOSED: a marker is written only by a verdict, so a
                // damaged one is still the verdict — with no pair to name.
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
            // The key to verify with is whatever the store holds (F-01): the
            // marker is fsync'd before the artifact's write-behind, so a death in
            // between restarts with the verdict and no artifact — acquired then.
            let key = stored.as_ref().map(|s| value_digest(&s.held));
            return quiet(EpochState::Conflict { held, second, key });
        }
        if let Some((held, divergent)) = stored
            .as_ref()
            .and_then(|s| Some((value_digest(&s.held), s.divergent?)))
        {
            // The store saw two certified values while no slot stood (a hand-off
            // lost, or a restart in between): the verdict is reached from the
            // store alone, and made durable here.
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
        // THE DECISION IS THE CHAIN'S BIT, not a roster comparison (Д-7). The
        // contract sets `changed[target]` by exactly the rule this used to re-derive
        // — `committee[target] != committee[target−1]` — in the same
        // `commitEpochCommittee` call that writes the committee, so the bit IS the
        // answer and deriving it again could only disagree with it. What that removes
        // is not an EVM read but a class: `committee_pair_for` existed to stop two
        // independent reads straddling a block and seeing a change the contract never
        // recorded, and a single frozen bit cannot straddle anything.
        //
        // The ROSTER is still read, for the ceremony itself — who deals to whom — but
        // it is no longer what decides.
        let mints = self.mints_at(epoch)?;
        if !mints {
            return quiet(EpochState::KeyOnly { digest: None });
        }
        let committee = (self.committee_for)(epoch)?;
        let me = self.me_key.public_key();
        // Model B: only a MEMBER of committee[target] deals to itself. A node that
        // is in committee[target-1] but not committee[target] does not deal.
        if !committee.iter().any(|p| *p == me) {
            return quiet(match artifact {
                Some(proposal) => EpochState::KeyOnly {
                    digest: Some(value_digest(&proposal)),
                },
                None => EpochState::Acquiring(Acquire::ArtifactForKey),
            });
        }
        // Timing gate (R2-L): a node only ever seals AT/AFTER the deadline, so
        // `height < deadline ⇒ we never sealed ⇒ no original log was ever
        // broadcast`, and re-deriving the seeded (idempotent) dealer + sealing
        // once later is safe. At/after the deadline we stay player-only — a torn
        // or absent journal cannot prove we did not already seal + broadcast a
        // possibly-divergent log, so we NEVER re-seal.
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
                     seal deadline — sitting out this epoch (we already participated; \
                     re-dealing would self-equivocate)"
                );
                return quiet(EpochState::SatOut { key });
            }
            JournalLoad::NoFile if past_seal => {
                tracing::warn!(
                    epoch,
                    "live DKG: no ceremony journal at or after the seal deadline — sitting out \
                     this epoch (a lost journal cannot prove we never sealed; re-dealing would \
                     self-equivocate)"
                );
                return quiet(EpochState::SatOut { key });
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
        // A resumed epoch at or past its boundary sends nothing: there is no dealer
        // left to re-ack and its own sealed log is re-fetched by hash.
        let outgoing = if past_boundary { Vec::new() } else { outgoing };
        Some((state, outgoing, None))
    }

    /// Load the per-epoch ceremony journal for `target`. The tri-state lets
    /// [`Self::recover`] tell a genuine first run (deal) from a present-but-damaged
    /// journal.
    fn load_journal(&self, target: u64) -> JournalLoad {
        let max = NonZeroU32::new(fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as u32)
            .expect("MAX_COMMITTEE_SIZE > 0");
        share_state::load_journal(&self.share_dir, target, &self.share_state, max)
    }

    /// Start a fresh ceremony for `target`, journaling its initial records. Returns
    /// the live ceremony and its initial sends, or `None` when the ceremony cannot
    /// be built over the committed roster (deterministic on the same inputs, so
    /// the caller records `Unrecoverable` rather than retrying). The dealer
    /// polynomial is SEEDED from the validator key + epoch
    /// (`ceremony::dealer_seed_rng`), so a `NoFile → start_fresh` after a datadir
    /// loss re-derives the IDENTICAL commitment — closing the former `OsRng`
    /// re-deal self-equivocation gap (§8.11.1).
    fn start_fresh(
        &mut self,
        target: u64,
        next: Set<PeerPubkey>,
    ) -> Option<(DkgCeremony, Vec<Outgoing>)> {
        match DkgCeremony::start(&self.namespace, target, next, self.me_key.clone()) {
            Ok((ceremony, step)) => {
                // Start records our own commitment/self-dealing; the outgoing is our
                // broadcast commitment + private shares, not an ack, so it is not gated.
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

    /// Resume `target`'s ceremony from its journaled `records` (mid-window restart).
    /// PRE-seal (`reconstruct_dealer`) it RE-DERIVES the seeded dealer and keeps
    /// distributing; at/after the deadline it is player-only and never re-seals
    /// (§8.11.1). With the artifact already held (`agreed`), each dealer's PINNED
    /// body is the one the rebuilt player stands on (D-10) — the first-recorded one
    /// otherwise. A `MissingPlayerDealing` (truncated journal dropped a
    /// publicly-acked dealing) is `None`: the caller records `Unrecoverable`, never
    /// a crash and never a per-tick retry.
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
                // Seal-state is intrinsic to the resumed ceremony (dealer retired; our
                // own log in `recorded` iff we sealed) — nothing to track separately.
                let own_log_recorded = resumed.ceremony.own_log_recorded(&self.me_key.public_key());
                tracing::info!(
                    epoch = target,
                    own_log_recorded,
                    "live DKG: ceremony resumed from journal"
                );
                Some((resumed.ceremony, resumed.outgoing))
            }
            Err(e) => {
                // The one `Err` a journal replay has is the same one a finalize has —
                // a log acking a dealing this node no longer holds — and it is the
                // same statement: one WARN, one count, per epoch.
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

    /// Whether an incoming DKG message should be BUFFERED when no ceremony for its
    /// epoch exists yet (the start-race), rather than dropped. Only DEALINGS
    /// (`Commitment`/`Share`) for a near-future, not-yet-sealed, not-yet-finalized
    /// epoch qualify — acks/reveals are meaningless without a live ceremony to feed,
    /// and `last_height` bounds the future window so far-future / garbage epochs
    /// cannot accumulate (a DoS guard); stale buffers are also evicted each tick.
    ///
    /// This is the EPOCH half of the buffer's admission only. The SEAT half — is
    /// the sender in `committee[epoch]` — is the caller's ([`Self::on_message`],
    /// the buffer branch), asked against that record when it is readable and
    /// deferred to the drain in [`Self::decide`] when it is not.
    fn is_bufferable(&self, epoch: u64, body: &DkgBody) -> bool {
        if !matches!(body, DkgBody::Commitment(_) | DkgBody::Share(_)) {
            return false;
        }
        // Only an UNDECIDED epoch can still start a dealer to drain the buffer into:
        // a decided one is dealing already (the frame is dispatched to it, not
        // buffered), has closed its dealing, is keyed, or will never deal.
        if self.epochs.contains_key(&epoch) {
            return false;
        }
        let now = self.epoch_of(self.height_now());
        // `epoch > now` is this buffer's OWN rule on top of the shared window: a
        // dealing for `now` or below is for a ceremony already started / past.
        if epoch <= now || !within_ingress_window(now, epoch) {
            return false;
        }
        true
    }

    /// The clock every deal/seal deadline reads: the last drained height, or `0`
    /// before the first tick.
    ///
    /// `0` is the right FLOOR for a deadline — every comparison below is "is the
    /// chain past height X yet", and answering "not yet" for a clock that has not
    /// started only delays an action until the first tick lands. It is NOT the
    /// right answer for a WINDOW: [`Self::on_confirm`] reads
    /// [`Self::last_height`] itself, so that it can tell "epoch 0" from "no clock".
    fn height_now(&self) -> u64 {
        self.last_height.unwrap_or(0)
    }

    /// Whether `epoch` is one this actor could act on at all: a ceremony it is
    /// already running, or one of the two it may still start / buffer for. Stated
    /// up front so an arbitrary epoch on the wire costs no committee resolve
    /// (E4-12, R-023).
    ///
    /// A deliberate SUPERSET of what the dispatch below will actually do with the
    /// frame, not the same predicate: this admits `[now, now+2]`, while
    /// [`Self::is_bufferable`] takes only `[now+1, now+2]` (`epoch <= now` is
    /// already-started / past for a dealing) and the ceremony dispatch takes only a
    /// live ceremony. Superset is the right side to err on here — this gate exists
    /// to bound COST, and a frame it lets through is refused a few lines later by
    /// the check that owns the decision.
    fn epoch_is_actionable(&self, epoch: u64) -> bool {
        if self.ceremony(epoch).is_some() {
            return true;
        }
        within_ingress_window(self.epoch_of(self.height_now()), epoch)
    }

    /// THE one place a BEACON frame is refused: one count on the channel's ingress
    /// metric, one `debug` line naming who sent what for which epoch and why, with
    /// this actor's own clock beside it (`None` before the first height tick). The
    /// metric registry is the shared `dpos_ingress_dropped_total` and stays shared
    /// (R-070 — one counter family per concern, never one endpoint for all).
    ///
    /// The reasons, and where each is decided:
    ///  * `undecodable` — [`Self::on_message`], a frame that is not a beacon
    ///    frame at all (`BeaconMessage::read`), one too short to carry an epoch
    ///    (the eight-byte peek), or one whose body does not decode as a
    ///    `DkgMsg`; `epoch` is `None` when the frame never yielded one;
    ///  * `epoch` — [`Self::epoch_is_actionable`], before the body decode;
    ///  * `confirm_window` — [`Self::on_confirm`], the entry-bar window;
    ///  * `no_seat` — the CONSUMER found no seat for the sender in the frame's
    ///    epoch: the live ceremony's roster ([`DkgCeremony::has_seat`]) at the
    ///    dispatch in [`Self::on_message`], `committee[epoch]` at the start-race
    ///    buffer in [`Self::on_message`] when that record is readable, and the
    ///    ceremony's roster at the drain in [`Self::recover`] for what was
    ///    buffered before it was; or `committee[target_epoch]` in
    ///    [`Self::on_confirm`].
    ///
    /// What is NOT counted here is not a refusal: an ack or a reveal with no live
    /// ceremony to feed (nothing to do, the resolver re-fetches a reveal), a
    /// confirmation for an epoch whose committee this node cannot read yet (the
    /// node's own state, not the frame's), and a confirmation the pool already
    /// holds at that width (a duplicate).
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
        // Decode bounded by MAX_COMMITTEE_SIZE (upper bound; exact n not needed).
        let max = NonZeroU32::new(fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as u32)
            .expect("MAX_COMMITTEE_SIZE > 0");
        let mut wire = buf;
        // A frame that is not a beacon frame is a refusal too, and it is counted
        // (`undecodable`): `refuse` is THE one place a frame is refused, and a
        // silent `return` here would make that a half-truth for exactly the
        // frames an operator most wants to see counted.
        let payload = match BeaconMessage::read(&mut wire) {
            Ok(BeaconMessage::Dkg(p)) => p,
            Err(_) => {
                self.refuse(&from, None, "undecodable");
                return;
            }
        };
        // THE EPOCH BEFORE THE BODY. `DkgMsg`'s wire is
        // `ceremony_epoch(u64) ‖ body_tag(u8) ‖ body` (`dkg_msg.rs:83-84`,
        // `:137`), so the epoch is readable from the first eight bytes without
        // touching the `Commitment` / `Reveal` decoders — which are the expensive
        // ones, being the polynomial and the signed log. Everything a stranger
        // naming a far epoch could have made this node do (a committee resolve, a
        // `pending` slot, a ceremony `handle`) is downstream of here. WHO the
        // sender is was settled before this function saw the bytes — the
        // channel's pre-decode `GatedReceiver` admits only the registered peer
        // set — and which SEAT it holds in `epoch` is the consumer's question,
        // asked below where the frame is consumed (`has_seat` at the ceremony
        // dispatch, `committee[epoch]` at the start-race buffer when readable,
        // `committee[target_epoch]` in `on_confirm`); this actor keeps no
        // membership opinion of its own (5.3-В, second round).
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
        // buffer it (drained by `recover`) rather than DROP it (a dropped
        // dealing leaves that dealer un-acked ⇒ `TooManyReveals` ⇒ `DkgFailed`).
        // Acks/Reveals with no live ceremony are still dropped (nothing to feed;
        // a re-sealed Reveal re-arrives once we are live via the long window). DKG-log
        // RECOVERY is no longer a gossip body — it rides the `commonware_resolver::p2p`
        // engine (see `on_resolver_message` / `fetch_missing_logs`).
        // The consumer's admission — the seat, and the local ban of a proven
        // equivocator (E-10) — the same question the start-race drain asks
        // (`ceremony_refusal`), counted like every other ingress refusal.
        if let Some(reason) = self.ceremony_refusal(epoch, &from, &body) {
            self.refuse(&from, Some(epoch), reason);
            return;
        }
        if let Some(c) = self.ceremony_mut(epoch) {
            let step = c.handle(from, body);
            let recorded_log = step.recorded_a_log();
            let acked = acked_dealers(&step);
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
            // A `ReceivedDealing` that did not land is queued for the retry there
            // (DB-08, `journal_or_defer`).
            let durable = self.journal_or_defer(epoch, step.journal, &acked);
            self.note_equivocation(epoch, step.equivocation.as_ref(), durable);
            if !durable {
                // A log that did not land is retried by its id from the ceremony.
                // The dealer comes from the ceremony, which `check`ed the signature to
                // get it — NOT from `from`. A peer may relay another dealer's valid
                // `Reveal`, and blaming the sender would leave the real dealer's
                // unbacked claim published while suppressing an honest log.
                if let Some(id) = step.recorded_log {
                    self.nondurable_logs.entry(epoch).or_default().insert(id);
                }
            }
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
                // The two claims made from here — the log's hash in the shared
                // `recorded_dkg_logs` index, and the `ShareConfirm`
                // `Confirmations::mint` signs from that same index — are gated on
                // per-`(dealer, hash)` durability in `publish_recorded_logs`: a log
                // named in `nondurable_logs` is excluded from both. The ACK gate above stays
                // separate because it answers a different question (is our own
                // `Player.view` recoverable), and an ack once withheld is not retried.
                self.drive_finalization(rng);
                // A newly-recorded log widens what this node can confirm, and the
                // entry bar is counted over confirmations that COVER the proposed
                // set — so the two widths a leader cannot wait a block for go out
                // on this edge. The rest ride the next height tick.
                let minted = self.confirmations.mint(ConfirmTrigger::Decisive);
                self.broadcast_all(minted).await;
            }
        } else if self.is_bufferable(epoch, &body) {
            // The consumer's seat check, as far as it can be asked BEFORE the
            // ceremony exists: the record a ceremony for `epoch` would be built
            // over is `committee[epoch]` (`recover`, the same `committee_for`),
            // so when that record is readable a sender with no seat in it is
            // refused HERE and occupies no slot — the price is the one memoized
            // record read, for an epoch the window already admitted. When the
            // record is not readable yet (the start-race's own case: this node is
            // behind the dealer) the dealing is buffered on its epoch alone and
            // the drain in `recover` asks the ceremony's roster instead.
            if (self.committee_for)(epoch).is_some_and(|roster| roster.position(&from).is_none()) {
                self.refuse(&from, Some(epoch), "no_seat");
                return;
            }
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
        self.debug_assert_settled();
    }

    /// `fetch_targeted` every PINNED body a ceremony lacks, by exact
    /// `(epoch, dealer, hash)`, via the DKG-log recovery resolver (§8.11.1). The
    /// pinned set — the artifact's `idx → hash`, mapped onto `committee[epoch]` — is
    /// the ONLY source of what to ask for: it is the truth about which body of each
    /// dealer is in force, and a dealer whose held log is a DIFFERENT body than the
    /// pinned one is fetched exactly like one this node holds nothing of (R-002: the
    /// per-dealer form called that dealer held and never asked). Before an artifact
    /// there is nothing to ask for by hash — a ceremony still collecting relies on the
    /// gossip `Reveal`s, and the agreement's own `verify` pulls a PROPOSAL's bodies by
    /// the proposal's hashes (`dkg_agree::fetch_bodies`). Bounded by the ceremony's
    /// lifetime — `on_height`'s one retention window — and by nothing else, so a
    /// ceremony still holding an agreed set never stops asking for the bodies its
    /// finalize needs. The resolver dedupes in-flight keys, so re-issuing the missing
    /// set each tick is idempotent; targeting aims at the known committee roster
    /// (the log holders, in `latest.primary` via the registry-union tracker).
    ///
    /// Runs in `on_height` AFTER finalize + the past-boundary sweep, so `self.epochs`
    /// already reflects every drop; it then `retain`s the resolver's in-flight fetches to
    /// exactly the keys it (re)issues this tick — CANCELLING the fetches of any epoch that
    /// finalized or was swept, so the resolver stops re-issuing dead keys every
    /// `fetch_retry_timeout` for the life of the process (the slow request leak).
    async fn fetch_missing_logs(&mut self) {
        // Snapshot the (key, targets) requests first — borrowing `self.epochs` and
        // `self.committee_for` immutably — then borrow `self.resolver` (mutably) to
        // issue the fetches, so the two borrows never overlap.
        let mut requests: Vec<(DkgLogKey, NonEmptyVec<PeerPubkey>)> = Vec::new();
        // Live-ceremony epochs whose committee we could NOT read THIS tick (the transient
        // `committee_for→None` EVM race, same root as [965]). Their keys don't enter
        // `wanted` below, so WITHOUT preserving them the `retain` would CANCEL their
        // in-flight recovery fetches on a single bad read — resetting accumulated resolver
        // progress and risking a shareless stall if the read flaps (review [893]). A
        // genuinely dead epoch (finalized/swept) holds no ceremony and no heal, so it is
        // in NEITHER set → still cancelled, preserving the stale-tail prune.
        let mut unreadable: BTreeSet<u64> = BTreeSet::new();
        for (e, slot) in &self.epochs {
            // Nothing pinned yet ⇒ nothing to name; a heal names its `want`. The
            // slot's presence in the map IS its lifetime — `on_height`'s one
            // retention window is the only bound. Gating the fetch on a second,
            // narrower clock would leave a retained ceremony holding an agreed set
            // while no longer asking for the bodies its finalize needs. A heal's
            // target set is exactly `pinned(E) − held`, so a body NO peer holds simply
            // backs off with the epoch's age-out — never an unbounded storm.
            let missing: Vec<LogId> = match &slot.state {
                // A set that arrived while still dealing is fetched for from here
                // too: the bodies are what the finalize after the seal needs, and a
                // lagging clock is no reason to wait for them.
                EpochState::Agreed { ceremony, set }
                | EpochState::Dealing {
                    ceremony,
                    agreed: Some(set),
                } => {
                    let Some(roster) = (self.committee_for)(*e) else {
                        unreadable.insert(*e);
                        continue;
                    };
                    // A body we hold under exactly that hash is served, not fetched.
                    // No `me` special-case: a torn-own-seal node re-fetches its OWN
                    // pinned log like any missing body (the peers that recorded its
                    // broadcast serve it), re-passing the finalize gate once it is
                    // held.
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
            // Target each fetch at the roster (the known holders). `fetch_targeted`
            // narrows within `latest.primary`; a committee member's logs are served
            // from any peer that holds them. The holders are in `latest.primary`
            // during E-1: since 4.3 the beacon plane's `EpochTransition` tracks
            // `committee[E-2] ∪ committee[E-1] ∪ committee[E]` as PRIMARY on the SAME
            // `OracleHandle` the resolver's `Provider` reads, so committee[E] is in it
            // by the INCOMING-committee leg (the Active registry is tier 2 now and
            // would not cover it), and the E-1→E boundary `track(E)` re-includes
            // committee[E] explicitly (STEP-0 reachability).
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

        // The keys we still WANT in flight after this tick = exactly the ones just
        // (re)issued. Drop everything else from the resolver so a finalized/swept epoch's
        // fetches stop retrying forever. `retain` needs an owned `'static` predicate, so
        // move a snapshot set in. Re-issued keys are deduped by the resolver (in-flight),
        // so this is purely a prune of the stale tail.
        let wanted: BTreeSet<DkgLogKey> = requests.iter().map(|(k, _)| k.clone()).collect();
        self.resolver
            .retain(move |key| wanted.contains(key) || unreadable.contains(&key.epoch))
            .await;
        for (key, targets) in requests {
            self.resolver.fetch_targeted(key, targets).await;
        }
    }

    /// Drive every `Acquiring` epoch, once per `on_height` tick:
    ///
    /// 0. Read the artifact STORE for every decided epoch first
    ///    ([`Self::reconcile_with_store`]): a phase that needs the artifact and
    ///    finds it held takes it from there, without the network — the store is
    ///    the owner of the fact, and a hand-off to this actor that was lost
    ///    (`ArtifactBridge::adopt` is a bounded `try_send`) is not a lost fact. An
    ///    epoch it made finalizable is finalized on this same tick.
    /// 1. Every phase that still needs the epoch's artifact — ask peers for it
    ///    ([`PullArtifact`]): the member whose instance lost its body or which is
    ///    being recovered past its boundary (`ArtifactForCeremony`), the member with
    ///    a share file and no artifact (`ArtifactForShare`, §5.4's partial success),
    ///    a sat-out / unrecoverable member (it still verifies with `PK_E`), and the
    ///    NON-MEMBER of a mint epoch that needs `PK_E` to verify its certificates
    ///    (`ArtifactForKey`, I4 / R-121 / R-122 — until this existed nothing asked
    ///    for a non-member's key on the frontier: the epoch manager's repair sweep
    ///    excludes `epoch >= frontier` and the cert-inlet's `ensure_key` is
    ///    contractually network-free). An epoch at or past its boundary that is
    ///    still acquiring raises `Stalled{NoArtifact}` (once); before the boundary
    ///    the wait is the normal shape of an epoch whose agreement has not converged.
    ///
    ///    Cheap by construction and not by care: the pull short-circuits on a local
    ///    hit, `ArtifactPull` throttles to one network attempt per epoch per
    ///    [`PULL_MIN_INTERVAL`](crate::beacon::artifact::PULL_MIN_INTERVAL), the
    ///    spawn is de-duplicated per epoch by `pull_artifact`'s own inflight set,
    ///    and `NotYet` is a delivery rather than a peer fault. A member's live
    ///    ceremony never pulls (its instance delivers the artifact): pulling for an
    ///    epoch whose agreement this node is a participant in spends the
    ///    `BEACON_RESOLVER_CHANNEL` budget (16/s) that the SAME ceremony's
    ///    dealer-log fetches need — measured: without that split the honest
    ///    committee of `testbed::cert_inlet_tests` stops at the last block before
    ///    its change boundary.
    /// 2. `Acquiring(Logs)` — attempt the journal recompute for every heal holding
    ///    all of its pinned bodies ([`Self::try_recompute`]).
    fn drive_acquisition(&mut self, now: u64, rng: &mut impl CryptoRngCore) {
        if self.reconcile_with_store() {
            self.drive_finalization(rng);
        }
        // A `Sealed` epoch the chain has ENTERED without its artifact is no longer
        // waiting on an instance it can join: whatever kept the artifact from this
        // node (an instance that never got the certificate, a peer set that did not
        // reach it), the value is at the peers now, and the pull is the heal — the
        // same edge the boundary pull used to be.
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

    /// Bring every decided epoch up to what the artifact STORE holds for it — the
    /// store is the one owner of "the epoch's artifact" (§5.3, first-wins and
    /// quorum-checked before it stores), and the actor's own intake is a bounded
    /// channel whose loss must not lose the fact:
    ///
    /// - a phase standing on no artifact takes the store's through the ordinary
    ///   artifact edge ([`Self::apply_artifact`]) — the lost hand-off healed;
    /// - a phase standing on a value the store does not hold, or a store that
    ///   noted a DIVERGENT second value (`ArtifactStore::note_divergent`), is
    ///   `Conflict` — the second certified value reaches the verdict even when
    ///   its push to this actor was dropped.
    ///
    /// Returns whether a ceremony now stands on a set it did not before (the
    /// caller finalizes). A handful of lock reads per tick.
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
            // A `Conflict` without a key takes the store's artifact as its key
            // (F-01) through the artifact edge, which knows the phase; it is
            // never compared, its verdict is already final.
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

    /// Did the network re-mint the beacon key at `epoch`? THE ONE predicate the
    /// deal decision and the acquisition ([`Self::recover`]) read, so they cannot
    /// disagree about which epochs have an artifact at all.
    ///
    /// The chain's frozen `changed[epoch]` bit, plus the deterministic bootstrap
    /// exception stated once. `None` where the bit is unreadable: an undecided
    /// read is never a mint, and never a carry-forward either — the epoch stays
    /// undecided and the next tick re-asks.
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

    /// Attempt the scoped share recompute for each `Acquiring(Logs)` epoch whose
    /// `want` is empty (we now hold every pinned body). Loads the retained journal,
    /// runs the pinned-set-scoped [`recompute_scoped`], and adopts the share IFF it
    /// self-verifies against the pinned `Output` ([`validate_share_on_poly`]).
    ///
    /// On adopt: persist + store the share, seed the serve cache (so peers can
    /// still fetch this epoch's logs while it is in-window), evict the now-superseded
    /// journal, fire `share_notify` (re-runs the in-process promote edge) +
    /// `dkg_ceremony_ok`; the epoch is `Keyed`. On a RETRYABLE failure (torn/short
    /// journal, self-check fail, any other `Err`) the phase STAYS (keep fetching) and
    /// NO share is adopted — a wrong-but-valid-looking share can never leak into
    /// consensus (the FORK-SAFETY guard). The one TERMINAL failure,
    /// `MissingPlayerDealing`, is `Unrecoverable`.
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
                // NoFile/Torn: the inputs could not even be loaded, so this is
                // not an attempt (`attempted` stays false — the next tick re-reads
                // the file; one read, no crypto, and the latch keeps it to one
                // line). Said and gauged: `want` is empty, so no fetch is issued
                // and nothing else would ever surface the park.
                _ => {
                    self.stall(e, StallReason::HealFailed);
                    continue;
                }
            };
            // Once per change of inputs, and only once the inputs are LOADED: the
            // recompute is a pure function of the journal and the pinned set, so
            // re-running it on the next tick could only re-derive the same
            // refusal. A body landing in the journal re-arms it
            // (`ingest_recompute_log`); a persist failure re-arms it below (the
            // disk, not the inputs, is what failed).
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
            // THE FORK-SAFETY GATE IS NO LONGER HERE — it is inside `adopt_share`,
            // where П-3 puts it, so this path cannot be the one that has it while
            // another does not. What is left on this side is the split between a
            // RETRYABLE failure (keep the phase, keep fetching) and the one TERMINAL
            // one, which the gate cannot express.
            let share = match recomputed {
                Ok((_out, share)) => share,
                Err(DkgError::MissingPlayerDealing) => {
                    // Terminal, not pending. Every retry is provably futile (see
                    // `EpochState::Unrecoverable`), and leaving the phase pending makes an
                    // unrecoverable state indistinguishable from one still in flight
                    // — which, now that the live-epoch pull exists, is the state
                    // most share-less epochs are in.
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
                    // Retryable in principle (a body the journal does not hold yet
                    // can change it), said once per set of inputs by `attempted`,
                    // and LATCHED (`Stalled{HealFailed}`): with `want` empty no
                    // fetch is issued, so without the latch the park would be
                    // invisible until the sweep.
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
            // The PINNED outcome — the canonical one the gate checks against — with the
            // recomputed share.
            match self.adopt_share(e, &committee, outcome, share) {
                Ok(()) => {}
                Err(AdoptRefusal::OffPolynomial) => {
                    // The journal holds EVERY pinned body (`want` is empty) and the
                    // share it yields is still off the certified polynomial: no
                    // input is left to change, so the epoch is not healable here.
                    // Terminal, said so; a share-less member is a safe verifier.
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
                    // THE PHASE STAYS and the attempt is re-armed: the disk failed,
                    // not the inputs, and §5.4's retry is the next height tick.
                    self.stall(e, StallReason::PersistFailed);
                    self.set_recompute_attempted(e, false);
                    continue;
                }
            }
            // ONLY NOW is the journal superseded. This must not move above the adopt: the
            // two files are disjoint (`file_for` vs `journal_file_for`), so a crash
            // between `evict_journal` and `adopt_share`'s `persist` would leave the node
            // holding NEITHER — it would restart with `want = dealers()` and refetch every
            // pinned log from peers instead of reloading its own share. Reclaiming after
            // the write means the worst a crash costs is a journal that outlives its
            // usefulness until the next sweep, which is what the window is for.
            //
            // Seed the serve cache from the journal first, so this epoch's logs stay
            // servable to peers for the window; both touch only the journal, which the
            // adopt does not read.
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

    /// Handle one inbound resolver message — serve a `Produce` (a peer is fetching a
    /// dealer log from us) or ingest a `Deliver` (we fetched a dealer log). Runs in
    /// the single-threaded loop so it can touch ceremony state directly.
    async fn on_resolver_message(&mut self, msg: LogMessage, rng: &mut impl CryptoRngCore) {
        match msg {
            LogMessage::Produce { key, response } => {
                // DROP the responder (no send) when we don't hold the log → the
                // resolver sends an empty "no data" response → the requester retries
                // another peer. `serve_log` serves from the live ceremony / serve store
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
                if valid {
                    // The recorded set just widened with NO height tick behind it. The
                    // entry bar counts confirmations that COVER the proposed set, and
                    // `AnyGrowth` is the trigger that carries every width — including
                    // the intermediate ones `Decisive` skips and leaves to the next
                    // tick. On a live chain that tick is the backstop; on a halted
                    // chain there is no next tick, which is the case this edge exists
                    // for. It belongs on the path EVERY member runs, not just a
                    // leader's: `covering` has no self-exclusion and `entry_bar` is a
                    // bare count, so one node minting moves the bar by at most 1.
                    let minted = self.confirmations.mint(ConfirmTrigger::AnyGrowth);
                    self.broadcast_all(minted).await;
                }
            }
        }
        self.debug_assert_settled();
    }

    /// Serve the encoded `SignedDealerLog` held under exactly `{epoch, dealer, hash}`,
    /// or nothing — never a dealer's OTHER log. The LIVE ceremony's recorded
    /// `signed_logs` first — an epoch still collecting is only in memory, and only
    /// this actor holds it — then fall through to [`DealerLogStore`], which owns the
    /// cached and durable tiers (and the never-cache-a-negative rule that goes with
    /// them; see that module).
    ///
    /// Returns `None` when no tier holds that body → we drop the responder → the
    /// resolver sends an empty "no data" response → the requester retries another peer.
    fn serve_log(&mut self, key: &DkgLogKey) -> Option<Bytes> {
        let id: LogId = (key.dealer.clone(), key.hash);
        if let Some(signed) = self.ceremony(key.epoch).and_then(|c| c.signed_log(&id)) {
            return Some(signed.encode());
        }
        self.log_store.get(key.epoch, &id)
    }

    /// Ingest a `SignedDealerLog` delivered by the resolver for `{epoch, dealer, hash}`:
    /// decode + re-`check` + record via the ceremony's recording path, journal it,
    /// then drive finalize (a recovered log may complete the set). Returns the
    /// resolver `deliver` verdict — a TWO-VALUED API (`true` = clear the fetch + stop;
    /// `false` = block this peer + `add_retry` the key elsewhere; `resolver engine.rs`):
    /// - `true` — the log `check`-verified AND is exactly the REQUESTED body (signed by
    ///   `key.dealer`, hashing to `key.hash` — the fetch is now genuinely satisfied) or
    ///   is an honest duplicate; OR there is no live ceremony for this epoch (already
    ///   finalized/swept — the fetch is genuinely moot, so let it clear rather than
    ///   block an honest peer).
    /// - `false` — a genuine forgery (`check` fails), a valid log that is NOT the one
    ///   asked for (a peer answering a targeted fetch for `(D, h)` with D' or with D's
    ///   other body), OR an UNDECODABLE delivery.
    ///   An undecode must NOT return `true`: `true` marks the fetch SATISFIED (clears it),
    ///   so one peer serving garbage for `key` would permanently kill `key`'s recovery
    ///   with no log recorded. `false` keeps the fetch alive (`add_retry` → another peer).
    ///   The per-peer block `false` also incurs is an unavoidable side-effect of the
    ///   resolver's two-valued deliver API (there is no "no-data, retry, don't block"
    ///   verdict on the deliver path — that only exists when the SERVER returns no data);
    ///   it is bounded + acceptable because a committee peer serving undecodable bytes for
    ///   an EXPLICIT `{epoch,dealer,hash}` fetch is anomalous, and keeping `key` recoverable
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
        if let Some(c) = self.ceremony_mut(key.epoch) {
            // Bind the delivered log to the REQUESTED `(key.dealer, key.hash)`: a
            // forgery, a valid log for a different dealer and the dealer's other body
            // all return `false` (block + re-fetch `key`).
            let (accepted, step) = c.ingest_signed_log(&(key.dealer.clone(), key.hash), signed);
            if accepted {
                // Same attribution as the gossip path: the ceremony's `check`ed id,
                // not the key's. They are equal here (`ingest_signed_log` rejects
                // anything else), so one mechanism covers both sites.
                let durable = self.append_journal(key.epoch, step.journal);
                // The pinned body of a dealer whose OTHER body arrived by gossip
                // lands here — which is where a victim of a two-log dealer proves
                // the equivocation.
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
        // No LIVE ceremony (finalized, or swept at the boundary). If this epoch is a
        // heal (§8.11.1, `Acquiring(Logs)`), ingest the delivered log into its RETAINED
        // journal + `want` set and attempt the scoped recompute; otherwise the fetch is
        // genuinely moot → `true` (don't block an honest peer for a key WE no longer
        // need).
        if matches!(
            self.state(key.epoch),
            Some(EpochState::Acquiring(Acquire::Logs(_)))
        ) {
            return self.ingest_recompute_log(key, signed, rng);
        }
        true
    }

    /// Ingest a resolver-delivered `SignedDealerLog` for an `Acquiring(Logs)` epoch
    /// (no live ceremony): re-`check` it against the pinned `Info`
    /// and, iff it is exactly the REQUESTED body (signed by `key.dealer`, hashing to
    /// `key.hash`), JOURNAL it (retained for the window) + drop the id from `want`,
    /// then attempt the scoped recompute. Verdict mirrors the live-ceremony ingest:
    /// `true` = valid + exactly the body asked for (or already held); `false` = a
    /// forgery, a wrong-body answer, or an unverifiable committee read (block +
    /// re-fetch the key — never `true`, which would clear a still-needed fetch).
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
                // Only mark the body satisfied + attempt recompute once the log is a
                // DURABLE part of the journal the recompute reads; a non-durable write
                // keeps `want` (retry the fetch next tick).
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
    use commonware_utils::{Faults as _, N3f1, NZUsize};
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

    /// The `changed` bit of a fixture that has no chain: unreadable for every
    /// epoch. ONE shared closure, so a helper can tell the fixture's default from
    /// a bit a test set itself (`Arc::ptr_eq`, see `standalone_actor_at`).
    fn unreadable_bit() -> ChangedAt {
        static BIT: std::sync::OnceLock<ChangedAt> = std::sync::OnceLock::new();
        BIT.get_or_init(|| Arc::new(|_epoch: u64| None)).clone()
    }

    impl<R> Wiring<R> {
        /// Every edge but the resolver INERT: parked receivers (their senders are
        /// held in [`Fixture`], so `run` sees a plane that never speaks, not one
        /// that died), a no-op pull, a reader over an empty store, a scratch
        /// directory removed with the actor, a `changed` bit that reads as
        /// unreadable (the fixture has no chain; a test that decides an epoch
        /// past the bootstrap sets its own rule, as `standalone_actor_at` does),
        /// an unregistered clock and a pool under a test namespace. A test
        /// overrides the fields it exercises.
        fn inert(resolver: R) -> Self {
            let (resolver_tx, resolver_rx) = tokio::sync::mpsc::channel::<LogMessage>(1);
            let (pinned_tx, pinned_rx) = tokio::sync::mpsc::channel::<PinnedRequest>(1);
            let (artifacts_tx, artifacts_rx) = tokio::sync::mpsc::channel::<AgreedArtifact>(1);
            let (body_lost_tx, body_lost_rx) = tokio::sync::mpsc::channel::<u64>(1);
            let (agreement_tx, agreement_rx) = tokio::sync::mpsc::channel::<u64>(1);
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

    /// A resolver mock that records its in-flight fetch set so a test can assert
    /// `fetch_missing_logs` CANCELS dead fetches (`retain`) — exercising the [804]
    /// uncancelled-fetch-leak fix. `fetch_targeted` inserts the key; `retain` prunes the
    /// set by the predicate; `cancel`/`clear` mirror the trait.
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
    /// A POSITIVE deal window (`INTERVAL − DKG_MARGIN_BLOCKS = 10` ticks): the
    /// epoch-2 ceremony is created by `recover(2)` at epoch-1 start (height 30)
    /// and sealed at the seal deadline (40). It used to be `INTERVAL = 20`, a
    /// ZERO-width window where the start landed exactly AT the deadline — which
    /// only ever worked because a missing journal at the deadline started fresh
    /// (R-036, closed by 5.3-А1: at/after the deadline it sits out; that geometry
    /// is kept as ONE fixture,
    /// `at_a_zero_width_deal_window_a_missing_journal_at_the_deadline_sits_out`).
    /// Every SEAL_DEADLINE/BOUNDARY-relative drive below keeps its meaning.
    /// (Production uses a much larger interval; this is the minimal test
    /// geometry, not a protocol constraint.)
    const INTERVAL: u64 = 30;
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

    /// A stand-in for the epoch-key agreement plane, wired on the actor's real
    /// seams: it takes the dealing-closed announcement, and once a quorum of
    /// dealer-log hashes has been published into `recorded` it hands that exact set
    /// back as an agreed artifact.
    ///
    /// What the real plane adds — agreeing ONE set across the committee, under a
    /// quorum certificate — is covered directly in [`crate::beacon::dkg_agree`] and
    /// [`crate::beacon::dkg_engine`]. The finalize path takes only the set, and
    /// these tests are about what the actor does with it.
    ///
    /// `recorded` is the caller's choice, and a PER-NODE index is faithful as far as
    /// it goes: the real plane's leader also proposes from its own index
    /// ([`crate::beacon::dkg_agree`]'s `attempt_proposal` reads `local_set(&self.recorded, ..)`).
    /// What the stub does not model is what happens when that leader has nothing to
    /// put up — the real plane refuses, the view is nullified, and the NEXT leader
    /// proposes from ITS index. The stub has one leader and no certification, so it
    /// stalls there forever. A test whose subject is a node that deliberately claims
    /// less than it holds must therefore hand the whole committee ONE index: it
    /// stands in for leader rotation, NOT for a property the real plane lacks.
    ///
    /// It waits for the quorum on its OWN clock rather than on the next
    /// announcement, because that is the property the real plane has and one of
    /// these tests turns on: a node whose height feed has frozen still gets its
    /// epoch agreed. One target at a time is enough — no test here runs two.
    ///
    /// `certified` is the one thing the real plane has that a per-node stub does
    /// not: ONE value per epoch across the committee. Stubs handed the same map
    /// deliver the FIRST set any of them certified — so a node that holds less than
    /// a quorum still receives the set its peers agreed, exactly as its instance
    /// would, and then fetches the bodies that set names by hash. A fresh map per
    /// spawn is the per-node behaviour.
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
        // The stub derives the group key over the set it certifies through the
        // actor's own pinned-set seam, exactly as an instance's `verify` does — so
        // the artifact carries the polynomial the actor's share must lie on.
        let (pinned_tx, pinned_rx) = tokio::sync::mpsc::channel::<PinnedRequest>(16);
        let quorum = <commonware_utils::N3f1 as commonware_utils::Faults>::quorum(n) as usize;
        drop(ctx.with_label("stub_agreement").spawn(move |c| async move {
            let mut agreed: BTreeSet<u64> = BTreeSet::new();
            while let Some(epoch) = announce_rx.recv().await {
                if agreed.contains(&epoch) {
                    continue;
                }
                // The set and its key, once certified by ANY node of the cohort, are
                // what every later node is handed (a restarted node holds too few
                // bodies to derive the key itself — the real instance fetches a
                // proposal's bodies to verify it; here the certifier's key stands in).
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

    /// `spawn_dealer` with an explicit on-disk `share_dir` (so the journal/share
    /// persist) + an rng seed (so a re-spawn after a restart uses fresh randomness,
    /// proving the resume does NOT depend on a deterministic re-deal).
    /// [`spawn_dealer_at`] for the callers that do not need the adopted-`Output`
    /// record — which is all of them but `run_reveal_check`.
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
    ) -> tokio::sync::mpsc::Sender<u64> {
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
        tokio::sync::mpsc::Sender<u64>,
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
        // The contract's rule over this fixture's single committee — see
        // `standalone_actor_at`. A single committee never changes, so what
        // starts a ceremony here is the unconditional bootstrap mint, exactly
        // as on-chain.
        wiring.changed = Arc::new(|_epoch: u64| Some(false));
        // The pool this dealer counts peers' `ShareConfirm`s into — a fixture
        // dealer mints and receives them for real (`Confirmations::mint` over
        // `recorded`), and one test pins that as a fact (`run_reveal_check`).
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
        // The OUTPUT of whatever this dealer adopts, for the assertions that need the
        // ceremony's own `Output` — see `DkgActor::adopted_outcomes`.
        let adopted = actor.adopted_outcomes.clone();
        let (height_tx, height_rx) = tokio::sync::mpsc::channel::<u64>(256);
        let rng = StdRng::seed_from_u64(rng_seed);
        drop(
            ctx.with_label("dealer")
                .spawn(move |_c| async move { actor.run(height_rx, rng).await }),
        );
        (height_tx, adopted, confirms)
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
        certified: Certified,
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
                // The contract's rule over this fixture's single committee — see
                // `standalone_actor_at`.
                wiring.changed = Arc::new(|_epoch: u64| Some(false));
                wiring
            },
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
    ///   LATE, so peers' dealings reach them BEFORE their own start (`recover`) — the
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
                // `late_lag`), so peers' dealings arrive before their own start.
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
        // step used to run before the start inserted it, so the seal landed one tick after
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
    /// two EARLY dealers' `Commitment`+`Share` BEFORE its own start. PRE-fix
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
            "early peer dealings that race ahead of the start must be buffered + \
             drained (not dropped) so every dealer is acked and the victim seeds"
        );
    }

    /// Leak regression (H2). A ceremony whose committee has a quorum it never reaches
    /// (here only the victim deals; its three peers never reveal) is SEALED at the
    /// deadline but no agreed set ever arrives, so `drive_finalization` never removes
    /// it. It has to survive the epoch boundary — an agreement that has not converged
    /// yet still asks this actor for the bodies — and then be evicted once the epoch
    /// ages out of the retention window, otherwise it lingers for the life of the
    /// process. Drives `on_height` directly (not `run`) to inspect the internal state.
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

            // Through the seal deadline: committee[2] enters (height 10), seals (11),
            // then stalls (1 valid log < quorum 3, so `ready()` never holds).
            for h in 0..=(SEAL_DEADLINE + 2) {
                actor.on_height(h, &mut arng).await;
            }
            assert!(
                actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .is_some_and(|c| c.dealing_closed()),
                "precondition: committee[2] must be sealed-but-stalled before the boundary"
            );

            // Cross the epoch-2 boundary: the ceremony stays, because an agreement
            // for epoch 2 can still be running there.
            for h in (SEAL_DEADLINE + 3)..=(BOUNDARY + 1) {
                actor.on_height(h, &mut arng).await;
            }
            assert!(
                actor.ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH).is_some(),
                "the chain entering the target epoch must not sweep its ceremony"
            );

            // One retention window on: the sweep must evict the stalled entry.
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

            // A real, decodable `Commitment` dealing tagged for epoch 1 (bufferable
            // with no height tick drained yet ⇒ `height_now() = 0` ⇒ now=0, so
            // 0 < 1 ≤ now+2). The body is
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
                committee_for,
                Arc::new(RwLock::new(BTreeMap::new())),
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                ShareState::Plaintext,
                Wiring::standalone(),
            );
            // Drive node-0 to START its epoch-2 ceremony but NOT seal it (so
            // `drive_finalization`'s `sealed` guard never finalizes+evicts under us —
            // node-0's own `view` lacks the peers' private dealings, so a real finalize
            // would `MissingPlayerDealing` anyway; this test isolates the recovery
            // ingest path, the resolver `Consumer::deliver` half).
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

            // `peer_logs[i]` is `keys[i+1]`'s sealed log; build the matching
            // `{epoch, dealer, hash}` key per log (`ingest_log` BINDS the delivered
            // log to the requested body). `dealer0` = the first peer dealer.
            let dealer0 = keys[1].public_key();
            let valid_key0 = DkgLogKey {
                epoch: DETERMINISTIC_BOOTSTRAP_EPOCH,
                dealer: dealer0.clone(),
                hash: log_hash(&peer_logs[0]),
            };

            // A wrong-epoch delivery (no live ceremony for epoch 3) is honest — it
            // returns `true` (don't block the peer) and touches nothing.
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
                actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .expect("ceremony")
                    .recorded_log_count(),
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
                actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .expect("ceremony")
                    .recorded_log_count(),
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
                actor
                    .ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH)
                    .expect("ceremony")
                    .recorded_log_count(),
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

    /// Positive-only serve cache ([965]/[954]) — a cold-miss serve for an UNSERVABLE epoch
    /// (absent / Torn journal) caches NOTHING: it returns `None` and leaves the serve
    /// store's cache untouched. So a Byzantine peer's distinct far-future `key.epoch`s (no
    /// journal → empty) can never accumulate negative entries → it stays bounded ([954]).
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
            // [954] bound: many distinct attacker-controlled far-future epochs (no journal)
            // never grow the cache.
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
            let (dealer, signed) = logs
                .iter()
                .find(|(pk, _)| *pk == keys[1].public_key())
                .expect("keys[1] sealed");
            let key = DkgLogKey {
                epoch: 2,
                dealer: dealer.clone(),
                hash: log_hash(signed),
            };

            // Committee unreadable → empty cold-load → None, and CRUCIALLY not cached.
            assert!(
                actor.serve_log(&key).is_none(),
                "transient None serves no log"
            );
            assert!(
                actor.log_store.cached(2).is_none(),
                "the transient-None empty result is NOT cached → no permanent poison ([965])"
            );

            // Committee now readable → the SAME serve re-parses and serves the log.
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

    /// [804] uncancelled-fetch leak — `fetch_missing_logs` CANCELS the resolver's
    /// in-flight fetches for an epoch that no longer has an open ceremony (finalized or
    /// swept), so the resolver stops re-issuing dead `{epoch,dealer,hash}` keys
    /// forever. An open ceremony holding an agreed set issues fetches for the pinned
    /// bodies it lacks, by hash; once the ceremony leaves `ceremonies`, the next
    /// `fetch_missing_logs` `retain`s them away.
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

            // Inject an OPEN shorthanded ceremony (node-0 started but no peer logs)
            // holding an agreed set that pins a body at every peer seat, so
            // `fetch_missing_logs` issues fetches for the 3 missing pinned bodies.
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

            // Finalize/sweep the ceremony (remove it), then re-run fetch_missing_logs:
            // with no open ceremony, `retain` must CANCEL every now-dead fetch.
            actor.epochs.clear();
            actor.fetch_missing_logs().await;
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
            // An agreed set pinning a body at every peer seat — what the fetch asks by.
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

            // Committee readable → fetches issued.
            actor.fetch_missing_logs().await;
            assert!(
                !in_flight.lock().unwrap().is_empty(),
                "an open ceremony with an agreed set issues fetches for the pinned bodies it lacks"
            );

            // Transient committee read failure while the ceremony is STILL live: the
            // in-flight fetches must be PRESERVED, not cancelled ([893]).
            readable.store(false, std::sync::atomic::Ordering::Relaxed);
            actor.fetch_missing_logs().await;
            assert!(
                !in_flight.lock().unwrap().is_empty(),
                "a transient committee_for->None for a LIVE ceremony preserves its in-flight fetches ([893])"
            );
        });
    }

    /// Serve-after-finalize. A node that FINALIZED its ceremony but has NOT yet crossed
    /// the epoch boundary must still serve a peer's recorded log from the eager
    /// serve store (no journal read, no `check`), so a late-restarting peer can
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
                committee_for,
                Arc::new(RwLock::new(BTreeMap::new())),
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                ShareState::Plaintext,
                Wiring::standalone(),
            );
            // Inject node-0's fully-recorded ceremony (already sealed in the queue
            // drive ⇒ `me ∈ recorded`, so the derived finalize gate passes), then drive
            // finalize (pre-boundary). After finalize node-0 holds no live ceremony but
            // DOES hold the serve-store copy for the epoch.
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

            // SERVE-AFTER-FINALIZE: a peer's log is still served from the serve cache.
            let key = DkgLogKey {
                epoch: DETERMINISTIC_BOOTSTRAP_EPOCH,
                dealer: peer.clone(),
                hash: peer_hash,
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

    /// R-002 on the actor's seams: a dealer of which this node holds a DIFFERENT
    /// body than the pinned one is refetched BY THE PINNED HASH — the per-dealer
    /// form called it held and never asked, which is how one Byzantine dealer left
    /// an honest member shareless.
    ///
    /// Node 0 runs the committee[2] ceremony but receives, at dealer 1's seat, a
    /// second valid log (`log2`) instead of the one everyone else recorded
    /// (`log1`). The agreed set pins `log1` there. Then:
    /// - `fetch_missing_logs` issues exactly ONE key: `(2, dealer1, h1)` — the
    ///   pinned body, not the dealer; a fetch keyed by a zero hash (M2) is neither
    ///   this key nor one any peer can answer;
    /// - `serve_log` answers `(2, dealer1, h2)` (held) and nothing for `h1` or a
    ///   zero hash — a server hands out the exact body or nothing;
    /// - delivering `log1` under `(2, dealer1, h1)` is accepted, proves the
    ///   equivocation (counter + evidence), completes the pinned set and finalizes
    ///   the share; delivering it under `(2, dealer1, h2)` — the OTHER body's key —
    ///   is refused (`deliver → false`), and the next tick asks for nothing more.
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

            // Dealer 1's SECOND valid log over the same `Info` (independent
            // polynomial, no acks ⇒ `TooManyReveals`, still `check`-valid) — what
            // `testbed::byzantine_roles::TwoRevealSender` sends the victim.
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

            // Run the 4 ceremonies to sealed; when dealer 1's `Reveal` reaches node 0,
            // hand it `log2` instead.
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
            // The agreed set: what nodes 1..3 recorded — `log1` at dealer 1's seat.
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
            // The key the certifier derived over the pinned set — node 2 holds every
            // pinned body, node 0 does not (that is the case).
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

            // (1) The fetch names the PINNED body of dealer 1, and only that.
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

            // (2) The server side: the exact body or nothing.
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

            // (3) The delivery: `log1` under the OTHER body's key is refused ...
            assert!(
                !actor.ingest_log(&key_h2, log1.encode(), &mut arng).await,
                "a valid log that is not the body asked for does not satisfy the fetch"
            );
            assert_eq!(actor.metrics.dkg_dealer_equivocation.get(), 0);
            // ... and under its own key it is recorded, proves the pair, and finalizes.
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
            // The evidence outlives the ceremony in the serve store: both bodies servable.
            assert!(actor.serve_log(&wanted).is_some() && actor.serve_log(&key_h2).is_some());
            actor.fetch_missing_logs().await;
            assert!(
                in_flight.lock().unwrap().is_empty(),
                "nothing left to fetch once the pinned set is held"
            );

            // (4) The evidence OUTLIVES the ceremony: the actor's own copy names the
            // pair after finalize, and only the retention sweep removes it.
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

    /// The evidence pair survives a RESTART: a journal carrying a
    /// `DealerEquivocation` record resumes into a ceremony that holds the pair, and
    /// the actor copies it out on the resume edge — so the copy that outlives the
    /// ceremony exists on a restarted node too, not only on the one that proved it.
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
            // Dealer 1's sealed log (from the journal) and a SECOND valid one.
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
            // The restart proper: `recover(2)` resumes the journal (player-only,
            // past the seal deadline).
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

    use crate::beacon::log_store::COLD_PARSE_COUNT;

    /// Serializes the tests that touch the process-global `COLD_PARSE_COUNT` (the two that
    /// read it + the transient-None test that increments it via a re-parse) so a parallel
    /// run cannot interleave their parse counts.
    static COLD_PARSE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Build a STANDALONE actor (no network drive) over `committee` with the given
    /// `share_dir` and a COLD serve store, mirroring the production construction.
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

    /// [`standalone_actor`] over the edges a test wires itself (`wiring`) — the
    /// pool it reads confirmations from, the agreement halves it plays, the clock
    /// it gauges.
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

    /// Like [`standalone_actor`] but takes an explicit `committee_for` closure, so a test
    /// can model a TRANSIENT `committee_for→None` EVM read race (review [965]).
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

    /// THE ONE standalone construction: [`standalone_actor_cf`] over an explicit
    /// epoch `interval` — the one knob a test of the deal-window GEOMETRY turns
    /// (`INTERVAL` is this module's, a positive window; `DKG_MARGIN_BLOCKS` is a
    /// zero-width one) — and the test's own `wiring`. Two of its edges are set
    /// HERE from the fixture: `share_dir` when the test names one (a restart test
    /// re-opens the same directory), and `changed` — THE CEREMONY-START
    /// DECISION'S ONE INPUT (Д-7) — when `wiring` still carries the fixture's
    /// unreadable default (`unreadable_bit`; a bit the test set on its wiring
    /// stands), stood in for by the rule the CONTRACT applies: `changed[e] =
    /// committee[e] != committee[e−1]`, over this fixture's own committee reader.
    /// Production reads the bit the contract wrote; a fixture that has no
    /// contract computes what it would have written from the same committees, so
    /// every test keeps the intent it had when the decision was a roster
    /// comparison — and the comparison lives in ONE place instead of at the
    /// decision.
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

    /// The refusals the shared ingress counter holds right now for the BEACON
    /// channel under `reason`, DRAINED (the debugging snapshot resets what it
    /// reads, so each call answers "since the last call").
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

    /// The BEACON ingress rule on a CONFIRMATION, measured by what the frame COSTS
    /// this node and by what the counter says about it. `now` = 5, the actionable
    /// window `[5, 7]`, `keys[4]` registered on the plane and in no committee.
    ///
    /// Three frames, one counter of committee-record lookups and the shared
    /// `dpos_ingress_dropped_total{channel="beacon"}`:
    ///  * a `Confirm` naming epoch 10^9 — no lookup at all, refused `epoch`. This
    ///    is the E4-12 / R-023 shape: before 4.3 `on_confirm` resolved
    ///    `committee_for(target_epoch)` for ANY epoch from ANY tracked sender, so
    ///    this frame bought one state read per message and the counter here would
    ///    read 1.
    ///  * the same frame from a sender with NO SEAT in epoch 6 — exactly ONE
    ///    lookup, the roster the consumer keys on anyway, and then `no_seat`:
    ///    nothing downstream, no `pending` slot, no pool record. This is the
    ///    consumer's own refusal (5.3-В, second round): the actor holds no
    ///    membership opinion of its own between the channel's gate and the
    ///    consumer, so the ONE read is the price of asking the roster, and it is
    ///    the same read a member's frame costs.
    ///  * the same frame from a MEMBER — admitted, so the epoch is resolved ONCE,
    ///    for the confirmation itself, and nothing is counted.
    ///
    /// Falsifier: the first count moving off 0; the seatless frame reaching
    /// `pending` or the pool, or not being COUNTED (M2 of 5.3-В round 2: `no_seat`
    /// dropped without `refuse` — the counter stays at 0 and this test is red).
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
                // Epoch 5's first height ⇒ `now` = epoch 5; the actionable window is
                // [5, 7].
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

                // (1) An epoch nobody here could act on: refused before any read.
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

                // (2) In-window epoch, sender in no committee: the consumer reads the
                // roster ONCE and refuses the sender it finds no seat for.
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

                // (3) The same frame from a member is admitted — the check refuses a
                // sender, not the feature — and the ONE read is the confirmation's own.
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

    /// The consumer's seat check on CEREMONY traffic, at the three places a
    /// ceremony frame keyed by its sender is taken in:
    ///  * the start-race BUFFER — no ceremony for epoch 2 yet. While
    ///    `committee[2]` is not readable, a dealing is buffered on its epoch alone
    ///    (there is no record to ask), stranger's and dealer's alike; once the
    ///    record IS readable, the stranger's next dealing is refused `no_seat` at
    ///    the buffer and occupies no slot (5.3-В round 3, E-09);
    ///  * the start-race DRAIN — what was buffered before the record was readable
    ///    is refused `no_seat` the moment `decide` drains it into the
    ///    ceremony that has a roster;
    ///  * the live DISPATCH — a ceremony for epoch 2 is running; an `Ack` and a
    ///    `Commitment` from a sender with no seat are refused `no_seat` before
    ///    `handle`, so the ceremony never buffers a stranger's half-dealing; the
    ///    same `Commitment` from a seated dealer is handled.
    ///
    /// Model B is the whole reason one predicate serves all three: dealers ==
    /// players == `committee[epoch]` (`ceremony::info_for`), so "no seat" is the
    /// same answer for an ack (a player's frame) and a dealing (a dealer's).
    /// Commonware gives that answer silently (`Player::dealer_message` → `None`,
    /// `Dealer::receive_player_ack` → `UnknownPlayer`); the counter is what this
    /// test pins.
    ///
    /// Falsifier (M2 of 5.3-В round 3): the buffer's seat check removed — the
    /// stranger's second dealing is buffered instead of refused, the count before
    /// the drain reads 0 and not 1; either other `no_seat` refusal removed — the
    /// stranger's dealing lands in the ceremony's buffer, or the drain replays it,
    /// and the count is short by one.
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
                // `committee[2]` is unreadable until `readable` flips: the start-race's
                // own shape (this node is behind the dealer and has no record yet).
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

                // A real, decodable dealing for epoch 2 from `keys[1]`. The body is
                // never verified before buffering or before the seat check, so the
                // same bytes stand in for any sender's dealing.
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

                // THE BUFFER, record unreadable. No height tick yet ⇒ `now` = 0 and
                // a dealing for 2 is bufferable; with no record to ask, one from the
                // stranger and one from the dealer are both buffered.
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

                // THE BUFFER, record readable. The stranger deals again: refused at
                // the buffer, and the buffer is exactly as it was.
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

                // THE DRAIN. Epoch 1's first height ⇒ `now` = 1 ⇒ `recover(2)`: the
                // bootstrap epoch always deals, the ceremony starts and the buffer
                // drains into it; the stranger's slot from before the record was
                // readable is refused there.
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

                // THE LIVE DISPATCH. An ack and a commitment from the stranger, then
                // the commitment again from the dealer (a re-receipt: handled, not
                // refused).
                let ack: DkgBody = {
                    let (mut cer, _) =
                        DkgCeremony::start(ns, 2, committee.clone(), keys[2].clone())
                            .expect("start");
                    let (_cer1, step1) =
                        DkgCeremony::start(ns, 2, committee.clone(), keys[1].clone())
                            .expect("start");
                    // Feed keys[1]'s dealing to keys[2]'s ceremony to obtain a real ack.
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

    /// A frame this actor cannot decode is REFUSED, and counted `undecodable`
    /// (5.3-В round 3, E-05): `refuse` is the one place a beacon frame is refused,
    /// so the three decode failures on the ingress path go through it too —
    ///  1. not a beacon frame at all (`BeaconMessage::read` fails on the tag);
    ///  2. a beacon frame too short to carry the eight-byte epoch (the peek);
    ///  3. a beacon frame whose epoch is actionable but whose body is not a
    ///     `DkgMsg` (an unknown body tag).
    ///
    /// And the ORDER is pinned by a fourth frame: a body that would fail to decode
    /// behind an epoch outside the window is refused `epoch`, never `undecodable`
    /// — the epoch peek is the cost gate and runs before the body decode.
    ///
    /// Falsifier (M3 of 5.3-В round 3): any one of the three decode failures back
    /// to a bare `return` — the `undecodable` count reads 2, not 3.
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
                // Epoch 5's first height ⇒ `now` = 5; the window is [5, 7].
                actor.on_height(INTERVAL * 5, &mut arng).await;
                assert_eq!(actor.epoch_of(actor.height_now()), 5);
                let _ = beacon_refusals(&snap, "undecodable");
                let _ = beacon_refusals(&snap, "epoch");
                let sender = keys[1].public_key();

                // (1) Not a beacon frame: an unknown wire tag.
                actor.on_message(sender.clone(), &[0xFF], &mut arng).await;
                assert_eq!(beacon_refusals(&snap, "undecodable"), 1, "unknown wire tag");

                // (2) A beacon frame too short to carry an epoch.
                let short = BeaconMessage::Dkg(Bytes::from_static(&[1, 2, 3])).encode();
                actor.on_message(sender.clone(), &short, &mut arng).await;
                assert_eq!(beacon_refusals(&snap, "undecodable"), 1, "no epoch to peek");

                // (3) An actionable epoch in front of a body that is no `DkgMsg`.
                let bad_body = {
                    let mut payload = 6u64.encode().to_vec();
                    payload.push(0xFF); // no such body tag
                    BeaconMessage::Dkg(Bytes::from(payload)).encode()
                };
                actor.on_message(sender.clone(), &bad_body, &mut arng).await;
                assert_eq!(beacon_refusals(&snap, "undecodable"), 1, "unknown body tag");
                assert_eq!(beacon_refusals(&snap, "epoch"), 0);

                // (4) The same bad body behind an epoch outside the window: the epoch
                // gate answers first, so this is `epoch`, not `undecodable`.
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

    /// ONE ingress window, read at its three sites. `now` = 5 (height
    /// `INTERVAL * 5`): `[5, 7]` is the window, and
    ///  * `epoch_is_actionable` admits 5..=7 and refuses 8 (and 4, absent a live
    ///    ceremony there);
    ///  * `is_bufferable` admits a dealing for 6 and 7, refuses 8 — and refuses 5
    ///    by its OWN rule (`epoch > now`), which the shared window does not carry;
    ///  * `on_confirm` reads the window before the committee: a confirmation for 8
    ///    costs no read, one for 7 costs exactly the confirmation's own.
    ///
    /// Falsifier (M2 of 5.3-В): the window widened to `now + 3` at any one site —
    /// 8 admitted there, and the three rules no longer one rule.
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
            // The epochs ahead stay UNDECIDED (their `changed` bit unreadable), which
            // is the start-race's own state: a decided epoch is dealing already or
            // will never deal, and neither buffers.
            actor.changed = Arc::new(|epoch: u64| (epoch <= 5).then_some(false));
            let mut arng = StdRng::seed_from_u64(0x5304);
            actor.on_height(INTERVAL * 5, &mut arng).await;
            assert_eq!(actor.epoch_of(actor.height_now()), 5);

            // Site 1: the cost gate.
            assert!(actor.epoch_is_actionable(5) && actor.epoch_is_actionable(7));
            assert!(!actor.epoch_is_actionable(4) && !actor.epoch_is_actionable(8));

            // Site 2: the start-race buffer, with its own `epoch > now` on top. The
            // body is never verified before buffering, so any dealing stands in.
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

            // Site 3: the entry-bar window, before the committee read.
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

    /// The resolver engine's exit STOPS the actor (5.3-Г1, map decision 4). The
    /// engine is the one holder of the actor's inbound `LogMessage` sender; when
    /// that sender is gone `run` returns — it does not park the branch and carry
    /// on "gossip-only", because the engine is a supervised sibling of the actor
    /// and the node is going down with it. The height sink and the gossip channel
    /// are both still OPEN here, so nothing but the resolver arm can end the loop.
    ///
    /// Falsifier (M3 of 5.3-Г1): the arm back to `resolver_rx = None; resolver =
    /// None` — `run` never returns and this test times out.
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
            let (height_tx, height_rx) = tokio::sync::mpsc::channel::<u64>(4);
            let run = ctx.with_label("actor").spawn(move |_| async move {
                actor.run(height_rx, StdRng::seed_from_u64(0x5306)).await
            });
            // The loop is up and parked on its four arms; now the engine "dies".
            ctx.sleep(Duration::from_millis(50)).await;
            drop(resolver_tx);
            tokio::select! {
                res = run => res.expect("the actor task must return, not fail"),
                _ = ctx.sleep(Duration::from_secs(5)) => panic!(
                    "the actor kept running after its resolver engine exited — the \
                     gossip-only degradation is back"
                ),
            }
            // Held open through the whole run so that neither could have been the
            // reason the loop ended.
            drop(height_tx);
        });
    }

    /// `dpos_dkg_clock_height` is the actor's clamp, not any one feeder's write.
    ///
    /// The inlet-fed shape is the one that used to lie: the cert inlet pushed the
    /// verified upstream frontier into the height channel and gauged nothing,
    /// while the finalized poller gauged its own lagging `fin + K`. The published
    /// clock then sat below the clock the ceremony geometry actually ran on and
    /// the whole reported lag was spurious. Feeding the LOW value last is what
    /// distinguishes "the gauge is the max" from "the gauge is whoever wrote
    /// last".
    #[test]
    fn the_dkg_clock_gauge_is_the_actors_max_over_every_feeder() {
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

            // The upstream cert frontier, well ahead of local execution.
            actor.on_height(1000, &mut arng).await;
            assert_eq!(clock.snapshot().1, 1000);

            // The finalized poller's `fin + K`, still catching up. The clock does
            // not rewind and neither does the gauge.
            actor.on_height(303, &mut arng).await;
            assert_eq!(actor.last_height, Some(1000));
            assert_eq!(clock.snapshot().1, 1000);

            clock.record_ordering_tip(1002);
            assert_eq!(clock.snapshot().2, 2, "both halves reported ⇒ a real lag");
        });
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
    /// The recording edge gossips only the two widths a leader cannot wait a block
    /// for; every intermediate one rides the next height tick.
    ///
    /// Each intermediate width is superseded by the next log in the same `Reveal`
    /// burst and `ConfirmPool::record` keeps only the widest a peer ever sees, so
    /// gossiping them cost the committee bytes for statements nothing counted. What
    /// must not be lost is the WIDEST set: it goes out on the recording edge when it
    /// completes the committee, and otherwise on the next `AnyGrowth` tick — which is
    /// what this asserts, at the width a stalled dealer set actually stops on.
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
            // n = 7 ⇒ quorum 5, so widths 6 and 7 are both above it and only 7
            // completes the committee.
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

            // A dealer set that stalls here — the case that must not be lost. The
            // height tick is the backstop that carries it.
            let minted = actor.confirmations.mint(ConfirmTrigger::AnyGrowth);
            assert_eq!(minted.len(), 1, "the tick must flush the widest set");
            let DkgBody::Confirm(flushed) = &minted[0].msg.body else {
                panic!("the minted message is not a confirmation");
            };
            assert_eq!(flushed.recorded, logs[..QUORUM + 1].to_vec());
            assert_eq!(pool.covering(TARGET, &logs[..QUORUM + 1]).len(), 1);

            // The complete set does not wait for a tick: nothing can supersede it,
            // and it is what the all-present case converges on within milliseconds.
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

    /// Also the standing pin on the CLAIMED-WIDTH rule (`previous >= confirmed.len()`),
    /// which now lives in [`Confirmations`]: an unchanged set mints NOTHING, and the
    /// next genuine widening mints again. Kept here rather than duplicated at the new
    /// seam — this exercises it through the actor's real recording path, which is where
    /// a regression would actually show up.
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
            // Put the actor's epoch clock where TARGET is inside `[now, now + 2]`:
            // since 4.3 `on_confirm` refuses a confirmation outside that window
            // BEFORE resolving its committee, and this test drives `on_confirm`
            // directly rather than through `on_height` (whose side effects — a
            // `recover` for `now + 1` — would be a second thing under test).
            // Epoch 5's first height: TARGET 6 is in.
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

            // Unchanged set, no re-mint: the statement only changes when a log is
            // recorded, so re-gossiping it every tick would be noise.
            assert!(actor
                .confirmations
                .mint(ConfirmTrigger::AnyGrowth)
                .is_empty());

            // Grown again: a new, wider statement, and the pool keeps the wider one.
            recorded
                .write()
                .unwrap()
                .insert(TARGET, logs.iter().copied().collect());
            assert_eq!(actor.confirmations.mint(ConfirmTrigger::AnyGrowth).len(), 1);
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

            // Per-target scratch on the ceremony's own lifetime: the confirmations
            // survive the chain entering the target (its agreement can still be
            // running) and go once the epoch ages out of the retention window.
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

    /// A confirmation that beats the actor's FIRST height tick is counted, and the
    /// window binds the moment a tick lands.
    ///
    /// `last_height` is `None` until `on_height` drains its first value, and the
    /// actor is spawned before that: the plane builds it after the geometry freeze
    /// while the poller's tick sits buffered, and `tokio::select!` picks a ready
    /// branch at random, so a peer's frame can be served first. Reading that state
    /// as height 0 would put `[now, now+2]` at `[0, 2]` and refuse every
    /// confirmation on any chain past epoch 2 — permanently, because
    /// `Confirmations::mint` is edge-triggered on width growth and never re-issues
    /// a full-width statement. This node's entry bar would then undercount one
    /// member for the whole epoch, with nothing to heal it.
    ///
    /// Falsifier: the pre-tick confirmation not landing in the pool (the `0` floor
    /// is back), or the post-tick out-of-window one landing (the window stopped
    /// binding once there IS a clock).
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
            // Far outside `[0, 2]`: this is the epoch a running chain would be in
            // when this node's actor comes up, and the whole point of the case.
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

            // One tick, and the window binds from there: epoch 5's first height, and
            // TARGET 40 is far outside `[5, 7]`.
            let mut arng = StdRng::seed_from_u64(0x4502);
            actor.on_height(INTERVAL * 5, &mut arng).await;
            assert_eq!(actor.last_height, Some(INTERVAL * 5));
            actor.on_confirm(TARGET, &keys[2].public_key(), confirm(&keys[2], TARGET));
            assert_eq!(
                pool.covering(TARGET, &logs).len(),
                1,
                "with a clock, `[now, now+2]` must still refuse an out-of-window epoch"
            );

            // ...and an epoch INSIDE the window still lands, so the refusal above is
            // the window and not the tick.
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

    /// The agreement plane keeps agreeing after the chain has entered the target
    /// epoch, so the ceremony it derives against outlives that boundary: a swept
    /// ceremony turns `derive_pinned` into a permanent `Unavailable`, which parks
    /// every `verify` and leaves `build_proposal` with nothing to pin. The instance
    /// is aborted when the epoch manager enters `target + 1`
    /// (`epoch_manager::prune_agreements`), a full epoch inside this window.
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

            // The chain reaches the target epoch and runs a whole epoch past it.
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

            // Past every window on the target, derived from the window itself.
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

    /// v42 shrink regression: a SHRINK must start a ceremony. `recover`'s
    /// change-test reads `committee[target−1]` against `committee[target]`, and a
    /// change-test that could not see the shrink left `getDkgQual` empty ⇒ infinite
    /// deferral (v42: 10→8 shrink, zero dealing on every node for 3 boundaries).
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
            // The incoming committee is the first 8 — a genuine SHRINK from 10.
            let incoming = Set::from_iter_dedup(all_keys[..8].iter().map(|k| k.public_key()));
            oracle.manager().track(0, outgoing.clone()).await;

            const TARGET: u64 = 5; // non-bootstrap
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

    /// Carry-forward control: a genuine NO-CHANGE epoch
    /// (`committee[t−1] == committee[t]`) must NOT start a ceremony — the key carries
    /// forward. Guards the change-test from firing spuriously.
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

            const TARGET: u64 = 5; // non-bootstrap
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

    /// The epoch slots — every per-epoch fact, the sit-out memory included — are
    /// bounded by the retention window.
    ///
    /// `SatOut` is the load-bearing phase: `recover` never re-decides a decided
    /// epoch, so a slot for a target the actor can still reach MUST survive the
    /// sweep (dropping it would re-open a settled sit-out, and re-dealing
    /// self-equivocates), and only slots below the floor may go.
    ///
    /// Self-verifying: it asserts the map actually grew before asserting it was
    /// pruned, so it cannot pass vacuously on an actor that decides nothing.
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

            // Walk the chain across several epochs. Each tick decides `now + 1` (a
            // stable committee ⇒ `KeyOnly`) and the trailing window. The walk has to
            // reach past the retention window, or nothing is below the floor and the
            // pruning assertions hold vacuously — so it is DERIVED from the window
            // rather than written down.
            let last: u64 = crate::beacon::JOURNAL_RETENTION_EPOCHS + 3;
            for e in 1..=last {
                actor.on_height(INTERVAL * e, &mut arng).await;
            }

            // Self-check: the PRODUCTION decide path is what filled the map — the
            // target `now + 1` was decided by `recover` (a stable committee ⇒
            // `KeyOnly`), on every tick of the walk. Without this the pruning
            // assertions below would hold trivially on an actor that decides nothing.
            let now = actor.epoch_of(INTERVAL * last);
            // The only target `recover` can still be asked to deal for.
            let reachable = now + 1;
            assert_eq!(
                actor.phase(reachable),
                Some("key_only"),
                "the walk did not decide its target — this test proves nothing about pruning"
            );
            assert!(actor.epochs.len() > JOURNAL_RETENTION_EPOCHS as usize);

            // Seed the load-bearing phase by hand, on both sides of the floor: a
            // standalone actor without a share dir has no way to produce a `SatOut`.
            // One epoch strictly below the retention floor, derived from the window so the
            // case stays a real aged-out slot when the window changes.
            let aged_out = now - JOURNAL_RETENTION_EPOCHS - 1;
            actor.epochs.remove(&reachable);
            actor.enter(reachable, EpochState::SatOut { key: None });
            actor.enter(aged_out, EpochState::SatOut { key: None });
            assert_eq!(actor.metrics.stalled_gauge(StallReason::SatOut), 2);

            // One more tick: the sweep runs at the same `now` that feeds `recover`.
            actor.on_height(INTERVAL * last, &mut arng).await;
            assert_eq!(
                actor.metrics.stalled_gauge(StallReason::SatOut),
                1,
                "the aged-out slot's latch steps the gauge down with it"
            );

            // SAFETY: a slot the actor can still reach as a target must survive. Dropping
            // it would re-open a settled sit-out, and re-dealing self-equivocates.
            assert_eq!(
                actor.phase(reachable),
                Some("sat_out"),
                "the sweep dropped the sit-out for a target recover can still reach — \
                 re-dealing that epoch would self-equivocate"
            );

            // BOUND: nothing below the floor survives.
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

    /// Post-restart serve from a COLD cache (R1) + post-restart journal eviction (R2) +
    /// the cold-cache fetch-burst bound (e). All three are standalone (no network): a
    /// fresh actor whose serve store is empty but whose epoch-2 journal is present on
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

            // (e) burst bound: a second serve for the SAME epoch (different dealer) is a
            // cache hit — NO additional parse.
            let peer_key2 = log_of(2);
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
        // ONE agreed value per epoch across the committee: the 6 holders certify the
        // full set, and the RESTARTED victim receives that set — which is what names
        // the bodies (incl. its own, in the torn case) it must fetch by hash. The
        // pre-restart victim is kept off it (a per-node stub that never reaches the
        // quorum): it crashes before its instance delivers anything, else it would
        // fetch and finalize through its links to {1,2,3} before the restart and
        // the restart would find a share on disk.
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

    /// Node-0's actor over a live epoch-2 ceremony that has ingested EVERY committee
    /// log through the resolver path, with each `victims` log ingested while the share
    /// dir was a FILE — so exactly those journal appends failed and the actor holds
    /// those bytes in memory with no way to back them across a restart. The share dir
    /// is left broken; a caller that wants the retry leg repairs `actor.share_dir`.
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
        /// by) whose journal record IS durable.
        fn durable_seats(&self) -> Vec<u8> {
            self.committee
                .iter()
                .enumerate()
                .filter(|(_, pk)| !self.victims.contains(*pk))
                .map(|(i, _)| i as u8)
                .collect()
        }

        /// This node's own seat in the committee.
        fn my_seat(&self) -> u8 {
            self.committee
                .iter()
                .position(|pk| *pk == self.me)
                .expect("a member") as u8
        }

        /// `(seat, keccak256(log))` for `seats`, taken from the CEREMONY — which holds
        /// every log whether or not it is claimable, so these pairs are the real bodies
        /// a proposal would pin, independent of the index under test.
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

        /// Dealers whose `PeerLog` the on-disk journal actually holds, read the way
        /// every other consumer of the journal reads it — by `check`ing the record
        /// against the epoch's `Info`, which is what establishes the dealer's identity.
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

        // The victims go LAST and against the broken dir: every durable ingest runs the
        // retry leg, so a victim ingested first would be healed before the assert.
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

    /// FLU-1169. A dealer log this node holds in memory but could not journal is not
    /// CLAIMED: it is absent from the shared `recorded_dkg_logs` index the agreement
    /// plane proposes from, and absent from the `ShareConfirm` minted off that index —
    /// while every dealer whose record DID land stays claimed. Both statements would
    /// be false for this node after a restart, and the plane can start an instance on
    /// the strength of them.
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

            // Independent of the index: `covers` is a superset test over the SIGNED
            // body, so asking the pool which confirmations cover the four real log
            // bodies answers "did this node sign for the log it cannot back" without
            // re-reading the map the assertions above already checked.
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

    /// The degenerate end of the same gate: with the share dir unwritable for EVERY
    /// ingest, this node can back nothing, so it claims nothing — the published index
    /// for the target stays empty rather than carrying a set the node would lose on
    /// restart. It keeps every log in ceremony memory, which is what lets it still
    /// finalize over a set the committee agreed without it; that half is proven end to
    /// end by `acked_dealing_withheld_on_append_failure_still_recoverable`, which needs
    /// a real player view and an agreement plane this fixture does not build.
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

    /// The exclusion is not permanent: `publish_recorded_logs` re-attempts every failed
    /// write before it decides what may be claimed, so the first publish edge after the
    /// dir is writable again both lands the record and widens the claim — from memory,
    /// with no re-fetch (the bytes never left).
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

    /// D3, the mint edge. A log that arrives by RECOVERY widens what this node can
    /// confirm with NO height tick behind it — and the entry bar counts confirmations
    /// that COVER the proposed set, so until the wider one is on the wire the widening
    /// is invisible to every leader. On a live chain the next tick carries it; on a
    /// halted chain there is no next tick, which is the case this edge exists for.
    /// `Decisive` is exactly the trigger that would skip this width, so the delivery
    /// path mints on `AnyGrowth`.
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

            // One mint at the quorum: that width, and only that width, is on the wire.
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

            // The remaining log arrives by RECOVERY, over the resolver seam the run loop
            // drives — not by gossip, and with no tick behind it.
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

            // Before the seal deadline the live-dealer ceremony must NOT finalize: a
            // `Dealing` epoch is never a finalize input, and it holds too few logs for
            // a set to be certified over anyway.
            let mut arng = StdRng::seed_from_u64(9);
            actor.drive_finalization(&mut arng);
            assert!(
                store.read().map(|s| s.is_empty()).unwrap_or(false),
                "a reconstructed live-dealer ceremony does not finalize before its seal"
            );
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("dealing"));

            // `on_height` step 1 seals it at the deadline; shorthanded ⇒ it does NOT
            // finalize on this tick, so we can observe that the seal recorded its OWN log.
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

            // Now that its own log is in, the agreed set covers the 3-log quorum and
            // the next tick finalizes over it.
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

    /// §5.4, rule 1: A SHARE WHOSE DISK WRITE FAILS IS NOT ADOPTED — the node is
    /// verify-only for the epoch, not a signer with material it cannot reload.
    ///
    /// The fixture is `pre_deadline_resume_seals_via_on_height_then_finalizes` with
    /// one thing changed: the share dir is a FILE, so `share_state::persist`'s
    /// `create_dir_all` fails and nothing else does. That is the whole difference,
    /// which is what makes the two tests a pair — the positive one proves this
    /// ceremony DOES finalize and store, so a green here cannot be a ceremony that
    /// simply never got that far.
    ///
    /// What is observable, and it has to be, because a refusal is otherwise
    /// indistinguishable from a stall: `dpos_dkg_share_persist_failed_total` moves,
    /// `dkg_ceremony_ok_total` does not, the store stays empty, the epoch is
    /// `Acquiring(Logs)` (the journal re-selected over the pinned set on the next
    /// tick) and `Stalled{PersistFailed}` is raised on it (§5.4).
    ///
    /// Falsifier: a share in the store (the refusal is gone and R-021 is back); a
    /// flat `dkg_share_persist_failed` (the write did not actually fail, so the test
    /// is about nothing); `dkg_ceremony_ok` moving (the adopt ran past the refusal).
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

            // A share dir that is a FILE: every `persist` fails on `create_dir_all`,
            // and nothing in the ceremony itself is broken by it.
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

    /// [1] negative — a STILL-DEALING ceremony (dealer alive) does NOT finalize early.
    /// `dealing_closed()` is false before `seal_dealings`, so even a ceremony holding a
    /// ready quorum over the agreed set must wait for the seal (the seal-before-finalize
    /// contract the gate states via the durable dealer-taken state).
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
            // A pre-seal resume with its dealer RECONSTRUCTED: the dealing is open,
            // and every peer log is already recorded so the agreed set below is a
            // selectable quorum. The seal gate is then the ONLY thing left to stop it.
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

    /// A `finalize→Err(MissingPlayerDealing)` is TERMINAL and named (§5.2: no fetch
    /// can produce a dealing this node acked and no longer holds), and it does not
    /// forfeit what the ceremony still owes its peers: the recorded logs stay
    /// servable. We construct the exact post-resume `MissingPlayerDealing` state:
    /// node-0 resumes holding ONLY its own self-dealing (its `view` lacks every
    /// peer's dealing), then the resolver delivers the 3 peer logs (each ACKING
    /// node-0). `select` then picks a quorum of peers whose private dealings node-0's
    /// `view` lacks → `Player::finalize` returns `MissingPlayerDealing`. The epoch
    /// MUST land in `Unrecoverable` with `Stalled{Unrecoverable}` raised, no share,
    /// and `serve_log` still answering for the recorded bodies. It used to keep a
    /// player-less ceremony in the map with no exit (E5-12).
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
                &BTreeMap::new(),
            )
            .expect("self-only resume");

            let store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut actor = standalone_actor(&oracle, key0, committee, None).await;
            actor.store = store.clone();
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);

            // Deliver the 3 peer logs via the resolver-ingest path (each acks node-0).
            // Now `recorded` has all 4 dealers, `ready()` (observe) returns Ok — but
            // node-0's `view` lacks the peers' private dealings.
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
            // The agreed set now covers a quorum, so the finalize runs and trips
            // MissingPlayerDealing.
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

    /// P1 1a (RED→GREEN): a finalized epoch's journal + serve-store entry SURVIVE the epoch
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

            // Cross the boundary but stay IN the retention window (now == E == 2).
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
    /// ceremony fetch stops at `epoch_start`; `Acquiring(Logs)` does not). Pre-fix the
    /// only fetch driver was the live ceremony, gated off past the boundary.
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
            // The body the artifact pinned for that dealer — what the heal asks for.
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
            // A demote-heal already in flight for epoch 2, still wanting `missing`'s
            // pinned body.
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

            // Drive the fetch (the heal is not gated on the clock).
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

    /// P2 2a (RED→GREEN) — the CORE heal. node-0 is a demoted committee[2] member (no
    /// share; a peer dealer's log missing from its journal). The actor SELF-DETECTS the
    /// demote from its OWN `on_height` loop (via a mock `outcome_at` returning the pinned
    /// Output — no epoch_manager signal), fetches the missing pinned-`dealers()` log
    /// (delivered here via the resolver ingest), recomputes its share scoped to
    /// `dealers()`, self-verifies it against the pinned Output, and adopts it —
    /// reproducing its CANONICAL share and firing the promote path (dkg_ceremony_ok).
    #[test]
    fn demoted_member_recomputes_share_and_heals() {
        // recover/try_recompute cold-load the journal → touches the process-global
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
            // Mock artifact reader: the agreed artifact's payload for epoch 2 (a change
            // epoch), None elsewhere — what `recover(2)` reads on the first tick.
            actor.outcome_at = artifact_reader(
                2,
                pinned_canon.iter().map(|(i, h)| (*i, *h)).collect(),
                &outcome_bytes,
            );
            let mut arng = StdRng::seed_from_u64(9);

            // now == 2 (height = epoch_start(2)). `recover(2)`: journal present, artifact
            // held, no share ⇒ the ceremony is resumed player-only over the PINNED
            // bodies and the epoch is `Agreed`, waiting on the one pinned body it lacks.
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

            // Deliver the held-back pinned-dealer log via the resolver ingest.
            let key = DkgLogKey {
                epoch: 2,
                dealer: held_dealer.clone(),
                hash: log_hash(&held_log),
            };
            let accepted = actor.ingest_log(&key, held_log.encode(), &mut arng).await;
            assert!(accepted, "the delivered pinned-dealer log is accepted");

            // HEALED: store[2] holds node-0's CANONICAL share; the epoch is Keyed;
            // the promote-driving metric incremented.
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

    /// Phase 1 (FLU-1166): the member's live-epoch artifact pull, issued at the one
    /// point where "member of `committee[E]`, has no artifact" is fully known — and
    /// where, before this, the information was simply discarded. Nothing else asks
    /// for the LIVE epoch: the epoch manager's repair sweep excludes
    /// `epoch >= frontier` by design.
    ///
    /// What is asserted here is the ACTOR's half of the de-duplication: it asks once
    /// per tick for an epoch whose ARTIFACT it lacks, share or no share — a share is
    /// not a key, and the third half below is the case that used to end the ask and
    /// must not (§5.4's partial success; the ask ends on the artifact arriving,
    /// pinned by
    /// `a_restarted_member_with_a_share_and_no_artifact_asks_for_it_and_keys_on_arrival`).
    /// The rate bound proper is not the actor's and is not visible to it — one
    /// network attempt per epoch per `PULL_MIN_INTERVAL`, covered by
    /// `artifact::tests::repeated_pulls_are_rate_bounded_per_epoch`, plus the plane's
    /// in-flight guard against stacking spawns between windows.
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
            // The artifact has not reached this node. At this seam that is
            // indistinguishable from a carry-forward epoch, which is exactly why the
            // ask has to be cheap rather than conditional.
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

            // The share lands, by whichever route — and the ask CONTINUES, because a
            // share is not a key: `PK_E` comes from the artifact and nothing else
            // (П-3), so a member holding the share and no artifact is §5.4's partial
            // success and still needs the fetch. What ends the ask is the ARTIFACT
            // arriving, pinned by
            // `a_restarted_member_with_a_share_and_no_artifact_asks_for_it_and_keys_on_arrival`.
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

    /// PARTIAL SUCCESS — the restart state project §5.4 names and nothing covered:
    /// the share file landed, the artifact's durable write did not, and this node is
    /// a MEMBER of the LIVE epoch. §5.4's outcome is `Acquiring{artifact}` with the
    /// share kept and `Keyed` on the artifact arriving from peers ("one fetch at
    /// `≤ f`"); this is that outcome as an assertion instead of prose.
    ///
    /// WHY THE ASK HAS TO COME FROM HERE. The state is "holds `E`'s share, holds no
    /// artifact for `E`", and every other leg that spends the network rung excludes
    /// it by construction: the non-member acquisition (`Acquiring(ArtifactForKey)`) excludes members
    /// (measured, see its doc — without that gate an honest committee spends its
    /// pull budget on the epoch it is itself dealing), the epoch manager's repair
    /// sweep excludes `epoch >= frontier` because repairing PAST epochs is what it
    /// is for, and the cert-inlet's `ensure_key` spends `PinEffort::Local`, which is
    /// contractually network-free. This loop is the only one that reads both halves
    /// of the state in one place.
    ///
    /// Falsifier: no ask while the artifact is missing (the hole itself — the state
    /// then parks execution to the end of the epoch); an ask that repeats after the
    /// artifact is local (a pull per tick for the epoch's whole life); the key
    /// resolving BEFORE the artifact arrives (then the fixture is not in the
    /// partial-success state at all and everything after it is vacuous).
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

            // THE TWO HALVES OF THE STATE, through the production seams: the share
            // written by `share_state::persist` and read back by the loader
            // `beacon::build` runs at launch, and an EMPTY artifact partition.
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
            // The production artifact reader, verbatim from `beacon::build`.
            actor.outcome_at = store_reader(&artifacts);
            let asked: Arc<std::sync::Mutex<Vec<u64>>> = Arc::default();
            actor.pull_artifact = {
                let asked = asked.clone();
                Arc::new(move |epoch: u64| asked.lock().expect("asked").push(epoch))
            };

            // THE STATE, stated before anything is driven — and the second half is
            // what makes the rest non-vacuous: the key is a projection of the
            // artifact alone (П-3), so a held share answers nothing.
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

            // A peer serves it — `ArtifactBridge`'s adopt edge, which is the only way
            // a verified artifact enters the store, and which hands it to the actor.
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

    /// THE EXPIRED CASE. Once an epoch leaves the retention window its journal is
    /// reclaimed, so there is nothing left to recompute from and the heal is over —
    /// and the code says so rather than retrying for the life of the process.
    ///
    /// Driven with the artifact READABLE and a heal already in flight, so the only
    /// thing standing between the actor and the work is the window itself. The
    /// in-window epochs are asserted to be asked for in the same breath, which is
    /// what keeps the absence for the expired one from passing vacuously.
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
            // `DkgOutcome` is not `Clone` (it carries the pinned polynomial), so the
            // two copies this test needs are re-parsed from the wire encoding.
            let outcome_bytes = crate::beacon::outcome::encode_outcome(&outcome);
            let missing = outcome.dealers().iter().next().cloned().expect("a dealer");
            // The body the artifact pinned for that dealer — what the heal asks for.
            let missing_hash = B256::repeat_byte(0x5A);

            // Epoch 2's journal on disk — the heal inputs, before the sweep reclaims
            // them.
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
            // Every epoch mints, so every in-window epoch this member holds no share
            // for is decided (a stable committee would be carry-forward, which asks
            // for nothing) — what keeps the absence below from passing vacuously.
            actor.changed = Arc::new(|_epoch: u64| Some(true));
            let asked: Arc<std::sync::Mutex<Vec<u64>>> = Arc::default();
            actor.pull_artifact = {
                let asked = asked.clone();
                Arc::new(move |epoch: u64| asked.lock().expect("asked").push(epoch))
            };
            // A heal already in flight for epoch 2, still short one pinned body.
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

            // One epoch past the retention edge of epoch 2, derived rather than written
            // down: the window is the crate's one retention constant now.
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
            // Every epoch in the window is asked for except the expired one (each is a
            // mint epoch this member never dealt for, past its seal: `SatOut`, which
            // still needs `PK_E` to verify), and the window is the crate's constant —
            // so the expectation is derived from it.
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

    /// Phase 2: `MissingPlayerDealing` is TERMINAL, not pending.
    ///
    /// This node acknowledged a dealer's private point and no longer holds it — a
    /// destroyed or replaced share directory, since the ack is withheld until the
    /// journal write is durable. The dealer does not reveal a point whose ack it
    /// holds and a sealed log cannot be re-opened, so every retry is provably
    /// futile. Before this the error collapsed to a boolean: the entry stayed
    /// pending, the logs kept being fetched, and the state was indistinguishable
    /// from an artifact still in flight — which, now that the live-epoch pull
    /// exists, is what most share-less epochs actually are.
    ///
    /// Not a panic: a share-less member is safe as a verifier, so it sits the epoch
    /// out instead of taking the node down with it.
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

            // The agreed outcome, derived from an identical UNDAMAGED journal.
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

            // The damage: every dealer log is on disk (so `want` is empty and the
            // recompute is attempted), but one peer's private dealing — which that
            // peer's log records this node ACKING — is gone.
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
            // now = 2: `recover(2)` replays the journal against the held artifact and
            // trips the missing acked dealing.
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

            // The second tick is the whole point: nothing is re-driven, re-fetched, or
            // re-warned.
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

    /// P3 3d (RED→GREEN, step 1f): with EVERY `append_journal` failing, node-0's ack to
    /// each peer dealer is WITHHELD, so every peer reveals node-0's point → node-0 lands
    /// in the pinned `Output.revealed()` set AND still finalizes its share (the "share is
    /// reconstructable" branch — recoverable via those reveals). A QUAL log can never
    /// record an ack node-0 cannot back with a durable view. Control: a WORKING dir →
    /// node-0 acks everyone → node-0 ∉ revealed.
    ///
    /// THE FIXTURE BREAKS THE JOURNAL WRITE AND ONLY THE JOURNAL WRITE, and that
    /// precision is now load-bearing. It used to make the whole `share_dir` a FILE,
    /// which failed `append_journal` and `share_state::persist` together — harmless
    /// while a failed persist only warned. Since П-3 a failed persist REFUSES the
    /// share (`adopt_share`), so the old fixture would assert the ack rule against a
    /// node that adopted nothing, and `seeded` would be false for the wrong reason.
    /// The journal PATH is made a directory instead: `append_mode_0600` cannot open
    /// it, `write_mode_0600` writes the share file beside it, and `load_journal`'s
    /// read of a directory is the same `NoFile` the old fixture produced — so the
    /// ceremony still deals fresh and only the ack leg is affected.
    #[test]
    fn acked_dealing_withheld_on_append_failure_still_recoverable() {
        let runtime = deterministic::Runner::default();
        let (seeded_fail, revealed_fail) = runtime.start(|ctx| async move {
            let bad = fresh_share_dir("append-fail");
            std::fs::create_dir_all(&bad).expect("share dir");
            // Only the JOURNAL path is unwritable — see the doc above.
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
        // ONE index for the whole committee: node-0 claims only what it journaled
        // durably, and the set it finalizes over is the one the committee agreed —
        // not the one node-0 was able to claim. Per-node, the stub would wait forever
        // on node-0's own empty set, because it has no leader rotation to move past a
        // member with nothing to propose (see `spawn_stub_agreement`).
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
                let _ = s.send(h).await;
            }
            ctx.sleep(Duration::from_millis(50)).await;
        }
        // The fixture's confirmation traffic is REAL (`Wiring::standalone` wires a
        // pool and the shared index, so `Confirmations::mint` signs and sends at
        // the quorum): pinned as a fact, so the fixtures' behaviour is not a silent
        // property of the wiring — a PEER's `ShareConfirm` for the bootstrap epoch
        // sits in node-0's pool, counted through `on_confirm`.
        let me0_seat = committee.position(&me0).expect("node 0 seats") as u8;
        assert!(
            victim_confirms
                .covering(DETERMINISTIC_BOOTSTRAP_EPOCH, &[])
                .iter()
                .any(|c| c.idx != me0_seat),
            "a peer's ShareConfirm reached node-0's pool over the sim network"
        );
        // The SHARE says it finalized; the OUTPUT says whether the peers revealed this
        // node's point. The two used to be one store entry; after П-3 the share store
        // holds the secret and the ceremony's `Output` is read through the actor's
        // test-only record of what it adopted.
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

    /// A mock [`AgreedOutcomeAt`]: the agreed artifact's payload for `epoch` — the
    /// pinned `logs` and the `Output` encoded in `outcome_bytes` — and `None` for
    /// every other epoch. What `recover(E)` reads on a restart.
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

    /// The production [`AgreedOutcomeAt`] over a store, verbatim from
    /// `beacon::build`: the held payload and the divergent second value.
    fn store_reader(store: &crate::beacon::artifact::ArtifactStore) -> AgreedOutcomeAt {
        let store = store.clone();
        Arc::new(move |epoch: u64| {
            store.view(epoch).map(|(held, divergent)| StoredArtifact {
                held: held.0.clone(),
                divergent,
            })
        })
    }

    /// A quorum-certified artifact over `logs`, built the way the agreement plane
    /// builds one. The write-back reads only the target epoch and the pinned set —
    /// the certificate is verified by the plane's own instance and by the pull
    /// seam before either hands one over — so the committee here is a fresh set
    /// whose only job is to make a real `Finalization` constructible.
    fn agreed_artifact(target_epoch: u64, logs: Vec<(u8, B256)>) -> AgreedArtifact {
        agreed_artifact_with_committee(target_epoch, logs).0
    }

    /// [`agreed_artifact`] carrying `group_key` — the polynomial the actor's share
    /// must lie on (F-02), derived by the certifier over exactly `logs`.
    fn agreed_artifact_keyed(
        target_epoch: u64,
        logs: Vec<(u8, B256)>,
        group_key: DkgOutcome,
    ) -> AgreedArtifact {
        certify(target_epoch, logs, Some(group_key)).0
    }

    /// [`agreed_artifact`] plus the committee its certificate was signed by — what
    /// a pull-seam bridge needs to VERIFY the same artifact.
    fn agreed_artifact_with_committee(
        target_epoch: u64,
        logs: Vec<(u8, B256)>,
    ) -> (AgreedArtifact, fluentbase_bls::EpochCommittee) {
        certify(target_epoch, logs, None)
    }

    /// Certify `(logs, group_key)` for `target_epoch` under a fresh 4-member
    /// committee (a random `deal` stands in for the key where the test does not
    /// care which polynomial the artifact names).
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

    /// The group key a certifier derives over `pinned` from `ceremony` — the
    /// polynomial an artifact naming that set carries. Panics unless every pinned
    /// body is held there and a quorum is selectable within them.
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
        let c = actor.ceremony(epoch).expect("ceremony");
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
                &BTreeMap::new(),
            )
            .expect("post-deadline resume");

            let store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let share_notify = Arc::new(tokio::sync::Notify::new());
            let mut actor = standalone_actor(&oracle, key0, committee.clone(), None).await;
            actor.store = store.clone();
            actor.share_notify = share_notify.clone();
            // Only a certified set may mint, so an epoch whose artifact has not
            // landed waits instead of finalizing over whatever this node holds.
            actor.insert_ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH, resumed.ceremony);

            // The last block of epoch E: every body held, and still nothing to
            // finalize over — this is the wedge.
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
            // No further height tick from here on: the artifact edge is the only
            // thing that runs.
            actor.on_artifact(artifact, &mut rng).await;

            assert!(
                store
                    .read()
                    .map(|s| s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
                    .unwrap_or(false),
                "the artifact's dealer-log set is the pinned set; the existing \
                 finalize rails do the rest"
            );
            // Blocker 2 of the write-back: the share-gate wants a LOCAL share at
            // E+1, and after П-3 that is one lookup rather than a resolver closure —
            // this node's share at the mint the chain names, paired with the
            // polynomial that mint's artifact carries.
            assert!(
                store
                    .read()
                    .map(|s| s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
                    .unwrap_or(false),
                "the E+1 share-gate must pass with no block of E+1 in existence"
            );
            // Blocker 1: the respawn edge is `share_notify`, and its permit
            // survives having had no waiter armed when it fired.
            assert!(
                futures::FutureExt::now_or_never(Box::pin(share_notify.notified())).is_some(),
                "the write-back must fire the edge the epoch manager respawns on"
            );
            // The write-back is complete, so its agreed set is spent rather than
            // pinning a ceremony open until the window ages it out.
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("keyed"),
                "a completed write-back lands the epoch in `Keyed`"
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
    /// every `PK_epoch` read and the serve path, none of which reach `on_artifact`.
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
            // The chain is PAST the target's boundary — where the height-driven
            // fetch gives up — and the height clock has stopped there.
            actor.last_height = Some(BOUNDARY);

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

    /// A certified set mints the instant its bodies are held, on no clock at all.
    ///
    /// A finalize gated on the finalized-height clock would leave a node whose clock
    /// has stopped sitting on an agreed key it could have minted — the exact coupling
    /// the certificate removes, since a quorum certificate states outright the one
    /// thing a deadline was ever there to make true: that every honest node selects
    /// over an identical set.
    ///
    /// The pinned set here is a strict SUBSET of the committee (node-0 never sealed
    /// its own log), so this is not the complete-set case either.
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
            // Sealed, and the epoch boundary is still a whole margin away.
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
    /// is on this node's disk — and no peer will start a second instance (their
    /// launchers already hold `E+1` in `started`). THE STORE IS THE OWNER of the
    /// fact and the actor READS it: `recover(E)` on the restart's first tick resumes
    /// the journal AND takes the stored artifact, so the epoch stands `Agreed` with
    /// the ceremony holding every body but one; the missing body arriving then
    /// finalizes it. There is no replay of the store into the actor's channel any
    /// more (5.3-А2 deleted `restart_replay`): this read is the whole of it, and
    /// `reconcile_with_store` re-reads on every later tick. Falsifier: `recover`
    /// ignoring the stored artifact (the epoch resumes `Sealed`, the body completes
    /// nothing, the store stays empty).
    #[test]
    fn a_restart_reads_the_stored_artifact_back_into_the_actor() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(97);
            // The same fixture again (deterministic), for the key the instance
            // derived over the FULL pinned set before the crash.
            let (_c2, _k2, full_journal) = node0_pre_seal_journal_full_sealed(97);
            oracle.manager().track(0, committee.clone()).await;

            // The on-disk state a crash leaves behind: the ceremony journal minus the
            // one peer log the recovery fetch had not delivered, and no share file.
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

            // The pinned set the instance certified before the crash — every seat,
            // the withheld body at the missing dealer's — and the key over it, the
            // way the instance's delivery edge filed it into the durable store.
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

            // The restart proper, over the directory and the store.
            let ceremony_store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut wiring = Wiring::standalone();
            wiring.outcome_at = store_reader(&artifacts);
            let mut actor =
                standalone_actor_wired(&oracle, key0, committee.clone(), Some(dir.clone()), wiring)
                    .await;
            actor.store = ceremony_store.clone();
            // The first height tick resumes the ceremony off the journal, still
            // inside epoch E (so nothing has swept it), and READS the artifact.
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

            // The awaited body lands: every pinned body is held, the epoch keys.
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
            // And the per-tick re-read of the same value changes nothing.
            actor.on_height(BOUNDARY, &mut rng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("keyed"));
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    // ---- 5.3-А1: the state machine's own tests --------------------------------

    /// A standalone actor over `committee` with `share_dir`, its clock at `height`
    /// and its slots empty — the restart shape every `recover(E)` cell starts from.
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

    /// A present-but-unreadable journal: a framed record whose body decodes as
    /// nothing. `load_journal` answers `Torn` for it (a non-empty file whose FIRST
    /// record fails to decode).
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

    /// §5.2 restart table, cell (NoFile, h < seal): a genuine first run deals — the
    /// seeded dealer makes the start idempotent even after a datadir loss.
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

    /// §5.2 restart table, cell (NoFile, h ≥ seal): a lost journal at the deadline
    /// cannot prove this node never sealed, so it sits out — it used to start fresh
    /// and seal a SECOND, differently-acked log (R-036 / E5-15). `Stalled{SatOut}`
    /// is raised once, and the epoch still asks for the artifact it needs to
    /// verify with.
    #[test]
    fn recover_no_journal_at_the_seal_deadline_sits_out() {
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
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("sat_out"));
            assert!(out.is_empty(), "nothing is dealt or re-sealed");
            assert!(
                !journal_path(&dir, DETERMINISTIC_BOOTSTRAP_EPOCH).exists(),
                "no journal is written for an epoch sat out"
            );
            assert!(actor
                .stalls(DETERMINISTIC_BOOTSTRAP_EPOCH)
                .contains(&StallReason::SatOut));
            // The verdict is memory, not a per-tick retry: two more ticks re-decide
            // nothing and the latch stays one.
            let mut arng = StdRng::seed_from_u64(1);
            actor.on_height(BOUNDARY, &mut arng).await;
            actor.on_height(BOUNDARY + 1, &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("sat_out"));
            assert_eq!(
                actor.stalls(DETERMINISTIC_BOOTSTRAP_EPOCH).len(),
                2,
                "SatOut + NoArtifact"
            );
            assert!(
                !asked.lock().expect("asked").is_empty(),
                "a sat-out member still acquires the epoch's key to verify with"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// §5.2 restart table, cell (Torn, h < seal): nothing was ever broadcast (a node
    /// seals only at/after the deadline) and the dealer is deterministic, so a torn
    /// journal before the deadline re-deals over a FRESH journal — it used to sit the
    /// epoch out (R-072 / E5-36).
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

    /// §5.2 restart table, cell (Torn, h ≥ seal): this node participated and may
    /// have sealed; it sits out, exactly as before.
    #[test]
    fn recover_torn_journal_at_the_seal_deadline_sits_out() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, _journal) = node0_pre_seal_journal_full_sealed(104);
            oracle.manager().track(0, committee.clone()).await;
            let dir = fresh_share_dir("recover-torn-post");
            write_torn_journal(&dir, DETERMINISTIC_BOOTSTRAP_EPOCH);
            let mut actor =
                restarted_actor(&oracle, key0, committee, dir.clone(), SEAL_DEADLINE).await;
            let mut out = Vec::new();
            assert!(!actor.decide(DETERMINISTIC_BOOTSTRAP_EPOCH, &mut out).await);
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("sat_out"));
            assert!(
                matches!(
                    actor.load_journal(DETERMINISTIC_BOOTSTRAP_EPOCH),
                    JournalLoad::Torn
                ),
                "the torn journal is left as it is: evidence, not a fresh start"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// §5.2 restart table, the journal-present cells: `Present, h < seal` resumes
    /// with a RECONSTRUCTED dealer (`Dealing`); `Present, h ≥ seal` resumes
    /// player-only (`Sealed`) for an epoch not yet entered; an epoch already
    /// entered resumes into `Acquiring(ArtifactForCeremony)` — its agreement ran
    /// without this node, so the artifact is at the peers.
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

    /// §5.2 restart table, the cells without a ceremony: a held share with the
    /// artifact is `Keyed`; a carry-forward epoch is `KeyOnly`; a non-member of a
    /// mint epoch is `KeyOnly` with the artifact and `Acquiring(ArtifactForKey)`
    /// without; an epoch whose committee cannot be read is not decided at all.
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

            // (share held, artifact held) ⇒ Keyed.
            actor
                .store
                .write()
                .expect("store")
                .insert(2, shares[&keys[0].public_key()].clone());
            actor.outcome_at = artifact_reader(2, logs.clone(), &outcome_bytes);
            actor.decide(2, &mut out).await;
            assert_eq!(actor.phase(2), Some("keyed"));

            // (¬mints) ⇒ KeyOnly, no artifact of its own.
            actor.decide(3, &mut out).await;
            assert_eq!(actor.phase(3), Some("key_only"));

            // (member ∉ C[4], no artifact) ⇒ Acquiring(ArtifactForKey) …
            actor.decide(4, &mut out).await;
            assert_eq!(actor.phase(4), Some("acquiring_artifact_for_key"));
            // … and KeyOnly on the artifact arriving.
            let mut arng = StdRng::seed_from_u64(7);
            actor
                .on_artifact(agreed_artifact(4, logs.clone()), &mut arng)
                .await;
            assert_eq!(actor.phase(4), Some("key_only"));

            // committee[5] unreadable ⇒ undecided (no slot), re-asked next tick.
            assert!(!actor.decide(5, &mut out).await);
            assert!(actor.phase(5).is_none());
            assert!(out.is_empty(), "none of these cells sends anything");
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// A sealed ceremony whose agreed set holds every pinned body and still has no
    /// selectable quorum is `Stalled{QuorumMissing}`: the ERROR line and the
    /// deferred counter fire ONCE for the epoch, the latch stays, and the epoch is
    /// still `Agreed` (a Reveal / a wider set could still complete it).
    #[test]
    fn a_below_quorum_agreed_set_stalls_the_epoch_once_with_quorum_missing() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(107);
            oracle.manager().track(0, committee.clone()).await;
            let me0 = key0.public_key();
            // Keep ONLY node-0's own sealed log: every other dealer's body is absent.
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
            // The set names exactly the one body this node holds — all held, and
            // one dealer is below the 3-dealer quorum.
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
            // Re-driven twice more: latched, so neither the counter nor the line repeats.
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

    /// F-02 / R-039: the adoption gate is the ARTIFACT's polynomial on the live
    /// path. The artifact pins the full set but carries a key derived over a
    /// different set (the three highest-seated dealers); the ceremony finalizes over the
    /// pinned four and its share lies on the four-dealer polynomial — off the
    /// certified one — so it is REFUSED (`dkg_share_off_polynomial`), nothing is
    /// stored, and the epoch heals over the retained journal (`Acquiring(Logs)`).
    /// A gate against the LOCAL output would adopt it with `ok = 1`.
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

    /// `Conflict`: two DIFFERENT quorum-certified artifacts for one epoch stop the
    /// epoch's signing — the share leaves the store, the phase is terminal, and
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

            // The same value again: first-wins, nothing changes.
            actor.on_artifact(first.clone(), &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("keyed"));
            assert_eq!(actor.metrics.dkg_artifact_conflict.get(), 0);

            // A different certified set (three of the four dealers).
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
            // Terminal: neither value re-enters anything.
            actor.on_artifact(first, &mut arng).await;
            actor.on_artifact(second, &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("conflict"));
            assert_eq!(actor.metrics.dkg_artifact_conflict.get(), 1);
        });
    }

    /// … and NOT from a forged one: `Conflict` is reachable only through the
    /// artifact seam's quorum check. A second artifact whose certificate does not
    /// verify is rejected at the bridge (`deliver → false`), never handed to the
    /// write-back, and the actor stays `Keyed`.
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

            // This node's seam: the store holds `held`; the adopt channel is the
            // actor's inbound.
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

            // A FORGED second artifact: a different set whose certificate does not
            // cover it (the payload was altered after signing). `certify` signs under
            // one fixed committee, so the certificate itself verifies — it is the
            // payload mismatch that makes this forged, exactly the check the seam
            // owns.
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

            // The contrast that keeps this from being vacuous: the SAME divergent set
            // under a certificate that DOES verify is handed over, and that is what
            // makes the epoch `Conflict`.
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

    /// R-026 / E5-11: the instance's body-lost verdict moves a `Sealed` epoch to
    /// acquiring the artifact from peers AT ONCE — asked on the signal and on
    /// every tick after — with `Stalled{BodyLost}` raised; the artifact arriving
    /// takes it through `Agreed` to `Keyed`. Nothing waits for the boundary.
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
            // A second signal for an epoch no longer waiting on its instance changes
            // nothing.
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

    // ---- 5.3-А1, second round --------------------------------------------------

    /// Two INDEPENDENT ceremonies over one 4-member committee: `(outcome_a,
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

    /// DB-01 (F-02, the third way into `Keyed`): a member restarts holding a
    /// share written over artifact X, and the artifact that then arrives from a
    /// peer is X′ — a different polynomial. The share is NOT keyed over it: it is
    /// refused exactly as the live path refuses one (`dkg_share_off_polynomial`,
    /// `Stalled{OffPolynomial}`), it leaves the store and its file, and the epoch
    /// heals over its journal (`Acquiring(Logs)`). Without the gate a share over
    /// any value became "the key" the moment any certified artifact showed up.
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

            // X′: certified over the SAME committee, a different polynomial.
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

    /// DB-01, the restart cell: `recover(E)` with a share over X in the store and
    /// artifact X′ on disk refuses the share the same way — the §5.2 row
    /// `(share, artifact) ⇒ Keyed` holds only for a share ON the artifact's
    /// polynomial. The positive half (a share on the polynomial keys) is
    /// `recover_decides_the_share_key_and_membership_cells`.
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

    /// DB-25: `Conflict` is durable. The verdict evicts the share FILE and writes
    /// a marker beside it, so a restart (`load_all` + `recover`) comes back
    /// `Conflict` with `Stalled{Conflict}` — never `Keyed` off a reloaded share.
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
            // EB-07: the share file OUTLIVES the verdict (an `evict_share` that
            // failed) — the marker-first order of `recover` is what the restart
            // below stands on, so the share is put back on disk before it.
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

            // The restart: fresh maps, the same directory, the production reload.
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

    /// A bridge over `store` whose hand-off channel to the actor is FULL: what it
    /// delivers lands in the store and nowhere else — the lost hand-off of DA-01.
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

    /// DA-01 (a): the store is the owner of the fact. A pulled artifact whose
    /// hand-off to the actor is lost (the write-back mailbox is full) still keys
    /// the epoch on the next height tick, read from the store — through the same
    /// `Agreed → Keyed` edge, without the network.
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

    /// DA-01 (b): the store holds A and the actor stands on nothing yet; a
    /// certified B ≠ A pushed to the actor is `Conflict` against the STORE's value
    /// — the actor never finalizes over B while the node serves A as `PK_E`.
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

    /// DA-01, the divergent value noted by the STORE: the bridge's hand-off of a
    /// second certified value is lost, and the epoch is still `Conflict` on the
    /// next tick, off the store's note.
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

    /// EA-01 / EB-04: the STORE is the durable owner of the conflict witness. A
    /// second certified value it notes (a lost hand-off, no actor tick yet) is
    /// on disk the instant it is noted; a restart BEFORE the actor's tick — fresh
    /// actor, fresh store RAM, the share file and the held artifact reloaded —
    /// decides the epoch `Conflict`, never `Keyed` off the reloaded share.
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

            // The second value reaches the STORE only (the hand-off is lost) and
            // the actor never ticks again before the death.
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

            // The restart: a fresh store that reloads the held artifact (the
            // durable journal's half) and its markers, a fresh actor that reloads
            // the share file — the production launch, before any tick.
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

    /// EB-01: the verdict's two durable steps are ordered marker-then-share, so a
    /// death BETWEEN them (simulated: `stop_signing` returns after the marker)
    /// leaves the marker beside a share file that outlived it — and the restart
    /// reads the marker first, evicts the share and stands in `Conflict`. The
    /// reverse order left neither, and a restart re-derived the share.
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

            // The death, between the two steps of `stop_signing`.
            actor.die_between_verdict_and_eviction = true;
            let mut three = first.0.logs.clone();
            three.pop();
            let second = agreed_artifact(DETERMINISTIC_BOOTSTRAP_EPOCH, three);
            actor.on_artifact(second.clone(), &mut arng).await;
            let (vd_first, vd_second) = (
                crate::beacon::artifact::value_digest(&first.0),
                crate::beacon::artifact::value_digest(&second.0),
            );
            // The crash state, self-verified: the marker landed FIRST, the share
            // file (and the RAM share) are still there.
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

            // The restart over the directory: the share reloads, the marker wins.
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

    /// EB-03 / EA-04: a journal heal holding every pinned body whose journal
    /// cannot be LOADED (torn) parks VISIBLY — `Stalled{HealFailed}`, one line,
    /// one gauge step — and is not spent: the load is not an attempt, so the next
    /// tick re-reads the file, and once the journal is back the recompute runs
    /// and keys the epoch. `attempted` set before the load would have spent the
    /// one attempt on a file that was never read.
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
            // The heal, over a journal that holds every pinned body: `want` is
            // empty, the recompute is due on the tick.
            let set = AgreedSet::of(&actor.stored(2).expect("the artifact").held);
            let heal = actor.heal_over(2, &committee, set);
            assert!(heal.want.is_empty(), "every pinned body is journaled");
            assert!(!heal.attempted);
            actor.epochs.insert(
                2,
                EpochSlot::new(EpochState::Acquiring(Acquire::Logs(Box::new(heal)))),
            );
            let mut arng = StdRng::seed_from_u64(9);

            // The journal is TORN under the heal: non-empty, first record unreadable.
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

            // The journal is back: the next tick loads it and the heal keys.
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

    /// EA-10: a conflict marker that does not read (not 64 bytes) is still the
    /// verdict — only a verdict ever writes one — so the epoch is `Conflict`
    /// (fail-closed) with no known pair, and the file is left for the operator.
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

    /// EA-09: `enter` on a slot that already stands is a transition, so the
    /// latches the slot holds are kept — or dropped WITH their gauge step — by
    /// the same rule every transition uses; never dropped un-counted.
    #[test]
    fn entering_a_standing_slot_keeps_its_latches_counted() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, _journal) = node0_pre_seal_journal_full_sealed(121);
            oracle.manager().track(0, committee.clone()).await;
            let mut actor = standalone_actor(&oracle, key0, committee, None).await;
            actor.enter(3, EpochState::SatOut { key: None });
            assert_eq!(actor.metrics.stalled_gauge(StallReason::SatOut), 1);
            actor.enter(
                3,
                EpochState::SatOut {
                    key: Some(B256::repeat_byte(7)),
                },
            );
            assert_eq!(
                actor.metrics.stalled_gauge(StallReason::SatOut),
                1,
                "the standing latch is kept, not re-raised on a fresh slot"
            );
            assert_eq!(actor.stalls(3).len(), 1);
            actor.enter(3, EpochState::KeyOnly { digest: None });
            assert_eq!(
                actor.metrics.stalled_gauge(StallReason::SatOut),
                0,
                "a phase that does not carry the latch drops it with its gauge step"
            );
            assert!(actor.stalls(3).is_empty());
        });
    }

    /// DA-02: `Conflict` is judged by VALUE. A second certificate over the SAME
    /// pinned set and key, differing only in the confirmation metadata a leader
    /// attached (`confirms`, part of `DkgProposal::digest`), is the held value
    /// again — first-wins, not a conflict.
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

    /// DB-03 / DA-06: a live share refused as off the certified polynomial raises
    /// `Stalled{OffPolynomial}` ONCE and heals over the journal exactly ONCE — the
    /// recompute over every pinned body yields the same off-polynomial share, and
    /// that is `Unrecoverable`, not a per-tick ERROR and a re-run of the crypto
    /// until the sweep.
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

            // The heal: every pinned body is in the journal, the recompute runs
            // once, and its share is off the same polynomial — terminal.
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
            // Two more ticks: nothing re-runs, nothing re-counts.
            actor.on_height(SEAL_DEADLINE + 3, &mut arng).await;
            actor.on_height(BOUNDARY, &mut arng).await;
            assert_eq!(actor.metrics.dkg_share_off_polynomial.get(), 2);
            assert_eq!(actor.metrics.dkg_share_unrecoverable.get(), 1);
            assert!(actor.store.read().expect("store").is_empty());
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// DB-11 / DA-12, the OLD geometry (`interval = DKG_MARGIN_BLOCKS`, a
    /// zero-width deal window): epoch 2's seal deadline IS epoch 1's first
    /// height, so the first tick at which epoch 2 is decidable is already at the
    /// deadline. Before 5.3-А1 that started a ceremony (R-036, `NoFile ⇒
    /// start_fresh` with no deadline gate) — the only reason the old fixtures
    /// dealt at all. Now it sits out: no journal, no dealing, `Stalled{SatOut}`.
    #[test]
    fn at_a_zero_width_deal_window_a_missing_journal_at_the_deadline_sits_out() {
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
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("sat_out"));
            assert!(actor.ceremony(DETERMINISTIC_BOOTSTRAP_EPOCH).is_none());
            assert!(
                !journal_path(&dir, DETERMINISTIC_BOOTSTRAP_EPOCH).exists(),
                "nothing was dealt"
            );
            assert!(actor
                .stalls(DETERMINISTIC_BOOTSTRAP_EPOCH)
                .contains(&StallReason::SatOut));
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// D-10 (DB-09), directly: `resume(.., preferred)` stands the rebuilt player
    /// on the PINNED body of a two-log dealer, not the first-recorded one. The
    /// observable is `Player::resume`'s integrity check: the dealer's first log
    /// acks this node for a dealing the journal no longer holds
    /// (`MissingPlayerDealing`), its second log reveals this node instead — so
    /// the resume fails on the first-recorded body and succeeds on the pinned.
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

        // The journal as a restart finds it: the dealer's dealing to node 0 is
        // gone, and the dealer's two bodies are its evidence pair, first-recorded
        // first (`pair`), or its first body alone. The fixture is deterministic,
        // so each shape is cut from a fresh copy (`JournalRecord` is not `Clone`).
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

    /// DA-07 / DB-31: an artifact for an epoch outside `decide`'s window
    /// (`[max(2, now − R), now + 1]`) decides nothing and starts no ceremony —
    /// the store keeps it for the tick the epoch enters the window; one for
    /// `now + 1` is decided.
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
    // ---- 5.3-А2: the wiring and the tails ---------------------------------------

    /// F-01: the verdict's marker is fsync'd BEFORE the artifact's write-behind
    /// lands, so a death in between restarts with the marker and no artifact in
    /// the store. `recover` reads the marker first (`Conflict`) and, the store
    /// holding nothing, the terminal ACQUIRES: it pulls on the tick like every
    /// keyless terminal (`needs_artifact`), and the store's artifact — the pull
    /// seam files it, the tick reads it back — becomes its `key`; the signing
    /// stays stopped (no share, no phase change). Falsifier (M2): `Conflict{key:
    /// None}` out of `needs_artifact()` — nothing is asked and the node never
    /// holds `PK_E` for the epoch.
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

            // The crash state: the marker on disk, the store empty.
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

            // The pull seam files a value into the store (the hand-off to the
            // actor's channel may be lost); the next tick reads it back.
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

    /// E-10: the local ban of a proven equivocator (5.3-Б) covers its DEALINGS,
    /// not only its logs. With the evidence pair held for dealer 1, a
    /// `Commitment` from dealer 1 is refused at the consumer and counted
    /// (`equivocator`); the same frame from an honest dealer is not. Without the
    /// ban a player that has not acked yet takes the dealing of the dealer's
    /// second polynomial and its share lies off the pinned one. Falsifier (M3):
    /// the ban removed — the counter stays at 0.
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
                // Self-verification of the fixture: the evidence is held for d1.
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

    /// E-10 at the start-race DRAIN: the ban holds where a BUFFERED dealing is
    /// consumed, not only on the live dispatch. A restart over a journal that
    /// holds `DealerEquivocation` for dealer 1: dealer 1's `Commitment` (and an
    /// honest dealer's) arrives BEFORE the first tick — no slot yet, so both are
    /// buffered; the first tick resumes the ceremony `Dealing`, restores the
    /// evidence into the slot (`enter`) and drains the buffer — and the
    /// equivocator's dealing is refused THERE (`equivocator`, once), the honest
    /// dealer's is not. Falsifier (M1): the drain admits on the seat alone — the
    /// counter stays at 0 and the equivocator's dealing is replayed into the
    /// ceremony.
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
                // THE BUFFER: no tick yet ⇒ no slot ⇒ both dealings are buffered;
                // the evidence is on disk, not in a slot, so nothing is refused here.
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

                // THE DRAIN: the first tick resumes `Dealing`, the evidence comes
                // back with the ceremony, the buffer drains into it.
                actor.on_height(SEAL_DEADLINE - 1, &mut arng).await;
                assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("dealing"));
                // Self-verification of the fixture: the evidence is held for d1.
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

    /// F-02: a body-lost signal that arrives while this node is still DEALING
    /// (its clock lags the instance) is kept in the slot and applied at the seal:
    /// the sealed ceremony goes straight to `Acquiring(ArtifactForCeremony)`,
    /// `Stalled{BodyLost}`, and the pull starts on that tick — not at the
    /// boundary. Falsifier: the signal dropped on `Dealing` — the epoch seals
    /// into `Sealed` and nothing is asked.
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
            // Into the deal window: the epoch-2 ceremony starts and is still dealing.
            actor.on_height(SEAL_DEADLINE - 1, &mut arng).await;
            assert_eq!(actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH), Some("dealing"));

            actor.on_body_lost(DETERMINISTIC_BOOTSTRAP_EPOCH);
            assert_eq!(
                actor.phase(DETERMINISTIC_BOOTSTRAP_EPOCH),
                Some("dealing"),
                "nothing to acquire for before the seal"
            );
            assert!(asked.lock().expect("asked").is_empty());

            // The seal: applied here.
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

    /// F-03: `BodyMissing` and `QuorumMissing` are two latches of `Agreed`, each
    /// on its own condition. An agreed set with a body this node lacks is
    /// `Stalled{BodyMissing}` and nothing is said about the quorum yet (the probe
    /// cannot read it over missing bodies); once that body lands and the set is
    /// still below the quorum, the epoch is `Stalled{QuorumMissing}` and the
    /// `BodyMissing` latch has LEFT — one gauge step, not two. Falsifier: the
    /// `BodyMissing` latch kept past `all_held` (both held), or `QuorumMissing`
    /// raised over a set whose bodies are not all here (both held at the first
    /// tick).
    #[test]
    fn the_body_missing_latch_leaves_when_every_pinned_body_is_held() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle = sim_oracle(&ctx);
            let (committee, key0, journal) = node0_pre_seal_journal_full_sealed(131);
            oracle.manager().track(0, committee.clone()).await;
            let me0 = key0.public_key();
            // Own log only in the ceremony; ONE peer log kept aside to land later.
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

            // The body lands: every pinned body held, the quorum still not.
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

    /// DB-08: a `ReceivedDealing` whose journal write failed is not lost with the
    /// withheld ack — it is retried from the record on the next publish edge, and
    /// lands once the directory is back, so a restart's resume rebuilds
    /// `Player.view` from the journal. Falsifier: the failed record dropped —
    /// the journal never carries the peer's dealing.
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

            // Dealer 1's dealing for this node — the commitment and the private
            // share addressed to `me` — against the broken directory.
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

            // The directory is back; the next tick's publish edge retries it.
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

    /// The dealers whose `ReceivedDealing` the bootstrap epoch's journal in `dir`
    /// carries — what a restart's resume would rebuild `Player.view` from.
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

    /// DB-08 at the start-race DRAIN: a buffered dealing whose `ReceivedDealing`
    /// did not land when the drain journaled it is queued for the retry, exactly
    /// as the live dispatch queues one (`journal_or_defer`, the one form). Fresh
    /// start over a broken directory: dealer 1's dealing (the commitment and my
    /// share) is buffered before the first tick; the tick starts the ceremony and
    /// drains — the write fails and the record is queued; the directory is back,
    /// the next edge lands it. Falsifier (M2): the drain only withholds the ack —
    /// the queue stays empty and the journal never carries the peer's dealing.
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
            // THE BUFFER: no tick yet ⇒ no slot ⇒ the dealing is buffered whole.
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

            // THE DRAIN, against the broken directory: a fresh start (no journal
            // to read there) and the replay of the buffered dealing, whose record
            // does not land.
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

            // The directory is back; the next tick's publish edge retries it.
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

    /// THE STRADDLE IS UNREACHABLE BECAUSE THE DECISION NO LONGER COMPARES ROSTERS
    /// (Д-7), and this is what replaced the test of the pair reader that closed it.
    ///
    /// `the_start_decision_reads_both_rosters_at_one_state` staged a `CommitteeFor`
    /// whose answer for the SAME epoch changed between calls — a validator rotating
    /// its consensus key between two reads — and asserted that `CommitteePairFor`
    /// resolved one state hash for both epochs so the node could not see a "change"
    /// the contract never recorded. The decision reads the contract's own
    /// `changed[target]` bit now, so there is no second read to straddle and the pair
    /// reader is deleted rather than kept for the property.
    ///
    /// What still has to hold, and what this asserts, is that a straddling ROSTER
    /// reader cannot move the decision: the bit decides, the roster only says who
    /// deals. The fixture is the same straddle, and the two answers it gives are both
    /// accepted for the ceremony while the bit alone says whether to start.
    #[test]
    fn a_straddling_roster_reader_cannot_move_the_start_decision() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let a = distinct_peer_set(0);
        let b = distinct_peer_set(1);

        // A straddling single-epoch reader: same epoch, different answers per call.
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

        // The BIT is the decision, and it is a pure function of the epoch: whatever the
        // roster reader does between calls, `mints_at` answers the same thing.
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
