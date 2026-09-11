//! Cert-inlet: a second producer into the singleton marshal.
//!
//! A validator's marshal is fed by ONE producer today — the local BFT engine,
//! which `verified()`s each block it proposes/verifies and `report()`s the
//! finalization cert it forms. The cert-inlet is the SECOND producer: it
//! BLS-verifies an upstream `(Finalization, OrderBlock)` against the on-chain
//! committee, makes the body local via `verified()`, then `report()`s the cert
//! — driving the marshal (and through it the executor, the sole reth writer)
//! exactly as a locally-formed finalization would. The inlet itself writes
//! NOTHING to reth.
//!
//! It is the SOLE producer for a non-validator follower
//! ([`crate::dpos::DposLayer::launch_follower`]) and a SECOND producer (next to
//! the local BFT engine) on an upstream-configured validator.

use crate::{
    beacon::{Beacon, ObservedCertificate, PinEffort},
    cert_follow::UpstreamFinalized,
    digest::Digest,
    scheme::epoch_committee_from_snapshot,
};
use alloy_consensus::Header;
use alloy_primitives::B256;
use commonware_consensus::simplex::types::Activity;
use commonware_parallel::Sequential;
use eyre::{ensure, eyre};
use fluentbase_bls::{
    fluent_namespace, oracle::SeedOracle, scheme::build_verifier, Scheme as BlsScheme,
};
use fluentbase_staking_reader::RethStakingStateReader;
use futures::future::BoxFuture;
use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{counter::Counter, family::Family},
};
use rand_core::CryptoRngCore;
use reth_ethereum_primitives::EthPrimitives;
use reth_evm::ConfigureEvm;
use reth_storage_api::{HeaderProvider, StateProviderFactory};
use std::sync::Arc;
use tracing::warn;

/// `{reason=...}` label set of the `dpos_cert_inlet_committee_read_deferred_total`
/// counter. ONE reason value is left — [`DEFER_COMMITTEE_NOT_COMMITTED`], the
/// inlet's single defer. The other two named a cursor this inlet no longer owns,
/// and the committee module answers both — but NOT with the same verdict, which
/// is the whole reason the module types its errors:
///
/// * `state_not_materialized` was this inlet's own executed-state probe missing.
///   In the module that is a `NotReadable` park (`committee/store.rs`, the
///   `Ok(None)` arm of step 4): no counter, no log, retried on the next wake-up.
/// * `probe_inconsistency` was the follower `finalized_hash` closure's
///   header-index fault, and it is PERMANENT, not transient: `Ok(None)` at a
///   materialized height is [`fluentbase_staking_reader::ReadError::Backend`]
///   (`executed.rs`, the `Ok(None)` arm of `executed_state_hash`), whose
///   `is_transient()` is `false`, so the module prints its once-per-epoch
///   `error!` and ticks
///   `dpos_committee_read_permanent_total{reason="anchor_fault"}`. The inlet
///   sees the same `None` it sees for a park, and that is correct here — it
///   skips the cert either way — but the CLASS is not lost, it is reported one
///   layer down.
///
/// The LABEL SET stays a set so the series keeps its shape.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct CommitteeReadDeferLabels {
    pub reason: &'static str,
}

/// The `reason` value for the committee-not-yet-committed defer (the executor has
/// not drained to epoch-(E-1)-start yet, or a deep backfill keeps the follower
/// `finalized_hash` closure returning `None`).
pub const DEFER_COMMITTEE_NOT_COMMITTED: &str = "committee_not_committed";

/// Consecutive DATA faults — an upstream serving cryptographically-unverifiable
/// certs over a HEALTHY connection — before the inlet rotates to the next
/// configured upstream URL ([`RotateUpstream`]).
///
/// A DATA fault is a cert that is STRUCTURALLY served but fails BLS verification
/// against a committee that IS readable (a forged/compromised upstream), or a
/// `payload != digest` structural mismatch. Connection-level failures rotate
/// inside the transport actor on their own; this counter is the ONLY signal a
/// data fault can never surface to that layer (the connection is fine; the
/// PAYLOAD is bad). The benign committee-lag skip (the committee module having
/// no record for the epoch yet) is NOT a data fault — it is transient boundary lag and
/// rotating away from a healthy upstream over it would be a churn footgun — so
/// it never increments the counter. Any successful verify/ingest resets it to 0.
///
/// Value 3: a single transient hiccup (a momentary cross-epoch race, a one-off
/// re-org served right at a boundary) must not trigger rotation, but a
/// persistently bad upstream is failed-over quickly.
pub const MAX_UPSTREAM_FAULTS: u32 = 3;

/// Certificates admitted on the multisig quorum ALONE, at a beacon-active epoch
/// whose scheme carries no seed pin. Not a fault on its own — a node legitimately
/// starts an epoch without its key — but a count that keeps CLIMBING means the
/// epoch's `PK_epoch` never arrived, and the seed slot of every one of those
/// certificates went unchecked.
pub const CERT_VOTE_ONLY_ADMISSIONS: &str = "dpos_cert_vote_only_admissions_total";

/// Boxed rotation callback the inlet calls after [`MAX_UPSTREAM_FAULTS`]
/// CONSECUTIVE data faults — drops the current upstream connection and moves to
/// the next configured URL (the node-side [`crate::cert_follow::CertUpstream::rotate`]
/// wired through a closure). Boxed (mirrors the executor's `ReJump` style) so the
/// inlet does not grow a `U: CertUpstream` generic on its already-wide type
/// parameters; the non-upstream / test inlets default it to `None`.
pub type RotateUpstream = Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;

/// Source of a per-epoch BLS verifier AT A SPECIFIC EXECUTED HASH — the
/// cold-start jump's committee read. Kept as a trait so the unit test can inject
/// a canned committee.
///
/// It used to have a second method, `scheme_at_finalized_tip`, and that method
/// was the cert-inlet's hot path: its own committee read, at its own cursor,
/// into its own `{prev, cur}` scheme cache. Both are gone — the inlet takes the
/// epoch's scheme from [`crate::committee::Committee`], the one map where a
/// record and its scheme live together — so what is left here is the ONE read
/// the module cannot answer: a committee at an ARBITRARY hash, which is what
/// authenticating a jump target means (the landing is not in the module's window
/// and is not this node's anchor).
///
/// `oracle` is the beacon's threshold face for the epoch, from
/// [`crate::beacon::Beacon::oracle_for`]: `Some` makes the built verifier
/// check the recovered seed slot and refuse a stripped one, `None` marks a
/// pre-beacon epoch where a seedless certificate is legal. The verifier read
/// itself is committee-only — the oracle comes from the caller, never from chain
/// storage (the PK_E layer stays deleted).
pub trait CommitteeSource: Send + Sync + 'static {
    /// Read `committee[epoch]` at a SPECIFIC executed hash. Used by the
    /// cold-start jump ([`crate::cold_start_jump::verify_jump_authenticated`])
    /// and the follower's `cold_start_register`, which both have a
    /// known-committed hash to read at.
    fn scheme_at(
        &self,
        epoch: u64,
        at_hash: B256,
        oracle: Option<Arc<dyn SeedOracle>>,
    ) -> eyre::Result<BlsScheme>;
}

/// [`CommitteeSource`] over a node's own reth state: committee snapshot at the
/// given executed hash → BLS verifier. The consensus-crate home for the
/// per-epoch verifier read both the cert-inlet (`--cert-follow`/upstream
/// validators) and the cold-start jump ([`crate::cold_start_jump`]) consume.
pub struct RethCommitteeSource<Provider, EvmConfig> {
    reader: RethStakingStateReader<Provider, EvmConfig>,
    namespace: Vec<u8>,
}

impl<Provider, EvmConfig> RethCommitteeSource<Provider, EvmConfig>
where
    Provider:
        StateProviderFactory + HeaderProvider<Header = Header> + Clone + Send + Sync + 'static,
    EvmConfig: ConfigureEvm<Primitives = EthPrimitives> + Clone + Send + Sync + 'static,
{
    pub fn new(reader: RethStakingStateReader<Provider, EvmConfig>, chain_id: u64) -> Self {
        Self {
            reader,
            namespace: fluent_namespace(chain_id),
        }
    }

    /// Build the verifier for `epoch` from the committee snapshot at `at_hash`,
    /// bound to `epoch` and to the caller's oracle.
    fn build_at(
        &self,
        epoch: u64,
        at_hash: B256,
        oracle: Option<Arc<dyn SeedOracle>>,
    ) -> eyre::Result<BlsScheme> {
        let snap = self.reader.epoch_committee_snapshot(epoch, at_hash)?;
        ensure!(
            !snap.validators.is_empty(),
            "epoch {epoch} has no committed committee at {at_hash}"
        );
        let committee = epoch_committee_from_snapshot(&snap)
            .map_err(|e| eyre!("epoch {epoch} committee has non-unique participants: {e:?}"))?;
        Ok(build_verifier(
            &self.namespace,
            committee.bimap,
            epoch,
            oracle,
        ))
    }
}

impl<Provider, EvmConfig> CommitteeSource for RethCommitteeSource<Provider, EvmConfig>
where
    Provider:
        StateProviderFactory + HeaderProvider<Header = Header> + Clone + Send + Sync + 'static,
    EvmConfig: ConfigureEvm<Primitives = EthPrimitives> + Clone + Send + Sync + 'static,
{
    fn scheme_at(
        &self,
        epoch: u64,
        at_hash: B256,
        oracle: Option<Arc<dyn SeedOracle>>,
    ) -> eyre::Result<BlsScheme> {
        self.build_at(epoch, at_hash, oracle)
    }
}

/// The marshal-facing sink the inlet drives: make a body local
/// ([`Self::verify_block`]) then report its finalization
/// ([`Self::report_finalization`]).
///
/// The real [`crate::MarshalMailbox`] implements this; its `Mailbox::new`
/// constructor is `pub(crate)` upstream, so a fake cannot impersonate the
/// concrete type — the trait is the seam that lets the unit test record call
/// order against a `FakeMarshal` while production wires the real mailbox.
pub trait MarshalSink: Send {
    /// Persist a verified body so the marshal resolves it WITHOUT a peer fetch.
    /// Fire-and-forget at the pinned `marshal::core::Mailbox::verified` (it
    /// `send_lossy`s and returns `()` in the locked `v2026.4.0` rev — the
    /// durability-ack `-> bool` variant is a NEWER upstream rev we are NOT on).
    fn verify_block(
        &mut self,
        round: commonware_consensus::types::Round,
        block: crate::order_block::OrderBlock,
    ) -> impl std::future::Future<Output = ()> + Send;

    /// Report a finalization certificate — drives storage + the executor.
    fn report_finalization(
        &mut self,
        finalization: commonware_consensus::simplex::types::Finalization<BlsScheme, Digest>,
    ) -> impl std::future::Future<Output = ()> + Send;
}

impl MarshalSink for crate::MarshalMailbox {
    async fn verify_block(
        &mut self,
        round: commonware_consensus::types::Round,
        block: crate::order_block::OrderBlock,
    ) {
        // `MarshalMailbox::verified` takes `V::Block`, which for
        // `Standard<OrderBlock>` IS `OrderBlock` — no `.into()` conversion.
        self.verified(round, block).await
    }

    async fn report_finalization(
        &mut self,
        finalization: commonware_consensus::simplex::types::Finalization<BlsScheme, Digest>,
    ) {
        use commonware_consensus::Reporter as _;
        // The marshal `Reporter::report` takes a `simplex::types::Activity`
        // (NOT a `marshal::types::Activity`); it routes `Finalization` to the
        // in-order finalization path and ignores other variants.
        self.report(Activity::Finalization(finalization)).await;
    }
}

/// The live-frontier tee: the beacon-plane cursors a validator's cert-inlet
/// advances from each live upstream cert it ingests. Re-homed here from the
/// (deleted) unified supervisor that used to feed them off the window stream.
///
/// `live_height` is a committee-read cursor for the FOLLOWER only: its inlet's
/// own committee source reads `committee[E]` at `max(EL-finalized, live_height)`
/// (`dpos.rs`, `inlet_committees`), so a follower resolves the ahead-committed
/// `committee[E+1]` at the LIVE upstream tip rather than its lagging
/// EL-finalized state (the boundary-wedge fix). On the VALIDATOR path it no
/// longer steers any committee read: the beacon plane takes every committee
/// through the committee module, at this node's own ordering-finalized anchor,
/// and the atomic is write-only there until 4.2 removes it together with
/// `upstream_frontier`. `dkg_height` is the `beacon::actor::DkgActor` deal
/// clock: dealing at the live frontier lets a still-catching-up early-joiner
/// deal its first epoch's DKG share before the deal deadline (the vrf-rotation
/// early-join fix) instead of K blocks late.
///
/// A validator-with-upstream wires BOTH cursors (it owns the beacon plane). A
/// FOLLOWER also wires the tee — but only for `live_height`, with a NO-OP
/// `dkg_height_tx` (the receiver is dropped — the follower has no beacon plane).
/// A no-upstream validator has no inlet at all → both cursors stay
/// finalized-driven, unchanged.
pub struct LiveFrontierTee {
    /// `committee_for` read cursor, advanced monotonically (`fetch_max`) — ONLY
    /// off VERIFIED certs (a trusted frontier; it must never be steerable by an
    /// unverified upstream cert).
    pub live_height: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// The TRUE upstream frontier the executor's steady-state re-jump triggers
    /// off (see [`crate::executor::ReJump::upstream_frontier`]). Unlike
    /// `live_height` this is advanced on EVERY structurally-valid cert in
    /// [`CertInlet::ingest`] — INCLUDING the "committee[E] not committed" deferred
    /// ones — so a deadlocked follower (whose marshal tip has frozen because the
    /// inlet stores nothing while it defers) still observes the climbing frontier
    /// and re-jumps. HEIGHT-ONLY and NOT a trust input: it only sizes the re-jump
    /// gap; the jump itself re-reads + BLS-authenticates the committee at the
    /// landing, so an inflated frontier can at worst trigger a jump that then
    /// fails closed — it can never select a committee.
    pub upstream_frontier: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// DkgActor deal clock; its `on_height` clamps to its own running max, so a
    /// stale `try_send` never pulls the clock backward.
    pub dkg_height_tx: tokio::sync::mpsc::Sender<u64>,
    /// Where a dropped `dkg_height_tx` send is counted. The inlet does NOT write
    /// the height gauge — the `DkgActor` does, at the clamp where all three
    /// feeders meet — but a full channel is the one way this feeder can silently
    /// leave the clock behind, so it has to be visible from here.
    pub plane_clock: crate::sync_metrics::PlaneClock,
}

/// One of two producers into the singleton marshal (the other is the local BFT
/// engine). BLS-verifies each upstream cert against the on-chain committee,
/// makes the body local via [`MarshalSink::verify_block`], then reports the cert.
/// The executor (the sole reth writer) then drives reth identically to a
/// locally-finalized cert.
pub struct CertInlet<E, M> {
    marshal: M,
    /// EVERY per-epoch scheme this inlet verifies with, from the ONE map that
    /// holds a committee record and its scheme in the same slot.
    ///
    /// It replaced a `CommitteeSource` generic plus a private `{prev, cur}`
    /// cache plus a cursor closure of its own — a second authority on "what is
    /// `committee[E]`" sitting beside the module, on a different anchor, with a
    /// different retention. Nothing here can be stale: an entry is write-once
    /// inside the module's window, and outside it there is no entry at all.
    committee: Arc<dyn crate::committee::Committee>,
    /// The live-frontier tee — see [`LiveFrontierTee`]. `Some` on a
    /// validator-with-upstream (the production-path / early-join fix) AND on a
    /// follower (its frontier-aware committee read, with a no-op dkg clock);
    /// `None` only in unit tests that do not exercise the frontier read.
    tee: Option<LiveFrontierTee>,
    /// B3 — the SECOND sink: each VERIFIED pair is also forwarded to the node's
    /// `consensus`-RPC serving window (the D4 `verified_tx` stream). A TIER-2
    /// follower aligns by reading THIS node's window WS, not its marshal archive,
    /// so a marshal-only inlet would break the cert-cascade. `None` on a
    /// validator (it serves from the Marshal source) and in unit tests that
    /// don't exercise the window. Only BLS-verified pairs ever enter (the emit is
    /// AFTER the verify gate), so the served window can never expose an
    /// unverified cert.
    window_tx: Option<tokio::sync::mpsc::UnboundedSender<UpstreamFinalized>>,
    /// The node's SHARED beacon-key store (see [`BeaconKeys`]) — the same handle
    /// `FluentApp` and `epoch_manager` hold on a validator. The inlet reads it as
    /// the store rung of [`BeaconKeys::get_pk`]'s ladder, and the ladder's own
    /// resolve fills it.
    ///
    /// It replaced a PRIVATE carry-forward cursor that answered "what is `PK_E`"
    /// as "the greatest observed change-epoch ≤ E" — an UNBOUNDED walk forward
    /// from the last key it happened to see. The chain's own `dkgQual` record,
    /// which `beacon::keys::AgreedKeys` reads, is the one policy both
    /// planes now use.
    ///
    /// A default-constructed store on an inlet nobody wired one into is a private
    /// empty one — the unit-test shape.
    randomness: Arc<dyn Beacon>,
    /// commonware ctx (the `CryptoRngCore` source the cert `verify()` needs).
    ctx: E,
    /// DATA-fault upstream-rotation trigger. `Some` on an upstream-configured
    /// inlet (a follower or an upstream-validator); `None` for tests. After
    /// [`MAX_UPSTREAM_FAULTS`] CONSECUTIVE data faults `ingest` invokes it (drop
    /// the connection + advance to the next URL) and resets the counter — the
    /// only failover signal for an upstream serving bad PAYLOAD over a healthy
    /// connection (see [`RotateUpstream`]).
    rotate: Option<RotateUpstream>,
    /// Consecutive data faults since the last successful verify/ingest. Reset to
    /// 0 on ANY success (a genuine ingest OR the benign committee-lag skip),
    /// after a rotation fires, AND whenever the underlying upstream CONNECTION
    /// changes (see [`Self::conn_gen`]). Only a BLS-verify failure / structural
    /// mismatch against a READABLE committee increments it.
    consecutive_faults: u32,
    /// Per-CONNECTION fault scoping (#7). The WS upstream actor auto-rotates to
    /// the next URL on a dropped/failed CONNECTION (connect/subscribe failure)
    /// WITHOUT signalling the inlet, so without this the data-fault streak from
    /// upstream A would carry into upstream B's budget — firing a premature
    /// data-fault `rotate()` after fewer than [`MAX_UPSTREAM_FAULTS`] B faults
    /// and skipping a possibly-healthy B. The actor bumps a shared generation
    /// counter each time it (re)establishes a connection; the inlet observes the
    /// token at the head of each `ingest` and, on a change, resets the streak —
    /// so the data-fault count is scoped to the LIVE connection (event-driven off
    /// the actor's own connection lifecycle, no poll/timer). `Some` on an
    /// upstream-configured inlet (wired from [`Self::with_connection_token`]);
    /// `None` for tests and the no-upstream inlet (streak is then inlet-global,
    /// the prior behaviour).
    conn_gen: Option<Arc<std::sync::atomic::AtomicU64>>,
    /// The connection generation the inlet last observed (`conn_gen` at the prior
    /// `ingest`). A mismatch vs the live `conn_gen` means the connection rotated
    /// underneath the inlet → reset the per-connection streak.
    last_seen_conn_gen: u64,
    /// Activation-relative epoch geometry `(dpos_activation_block,
    /// epoch_block_interval)` for the defense-in-depth height↔epoch bind in
    /// `ingest`. `Some` on the FOLLOWER inlet (the untrusted-upstream consumer,
    /// where committee reads are fully trusted); `None` on a validator inlet (it
    /// owns a consensus plane that re-derives + cross-checks the result) and in
    /// tests. `interval` MUST be `> 0` (the follower cold-start guards it before
    /// wiring; the shared `epoch_at_block` answers `None` on a zero interval and
    /// `ingest` treats that as a height that matches no epoch).
    epoch_bind: Option<(u64, u64)>,
    /// `dpos_cert_inlet_committee_read_deferred_total{reason}` — ticks once per
    /// cert deferred because the committee read hit a TRANSIENT reth
    /// pipeline-backfill state-miss (`ReadError::StateNotMaterialized`), NOT a
    /// corrupt read. Default (unregistered) on inlets not wired via
    /// [`Self::with_committee_read_deferred_metric`] (validator inlets / tests):
    /// the increment is then a harmless no-op on a local family.
    committee_read_deferred: Family<CommitteeReadDeferLabels, Counter>,
    /// `dpos_cert_inlet_carry_forward_pin_verify_failed_total` — ticks once per
    /// BLS-verify failure of a cert whose scheme was built THIS ingest with a
    /// DERIVED seed pin: one the ladder resolved (the shared store, or an earlier
    /// epoch's boundary block via the walk) rather than one this block asserted
    /// itself. Splits that regime from genuine forged-upstream data faults on
    /// dashboards — the metric NAME is kept because the regime it names is the
    /// same one, now bounded: a key carried forward from an earlier mint.
    /// Default (unregistered) on inlets not wired via
    /// [`Self::with_carry_forward_fail_metric`].
    carry_forward_verify_failed: Counter,
    /// Once-per-episode rate limit for the committee-not-readable WARN: during a
    /// deep backfill the module answers `NotReadable` for every cert, ~1/s for
    /// minutes, so the un-limited warn used to spam. Cleared on the next clean
    /// verified ingest.
    committee_not_committed_warned: bool,
}

impl<E, M> CertInlet<E, M>
where
    E: CryptoRngCore + Send,
    M: MarshalSink,
{
    /// The inlet ALWAYS BLS-verifies upstream certs against the on-chain
    /// committee (no `verify:false` mode exists in v1 — the standalone bare
    /// `--cert-upstream` trust relay is the separate `launch_consensus_node`
    /// path, not an inlet).
    pub fn new(marshal: M, committee: Arc<dyn crate::committee::Committee>, ctx: E) -> Self {
        Self {
            marshal,
            committee,
            tee: None,
            window_tx: None,
            randomness: crate::beacon::absent_unregistered(),
            ctx,
            rotate: None,
            consecutive_faults: 0,
            conn_gen: None,
            last_seen_conn_gen: 0,
            epoch_bind: None,
            committee_read_deferred: Family::default(),
            carry_forward_verify_failed: Counter::default(),
            committee_not_committed_warned: false,
        }
    }

    /// Attach the (already-registered) `dpos_cert_inlet_committee_read_deferred_total`
    /// counter family. Builder-style: the follower launch registers the family on
    /// the launch context and passes a clone here (Arc-backed → shares the
    /// counter); a validator inlet / unit test leaves the default unregistered
    /// family (the increment is then observationally inert).
    pub fn with_committee_read_deferred_metric(
        mut self,
        metric: Family<CommitteeReadDeferLabels, Counter>,
    ) -> Self {
        self.committee_read_deferred = metric;
        self
    }

    /// Attach the (already-registered) carry-forward-pin verify-failure counter
    /// (see `carry_forward_verify_failed`). Builder-style, same idiom as
    /// [`Self::with_committee_read_deferred_metric`].
    pub fn with_carry_forward_fail_metric(mut self, metric: Counter) -> Self {
        self.carry_forward_verify_failed = metric;
        self
    }

    /// Attach the randomness provider. Builder-style because the provider is
    /// assembled at the launch site, after this inlet exists.
    pub fn with_randomness(mut self, randomness: Arc<dyn Beacon>) -> Self {
        self.randomness = randomness;
        self
    }

    /// Attach the activation-relative epoch geometry for the height↔epoch bind
    /// (defense-in-depth; see `epoch_bind`). Builder-style: the FOLLOWER inlet
    /// wires `(dpos_activation_block, epoch_block_interval)`; a validator inlet /
    /// unit test leaves it `None` (the bind is then a no-op). `interval` MUST be
    /// `> 0` (the caller's cold-start guards it).
    pub fn with_epoch_math(mut self, activation: u64, interval: u64) -> Self {
        self.epoch_bind = Some((activation, interval));
        self
    }

    /// Attach the DATA-fault upstream-rotation trigger (see [`RotateUpstream`]).
    /// Builder-style: an upstream-configured inlet (follower / upstream-validator)
    /// wires this from its `CertUpstream` handle; a unit test that does not
    /// exercise rotation leaves it `None`. Without it a persistently bad upstream
    /// is still skipped non-fatally — it just never fails over to a backup URL.
    pub fn with_rotate(mut self, rotate: RotateUpstream) -> Self {
        self.rotate = Some(rotate);
        self
    }

    /// Attach the per-connection fault-scoping token (#7 — see [`Self::conn_gen`]).
    /// Builder-style: an upstream-configured inlet wires the SAME
    /// `Arc<AtomicU64>` the WS actor bumps on each (re)connect, so a
    /// connection-level auto-rotation resets the inlet's data-fault streak (the
    /// streak from upstream A never bleeds into upstream B's budget). A unit test
    /// / no-upstream inlet leaves it `None` (streak inlet-global, prior behaviour).
    /// The initial generation is observed eagerly so the FIRST connection's
    /// faults are not falsely reset against a stale `0`.
    pub fn with_connection_token(mut self, conn_gen: Arc<std::sync::atomic::AtomicU64>) -> Self {
        self.last_seen_conn_gen = conn_gen.load(std::sync::atomic::Ordering::Acquire);
        self.conn_gen = Some(conn_gen);
        self
    }

    /// Attach the B3 serving-window sink. Builder-style: a follower with a
    /// `consensus`-RPC feed wires this; a validator (or a unit test that does not
    /// exercise the window) leaves it `None`.
    pub fn with_window(
        mut self,
        window_tx: tokio::sync::mpsc::UnboundedSender<UpstreamFinalized>,
    ) -> Self {
        self.window_tx = Some(window_tx);
        self
    }

    /// Attach the live-frontier tee (see [`LiveFrontierTee`]). Builder-style: a
    /// validator-with-upstream wires this from its beacon plane so committee
    /// resolution + DKG dealing track the LIVE cert frontier; a follower (no
    /// beacon plane) and unit tests leave it `None`.
    pub fn with_tee(mut self, tee: LiveFrontierTee) -> Self {
        self.tee = Some(tee);
        self
    }

    /// BLS-verify one upstream cert, then drive the marshal with it.
    ///
    /// INFALLIBLE, and that is the contract rather than an accident of the
    /// current body. Every outcome a cert can have here is a skip: a malformed
    /// or cross-epoch cert, a verify failure, an epoch whose committee this node
    /// cannot read yet. Risk-3 is why — a single bad upstream cert must not halt
    /// the inlet; the marshal stalls naturally at the gap until a good cert
    /// arrives. The `eyre::Result<()>` this used to return carried exactly one
    /// `Err`, the committee source's read fault, and that class now belongs to
    /// the committee module, which can tell "I cannot read this yet" from "the
    /// contract answered something impossible" where an `eyre::Report` could
    /// not: it reports the permanent one itself (`error!` +
    /// `dpos_committee_read_permanent_total`) and defers the other. Keeping the
    /// return type would have kept two call-site branches that can never be
    /// taken.
    pub async fn ingest(&mut self, uf: UpstreamFinalized) {
        // Per-CONNECTION fault scoping (#7): if the WS actor (re)connected since
        // the last cert — its own connect/subscribe-failure auto-rotation, which
        // the inlet has no other way to observe — the data-fault streak belongs
        // to the OLD connection. Reset it so A's faults never bleed into B's
        // budget (a premature `rotate()` away from a possibly-healthy B). See
        // `conn_gen`.
        if let Some(conn_gen) = &self.conn_gen {
            let gen = conn_gen.load(std::sync::atomic::Ordering::Acquire);
            if gen != self.last_seen_conn_gen {
                self.last_seen_conn_gen = gen;
                self.consecutive_faults = 0;
            }
        }
        let round = uf.finalization.proposal.round;
        let epoch = round.epoch().get();
        // DEFENSE-IN-DEPTH height↔epoch bind: an honest cert always
        // satisfies `round.epoch() == epoch_of(block.height)` — the per-epoch
        // engine proposes only its own height range `[epoch_start(E),
        // epoch_start(E+1))`. A mismatch is a malformed / cross-epoch cert (a
        // Byzantine `committee[E]` finalizing an out-of-range height), so SKIP it
        // as a DATA FAULT (same class as a tampered body) BEFORE selecting
        // `committee[E]`'s scheme for a block that does not belong to it. Honest
        // input never trips this; on the validator inlet (`epoch_bind == None`) it
        // is a no-op (the consensus plane re-derives + cross-checks instead).
        if let Some((activation, interval)) = self.epoch_bind {
            // `epoch_bind` carries a non-zero interval by construction (it is the
            // frozen geometry), so the shared epoch function cannot answer `None`.
            let height_epoch = fluentbase_staking_reader::reader::epoch_at_block(
                uf.block.height,
                activation,
                interval,
            );
            if height_epoch != Some(epoch) {
                warn!(
                    height = uf.block.height,
                    cert_epoch = epoch,
                    height_epoch,
                    "cert-inlet: cert round-epoch != block height-epoch; \
                     skipping (malformed/cross-epoch)"
                );
                self.record_data_fault().await;
                return;
            }
        }
        // Advance the executor's steady-state re-jump frontier off EVERY
        // structurally-valid cert — CRUCIALLY including the committee-not-committed
        // deferred ones below — so a deadlocked follower (frozen marshal tip) still
        // sees the climbing upstream frontier and re-jumps. HEIGHT-ONLY: the
        // committee read uses the verified-only `live_height` tee, NOT this
        // (see [`LiveFrontierTee::upstream_frontier`]).
        if let Some(tee) = &self.tee {
            tee.upstream_frontier
                .fetch_max(uf.block.height, std::sync::atomic::Ordering::Relaxed);
        }
        // The inlet ALWAYS verifies. The certificate must sign THIS artifact: BLS
        // verify alone proves a quorum signed `proposal.payload`, NOT that the
        // served body matches it. A swapped body under a valid cert is the same
        // Risk-3 skip as a bad signature.
        let digest = uf.block.digest();
        if uf.finalization.proposal.payload != digest {
            warn!(
                height = uf.block.height,
                epoch, "cert-inlet: cert payload != block digest; skipping (tampered/mismatched)"
            );
            // DATA FAULT: a structural mismatch (the served body does not match
            // the cert) over a healthy connection — count it toward rotation.
            self.record_data_fault().await;
            return;
        }
        // ACQUISITION, and it is load-bearing rather than bookkeeping. The scheme's
        // oracle answers `verify_seed` from the SYNC key-store probe alone, and the
        // only thing that ever fills that store for an artifact-sourced epoch is
        // this ladder walk — `BeaconKeys::get_pk` writes back what it resolves.
        // Drop this call and every such epoch stays permanently keyless, admitting
        // its certificates on the multisig half for the life of the process.
        //
        // Runs on EVERY certificate, unlike the pin resolve it replaces: there is
        // no cached-pin state to bound it with any more, and the store hit it
        // starts with is the same one the old `cached_pinned` short-circuit was
        // approximating.
        //
        // `Local` and never `Thorough`: ingress runs against a ~1 s verify budget
        // and the network rung's is seconds. A pull here would move a peer
        // round-trip onto the vote path, where a missing key costs a vote-only
        // admission and a stall costs a missed view.
        let key_known = self.randomness.ensure_key(epoch, PinEffort::Local).await;
        // The epoch's scheme, from the committee module's map. This used to be
        // the inlet's OWN committee read at its OWN cursor into its OWN
        // `{prev, cur}` cache, with three outcomes (ready / not committed yet /
        // fatal) and a rebuild-on-miss dance. The module has already collapsed
        // all of that: the record is write-once inside its window, the scheme
        // lives in the same slot, and reading is what produces both.
        //
        // `None` is the ONE deferral, and it is the same non-fatal skip the
        // "committee[E] not committed at the finalized tip" arm always was:
        // `ingest` runs in the SINGLE task that drains the cert source AND feeds
        // the executor that advances the anchor the module reads at, so blocking
        // or failing here would starve the very progress it waits on. Skip this
        // cert, keep draining, and let a later cert of the same epoch find the
        // record.
        //
        // There is NO fatal arm left. The corrupt-read class the old `Err`
        // carried has not disappeared — the module reports it, counts it under
        // `dpos_committee_read_permanent_total` and logs it once — but it no
        // longer kills a follower that is merely reading a state it does not
        // have yet, because the module can tell those two apart and a
        // `eyre::Result` from a committee source could not.
        let Some(scheme) = self.committee.scheme(epoch) else {
            self.committee_read_deferred
                .get_or_create(&CommitteeReadDeferLabels {
                    reason: DEFER_COMMITTEE_NOT_COMMITTED,
                })
                .inc();
            if !self.committee_not_committed_warned {
                self.committee_not_committed_warned = true;
                warn!(
                    height = uf.block.height,
                    epoch,
                    "cert-inlet: no committee[E] at this node's committee anchor yet; deferring \
                     certs until it is readable (rate-limited; \
                     dpos_cert_inlet_committee_read_deferred_total ticks per cert)"
                );
            }
            // NOT a data fault: an unreadable committee is this node's own lag,
            // not an unverifiable certificate (#4). Rotating away from a HEALTHY
            // upstream over it would be a churn footgun, so `consecutive_faults`
            // is left untouched — neither incremented nor reset.
            return;
        };
        // The only direct witness that an epoch is being admitted WITHOUT its seed
        // checked: the multisig quorum is verified, the seed slot is not because
        // the oracle answers `NoKey`. Keyed on the acquisition above rather than on
        // the scheme, which no longer holds the key to be asked about — and the
        // acquisition runs per certificate, so this counts admissions rather than
        // first-of-epoch misses.
        if !key_known && self.randomness.mandatory_at(epoch) {
            metrics::counter!(CERT_VOTE_ONLY_ADMISSIONS).increment(1);
        }
        if !uf.finalization.verify(&mut self.ctx, &scheme, &Sequential) {
            warn!(
                height = uf.block.height,
                epoch, "cert-inlet: BLS verify FAILED; skipping (marshal stalls naturally)"
            );
            // Nothing to evict. The eviction that stood here existed because the
            // inlet's own cache could hold a scheme built from a committee read
            // at a cursor that had since moved; a module entry cannot be that —
            // it is write-once inside the window and it leaves the map only with
            // the window. A verify failure is therefore a statement about the
            // CERTIFICATE, and re-reading the committee could only produce the
            // same record.
            // Regime split: a failure while the epoch's key WAS resolvable points
            // at a key carried forward from the wrong mint rather than at a forged
            // upstream — count it separately so dashboards can tell the two apart.
            // A failure with no key resolvable cannot be this class at all: the
            // seed half was never checked.
            if key_known {
                self.carry_forward_verify_failed.inc();
            }
            // DATA FAULT: the cert FAILS BLS against a committee that IS readable
            // (the `scheme` above resolved) — a forged / compromised upstream
            // serving bad payload over a healthy connection. Count it toward
            // rotation (the connection-level failover can NEVER detect this).
            self.record_data_fault().await;
            return;
        }
        // The certificate is verified and its seed slot rode along inside that
        // verification whenever the key was resolvable. Capture σ HERE rather
        // than leave the transport as its only source: a node following epoch E
        // from outside `committee[E]` sees a verified σ for every finalized
        // round of E and, until this line existed, dropped every one of them —
        // which is why a zero-overlap boundary had nothing to witness with.
        //
        // Keyed by the CERTIFICATE'S OWN round, which is what makes the capture
        // fork-safe: σ signs `seed_message(round)`, so for one round there is at
        // most one signature that verifies, and a round this node will not ask
        // for can only sit unused. This is a supply of BYTES for a round, never
        // a claim about WHICH round a block's witness names (§13 rule 28).
        let _observed = self
            .randomness
            .observe_certificate(ObservedCertificate::Finalization(round, &uf.finalization));
        // PLAN row 5.2: the synchronous `Refused` becomes this inlet's data
        // fault. Until then nobody reads the verdict here.

        // Verified: a clean ingest — reset the data-fault streak and end any
        // open defer WARN episodes (the next backfill / boundary-lag window
        // warns afresh).
        self.consecutive_faults = 0;
        self.committee_not_committed_warned = false;
        // No scheme retention of its own any more: the inlet holds no map, and
        // the one it now reads is retained by the committee module's own read
        // window. The prune that stood here kept `{prev, cur}` on the inlet's
        // clock, which during a deep catch-up could drop an entry the epoch
        // manager was still soft-entering against — two retentions over one
        // question. The beacon's key store keeps its own, below.
        self.randomness.observe_cert(epoch);
        // Re-homed live-frontier tee: advance the beacon-plane cursors off the
        // VERIFIED live upstream tip (skipped/tampered certs above never reach
        // here). `committee_for` then resolves committee[E+1] and the DkgActor
        // deals at the live frontier instead of this node's lagging EL-finalized
        // state. Both feeders are monotone (`fetch_max` / DkgActor `on_height`
        // clamps to its running max), so a stale tee can never rewind either
        // clock. `Some` only on a validator-with-upstream (it owns the plane).
        if let Some(tee) = &self.tee {
            tee.live_height
                .fetch_max(uf.block.height, std::sync::atomic::Ordering::Relaxed);
            if tee.dkg_height_tx.try_send(uf.block.height).is_err() {
                tee.plane_clock.note_height_drop();
            }
        }
        // Make the body local so the marshal resolves it without a peer, THEN
        // report the cert to drive storage + the executor — in that order. Plus
        // (B3) feed the serving window so a tier-2 follower can align via THIS
        // node's `consensus` WS window (NOT an alternative — a marshal-only inlet
        // fails the tier-2 cascade). A dropped window receiver (RPC shutting down)
        // is benign — the marshal sink is the load-bearing one.
        //
        // Clone discipline (the block tx Vec is up to 4 MB): the marshal
        // `verify_block` and the window BOTH need the body, but the window also
        // needs the finalization while `report_finalization` consumes it. So with
        // a window present we clone the body ONCE (for the marshal) + the
        // finalization ONCE (small — committee-bounded bitmap + sig), then MOVE
        // the body + the original finalization into the window — never a second
        // full-block clone. With NO window we MOVE the body into the marshal (zero
        // body clones) and the finalization into the report.
        // Clone the sender (cheap `Arc` clone) so the marshal mutable borrows
        // below are not entangled with a `&self.window_tx` field borrow.
        match self.window_tx.clone() {
            Some(tx) => {
                self.marshal.verify_block(round, uf.block.clone()).await;
                let _ = tx.send(UpstreamFinalized {
                    finalization: uf.finalization.clone(),
                    block: uf.block,
                });
                self.marshal.report_finalization(uf.finalization).await;
            }
            None => {
                self.marshal.verify_block(round, uf.block).await;
                self.marshal.report_finalization(uf.finalization).await;
            }
        }
    }

    /// Record one DATA fault (BLS-verify failure / structural mismatch against a
    /// READABLE committee) and, once [`MAX_UPSTREAM_FAULTS`] CONSECUTIVE faults
    /// accumulate, fire the upstream-rotation trigger and reset the streak. The
    /// rotation drops the current connection so the transport actor's run loop
    /// advances to the next configured URL — the only failover path for an
    /// upstream serving bad payload over a healthy connection (connection-level
    /// failover can never see it). No trigger configured (a unit test) ⇒ just
    /// count (the inlet keeps skipping non-fatally).
    async fn record_data_fault(&mut self) {
        self.consecutive_faults += 1;
        if self.consecutive_faults >= MAX_UPSTREAM_FAULTS {
            if let Some(rotate) = &self.rotate {
                warn!(
                    faults = self.consecutive_faults,
                    "cert-inlet: {MAX_UPSTREAM_FAULTS} consecutive upstream data faults; \
                     rotating to the next configured upstream URL"
                );
                rotate().await;
            }
            // Reset whether or not a trigger fired: without one the streak
            // would grow unboundedly + re-warn every cert; the skip path
            // already keeps the marshal stalled at the gap.
            self.consecutive_faults = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        beacon::testing::{
            encode_outcome, group_public_key, parse_outcome, AgreedKeyAt, AgreedKeys, BeaconKeys,
            DkgQualFor, LiveBeaconConfig,
        },
        order_block::OrderBlock,
    };
    use alloy_primitives::Bytes;
    use commonware_codec::DecodeExt as _;
    use commonware_consensus::{
        simplex::types::{Finalization, Finalize, Proposal},
        types::{Epoch, Round, View},
    };
    use commonware_cryptography::{
        bls12381::{
            dkg::deal,
            primitives::{
                group::Share,
                sharing::{Mode, Sharing},
                variant::MinSig,
            },
        },
        ed25519::PrivateKey as Ed25519PrivateKey,
        Signer as _,
    };
    use commonware_math::algebra::Random as _;
    use commonware_runtime::{deterministic, Runner as _};
    use commonware_utils::{
        ordered::{BiMap, Set},
        N3f1, TryCollect as _,
    };
    use fluentbase_bls::beacon::GroupPublic;
    use fluentbase_bls::{
        beacon::seed_namespace, fluent_namespace, keys::ValidatorBlsKeypair, scheme::build_signer,
        BlsPubkey, PeerPubkey,
    };
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;
    use std::sync::{Arc, Mutex};

    const CHAIN_ID: u64 = 20_994;
    const COMMITTEE_N: usize = 4;

    /// A plain (seedless) committee. Holds the keypairs rather than the schemes,
    /// because a scheme is bound to the epoch it was issued for and these tests
    /// certify at several.
    struct Committee {
        bls_kps: Vec<ValidatorBlsKeypair>,
        bimap: BiMap<PeerPubkey, BlsPubkey>,
        namespace: Vec<u8>,
    }

    impl Committee {
        fn signers(&self, epoch: u64) -> Vec<BlsScheme> {
            self.bls_kps
                .iter()
                .map(|kp| {
                    build_signer(&self.namespace, self.bimap.clone(), kp, epoch, None)
                        .expect("member")
                })
                .collect()
        }

        fn verifier(&self, epoch: u64) -> BlsScheme {
            build_verifier(&self.namespace, self.bimap.clone(), epoch, None)
        }
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
        Committee {
            bls_kps,
            bimap,
            namespace: fluent_namespace(CHAIN_ID),
        }
    }

    fn sample_order(parent: Digest, height: u64) -> OrderBlock {
        OrderBlock {
            parent,
            height,
            proposal_view: 0,
            timestamp: 1_700_000_000 + height,
            gas_limit: 30_000_000,
            extra_data: Bytes::new(),
            result: B256::ZERO,
            txs: Vec::new(),
            equivocation: None,
        }
    }

    /// 2f+1 finalize votes over the block's digest → a REAL finalization cert.
    fn certify(c: &Committee, epoch: u64, block: &OrderBlock) -> UpstreamFinalized {
        let round = Round::new(Epoch::new(epoch), View::new(block.height));
        let prop = Proposal::new(round, View::new(block.height - 1), block.digest());
        let finalizes: Vec<_> = c
            .signers(epoch)
            .iter()
            .take(3)
            .map(|s| Finalize::sign(s, prop.clone()).expect("sign"))
            .collect();
        let finalization =
            Finalization::from_finalizes(&c.verifier(epoch), finalizes.iter(), &Sequential)
                .expect("quorum");
        UpstreamFinalized {
            finalization,
            block: block.clone(),
        }
    }

    /// Records the marshal driving calls in order, so a test can assert the
    /// inlet `verified()`s before it `report()`s — and that a rejected cert
    /// drives NOTHING.
    #[derive(Clone, Default)]
    struct FakeMarshal {
        calls: Arc<Mutex<Vec<&'static str>>>,
    }

    impl MarshalSink for FakeMarshal {
        async fn verify_block(&mut self, _round: Round, _block: OrderBlock) {
            self.calls.lock().unwrap().push("verified");
        }
        async fn report_finalization(&mut self, _f: Finalization<BlsScheme, Digest>) {
            self.calls.lock().unwrap().push("report");
        }
    }

    /// Recorded `epoch` committee reads the canned module observed.
    type TestInlet = CertInlet<deterministic::Context, FakeMarshal>;
    type SchemeReads = Arc<Mutex<Vec<u64>>>;

    /// One committee, readable at every epoch — the verifier is built for the
    /// epoch actually asked about, because a scheme refuses a foreign one. The
    /// module memoises it exactly as the production store does, so `reads` counts
    /// what the real one would make it count.
    fn inlet(ctx: deterministic::Context, c: &Committee) -> (TestInlet, FakeMarshal, SchemeReads) {
        let marshal = FakeMarshal::default();
        let reads: SchemeReads = Arc::new(Mutex::new(Vec::new()));
        let bimap = c.bimap.clone();
        let recorded = reads.clone();
        let committee = crate::committee::testing::SchemeCommittee::new(move |epoch| {
            recorded.lock().unwrap().push(epoch);
            Some(build_verifier(
                &fluent_namespace(CHAIN_ID),
                bimap.clone(),
                epoch,
                None,
            ))
        });
        let inlet = CertInlet::new(marshal.clone(), committee, ctx);
        (inlet, marshal, reads)
    }

    // `committee_read_fault_defers_transient_and_blocknotfound_else_corruption`
    // stood here. Its subject — the inlet's own mapping of a committee-read error
    // into the family-5 fault taxonomy — went with the read: the inlet makes none.
    // The same split now lives in `CommitteeError::is_transient`, over a typed
    // error rather than an `eyre::Report` downcast, and is pinned by the module's
    // own tests plus `tests/slasher_integration.rs`.

    /// Reds the moment cert ingress starts spending the NETWORK rung.
    ///
    /// This replaces the old `ingress_sources().pull.is_none()` assertion, which
    /// died with the field it read. It asserts the same property one level out
    /// and more strongly: drive a REAL `ingest` against a recording provider and
    /// require that every pin resolution it asked for was `Local`. A helper that
    /// merely returned `Local` would be a tautology; only the call actually made
    /// can witness this.
    ///
    /// The property: ingress runs against a ~1 s verify budget while the network
    /// rung's is seconds, so a pull here costs a missed view.
    #[test]
    fn cert_ingress_never_spends_the_network_rung() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let c = committee(1);
            let (mut inlet, _marshal, _reads) = inlet(ctx, &c);
            let spy = Arc::new(crate::beacon::testing::Canned::new());
            inlet = inlet.with_randomness(spy.clone());

            let block = sample_order(Digest(B256::repeat_byte(0xaa)), 65);
            inlet.ingest(certify(&c, 0, &block)).await;

            let efforts = spy.efforts();
            assert!(
                !efforts.is_empty(),
                "the ingest must have resolved a pin at all, or this test proves nothing"
            );
            assert!(
                efforts.iter().all(|(_, e)| *e == PinEffort::Local),
                "cert ingress asked for {efforts:?}; a peer round-trip on the vote \
                 path costs a missed view"
            );
        });
    }

    #[test]
    fn valid_cert_verifies_then_reports_in_order() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let c = committee(1);
            let (mut inlet, marshal, reads) = inlet(ctx, &c);
            let block = sample_order(Digest(B256::repeat_byte(0xaa)), 65);
            inlet.ingest(certify(&c, 0, &block)).await;
            assert_eq!(
                *marshal.calls.lock().unwrap(),
                vec!["verified", "report"],
                "valid cert: exactly one verified THEN one report"
            );
            let r = reads.lock().unwrap();
            assert_eq!(r.len(), 1, "one committee read for epoch 0");
            assert_eq!(r[0], 0, "read committee[0] at the finalized tip");
        });
    }

    #[test]
    fn cross_epoch_cert_skips_as_data_fault_without_reading_committee() {
        // Defense-in-depth: a cert whose round-epoch disagrees with its
        // block's height-derived epoch is a malformed / cross-epoch cert — skipped
        // BEFORE committee[E] is ever read, driving the marshal with ZERO calls.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let c = committee(1);
            let (inlet, marshal, reads) = inlet(ctx, &c);
            // activation=0, interval=64 ⇒ epoch_of(65) == 1.
            let mut inlet = inlet.with_epoch_math(0, 64);
            let block = sample_order(Digest(B256::repeat_byte(0xaa)), 65);
            // The cert is itself BLS-valid for epoch 2, but height 65 ∈ epoch 1 ⇒
            // the bind fails and the cert is dropped before verification.
            inlet.ingest(certify(&c, 2, &block)).await;
            assert!(
                marshal.calls.lock().unwrap().is_empty(),
                "cross-epoch cert drives the marshal with ZERO calls"
            );
            assert!(
                reads.lock().unwrap().is_empty(),
                "committee[E] is NOT read for a cross-epoch cert (bind precedes the read)"
            );
        });
    }

    #[test]
    fn matching_epoch_cert_passes_the_height_epoch_bind() {
        // The bind is a no-op for an honest cert (round-epoch ==
        // height-epoch); it proceeds to verify THEN report exactly as without it.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let c = committee(1);
            let (inlet, marshal, _reads) = inlet(ctx, &c);
            // activation=0, interval=64 ⇒ epoch_of(65) == 1; cert epoch 1 matches.
            let mut inlet = inlet.with_epoch_math(0, 64);
            let block = sample_order(Digest(B256::repeat_byte(0xaa)), 65);
            inlet.ingest(certify(&c, 1, &block)).await;
            assert_eq!(
                *marshal.calls.lock().unwrap(),
                vec!["verified", "report"],
                "matching-epoch cert proceeds to verify THEN report"
            );
        });
    }

    /// A committee source that models the follower's frontier-aware read at the
    /// trait boundary: it resolves `committee[E]` only when `max(finalized,
    /// live_frontier) >= committed_from` (epoch 0 — the cold-start epoch — is
    /// always readable). This reproduces what the production `finalized_hash`
    /// closure does after the boundary-wedge fix (read committee at `max(EL-
    /// finalized, live-frontier)` instead of the lagging finalized tip alone). The
    /// inlet's tee advances `live_frontier`, so a verified cert moves the cursor.
    /// Drive a follower across the epoch-0→1 boundary: ingest the epoch-0 cert at
    /// the last block of epoch 0 (height 95) then the epoch-1 boundary cert (height
    /// 96), against a finalized tip frozen at 69 where `committee[1]` is committed
    /// only at tip `>= 70`. Returns the marshal driving calls. With the tee wired,
    /// the epoch-0 cert advances `live_frontier` to 95 so the boundary cert
    /// resolves; without it, `live_frontier` stays 0 and the boundary cert defers.
    async fn run_boundary(
        ctx: deterministic::Context,
        c: &Committee,
        wire_tee: bool,
    ) -> Vec<&'static str> {
        let marshal = FakeMarshal::default();
        let finalized = Arc::new(std::sync::atomic::AtomicU64::new(69));
        let live_frontier = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let committee = {
            let bimap = c.bimap.clone();
            let live = live_frontier.clone();
            let fin = finalized.clone();
            crate::committee::testing::SchemeCommittee::new(move |epoch| {
                // The stand-in for what the module's anchor does on a follower:
                // `committee[E]` for `E >= 1` is only readable once the node's own
                // cursor — here `max(EL-finalized, live frontier)` — reaches the
                // height that commits it. Epoch 0 (the cold-start epoch) always is.
                let cursor = fin
                    .load(std::sync::atomic::Ordering::Relaxed)
                    .max(live.load(std::sync::atomic::Ordering::Relaxed));
                if epoch >= 1 && cursor < 70 {
                    return None;
                }
                Some(build_verifier(
                    &fluent_namespace(CHAIN_ID),
                    bimap.clone(),
                    epoch,
                    None,
                ))
            })
        };
        let mut inlet = CertInlet::new(marshal.clone(), committee, ctx);
        if wire_tee {
            // Dropped receiver ⇒ the DkgActor clock is a benign no-op (the follower
            // has no beacon plane); only `live_height` matters here.
            let (dkg_tx, _dkg_rx) = tokio::sync::mpsc::channel::<u64>(1);
            inlet = inlet.with_tee(super::LiveFrontierTee {
                live_height: live_frontier,
                upstream_frontier: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                dkg_height_tx: dkg_tx,
                plane_clock: crate::sync_metrics::PlaneClock::default(),
            });
        }
        inlet
            .ingest(certify(
                c,
                0,
                &sample_order(Digest(B256::repeat_byte(0xaa)), 95),
            ))
            .await;
        inlet
            .ingest(certify(
                c,
                1,
                &sample_order(Digest(B256::repeat_byte(0xbb)), 96),
            ))
            .await;
        let calls = marshal.calls.lock().unwrap().clone();
        calls
    }

    #[test]
    fn boundary_cert_reads_committee_at_live_frontier_not_lagging_finalized() {
        // The follower epoch-boundary wedge + its fix, deterministically (no flaky
        // docker). The finalized tip lags at 69 (cold-start anchor jitter);
        // committee[1] is ahead-committed only at a tip >= 70. The FIRST cert of
        // epoch 1 (height 96) is the very cert that must advance the executor's
        // finalized tip — a producer↔consumer cycle when the committee read is
        // anchored at that lagging tip.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let c = committee(1);
            // WITH the live-frontier tee: the epoch-0 height-95 cert advances
            // live_frontier to 95, so the boundary cert resolves committee[1] at
            // max(69,95)=95 >= 70 and drives the marshal — the wedge cannot form.
            assert_eq!(
                run_boundary(ctx.clone(), &c, true).await,
                vec!["verified", "report", "verified", "report"],
                "with the tee the boundary cert (96) verifies + drives the marshal"
            );
            // WITHOUT the tee (pre-fix): live_frontier stays 0, so the boundary
            // cert reads committee[1] at max(69,0)=69 < 70 → defers-and-drops →
            // the documented permanent wedge (only the epoch-0 cert ever drove).
            assert_eq!(
                run_boundary(ctx.clone(), &c, false).await,
                vec!["verified", "report"],
                "without the tee the boundary cert defers — the wedge this fix removes"
            );
        });
    }

    #[test]
    fn boundary_cert_defers_when_committee_uncommitted_at_both_anchors() {
        // Safe-degrade: a cert whose committee is committed at NEITHER the
        // finalized tip NOR the live frontier defers non-fatally (no crash, no
        // accept-unverified, no marshal drive) — exactly as before the fix.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let c = committee(1);
            let marshal = FakeMarshal::default();
            let live_frontier = Arc::new(std::sync::atomic::AtomicU64::new(95));
            // Committed at NEITHER anchor: the module never answers for epoch 1.
            let committee_module = {
                let bimap = c.bimap.clone();
                crate::committee::testing::SchemeCommittee::new(move |epoch| {
                    (epoch == 0).then(|| {
                        build_verifier(&fluent_namespace(CHAIN_ID), bimap.clone(), epoch, None)
                    })
                })
            };
            let (dkg_tx, _dkg_rx) = tokio::sync::mpsc::channel::<u64>(1);
            let upstream_frontier = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let mut inlet = CertInlet::new(marshal.clone(), committee_module, ctx).with_tee(
                super::LiveFrontierTee {
                    live_height: live_frontier,
                    upstream_frontier: upstream_frontier.clone(),
                    dkg_height_tx: dkg_tx,
                    plane_clock: crate::sync_metrics::PlaneClock::default(),
                },
            );
            inlet
                .ingest(certify(&c, 1, &sample_order(Digest(B256::repeat_byte(0xbb)), 96)))
                .await
                ;
            assert!(
                marshal.calls.lock().unwrap().is_empty(),
                "an uncommitted-at-both-anchors boundary cert drives the marshal with ZERO calls"
            );
            // ...but a DEFERRED cert STILL advances the re-jump frontier (the
            // deadlock fix: a frozen marshal must not freeze the re-jump trigger).
            assert_eq!(
                upstream_frontier.load(std::sync::atomic::Ordering::Relaxed),
                96,
                "deferred cert advances upstream_frontier so the executor can re-jump out of the wedge"
            );
        });
    }

    #[test]
    fn wrong_signature_cert_skips_with_no_report_and_returns_ok() {
        // A cert formed by a DIFFERENT committee fails BLS against ours: the
        // inlet WARNs + skips + returns Ok (Risk-3), driving the marshal with
        // ZERO calls.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let ours = committee(1);
            let theirs = committee(2);
            let (mut inlet, marshal, _) = inlet(ctx, &ours);
            let block = sample_order(Digest(B256::repeat_byte(0xaa)), 65);
            // Cert is a valid quorum of `theirs`, but our verifier rejects it.
            inlet.ingest(certify(&theirs, 0, &block)).await;
            assert!(
                marshal.calls.lock().unwrap().is_empty(),
                "wrong-sig cert must drive ZERO marshal calls"
            );
        });
    }

    #[test]
    fn tampered_body_cert_skips_with_no_report_and_returns_ok() {
        // The cert signs block A's digest, but the served body is block B
        // (payload != digest) → BLS verify FAILs → skip, no report, Ok.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let c = committee(1);
            let (mut inlet, marshal, _) = inlet(ctx, &c);
            let signed = sample_order(Digest(B256::repeat_byte(0xaa)), 65);
            let mut uf = certify(&c, 0, &signed);
            // Swap in a different body the cert does NOT sign.
            uf.block = sample_order(Digest(B256::repeat_byte(0xab)), 65);
            inlet.ingest(uf).await;
            assert!(
                marshal.calls.lock().unwrap().is_empty(),
                "tampered body must drive ZERO marshal calls"
            );
        });
    }

    /// A committee module that has no record for the epoch yet — the executor
    /// lagging behind the inlet on the first cert of a new epoch.
    fn unready_committee() -> Arc<crate::committee::testing::SchemeCommittee> {
        crate::committee::testing::SchemeCommittee::new(|_| None)
    }

    #[test]
    fn committee_not_yet_committed_skips_non_fatally_without_blocking() {
        // MUST-FIX #4: the first cert of an epoch whose committee[E] is not yet
        // readable at the finalized tip must be SKIPPED (Ok, drive NOTHING) — NOT
        // block-sleep (it would starve the executor it waits on) and NOT return
        // Err (it would shut the node down). A later cert re-triggers once the
        // executor has caught up.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let c = committee(1);
            let marshal = FakeMarshal::default();
            let metric: Family<CommitteeReadDeferLabels, Counter> = Family::default();
            let mut inlet = CertInlet::new(marshal.clone(), unready_committee(), ctx)
                .with_committee_read_deferred_metric(metric.clone());
            let block = sample_order(Digest(B256::repeat_byte(0xaa)), 65);
            inlet.ingest(certify(&c, 0, &block)).await;
            assert!(
                marshal.calls.lock().unwrap().is_empty(),
                "a deferred cert must drive ZERO marshal calls"
            );
            assert_eq!(
                metric
                    .get_or_create(&CommitteeReadDeferLabels {
                        reason: DEFER_COMMITTEE_NOT_COMMITTED,
                    })
                    .get(),
                1,
                "the committee-not-committed defer ticks the counter (the primary defer \
                 regime during a deep backfill must be visible to operators)"
            );
        });
    }

    // Two tests stood here and both lost their subject in this step:
    // `state_not_materialized_defers_non_fatally_and_ticks_counter` and
    // `corrupt_committee_read_stays_fatal`. They asserted the inlet's OWN
    // committee-read taxonomy — a transient reth state-miss defers, a corrupt
    // read is fatal — over a `CommitteeSource` that returned `eyre::Result`. The
    // inlet no longer reads a committee: it asks the committee module for the
    // epoch's scheme, and the module owns both verdicts. The transient half is
    // `CommitteeError::NotReadable`, pinned by
    // `committee::tests::{a_height_below_the_commit_is_refused_without_a_read, …}`;
    // the permanent half is `Read(permanent)` with its own `error!` and
    // `dpos_committee_read_permanent_total`, pinned by the module's own
    // permanent-read tests. What is left on THIS side is the single defer the
    // test above asserts — a cert whose epoch has no scheme yet drives the marshal
    // zero times, non-fatally, and ticks the counter. Re-adding an inlet-side
    // fatal arm would be re-adding the second committee authority.

    #[test]
    fn consecutive_data_faults_rotate_once_lag_does_not_success_resets() {
        // #7: N=MAX_UPSTREAM_FAULTS consecutive BLS-verify failures (a forged
        // upstream over a healthy connection) trigger EXACTLY ONE `rotate()` call;
        // a benign committee-lag `Ok(None)` does NOT count toward rotation; and a
        // successful verify RESETS the streak (so faults must be CONSECUTIVE).
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let ours = committee(1);
            let theirs = committee(2);
            let rotations = Arc::new(std::sync::atomic::AtomicU32::new(0));
            let rotate: RotateUpstream = {
                let rotations = rotations.clone();
                Arc::new(move || {
                    let rotations = rotations.clone();
                    Box::pin(async move {
                        rotations.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }) as BoxFuture<'static, ()>
                })
            };
            let (inlet, _marshal, _) = inlet(ctx, &ours);
            let mut inlet = inlet.with_rotate(rotate);
            let block = sample_order(Digest(B256::repeat_byte(0xaa)), 65);

            // MAX_UPSTREAM_FAULTS-1 wrong-committee certs: a data fault each, but
            // below the threshold ⇒ no rotation yet.
            for _ in 0..MAX_UPSTREAM_FAULTS - 1 {
                inlet.ingest(certify(&theirs, 0, &block)).await;
            }
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "below threshold: no rotation"
            );
            // The Nth consecutive data fault ⇒ exactly one rotation, streak reset.
            inlet.ingest(certify(&theirs, 0, &block)).await;
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "MAX_UPSTREAM_FAULTS consecutive data faults ⇒ exactly one rotate()"
            );

            // After the reset, a fresh streak must climb from zero again — a
            // single more fault does NOT immediately re-rotate.
            inlet.ingest(certify(&theirs, 0, &block)).await;
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "the counter reset after rotating: one post-reset fault must not re-rotate"
            );

            // A SUCCESS resets the streak: 2 faults, then a good cert, then 2 more
            // faults must NOT reach the threshold (no further rotation).
            inlet.ingest(certify(&theirs, 0, &block)).await; // streak now 2
            inlet.ingest(certify(&ours, 0, &block)).await; // success ⇒ reset
            inlet.ingest(certify(&theirs, 0, &block)).await; // streak 1
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "a successful verify resets the consecutive-fault streak (faults must be \
                 consecutive to rotate)"
            );
        });
    }

    #[test]
    fn connection_change_resets_the_per_connection_fault_streak() {
        // #7: the data-fault streak is scoped to the LIVE connection. The WS
        // actor auto-rotates to the next URL on a dropped/failed CONNECTION
        // (bumping the shared `conn_gen`) WITHOUT signalling the inlet, so without
        // per-connection scoping upstream A's faults would carry into upstream B's
        // rotation budget — a premature `rotate()` after fewer than
        // MAX_UPSTREAM_FAULTS B faults. With the token, a connection change resets
        // the streak so A's faults never bleed into B.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let ours = committee(1);
            let theirs = committee(2);
            let rotations = Arc::new(std::sync::atomic::AtomicU32::new(0));
            let rotate: RotateUpstream = {
                let rotations = rotations.clone();
                Arc::new(move || {
                    let rotations = rotations.clone();
                    Box::pin(async move {
                        rotations.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }) as BoxFuture<'static, ()>
                })
            };
            let conn_gen = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let (inlet, _marshal, _) = inlet(ctx, &ours);
            let mut inlet = inlet
                .with_rotate(rotate)
                .with_connection_token(conn_gen.clone());
            let block = sample_order(Digest(B256::repeat_byte(0xaa)), 65);

            // Upstream A serves MAX_UPSTREAM_FAULTS-1 bad certs (below threshold).
            for _ in 0..MAX_UPSTREAM_FAULTS - 1 {
                inlet.ingest(certify(&theirs, 0, &block)).await;
            }
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "A below threshold: no rotation yet"
            );

            // The WS actor's connection-level auto-rotation to upstream B bumps
            // the generation. B then serves ONE bad cert: with per-connection
            // scoping this is B's FIRST fault — below threshold, NO rotation. If
            // A's streak had bled in, this Nth total fault would have rotated.
            conn_gen.fetch_add(1, std::sync::atomic::Ordering::Release);
            inlet.ingest(certify(&theirs, 0, &block)).await;
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "A's fault streak must NOT carry into B's rotation budget after a \
                 connection change"
            );

            // B continues serving faults from a reset streak: it now takes the
            // full MAX_UPSTREAM_FAULTS B-only faults to rotate (already 1 above).
            for _ in 0..MAX_UPSTREAM_FAULTS - 1 {
                inlet.ingest(certify(&theirs, 0, &block)).await;
            }
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "B rotates only after its OWN MAX_UPSTREAM_FAULTS consecutive faults"
            );
        });
    }

    #[test]
    fn committee_lag_never_counts_toward_rotation() {
        // #7 / #4 boundary: a benign committee-not-yet-committed skip (`Ok(None)`)
        // is transient lag, NOT a data fault — it must NEVER count toward rotation
        // even repeated MANY times past the threshold (rotating away from a HEALTHY
        // upstream over normal boundary lag would be a churn footgun).
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let c = committee(1);
            let rotations = Arc::new(std::sync::atomic::AtomicU32::new(0));
            let rotate: RotateUpstream = {
                let rotations = rotations.clone();
                Arc::new(move || {
                    let rotations = rotations.clone();
                    Box::pin(async move {
                        rotations.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }) as BoxFuture<'static, ()>
                })
            };
            let marshal = FakeMarshal::default();
            let mut inlet = CertInlet::new(marshal, unready_committee(), ctx).with_rotate(rotate);
            let block = sample_order(Digest(B256::repeat_byte(0xaa)), 65);
            for _ in 0..MAX_UPSTREAM_FAULTS * 3 {
                inlet.ingest(certify(&c, 0, &block)).await;
            }
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "committee-lag skips must never trigger rotation"
            );
        });
    }

    #[test]
    fn verified_pair_feeds_both_marshal_and_window() {
        // B3: with `with_window` set, a valid cert drives the marshal (verified +
        // report) AND emits the verified pair to the serving window — a
        // marshal-only emit would break the tier-2 cert-cascade. A rejected cert
        // emits to NEITHER.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let c = committee(1);
            let (inlet, marshal, _) = inlet(ctx, &c);
            let (window_tx, mut window_rx) = tokio::sync::mpsc::unbounded_channel();
            let mut inlet = inlet.with_window(window_tx);
            let block = sample_order(Digest(B256::repeat_byte(0xaa)), 65);
            inlet.ingest(certify(&c, 0, &block)).await;
            assert_eq!(
                *marshal.calls.lock().unwrap(),
                vec!["verified", "report"],
                "marshal driven verified THEN report"
            );
            let emitted = window_rx
                .try_recv()
                .expect("window receives the verified pair");
            assert_eq!(
                emitted.block.height, 65,
                "the verified pair reaches the window"
            );
            assert!(window_rx.try_recv().is_err(), "exactly one window emit");

            // A wrong-committee cert: NO marshal call, NO window emit.
            let theirs = committee(2);
            inlet.ingest(certify(&theirs, 0, &block)).await;
            assert!(
                window_rx.try_recv().is_err(),
                "a rejected cert must NOT enter the serving window"
            );
        });
    }

    #[test]
    fn verified_cert_advances_the_live_frontier_tee() {
        // Re-homed tee: a VERIFIED cert advances both beacon-plane cursors from
        // `uf.block.height` (the live upstream frontier); a rejected cert leaves
        // them untouched; the advance is monotone (a lower-height cert does not
        // rewind `live_height`).
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let c = committee(1);
            let (inlet, _marshal, _) = inlet(ctx, &c);
            let live_height = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let (dkg_tx, mut dkg_rx) = tokio::sync::mpsc::channel::<u64>(8);
            let mut inlet = inlet.with_tee(super::LiveFrontierTee {
                live_height: live_height.clone(),
                upstream_frontier: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                dkg_height_tx: dkg_tx,
                plane_clock: crate::sync_metrics::PlaneClock::default(),
            });

            let block = sample_order(Digest(B256::repeat_byte(0xaa)), 65);
            inlet.ingest(certify(&c, 0, &block)).await;
            assert_eq!(
                live_height.load(std::sync::atomic::Ordering::Relaxed),
                65,
                "live_height advances to the verified cert's height"
            );
            assert_eq!(
                dkg_rx.try_recv(),
                Ok(65),
                "dkg clock fed the verified height"
            );

            // A wrong-committee cert at a higher height must NOT advance either.
            let theirs = committee(2);
            let higher = sample_order(Digest(B256::repeat_byte(0xbb)), 99);
            inlet.ingest(certify(&theirs, 0, &higher)).await;
            assert_eq!(
                live_height.load(std::sync::atomic::Ordering::Relaxed),
                65,
                "a rejected cert must NOT advance live_height"
            );
            assert!(
                dkg_rx.try_recv().is_err(),
                "a rejected cert feeds no dkg tick"
            );

            // `fetch_max` is monotone — a lower verified height does not rewind.
            live_height.store(200, std::sync::atomic::Ordering::Relaxed);
            inlet.ingest(certify(&c, 0, &block)).await;
            assert_eq!(
                live_height.load(std::sync::atomic::Ordering::Relaxed),
                200,
                "a lower verified cert does not rewind live_height (fetch_max)"
            );
        });
    }

    #[test]
    fn inflight_guard_deregisters_on_panic_unwind() {
        // FIX #2: the in-flight height is removed even when the fetch task
        // unwinds (a malformed/oversized body panicking in decode/deliver). The
        // RAII `InflightGuard` is the mechanism — a plain trailing `remove` would
        // be skipped on unwind, wedging the resolver on a height it never
        // re-fetches. Drive the guard through a `catch_unwind` to model the panic.
        let inflight: std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<u64>>> =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::BTreeSet::new()));
        inflight.lock().unwrap().insert(42);

        let inflight_for_panic = inflight.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = super::InflightGuard {
                inflight: &inflight_for_panic,
                height: 42,
            };
            panic!("simulated malformed-body decode panic");
        }));
        assert!(result.is_err(), "the closure must have panicked");
        assert!(
            !inflight.lock().unwrap().contains(&42),
            "the in-flight height must be de-registered on panic-unwind (else the \
             resolver wedges on a height it can never re-fetch)"
        );

        // And the ordinary (no-panic) path de-registers exactly the same way.
        inflight.lock().unwrap().insert(7);
        {
            let _guard = super::InflightGuard {
                inflight: &inflight,
                height: 7,
            };
        }
        assert!(
            !inflight.lock().unwrap().contains(&7),
            "the in-flight height must be de-registered on a normal drop too"
        );
    }

    /// A beacon-active committee: `COMMITTEE_N` combined-scheme signers over one
    /// real DKG (share index == commonware participant index), an assembler that
    /// recovers the round seed into the finalization cert, and the encoded DKG
    /// `Output` the epoch's agreement artifact carries. Its group key `PK_epoch`
    /// is exactly what `parse_outcome(outcome_bytes)`/`group_public_key` resolve
    /// to — so the inlet's resolved pin checks seeds against the same key the
    /// signers produce them under.
    struct BeaconFixture {
        /// One entry per member: its BLS keypair and its threshold share. The
        /// SCHEMES are built per epoch by [`BeaconFixture::signers`], because a
        /// scheme is bound to the epoch it was issued for and these fixtures
        /// certify at several.
        members: Vec<(ValidatorBlsKeypair, Share)>,
        sharing: Sharing<MinSig>,
        seed_ns: Vec<u8>,
        outcome_bytes: Vec<u8>,
        bimap: BiMap<PeerPubkey, BlsPubkey>,
        namespace: Vec<u8>,
    }

    impl BeaconFixture {
        fn oracle(&self, share: Option<Share>) -> Arc<dyn SeedOracle> {
            Arc::new(crate::beacon::testing::DealtOracle {
                sharing: self.sharing.clone(),
                share,
                namespace: self.seed_ns.clone(),
            })
        }

        /// Beacon-active signers bound to `epoch`.
        fn signers(&self, epoch: u64) -> Vec<BlsScheme> {
            self.members
                .iter()
                .map(|(kp, share)| {
                    build_signer(
                        &self.namespace,
                        self.bimap.clone(),
                        kp,
                        epoch,
                        Some(self.oracle(Some(share.clone()))),
                    )
                    .expect("member")
                })
                .collect()
        }

        /// The verifier-flavoured assembler (polynomial, no share) that recovers
        /// the round seed into the finalization cert, mirroring how a real
        /// notarization/finalization cert carries one.
        fn assembler(&self, epoch: u64) -> BlsScheme {
            build_verifier(
                &self.namespace,
                self.bimap.clone(),
                epoch,
                Some(self.oracle(None)),
            )
        }
    }

    fn beacon_committee(seed: u64) -> BeaconFixture {
        let mut rng = StdRng::seed_from_u64(seed);
        let peer_sks: Vec<Ed25519PrivateKey> = (0..COMMITTEE_N)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let bls_kps: Vec<ValidatorBlsKeypair> = (0..COMMITTEE_N)
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
        let seed_ns = seed_namespace(&ns);
        // Deal a real committee DKG over the peer set: `deal` indexes shares by the
        // player's position in the commonware-sorted `Set`, which matches the
        // BiMap's participant ordering, so each signer's share index == its vote
        // participant index (asserted inside `build_signer`).
        let players: Set<PeerPubkey> =
            Set::from_iter_dedup(peer_sks.iter().map(|k| k.public_key()));
        let (outcome, share_map) =
            deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
                .expect("deal");
        let sharing = outcome.public().clone();
        let members: Vec<(ValidatorBlsKeypair, Share)> = peer_sks
            .iter()
            .zip(bls_kps)
            .map(|(p, kp)| {
                let share = share_map.get_value(&p.public_key()).expect("share").clone();
                (kp, share)
            })
            .collect();
        BeaconFixture {
            members,
            sharing,
            seed_ns,
            outcome_bytes: encode_outcome(&outcome),
            bimap,
            namespace: ns,
        }
    }

    fn beacon_order(height: u64) -> OrderBlock {
        sample_order(Digest(B256::repeat_byte(0xaa)), height)
    }

    /// A REAL seeded finalization cert: every signer's Finalize carries a threshold
    /// seed partial, and the beacon-active assembler recovers the round seed into
    /// the cert (`certificate.seed = Some(..)`).
    // The seed slot of an admitted certificate is the general source of σ for a
    // node outside `committee[E]` — the by-round transport only covers what this
    // cannot reach. Three outcomes, and the difference between the last two is
    // the whole reason provenance exists: a σ we could not check is HELD, a σ
    // that fails its key is DROPPED, and neither is ever served.
    #[test]
    fn a_certificate_seed_is_captured_under_the_certificates_own_round() {
        let bc = beacon_committee(9);
        let block = sample_order(Digest(B256::repeat_byte(0xcc)), 40);
        let uf = certify_seeded(&bc, 5, &block);
        let round = uf.finalization.proposal.round;

        let known = crate::beacon::testing::Canned::new()
            .with_seed_namespace(bc.seed_ns.clone())
            .with_pin(5, *bc.sharing.public());
        let _ = Beacon::observe_certificate(
            &known,
            ObservedCertificate::Finalization(round, &uf.finalization),
        );
        assert!(
            known.store().lookup(round).is_some(),
            "a checked seed is served"
        );
        assert!(known.store().quarantined_epochs().is_empty());
    }

    #[test]
    fn a_certificate_seed_with_no_resolvable_key_is_held_not_served() {
        let bc = beacon_committee(9);
        let block = sample_order(Digest(B256::repeat_byte(0xcc)), 40);
        let uf = certify_seeded(&bc, 5, &block);
        let round = uf.finalization.proposal.round;

        let keyless = crate::beacon::testing::Canned::new().with_seed_namespace(bc.seed_ns.clone());
        let _ = Beacon::observe_certificate(
            &keyless,
            ObservedCertificate::Finalization(round, &uf.finalization),
        );
        assert_eq!(
            keyless.store().lookup(round),
            None,
            "an unchecked seed never reaches the served map"
        );
        assert_eq!(keyless.store().quarantined_epochs(), vec![5]);
    }

    #[test]
    fn a_certificate_seed_that_fails_its_key_is_neither_served_nor_held() {
        let bc = beacon_committee(9);
        let other = beacon_committee(10);
        let block = sample_order(Digest(B256::repeat_byte(0xcc)), 40);
        let uf = certify_seeded(&bc, 5, &block);
        let round = uf.finalization.proposal.round;

        // The key of a DIFFERENT committee: the σ is a well-formed curve point
        // that verifies under nobody's key here.
        let wrong = crate::beacon::testing::Canned::new()
            .with_seed_namespace(bc.seed_ns.clone())
            .with_pin(5, *other.sharing.public());
        let _ = Beacon::observe_certificate(
            &wrong,
            ObservedCertificate::Finalization(round, &uf.finalization),
        );
        assert_eq!(wrong.store().lookup(round), None);
        assert!(
            wrong.store().quarantined_epochs().is_empty(),
            "a refused seed is not parked for a re-check that can never pass"
        );
    }

    /// A `CertUpstream` that answers one canned finalization for every height.
    #[derive(Clone)]
    struct SeededUpstream(UpstreamFinalized);

    impl crate::cert_follow::CertUpstream for SeededUpstream {
        async fn get_finalization(
            &self,
            _height: commonware_consensus::types::Height,
        ) -> Option<UpstreamFinalized> {
            Some(self.0.clone())
        }
        async fn get_latest(&self) -> Option<UpstreamFinalized> {
            Some(self.0.clone())
        }
        async fn rotate(&self) {}
    }

    // THE PLANE ARM. `CertInlet::ingest` stands up only with WS upstreams
    // configured, while `ValidatorUpstream::Plane` is the default — so this
    // by-height pull is the door a plane-native validator's epoch-E certificates
    // actually come through, and a capture wired only into the inlet would leave
    // the default configuration unserved. Asserted on the STORE and on the
    // certificate's own round, because that key is what makes the capture
    // fork-safe.
    #[test]
    fn the_plane_arms_by_height_pull_captures_the_certificate_seed() {
        let bc = beacon_committee(9);
        let block = sample_order(Digest(B256::repeat_byte(0xdd)), 41);
        let uf = certify_seeded(&bc, 5, &block);
        let round = uf.finalization.proposal.round;
        let known = Arc::new(
            crate::beacon::testing::Canned::new()
                .with_seed_namespace(bc.seed_ns.clone())
                .with_pin(5, *bc.sharing.public()),
        );

        let runtime = commonware_runtime::deterministic::Runner::default();
        let store_probe = known.store().clone();
        let probe_in_task = store_probe.clone();
        runtime.start(|ctx| async move {
            // A stand-in marshal that accepts: the capture now rides the
            // marshal's verdict, so a dropped receiver would mean "rejected".
            let (tx, mut rx) = tokio::sync::mpsc::channel(8);
            use commonware_runtime::{Metrics as _, Spawner as _};
            drop(ctx.with_label("fake_marshal").spawn(move |_| async move {
                while let Some(msg) = rx.recv().await {
                    if let commonware_consensus::marshal::resolver::handler::Message::Deliver {
                        response,
                        ..
                    } = msg
                    {
                        let _ = response.send(true);
                    }
                }
            }));
            let resolver = UpstreamResolver::new(
                ctx.clone(),
                SeededUpstream(uf),
                commonware_consensus::marshal::resolver::handler::Handler::<Digest>::new(tx),
                known.clone(),
            );
            resolver.spawn_finalized(commonware_consensus::types::Height::new(41));
            // The pull runs in its own task; step the virtual clock until it has
            // landed rather than assuming a scheduling order.
            for _ in 0..64 {
                if probe_in_task.lookup(round).is_some() {
                    break;
                }
                commonware_runtime::Clock::sleep(&ctx, std::time::Duration::from_millis(1)).await;
            }
        });
        assert!(
            store_probe.lookup(round).is_some(),
            "the plane arm files σ under the certificate's own round"
        );
    }

    fn certify_seeded(bc: &BeaconFixture, epoch: u64, block: &OrderBlock) -> UpstreamFinalized {
        let round = Round::new(Epoch::new(epoch), View::new(block.height));
        let prop = Proposal::new(round, View::new(block.height - 1), block.digest());
        let finalizes: Vec<_> = bc
            .signers(epoch)
            .iter()
            .map(|s| Finalize::sign(s, prop.clone()).expect("sign"))
            .collect();
        let finalization =
            Finalization::from_finalizes(&bc.assembler(epoch), finalizes.iter(), &Sequential)
                .expect("quorum + recovered seed");
        UpstreamFinalized {
            finalization,
            block: block.clone(),
        }
    }

    /// A committee source that REBUILDS the verifier with the pin the inlet
    /// resolved — exactly what production `RethCommitteeSource::build_at` does
    /// (`build_verifier(ns, bimap, epoch, oracle)`). Records
    /// `(epoch, pin.is_some())` per finalized-tip read, so a test can observe the
    /// pin resolution directly (not only the downstream verify outcome). Unlike
    /// `CannedCommittees` — which discards the pin and is thus blind to it — this
    /// exercises the real ladder → build_verifier → verify_certificate seed-pin
    /// path.
    /// Recorded `(epoch, pin.is_some())` per finalized-tip committee read — the
    /// pin-resolution trail a beacon-aware test asserts against.
    type PinReads = Arc<Mutex<Vec<(u64, bool)>>>;

    type BeaconInlet = CertInlet<deterministic::Context, FakeMarshal>;

    /// A committee module wired the way production wires one: the scheme is built
    /// by ONE verifier over the epoch's record plus `Beacon::oracle_for(epoch)`,
    /// from a beacon handle that arrives AFTER the module (the production build
    /// order — the beacon is constructed from the module's own facade). The test
    /// fills the slot with the same provider it hands the inlet, so what the
    /// scheme is bound to and what the inlet resolves keys through are one object.
    fn beacon_inlet(
        ctx: deterministic::Context,
        bc: &BeaconFixture,
        marshal: FakeMarshal,
    ) -> (BeaconInlet, PinReads, crate::committee::BeaconSlot) {
        let reads: PinReads = Arc::new(Mutex::new(Vec::new()));
        let slot: crate::committee::BeaconSlot = Arc::new(std::sync::OnceLock::new());
        let namespace = bc.namespace.clone();
        let bimap = bc.bimap.clone();
        let recorded = reads.clone();
        let beacon = slot.clone();
        let committee = crate::committee::testing::SchemeCommittee::new(move |epoch| {
            // Unset ⇒ the inlet's own default provider, which is what a test that
            // never wires one gets on the other side too.
            let oracle = beacon
                .get()
                .and_then(std::sync::Weak::upgrade)
                .unwrap_or_else(crate::beacon::absent_unregistered)
                .oracle_for(epoch);
            recorded.lock().unwrap().push((epoch, oracle.is_some()));
            Some(build_verifier(&namespace, bimap.clone(), epoch, oracle))
        });
        let inlet = CertInlet::new(marshal, committee, ctx);
        (inlet, reads, slot)
    }

    /// Hand the SAME provider to the inlet and to the committee module's
    /// verifier, as the production wiring does.
    fn with_beacon(
        inlet: BeaconInlet,
        slot: &crate::committee::BeaconSlot,
        randomness: Arc<dyn Beacon>,
    ) -> BeaconInlet {
        let _ = slot.set(Arc::downgrade(&randomness));
        inlet.with_randomness(randomness)
    }

    /// A provider over canned ladder pieces. The tests keep building the REAL
    /// rungs (`canned_held` is an actual `AgreedKeys`), so what they pin is the
    /// ladder's behaviour, not a stubbed answer.
    fn canned_randomness(
        keys: BeaconKeys,
        held: Option<AgreedKeys>,
    ) -> Arc<dyn crate::beacon::Beacon> {
        crate::beacon::testing::LiveBeacon::build(LiveBeaconConfig {
            seeds: crate::beacon::testing::SeedStore::new(),
            keys,
            resolver: Arc::new(|_| crate::beacon::testing::BeaconResolve::Absent),
            ceremony: Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new())),
            dkg_qual: Arc::new(|_| Some(false)),
            held,
            pull: None,
            metrics: crate::beacon::testing::BeaconMetrics::default(),
            chain_id: CHAIN_ID,
            artifacts: crate::beacon::testing::ArtifactStore::new(),
            geometry: tokio::sync::watch::channel(Some((0, 1))).1,
        })
    }

    /// The ladder's held-artifact rung over a canned store. `at` answers for a
    /// MINTING epoch; `mints` is the chain's `dkgQual` record, which is what says
    /// which epoch that is for the epoch being asked about.
    fn canned_held(
        at: impl Fn(u64) -> Option<GroupPublic> + Send + Sync + 'static,
        mints: &[u64],
    ) -> AgreedKeys {
        let set: std::collections::BTreeSet<u64> = mints.iter().copied().collect();
        let dkg_qual: DkgQualFor = Arc::new(move |e| Some(set.contains(&e)));
        AgreedKeys::new(
            Arc::new(move |epoch: u64| {
                let key = at(epoch);
                Box::pin(async move { key }) as futures::future::BoxFuture<'static, _>
            }) as AgreedKeyAt,
            dkg_qual,
        )
    }

    /// The group key a fixture's ceremony produced — what its seeded certs verify
    /// against, and therefore what the ladder must answer with.
    fn fixture_key(bc: &BeaconFixture) -> GroupPublic {
        *group_public_key(&parse_outcome(&bc.outcome_bytes).expect("outcome"))
    }

    fn count_rotations() -> (Arc<std::sync::atomic::AtomicU32>, RotateUpstream) {
        let rotations = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let rotate: RotateUpstream = {
            let rotations = rotations.clone();
            Arc::new(move || {
                let rotations = rotations.clone();
                Box::pin(async move {
                    rotations.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }) as BoxFuture<'static, ()>
            })
        };
        (rotations, rotate)
    }

    #[test]
    fn boundary_pin_admits_genuine_seed_and_rejects_tampered_seed_as_data_fault() {
        // Bug 2, ingest path: the ladder pins the epoch off its agreed artifact;
        // a later cert whose recovered seed slot is tampered (a valid seed for the
        // WRONG round, on an otherwise-valid multisig quorum) is a DATA FAULT —
        // never archived (marshal), never teed (window) — while the genuine seeded
        // cert for the same epoch verifies and IS processed. If the ingest pin
        // silently resolved to None (the fix inert), the tampered certs would
        // verify vote-only and be archived/teed and never rotate → this test reds.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let bc = beacon_committee(1);
            let pk = fixture_key(&bc);
            let marshal = FakeMarshal::default();
            let (inlet, _reads, slot) = beacon_inlet(ctx, &bc, marshal.clone());
            let (rotations, rotate) = count_rotations();
            let (window_tx, mut window_rx) = tokio::sync::mpsc::unbounded_channel();
            let randomness = canned_randomness(
                BeaconKeys::new(),
                Some(canned_held(move |e| (e == 2).then_some(pk), &[2])),
            );
            let mut inlet = with_beacon(
                inlet.with_rotate(rotate).with_window(window_tx),
                &slot,
                randomness,
            );

            let boundary = certify_seeded(&bc, 2, &beacon_order(64));
            // A valid seed for a foreign round (9, 999) — stands in for a tampered
            // seed slot: a decodable G1 point that will not verify for any test round.
            let wrong = certify_seeded(&bc, 9, &beacon_order(999))
                .finalization
                .certificate
                .seed;

            inlet.ingest(boundary).await;
            inlet
                .ingest(certify_seeded(&bc, 2, &beacon_order(65)))
                .await;
            assert_eq!(
                *marshal.calls.lock().unwrap(),
                vec!["verified", "report", "verified", "report"],
                "the boundary AND the genuine seeded later cert both verify + drive the marshal"
            );

            // Three consecutive seed-tampered (vote-valid) later certs: each is a
            // DATA FAULT rejected by the pinned seed check, so the marshal is driven
            // ZERO more times and the third fault rotates exactly once.
            for h in 66..=68u64 {
                let mut t = certify_seeded(&bc, 2, &beacon_order(h));
                t.finalization.certificate.seed = wrong;
                inlet.ingest(t).await;
            }
            assert_eq!(
                *marshal.calls.lock().unwrap(),
                vec!["verified", "report", "verified", "report"],
                "seed-tampered certs drive the marshal ZERO further times (not archived)"
            );
            assert_eq!(
                window_rx.try_recv().expect("boundary teed").block.height,
                64
            );
            assert_eq!(
                window_rx
                    .try_recv()
                    .expect("genuine cert teed")
                    .block
                    .height,
                65
            );
            assert!(
                window_rx.try_recv().is_err(),
                "no seed-tampered cert reaches the serving window (not teed)"
            );
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "MAX_UPSTREAM_FAULTS seed-tampered certs are data faults ⇒ exactly one rotation"
            );
        });
    }

    #[test]
    fn a_non_change_stretch_pins_from_the_shared_ladder_not_a_forward_cursor() {
        // The stable-stretch property, carried by the ONE ladder rather than by a
        // private forward cursor: the key minted at change-epoch 2 keys epochs 3
        // and 5 too (neither ran an agreement of its own, so no artifact is keyed
        // under them). The difference that matters is WHERE it comes from — the
        // chain's own `dkgQual` record naming the minting epoch, instead of "the
        // greatest key I happen to have seen", which is the unbounded answer that
        // pins a stale pre-rotation key past an unseen change boundary.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let bc = beacon_committee(2);
            let pk2 = fixture_key(&bc);
            let marshal = FakeMarshal::default();
            let (inlet, reads, slot) = beacon_inlet(ctx, &bc, marshal.clone());
            let keys = BeaconKeys::new();
            let mut inlet = with_beacon(
                inlet,
                &slot,
                canned_randomness(
                    keys.clone(),
                    Some(canned_held(move |e| (e == 2).then_some(pk2), &[2])),
                ),
            );

            let wrong = certify_seeded(&bc, 9, &beacon_order(999))
                .finalization
                .certificate
                .seed;

            inlet
                .ingest(certify_seeded(&bc, 2, &beacon_order(64)))
                .await;
            assert_eq!(
                keys.cached_only(2),
                Some(pk2),
                "the ladder's resolve fills the SHARED store"
            );
            assert_eq!(
                keys.attested(2),
                Some(pk2),
                "a quorum signed this key FOR epoch 2, so the attested tier"
            );

            inlet
                .ingest(certify_seeded(&bc, 3, &beacon_order(200)))
                .await;
            inlet
                .ingest(certify_seeded(&bc, 5, &beacon_order(400)))
                .await;

            assert_eq!(
                *reads.lock().unwrap(),
                vec![(2, true), (3, true), (5, true)],
                "the in-force key resolves Some for the minting epoch AND every \
                 later carry-forward epoch"
            );
            assert_eq!(
                marshal.calls.lock().unwrap().len(),
                6,
                "all three certs verify + drive the marshal"
            );

            // A seed-tampered cert for the carried-forward epoch 5 is still rejected
            // — proving the resolved pin is genuinely PK_epoch, not None.
            let mut t = certify_seeded(&bc, 5, &beacon_order(401));
            t.finalization.certificate.seed = wrong;
            inlet.ingest(t).await;
            assert_eq!(
                marshal.calls.lock().unwrap().len(),
                6,
                "the carried-forward pin rejects a seed-tampered epoch-5 cert"
            );
        });
    }

    #[test]
    fn pre_boundary_epoch_is_not_pinned_to_the_new_key() {
        // Epoch keying: a key minted at a change epoch is `PK_E` for that epoch
        // and FORWARD, never backward. An epoch BEFORE the beacon's bootstrap
        // resolves no key (vote-only), so it is NOT verified against the new
        // epoch's key, and a seed-tampered pre-beacon cert is therefore ADMITTED
        // (the documented residual-window degrade). If resolution ever
        // back-applied the new key to an earlier epoch, that cert would be
        // rejected instead.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let bc = beacon_committee(3);
            let pk = fixture_key(&bc);
            let marshal = FakeMarshal::default();
            let (inlet, reads, slot) = beacon_inlet(ctx, &bc, marshal.clone());
            let mut inlet = with_beacon(
                inlet,
                &slot,
                canned_randomness(
                    BeaconKeys::new(),
                    Some(canned_held(move |e| (e == 2).then_some(pk), &[2])),
                ),
            );

            let wrong = certify_seeded(&bc, 9, &beacon_order(999))
                .finalization
                .certificate
                .seed;

            inlet
                .ingest(certify_seeded(&bc, 2, &beacon_order(64)))
                .await;

            // A pre-beacon epoch-1 cert with a tampered seed: epoch 1 predates the
            // bootstrap mint, so it resolves no key, verifies vote-only and drives
            // the marshal.
            let mut pre = certify_seeded(&bc, 1, &beacon_order(30));
            pre.finalization.certificate.seed = wrong;
            inlet.ingest(pre).await;

            assert_eq!(
                *reads.lock().unwrap(),
                vec![(2, true), (1, false)],
                "epoch 1 (pre-bootstrap) resolves NO key"
            );
            assert_eq!(
                *marshal.calls.lock().unwrap(),
                vec!["verified", "report", "verified", "report"],
                "the pre-beacon cert is admitted vote-only (not pinned to the new key)"
            );
        });
    }

    /// TWO independent DKG ceremonies over ONE committee (same peers, vote keys,
    /// bimap, namespace — different group polynomials, so `PK_a != PK_b`): the
    /// change-epoch rotation shape the dropped-boundary-cert poison needs (certs
    /// vote-valid under either fixture; seeds verify only under their own PK).
    fn beacon_committee_pair(seed: u64) -> (BeaconFixture, BeaconFixture) {
        let mut rng = StdRng::seed_from_u64(seed);
        let peer_sks: Vec<Ed25519PrivateKey> = (0..COMMITTEE_N)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let bls_kps: Vec<ValidatorBlsKeypair> = (0..COMMITTEE_N)
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
        let seed_ns = seed_namespace(&ns);
        let players: Set<PeerPubkey> =
            Set::from_iter_dedup(peer_sks.iter().map(|k| k.public_key()));
        let ceremony = |rng: &mut StdRng| {
            let (outcome, share_map) =
                deal::<MinSig, PeerPubkey, N3f1>(rng, Mode::NonZeroCounter, players.clone())
                    .expect("deal");
            let sharing = outcome.public().clone();
            let members: Vec<(ValidatorBlsKeypair, Share)> = peer_sks
                .iter()
                .zip(bls_kps.iter())
                .map(|(p, kp)| {
                    let share = share_map.get_value(&p.public_key()).expect("share").clone();
                    (kp.clone(), share)
                })
                .collect();
            BeaconFixture {
                members,
                sharing,
                seed_ns: seed_ns.clone(),
                outcome_bytes: encode_outcome(&outcome),
                bimap: bimap.clone(),
                namespace: ns.clone(),
            }
        };
        let a = ceremony(&mut rng);
        let b = ceremony(&mut rng);
        (a, b)
    }

    #[test]
    fn boundary_key_source_repins_after_a_dropped_change_boundary_cert() {
        // The scheme-cache poison and both halves of its fix. Change-epoch-2's
        // BOUNDARY cert (the sole live carrier of PK_2) deferred on a transient
        // committee read and was DROPPED, so nothing local has ever seen PK_2 when
        // the SECOND (non-boundary) epoch-2 cert arrives.
        //
        // WITHOUT a source the epoch resolves NO key and the cert is admitted
        // vote-only. That is already a change: the retired forward cursor would
        // have answered PK_1 here — the greatest key it had seen — and that stale
        // pin rejects the PK_2-seeded cert, driving the marshal zero further
        // times. Refusing beats guessing.
        //
        // WITH the source the pin re-resolves PK_2 authoritatively off the
        // marshal-backfilled boundary block, the cert verifies + drives the
        // marshal, and the now-pinned cache entry carries a subsequent epoch-2
        // cert too.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (f1, f2) = beacon_committee_pair(4);
            let pk3 = fixture_key(&f2);

            // WITHOUT the source: no key for epoch 3, so vote-only admission.
            let marshal = FakeMarshal::default();
            // No provider wired on purpose: the inlet AND the module's verifier
            // both fall back to the default absent one.
            let (mut inlet, _reads, _slot) = beacon_inlet(ctx.clone(), &f1, marshal.clone());
            inlet
                .ingest(certify_seeded(&f1, 2, &beacon_order(64)))
                .await;
            inlet
                .ingest(certify_seeded(&f2, 3, &beacon_order(129)))
                .await;
            assert_eq!(
                *marshal.calls.lock().unwrap(),
                vec!["verified", "report", "verified", "report"],
                "epoch 2's key is never carried forward onto epoch 3: the cert is \
                 admitted vote-only rather than rejected against a stale pin"
            );

            // WITH a source resolving PK_3 for epoch 3 (its artifact, arrived
            // late): the same sequence verifies end-to-end.
            let marshal = FakeMarshal::default();
            let (inlet, _reads, slot) = beacon_inlet(ctx, &f1, marshal.clone());
            let mut inlet = with_beacon(
                inlet,
                &slot,
                canned_randomness(
                    BeaconKeys::new(),
                    Some(canned_held(move |e| (e == 3).then_some(pk3), &[3])),
                ),
            );
            inlet
                .ingest(certify_seeded(&f2, 3, &beacon_order(129)))
                .await;
            // The cache entry is now pinned, so the next epoch-2 cert rides it
            // without re-consulting the source, and verifies too.
            inlet
                .ingest(certify_seeded(&f2, 3, &beacon_order(130)))
                .await;
            assert_eq!(
                *marshal.calls.lock().unwrap(),
                vec!["verified", "report", "verified", "report"],
                "with the source the unpinned epoch verifies: re-pinned cert AND \
                 the subsequent cert both drive the marshal"
            );
        });
    }

    /// THE SELF-HEAL, which is what this whole change buys: a cached scheme
    /// starts checking the seed the moment its epoch's key resolves, with no
    /// rebuild, no eviction and no BLS failure in between.
    ///
    /// The state it starts from is the keyless window — epoch 2 is first seen
    /// while `PK_2` is unresolvable everywhere (its artifact has not arrived and
    /// the store is empty), so its certificates are admitted on the multisig half
    /// alone. Before the oracle the exit from that state was a REBUILD, and the
    /// only path that rebuilt an entry was the verify-FAIL eviction — which never
    /// fires for honest traffic, because a keyless scheme is the more permissive
    /// one. The degraded mode therefore outlived the epoch.
    #[test]
    fn a_cached_scheme_starts_checking_the_seed_once_the_key_resolves() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (f1, f2) = beacon_committee_pair(5);
            let pk2 = fixture_key(&f2);
            // Epoch 2's artifact is ABSENT until `arrived` flips.
            let arrived = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let seen = arrived.clone();
            let held = canned_held(
                move |e| (e == 2 && seen.load(std::sync::atomic::Ordering::Relaxed)).then_some(pk2),
                &[2],
            );

            let marshal = FakeMarshal::default();
            let (inlet, reads, slot) = beacon_inlet(ctx, &f1, marshal.clone());
            let mut inlet = with_beacon(inlet, &slot, canned_randomness(BeaconKeys::new(), Some(held)));

            inlet
                .ingest(certify_seeded(&f2, 2, &beacon_order(129)))
                .await
                ;
            inlet
                .ingest(certify_seeded(&f2, 2, &beacon_order(130)))
                .await
                ;
            assert_eq!(
                *reads.lock().unwrap(),
                vec![(2, true)],
                "ONE committee read, and the scheme is built WITH an oracle from the \
                 start — being keyless is a property of the key store, not of the scheme"
            );

            arrived.store(true, std::sync::atomic::Ordering::Relaxed);
            inlet
                .ingest(certify_seeded(&f2, 2, &beacon_order(131)))
                .await
                ;
            assert_eq!(
                *reads.lock().unwrap(),
                vec![(2, true)],
                "and STILL one read: the key arriving rebuilds nothing"
            );
            assert_eq!(
                marshal.calls.lock().unwrap().len(),
                6,
                "all three certs verified — the key arrived without a BLS failure"
            );

            // The seed half is genuinely checked NOW: a seed-tampered epoch-2 cert
            // (a valid seed for a foreign round) is rejected, where the same cached
            // scheme admitted the keyless certs above.
            let wrong = certify_seeded(&f2, 9, &beacon_order(999))
                .finalization
                .certificate
                .seed;
            let mut tampered = certify_seeded(&f2, 2, &beacon_order(132));
            tampered.finalization.certificate.seed = wrong;
            inlet
                .ingest(tampered)
                .await
                ;
            assert_eq!(
                marshal.calls.lock().unwrap().len(),
                6,
                "the epoch key is resolvable, so the seed-tampered cert drives the marshal zero times"
            );
        });
    }

    /// The acquisition is MEMOISED, so the certificate path stays cheap: the
    /// artifact rung answers once, `get_pk` writes the key into the shared store,
    /// and every later certificate of the epoch resolves off that store. The
    /// scheme is built once too — it reads the key live, so a key arriving after
    /// it was built needs no rebuild.
    ///
    /// A rung that answers exactly once is what makes a regression visible: if
    /// anything re-consulted it, the second answer would be empty.
    #[test]
    fn the_artifact_rung_is_consulted_once_and_the_committee_read_once() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (f1, f2) = beacon_committee_pair(6);
            let pk2 = fixture_key(&f2);
            // A rung that answers for epoch 2 exactly ONCE, so a second resolve
            // would come back empty and could only downgrade.
            let consults = Arc::new(std::sync::atomic::AtomicU32::new(0));
            let seen = consults.clone();
            let held = canned_held(
                move |e| {
                    let first = seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0;
                    (e == 2 && first).then_some(pk2)
                },
                &[2],
            );

            let marshal = FakeMarshal::default();
            let (inlet, reads, slot) = beacon_inlet(ctx, &f1, marshal.clone());
            let mut inlet = with_beacon(
                inlet,
                &slot,
                canned_randomness(BeaconKeys::new(), Some(held)),
            );

            inlet
                .ingest(certify_seeded(&f2, 2, &beacon_order(129)))
                .await;
            inlet
                .ingest(certify_seeded(&f2, 2, &beacon_order(130)))
                .await;
            assert_eq!(
                consults.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "once the key is in the store the artifact rung is never re-consulted"
            );
            assert_eq!(
                *reads.lock().unwrap(),
                vec![(2, true)],
                "the entry is not rebuilt, so the committee is read once — and it was \
                 built WITH an oracle, which is what makes the seed half checked at all"
            );

            let wrong = certify_seeded(&f2, 9, &beacon_order(999))
                .finalization
                .certificate
                .seed;
            let mut tampered = certify_seeded(&f2, 2, &beacon_order(131));
            tampered.finalization.certificate.seed = wrong;
            inlet.ingest(tampered).await;
            assert_eq!(
                marshal.calls.lock().unwrap().len(),
                4,
                "the surviving pin still rejects a seed-tampered cert — had the \
                 pin-less ingest replaced the entry, this one would have been admitted"
            );
        });
    }

    // TODO: pin against the real `MarshalActor` — build it via
    // `MarshalActor::init` + `start((rx, NoopResolver), buffer_mailbox)` on the
    // deterministic runtime with NO resolver peers, feed a valid cert through a
    // real `MarshalMailbox`, and assert it dispatches the block to a recording
    // application-`Reporter` (body resolved LOCALLY, `NoopResolver` never fired)
    // and `processed_height` advanced. The real-marshal harness (archives +
    // `buffered::Engine` + `CertProvider` + `Epocher` + application reporter) is
    // heavy; the `MarshalSink` seam above keeps the call-order proof
    // deterministic and cheap. The follower path (`launch_follower`) exercises
    // the real marshal end-to-end under `make smoke-cert-follow`.
}

/// A no-op [`commonware_resolver::Resolver`] for the marshal's resolver
/// channel: the cert-inlet makes every body local via [`MarshalSink::verify_block`]
/// before reporting its cert, so the marshal never needs a peer fetch — the
/// resolver is dead weight that must still satisfy the `start` bound. The
/// near-planeless follower ([`crate::dpos::DposLayer::launch_follower`]) runs the
/// marshal with no resolver peers, handing it this `(rx, NoopResolver)` channel.
/// A genuine no-op: it answers nothing, fetches nothing.
#[derive(Clone)]
pub struct NoopResolver<K, P> {
    _key: std::marker::PhantomData<K>,
    _peer: std::marker::PhantomData<P>,
}

impl<K, P> Default for NoopResolver<K, P> {
    fn default() -> Self {
        Self {
            _key: std::marker::PhantomData,
            _peer: std::marker::PhantomData,
        }
    }
}

impl<K, P> commonware_resolver::Resolver for NoopResolver<K, P>
where
    K: commonware_utils::Span,
    P: commonware_cryptography::PublicKey,
{
    type Key = K;
    type PublicKey = P;

    async fn fetch(&mut self, _key: Self::Key) {}
    async fn fetch_all(&mut self, _keys: Vec<Self::Key>) {}
    async fn fetch_targeted(
        &mut self,
        _key: Self::Key,
        _targets: commonware_utils::vec::NonEmptyVec<Self::PublicKey>,
    ) {
    }
    async fn fetch_all_targeted(
        &mut self,
        _requests: Vec<(
            Self::Key,
            commonware_utils::vec::NonEmptyVec<Self::PublicKey>,
        )>,
    ) {
    }
    async fn cancel(&mut self, _key: Self::Key) {}
    async fn clear(&mut self) {}
    async fn retain(&mut self, _predicate: impl Fn(&Self::Key) -> bool + Send + 'static) {}
}

/// The marshal-`Request` key the follower's resolver serves
/// ([`commonware_consensus::marshal::resolver::handler::Request`] over the
/// `Standard<OrderBlock>` commitment, which IS the ordering [`Digest`]).
type MarshalRequest = commonware_consensus::marshal::resolver::handler::Request<Digest>;

/// An [`commonware_resolver::Resolver`] for the near-planeless follower that
/// backfills the marshal's by-height gap from the cert UPSTREAM instead of from
/// peers (a follower has zero consensus-plane connectivity, so the p2p resolver
/// would find nothing).
///
/// # Why a follower needs a real resolver
///
/// The inlet only ingests the upstream's LIVE finalized stream — the certs the
/// upstream emits going forward from subscribe time. After the cold-start jump
/// the marshal floor sits at `landing − 2K` while the first live cert the inlet
/// sees is at the upstream's CURRENT frontier (≥ `landing`), so there is always
/// a multi-height gap between the floor and the first ingested cert. The marshal
/// dispatches to the executor only CONTIGUOUSLY from `floor + 1`
/// (`try_dispatch_blocks`), so with a [`NoopResolver`] the gap never fills, no
/// block is ever dispatched, and the executor stays idle (the cert-follow wedge).
/// The live stream is also lossy under load (`LIVE_FINALIZED_BUFFER` overflow
/// drops events), and the marshal's own design delegates that recovery to this
/// resolver too.
///
/// # What it serves
///
/// Only [`MarshalRequest::Finalized`] (by height): it pulls
/// `(finalization, block)` from the upstream and delivers the encoded tuple back
/// through the marshal [`Handler`](commonware_consensus::marshal::resolver::handler::Handler),
/// which BLS-verifies it against the per-epoch committee in `verify_delivered`
/// (the trustless gate — a tampered cert never dispatches). `Block`/`Notarized`
/// requests are no-ops: a follower has no block-by-digest pull seam, and those
/// digest gaps fill on their own once the contiguous by-height deliveries land.
///
/// SINGLE-WRITER intact: this only DELIVERS into the marshal — it never touches
/// reth. The executor remains the sole reth writer.
/// RAII de-register for one in-flight [`UpstreamResolver`] fetch height. Removing
/// the height in `Drop` (not as a trailing statement) makes the cleanup
/// panic-safe under `panic=unwind`: if the spawned fetch task panics mid-pull
/// (e.g. decoding a malformed/oversized body), the height is STILL removed from
/// `inflight`, so the marshal can re-request it on its next repair sweep instead
/// of the resolver wedging on a height it will never re-fetch.
struct InflightGuard<'a> {
    inflight: &'a std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<u64>>>,
    height: u64,
}

impl Drop for InflightGuard<'_> {
    fn drop(&mut self) {
        // A poisoned lock (a prior panic WHILE holding it) still must not block
        // de-registration: recover the guard and remove regardless.
        let mut set = self
            .inflight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        set.remove(&self.height);
    }
}

/// The follower's by-height backfill over the cert upstream: the SECOND door
/// into [`crate::beacon::Beacon::observe_certificate`], beside the live-stream
/// inlet above. Which σ each door files, and why only one of them prunes, is on
/// [`Self::spawn_finalized`].
pub struct UpstreamResolver<E, U> {
    ctx: E,
    upstream: U,
    /// Deliver channel into the marshal actor (the `Consumer` side of the
    /// marshal's `resolver_rx`). The follower hands the actor the paired
    /// `mpsc::Receiver`; this `Handler` is how a resolved fetch lands.
    handler: commonware_consensus::marshal::resolver::handler::Handler<Digest>,
    /// In-flight `Finalized` heights, so repeated `fetch_all` repair bursts for
    /// the same gap do not spawn duplicate pulls. The spawned task removes its
    /// own height on completion.
    inflight: std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<u64>>>,
    /// The seed-capture seam for the PLANE arm. This resolver is the door a
    /// plane-native validator's epoch-E certificates actually come through — the
    /// live-stream inlet stands up only with WS upstreams configured — so
    /// capturing only there would leave the default configuration unserved.
    randomness: std::sync::Arc<dyn Beacon>,
}

impl<E, U> Clone for UpstreamResolver<E, U>
where
    E: Clone,
    U: Clone,
{
    fn clone(&self) -> Self {
        Self {
            ctx: self.ctx.clone(),
            upstream: self.upstream.clone(),
            handler: self.handler.clone(),
            inflight: self.inflight.clone(),
            randomness: self.randomness.clone(),
        }
    }
}

impl<E, U> UpstreamResolver<E, U>
where
    E: commonware_runtime::Spawner + commonware_runtime::Metrics + Clone + Send + Sync + 'static,
    U: crate::cert_follow::CertUpstream,
{
    pub fn new(
        ctx: E,
        upstream: U,
        handler: commonware_consensus::marshal::resolver::handler::Handler<Digest>,
        randomness: std::sync::Arc<dyn Beacon>,
    ) -> Self {
        Self {
            ctx,
            upstream,
            handler,
            inflight: std::sync::Arc::new(std::sync::Mutex::new(std::collections::BTreeSet::new())),
            randomness,
        }
    }

    /// Spawn one by-height pull → deliver, deduped by height. The marshal
    /// re-requests on its next repair sweep if the upstream did not (yet) have
    /// the height, so a transient miss self-heals.
    ///
    /// This is where the SECOND door files σ, and why the door exists: there is no
    /// point AFTER the marshal's verification where σ is still visible
    /// (`Update::Tip` does not carry it, and the marshal collapses the oracle's
    /// `Valid`/`NoKey` answer into a bool before returning), so the certificate
    /// goes to the beacon at ingress — the same gate the by-round transport needs
    /// regardless. A pure FOLLOWER runs this too and KEEPS what it files: its
    /// beacon holds its own seed store, and the σ landing here is the only route
    /// by which a follower's executor can derive a beacon-active block's
    /// `prev_randao`.
    ///
    /// Only ONE of the two doors prunes what it files: `observe_cert`, which
    /// carries the retention window, has its single production call site in
    /// [`CertInlet::ingest`] and none here. So a σ repaired through the gap door
    /// stays unpruned until the live stream next delivers a certificate — about a
    /// second later on a following node. A delay, not a leak.
    fn spawn_finalized(&self, height: commonware_consensus::types::Height) {
        let h = height.get();
        if !self.inflight.lock().unwrap().insert(h) {
            return; // already in flight
        }
        let upstream = self.upstream.clone();
        let mut handler = self.handler.clone();
        let inflight = self.inflight.clone();
        let randomness = self.randomness.clone();
        drop(
            self.ctx
                .with_label("upstream_resolver_fetch")
                .spawn(move |_| async move {
                    use commonware_codec::Encode as _;
                    use commonware_resolver::Consumer as _;
                    // RAII de-register: the height is removed from `inflight` on
                    // EVERY exit of this task — completion AND panic-unwind (the
                    // dev profile is panic=unwind; a malformed/oversized ~4 MB body
                    // can panic in `deliver`/decode). A plain trailing `remove`
                    // would be SKIPPED on unwind, leaving the height in `inflight`
                    // forever → every later `spawn_finalized(h)` short-circuits →
                    // the marshal can never re-fetch it → contiguous dispatch
                    // wedges permanently.
                    let _guard = InflightGuard {
                        inflight: &inflight,
                        height: h,
                    };
                    // DELIBERATELY the single-shot pull, not `_everywhere`. This
                    // is the marshal's gap repair: up to `MAX_REPAIR` concurrent
                    // by-height pulls per sweep, roughly one sweep per second. A
                    // walk here would be that many short-lived connections per
                    // second, forever, for a height nobody holds. The escape from
                    // a permanently unservable gap on this path is the re-jump,
                    // not a wider search.
                    if let Some(uf) = upstream.get_finalization(height).await {
                        let round = uf.finalization.proposal.round;
                        let captured = uf.finalization.clone();
                        let key = MarshalRequest::Finalized { height };
                        let value = (uf.finalization, uf.block).encode();
                        // `deliver` routes into the marshal actor, which decodes
                        // + BLS-verifies the cert before storing it; a `false`
                        // return (decode/verify reject) just leaves the height
                        // for the next repair sweep.
                        //
                        // The σ capture rides that verdict rather than preceding
                        // it. σ authenticates itself under `PK_e`, so recording
                        // was never the risk — but QUARANTINE is keyed on a round
                        // the RESPONDER chose, and taking that from an unverified
                        // pull would let one peer name any round it liked. After
                        // `true` the multisig has been checked against
                        // `committee[epoch]`, so the round came from a quorum.
                        if handler.deliver(key, value).await {
                            let _observed = randomness.observe_certificate(
                                ObservedCertificate::Finalization(round, &captured),
                            );
                            // PLAN row 5.2: the synchronous `Refused` becomes this
                            // resolver's data fault (rotate the upstream). Until
                            // then nobody reads the verdict here.
                        }
                    }
                }),
        );
    }
}

/// The follower's marshal resolver: either the real upstream-backed backfill
/// ([`UpstreamResolver`], an upstream is configured) or a [`NoopResolver`] (no
/// upstream — a degenerate follower with only a parked stream). One concrete
/// type so [`crate::OuterEngine::run_follower`] hands the marshal a single
/// `Resolver` regardless of config.
#[derive(Clone)]
pub enum FollowerResolver<E, U> {
    /// Upstream-backed by-height backfill (the production follower path).
    Upstream(UpstreamResolver<E, U>),
    /// No upstream configured — fetches nothing (the gap can never fill; the
    /// follower would wedge, but this is not a reachable production config).
    Noop(NoopResolver<MarshalRequest, commonware_cryptography::ed25519::PublicKey>),
}

impl<E, U> commonware_resolver::Resolver for FollowerResolver<E, U>
where
    E: commonware_runtime::Spawner + commonware_runtime::Metrics + Clone + Send + Sync + 'static,
    U: crate::cert_follow::CertUpstream,
{
    type Key = MarshalRequest;
    type PublicKey = commonware_cryptography::ed25519::PublicKey;

    async fn fetch(&mut self, key: Self::Key) {
        match self {
            Self::Upstream(r) => r.fetch(key).await,
            Self::Noop(r) => r.fetch(key).await,
        }
    }
    async fn fetch_all(&mut self, keys: Vec<Self::Key>) {
        match self {
            Self::Upstream(r) => r.fetch_all(keys).await,
            Self::Noop(r) => r.fetch_all(keys).await,
        }
    }
    async fn fetch_targeted(
        &mut self,
        key: Self::Key,
        targets: commonware_utils::vec::NonEmptyVec<Self::PublicKey>,
    ) {
        match self {
            Self::Upstream(r) => r.fetch_targeted(key, targets).await,
            Self::Noop(r) => r.fetch_targeted(key, targets).await,
        }
    }
    async fn fetch_all_targeted(
        &mut self,
        requests: Vec<(
            Self::Key,
            commonware_utils::vec::NonEmptyVec<Self::PublicKey>,
        )>,
    ) {
        match self {
            Self::Upstream(r) => r.fetch_all_targeted(requests).await,
            Self::Noop(r) => r.fetch_all_targeted(requests).await,
        }
    }
    async fn cancel(&mut self, key: Self::Key) {
        match self {
            Self::Upstream(r) => r.cancel(key).await,
            Self::Noop(r) => r.cancel(key).await,
        }
    }
    async fn clear(&mut self) {
        match self {
            Self::Upstream(r) => r.clear().await,
            Self::Noop(r) => r.clear().await,
        }
    }
    async fn retain(&mut self, predicate: impl Fn(&Self::Key) -> bool + Send + 'static) {
        match self {
            Self::Upstream(r) => r.retain(predicate).await,
            Self::Noop(r) => r.retain(predicate).await,
        }
    }
}

impl<E, U> commonware_resolver::Resolver for UpstreamResolver<E, U>
where
    E: commonware_runtime::Spawner + commonware_runtime::Metrics + Clone + Send + Sync + 'static,
    U: crate::cert_follow::CertUpstream,
{
    type Key = MarshalRequest;
    type PublicKey = commonware_cryptography::ed25519::PublicKey;

    async fn fetch(&mut self, key: Self::Key) {
        if let MarshalRequest::Finalized { height } = key {
            self.spawn_finalized(height);
        }
        // Block/Notarized: no follower pull seam — fill via the by-height path.
    }

    async fn fetch_all(&mut self, keys: Vec<Self::Key>) {
        for key in keys {
            if let MarshalRequest::Finalized { height } = key {
                self.spawn_finalized(height);
            }
        }
    }

    async fn fetch_targeted(
        &mut self,
        key: Self::Key,
        _targets: commonware_utils::vec::NonEmptyVec<Self::PublicKey>,
    ) {
        // The single upstream IS the only target; ignore the peer list.
        if let MarshalRequest::Finalized { height } = key {
            self.spawn_finalized(height);
        }
    }

    async fn fetch_all_targeted(
        &mut self,
        requests: Vec<(
            Self::Key,
            commonware_utils::vec::NonEmptyVec<Self::PublicKey>,
        )>,
    ) {
        for (key, _targets) in requests {
            if let MarshalRequest::Finalized { height } = key {
                self.spawn_finalized(height);
            }
        }
    }

    async fn cancel(&mut self, key: Self::Key) {
        if let MarshalRequest::Finalized { height } = key {
            self.inflight.lock().unwrap().remove(&height.get());
        }
    }

    async fn clear(&mut self) {
        self.inflight.lock().unwrap().clear();
    }

    async fn retain(&mut self, predicate: impl Fn(&Self::Key) -> bool + Send + 'static) {
        self.inflight.lock().unwrap().retain(|&h| {
            predicate(&MarshalRequest::Finalized {
                height: commonware_consensus::types::Height::new(h),
            })
        });
    }
}
