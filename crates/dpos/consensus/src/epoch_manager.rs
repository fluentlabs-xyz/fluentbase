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
    beacon::dkg_engine::agreement_partition,
    beacon::keys::{pk_prefix, AgreedKeys, BeaconKeys, KeySource, KeySources, OwnKeyFor},
    engine::{EpochEngine, EpochEngineConfig},
    epocher::OriginEpocher,
    order_block::OrderBlock,
    outer::{EpochSchemeProvider, SharedMux, SCHEME_RETENTION_EPOCHS},
    scheme::soft_enter_verifier,
    slasher::Mailbox as SlasherMailbox,
    timeouts::ConsensusTimeouts,
    weighted_vrf::{constant_fallback_seed, witness_fallback_seed},
};
use commonware_consensus::{
    marshal::{core::Mailbox as MarshalMailbox, standard::Standard},
    types::{Epoch, Epocher as _, Height, Round, View},
};
use commonware_cryptography::ed25519::PublicKey;
use commonware_p2p::{Blocker, Receiver, Sender};
use commonware_runtime::{
    buffer::paged::CacheRef, spawn_cell, BufferPooler, Clock, ContextCell, Handle, Metrics,
    Spawner, Storage,
};
use commonware_utils::vec::NonEmptyVec;
use fluentbase_bls::{
    beacon as beacon_bls, beacon::GroupPublic, fluent_namespace, keys::ValidatorBlsKeypair,
    scheme::BeaconKey, Scheme as BlsScheme,
};
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use futures::future::BoxFuture;
use rand_core::CryptoRngCore;
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::{mpsc, watch, Notify};
use tracing::{debug, info, warn};

/// The per-epoch validator role — a pure function of current state, NOT the
/// emergent product of a boundary-coupled transition zoo. A committee member
/// becomes a [`Role::Signer`] the instant it is in `committee[E]` at the live
/// frontier and caught up to the upstream tip — no epoch-boundary wait (the
/// cycle-2 fix; see the `dpos_role_state_binding` plan).
///
/// The decision is BEACON-INDEPENDENT and SYNC-INDEPENDENT: whether a node holds
/// a usable DKG share, and whether the E-1 boundary block has reached the local
/// marshal yet, are SPAWN-time concerns [`Actor::reconcile_roles`] gates
/// SEPARATELY (the share-gate and the `Inline::genesis` precondition) — a
/// `Signer` that holds no share for a beacon-active epoch, or whose boundary
/// block has not yet landed, stays on the verify-only scheme (no participating
/// engine) until both hold, because a shareless Simplex member rejects honest
/// peers' seeded votes and wedges the chain, and the engine `unreachable!`s
/// without its boundary block. Neither is modelled by the role verdict itself.
///
/// "Caught up" is NOT a separate signal: a node reaches the live frontier when
/// `is_live` (its f+1-corroborated `highest_observed_epoch` reaches `E`) and its
/// always-on executor has derived the chain up to E-1's boundary (the
/// `Inline::genesis` spawn gate). The validator's executor is the sole reth
/// writer and follows the chain by LOCAL derivation, so no cert-follow plane and
/// no `caught_up` flag are needed on a validator. The membership→role map is
/// `Signer` iff `is_member`, evaluated inline at the sole call site in
/// [`Actor::reconcile_roles`] (the caller only reaches it at the live frontier,
/// so liveness is not a separate input).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Run a participating Simplex engine for the epoch (propose / vote / sign).
    Signer,
    /// Verify-only: follow finalized certs, never propose or sign.
    Verifier,
}

/// Outcome of the `Inline::genesis(E)` precondition lookup on the E-1 terminal
/// block (see [`Actor::boundary_lookup`]), which also supplies the seedless arm's
/// base for epoch E.
enum BoundaryLookup {
    /// No predecessor epoch (epoch 0) or no computable terminal height. Nothing to
    /// wait for and no seed to inherit — every node reaches this identically, so
    /// the constant base stays agreed.
    NotApplicable,
    /// The E-1 terminal block is not in marshal storage yet — defer the spawn.
    Missing,
    /// The block is present. `seed` is [`witness_fallback_seed`] of its
    /// `parent_seed` — a one-way compression of the threshold signature, neither
    /// the signature itself nor the round that witness pins. `None` only on
    /// pre-bootstrap links where the witness is forbidden.
    Present { seed: Option<[u8; 32]> },
}

/// Which base the leader elector's seedless arm gets for an epoch, and why —
/// the variant, not just the bytes, so the caller can meter the predictable case
/// without re-deriving the choice.
#[derive(Debug, PartialEq, Eq)]
enum SeedlessBase {
    /// Inherited from the E-1 terminal block's witness seed: not derivable from
    /// constants, so the epoch's first leader is not known an epoch ahead.
    Witness([u8; 32]),
    /// The constant derivation, reached only where no witness can exist. Agreed
    /// across nodes but predictable.
    Constant([u8; 32]),
}

/// Choose the seedless arm's base from the E-1 terminal-block lookup. `None` for
/// [`BoundaryLookup::Missing`]: that epoch defers its spawn rather than electing
/// anything, so there is no base to choose. Pure — the metric for the predictable
/// case is emitted by the caller.
fn seedless_base(lookup: &BoundaryLookup, snap: &ValidatorSetSnapshot) -> Option<SeedlessBase> {
    match lookup {
        BoundaryLookup::Missing => None,
        BoundaryLookup::Present {
            seed: Some(witness),
        } => Some(SeedlessBase::Witness(*witness)),
        // Witness-less terminal block or no predecessor: the constant base is
        // predictable, but no seed exists to do better, and every node takes this
        // branch on the same block.
        BoundaryLookup::NotApplicable | BoundaryLookup::Present { seed: None } => {
            Some(SeedlessBase::Constant(constant_fallback_seed(snap)))
        }
    }
}

/// Outcome of a per-epoch beacon-key resolve (see [`BeaconResolver`]).
pub enum BeaconResolve {
    /// The epoch's key + share — the mint at the chain's `dkgQual` key epoch
    /// (`beacon::carry::select_carry_scheme`), an exact-epoch ceremony or a
    /// carried one no re-mint superseded.
    Key(BeaconKey),
    /// No usable local material for the epoch: nothing stored at or below it,
    /// a chain-declined or superseded mint, or an undecided (unreadable)
    /// `dkgQual` bit. ⇒ a fallback (pure-multisig) epoch / share-gate demote;
    /// re-resolved on the next edge.
    Absent,
}

/// Resolves the per-epoch [`BeaconKey`] (live-DKG store + `dkgQual`-bit-gated
/// carry-forward). Built at the launch site over the `CeremonyStore`; see
/// `dpos.rs::beacon_share_resolver`. This is the LOCAL DKG material (full
/// polynomial + this node's share) — required to SIGN seed partials and
/// verify individual partials. The polynomial is NOT on-chain, so this stays
/// node-local.
pub type BeaconResolver = Arc<dyn Fn(u64) -> BeaconResolve + Send + Sync>;

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

/// Target epochs below the cutoff whose agreement journal partition is swept on
/// every prune.
///
/// The sweep is what makes the removal survive CANCELLATION. An agreement
/// supervisor removes its own partition after it delivers, but every external
/// `abort()` — this prune, the SafetyHalt unwind, the manager's own exit —
/// cancels it at an await and that removal never runs. Nothing else reclaims
/// `dkg_epoch_{n}`, and unlike the ceremony journals there is no reconciler for
/// it, so the directory and its one blob per traversed view survive forever.
///
/// A band swept on every prune reclaims them all: it is driven by epoch NUMBER
/// rather than by a map this process populated, so it also collects a previous
/// process's leftovers on the first boundary after a restart, and it re-collects a
/// partition that an aborted-but-not-yet-stopped voter recreated with one last
/// journal write. The band is the trailing scheme-retention window — the same
/// distance the rest of this actor keeps state for.
const AGREEMENT_SWEEP_SPAN: u64 = SCHEME_RETENTION_EPOCHS as u64;

/// Drop every epoch-key agreement instance whose target is below `cutoff`,
/// aborting it on the way out, and reclaim the journal partitions of the targets
/// below the cutoff that no longer have one.
///
/// The SAME cutoff the per-epoch engines are pruned on, and for a matching
/// reason: below the frontier an instance has either delivered its artifact —
/// its supervisor has already returned, so the abort is a no-op — or it is still
/// agreeing a key for an epoch the chain has gone past.
async fn prune_agreements<E: Storage>(
    context: &E,
    agreements: &mut BTreeMap<Epoch, Handle<()>>,
    cutoff: u64,
) {
    let stale: Vec<Epoch> = agreements
        .keys()
        .copied()
        .filter(|e| e.get() < cutoff)
        .collect();
    for e in stale {
        if let Some(handle) = agreements.remove(&e) {
            handle.abort();
            // JOINED, and before the sweep below — for the same reason
            // `run_agreement` joins before destroying its own partition: `abort`
            // only REQUESTS cancellation, so the simplex voter under this
            // supervisor runs on to its next await and its `on_stopped` still
            // syncs the journal. A sweep that ran while it was still stopping
            // would remove a partition the voter then recreates with one last
            // write, and nothing reclaims that one. The map removal above is what
            // makes the guard below unable to protect these epochs, so the wait
            // has to be here.
            drop(handle.await);
            info!(?e, "epoch-key agreement instance pruned (transition)");
        }
    }
    for epoch in cutoff.saturating_sub(AGREEMENT_SWEEP_SPAN)..cutoff {
        // A live instance still owns its partition even below the cutoff: it was
        // adopted after this prune's abort pass.
        if agreements.contains_key(&Epoch::new(epoch)) {
            continue;
        }
        match context.remove(&agreement_partition(epoch), None).await {
            Ok(()) => info!(epoch, "epoch-key agreement journal partition reclaimed"),
            // Nothing to reclaim: the overwhelmingly common case, since the band is
            // swept on every prune whether or not an instance ever ran there.
            Err(commonware_runtime::Error::PartitionMissing(_)) => {}
            Err(err) => warn!(
                epoch,
                ?err,
                "could not reclaim the epoch-key agreement journal partition"
            ),
        }
    }
}

/// `recv()` on an optional receiver, or park forever when it is `None` — so the
/// agreement-intake branch of the manager's `select!` is inert on a node with no
/// agreement plane wired, without a second loop shape.
async fn recv_agreement(
    rx: Option<&mut mpsc::Receiver<(Epoch, Handle<()>)>>,
) -> Option<(Epoch, Handle<()>)> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

/// The 3 plane-owned simplex broker handles a SIGNER engine registers per-epoch
/// sub-channels against (vote/cert/resolver). Bundled into one struct so the
/// manager threads a single `Option<Muxes>` instead of 3 positional handles —
/// `None` ⇒ a FOLLOWER (no plane): it only ever soft-enters (Verifier forever,
/// `signer_keypair == None`), so [`Actor::spawn_engine`] is unreachable and the
/// muxes are never touched. This makes "a follower has no plane" a COMPILE-time
/// fact (the `None` arm) rather than a fabricated-but-idle socket plane.
pub struct Muxes<HS, HR>
where
    HS: Sender<PublicKey = PublicKey>,
    HR: Receiver<PublicKey = PublicKey>,
{
    pub vote: SharedMux<HS, HR>,
    pub cert: SharedMux<HS, HR>,
    pub res: SharedMux<HS, HR>,
}

/// Max distinct future epochs one peer may pin on the vote backup channel before
/// its live frontier is corroborated. Two covers the legitimate case (a peer is
/// at most ~1 boundary ahead of what it gossips) with slack; together with the
/// committee bound this caps the corroboration map at `n · 2` epochs and stops a
/// Byzantine minority from crowding out the honest frontier.
const PINS_PER_SENDER: usize = 2;

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
    /// Supervisor handles of the epoch-key AGREEMENT instances, keyed by their
    /// TARGET epoch — a SECOND map, and it must stay separate from
    /// `active_epochs`.
    ///
    /// `reconcile_roles` polls `engine_handle_dead` over `active_epochs` and reads
    /// a COMPLETED handle as a caught panic: it drops the entry, bumps
    /// `engine_respawned` and spawns a replacement. An agreement instance is
    /// SUPPOSED to complete — it aborts itself the moment its own finalization
    /// lands ([`crate::beacon::dkg_engine`]) — so an entry over there would be
    /// resurrected on every reconcile, for the life of the process. Here nothing
    /// polls it: a completed supervisor simply sits until `abort_below` drops it,
    /// and `abort()` on an already-completed handle is a no-op.
    dkg_agreements: BTreeMap<Epoch, Handle<()>>,
    /// Where those handles come from. `None` ⇒ no agreement plane wired ⇒ the
    /// branch parks forever and the map stays empty; the beacon plane holds the
    /// sending half and posts a handle per instance it starts.
    agreement_intake: Option<mpsc::Receiver<(Epoch, Handle<()>)>>,
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
    /// `0` until the first reconcile full-enters an epoch, during which backup
    /// corroboration is disabled (the cold-start epoch full-enters from the
    /// verified boundary trigger).
    committee_size: usize,
    /// The role this node currently holds per epoch — the single source of truth
    /// the reconciler diffs against. `Signer` ⟺ a participating engine in
    /// `active_epochs`; `Verifier` ⟺ a verify-only scheme registered, no engine.
    roles: BTreeMap<Epoch, Role>,
    /// Live-frontier epochs whose `Verifier→Signer` spawn is parked on marshal
    /// block availability: the `Inline::genesis(E)` precondition (the E-1 boundary
    /// block not yet in marshal storage) — resumes as backfill lands.
    /// Re-checked on every reconcile edge (boundary / share /
    /// spawn_unblocked / vote_backup) — `reconcile_roles` is idempotent, so a parked
    /// epoch spawns the instant the boundary block lands. Never panics (defer, never
    /// `unreachable!`).
    deferred_spawns: BTreeSet<Epoch>,
    /// The most-recent boundary delivery `(epoch, snapshot)`. The non-boundary
    /// edges (share / spawn_unblocked / vote_backup) carry no snapshot, so they
    /// reconcile the CURRENT live epoch from this cache.
    latest_live: Option<(Epoch, ValidatorSetSnapshot)>,
    /// Last `(highest_entered, highest_observed)` pair for which
    /// [`pipeline_catchup_span`] ran and registered nothing new (bug 15).
    /// Identical re-attempts are suppressed until an edge (share landed /
    /// execution progressed / frontier moved) clears it, so a backup-vote storm
    /// (128/s/peer × n) cannot re-run the EVM span fan-out + marshal hint per
    /// vote while nothing changed.
    catchup_no_progress: Option<(Epoch, Epoch)>,
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
    pub app: FluentApp<XC, A>,
    pub timeouts: ConsensusTimeouts,
    pub mailbox_size: usize,
    /// Per-epoch beacon resolver: returns the [`BeaconKey`] (`PK_epoch` sharing +
    /// this node's share + namespace) for `epoch`, sourced from the live-DKG
    /// store under the frozen on-chain `dkgQual`-bit carry arbitration
    /// (`beacon::carry`). [`BeaconResolve::Absent`] ⇒ a fallback (pure-multisig)
    /// epoch. Called per epoch in `reconcile_roles` for the live engine + the
    /// soft-enter verifier.
    pub beacon_resolver: BeaconResolver,
    /// Edge-trigger the `DkgActor` fires when a share lands in the live-DKG store.
    /// `enter()` arms `notified()` and re-checks `beacon_resolver`, so a signer that
    /// reaches the boundary before its share is memoized wakes the instant it lands
    /// instead of polling. Same `Arc` the actor holds (via `SharedBeaconPlane`).
    pub beacon_share_notify: Arc<Notify>,
    /// Edge-trigger fired by the executor each time it records a finalized
    /// OrderBlock (i.e. the marshal now holds another finalized block). It is the
    /// MID-EPOCH promotion trigger (the cycle-2 fix): re-runs `reconcile_roles` for
    /// the live epoch so a caught-up member promotes the instant its
    /// `Inline::genesis(E)` precondition is met (the E-1 boundary block landing IS
    /// an executor finalized-advance) — no boundary-finalize wait. Fires even in a
    /// thin-quorum stall, because the LOCAL executor still advances to the stall
    /// tip while the chain is globally stalled.
    pub spawn_unblocked: Arc<Notify>,
    /// Fork-safety latch (Phase 3 `SafetyHalt`). Read by [`Actor::reconcile_roles`]
    /// so a halted node is NEVER (re-)promoted to a participating `Signer`
    /// (verify-only forever), and awaited on the [`Actor::run`] select so the
    /// instant the executor / a re-jump engages it (result divergence / EL Invalid
    /// / L1 fork) every running engine is aborted — stop signing/proposing/voting
    /// immediately, not just at the next boundary. Cross-launch singleton from
    /// `dpos.rs::launch`, shared with the executor + the OuterEngine supervisor.
    pub safety_halt: crate::sync_metrics::SafetyHalt,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`].
    pub marshal_mailbox: MarshalMailbox<BlsScheme, Standard<OrderBlock>>,
    /// Peers to target when re-driving a finalization fetch: the highest known
    /// epoch's committee, from `EpochSchemeProvider::latest_scheme`. The SAME
    /// closure the executor's catch-up re-fetch uses — built once in
    /// [`crate::outer::OuterBuilder::build`].
    pub peers_for_finalization: crate::executor::PeersForFinalization,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`].
    pub slasher_mailbox: SlasherMailbox,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`]: the
    /// notarization arm of the simplex reporter, forwarding `SpecNotarized`
    /// commands to the executor for speculative execution.
    pub spec_exec_mailbox: crate::spec_exec::Mailbox,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`]: the SAME
    /// `Arc` handed to [`FluentApp`] (an `Arc` clone, not a second map — W1
    /// writes here and the vote path reads through `FluentApp`). The manager
    /// cannot reach the map through `cfg.app` (no accessor; `app` is moved),
    /// so it holds its own clone for writer W1 (insert `(E, PK_E)` BEFORE
    /// `spawn_engine`) and writer W3 (best-effort `E−1` backfill).
    pub group_keys: BeaconKeys,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`]: beacon counters,
    /// threaded into each per-epoch engine for the demote counters.
    pub beacon_metrics: crate::beacon::metrics::BeaconMetrics,
    /// Cross-epoch singleton from [`crate::outer::OuterEngine`].
    pub page_cache: CacheRef,
    /// Callback into [`crate::outer::EpochSchemeProvider`] so marshal can verify
    /// cross-epoch finalization certificates (trailing-window pruned; see SCHEME_RETENTION_EPOCHS).
    pub register_scheme: Arc<dyn Fn(Epoch, BlsScheme) + Send + Sync>,
    /// The same [`crate::outer::EpochSchemeProvider`] `register_scheme` writes
    /// into, held directly because the repair sweep has to READ it: which epochs
    /// are registered unpinned is a question only the registry can answer, and
    /// the answer covers registration paths the epoch manager never sees (the
    /// bulk catch-up span, `cold_start_register`).
    pub scheme_pins: EpochSchemeProvider,
    /// The `PK_epoch` ladder's held-artifact rung: the agreement artifacts this
    /// node already holds, keyed through the chain's `dkgQual` record. `None`
    /// where the node has no artifact store (unit tests, a pure follower), which
    /// makes the rung absent rather than answering "undecided" forever — the
    /// store rung still answers.
    pub held_keys: Option<AgreedKeys>,
    /// The same ladder's NETWORK rung: one bounded pull of the minting epoch's
    /// artifact from a peer. Spent only by the off-vote-path repair sweep — never
    /// by `soft_enter`, whose caller is on the reconcile loop. `None` where there
    /// is no plane to pull through.
    pub pull_keys: Option<AgreedKeys>,
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
    pub byzantine: Option<crate::byzantine::ByzantineMode>,
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
            dkg_agreements: BTreeMap::new(),
            agreement_intake: None,
            boundary_rx,
            highest_entered_epoch: Epoch::new(0),
            highest_observed_epoch: Epoch::new(0),
            observed_reporters: BTreeMap::new(),
            sender_pins: BTreeMap::new(),
            committee_size: 0,
            roles: BTreeMap::new(),
            deferred_spawns: BTreeSet::new(),
            latest_live: None,
            catchup_no_progress: None,
            cfg,
        };
        (actor, boundary_tx)
    }

    /// Take ownership of the epoch-key agreement instances the beacon plane
    /// starts, so they are pruned on the same frontier cutoff as the per-epoch
    /// engines and torn down with the manager. Left unwired the manager owns none
    /// and the branch is inert.
    pub fn with_agreement_intake(
        mut self,
        agreement_intake: mpsc::Receiver<(Epoch, Handle<()>)>,
    ) -> Self {
        self.agreement_intake = Some(agreement_intake);
        self
    }

    /// Start the manager. The 3 simplex broker handles (vote/cert/resolver) are
    /// owned by the always-on plane (node crate); this manager CLONES them per
    /// promotion to register per-epoch sub-channels and drops them on exit (the
    /// `SubReceiver`s auto-deregister, freeing the slots for the next promotion). The
    /// vote Muxer's backup receiver is the plane's re-settable forwarder, fresh per
    /// promotion.
    pub fn start<HS, HR>(
        mut self,
        muxes: Option<Muxes<HS, HR>>,
        vote_backup: mpsc::Receiver<(u64, (PublicKey, commonware_runtime::IoBuf))>,
    ) -> Handle<()>
    where
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
    {
        spawn_cell!(self.context, self.run(muxes, vote_backup).await)
    }

    async fn run<HS, HR>(
        mut self,
        muxes: Option<Muxes<HS, HR>>,
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
        // Cloned Arcs so the per-iteration `notified()` futures borrow the local
        // handles, not `self` (the arms below take `&mut self`).
        let share_notify = self.cfg.beacon_share_notify.clone();
        let spawn_unblocked = self.cfg.spawn_unblocked.clone();
        let safety_halt = self.cfg.safety_halt.clone();
        // Captured ONCE, before the loop — never per-iteration. A `Notify` permit
        // is object-scoped, so re-arming `notified()` on THIS handle each
        // iteration below cannot lose a fill; re-deriving the handle each
        // iteration is what would (see `beacon::keys`).
        let key_notify = self.cfg.group_keys.notifier();
        // The repair sweep runs OFF this loop — see `spawn_repair_sweep`. Waking
        // it is a synchronous `send_replace`, so no arm below can be held by it.
        // Dropping this sender on the way out is what stops the task.
        let (sweep_wake, sweep_handle) = self.spawn_repair_sweep();
        // Taken out so the intake branch borrows the local rather than `self` (the
        // arms below take `&mut self`).
        let mut agreement_intake = self.agreement_intake.take();
        loop {
            // Arm the edge wakeups BEFORE the select. The producers use `notify_one`
            // (permit-storing), so even a signal that fires while no waiter is armed —
            // between a reconcile and the next select — is held as a permit and
            // consumed by the next `notified()` (no lost wakeup).
            let share_n = share_notify.notified();
            let spawn_n = spawn_unblocked.notified();
            let halt_n = safety_halt.engaged_edge();
            let key_n = key_notify.notified();
            tokio::pin!(share_n, spawn_n, halt_n, key_n);
            tokio::select! {
                // Edge: the fork-safety latch was engaged (result divergence / EL
                // Invalid / L1 fork). Abort every participating engine NOW so the node
                // stops signing/proposing/voting immediately (not at the next
                // boundary), and clear any parked promotion. `reconcile_roles` keeps
                // it a Verifier forever thereafter (the latch is permanent). The
                // manager itself stays UP (marshal keeps verifying certs); recovery is
                // external (L1 proof + governance).
                _ = &mut halt_n => {
                    for (epoch, handle) in std::mem::take(&mut self.active_epochs) {
                        warn!(?epoch, "SafetyHalt engaged — aborting participating engine \
                            (demote to verify-only permanently)");
                        handle.abort();
                    }
                    // The agreement plane goes with them. Its output is a share
                    // this node will never get to sign with — the latch is
                    // permanent and `reconcile_roles` keeps it a Verifier forever
                    // — so leaving it voting on a second plane after a fork-safety
                    // halt buys nothing and costs the network a vote it should
                    // not be casting.
                    for (epoch, handle) in std::mem::take(&mut self.dkg_agreements) {
                        warn!(?epoch, "SafetyHalt engaged — aborting epoch-key agreement instance");
                        handle.abort();
                    }
                    self.deferred_spawns.clear();
                    for role in self.roles.values_mut() {
                        *role = Role::Verifier;
                    }
                }
                recv = self.boundary_rx.recv() => {
                    match recv {
                        Some((epoch, snap)) => {
                            // The only edge carrying a fresh snapshot — cache it for the
                            // non-boundary edges, then reconcile (folds enter + prune_old).
                            self.latest_live = Some((epoch, snap.clone()));
                            self.reconcile_roles(epoch, snap, muxes.as_ref()).await;
                            // AFTER the reconcile, so the frontier the sweep runs
                            // under already includes this boundary's epoch and it
                            // cannot pin under the engine just spawned for it.
                            sweep_wake.send_replace((
                                self.highest_observed_epoch,
                                self.highest_entered_epoch,
                            ));
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
                            // Corroboration / catch-up only affects a role decision when it
                            // moves the frontier (`highest_observed_epoch` flips `is_live`)
                            // or the entered tip (`highest_entered_epoch` registers new
                            // schemes). Reconcile ONLY on that change — otherwise every
                            // backup vote (>100/s during catch-up) re-runs an identical
                            // no-op reconcile. The change-gate still breaks the cycle-2
                            // deadlock: the FIRST vote that corroborates the new frontier
                            // reconciles, even with the chain stalled.
                            let before = (self.highest_observed_epoch, self.highest_entered_epoch);
                            self.handle_msg_for_unregistered_epoch(Epoch::new(their_epoch), from).await;
                            if (self.highest_observed_epoch, self.highest_entered_epoch) != before {
                                self.reconcile_live(muxes.as_ref()).await;
                            }
                        }
                        None => {
                            info!("vote backup channel closed, epoch_manager exiting");
                            break;
                        }
                    }
                }
                // Edge: a DKG share landed — re-run reconcile so a member parked by
                // the share-gate spawns now that its share is present (the running
                // scheme is frozen at construction, so this is a respawn). Also
                // clears the catch-up no-progress memo (a landed share can make a
                // previously-unresolvable span read succeed — bug 15).
                _ = &mut share_n => {
                    self.catchup_no_progress = None;
                    self.reconcile_live(muxes.as_ref()).await;
                }
                // Edge: the executor recorded a finalized block — the MID-EPOCH
                // promotion trigger. A caught-up member promotes the instant its
                // `Inline::genesis` precondition is met; gated on a pending parked
                // spawn so the per-block fire is a no-op in steady state. Execution
                // progress can also unblock a catch-up span read → clear the memo.
                _ = &mut spawn_n => {
                    self.catchup_no_progress = None;
                    if !self.deferred_spawns.is_empty() {
                        self.reconcile_live(muxes.as_ref()).await;
                    }
                }
                // Edge: a `PK_epoch` landed in the shared store — the FAST path
                // into the repair sweep, not the load-bearing one. The LIVE
                // frontier needs neither: `soft_enter` does not short-circuit on a
                // recorded `Role::Verifier`, and `reconcile_live` re-runs the ladder
                // on every non-boundary edge while the epoch is still the frontier.
                // A BELOW-frontier epoch has no other retry — `reconcile_live`'s
                // `is_live_epoch` guard returns before touching it.
                //
                // This arm CANNOT be the only trigger, and that is the whole reason
                // the boundary arm also sweeps. It fires on a `BeaconKeys::record`,
                // and every production writer of that store needs either this node's
                // own DKG material or a change epoch's agreement artifact. A node
                // with no DKG material on a committee that has not changed — the
                // case this repair exists for — never fires it at all.
                //
                // NB a near-miss that makes a cheaper fix tempting and wrong: an
                // attested insert IS followed one task-hop later by `spawn_unblocked`, but
                // that arm is gated on a non-empty `deferred_spawns`, and a node
                // demoted by the promote value-gate never enters that set. Same
                // event, wrong gate.
                _ = &mut key_n => {
                    sweep_wake.send_replace((
                        self.highest_observed_epoch,
                        self.highest_entered_epoch,
                    ));
                }
                // Edge: the beacon plane started an epoch-key agreement instance
                // and handed us its supervisor. Adopting it here is what puts the
                // instance under the same frontier cutoff as the engines; a second
                // handle for an epoch we already hold replaces the first, aborting
                // it so two instances never run for one target.
                adopted = recv_agreement(agreement_intake.as_mut()) => match adopted {
                    Some((epoch, handle)) => {
                        if let Some(previous) = self.dkg_agreements.insert(epoch, handle) {
                            warn!(?epoch, "replacing a live epoch-key agreement instance");
                            previous.abort();
                        }
                    }
                    None => agreement_intake = None,
                },
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
        for (epoch, handle) in std::mem::take(&mut self.dkg_agreements) {
            info!(?epoch, "aborting epoch-key agreement instance on exit");
            handle.abort();
        }
        // Aborted, not left to notice the dropped sender: it can be parked inside
        // a peer pull for the whole `PULL_TIMEOUT`, and nothing it could still do
        // matters once the manager that owns the scheme registry is gone.
        sweep_handle.abort();
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
            &mut self.catchup_no_progress,
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

    /// Reconcile this node's per-epoch role from current state — the single
    /// decision point, folding the old `enter` + `prune_old`. `role(E) = Signer iff
    /// (I ∈ committee[E]) ∧ is_live_epoch(E)`, else `Verifier`; a `Signer`
    /// additionally needs a usable DKG share (share-gate) and the
    /// `Inline::genesis(E)` precondition before its engine spawns. Idempotent — safe
    /// to call repeatedly for the same `(epoch, snap)` on any edge.
    async fn reconcile_roles<HS, HR>(
        &mut self,
        epoch: Epoch,
        snap: ValidatorSetSnapshot,
        muxes: Option<&Muxes<HS, HR>>,
    ) where
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
    {
        // W3 (P1) — best-effort group-key backfill for the PREVIOUS epoch, off
        // the vote path: covers a node promoted mid-`E` that never ran `E−1`'s
        // engine (no W1 entry for `E−1`). Insert ONLY on success — a failure is
        // never cached, so this re-attempts on every reconcile edge (boundary /
        // share / spawn_unblocked / vote_backup). A warm-up, not the boundary
        // repair path: the first block of `E+1` is verified ~1 s after the
        // spawn, so the repair that fires there is the per-vote lazy resolve.
        if let Some(prev) = epoch.get().checked_sub(1) {
            if self.cfg.group_keys.cached_only(prev).is_none() {
                // Best-effort: an undecided resolve is NOT backfilled — the
                // next edge re-attempts.
                if let BeaconResolve::Key((sharing, _, _)) = (self.cfg.beacon_resolver)(prev) {
                    let pk = *sharing.public();
                    debug!(
                        epoch = prev,
                        group_public = %pk_prefix(&pk),
                        "W3: backfilling previous-epoch group key from own DKG material"
                    );
                    self.cfg.group_keys.set_pk(prev, pk, KeySource::LocalDkg);
                }
            }
        }

        // Boundary bookkeeping (idempotent; monotone). Committee size is keyed on
        // the HIGHEST-ENTERED epoch (follows validator-set growth and shrink) — it
        // feeds the f+1 corroboration threshold. Reaching an epoch RESOLVES it:
        // free pending corroboration pins ≤ it so a healthy node's boundary-race
        // pins don't permanently mute honest senders.
        self.highest_entered_epoch = self.highest_entered_epoch.max(epoch);
        if epoch == self.highest_entered_epoch {
            self.committee_size = snap.validators.len();
        }
        // Prune the cross-epoch beacon-key store to the same trailing
        // scheme-retention window: every reader is an exact `lookup` for the epoch
        // under reconcile (near the entered frontier — W1/W3/attested/ladder), so
        // entries older than that can never be read again.
        self.cfg.group_keys.retain_from(
            self.highest_entered_epoch
                .get()
                .saturating_sub(SCHEME_RETENTION_EPOCHS as u64),
        );
        prune_resolved(
            &mut self.observed_reporters,
            &mut self.sender_pins,
            self.highest_entered_epoch,
        );

        // Exit-at-transition: abort every engine strictly below the live epoch
        // (folded `prune_old`; `e < cutoff` only, so a stale/replayed boundary for
        // an OLD epoch can never abort a newer engine).
        self.abort_below(epoch).await;

        // Below the live frontier → soft-enter (verify-only scheme, NO engine): a
        // Simplex engine for a stale epoch has no live peers and would drive a dead
        // fork. Verify-only lets the marshal verify this epoch's certs (with a
        // `PK_epoch` seed pin when the boundary block is resolvable — see
        // `soft_enter` / `BoundaryWalk::key_for`; else vote-only).
        if !self.is_live_epoch(epoch) {
            self.soft_enter(epoch, &snap).await;
            self.deferred_spawns.remove(&epoch);
            info!(?epoch, "epoch soft-entered (scheme only, catch-up)");
            return;
        }

        // At the live frontier (checked above): role = f(member). "Caught up" is not
        // a separate input: a member only spawns a participating engine once the
        // share-gate AND `boundary_lookup` both hold below, which together
        // mean the local executor has derived up to E-1's boundary.
        //
        // Fork-safety latch (Phase 3): a SafetyHalted node is NEVER a member for
        // role purposes — it stays a Verifier forever (verify-only, never
        // re-promoted), whatever the committee says. The halt edge above already
        // aborted any running engine; this keeps future reconciles from re-spawning.
        let is_member = !self.cfg.safety_halt.is_engaged()
            && self.cfg.signer_keypair.is_some()
            && snap
                .validators
                .iter()
                .any(|v| v.keys.peer_pubkey == self.cfg.me);

        // Already a running signer for the live epoch — keep it, UNLESS its handle
        // has completed. The committee is frozen per epoch, so membership cannot
        // change mid-epoch; a frontier move aborts this engine via `abort_below` on
        // the next boundary. But under `catch_panics(true)` a child engine panic
        // completes the `Handle` without reaching the manager (nothing joins engine
        // handles), so a dead engine would otherwise wedge behind this gate forever.
        // Poll the handle: `Pending` ⇒ alive, keep it; `Ready` ⇒ dead, drop the
        // entry and fall through to the spawn path (re-spawn is safe — W1
        // `insert_group_key` is idempotent and a pre-drop `AlreadyRegistered` is
        // handled by `spawn_engine` returning false, retried next edge). The
        // respawn stays behind every gate below (share-gate, boundary-block,
        // safety-halt), reached only because the caller is at the live frontier.
        if let Some(handle) = self.active_epochs.get_mut(&epoch) {
            if !engine_handle_dead(handle) {
                return;
            }
            self.active_epochs.remove(&epoch);
            self.cfg.beacon_metrics.engine_respawned.inc();
            warn!(
                ?epoch,
                "live-frontier engine handle completed unexpectedly (panic caught by \
                 catch_panics, or early exit) — respawning"
            );
        }

        // Role is a pure function of membership (the caller is already at the
        // live frontier, so liveness is not a separate input); see [`Role`].
        let assigned_role = if is_member {
            Role::Signer
        } else {
            Role::Verifier
        };
        match assigned_role {
            // Not a member (rotated out). Register verify-only so the marshal
            // verifies this epoch's certs; no participating engine.
            Role::Verifier => {
                if self.cfg.signer_keypair.is_some() && !is_member {
                    self.cfg.beacon_metrics.engine_demoted_rotated_out.inc();
                }
                self.soft_enter(epoch, &snap).await;
            }
            Role::Signer => {
                // Share-gate: a beacon-active member with no usable DKG share must
                // NOT run a participating engine — a `beacon: None` Simplex member
                // rejects honest peers' seeded votes (`combined_scheme::verify_attestation`)
                // and the batcher blocks them → wedge. Resolve the local share
                // NON-BLOCKINGLY (blocking would stall the whole reconcile loop, and
                // re-block on every share edge for a genuinely shareless member);
                // on absence, register verify-only + stay off the consensus plane
                // (the surviving NoBeaconPolynomial effect). The `beacon_share_notify`
                // edge re-runs reconcile and promotes the instant the share lands.
                let beacon_active =
                    epoch.get() >= crate::beacon::actor::DETERMINISTIC_BOOTSTRAP_EPOCH;
                let beacon = match (self.cfg.beacon_resolver)(epoch.get()) {
                    BeaconResolve::Key(key) => Some(key),
                    BeaconResolve::Absent => None,
                };
                if beacon_active && beacon.is_none() {
                    self.cfg.beacon_metrics.engine_demoted_no_polynomial.inc();
                    self.soft_enter(epoch, &snap).await;
                    info!(
                        ?epoch,
                        "committee member without a usable DKG share — verify-only (share-gate)"
                    );
                    return;
                }

                // `Inline::genesis(E)` precondition: the E-1 boundary block must be
                // in marshal storage before the per-epoch engine starts (else the
                // engine hits `unreachable!`). On a mid-epoch promotion the marshal
                // may still be backfilling it — DEFER, never panic; the executor's
                // `spawn_unblocked` edge (or the next boundary) re-pokes. Register
                // verify-only meanwhile so the marshal verifies this epoch's certs.
                let lookup = self.boundary_lookup(epoch).await;
                let fallback_seed = match seedless_base(&lookup, &snap) {
                    None => {
                        self.deferred_spawns.insert(epoch);
                        self.soft_enter(epoch, &snap).await;
                        self.cfg.beacon_metrics.engine_spawn_deferred.inc();
                        info!(
                            ?epoch,
                            boundary = ?epoch
                                .get()
                                .checked_sub(1)
                                .and_then(|prev| self.cfg.epocher.last(Epoch::new(prev)))
                                .map(|h| h.get()),
                            "signer spawn deferred — E-1 boundary block not yet in marshal; \
                             verify-only until it lands"
                        );
                        return;
                    }
                    Some(SeedlessBase::Witness(base)) => base,
                    Some(SeedlessBase::Constant(base)) => {
                        self.cfg.beacon_metrics.fallback_seed_constant.inc();
                        base
                    }
                };

                // Promote-gate VALUE check (defense-in-depth; f297cc36 extended
                // from key PRESENCE to key VALUE): when a `committee[E]` quorum
                // has already certified a `PK_epoch` for E, a resolver key that
                // DIFFERS is a diverged local reconstruction, whatever produced
                // it. Never sign, W1-publish, or witness-check under it: demote
                // to verify-only; the recompute-heal stores the correct
                // exact-epoch `(PK_E, share)` and its `share_notify` edge
                // re-runs this reconcile, which then promotes with the matching
                // key.
                //
                // The comparand is the agreement artifact, reached through the
                // store (`attested`). It used to be a second, weaker source
                // underneath: the boundary block's own `beacon_outcome`, read
                // by height. That source is strictly worse and no longer
                // exists — it required a block of E to have been produced and
                // stored, so it was blind at exactly the moment the gate is
                // asked (the epoch's first spawn), whereas the artifact is
                // certified BEFORE the epoch starts and covers a stable
                // carry-forward epoch, which no boundary block ever did.
                if let Some((sharing, _, _)) = beacon.as_ref() {
                    let pk = *sharing.public();
                    if let Some(net) = self.cfg.group_keys.attested(epoch.get()) {
                        if net != pk {
                            self.cfg.beacon_metrics.engine_demoted_key_divergence.inc();
                            warn!(
                                ?epoch,
                                resolved = %pk_prefix(&pk),
                                network = %pk_prefix(&net),
                                "resolved PK_epoch DIVERGES from the quorum-attested \
                                 key — verify-only (promote value-gate)"
                            );
                            self.soft_enter(epoch, &snap).await;
                            return;
                        }
                    }
                }

                // Promote-gate SHARE check. `CombinedScheme::new` asserts only that
                // the share's INDEX equals this node's participant index — never
                // that its VALUE lies on the sharing. While blocks flow, a bad share
                // is exposed on the notarize path; in a sustained stall there are no
                // proposals, so it is not, and since every Nullify now carries a seed
                // partial and `t == quorum`, one such member on the plane makes the
                // nullify quorum unreachable exactly when nullification is the escape
                // hatch. The probe is purely local (share vs its own sharing), so it
                // also covers the cold-ceremony window where the VALUE gate above is
                // a no-op for want of a network-attested key.
                if let Some((sharing, Some(share), namespace)) = beacon.as_ref() {
                    let probe = Round::new(epoch, View::new(1));
                    let partial = beacon_bls::sign_seed_partial(share, namespace, probe);
                    if !beacon_bls::verify_seed_partial(sharing, namespace, probe, &partial) {
                        self.cfg.beacon_metrics.engine_demoted_bad_share.inc();
                        warn!(
                            ?epoch,
                            "resolved DKG share does not verify against its own sharing — \
                             verify-only (promote share-gate)"
                        );
                        self.soft_enter(epoch, &snap).await;
                        return;
                    }
                }

                // W1 (P1) — publish `PK_epoch` into the cross-epoch group-key
                // map BEFORE the engine exists (never inside `spawn_engine`):
                // the engine cannot cast a vote before it is spawned, so every
                // node that votes on epoch E finds `group_keys[E]` populated at
                // its first vote — the same-epoch quorum argument (`b = 0`).
                // Infallible here: `beacon` is already resolved and the
                // share-gate above demoted the shareless case to verify-only.
                // One epoch later this same entry IS the boundary warm (W2):
                // `E+1`'s gate needs `PK_E` and reads the map, no I/O.
                //
                // It stands down where the epoch-key agreement plane already
                // spoke: an artifact is a `committee[epoch]` quorum over this
                // exact key, and a local reconstruction has nothing to add to
                // it. Rung 1 of the ladder is answered by the artifact's entry
                // either way, which is what `repair_unpinned_schemes` depends on
                // (`own` is `None` there BECAUSE W1 fills the store).
                if let Some((sharing, _, _)) = beacon.as_ref() {
                    let pk = *sharing.public();
                    match own_key_publication(self.cfg.group_keys.attested(epoch.get()), pk) {
                        OwnKeyPublication::Publish => {
                            // The value fingerprint is the point: a restarted signer whose
                            // carried-forward key diverged from the network's is only
                            // diagnosable by grepping this line across nodes (soak
                            // 2026-07-14 v5@epoch77 reject{bad_signature}).
                            info!(
                                ?epoch,
                                group_public = %pk_prefix(&pk),
                                "W1: publishing own epoch group key"
                            );
                            self.cfg
                                .group_keys
                                .set_pk(epoch.get(), pk, KeySource::LocalDkg);
                        }
                        OwnKeyPublication::DeferAgreeing => debug!(
                            ?epoch,
                            group_public = %pk_prefix(&pk),
                            "W1: the agreement plane already published this epoch's key"
                        ),
                        // The divergence witness the suppressed `set_pk` used to
                        // raise from inside the store. Same metric name, so a
                        // dashboard watching for a split key still sees this one.
                        OwnKeyPublication::DeferDiverging(agreed) => {
                            metrics::counter!(
                                "dpos_group_key_conflict_total",
                                "winner" => "agreed_kept"
                            )
                            .increment(1);
                            warn!(
                                ?epoch,
                                resolved = %pk_prefix(&pk),
                                agreed = %pk_prefix(&agreed),
                                "resolved PK_epoch DIVERGES from the quorum-agreed key; \
                                 keeping the agreed one"
                            );
                        }
                    }
                }
                if self
                    .spawn_engine(epoch, snap, beacon, fallback_seed, muxes)
                    .await
                {
                    self.roles.insert(epoch, Role::Signer);
                    self.deferred_spawns.remove(&epoch);
                    // Stable greppable token for the production-path smoke
                    // (`smoke-production-path`): the in-process Verifier→Signer
                    // promotion — a joiner that catches up + holds its DKG share
                    // re-promotes here without a process restart.
                    info!(
                        ?epoch,
                        "promoted to Signer in-process: per-epoch BFT engine started"
                    );
                }
            }
        }
    }

    /// Register a verify-only (multisig) scheme for `epoch` and record the
    /// `Verifier` role — UNLESS this node already holds a `Signer` for `epoch` (a
    /// running engine in `active_epochs` or a recorded `Signer` role). Idempotent:
    /// never downgrades an active signer to verify-only, which `EpochSchemeProvider`
    /// refuses (`signer→verifier downgrade`) and which would desync `self.roles`
    /// from the provider. This is the one site that READS `self.roles`, making it
    /// the diff source of truth the field doc promises.
    async fn soft_enter(&mut self, epoch: Epoch, snap: &ValidatorSetSnapshot) {
        if self.active_epochs.contains_key(&epoch) || self.roles.get(&epoch) == Some(&Role::Signer)
        {
            return;
        }
        let resolver = self.cfg.beacon_resolver.clone();
        let own: OwnKeyFor = Arc::new(move |e| match resolver(e) {
            BeaconResolve::Key((sharing, _, _)) => Some(*sharing.public()),
            BeaconResolve::Absent => None,
        });
        register_soft_entered(
            &self.cfg.group_keys,
            Some(&own),
            self.cfg.held_keys.as_ref(),
            epoch,
            snap,
            self.cfg.chain_id,
            self.cfg.register_scheme.as_ref(),
        )
        .await;
        self.roles.insert(epoch, Role::Verifier);
    }

    /// Start the repair sweep on its OWN task and return the handle that wakes
    /// it. Called once, before [`Self::run`]'s `select!`.
    ///
    /// **The sweep must not run on that `select!`.** Its ladder spends one
    /// bounded peer pull per unpinned epoch, and the pull's caller-side rate
    /// bound SLEEPS until the epoch's next slot before it even issues the fetch
    /// (`PULL_MIN_INTERVAL` + `PULL_TIMEOUT`), once per epoch in a work list
    /// bounded only by [`SCHEME_RETENTION_EPOCHS`]. Awaited inline that is a
    /// ~100 s section during which `share_n`, `spawn_unblocked` and `vote_backup`
    /// do not run — the three arms that turn this node into a signer.
    ///
    /// Everything the sweep touches is a cross-epoch singleton this actor holds
    /// by `Arc`, so the task needs no borrow of `self`; the only per-wake state
    /// is the frontier, which rides the wake. A `watch` (not a `Notify`) because
    /// the frontier has to ride it and because its receiver is created ONCE here,
    /// before the task's loop — the baseline-at-subscribe hazard that rules
    /// `watch` out in [`crate::beacon::keys`] needs a per-iteration `subscribe`,
    /// which this is not.
    fn spawn_repair_sweep(&self) -> (watch::Sender<(Epoch, Epoch)>, Handle<()>) {
        let (wake_tx, wake_rx) =
            watch::channel((self.highest_observed_epoch, self.highest_entered_epoch));
        let hint: RepairHint = {
            let epocher = self.cfg.epocher.clone();
            let peers = self.cfg.peers_for_finalization.clone();
            let mailbox = self.cfg.marshal_mailbox.clone();
            Arc::new(move |upgraded: Vec<Epoch>| {
                let epocher = epocher.clone();
                let peers = peers.clone();
                let mailbox = mailbox.clone();
                Box::pin(async move {
                    for epoch in upgraded {
                        let (Some(boundary), Some(targets)) = (epocher.last(epoch), peers()) else {
                            continue;
                        };
                        info!(
                            ?epoch,
                            boundary = boundary.get(),
                            "below-frontier epoch pinned by a late beacon key — re-driving its \
                             finalization fetch"
                        );
                        mailbox.hint_finalized(boundary, targets).await;
                    }
                }) as BoxFuture<'static, ()>
            })
        };
        let handle = self.context.with_label("repair_sweep").spawn({
            let provider = self.cfg.scheme_pins.clone();
            let store = self.cfg.group_keys.clone();
            let held = self.cfg.held_keys.clone();
            let pull = self.cfg.pull_keys.clone();
            let namespace = fluent_namespace(self.cfg.chain_id);
            move |_| run_repair_sweep(wake_rx, provider, store, held, pull, namespace, hint)
        });
        (wake_tx, handle)
    }

    /// Re-run [`Self::reconcile_roles`] for the CURRENT live epoch (the cached most
    /// recent boundary delivery). The non-boundary edges (share / spawn_unblocked /
    /// vote_backup) carry no fresh snapshot, so they reconcile this.
    ///
    /// Only reconciles the cached epoch while it is STILL the live frontier. Once
    /// corroboration advanced the frontier past it, the cached epoch is
    /// below-frontier — already aborted/soft-entered by `abort_below` on the
    /// boundary that passed it, and carrying NO signer obligation. Re-running
    /// `reconcile_roles` on it would soft-enter a verify-only scheme over an epoch
    /// registered as `Signer` → `EpochSchemeProvider` downgrade-refusal churn +
    /// `roles`↔provider divergence. The next BOUNDARY delivery refreshes
    /// `latest_live` to the new frontier and reconciles it there (with its snapshot).
    async fn reconcile_live<HS, HR>(&mut self, muxes: Option<&Muxes<HS, HR>>)
    where
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
    {
        if let Some((epoch, snap)) = self.latest_live.clone() {
            if !self.is_live_epoch(epoch) {
                return;
            }
            self.reconcile_roles(epoch, snap, muxes).await;
        }
    }

    /// The `Inline::genesis(E)` precondition lookup on the E-1 terminal (boundary)
    /// block — the exact lookup `Inline::genesis` itself performs, so the guard is
    /// precise, not heuristic — which ALSO yields the seedless arm's base for epoch
    /// E. Gate and base come from one `get_block` so it is impossible to spawn on a
    /// block the base was not taken from, and the block is fetched exactly as often
    /// as when this only returned a bool.
    async fn boundary_lookup(&mut self, epoch: Epoch) -> BoundaryLookup {
        let Some(prev) = epoch.get().checked_sub(1).map(Epoch::new) else {
            return BoundaryLookup::NotApplicable; // epoch 0 — genesis needs no predecessor block
        };
        let Some(last) = self.cfg.epocher.last(prev) else {
            return BoundaryLookup::NotApplicable;
        };
        let Some(block) = self.cfg.marshal_mailbox.get_block(last).await else {
            return BoundaryLookup::Missing;
        };
        BoundaryLookup::Present {
            seed: block.parent_seed.as_ref().map(witness_fallback_seed),
        }
    }

    /// Abort engines of all epochs strictly below `current` (exit-at-transition;
    /// see the lifecycle note above the actor). `e < cutoff` only, so a
    /// stale/replayed boundary for an OLD epoch can never abort a newer engine.
    ///
    /// Also PRUNES `deferred_spawns` of every parked epoch `< cutoff` — using the
    /// SAME cutoff as the engine abort. A frontier that advances E-1 → E+1 via a
    /// catch-up span (no boundary delivery for exactly epoch E) leaves
    /// `deferred_spawns[E]` orphaned: E is now below-frontier, will only ever
    /// soft-enter, and its `Inline::genesis(E)` precondition is moot. If it were
    /// left in the set, the `spawn_unblocked` edge would fire `reconcile_live`
    /// (a no-op for the stale E) on EVERY finalized block for the process
    /// lifetime. Pruning here makes an EMPTY set the true "no pending promotion"
    /// signal that the `spawn_unblocked` edge gates on.
    async fn abort_below(&mut self, current: Epoch) {
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
                self.roles.insert(e, Role::Verifier);
                info!(?e, "epoch exited (transition)");
            }
        }
        let context = self.context.as_present().clone();
        prune_agreements(&context, &mut self.dkg_agreements, cutoff).await;
        self.deferred_spawns.retain(|e| e.get() >= cutoff);
        // Prune `roles` to the trailing scheme-retention window: the sole reader is
        // `soft_enter`'s `roles.get(&epoch)` for the epoch under reconcile
        // (current / near-frontier == cutoff), so entries older than
        // `cutoff − SCHEME_RETENTION_EPOCHS` are never read again — keep current +
        // the trailing window, drop the rest (unbounded across a months-long
        // process otherwise).
        let roles_floor = cutoff.saturating_sub(SCHEME_RETENTION_EPOCHS as u64);
        self.roles.retain(|e, _| e.get() >= roles_floor);
    }

    /// Build + start the per-epoch Simplex engine and register its 3 sub-channels
    /// against the plane-owned Muxers. Returns `false` (spawning nothing) on an
    /// invalid committee snapshot or a muxer-register failure — the caller leaves
    /// the epoch un-promoted to retry on the next edge, rather than panicking.
    async fn spawn_engine<HS, HR>(
        &mut self,
        epoch: Epoch,
        snap: ValidatorSetSnapshot,
        beacon: Option<BeaconKey>,
        fallback_seed: [u8; 32],
        muxes: Option<&Muxes<HS, HR>>,
    ) -> bool
    where
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
    {
        // W1 ordering tripwire (P1-b): by the time the engine object can come
        // into existence, this beacon-active signer's `PK_epoch` MUST already
        // be in the shared group-key map — the insert happens-before the
        // engine, hence before any vote. A future refactor that moves the W1
        // insert after (or inside) the spawn trips this on every deterministic
        // run.
        debug_assert!(
            beacon.is_none() || self.cfg.group_keys.cached_only(epoch.get()).is_some(),
            "W1 ordering violated: PK_epoch not in group_keys before spawn_engine({epoch:?})"
        );
        // `None` ⇒ a FOLLOWER manager (no plane). A follower's `signer_keypair`
        // is `None`, so `is_member` in `reconcile_roles` is always false → the
        // `Role::Signer` arm that reaches here is never taken. Defend it as a
        // compile-time fact rather than fabricating an idle plane.
        let Some(muxes) = muxes else {
            unreachable!("follower (Option<Muxes>::None) never spawns an engine: is_member==false")
        };
        let (vote_mux, cert_mux, res_mux) = (&muxes.vote, &muxes.cert, &muxes.res);
        let engine_ctx = self.context.with_label("simplex");
        let engine = match EpochEngine::new(
            engine_ctx,
            EpochEngineConfig {
                blocker: self.cfg.blocker.clone(),
                snapshot: snap,
                epoch,
                fallback_seed,
                epocher: self.cfg.epocher.clone(),
                chain_id: self.cfg.chain_id,
                signer_keypair: self.cfg.signer_keypair.clone(),
                app: self.cfg.app.clone(),
                timeouts: self.cfg.timeouts,
                mailbox_size: self.cfg.mailbox_size,
                register_scheme: self.cfg.register_scheme.clone(),
                beacon,
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
                warn!(?epoch, %e, "skipping epoch spawn — invalid committee snapshot");
                return false;
            }
        };
        let vote_sub = match vote_mux.lock().await.register(epoch.get()).await {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    ?epoch,
                    ?e,
                    "skipping epoch spawn — vote muxer register failed"
                );
                return false;
            }
        };
        let cert_sub = match cert_mux.lock().await.register(epoch.get()).await {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    ?epoch,
                    ?e,
                    "skipping epoch spawn — cert muxer register failed"
                );
                return false;
            }
        };
        let res_sub = match res_mux.lock().await.register(epoch.get()).await {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    ?epoch,
                    ?e,
                    "skipping epoch spawn — res muxer register failed"
                );
                return false;
            }
        };
        let handle = engine.start(vote_sub, cert_sub, res_sub);
        self.active_epochs.insert(epoch, handle);
        info!(?epoch, "epoch entered (signer)");
        true
    }
}

/// What W1 does with the key it resolved for an epoch it is about to sign.
///
/// A free fn so the suppression rule is testable without an `Actor`, and so the
/// three outcomes are named rather than implied by a nested `if`.
// A `GroupPublic` (G2) is ~288 B; this is a transient return value matched
// immediately by its one caller and never stored, so the stack copy is cheaper
// than the heap allocation boxing would add — the identical trade `BoundaryOutcome`
// makes (`beacon::keys`).
#[allow(clippy::large_enum_variant)]
#[derive(Debug, PartialEq, Eq)]
enum OwnKeyPublication {
    /// No quorum-agreed key is recorded for the epoch: publish, exactly as W1
    /// always has. This is every stable (carry-forward) epoch, where no agreement
    /// instance runs at all — and it is why the suppression is conditional: an
    /// unconditional one would stop answering ladder rung 1 for those, and
    /// `repair_unpinned_schemes` would leave them unpinned and vote-only.
    Publish,
    /// A quorum already published this exact key. The store entry answers rung 1
    /// at a strictly higher provenance, so W1 has nothing to add.
    DeferAgreeing,
    /// A quorum published a DIFFERENT key for this epoch. The agreed one holds
    /// (the tiering in `BeaconKeys::insert` would have kept it anyway); the
    /// divergence is the thing worth saying out loud.
    DeferDiverging(GroupPublic),
}

fn own_key_publication(agreed: Option<GroupPublic>, resolved: GroupPublic) -> OwnKeyPublication {
    match agreed {
        None => OwnKeyPublication::Publish,
        Some(agreed) if agreed == resolved => OwnKeyPublication::DeferAgreeing,
        Some(agreed) => OwnKeyPublication::DeferDiverging(agreed),
    }
}

/// Resolve `PK_epoch` through the ONE ladder ([`resolve`]) and register the
/// verify-only scheme for `epoch`. Returns whether the registered scheme carries
/// a pin — which is how the beacon-key repair edge tells "this fill upgraded a
/// pin-less epoch" from "already pinned, nothing to re-drive".
///
/// The ladder's ORDER is documented on [`BeaconKeys::get_pk`] — do NOT re-derive
/// it here.
///
/// A free fn over the pieces so the pin-less-then-pinned upgrade is testable
/// without standing up the full generic `Actor` (which needs a live marshal, a
/// slasher and a spec-exec mailbox). The caller keeps the role bookkeeping and
/// the already-a-Signer guard, which are `Actor` state.
async fn register_soft_entered(
    store: &BeaconKeys,
    own: Option<&OwnKeyFor>,
    held: Option<&AgreedKeys>,
    epoch: Epoch,
    snap: &ValidatorSetSnapshot,
    chain_id: u64,
    register: &(dyn Fn(Epoch, BlsScheme) + Send + Sync),
) -> bool {
    let cert_seed_pin = store
        .get_pk(
            epoch.get(),
            KeySources {
                own,
                held,
                ..Default::default()
            },
        )
        .await;
    debug!(
        ?epoch,
        pinned = cert_seed_pin.is_some(),
        "soft-enter: registering verify-only scheme"
    );
    match soft_enter_verifier(snap, chain_id, cert_seed_pin) {
        Some(scheme) => {
            register(epoch, scheme);
            cert_seed_pin.is_some()
        }
        None => false,
    }
}

/// What the sweep does with the epochs it upgraded: re-drive each one's
/// finalization fetch. A callback rather than the mailbox itself so the sweep's
/// own loop is testable without a marshal.
type RepairHint = Arc<dyn Fn(Vec<Epoch>) -> BoxFuture<'static, ()> + Send + Sync>;

/// The repair sweep's driver: one sweep per wake, never two at once, on a task
/// of its own. Exits when the last wake sender drops (the epoch manager left).
///
/// Wakes COALESCE rather than queue — several arriving during a sweep produce one
/// more sweep, not one each — which is the property that keeps a sweep whose
/// ladder is spending network pulls from stacking concurrent pulls for the same
/// epoch behind the pull throttle.
///
/// The frontier it sweeps under is the one from the wake it consumed, so an
/// in-flight sweep can be running under a frontier the manager has already
/// advanced past. That is safe in ONE direction only, and it is this one: a stale
/// frontier is a LOWER frontier, and the frontier is an upper exclusion bound, so
/// staleness can only make the sweep skip an epoch it could have repaired — never
/// pin under a live epoch's engine. The next wake repairs the skipped one.
async fn run_repair_sweep(
    mut wake: watch::Receiver<(Epoch, Epoch)>,
    provider: EpochSchemeProvider,
    store: BeaconKeys,
    held: Option<AgreedKeys>,
    pull: Option<AgreedKeys>,
    namespace: Vec<u8>,
    hint: RepairHint,
) {
    loop {
        let (observed, entered) = *wake.borrow_and_update();
        let upgraded = repair_unpinned_schemes(
            &provider,
            &store,
            held.as_ref(),
            pull.as_ref(),
            &namespace,
            observed,
            entered,
        )
        .await;
        if !upgraded.is_empty() {
            hint(upgraded).await;
        }
        if wake.changed().await.is_err() {
            debug!("epoch manager gone — repair sweep stopping");
            return;
        }
    }
}

/// Resolve and attach the cert-seed pin for every registered epoch that is still
/// unpinned and below the live frontier. Returns the epochs this call actually
/// upgraded, newest last.
///
/// The work list comes from the scheme provider rather than from any set the
/// epoch manager keeps. That is the whole point: a shadow set only knows about
/// the epochs whose registration also wrote it, so it misses the bulk catch-up
/// span and `cold_start_register`, and it goes stale the moment an epoch is
/// pinned by a path that does not update it.
///
/// **Epochs at or above the frontier are excluded, and not merely as a
/// nicety.** A live epoch's scheme belongs to its engine. Pinning the verifier
/// entry underneath a signer that is about to register would make
/// [`EpochSchemeProvider::register`] refuse that signer registration whenever the
/// signer's own scheme is unpinned — which is exactly the share-less signer case
/// — and the node would silently fail to enter as a signer.
///
/// **The frontier is the higher of the two epochs this node has evidence for,
/// and it has to be both.** `observed` is the f+1-corroborated live epoch, which
/// is the tighter bound whenever it is available — but it is fed ONLY by the
/// vote-backup arm, and a follower PARKS that arm, so on a follower it is
/// `Epoch(0)` for the life of the process and a sweep scoped to it discards every
/// registered epoch. `entered` is the highest epoch whose scheme this node has
/// registered, advanced on every boundary delivery and every catch-up span, so it
/// is evidence a follower does have. Taking the max never widens the exclusion
/// zone below what a signer needs: every path that registers a scheme for `E`
/// advances `entered` to `E` first, so a live epoch is at or above the frontier
/// under either input.
///
/// **A pin is write-once and a wrong one is terminal for that epoch on this
/// node**, so the ladder is run with a PROVENANCE FLOOR and not just with `own`
/// dropped. `apply_pin` refuses an already-pinned entry and nothing re-registers
/// a below-frontier epoch's scheme, so a locally reconstructed key that diverged
/// from the network's (soak 2026-07-14) makes the marshal reject every valid
/// certificate of the epoch for the life of the process — a later `Agreed` write
/// corrects the STORE and leaves the pin wrong. The promote path can afford the
/// own-DKG rung because it has a value-gate that demotes on a mismatch; this path
/// has none and should not grow one.
///
/// Dropping `own` alone does NOT deliver that, which is what the floor fixes:
/// rung 1 is the shared store, W1/W3 write locally reconstructed keys into it
/// under [`KeySource::LocalDkg`], and a provenance-blind rung 1 returns before
/// `held`/`pull` are ever consulted. `store_floor: Carried` refuses exactly that
/// tier and nothing else. Rung 1 stays load-bearing: the agreement write-back
/// publishes at `KeySource::Agreed` and both later rungs memoise their answers at
/// `Agreed`/`Carried`, so every epoch whose key this node legitimately knows is
/// still a map hit rather than a re-walk or a re-fetch.
async fn repair_unpinned_schemes(
    provider: &EpochSchemeProvider,
    store: &BeaconKeys,
    held: Option<&AgreedKeys>,
    pull: Option<&AgreedKeys>,
    namespace: &[u8],
    observed: Epoch,
    entered: Epoch,
) -> Vec<Epoch> {
    let frontier = observed.max(entered);
    let mut upgraded = Vec::new();
    for epoch in provider.unpinned_epochs() {
        if epoch >= frontier {
            continue;
        }
        let Some(pk) = store
            .get_pk(
                epoch.get(),
                KeySources {
                    held,
                    pull,
                    // `own` absent AND the store floored at the weakest tier
                    // nothing local can write — see this function's docs. Either
                    // half alone leaves the locally-derived key reachable.
                    store_floor: Some(KeySource::Carried),
                    ..Default::default()
                },
            )
            .await
        else {
            continue;
        };
        if provider.apply_pin(epoch, pk, namespace) {
            upgraded.push(epoch);
        }
    }
    upgraded
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
    no_progress: &mut Option<(Epoch, Epoch)>,
) {
    if their_epoch <= *highest_entered_epoch {
        return;
    }
    // Suppress an identical re-attempt (bug 15): if the last run at this exact
    // (entered, observed) state registered nothing new, a backup-vote storm must
    // not re-run the EVM span fan-out + marshal hint while nothing changed. The
    // memo is cleared on the share / spawn_unblocked edges (a share landing or
    // execution progressing can change the outcome) and a frontier move changes
    // `state` itself — so the first vote corroborating a NEW frontier still runs.
    let state = (*highest_entered_epoch, highest_observed_epoch);
    if *no_progress == Some(state) {
        return;
    }
    let prior_entered = *highest_entered_epoch;
    let entered = highest_entered_epoch.get();
    let span_from = Epoch::new(entered + 1);
    let span_top = highest_observed_epoch.min(Epoch::new(entered + CATCHUP_SPAN_CAP));
    let registered_to = if span_top >= span_from {
        soft_enter_span(span_from, span_top).await
    } else {
        *highest_entered_epoch
    };
    // Memoize a no-progress attempt (nothing new registered) so identical
    // re-votes early-out above; a real advance clears the memo (progress moves
    // `state`, and this stores `None`).
    *no_progress = (registered_to == prior_entered).then_some(state);
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

/// True when a per-epoch engine `Handle` has COMPLETED (its task finished — normal
/// exit or a panic caught by the runtime's `catch_panics(true)`, which completes
/// the handle without propagating to the manager). Commonware `Handle` exposes only
/// `abort()` + `impl Future` (no `is_finished`) and is `Unpin`, so we poll it once
/// with a no-op waker: `Ready` ⇒ dead, `Pending` ⇒ still running. Extracted as a
/// free fn (mirroring `corroborate_frontier` / `pipeline_catchup_span`) so the
/// alive/completed/aborted branches are unit-testable on real spawned handles.
fn engine_handle_dead(handle: &mut Handle<()>) -> bool {
    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    matches!(Pin::new(handle).poll(&mut cx), Poll::Ready(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        beacon::actor::DETERMINISTIC_BOOTSTRAP_EPOCH, outer::EpochSchemeProvider,
        scheme::epoch_committee_from_snapshot,
    };
    use alloy_primitives::{Address, B256};
    use commonware_codec::DecodeExt;
    use commonware_cryptography::{
        bls12381::{dkg::deal_anonymous, primitives::variant::MinSig},
        certificate::Provider as _,
        ed25519::PrivateKey as Ed25519PrivateKey,
        Signer,
    };
    use commonware_math::algebra::Random as _;
    use commonware_utils::{test_rng, N3f1, NZU32};
    use fluentbase_bls::scheme::build_signer;
    use fluentbase_bls::BlsPubkey;
    use fluentbase_staking_reader::reader::{ConsensusKeys, ValidatorWithKeys};
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

    /// The repair this phase exists for: an epoch soft-entered while `PK_E` was
    /// unresolvable registers a PIN-LESS verifier, and a key landing afterwards
    /// must upgrade it. Both halves have to hold — `soft_enter` has to re-resolve
    /// off the shared store rather than remember its earlier miss, AND
    /// `EpochSchemeProvider::register` has to accept the replacement (it does: same
    /// committee, verifier→verifier, and the monotonicity guard blocks only the
    /// opposite direction).
    #[tokio::test]
    async fn a_pin_less_soft_enter_is_upgraded_by_a_key_that_arrives_later() {
        let epoch = Epoch::new(9);
        let snap = ValidatorSetSnapshot {
            block_hash: B256::repeat_byte(0x11),
            block_number: 42,
            epoch: epoch.get(),
            validators: (0..4u8)
                .map(|i| {
                    let mut rng = StdRng::seed_from_u64(i as u64);
                    ValidatorWithKeys {
                        address: Address::repeat_byte(i),
                        keys: ConsensusKeys {
                            bls_pubkey: BlsPubkey::decode(
                                ValidatorBlsKeypair::generate(&mut rng)
                                    .public_bytes()
                                    .as_slice(),
                            )
                            .unwrap(),
                            peer_pubkey: Ed25519PrivateKey::random(&mut rng).public_key(),
                            activation_epoch: 1,
                        },
                        tombstoned: false,
                    }
                })
                .collect(),
            weights: None,
        };
        let store = BeaconKeys::new();
        let provider = EpochSchemeProvider::new();
        let register = {
            let provider = provider.clone();
            move |e: Epoch, scheme: BlsScheme| provider.register(e, scheme)
        };
        let pinned_now = || provider.scoped(epoch).expect("registered").is_seed_pinned();

        // Nothing resolvable anywhere: registered, vote-only.
        assert!(
            !register_soft_entered(&store, None, None, epoch, &snap, 1, &register).await,
            "an unresolvable key must register pin-less, not skip the epoch"
        );
        assert!(!pinned_now());

        // The key lands — via the agreement write-back or a DKG share — and the
        // SAME call path now pins. Nothing re-ran an artifact read; the store rung
        // answered.
        let (sharing, _) =
            deal_anonymous::<MinSig, N3f1>(&mut test_rng(), Default::default(), NZU32!(4));
        store.set_pk(epoch.get(), *sharing.public(), KeySource::Agreed);
        assert!(
            register_soft_entered(&store, None, None, epoch, &snap, 1, &register).await,
            "the late key must be picked up by the next soft-enter"
        );
        assert!(
            pinned_now(),
            "and the provider must accept the pin-less → pinned replacement"
        );
    }

    /// Committee fixture for the repair-sweep tests. Returns the snapshot and the
    /// BLS keypairs behind it, because one test has to build a SIGNER scheme and
    /// that needs a keypair the committee actually contains.
    fn repair_fixture(epoch: Epoch) -> (ValidatorSetSnapshot, Vec<ValidatorBlsKeypair>) {
        let mut keypairs = Vec::new();
        let validators = (0..4u8)
            .map(|i| {
                let mut rng = StdRng::seed_from_u64(0x5EED + i as u64);
                let kp = ValidatorBlsKeypair::generate(&mut rng);
                let bls_pubkey = BlsPubkey::decode(kp.public_bytes().as_slice()).unwrap();
                keypairs.push(kp);
                ValidatorWithKeys {
                    address: Address::repeat_byte(i),
                    keys: ConsensusKeys {
                        bls_pubkey,
                        peer_pubkey: Ed25519PrivateKey::random(&mut rng).public_key(),
                        activation_epoch: 1,
                    },
                    tombstoned: false,
                }
            })
            .collect();
        (
            ValidatorSetSnapshot {
                block_hash: B256::repeat_byte(0x22),
                block_number: 7,
                epoch: epoch.get(),
                validators,
                weights: None,
            },
            keypairs,
        )
    }

    fn some_group_key() -> GroupPublic {
        let (sharing, _) =
            deal_anonymous::<MinSig, N3f1>(&mut test_rng(), Default::default(), NZU32!(4));
        *sharing.public()
    }

    /// Register `epoch` pin-less the way the bulk catch-up span does — straight
    /// into the provider, touching neither `roles` nor any epoch-manager state.
    /// A group key distinct from [`some_group_key`].
    fn some_other_group_key() -> GroupPublic {
        let mut rng = StdRng::seed_from_u64(0xC0FFEE);
        let (sharing, _) = commonware_cryptography::bls12381::dkg::deal_anonymous::<
            commonware_cryptography::bls12381::primitives::variant::MinSig,
            commonware_utils::N3f1,
        >(&mut rng, Default::default(), commonware_utils::NZU32!(4));
        *sharing.public()
    }

    fn register_pin_less(provider: &EpochSchemeProvider, epoch: Epoch) {
        let (snap, _) = repair_fixture(epoch);
        let scheme = soft_enter_verifier(&snap, 1, None).expect("valid committee");
        provider.register(epoch, scheme);
    }

    fn is_pinned(provider: &EpochSchemeProvider, epoch: Epoch) -> bool {
        provider.scoped(epoch).expect("registered").is_seed_pinned()
    }

    /// The sweep runs on EVERY boundary delivery, so the steady state — nothing
    /// unpinned — has to cost nothing. If it did not, the boundary edge would be
    /// the wrong place to drive it and the key wake would be the only affordable
    /// trigger, which is the arrangement this phase exists to replace (that wake
    /// never fires at all for a node with no DKG material on a stable chain).
    ///
    /// Reds if the sweep ever resolves for an epoch it cannot pin: the walk here
    /// panics if consulted, and an all-pinned provider must never consult it.
    #[tokio::test]
    async fn a_sweep_with_nothing_to_repair_is_a_no_op() {
        let epoch = Epoch::new(5);
        let (snap, _) = repair_fixture(epoch);
        let provider = EpochSchemeProvider::new();
        provider.register(
            epoch,
            soft_enter_verifier(&snap, 1, Some(some_group_key())).expect("valid committee"),
        );

        let exploding = AgreedKeys::new(
            Arc::new(|_| {
                Box::pin(async { panic!("an already-pinned epoch must not be resolved for") })
            }),
            Arc::new(|_| Some(true)),
        );

        let upgraded = repair_unpinned_schemes(
            &provider,
            &BeaconKeys::new(),
            Some(&exploding),
            None,
            &fluent_namespace(1),
            Epoch::new(7),
            Epoch::new(0),
        )
        .await;

        assert!(upgraded.is_empty());
    }

    /// The scope half of the gap this phase closes. The edge this replaces looked
    /// at ONE cached epoch — the most recently boundary-delivered — so an epoch
    /// that missed its own boundary window was never retried again. Here the key
    /// for the OLDER of two unpinned epochs is the one that resolves; a sweep
    /// scoped to the newest would report nothing.
    ///
    /// Reds if `unpinned_epochs()` is narrowed back to a single entry.
    #[tokio::test]
    async fn the_sweep_repairs_every_unpinned_epoch_not_just_the_newest() {
        let (older, newer) = (Epoch::new(5), Epoch::new(6));
        let provider = EpochSchemeProvider::new();
        register_pin_less(&provider, older);
        register_pin_less(&provider, newer);

        let store = BeaconKeys::new();
        store.set_pk(older.get(), some_group_key(), KeySource::Agreed);

        let upgraded = repair_unpinned_schemes(
            &provider,
            &store,
            None,
            None,
            &fluent_namespace(1),
            Epoch::new(7),
            Epoch::new(0),
        )
        .await;

        assert_eq!(upgraded, vec![older]);
        assert!(is_pinned(&provider, older));
        assert!(
            !is_pinned(&provider, newer),
            "an epoch whose key is still unresolvable must stay unpinned, not \
             inherit its neighbour's"
        );
    }

    /// `verify_certificate` reads pin presence as "this epoch is beacon-active"
    /// and rejects every SEEDLESS certificate under it. Pre-bootstrap epochs are
    /// legitimately seedless, so a pin there rejects the whole epoch. Before the
    /// sweep existed this held only because no pin source reached that far back.
    ///
    /// Reds if the `DETERMINISTIC_BOOTSTRAP_EPOCH` guard leaves `apply_pin`.
    #[tokio::test]
    async fn the_sweep_never_pins_below_the_bootstrap_epoch() {
        let pre_beacon = Epoch::new(DETERMINISTIC_BOOTSTRAP_EPOCH - 1);
        let provider = EpochSchemeProvider::new();
        register_pin_less(&provider, pre_beacon);

        let store = BeaconKeys::new();
        store.set_pk(pre_beacon.get(), some_group_key(), KeySource::Agreed);

        let upgraded = repair_unpinned_schemes(
            &provider,
            &store,
            None,
            None,
            &fluent_namespace(1),
            Epoch::new(7),
            Epoch::new(0),
        )
        .await;

        assert!(upgraded.is_empty());
        assert!(!is_pinned(&provider, pre_beacon));
    }

    /// A signer's scheme is built by its own engine from its own beacon material.
    /// Pinning underneath it makes this sweep a second writer of that field, and
    /// — because `register` refuses a replacement that drops a pin — it would
    /// then block the engine's own registration whenever the signer is
    /// share-less and therefore builds an unpinned scheme.
    ///
    /// Reds if the `me().is_none()` filter leaves `unpinned_epochs()`.
    #[tokio::test]
    async fn the_sweep_never_touches_a_signer_scheme() {
        let epoch = Epoch::new(5);
        let (snap, keypairs) = repair_fixture(epoch);
        let committee = epoch_committee_from_snapshot(&snap).expect("valid committee");
        let signer = build_signer(&fluent_namespace(1), committee.bimap, &keypairs[0], None)
            .expect("keypair is a committee member");
        let provider = EpochSchemeProvider::new();
        provider.register(epoch, signer);

        let store = BeaconKeys::new();
        store.set_pk(epoch.get(), some_group_key(), KeySource::Agreed);

        let upgraded = repair_unpinned_schemes(
            &provider,
            &store,
            None,
            None,
            &fluent_namespace(1),
            Epoch::new(7),
            Epoch::new(0),
        )
        .await;

        assert!(upgraded.is_empty());
        assert!(!is_pinned(&provider, epoch));
    }

    /// The coverage the provider-sourced work list buys over an epoch-manager
    /// shadow set: the bulk catch-up span registers straight into the provider
    /// and writes no manager state at all, so a shadow-sourced sweep is blind to
    /// every epoch it registered — which is the whole of a deep catch-up.
    ///
    /// Reds if the work list is re-sourced from `roles` (nothing put this epoch
    /// there).
    #[tokio::test]
    async fn a_span_registered_epoch_is_repairable() {
        let epoch = Epoch::new(5);
        let provider = EpochSchemeProvider::new();
        register_pin_less(&provider, epoch);

        let store = BeaconKeys::new();
        store.set_pk(epoch.get(), some_group_key(), KeySource::Agreed);

        let upgraded = repair_unpinned_schemes(
            &provider,
            &store,
            None,
            None,
            &fluent_namespace(1),
            Epoch::new(7),
            Epoch::new(0),
        )
        .await;

        assert_eq!(upgraded, vec![epoch]);
    }

    /// The follower half of the frontier. A follower PARKS the vote-backup arm,
    /// so nothing ever corroborates `highest_observed_epoch` and it stays at
    /// `Epoch(0)` for the life of the process — under which every registered
    /// epoch is at or above the frontier and the sweep discards all of them. The
    /// epoch a boundary delivery entered is the evidence a follower does have.
    ///
    /// Reds if the sweep is re-scoped to `observed` alone.
    #[tokio::test]
    async fn the_sweep_reaches_below_an_entered_epoch_with_no_corroborated_frontier() {
        let epoch = Epoch::new(5);
        let provider = EpochSchemeProvider::new();
        register_pin_less(&provider, epoch);

        let store = BeaconKeys::new();
        store.set_pk(epoch.get(), some_group_key(), KeySource::Agreed);

        let namespace = fluent_namespace(1);
        let never_corroborated = Epoch::new(0);
        assert!(
            repair_unpinned_schemes(
                &provider,
                &store,
                None,
                None,
                &namespace,
                never_corroborated,
                Epoch::new(0),
            )
            .await
            .is_empty(),
            "with no frontier evidence at all the sweep must stay inert — the \
             pre-fix follower state, kept here so the assertion below is not \
             vacuous"
        );

        // The boundary for epoch 6 lands and `reconcile_roles` enters it.
        assert_eq!(
            repair_unpinned_schemes(
                &provider,
                &store,
                None,
                None,
                &namespace,
                never_corroborated,
                Epoch::new(6),
            )
            .await,
            vec![epoch]
        );
        assert!(is_pinned(&provider, epoch));
    }

    /// The caller re-drives a marshal fetch per returned epoch, so the return
    /// value has to be the unpinned→pinned TRANSITION and not "every epoch I
    /// looked at". Otherwise every trigger re-hints for as long as anything is
    /// unpinned, and on a catch-up node that is every boundary.
    ///
    /// Reds if `apply_pin` stops refusing an already-pinned entry.
    #[tokio::test]
    async fn a_repaired_epoch_re_drives_its_finalization_hint_exactly_once() {
        let epoch = Epoch::new(5);
        let provider = EpochSchemeProvider::new();
        register_pin_less(&provider, epoch);

        let store = BeaconKeys::new();
        store.set_pk(epoch.get(), some_group_key(), KeySource::Agreed);

        let namespace = fluent_namespace(1);
        let sweep = || {
            repair_unpinned_schemes(
                &provider,
                &store,
                None,
                None,
                &namespace,
                Epoch::new(7),
                Epoch::new(0),
            )
        };
        assert_eq!(sweep().await, vec![epoch]);
        assert_eq!(sweep().await, Vec::<Epoch>::new());
        assert!(is_pinned(&provider, epoch));
    }

    /// The sweep's pin is WRITE-ONCE (`apply_pin` refuses an already-pinned
    /// entry) and nothing re-registers a below-frontier scheme, so a pin taken
    /// from a key this node reconstructed itself is terminal for that epoch on
    /// this node if the reconstruction diverged (soak 2026-07-14) — every valid
    /// certificate of the epoch rejects, and the later `Agreed` write that
    /// corrects the STORE leaves the pin wrong. Dropping the own-DKG rung does
    /// NOT prevent that: W1/W3 write the same locally derived key into the shared
    /// store, and the store rung answers before `held`/`pull` are consulted.
    ///
    /// The floor is a floor and not an `Agreed`-only gate, because that would
    /// hollow out rung 1: `Carried` is derived from chain facts alone (an
    /// attested mint plus the chain's `dkgQual` bit) and is how the ladder
    /// memoises a resolved carry.
    ///
    /// Reds if the store rung goes back to being provenance-blind.
    #[tokio::test]
    async fn the_sweep_refuses_a_locally_derived_store_key() {
        let (local, carried) = (Epoch::new(5), Epoch::new(6));
        let provider = EpochSchemeProvider::new();
        register_pin_less(&provider, local);
        register_pin_less(&provider, carried);

        let store = BeaconKeys::new();
        store.set_pk(local.get(), some_group_key(), KeySource::LocalDkg);
        store.set_pk(carried.get(), some_other_group_key(), KeySource::Carried);

        let upgraded = repair_unpinned_schemes(
            &provider,
            &store,
            None,
            None,
            &fluent_namespace(1),
            Epoch::new(9),
            Epoch::new(0),
        )
        .await;

        assert_eq!(upgraded, vec![carried]);
        assert!(
            !is_pinned(&provider, local),
            "a key this node reconstructed itself must not become a write-once pin"
        );
        assert!(is_pinned(&provider, carried));
    }

    /// The sweep spends one bounded peer pull per unpinned epoch, and that pull
    /// SLEEPS on its caller-side rate bound before it even issues the fetch — up
    /// to `SCHEME_RETENTION_EPOCHS` × (`PULL_MIN_INTERVAL` + `PULL_TIMEOUT`) of
    /// awaiting per sweep. Awaited on the epoch manager's `select!`, that is a
    /// ~100 s window in which `share_n`, `spawn_unblocked` and `vote_backup` do
    /// not run — the three arms that turn this node into a signer.
    ///
    /// So the driver here is the manager's loop in miniature: a second arm with
    /// work queued on it, and the sweep wake sent from that arm exactly as the
    /// manager sends it. The whole window under test is the one in which the
    /// sweep is parked inside its pull, and the assertion is that the second arm
    /// is served throughout it. Under the pre-fix inline shape that count is
    /// zero, and `parked.notified()` below is never even reached.
    ///
    /// The timeout is there so a regression FAILS instead of hanging the suite.
    /// It costs nothing on the passing path — the future is dropped, not awaited.
    #[tokio::test]
    async fn a_parked_sweep_does_not_hold_its_driver() {
        const OTHER_ARM_EVENTS: usize = 8;
        let epoch = Epoch::new(5);
        let provider = EpochSchemeProvider::new();
        register_pin_less(&provider, epoch);

        // Both directions are permit-storing, so neither side can miss the
        // other's signal by being late to await it.
        let parked = Arc::new(Notify::new());
        let gate = Arc::new(Notify::new());
        let pulls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let pk = some_group_key();
        let pull = AgreedKeys::new(
            Arc::new({
                let (parked, gate, pulls) = (parked.clone(), gate.clone(), pulls.clone());
                move |_| {
                    let (parked, gate, pulls) = (parked.clone(), gate.clone(), pulls.clone());
                    Box::pin(async move {
                        pulls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        parked.notify_one();
                        gate.notified().await;
                        Some(pk)
                    }) as BoxFuture<'static, Option<GroupPublic>>
                }
            }),
            Arc::new(|_| Some(true)),
        );

        let (hint_tx, mut hint_rx) = mpsc::unbounded_channel();
        let hint: RepairHint = Arc::new(move |upgraded| {
            let hint_tx = hint_tx.clone();
            Box::pin(async move {
                drop(hint_tx.send(upgraded));
            }) as BoxFuture<'static, ()>
        });

        let frontier = (Epoch::new(9), Epoch::new(0));
        let (wake, wake_rx) = watch::channel(frontier);
        let sweep = tokio::spawn(run_repair_sweep(
            wake_rx,
            provider.clone(),
            BeaconKeys::new(),
            None,
            Some(pull),
            fluent_namespace(1),
            hint,
        ));

        let served = tokio::time::timeout(std::time::Duration::from_secs(30), async move {
            let (work_tx, mut work_rx) = mpsc::unbounded_channel();
            for _ in 0..OTHER_ARM_EVENTS {
                work_tx.send(()).expect("receiver is alive");
            }
            parked.notified().await;

            let mut served = 0usize;
            while served < OTHER_ARM_EVENTS {
                tokio::select! {
                    Some(()) = work_rx.recv() => {
                        served += 1;
                        wake.send_replace(frontier);
                    }
                }
            }
            assert_eq!(
                pulls.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "wakes arriving during a sweep must coalesce into one more sweep, \
                 not stack concurrent pulls behind the pull throttle"
            );

            gate.notify_one();
            assert_eq!(
                hint_rx.recv().await,
                Some(vec![epoch]),
                "the released sweep still re-drives the epoch it pinned"
            );
            served
        })
        .await
        .expect("a sweep awaited on its driver never lets the driver reach these assertions");

        assert_eq!(served, OTHER_ARM_EVENTS);
        assert!(is_pinned(&provider, epoch));
        sweep.abort();
    }

    /// The seedless arm must inherit the E-1 terminal block's witness whenever the
    /// block carries one, and fall back to the predictable constant derivation ONLY
    /// where no witness can exist — a witness-less block or no predecessor epoch.
    /// `Missing` is not a base at all: that epoch defers its spawn.
    ///
    /// The fixture's committee is empty on purpose: what is under test is which arm
    /// is selected, not the constant derivation's own inputs (pinned in
    /// `weighted_vrf`).
    #[test]
    fn the_witness_base_is_inherited_whenever_the_boundary_block_carries_one() {
        let snap = ValidatorSetSnapshot {
            block_hash: B256::repeat_byte(0x11),
            block_number: 42,
            epoch: 9,
            validators: Vec::new(),
            weights: None,
        };
        let witness = [0xA7u8; 32];
        let constant = constant_fallback_seed(&snap);
        assert_ne!(witness, constant);

        assert_eq!(
            seedless_base(
                &BoundaryLookup::Present {
                    seed: Some(witness)
                },
                &snap
            ),
            Some(SeedlessBase::Witness(witness))
        );
        assert_eq!(
            seedless_base(&BoundaryLookup::Present { seed: None }, &snap),
            Some(SeedlessBase::Constant(constant))
        );
        assert_eq!(
            seedless_base(&BoundaryLookup::NotApplicable, &snap),
            Some(SeedlessBase::Constant(constant))
        );
        assert_eq!(seedless_base(&BoundaryLookup::Missing, &snap), None);
    }

    // A single peer (even naming u64::MAX) must NOT advance the live frontier —
    // the P2-11 permanent-soft-enter halt. n = 4 ⇒ f = 1 ⇒ threshold f+1 = 2.
    /// W1's suppression rule, and the reason it is CONDITIONAL.
    ///
    /// Where the agreement plane has published a key, a locally reconstructed one
    /// has nothing to add and W1 stands down. Where it has not — every stable
    /// carry-forward epoch, which runs no agreement instance at all — W1 must still
    /// publish, or ladder rung 1 stops answering for epochs this node signed and
    /// `repair_unpinned_schemes` (which passes `own: None` precisely BECAUSE W1
    /// fills the store) leaves them unpinned and vote-only.
    #[test]
    fn w1_defers_to_an_agreed_key_and_publishes_where_there_is_none() {
        let mine = some_group_key();
        let theirs = {
            let mut rng = StdRng::seed_from_u64(0xB0B);
            let (sharing, _) =
                commonware_cryptography::bls12381::dkg::deal_anonymous::<
                    commonware_cryptography::bls12381::primitives::variant::MinSig,
                    commonware_utils::N3f1,
                >(&mut rng, Default::default(), commonware_utils::NZU32!(4));
            *sharing.public()
        };
        assert_ne!(mine, theirs, "the fixture must produce two distinct keys");

        assert_eq!(own_key_publication(None, mine), OwnKeyPublication::Publish);
        assert_eq!(
            own_key_publication(Some(mine), mine),
            OwnKeyPublication::DeferAgreeing
        );
        assert_eq!(
            own_key_publication(Some(theirs), mine),
            OwnKeyPublication::DeferDiverging(theirs)
        );
    }

    /// The suppression is only sound because the artifact's entry answers the rung
    /// W1's did — and it is now ALSO the promote value-gate's comparand, which is
    /// what the shrink changed: the gate used to read a finalized boundary block's
    /// outcome, and no block carries one.
    #[tokio::test]
    async fn an_agreed_key_answers_rung_one_and_is_the_value_gate_comparand() {
        let store = BeaconKeys::new();
        let pk = some_group_key();
        store.set_pk(9, pk, KeySource::Agreed);
        assert_eq!(
            store.get_pk(9, KeySources::default()).await,
            Some(pk),
            "the artifact must answer the store rung W1 used to"
        );
        assert_eq!(
            store.attested(9),
            Some(pk),
            "and it is what the promote value-gate compares against"
        );
        // Tiering: a local reconstruction can never displace it, whatever the
        // write order — which is what makes W1 standing down a no-op for
        // readers rather than a loss.
        store.set_pk(9, some_other_group_key(), KeySource::LocalDkg);
        assert_eq!(store.cached_only(9), Some(pk));
        assert_eq!(store.attested(9), Some(pk));
    }

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
            &mut None,
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
        assert_eq!(
            entered,
            Epoch::new(3),
            "entered tip advanced to the frontier"
        );

        // A second identical vote at the now-entered frontier is a no-op (early-out).
        futures::executor::block_on(pipeline_catchup_span(
            &mut entered,
            Epoch::new(3),
            Epoch::new(3),
            &epocher,
            soft.as_ref(),
            mk_hint(hints.clone()),
            &mut None,
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
            &mut None,
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

    /// A span that registers NOTHING (unresolvable committee read) is memoized
    /// (bug 15): an identical re-vote must NOT re-run the fan-out; but clearing
    /// the memo (a share/spawn edge) OR a frontier advance must let it re-run.
    #[test]
    fn no_progress_span_is_memoized_and_cleared_by_edges() {
        use std::sync::Mutex as StdMutex;

        let epocher = OriginEpocher::new(0, 32u64.try_into().unwrap());
        let calls = std::sync::Arc::new(StdMutex::new(Vec::<(u64, u64)>::new()));
        // Span callback that registers nothing: returns `from-1` (== entered).
        let no_progress_span: Arc<dyn Fn(Epoch, Epoch) -> BoxFuture<'static, Epoch> + Send + Sync> = {
            let calls = calls.clone();
            Arc::new(move |from: Epoch, to: Epoch| {
                calls.lock().unwrap().push((from.get(), to.get()));
                Box::pin(async move { Epoch::new(from.get() - 1) }) as BoxFuture<'static, Epoch>
            })
        };
        let noop_hint = |_b: Height| Box::pin(async move {}) as BoxFuture<'static, ()>;

        let mut entered = Epoch::new(0);
        let mut memo = None;
        let run = |entered: &mut Epoch, memo: &mut Option<(Epoch, Epoch)>, observed: u64| {
            futures::executor::block_on(pipeline_catchup_span(
                entered,
                Epoch::new(observed),
                Epoch::new(observed),
                &epocher,
                no_progress_span.as_ref(),
                noop_hint,
                memo,
            ));
        };

        run(&mut entered, &mut memo, 3);
        assert_eq!(
            calls.lock().unwrap().len(),
            1,
            "first attempt runs the span"
        );
        // Identical re-vote: suppressed by the memo.
        run(&mut entered, &mut memo, 3);
        assert_eq!(
            calls.lock().unwrap().len(),
            1,
            "identical no-progress re-vote must NOT re-run the span"
        );
        // An edge clears the memo → the span runs again.
        memo = None;
        run(&mut entered, &mut memo, 3);
        assert_eq!(
            calls.lock().unwrap().len(),
            2,
            "clearing the memo re-enables the span"
        );
        // A frontier advance changes `state` → the span runs again without a clear.
        run(&mut entered, &mut memo, 4);
        assert_eq!(
            calls.lock().unwrap().len(),
            3,
            "a frontier advance re-enables the span"
        );
    }

    // `engine_handle_dead` must read PENDING (parked engine) as alive and both
    // COMPLETED (returned) and ABORTED handles as dead — the three branches the
    // respawn gate in `reconcile_roles` diffs on. Under `catch_panics(true)` a
    // panicked child engine surfaces as a COMPLETED handle, so this is the exact
    // signal that revives it.
    #[test]
    fn engine_handle_dead_distinguishes_pending_completed_aborted() {
        use commonware_runtime::{deterministic, Clock, Runner as _, Spawner};
        use std::time::Duration;

        let runner = deterministic::Runner::default();
        runner.start(|ctx| async move {
            let mut pending = ctx.clone().spawn(|c| async move {
                c.sleep(Duration::from_secs(3600)).await;
            });
            let mut done = ctx.clone().spawn(|_| async move {});
            let mut killed = ctx.clone().spawn(|c| async move {
                c.sleep(Duration::from_secs(3600)).await;
            });
            killed.abort();

            // Drive the deterministic scheduler until the completed + aborted
            // handles are observably dead (bounded so a regression fails instead of
            // hanging). Stop polling a handle once dead — a completed handle must
            // not be re-polled, which the production respawn path guarantees by
            // removing the entry.
            let mut done_dead = false;
            let mut killed_dead = false;
            for _ in 0..2_000 {
                if !done_dead {
                    done_dead = engine_handle_dead(&mut done);
                }
                if !killed_dead {
                    killed_dead = engine_handle_dead(&mut killed);
                }
                if done_dead && killed_dead {
                    break;
                }
                ctx.sleep(Duration::from_millis(1)).await;
            }
            assert!(done_dead, "a completed engine eventually reads dead");
            assert!(killed_dead, "an aborted engine eventually reads dead");
            assert!(
                !engine_handle_dead(&mut pending),
                "the still-parked child reads alive throughout"
            );
        });
    }

    /// The agreement instances live in their OWN map and are pruned on the same
    /// frontier cutoff as the engines — and pruning really aborts them, which for
    /// a still-running instance is the only thing that stops it.
    ///
    /// It also WAITS for them. The partition sweep that follows the abort pass
    /// cannot see these epochs any more — the abort pass removed them from the map
    /// the sweep's guard consults — so an instance still stopping while the sweep
    /// runs would have its partition removed and then recreated by its voter's
    /// last journal write, leaving a directory nothing ever reclaims.
    #[test]
    fn agreement_instances_prune_on_the_engine_cutoff_and_are_aborted() {
        use commonware_runtime::{deterministic, Clock, Runner as _, Spawner};
        use std::{
            sync::atomic::{AtomicUsize, Ordering},
            time::Duration,
        };

        /// Set on drop, which for an aborted task is the moment its future is
        /// dropped — the observable proof that `abort()` reached it.
        struct Tombstone(Arc<AtomicUsize>);
        impl Drop for Tombstone {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let runner = deterministic::Runner::timed(Duration::from_secs(10));
        runner.start(|ctx| async move {
            let dropped = Arc::new(AtomicUsize::new(0));
            let mut agreements: BTreeMap<Epoch, Handle<()>> = BTreeMap::new();
            for epoch in [3u64, 4, 5] {
                let mark = Tombstone(dropped.clone());
                agreements.insert(
                    Epoch::new(epoch),
                    ctx.with_label("agreement").spawn(move |_| async move {
                        let _mark = mark;
                        std::future::pending::<()>().await;
                    }),
                );
            }
            // Let the spawned tasks reach their park before anything is aborted.
            ctx.sleep(Duration::from_millis(1)).await;

            prune_agreements(&ctx, &mut agreements, 5).await;
            assert_eq!(
                agreements.keys().copied().collect::<Vec<_>>(),
                vec![Epoch::new(5)],
                "the frontier's own instance must survive its cutoff"
            );

            assert_eq!(
                dropped.load(Ordering::SeqCst),
                2,
                "prune returned before the instances it aborted had stopped, so the partition \
                 sweep it runs next races their last journal write"
            );
        });
    }

    /// An agreement supervisor removes its own journal partition after it
    /// delivers, but every EXTERNAL abort — this prune, the SafetyHalt unwind, the
    /// manager's exit — cancels it at an await and that removal never runs. Nothing
    /// else reclaims `dkg_epoch_{n}`, so the prune sweeps the band below the cutoff
    /// itself, by epoch number and not by the map, which also collects what a
    /// previous process left behind.
    #[test]
    fn pruning_reclaims_the_journal_partitions_an_abort_left_behind() {
        use commonware_runtime::{deterministic, Runner as _, Spawner as _, Storage as _};
        use std::time::Duration;

        let runner = deterministic::Runner::timed(Duration::from_secs(10));
        runner.start(|ctx| async move {
            // Epoch 3: aborted by an earlier prune (or a previous process) — the
            // leak. Epoch 5: the frontier's own live instance. Epoch 20: outside the
            // swept band, so a sweep that ignored the band would look identical.
            for epoch in [3u64, 5, 20] {
                ctx.open(&agreement_partition(epoch), b"blob")
                    .await
                    .expect("partition");
            }
            let mut agreements: BTreeMap<Epoch, Handle<()>> = BTreeMap::new();
            agreements.insert(
                Epoch::new(5),
                ctx.with_label("live")
                    .spawn(move |_| async move { std::future::pending::<()>().await }),
            );

            prune_agreements(&ctx, &mut agreements, 5).await;

            assert!(
                ctx.scan(&agreement_partition(3)).await.is_err(),
                "the partition an aborted supervisor left behind was never reclaimed"
            );
            assert!(
                ctx.scan(&agreement_partition(5)).await.is_ok(),
                "the live instance's own partition must survive"
            );
            assert!(
                ctx.scan(&agreement_partition(20)).await.is_ok(),
                "the sweep must stay inside its band"
            );
        });
    }
}
