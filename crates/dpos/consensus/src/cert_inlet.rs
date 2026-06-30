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

use crate::{cert_follow::UpstreamFinalized, digest::Digest, scheme::epoch_committee_from_snapshot};
use alloy_consensus::Header;
use alloy_primitives::B256;
use commonware_consensus::simplex::types::Activity;
use commonware_parallel::Sequential;
use eyre::{ensure, eyre};
use fluentbase_bls::{fluent_namespace, scheme::build_verifier, Scheme as BlsScheme};
use fluentbase_staking_reader::RethStakingStateReader;
use futures::future::BoxFuture;
use rand_core::CryptoRngCore;
use reth_ethereum_primitives::EthPrimitives;
use reth_evm::ConfigureEvm;
use reth_storage_api::{HeaderProvider, StateProviderFactory};
use std::{
    collections::{btree_map::Entry, BTreeMap},
    sync::Arc,
};
use tracing::warn;

/// Consecutive DATA faults — an upstream serving cryptographically-unverifiable
/// certs over a HEALTHY connection — before the inlet rotates to the next
/// configured upstream URL ([`RotateUpstream`]).
///
/// A DATA fault is a cert that is STRUCTURALLY served but fails BLS verification
/// against a committee that IS readable (a forged/compromised upstream), or a
/// `payload != digest` structural mismatch. Connection-level failures rotate
/// inside the transport actor on their own; this counter is the ONLY signal a
/// data fault can never surface to that layer (the connection is fine; the
/// PAYLOAD is bad). The benign committee-lag skip (`scheme_at_finalized_tip`
/// returning `Ok(None)`) is NOT a data fault — it is transient boundary lag and
/// rotating away from a healthy upstream over it would be a churn footgun — so
/// it never increments the counter. Any successful verify/ingest resets it to 0.
///
/// Value 3: a single transient hiccup (a momentary cross-epoch race, a one-off
/// re-org served right at a boundary) must not trigger rotation, but a
/// persistently bad upstream is failed-over quickly.
pub const MAX_UPSTREAM_FAULTS: u32 = 3;

/// Boxed rotation callback the inlet calls after [`MAX_UPSTREAM_FAULTS`]
/// CONSECUTIVE data faults — drops the current upstream connection and moves to
/// the next configured URL (the node-side [`crate::cert_follow::CertUpstream::rotate`]
/// wired through a closure). Boxed (mirrors the executor's `ReJump` style) so the
/// inlet does not grow a `U: CertUpstream` generic on its already-wide type
/// parameters; the non-upstream / test inlets default it to `None`.
pub type RotateUpstream = Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;

/// Source of a per-epoch BLS verifier for the inlet — the on-chain committee
/// read ([`RethCommitteeSource::scheme_at`]). Kept as a trait so the unit test
/// can inject a canned committee.
///
/// MULTISIG-ONLY (the verifier ignores the seed now that the PK_E layer is
/// gone), so the source needs no per-epoch beacon key.
pub trait CommitteeSource: Send + Sync + 'static {
    /// Read `committee[epoch]` at a SPECIFIC executed hash. Used by the
    /// cold-start jump ([`crate::cold_start_jump::verify_jump_authenticated`])
    /// and the follower's `cold_start_register`, which both have a
    /// known-committed hash to read at.
    fn scheme_at(&self, epoch: u64, at_hash: B256) -> eyre::Result<BlsScheme>;

    /// Read `committee[epoch]` at the node's CURRENT FINALIZED (committed) tip.
    /// This is the inlet's hot-path read: a committee read MUST use a
    /// guaranteed-committed block, and the finalized tip is exactly that.
    /// Committees are epoch-frozen + content-invariant across any in-epoch
    /// executed hash (MEMORY `epoch-frozen-committee`), so once the finalized
    /// tip passes epoch-(E-1)-start the read is valid.
    ///
    /// `Ok(None)` ⇒ `committee[epoch]` is NOT yet committed at the finalized tip
    /// (transient — the executor, which the inlet feeds, has not drained the
    /// queue that far; the caller SKIPS this cert non-fatally and a later cert of
    /// the same epoch re-triggers the read). `Ok(Some)` ⇒ the verifier. `Err` ⇒ a
    /// real read error (fatal).
    fn scheme_at_finalized_tip(&self, epoch: u64) -> eyre::Result<Option<BlsScheme>>;
}

/// [`CommitteeSource`] over a node's own reth state: committee snapshot at the
/// given executed hash → BLS verifier. The consensus-crate home for the
/// per-epoch verifier read both the cert-inlet (`--cert-follow`/upstream
/// validators) and the cold-start jump ([`crate::cold_start_jump`]) consume.
pub struct RethCommitteeSource<Provider, EvmConfig> {
    reader: RethStakingStateReader<Provider, EvmConfig>,
    namespace: Vec<u8>,
    /// The node's current finalized (committed) tip hash, or `None` while no
    /// block is finalized yet. A closure (not a Provider bound) so the
    /// finalized-tip read does not thread new generics through the source —
    /// built at construction from the local provider clone.
    finalized_hash: Arc<dyn Fn() -> Option<B256> + Send + Sync>,
}

impl<Provider, EvmConfig> RethCommitteeSource<Provider, EvmConfig>
where
    Provider:
        StateProviderFactory + HeaderProvider<Header = Header> + Clone + Send + Sync + 'static,
    EvmConfig: ConfigureEvm<Primitives = EthPrimitives> + Clone + Send + Sync + 'static,
{
    pub fn new(
        reader: RethStakingStateReader<Provider, EvmConfig>,
        chain_id: u64,
        finalized_hash: Arc<dyn Fn() -> Option<B256> + Send + Sync>,
    ) -> Self {
        Self {
            reader,
            namespace: fluent_namespace(chain_id),
            finalized_hash,
        }
    }

    /// Build the MULTISIG-ONLY verifier for `epoch` from the committee snapshot
    /// at `at_hash`. `verify_certificate` ignores the seed now that the PK_E
    /// layer is gone, so the source needs no per-epoch beacon key.
    fn build_at(&self, epoch: u64, at_hash: B256) -> eyre::Result<BlsScheme> {
        let snap = self.reader.epoch_committee_snapshot(epoch, at_hash)?;
        ensure!(
            !snap.validators.is_empty(),
            "epoch {epoch} has no committed committee at {at_hash}"
        );
        let committee = epoch_committee_from_snapshot(&snap)
            .map_err(|e| eyre!("epoch {epoch} committee has non-unique participants: {e:?}"))?;
        Ok(build_verifier(&self.namespace, committee.bimap, None))
    }
}

impl<Provider, EvmConfig> CommitteeSource for RethCommitteeSource<Provider, EvmConfig>
where
    Provider:
        StateProviderFactory + HeaderProvider<Header = Header> + Clone + Send + Sync + 'static,
    EvmConfig: ConfigureEvm<Primitives = EthPrimitives> + Clone + Send + Sync + 'static,
{
    fn scheme_at(&self, epoch: u64, at_hash: B256) -> eyre::Result<BlsScheme> {
        self.build_at(epoch, at_hash)
    }

    fn scheme_at_finalized_tip(&self, epoch: u64) -> eyre::Result<Option<BlsScheme>> {
        let Some(hash) = (self.finalized_hash)() else {
            return Ok(None);
        };
        let snap = self.reader.epoch_committee_snapshot(epoch, hash)?;
        if snap.validators.is_empty() {
            // committee[E] not yet committed at the finalized tip (the executor
            // has not drained the queue to epoch-(E-1)-start) — transient, retry.
            return Ok(None);
        }
        let committee = epoch_committee_from_snapshot(&snap)
            .map_err(|e| eyre!("epoch {epoch} committee has non-unique participants: {e:?}"))?;
        Ok(Some(build_verifier(&self.namespace, committee.bimap, None)))
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
/// `live_height` is the [`crate::beacon::actor::CommitteeFor`] read cursor
/// (`committee_for` reads `committee[E]` at `max(EL-finalized, live_height)`):
/// an upstream-configured validator/newcomer resolves the ahead-committed
/// `committee[E+1]` at the LIVE upstream tip rather than its lagging
/// EL-finalized state (the production-path "Option A" fix). `dkg_height` is the
/// [`crate::beacon::actor::DkgActor`] deal clock: dealing at the live frontier
/// lets a still-catching-up early-joiner deal its first epoch's DKG share before
/// the deal deadline (the vrf-rotation early-join fix) instead of K blocks late.
///
/// A validator-with-upstream wires BOTH cursors (it owns the beacon plane). A
/// FOLLOWER also wires the tee — but only for `live_height` (its frontier-aware
/// committee read: `committee[E]` at `max(EL-finalized, live_height)`, the
/// boundary-wedge fix), with a NO-OP `dkg_height_tx` (the receiver is dropped —
/// the follower has no beacon plane). A no-upstream validator has no inlet at all
/// → both cursors stay finalized-driven, unchanged.
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
}

/// One of two producers into the singleton marshal (the other is the local BFT
/// engine). BLS-verifies each upstream cert against the on-chain committee,
/// makes the body local via [`MarshalSink::verify_block`], then reports the cert.
/// The executor (the sole reth writer) then drives reth identically to a
/// locally-finalized cert.
pub struct CertInlet<C, E, M> {
    marshal: M,
    committees: C,
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
    /// Per-epoch verifier cache, pruned to {prev, cur} on registration.
    schemes: BTreeMap<u64, BlsScheme>,
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
    /// wiring — `epoch_of_block` div-by-zeroes on a zero interval).
    epoch_bind: Option<(u64, u32)>,
}

impl<C, E, M> CertInlet<C, E, M>
where
    C: CommitteeSource,
    E: CryptoRngCore + Send,
    M: MarshalSink,
{
    /// The inlet ALWAYS BLS-verifies upstream certs against the on-chain
    /// committee (no `verify:false` mode exists in v1 — the standalone
    /// `--sequencer-url` trust relay is the separate `launch_consensus_node`
    /// path, not an inlet).
    pub fn new(marshal: M, committees: C, ctx: E) -> Self {
        Self {
            marshal,
            committees,
            tee: None,
            window_tx: None,
            schemes: BTreeMap::new(),
            ctx,
            rotate: None,
            consecutive_faults: 0,
            conn_gen: None,
            last_seen_conn_gen: 0,
            epoch_bind: None,
        }
    }

    /// Attach the activation-relative epoch geometry for the height↔epoch bind
    /// (defense-in-depth; see `epoch_bind`). Builder-style: the FOLLOWER inlet
    /// wires `(dpos_activation_block, epoch_block_interval)`; a validator inlet /
    /// unit test leaves it `None` (the bind is then a no-op). `interval` MUST be
    /// `> 0` (the caller's cold-start guards it).
    pub fn with_epoch_math(mut self, activation: u64, interval: u32) -> Self {
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
    /// On verify-FAIL: WARN + skip + return `Ok` (NOT `Err` — Risk-3: a single
    /// bad upstream cert must not halt the inlet; the marshal stalls naturally
    /// at the gap until a good cert arrives).
    pub async fn ingest(&mut self, uf: UpstreamFinalized) -> eyre::Result<()> {
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
            let height_epoch = fluentbase_staking_reader::reader::epoch_of_block(
                uf.block.height,
                interval,
                activation,
            );
            if height_epoch != epoch {
                warn!(
                    height = uf.block.height,
                    cert_epoch = epoch,
                    height_epoch,
                    "cert-inlet: cert round-epoch != block height-epoch; \
                     skipping (malformed/cross-epoch)"
                );
                self.record_data_fault().await;
                return Ok(());
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
            return Ok(());
        }
        let scheme = match self.schemes.entry(epoch) {
            Entry::Occupied(o) => o.into_mut(),
            Entry::Vacant(v) => {
                // Read committee[E] at the node's CURRENT FINALIZED tip — a
                // GUARANTEED-committed block where committee[E] is committed
                // (ahead-committed at epoch-(E-1)-start). If it is not yet
                // readable there, the executor (which THIS inlet feeds the
                // already-`report()`ed certs) has not drained that far yet.
                //
                // NON-BLOCKING + NON-FATAL by design: `ingest` runs in the
                // SINGLE task that drains the cert source AND feeds the
                // executor that advances the finalized tip. Block-sleeping
                // here would stop draining and starve the very executor we
                // wait on (producer↔consumer cycle); returning `Err` would
                // shut the whole node down on a transient lag. Instead SKIP
                // this first-of-epoch cert and keep draining — the executor
                // catches up in the background, and the NEXT cert of this
                // epoch re-reads committee[E] (now committed) and ingests
                // normally. The marshal stalls naturally at the gap until
                // then, exactly as for any skipped cert (Risk-3). A real read
                // error (`Err`) IS still fatal (committee state is corrupt,
                // not merely lagging).
                match self.committees.scheme_at_finalized_tip(epoch)? {
                    Some(s) => v.insert(s),
                    None => {
                        warn!(
                            height = uf.block.height,
                            epoch,
                            "cert-inlet: committee[E] not yet committed at the finalized tip; \
                             deferring this cert (a later cert re-triggers once the executor \
                             catches up)"
                        );
                        // NOT a data fault: committee-not-yet-committed is
                        // transient boundary lag, NOT an unverifiable cert
                        // (#4). It must not count toward rotation — rotating
                        // away from a HEALTHY upstream over normal lag would be
                        // a churn footgun. Leave `consecutive_faults` untouched
                        // (neither increment nor reset).
                        return Ok(());
                    }
                }
            }
        };
        if !uf.finalization.verify(&mut self.ctx, scheme, &Sequential) {
            warn!(
                height = uf.block.height,
                epoch, "cert-inlet: BLS verify FAILED; skipping (marshal stalls naturally)"
            );
            // DATA FAULT: the cert FAILS BLS against a committee that IS readable
            // (the `scheme` above resolved) — a forged / compromised upstream
            // serving bad payload over a healthy connection. Count it toward
            // rotation (the connection-level failover can NEVER detect this).
            self.record_data_fault().await;
            return Ok(());
        }
        // Verified: a clean ingest — reset the data-fault streak.
        self.consecutive_faults = 0;
        // Retain {prev, cur} only.
        let keep_from = epoch.saturating_sub(1);
        self.schemes.retain(|e, _| *e >= keep_from);
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
            let _ = tee.dkg_height_tx.try_send(uf.block.height);
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
        Ok(())
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
    use crate::order_block::OrderBlock;
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
    };
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;
    use std::sync::{Arc, Mutex};

    const CHAIN_ID: u64 = 20_994;
    const COMMITTEE_N: usize = 4;

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
        let verifier = fluentbase_bls::scheme::build_verifier(&ns, bimap, None);
        Committee { signers, verifier }
    }

    fn sample_order(parent: Digest, height: u64) -> OrderBlock {
        OrderBlock {
            parent,
            height,
            timestamp: 1_700_000_000 + height,
            fee_recipient: Address::ZERO,
            gas_limit: 30_000_000,
            extra_data: Bytes::new(),
            result: B256::ZERO,
            txs: Vec::new(),
            beacon_outcome: None,
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

    /// Canned committee verifier with a read-counter so a test can assert the
    /// finalized-tip read was (or was not) consulted.
    struct CannedCommittees {
        verifier: BlsScheme,
        reads: Arc<Mutex<Vec<u64>>>,
    }

    impl CommitteeSource for CannedCommittees {
        fn scheme_at(&self, _epoch: u64, _at_hash: B256) -> eyre::Result<BlsScheme> {
            Ok(self.verifier.clone())
        }
        fn scheme_at_finalized_tip(&self, epoch: u64) -> eyre::Result<Option<BlsScheme>> {
            self.reads.lock().unwrap().push(epoch);
            Ok(Some(self.verifier.clone()))
        }
    }

    type TestInlet = CertInlet<CannedCommittees, deterministic::Context, FakeMarshal>;
    /// Recorded `epoch` finalized-tip committee reads the canned source observed.
    type SchemeReads = Arc<Mutex<Vec<u64>>>;

    fn inlet(
        ctx: deterministic::Context,
        c: &Committee,
    ) -> (TestInlet, FakeMarshal, SchemeReads) {
        let marshal = FakeMarshal::default();
        let reads = Arc::new(Mutex::new(Vec::new()));
        let inlet = CertInlet::new(
            marshal.clone(),
            CannedCommittees {
                verifier: c.verifier.clone(),
                reads: reads.clone(),
            },
            ctx,
        );
        (inlet, marshal, reads)
    }

    #[test]
    fn valid_cert_verifies_then_reports_in_order() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let c = committee(1);
            let (mut inlet, marshal, reads) = inlet(ctx, &c);
            let block = sample_order(Digest(B256::repeat_byte(0xaa)), 65);
            inlet.ingest(certify(&c, 0, &block)).await.expect("ok");
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
            inlet
                .ingest(certify(&c, 2, &block))
                .await
                .expect("ok (non-fatal skip)");
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
            inlet.ingest(certify(&c, 1, &block)).await.expect("ok");
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
    struct FrontierCommittees {
        verifier: BlsScheme,
        finalized: Arc<std::sync::atomic::AtomicU64>,
        live_frontier: Arc<std::sync::atomic::AtomicU64>,
        /// `committee[E]` for `E >= 1` is committed only at a tip `>=` this height.
        committed_from: u64,
    }

    impl CommitteeSource for FrontierCommittees {
        fn scheme_at(&self, _epoch: u64, _at_hash: B256) -> eyre::Result<BlsScheme> {
            Ok(self.verifier.clone())
        }
        fn scheme_at_finalized_tip(&self, epoch: u64) -> eyre::Result<Option<BlsScheme>> {
            let tip = self
                .finalized
                .load(std::sync::atomic::Ordering::Relaxed)
                .max(self.live_frontier.load(std::sync::atomic::Ordering::Relaxed));
            if epoch == 0 || tip >= self.committed_from {
                Ok(Some(self.verifier.clone()))
            } else {
                Ok(None)
            }
        }
    }

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
        let committees = FrontierCommittees {
            verifier: c.verifier.clone(),
            finalized,
            live_frontier: live_frontier.clone(),
            committed_from: 70,
        };
        let mut inlet = CertInlet::new(marshal.clone(), committees, ctx);
        if wire_tee {
            // Dropped receiver ⇒ the DkgActor clock is a benign no-op (the follower
            // has no beacon plane); only `live_height` matters here.
            let (dkg_tx, _dkg_rx) = tokio::sync::mpsc::channel::<u64>(1);
            inlet = inlet.with_tee(super::LiveFrontierTee {
                live_height: live_frontier,
                upstream_frontier: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                dkg_height_tx: dkg_tx,
            });
        }
        inlet
            .ingest(certify(c, 0, &sample_order(Digest(B256::repeat_byte(0xaa)), 95)))
            .await
            .expect("epoch-0 cert ok");
        inlet
            .ingest(certify(c, 1, &sample_order(Digest(B256::repeat_byte(0xbb)), 96)))
            .await
            .expect("boundary cert ok (non-fatal even when deferred)");
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
            let committees = FrontierCommittees {
                verifier: c.verifier.clone(),
                finalized: Arc::new(std::sync::atomic::AtomicU64::new(69)),
                live_frontier: live_frontier.clone(),
                committed_from: 200,
            };
            let (dkg_tx, _dkg_rx) = tokio::sync::mpsc::channel::<u64>(1);
            let upstream_frontier = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let mut inlet = CertInlet::new(marshal.clone(), committees, ctx).with_tee(
                super::LiveFrontierTee {
                    live_height: live_frontier,
                    upstream_frontier: upstream_frontier.clone(),
                    dkg_height_tx: dkg_tx,
                },
            );
            inlet
                .ingest(certify(&c, 1, &sample_order(Digest(B256::repeat_byte(0xbb)), 96)))
                .await
                .expect("deferred cert is non-fatal Ok");
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
            inlet.ingest(certify(&theirs, 0, &block)).await.expect("Ok, not Err");
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
            inlet.ingest(uf).await.expect("Ok, not Err");
            assert!(
                marshal.calls.lock().unwrap().is_empty(),
                "tampered body must drive ZERO marshal calls"
            );
        });
    }

    /// A committee source whose finalized-tip read is NOT yet committed
    /// (`Ok(None)`) — models the executor lagging behind the inlet on the first
    /// cert of a new epoch.
    struct UnreadyCommittees;
    impl CommitteeSource for UnreadyCommittees {
        fn scheme_at(&self, _epoch: u64, _at_hash: B256) -> eyre::Result<BlsScheme> {
            unreachable!("hot path uses scheme_at_finalized_tip")
        }
        fn scheme_at_finalized_tip(&self, _epoch: u64) -> eyre::Result<Option<BlsScheme>> {
            Ok(None)
        }
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
            let mut inlet =
                CertInlet::new(marshal.clone(), UnreadyCommittees, ctx);
            let block = sample_order(Digest(B256::repeat_byte(0xaa)), 65);
            inlet
                .ingest(certify(&c, 0, &block))
                .await
                .expect("an unreadable committee must be Ok (non-fatal), not Err");
            assert!(
                marshal.calls.lock().unwrap().is_empty(),
                "a deferred cert must drive ZERO marshal calls"
            );
        });
    }

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
                inlet.ingest(certify(&theirs, 0, &block)).await.expect("Ok");
            }
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "below threshold: no rotation"
            );
            // The Nth consecutive data fault ⇒ exactly one rotation, streak reset.
            inlet.ingest(certify(&theirs, 0, &block)).await.expect("Ok");
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "MAX_UPSTREAM_FAULTS consecutive data faults ⇒ exactly one rotate()"
            );

            // After the reset, a fresh streak must climb from zero again — a
            // single more fault does NOT immediately re-rotate.
            inlet.ingest(certify(&theirs, 0, &block)).await.expect("Ok");
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "the counter reset after rotating: one post-reset fault must not re-rotate"
            );

            // A SUCCESS resets the streak: 2 faults, then a good cert, then 2 more
            // faults must NOT reach the threshold (no further rotation).
            inlet.ingest(certify(&theirs, 0, &block)).await.expect("Ok"); // streak now 2
            inlet.ingest(certify(&ours, 0, &block)).await.expect("Ok"); // success ⇒ reset
            inlet.ingest(certify(&theirs, 0, &block)).await.expect("Ok"); // streak 1
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
                inlet.ingest(certify(&theirs, 0, &block)).await.expect("Ok");
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
            inlet.ingest(certify(&theirs, 0, &block)).await.expect("Ok");
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "A's fault streak must NOT carry into B's rotation budget after a \
                 connection change"
            );

            // B continues serving faults from a reset streak: it now takes the
            // full MAX_UPSTREAM_FAULTS B-only faults to rotate (already 1 above).
            for _ in 0..MAX_UPSTREAM_FAULTS - 1 {
                inlet.ingest(certify(&theirs, 0, &block)).await.expect("Ok");
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
            let mut inlet = CertInlet::new(marshal, UnreadyCommittees, ctx).with_rotate(rotate);
            let block = sample_order(Digest(B256::repeat_byte(0xaa)), 65);
            for _ in 0..MAX_UPSTREAM_FAULTS * 3 {
                inlet.ingest(certify(&c, 0, &block)).await.expect("Ok");
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
            inlet.ingest(certify(&c, 0, &block)).await.expect("ok");
            assert_eq!(
                *marshal.calls.lock().unwrap(),
                vec!["verified", "report"],
                "marshal driven verified THEN report"
            );
            let emitted = window_rx.try_recv().expect("window receives the verified pair");
            assert_eq!(emitted.block.height, 65, "the verified pair reaches the window");
            assert!(window_rx.try_recv().is_err(), "exactly one window emit");

            // A wrong-committee cert: NO marshal call, NO window emit.
            let theirs = committee(2);
            inlet.ingest(certify(&theirs, 0, &block)).await.expect("Ok");
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
            });

            let block = sample_order(Digest(B256::repeat_byte(0xaa)), 65);
            inlet.ingest(certify(&c, 0, &block)).await.expect("ok");
            assert_eq!(
                live_height.load(std::sync::atomic::Ordering::Relaxed),
                65,
                "live_height advances to the verified cert's height"
            );
            assert_eq!(dkg_rx.try_recv(), Ok(65), "dkg clock fed the verified height");

            // A wrong-committee cert at a higher height must NOT advance either.
            let theirs = committee(2);
            let higher = sample_order(Digest(B256::repeat_byte(0xbb)), 99);
            inlet.ingest(certify(&theirs, 0, &higher)).await.expect("Ok");
            assert_eq!(
                live_height.load(std::sync::atomic::Ordering::Relaxed),
                65,
                "a rejected cert must NOT advance live_height"
            );
            assert!(dkg_rx.try_recv().is_err(), "a rejected cert feeds no dkg tick");

            // `fetch_max` is monotone — a lower verified height does not rewind.
            live_height.store(200, std::sync::atomic::Ordering::Relaxed);
            inlet.ingest(certify(&c, 0, &block)).await.expect("ok");
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
        _requests: Vec<(Self::Key, commonware_utils::vec::NonEmptyVec<Self::PublicKey>)>,
    ) {
    }
    async fn cancel(&mut self, _key: Self::Key) {}
    async fn clear(&mut self) {}
    async fn retain(&mut self, _predicate: impl Fn(&Self::Key) -> bool + Send + 'static) {}
}

/// The marshal-`Request` key the follower's resolver serves
/// ([`commonware_consensus::marshal::resolver::handler::Request`] over the
/// `Standard<OrderBlock>` commitment, which IS the ordering [`Digest`]).
type MarshalRequest =
    commonware_consensus::marshal::resolver::handler::Request<Digest>;

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
    ) -> Self {
        Self {
            ctx,
            upstream,
            handler,
            inflight: std::sync::Arc::new(std::sync::Mutex::new(std::collections::BTreeSet::new())),
        }
    }

    /// Spawn one by-height pull → deliver, deduped by height. The marshal
    /// re-requests on its next repair sweep if the upstream did not (yet) have
    /// the height, so a transient miss self-heals.
    fn spawn_finalized(&self, height: commonware_consensus::types::Height) {
        let h = height.get();
        if !self.inflight.lock().unwrap().insert(h) {
            return; // already in flight
        }
        let upstream = self.upstream.clone();
        let mut handler = self.handler.clone();
        let inflight = self.inflight.clone();
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
                    if let Some(uf) = upstream.get_finalization(height).await {
                        let key = MarshalRequest::Finalized { height };
                        let value = (uf.finalization, uf.block).encode();
                        // `deliver` routes into the marshal actor, which decodes
                        // + BLS-verifies the cert before storing it; a `false`
                        // return (decode/verify reject) just leaves the height
                        // for the next repair sweep.
                        let _ = handler.deliver(key, value).await;
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
        requests: Vec<(Self::Key, commonware_utils::vec::NonEmptyVec<Self::PublicKey>)>,
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
        requests: Vec<(Self::Key, commonware_utils::vec::NonEmptyVec<Self::PublicKey>)>,
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
