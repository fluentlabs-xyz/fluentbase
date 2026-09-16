//! Cert-inlet: a second producer into the singleton marshal.
//!
//! It BLS-verifies an upstream `(Finalization, OrderBlock)` against the on-chain
//! committee, makes the body local via `verified()`, then `report()`s the cert,
//! driving the marshal and, through it, the executor (the sole reth writer)
//! exactly as a locally-formed finalization would. The inlet itself writes
//! nothing to reth. It is the sole producer for a non-validator follower and a
//! second producer next to the local BFT engine on an upstream-configured
//! validator.

use crate::{
    beacon::{Beacon, Observed, ObservedCertificate, PinEffort},
    cert_follow::UpstreamFinalized,
    committee::Committee,
    digest::Digest,
};
use alloy_primitives::B256;
use commonware_consensus::simplex::types::Activity;
use commonware_parallel::Sequential;
use commonware_runtime::Handle;
use eyre::{ensure, eyre};
use fluentbase_bls::{
    fluent_namespace, oracle::SeedOracle, scheme::build_verifier, Scheme as BlsScheme,
};
use futures::future::BoxFuture;
use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{counter::Counter, family::Family},
};
use rand_core::CryptoRngCore;
use std::sync::Arc;
use tracing::warn;

/// `{reason=...}` label set of the `dpos_cert_inlet_committee_read_deferred_total`
/// counter. Kept a label family so the series keeps its shape.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct CommitteeReadDeferLabels {
    pub reason: &'static str,
}

/// `reason` value for a cert deferred because the epoch's committee is not yet
/// committed at this node's anchor.
pub const DEFER_COMMITTEE_NOT_COMMITTED: &str = "committee_not_committed";

/// Consecutive data faults — an upstream serving cryptographically unverifiable
/// certs over a healthy connection — before the inlet rotates to the next
/// configured upstream URL ([`RotateUpstream`]).
///
/// A data fault is a cert that is structurally served but fails BLS verification
/// against a committee that is readable, or a `payload != digest` structural
/// mismatch. Connection-level failures rotate inside the transport actor on their
/// own, which is why this counter exists. The benign committee-lag skip is
/// transient boundary lag, not a data fault, so it neither increments nor resets
/// the streak; any successful verify resets it to 0.
///
/// Three: a single transient hiccup must not trigger rotation, but a persistently
/// bad upstream is failed over quickly.
pub const MAX_UPSTREAM_FAULTS: u32 = 3;

/// Certificates admitted on the multisig quorum alone at a beacon-active epoch
/// whose scheme carries no seed pin. A count that keeps climbing means the
/// epoch's `PK_epoch` never arrived, so those seed slots went unchecked.
pub const CERT_VOTE_ONLY_ADMISSIONS: &str = "dpos_cert_vote_only_admissions_total";

/// Boxed callback the inlet calls after [`MAX_UPSTREAM_FAULTS`] consecutive data
/// faults: drop the current upstream connection and move to the next configured
/// URL. Boxed so the inlet does not grow a `U: CertUpstream` generic; non-upstream
/// and test inlets default it to `None`.
pub type RotateUpstream = Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;

/// Source of a per-epoch BLS verifier at a specific executed hash — the
/// cold-start jump's committee read. A trait so the unit test can inject a canned
/// committee.
///
/// `oracle` is the beacon's threshold face for the epoch
/// ([`crate::beacon::Beacon::oracle_for`]): `Some` makes the built verifier check
/// the recovered seed slot and refuse a stripped one, `None` marks a pre-beacon
/// epoch where a seedless certificate is legal. The verifier read itself is
/// committee-only; the oracle comes from the caller, never from chain storage.
pub trait CommitteeSource: Send + Sync + 'static {
    /// Read `committee[epoch]` at a specific executed hash. Used by the two
    /// by-height seams that fetch one finalization outside both writers
    /// ([`crate::cold_start_jump::verify_jump_authenticated`]).
    fn scheme_at(
        &self,
        epoch: u64,
        at_hash: B256,
        oracle: Option<Arc<dyn SeedOracle>>,
    ) -> eyre::Result<BlsScheme>;
}

/// [`CommitteeSource`] over the committee module: the verifier is built from the
/// module's frozen record, so every by-height seam reads the committee the rest
/// of the process reads.
///
/// `at_hash` must be the module's own anchor hash ([`Committee::anchor_hash`]);
/// any other hash is refused, so this adapter cannot be used to read the contract
/// at an arbitrary block. The scheme is built here rather than taken from
/// [`Committee::scheme`] because the caller chooses the oracle: the seams pass
/// `None` for a vote-only verify, while the module's scheme carries the beacon's.
pub struct ModuleCommitteeSource {
    committee: Arc<dyn Committee>,
    namespace: Vec<u8>,
}

impl ModuleCommitteeSource {
    pub fn new(committee: Arc<dyn Committee>, chain_id: u64) -> Self {
        Self {
            committee,
            namespace: fluent_namespace(chain_id),
        }
    }
}

impl CommitteeSource for ModuleCommitteeSource {
    fn scheme_at(
        &self,
        epoch: u64,
        at_hash: B256,
        oracle: Option<Arc<dyn SeedOracle>>,
    ) -> eyre::Result<BlsScheme> {
        let anchor = self
            .committee
            .anchor_hash()
            .ok_or_else(|| eyre!("the committee module's anchor is not executed yet"))?;
        ensure!(
            at_hash == anchor,
            "committee[{epoch}] asked at {at_hash}, but the module reads only at its own \
             anchor {anchor}"
        );
        let record = self.committee.committee(epoch)?;
        Ok(build_verifier(
            &self.namespace,
            record.bls.bimap.clone(),
            epoch,
            oracle,
        ))
    }
}

/// The marshal-facing sink the inlet drives: make a body local
/// ([`Self::verify_block`]) then report its finalization
/// ([`Self::report_finalization`]).
pub trait MarshalSink: Send {
    /// Persist a verified body so the marshal resolves it without a peer fetch.
    fn verify_block(
        &mut self,
        round: commonware_consensus::types::Round,
        block: crate::order_block::OrderBlock,
    ) -> impl std::future::Future<Output = ()> + Send;

    /// Report a finalization certificate; drives storage and the executor.
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
        self.verified(round, block).await
    }

    async fn report_finalization(
        &mut self,
        finalization: commonware_consensus::simplex::types::Finalization<BlsScheme, Digest>,
    ) {
        use commonware_consensus::Reporter as _;
        // `Reporter::report` takes a `simplex::types::Activity`; the marshal routes
        // `Finalization` to its in-order path and ignores other variants.
        self.report(Activity::Finalization(finalization)).await;
    }
}

/// One of two producers into the singleton marshal (the other is the local BFT
/// engine). BLS-verifies each upstream cert against the on-chain committee, makes
/// the body local, then reports the cert.
pub struct CertInlet<E, M> {
    marshal: M,
    /// Every per-epoch scheme this inlet verifies with, from the one map that
    /// holds a committee record and its scheme in the same slot. An entry is
    /// write-once inside the module's window, and outside it there is no entry.
    committee: Arc<dyn crate::committee::Committee>,
    /// Second sink for each verified pair: the node's `consensus`-RPC serving
    /// window, which a tier-2 follower aligns by. Only BLS-verified pairs enter
    /// (the emit is after the verify gate). `None` on a validator, which serves
    /// from the marshal.
    window_tx: Option<tokio::sync::mpsc::UnboundedSender<UpstreamFinalized>>,
    /// The node's shared beacon handle. The inlet asks it to make the epoch's key
    /// resolvable ([`Beacon::ensure_key`]) and to judge the certificate's sigma
    /// ([`Beacon::observe_certificate`]).
    ///
    /// A default-constructed store on an inlet nobody wired one into is a private
    /// empty one — the unit-test shape.
    randomness: Arc<dyn Beacon>,
    /// commonware ctx (the `CryptoRngCore` source the cert `verify()` needs).
    ctx: E,
    /// Data-fault upstream-rotation trigger. `Some` on an upstream-configured
    /// inlet; `None` for tests. After [`MAX_UPSTREAM_FAULTS`] consecutive data
    /// faults `ingest` invokes it and resets the counter.
    rotate: Option<RotateUpstream>,
    /// Consecutive data faults since the last successful verify. Reset on a
    /// genuine ingest and after a rotation; a committee-lag skip leaves it
    /// untouched. Only a BLS-verify failure or structural mismatch against a
    /// readable committee increments it.
    consecutive_faults: u32,
    /// Per-connection fault scoping. The WS upstream actor auto-rotates to the
    /// next URL on a connection-level failure without signalling the inlet, so
    /// without this a data-fault streak from upstream A would carry into B's
    /// budget and fire a premature rotation. The actor bumps a shared generation
    /// on each (re)connect; the inlet resets the streak when the token changes.
    /// `Some` on an upstream-configured inlet; `None` for tests and the
    /// no-upstream inlet, whose streak is then inlet-global.
    conn_gen: Option<Arc<std::sync::atomic::AtomicU64>>,
    last_seen_conn_gen: u64,
    /// Activation-relative epoch geometry `(dpos_activation_block,
    /// epoch_block_interval)` for the height/epoch bind in `ingest`. `Some` on the
    /// follower inlet; `None` on a validator inlet (which re-derives and
    /// cross-checks on its consensus plane) and in tests. `interval` must be
    /// `> 0`; `ingest` treats a zero interval as a height matching no epoch.
    epoch_bind: Option<(u64, u64)>,
    /// `dpos_cert_inlet_committee_read_deferred_total{reason}`: ticks once per cert
    /// deferred because the epoch's committee is not yet readable. Default
    /// (unregistered) on inlets not wired via
    /// [`Self::with_committee_read_deferred_metric`], where the increment is a
    /// no-op on a local family.
    committee_read_deferred: Family<CommitteeReadDeferLabels, Counter>,
    /// `dpos_cert_inlet_carry_forward_pin_verify_failed_total`: ticks once per
    /// BLS-verify failure of a cert whose scheme was built this ingest with a
    /// derived seed pin (from the shared store or an earlier epoch's boundary
    /// block) rather than one this block asserted. Separates that regime from
    /// forged-upstream data faults on dashboards.
    carry_forward_verify_failed: Counter,
    /// The late-verdict consumer: sigma admitted while this node had no `PK_E`,
    /// refused once the key landed. The inlet is the only consumer, because the
    /// only action a late `Refused` has is to rotate away from the upstream that
    /// served it.
    ///
    /// Taken in [`Self::with_randomness`] rather than through a builder of its own,
    /// so the receiver is handed out once by `Beacon::faults` and a node with no
    /// inlet arms nothing.
    ///
    /// Drained at the head of [`Self::ingest`]; this type has no `select!` loop of
    /// its own.
    faults: Option<tokio::sync::mpsc::UnboundedReceiver<crate::beacon::DataFault>>,
    /// Once-per-episode rate limit for the committee-not-readable warn, cleared on
    /// the next clean verified ingest.
    committee_not_committed_warned: bool,
}

impl<E, M> CertInlet<E, M>
where
    E: CryptoRngCore + Send,
    M: MarshalSink,
{
    /// The inlet always BLS-verifies upstream certs against the on-chain committee
    /// (no `verify:false` mode exists).
    pub fn new(marshal: M, committee: Arc<dyn crate::committee::Committee>, ctx: E) -> Self {
        Self {
            marshal,
            committee,
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
            faults: None,
            committee_not_committed_warned: false,
        }
    }

    /// Attach the registered `dpos_cert_inlet_committee_read_deferred_total` family.
    /// A validator inlet or unit test leaves the default unregistered family.
    pub fn with_committee_read_deferred_metric(
        mut self,
        metric: Family<CommitteeReadDeferLabels, Counter>,
    ) -> Self {
        self.committee_read_deferred = metric;
        self
    }

    /// Attach the registered carry-forward-pin verify-failure counter.
    pub fn with_carry_forward_fail_metric(mut self, metric: Counter) -> Self {
        self.carry_forward_verify_failed = metric;
        self
    }

    /// Attach the randomness provider, assembled at the launch site.
    pub fn with_randomness(mut self, randomness: Arc<dyn Beacon>) -> Self {
        self.faults = randomness.faults();
        self.randomness = randomness;
        self
    }

    /// Attach the activation-relative epoch geometry for the height/epoch bind.
    /// `interval` must be `> 0`; the caller's cold-start guards it.
    pub fn with_epoch_math(mut self, activation: u64, interval: u64) -> Self {
        self.epoch_bind = Some((activation, interval));
        self
    }

    /// Attach the data-fault upstream-rotation trigger. Without it a persistently
    /// bad upstream is still skipped non-fatally, but never fails over.
    pub fn with_rotate(mut self, rotate: RotateUpstream) -> Self {
        self.rotate = Some(rotate);
        self
    }

    /// Attach the per-connection fault-scoping token (see [`Self::conn_gen`]).
    /// Observes the initial generation eagerly so the first connection's faults
    /// are not reset against a stale `0`.
    pub fn with_connection_token(mut self, conn_gen: Arc<std::sync::atomic::AtomicU64>) -> Self {
        self.last_seen_conn_gen = conn_gen.load(std::sync::atomic::Ordering::Acquire);
        self.conn_gen = Some(conn_gen);
        self
    }

    /// Attach the serving-window sink.
    pub fn with_window(
        mut self,
        window_tx: tokio::sync::mpsc::UnboundedSender<UpstreamFinalized>,
    ) -> Self {
        self.window_tx = Some(window_tx);
        self
    }

    /// BLS-verify one upstream cert, then drive the marshal with it.
    ///
    /// Infallible by contract: every outcome is a skip — a malformed or
    /// cross-epoch cert, a verify failure, an epoch whose committee this node
    /// cannot read yet. A single bad upstream cert must not halt the inlet; the
    /// marshal stalls naturally at the gap until a good cert arrives.
    pub async fn ingest(&mut self, uf: UpstreamFinalized) {
        if let Some(conn_gen) = &self.conn_gen {
            let gen = conn_gen.load(std::sync::atomic::Ordering::Acquire);
            if gen != self.last_seen_conn_gen {
                self.last_seen_conn_gen = gen;
                self.consecutive_faults = 0;
            }
        }
        // Charge late verdicts before judging this certificate, so a charge made
        // here outlives it.
        let late_charges = self.drain_late_verdicts().await;
        let round = uf.finalization.proposal.round;
        let epoch = round.epoch().get();
        // Height/epoch bind: an honest cert satisfies `round.epoch() ==
        // epoch_of(block.height)` because the per-epoch engine proposes only its
        // own height range. A mismatch is a malformed or cross-epoch cert, skipped
        // as a data fault before selecting `committee[E]`'s scheme.
        if let Some((activation, interval)) = self.epoch_bind {
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
        // BLS verify alone proves a quorum signed `proposal.payload`, not that the
        // served body matches it; a swapped body under a valid cert is a data fault.
        let digest = uf.block.digest();
        if uf.finalization.proposal.payload != digest {
            warn!(
                height = uf.block.height,
                epoch, "cert-inlet: cert payload != block digest; skipping (tampered/mismatched)"
            );
            self.record_data_fault().await;
            return;
        }
        // Acquisition, and load-bearing: the scheme's oracle answers `verify_seed`
        // from a sync resolve alone, so on an epoch whose artifact has not reached
        // this node this call is the only thing that asks for it.
        //
        // `Local` and never a network rung: ingress runs against a ~1 s verify
        // budget, and a pull would move a peer round-trip onto the vote path.
        let key_known = self.randomness.ensure_key(epoch, PinEffort::Local).await;
        // The epoch's scheme from the committee module's map. `None` is the one
        // deferral: `ingest` runs in the single task that drains the cert source
        // and feeds the executor that advances the anchor the module reads at, so
        // blocking or failing here would starve the progress it waits on.
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
            return;
        };
        // The only direct witness that an epoch is admitted without its seed
        // checked: the multisig quorum is verified, the seed slot is not because
        // the oracle answers `NoKey`.
        if !key_known && self.randomness.mandatory_at(epoch) {
            metrics::counter!(CERT_VOTE_ONLY_ADMISSIONS).increment(1);
        }
        if !uf.finalization.verify(&mut self.ctx, &scheme, &Sequential) {
            warn!(
                height = uf.block.height,
                epoch, "cert-inlet: BLS verify FAILED; skipping (marshal stalls naturally)"
            );
            // A verify failure while the key was resolvable points at a key carried
            // forward from the wrong mint rather than a forged upstream, so it is
            // counted separately; a failure with no key resolvable never checked the
            // seed half at all.
            if key_known {
                self.carry_forward_verify_failed.inc();
            }
            self.record_data_fault().await;
            return;
        }
        // Capture sigma from the verified certificate rather than from transport
        // alone: a node following epoch E from outside `committee[E]` sees a
        // verified sigma for every finalized round of E. Keyed by the certificate's
        // own round, which makes the capture fork-safe — sigma signs
        // `seed_message(round)`, so a round this node will not ask for can only sit
        // unused.
        //
        // The synchronous verdict. `Refused` means this certificate's sigma does not
        // verify under a key a `committee[minted_at]` quorum attested — a statement
        // about the sender, so a data fault and a skip.
        //
        // `Pending` is not a fault: the multisig quorum was checked and the sigma is
        // held for the key to settle.
        if self
            .randomness
            .observe_certificate(ObservedCertificate::Finalization(round, &uf.finalization))
            == Observed::Refused
        {
            warn!(
                height = uf.block.height,
                epoch,
                %round,
                "cert-inlet: the certificate's σ is REFUSED under this epoch's attested key; \
                 skipping and counting a data fault"
            );
            self.record_data_fault().await;
            return;
        }

        // A clean ingest: clear the data-fault streak, unless this certificate
        // carried late charges, which must survive it, and end any open defer-warn
        // episode.
        if late_charges == 0 {
            self.consecutive_faults = 0;
        }
        self.committee_not_committed_warned = false;
        // Make the body local before reporting the cert. With a window present,
        // clone the body once (for the marshal) and the finalization once (small),
        // then move both originals into the window; without one, move both into the
        // marshal and the report.
        match self.window_tx.clone() {
            Some(tx) => {
                self.marshal.verify_block(round, uf.block.clone()).await;
                // A dropped window receiver (RPC shutting down) is benign; the
                // marshal sink below is the load-bearing delivery.
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

    /// Take every late `Refused` the beacon has filed and charge it to the
    /// upstream's streak, capped at the rotation threshold per message; each refused
    /// round is one bad certificate.
    ///
    /// Returns how many faults it charged so [`Self::ingest`] does not erase them
    /// with the very certificate they arrived on. A synchronous fault survives its
    /// own certificate by returning before the reset; a late one is drained at the
    /// top of the same call, so without this count the reset at the bottom wiped
    /// every sub-threshold charge.
    async fn drain_late_verdicts(&mut self) -> usize {
        let mut charges = 0usize;
        if let Some(faults) = self.faults.as_mut() {
            while let Ok(fault) = faults.try_recv() {
                warn!(
                    epoch = fault.epoch,
                    refused = fault.refused,
                    "cert-inlet: the beacon refused σ this upstream served once the epoch key \
                     landed; counting it toward rotation"
                );
                charges += fault.refused.min(MAX_UPSTREAM_FAULTS as usize);
            }
        }
        for _ in 0..charges {
            self.record_data_fault().await;
        }
        charges
    }

    /// Record one data fault and, once [`MAX_UPSTREAM_FAULTS`] consecutive faults
    /// accumulate, fire the upstream-rotation trigger and reset the streak. With no
    /// trigger configured it just counts.
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
            // Reset whether or not a trigger fired, so the streak stays bounded.
            self.consecutive_faults = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        beacon::testing::{
            encode_outcome, group_public_key, parse_outcome, LiveBeaconConfig, MintFixture,
        },
        order_block::OrderBlock,
    };
    use alloy_primitives::Bytes;
    use commonware_codec::DecodeExt as _;
    use commonware_consensus::{
        simplex::types::{Finalization, Finalize, Proposal},
        types::{Epoch, Height, Round, View},
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

    /// A plain (seedless) committee. Holds keypairs, not schemes, because a scheme
    /// is bound to the epoch it was issued for.
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

    /// 2f+1 finalize votes over the block's digest — a real finalization cert.
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

    /// Records the marshal calls in order so a test can assert the inlet verifies
    /// before it reports, and that a rejected cert drives nothing.
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

    /// The module adapter builds the verifier from the module's record and refuses
    /// any hash but the module's own anchor, so no seam can read the contract at a
    /// hash of its choosing through it.
    #[test]
    fn the_module_adapter_answers_at_the_anchor_and_refuses_any_other_hash() {
        use crate::committee::{CommitteeRecord, Member};
        use commonware_utils::TryFromIterator as _;

        let c = committee(41);
        let epoch = 7u64;
        let record = CommitteeRecord {
            epoch,
            members: c
                .bimap
                .iter_pairs()
                .map(|(peer, bls)| Member {
                    address: alloy_primitives::Address::ZERO,
                    peer: peer.clone(),
                    bls: *bls,
                })
                .collect(),
            weights: vec![1u128; COMMITTEE_N],
            changed: false,
            snapshot: (0, B256::ZERO),
            participants: Set::try_from_iter(c.bimap.iter().cloned()).unwrap(),
            bls: fluentbase_bls::scheme::EpochCommittee {
                epoch,
                bimap: c.bimap.clone(),
            },
        };
        let module = crate::committee::testing::SchemeCommittee::with_geometry(
            |_| None,
            move |_| Some(record.clone()),
            None,
        );
        let anchor = B256::repeat_byte(0xA7);
        let elsewhere = B256::repeat_byte(0xB8);
        let adapter = ModuleCommitteeSource::new(module.clone(), CHAIN_ID);

        let unexecuted = adapter
            .scheme_at(epoch, anchor, None)
            .expect_err("no anchor hash yet, so nothing can be read");
        assert!(
            format!("{unexecuted:#}").contains("anchor is not executed"),
            "{unexecuted:#}"
        );

        module.set_anchor_hash(Some(anchor));
        let refused = adapter
            .scheme_at(epoch, elsewhere, None)
            .expect_err("a hash other than the anchor must be refused");
        assert!(
            format!("{refused:#}").contains("reads only at its own anchor"),
            "{refused:#}"
        );

        let scheme = adapter
            .scheme_at(epoch, anchor, None)
            .expect("the anchor hash reads the record");
        let block = sample_order(Digest(B256::repeat_byte(0x11)), 40);
        let uf = certify(&c, epoch, &block);
        deterministic::Runner::default().start(|mut ctx| async move {
            assert!(
                uf.finalization.verify(&mut ctx, &scheme, &Sequential),
                "the scheme built from the module's record verifies the committee's cert"
            );
        });
    }

    /// Recorded `epoch` committee reads the canned module observed.
    type TestInlet = CertInlet<deterministic::Context, FakeMarshal>;
    type SchemeReads = Arc<Mutex<Vec<u64>>>;

    /// One committee, readable at every epoch — the verifier is built for the epoch
    /// actually asked about, because a scheme refuses a foreign one.
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

    /// Reds if cert ingress starts spending the network rung: every pin resolution
    /// the real `ingest` asks for must be `Local`.
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
        // A cert whose round-epoch disagrees with its block's height-derived epoch
        // is skipped before `committee[E]` is read.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let c = committee(1);
            let (inlet, marshal, reads) = inlet(ctx, &c);
            // activation=0, interval=64 ⇒ epoch_of(65) == 1.
            let mut inlet = inlet.with_epoch_math(0, 64);
            let block = sample_order(Digest(B256::repeat_byte(0xaa)), 65);
            // The cert is BLS-valid for epoch 2, but height 65 is in epoch 1, so the
            // bind fails before verification.
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
        // The bind is a no-op for an honest cert (round-epoch == height-epoch).
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

    /// Safe-degrade: a cert whose committee the module cannot answer for defers
    /// non-fatally — no accept-unverified, no marshal drive. The positive control
    /// below drives the same cert through a module that can read epoch 1.
    #[test]
    fn boundary_cert_defers_when_the_committee_is_unreadable() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let c = committee(1);
            let marshal = FakeMarshal::default();
            // The module never answers for epoch 1.
            let committee_module = {
                let bimap = c.bimap.clone();
                crate::committee::testing::SchemeCommittee::new(move |epoch| {
                    (epoch == 0).then(|| {
                        build_verifier(&fluent_namespace(CHAIN_ID), bimap.clone(), epoch, None)
                    })
                })
            };
            // Same committee, a module that can read epoch 1 — the positive control.
            let (control, control_marshal, _reads) = inlet(ctx.clone(), &c);
            let mut inlet = CertInlet::new(marshal.clone(), committee_module, ctx.clone());
            inlet
                .ingest(certify(
                    &c,
                    1,
                    &sample_order(Digest(B256::repeat_byte(0xbb)), 96),
                ))
                .await;
            assert!(
                marshal.calls.lock().unwrap().is_empty(),
                "a boundary cert whose committee is unreadable drives the marshal with ZERO calls"
            );
            let mut control = control;
            control
                .ingest(certify(
                    &c,
                    1,
                    &sample_order(Digest(B256::repeat_byte(0xbb)), 96),
                ))
                .await;
            assert_eq!(
                *control_marshal.calls.lock().unwrap(),
                vec!["verified", "report"],
                "the control cert did not drive the marshal either — then the assertion above \
                 says nothing about the DEFER"
            );
        });
    }

    #[test]
    fn wrong_signature_cert_skips_with_no_report_and_returns_ok() {
        // A cert formed by a different committee fails BLS against ours: the inlet
        // skips it and drives the marshal with zero calls.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let ours = committee(1);
            let theirs = committee(2);
            let (mut inlet, marshal, _) = inlet(ctx, &ours);
            let block = sample_order(Digest(B256::repeat_byte(0xaa)), 65);
            // A valid quorum of `theirs`, which our verifier rejects.
            inlet.ingest(certify(&theirs, 0, &block)).await;
            assert!(
                marshal.calls.lock().unwrap().is_empty(),
                "wrong-sig cert must drive ZERO marshal calls"
            );
        });
    }

    #[test]
    fn tampered_body_cert_skips_with_no_report_and_returns_ok() {
        // The cert signs block A's digest but the served body is block B, so
        // `payload != digest` and the inlet skips it.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let c = committee(1);
            let (mut inlet, marshal, _) = inlet(ctx, &c);
            let signed = sample_order(Digest(B256::repeat_byte(0xaa)), 65);
            let mut uf = certify(&c, 0, &signed);
            // Swap in a different body the cert does not sign.
            uf.block = sample_order(Digest(B256::repeat_byte(0xab)), 65);
            inlet.ingest(uf).await;
            assert!(
                marshal.calls.lock().unwrap().is_empty(),
                "tampered body must drive ZERO marshal calls"
            );
        });
    }

    /// A committee module that has no record for the epoch yet.
    fn unready_committee() -> Arc<crate::committee::testing::SchemeCommittee> {
        crate::committee::testing::SchemeCommittee::new(|_| None)
    }

    #[test]
    fn committee_not_yet_committed_skips_non_fatally_without_blocking() {
        // The first cert of an epoch whose committee[E] is not readable yet must be
        // skipped: block-sleeping would starve the executor it waits on, and
        // returning an error would shut the node down. A later cert retries.
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

    #[test]
    fn consecutive_data_faults_rotate_once_lag_does_not_success_resets() {
        // N consecutive BLS-verify failures trigger exactly one `rotate()`; a benign
        // committee-lag skip does not count; and a successful verify resets the
        // streak.
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

            // MAX_UPSTREAM_FAULTS-1 wrong-committee certs: below threshold, no
            // rotation yet.
            for _ in 0..MAX_UPSTREAM_FAULTS - 1 {
                inlet.ingest(certify(&theirs, 0, &block)).await;
            }
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "below threshold: no rotation"
            );
            // The Nth consecutive data fault rotates exactly once and resets the streak.
            inlet.ingest(certify(&theirs, 0, &block)).await;
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "MAX_UPSTREAM_FAULTS consecutive data faults ⇒ exactly one rotate()"
            );

            // After the reset a fresh streak climbs from zero, so one more fault does
            // not re-rotate.
            inlet.ingest(certify(&theirs, 0, &block)).await;
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "the counter reset after rotating: one post-reset fault must not re-rotate"
            );

            // A success resets the streak: two faults, a good cert, two more faults
            // must not reach the threshold.
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
        // The data-fault streak is scoped to the live connection: a connection change
        // resets it, so upstream A's faults never bleed into B's budget.
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

            for _ in 0..MAX_UPSTREAM_FAULTS - 1 {
                inlet.ingest(certify(&theirs, 0, &block)).await;
            }
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "A below threshold: no rotation yet"
            );

            // The WS actor's auto-rotation to upstream B bumps the generation. B's one
            // bad cert is its first fault, so no rotation happens.
            conn_gen.fetch_add(1, std::sync::atomic::Ordering::Release);
            inlet.ingest(certify(&theirs, 0, &block)).await;
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "A's fault streak must NOT carry into B's rotation budget after a \
                 connection change"
            );

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
        // A committee-not-yet-committed skip is transient lag, not a data fault: it
        // must never count toward rotation even repeated past the threshold.
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
        // With `with_window` set, a valid cert drives the marshal and emits the
        // verified pair to the serving window; a rejected cert emits to neither.
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

            // A wrong-committee cert: no marshal call, no window emit.
            let theirs = committee(2);
            inlet.ingest(certify(&theirs, 0, &block)).await;
            assert!(
                window_rx.try_recv().is_err(),
                "a rejected cert must NOT enter the serving window"
            );
        });
    }

    #[test]
    fn inflight_guard_deregisters_on_panic_unwind() {
        // The in-flight height is removed even when the fetch task unwinds; the RAII
        // guard is the mechanism, since a trailing remove would be skipped on unwind
        // and wedge the resolver on a height it never re-fetches.
        let inflight: super::Inflight =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::new()));
        inflight.lock().unwrap().insert(42, None);

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
            !inflight.lock().unwrap().contains_key(&42),
            "the in-flight height must be de-registered on panic-unwind (else the \
             resolver wedges on a height it can never re-fetch)"
        );

        inflight.lock().unwrap().insert(7, None);
        {
            let _guard = super::InflightGuard {
                inflight: &inflight,
                height: 7,
            };
        }
        assert!(
            !inflight.lock().unwrap().contains_key(&7),
            "the in-flight height must be de-registered on a normal drop too"
        );
    }

    /// A cancelled height's pull is aborted, not merely forgotten: an upstream that
    /// answers after the cancel must not deliver into the marshal.
    #[test]
    fn a_cancelled_fetch_aborts_the_pull_in_flight() {
        use commonware_resolver::Resolver as _;
        use commonware_runtime::{Clock as _, Metrics as _, Spawner as _};

        /// Parks every by-height pull on `gate`, and counts the pulls that came back
        /// from it — the ones that would have delivered.
        #[derive(Clone)]
        struct ParkedUpstream {
            gate: Arc<tokio::sync::Notify>,
            served: Arc<std::sync::atomic::AtomicUsize>,
            uf: UpstreamFinalized,
        }
        impl crate::cert_follow::CertUpstream for ParkedUpstream {
            async fn get_finalization(&self, _height: Height) -> Option<UpstreamFinalized> {
                self.gate.notified().await;
                self.served
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Some(self.uf.clone())
            }
            async fn get_latest(&self) -> Option<UpstreamFinalized> {
                None
            }
            async fn rotate(&self) {}
        }

        let c = committee(9);
        let block = sample_order(Digest(B256::repeat_byte(0xaa)), 41);
        let uf = certify(&c, 0, &block);
        let served = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let served_probe = served.clone();
        let delivered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let delivered_probe = delivered.clone();
        deterministic::Runner::default().start(|ctx| async move {
            let (tx, mut rx) = tokio::sync::mpsc::channel(8);
            drop(ctx.with_label("fake_marshal").spawn(move |_| async move {
                while let Some(msg) = rx.recv().await {
                    if let commonware_consensus::marshal::resolver::handler::Message::Deliver {
                        response,
                        ..
                    } = msg
                    {
                        delivered.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let _ = response.send(true);
                    }
                }
            }));
            let gate = Arc::new(tokio::sync::Notify::new());
            let mut resolver = UpstreamResolver::new(
                ctx.clone(),
                ParkedUpstream {
                    gate: gate.clone(),
                    served,
                    uf,
                },
                commonware_consensus::marshal::resolver::handler::Handler::<Digest>::new(tx),
                Arc::new(crate::beacon::testing::Canned::new()),
            );
            let key = super::MarshalRequest::Finalized {
                height: Height::new(41),
            };
            resolver.fetch(key.clone()).await;
            ctx.sleep(std::time::Duration::from_millis(1)).await;
            assert!(
                matches!(resolver.inflight.lock().unwrap().get(&41), Some(Some(_))),
                "the pull's handle is held while it is in flight"
            );
            resolver.cancel(key).await;
            assert!(
                resolver.inflight.lock().unwrap().is_empty(),
                "the cancelled height is forgotten"
            );
            // The upstream answers now (a stored permit, so the wake does not depend on
            // the pull having registered); an aborted pull never comes back from it.
            gate.notify_one();
            ctx.sleep(std::time::Duration::from_millis(5)).await;
        });
        assert_eq!(
            served_probe.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "the cancelled pull must not return from the upstream"
        );
        assert_eq!(
            delivered_probe.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "the cancelled pull must not deliver into the marshal"
        );
    }

    /// A beacon-active committee: `COMMITTEE_N` combined-scheme signers over one
    /// real DKG (share index == commonware participant index) and an assembler that
    /// recovers the round seed into the finalization cert. Its group key is what
    /// `group_public_key` resolves to, so the inlet's resolved pin checks seeds
    /// against the same key the signers use.
    struct BeaconFixture {
        /// One entry per member: its BLS keypair and its threshold share. Schemes
        /// are built per epoch by [`BeaconFixture::signers`].
        members: Vec<(ValidatorBlsKeypair, Share)>,
        sharing: Sharing<MinSig>,
        seed_ns: Vec<u8>,
        /// The ceremony's own `Output`, which the epoch's agreement artifact carries.
        outcome: crate::beacon::testing::DkgOutcome,
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

        /// The verifier-flavoured assembler (polynomial, no share) that recovers the
        /// round seed into the finalization cert.
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
        // `deal` indexes shares by the player's position in the commonware-sorted
        // `Set`, which matches the BiMap ordering, so each signer's share index equals
        // its vote participant index (asserted inside `build_signer`).
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
            outcome,
            bimap,
            namespace: ns,
        }
    }

    fn beacon_order(height: u64) -> OrderBlock {
        sample_order(Digest(B256::repeat_byte(0xaa)), height)
    }

    /// A real seeded finalization cert: every signer's Finalize carries a threshold
    /// seed partial, and the assembler recovers the round seed into the cert.
    #[test]
    fn a_certificate_seed_is_captured_under_the_certificates_own_round() {
        let bc = beacon_committee(9);
        let block = sample_order(Digest(B256::repeat_byte(0xcc)), 40);
        let uf = certify_seeded(&bc, 5, &block);
        let round = uf.finalization.proposal.round;

        let known = crate::beacon::testing::Canned::new()
            .with_seed_namespace(bc.seed_ns.clone())
            .with_mint(5, bc.outcome.clone());
        let _ = Beacon::observe_certificate(
            &known,
            ObservedCertificate::Finalization(round, &uf.finalization),
        );
        assert!(
            known.store().seed(round).is_some(),
            "a checked seed is served"
        );
        assert!(known.store().pending_epochs().is_empty());
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
            keyless.store().seed(round),
            None,
            "an unchecked seed is never served"
        );
        assert_eq!(keyless.store().pending_epochs(), vec![5]);
    }

    #[test]
    fn a_certificate_seed_that_fails_its_key_is_neither_served_nor_held() {
        let bc = beacon_committee(9);
        let other = beacon_committee(10);
        let block = sample_order(Digest(B256::repeat_byte(0xcc)), 40);
        let uf = certify_seeded(&bc, 5, &block);
        let round = uf.finalization.proposal.round;

        // The key of a different committee: the sigma is a well-formed curve point
        // that verifies under no key here.
        let wrong = crate::beacon::testing::Canned::new()
            .with_seed_namespace(bc.seed_ns.clone())
            .with_mint(5, other.outcome.clone());
        let _ = Beacon::observe_certificate(
            &wrong,
            ObservedCertificate::Finalization(round, &uf.finalization),
        );
        assert_eq!(wrong.store().seed(round), None);
        assert!(
            wrong.store().pending_epochs().is_empty(),
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

    // The plane arm. `CertInlet::ingest` stands up only with WS upstreams
    // configured, while `ValidatorUpstream::Plane` is the default, so this by-height
    // pull is the door a plane-native validator's epoch-E certificates come through;
    // a capture wired only into the inlet would leave the default configuration
    // unserved.
    #[test]
    fn the_plane_arms_by_height_pull_captures_the_certificate_seed() {
        let bc = beacon_committee(9);
        let block = sample_order(Digest(B256::repeat_byte(0xdd)), 41);
        let uf = certify_seeded(&bc, 5, &block);
        let round = uf.finalization.proposal.round;
        let known = Arc::new(
            crate::beacon::testing::Canned::new()
                .with_seed_namespace(bc.seed_ns.clone())
                .with_mint(5, bc.outcome.clone()),
        );

        let runtime = commonware_runtime::deterministic::Runner::default();
        let store_probe = known.store().clone();
        let probe_in_task = store_probe.clone();
        runtime.start(|ctx| async move {
            // A stand-in marshal that accepts, so the capture can ride its verdict.
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
            // The pull runs in its own task, so step the clock until it lands.
            for _ in 0..64 {
                if probe_in_task.seed(round).is_some() {
                    break;
                }
                commonware_runtime::Clock::sleep(&ctx, std::time::Duration::from_millis(1)).await;
            }
        });
        assert!(
            store_probe.seed(round).is_some(),
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

    /// Recorded `(epoch, pin.is_some())` per finalized-tip committee read — the
    /// pin-resolution trail a beacon-aware test asserts against.
    type PinReads = Arc<Mutex<Vec<(u64, bool)>>>;

    type BeaconInlet = CertInlet<deterministic::Context, FakeMarshal>;

    /// A committee module wired the way production wires one: the scheme is built
    /// over the epoch's record plus `Beacon::oracle_for(epoch)`, from a beacon handle
    /// that arrives after the module. The test fills the slot with the same provider
    /// it hands the inlet.
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
            // Unset means the inlet's own default provider, which is what a test that
            // never wires one gets too.
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

    /// Hand the same provider to the inlet and to the committee module's verifier.
    fn with_beacon(
        inlet: BeaconInlet,
        slot: &crate::committee::BeaconSlot,
        randomness: Arc<dyn Beacon>,
    ) -> BeaconInlet {
        let _ = slot.set(Arc::downgrade(&randomness));
        inlet.with_randomness(randomness)
    }

    /// A provider over the real key index: the chain's mint record plus the artifact
    /// store. The tests exercise the shipped resolve, not a stubbed answer.
    fn canned_randomness(mints: &MintFixture) -> Arc<dyn crate::beacon::Beacon> {
        crate::beacon::testing::LiveBeacon::build(LiveBeaconConfig {
            seeds: crate::beacon::testing::SeedStore::new(),
            keys: mints.keys.clone(),
            ceremony: Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new())),
            acquire: None,
            metrics: crate::beacon::testing::BeaconMetrics::default(),
            chain_id: CHAIN_ID,
            artifacts: mints.artifacts.clone(),
            geometry: tokio::sync::watch::channel(Some((0, 1))).1,
        })
    }

    /// A [`MintFixture`] that has minted `epoch` with `bc`'s own outcome.
    fn minted_at(bc: &BeaconFixture, epoch: u64) -> MintFixture {
        let mints = MintFixture::new();
        mints.mint(epoch, bc.outcome.clone());
        mints
    }

    /// The group key a fixture's ceremony produced, which its seeded certs verify
    /// against and the ladder must answer with.
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
        // The ladder pins the epoch off its agreed artifact. A later cert whose
        // recovered seed slot is tampered — a valid seed for the wrong round on an
        // otherwise valid multisig quorum — is a data fault: never archived, never
        // teed, and counted toward rotation.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let bc = beacon_committee(1);
            let _pk = fixture_key(&bc);
            let marshal = FakeMarshal::default();
            let (inlet, _reads, slot) = beacon_inlet(ctx, &bc, marshal.clone());
            let (rotations, rotate) = count_rotations();
            let (window_tx, mut window_rx) = tokio::sync::mpsc::unbounded_channel();
            let mints = minted_at(&bc, 2);
            let randomness = canned_randomness(&mints);
            let mut inlet = with_beacon(
                inlet.with_rotate(rotate).with_window(window_tx),
                &slot,
                randomness,
            );

            let boundary = certify_seeded(&bc, 2, &beacon_order(64));
            // A valid seed for a foreign round, standing in for a tampered seed slot:
            // a decodable G1 point that verifies for no test round.
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

            // Three consecutive seed-tampered certs: each is a data fault, so the
            // marshal is not driven again and the third rotates.
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

    /// A provider over the real key index, concrete so a test can drive the late
    /// verdict the way the `KeyAvailable` edge does in production.
    fn live_beacon(mints: &MintFixture) -> Arc<crate::beacon::testing::LiveBeacon> {
        crate::beacon::testing::LiveBeacon::build(LiveBeaconConfig {
            seeds: crate::beacon::testing::SeedStore::new(),
            keys: mints.keys.clone(),
            ceremony: Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new())),
            acquire: None,
            metrics: crate::beacon::testing::BeaconMetrics::default(),
            chain_id: CHAIN_ID,
            artifacts: mints.artifacts.clone(),
            geometry: tokio::sync::watch::channel(Some((0, 1))).1,
        })
    }

    /// A committee module whose verifier carries no seed oracle — the shape
    /// `plane_upstream::verifier_for` builds. Such a verifier admits a certificate
    /// whose sigma slot was swapped under an intact multisig, which is why the sigma
    /// verdict is read at ingress.
    fn oracle_less_inlet(
        ctx: deterministic::Context,
        bc: &BeaconFixture,
        marshal: FakeMarshal,
    ) -> BeaconInlet {
        let namespace = bc.namespace.clone();
        let bimap = bc.bimap.clone();
        let committee = crate::committee::testing::SchemeCommittee::new(move |epoch| {
            Some(build_verifier(&namespace, bimap.clone(), epoch, None))
        });
        CertInlet::new(marshal, committee, ctx)
    }

    /// A certificate whose sigma slot is forged passes a verifier built without the
    /// epoch's oracle, so the beacon's verdict is the only gate left: `Refused` must
    /// skip the certificate and count a data fault, so three of them rotate away from
    /// the upstream that served them.
    ///
    /// The genuine certificate is ingested first, so the oracle-less verifier
    /// admitting the multisig is witnessed rather than assumed.
    #[test]
    fn a_forged_seed_under_an_oracle_less_verifier_is_refused_at_the_ingress() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let bc = beacon_committee(3);
            let marshal = FakeMarshal::default();
            let (rotations, rotate) = count_rotations();
            let (window_tx, mut window_rx) = tokio::sync::mpsc::unbounded_channel();
            let mints = minted_at(&bc, 2);
            let mut inlet = oracle_less_inlet(ctx, &bc, marshal.clone())
                .with_rotate(rotate)
                .with_window(window_tx)
                .with_randomness(live_beacon(&mints));

            let wrong = certify_seeded(&bc, 9, &beacon_order(999))
                .finalization
                .certificate
                .seed;
            let genuine = certify_seeded(&bc, 2, &beacon_order(64));
            assert_ne!(
                genuine.finalization.certificate.seed, wrong,
                "the forgery must differ from the genuine σ"
            );

            inlet.ingest(genuine).await;
            assert_eq!(
                *marshal.calls.lock().unwrap(),
                vec!["verified", "report"],
                "the oracle-less verifier admits a genuine certificate"
            );
            assert_eq!(window_rx.try_recv().expect("teed").block.height, 64);

            for h in 65..=67u64 {
                let mut forged = certify_seeded(&bc, 2, &beacon_order(h));
                forged.finalization.certificate.seed = wrong;
                inlet.ingest(forged).await;
            }
            assert_eq!(
                *marshal.calls.lock().unwrap(),
                vec!["verified", "report"],
                "a σ-refused certificate drives the marshal ZERO further times"
            );
            assert!(
                window_rx.try_recv().is_err(),
                "and reaches no serving window"
            );
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "MAX_UPSTREAM_FAULTS σ refusals are data faults ⇒ exactly one rotation"
            );
        });
    }

    /// sigma admitted while this node held no `PK_E` is `Pending`, not a fault, and
    /// gets the same vote-only admission as the certificate. When the key lands the
    /// settle refuses what does not verify and files a `DataFault`; the inlet charges
    /// the upstream's streak and fails over. The three halves are asserted at their
    /// own moments: keyless, key lands, next ingest.
    #[test]
    fn a_late_refusal_costs_the_upstream_a_rotation_once_the_key_lands() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let bc = beacon_committee(4);
            let marshal = FakeMarshal::default();
            let (rotations, rotate) = count_rotations();
            // Nothing minted yet: the keyless window in which a forged sigma is
            // admitted rather than refused.
            let mints = MintFixture::new();
            let beacon = live_beacon(&mints);
            let (inlet, _reads, slot) = beacon_inlet(ctx, &bc, marshal.clone());
            let mut inlet = with_beacon(inlet.with_rotate(rotate), &slot, beacon.clone());

            let wrong = certify_seeded(&bc, 9, &beacon_order(999))
                .finalization
                .certificate
                .seed;
            let mut rounds = Vec::new();
            for h in 64..=66u64 {
                let mut forged = certify_seeded(&bc, 2, &beacon_order(h));
                forged.finalization.certificate.seed = wrong;
                let round = forged.finalization.proposal.round;
                assert_eq!(
                    Beacon::observe_certificate(
                        beacon.as_ref(),
                        ObservedCertificate::Finalization(round, &forged.finalization),
                    ),
                    Observed::Pending,
                    "with no key resolvable the σ is HELD, not judged"
                );
                assert!(
                    Beacon::seed(beacon.as_ref(), round).is_none(),
                    "a held σ is never served"
                );
                rounds.push(round);
            }
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "the keyless window is not a fault: nothing has been judged yet"
            );

            // The key lands: in production an artifact insert, and the beacon's
            // wake-up bridge calls `settle_pending` before publishing `KeyAvailable`.
            mints.mint(2, bc.outcome.clone());
            beacon.settle_pending();
            for round in &rounds {
                assert!(
                    Beacon::seed(beacon.as_ref(), *round).is_none(),
                    "a refused σ is dropped, not promoted"
                );
            }

            // The next certificate is where the inlet reads its channel; a clean one,
            // so the rotation cannot be confused with a synchronous fault.
            inlet
                .ingest(certify_seeded(&bc, 2, &beacon_order(67)))
                .await;
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "three late refusals charge the streak to its threshold ⇒ one rotation"
            );
            assert_eq!(
                *marshal.calls.lock().unwrap(),
                vec!["verified", "report"],
                "and the clean certificate it rode in on is still processed"
            );
        });
    }

    /// The two halves of the streak are symmetric: a synchronous fault survives the
    /// certificate it was charged on by returning before the reset, and a late one
    /// charged at the top of the same call must also survive it. One late charge plus
    /// two synchronous ones reaches `MAX_UPSTREAM_FAULTS`.
    #[test]
    fn a_sub_threshold_late_charge_survives_the_certificate_it_rode_in_on() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let bc = beacon_committee(4);
            let marshal = FakeMarshal::default();
            let (rotations, rotate) = count_rotations();
            // Keyless to begin with: the window in which a forged sigma is held rather
            // than judged.
            let mints = MintFixture::new();
            let beacon = live_beacon(&mints);
            let (inlet, _reads, slot) = beacon_inlet(ctx, &bc, marshal.clone());
            let mut inlet = with_beacon(inlet.with_rotate(rotate), &slot, beacon.clone());

            let wrong = certify_seeded(&bc, 9, &beacon_order(999))
                .finalization
                .certificate
                .seed;

            // One forged round held while keyless: one late charge, below the
            // threshold.
            let mut forged = certify_seeded(&bc, 2, &beacon_order(64));
            assert_ne!(
                forged.finalization.certificate.seed, wrong,
                "the splice must change the σ, or this test refuses nothing"
            );
            forged.finalization.certificate.seed = wrong;
            let round = forged.finalization.proposal.round;
            assert_eq!(
                Beacon::observe_certificate(
                    beacon.as_ref(),
                    ObservedCertificate::Finalization(round, &forged.finalization),
                ),
                Observed::Pending,
                "with no key resolvable the σ is HELD, not judged"
            );
            mints.mint(2, bc.outcome.clone());
            beacon.settle_pending();

            // The clean certificate the charge rides in on; it must not erase the
            // accusation it delivered.
            inlet
                .ingest(certify_seeded(&bc, 2, &beacon_order(65)))
                .await;
            assert_eq!(
                *marshal.calls.lock().unwrap(),
                vec!["verified", "report"],
                "the clean certificate is still processed"
            );
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "one charge is below the rotation threshold on its own"
            );

            // Two synchronous faults, which with the surviving late charge reach the
            // threshold.
            for h in 66..=67u64 {
                let mut tampered = certify_seeded(&bc, 2, &beacon_order(h));
                tampered.finalization.certificate.seed = wrong;
                inlet.ingest(tampered).await;
            }
            assert_eq!(
                *marshal.calls.lock().unwrap(),
                vec!["verified", "report"],
                "and the tampered certificates drive the marshal zero further times"
            );
            assert_eq!(
                rotations.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "a late charge plus two synchronous ones is the threshold: one rotation"
            );
        });
    }

    #[test]
    fn a_non_change_stretch_pins_from_the_shared_ladder_not_a_forward_cursor() {
        // The key minted at change-epoch 2 also keys epochs 3 and 5, which ran no
        // agreement of their own. It comes from the chain's `dkgQual` record naming
        // the minting epoch, not from "the greatest key seen", which would pin a stale
        // pre-rotation key past an unseen change boundary.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let bc = beacon_committee(2);
            let pk2 = fixture_key(&bc);
            let marshal = FakeMarshal::default();
            let (inlet, reads, slot) = beacon_inlet(ctx, &bc, marshal.clone());
            let mints = minted_at(&bc, 2);
            let mut inlet = with_beacon(inlet, &slot, canned_randomness(&mints));

            let wrong = certify_seeded(&bc, 9, &beacon_order(999))
                .finalization
                .certificate
                .seed;

            inlet
                .ingest(certify_seeded(&bc, 2, &beacon_order(64)))
                .await;
            // The mint's artifact is what answers, and it answers the same value the
            // certificates verify under.
            assert_eq!(
                mints.keys.key_at(2),
                Some(pk2),
                "epoch 2's key is the artifact's, and the ingress resolves it"
            );
            assert_eq!(
                mints.keys.minted_at(2),
                Some(2),
                "and epoch 2 is its own mint here, which is what the chain record says"
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

            // A seed-tampered cert for the carried-forward epoch 5 is still rejected,
            // so the resolved pin is genuinely PK_epoch and not None.
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
        // A key minted at a change epoch is `PK_E` for that epoch and forward, never
        // backward. An epoch before the beacon's bootstrap resolves no key and is
        // admitted vote-only, so a seed-tampered pre-beacon cert is admitted rather
        // than checked against the new key.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let bc = beacon_committee(3);
            let _pk = fixture_key(&bc);
            let marshal = FakeMarshal::default();
            let (inlet, reads, slot) = beacon_inlet(ctx, &bc, marshal.clone());
            let mut inlet = with_beacon(inlet, &slot, canned_randomness(&minted_at(&bc, 2)));

            let wrong = certify_seeded(&bc, 9, &beacon_order(999))
                .finalization
                .certificate
                .seed;

            inlet
                .ingest(certify_seeded(&bc, 2, &beacon_order(64)))
                .await;

            // A pre-beacon epoch-1 cert with a tampered seed: epoch 1 predates the
            // bootstrap mint, so it resolves no key and is admitted vote-only.
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

    /// Two independent DKG ceremonies over one committee (same peers, vote keys,
    /// bimap and namespace, but different group polynomials, so `PK_a != PK_b`).
    /// Certs are vote-valid under either; seeds verify only under their own key.
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
                outcome,
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
        // A change boundary whose cert was dropped: nothing local has seen the new
        // epoch's key when the next cert of that epoch arrives.
        //
        // With no source the epoch resolves no key and its cert is admitted vote-only
        // rather than rejected against a stale pin. With a source resolving the key
        // from its artifact, the same sequence verifies and drives the marshal.
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (f1, f2) = beacon_committee_pair(4);
            let _pk3 = fixture_key(&f2);

            // No source: no key for this epoch, so vote-only admission.
            let marshal = FakeMarshal::default();
            // No provider wired on purpose: the inlet and the module's verifier both
            // fall back to the default absent one.
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

            // A source resolving the key from its late-arriving artifact.
            let marshal = FakeMarshal::default();
            let (inlet, _reads, slot) = beacon_inlet(ctx, &f1, marshal.clone());
            let mut inlet = with_beacon(inlet, &slot, canned_randomness(&minted_at(&f2, 3)));
            inlet
                .ingest(certify_seeded(&f2, 3, &beacon_order(129)))
                .await;
            // The cache entry is now pinned, so the next cert rides it without
            // re-consulting the source.
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

    /// A cached scheme starts checking the seed the moment its epoch's key resolves,
    /// with no rebuild, no eviction and no BLS failure in between.
    ///
    /// Epoch 2 is first seen while `PK_2` is unresolvable, so its certificates are
    /// admitted on the multisig half alone; before the oracle, the only path that
    /// rebuilt an entry was the verify-fail eviction, which never fires for honest
    /// traffic.
    #[test]
    fn a_cached_scheme_starts_checking_the_seed_once_the_key_resolves() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (f1, f2) = beacon_committee_pair(5);
            let _pk2 = fixture_key(&f2);
            // Epoch 2's artifact is absent until the fixture files it.
            let mints = MintFixture::new();

            let marshal = FakeMarshal::default();
            let (inlet, reads, slot) = beacon_inlet(ctx, &f1, marshal.clone());
            let mut inlet = with_beacon(inlet, &slot, canned_randomness(&mints));

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

            // Filing the artifact is what makes the epoch's key resolvable.
            mints.mint(2, f2.outcome.clone());
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

            // The seed half is checked now: a seed-tampered epoch-2 cert is rejected,
            // where the same cached scheme admitted the keyless certs above.
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

    /// The key resolve is memoised: the chain, asked for the epoch's `changed` bit,
    /// answers once however many certificates of the epoch arrive. The scheme is built
    /// once too and reads the key live, so a key arriving after it was built needs no
    /// rebuild.
    ///
    /// The mint is epoch 3, not the bootstrap epoch, because `minted_at` answers the
    /// bootstrap epoch without reading the chain, which would make the count vacuous.
    #[test]
    fn the_mint_resolve_reads_the_chain_once_and_the_committee_once() {
        const MINT: u64 = 3;
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (f1, f2) = beacon_committee_pair(6);
            let _pk2 = fixture_key(&f2);
            // The artifact is filed once and stays filed: the store is insert-only.
            let mints = minted_at(&f2, MINT);

            let marshal = FakeMarshal::default();
            let (inlet, reads, slot) = beacon_inlet(ctx, &f1, marshal.clone());
            let mut inlet = with_beacon(inlet, &slot, canned_randomness(&mints));

            inlet
                .ingest(certify_seeded(&f2, MINT, &beacon_order(129)))
                .await;
            inlet
                .ingest(certify_seeded(&f2, MINT, &beacon_order(130)))
                .await;
            assert_eq!(
                mints.chain_reads(MINT),
                1,
                "TWO certificates of the epoch, ONE chain read: the mint memo is what \
                 makes the resolve cheap, and a second consult means it is gone"
            );
            assert_eq!(
                mints.artifacts.epochs(),
                vec![MINT],
                "one artifact, filed once: the store is insert-only, so the resolve after \
                 the first is a map hit with nothing to re-consult"
            );
            assert_eq!(
                *reads.lock().unwrap(),
                vec![(MINT, true)],
                "the entry is not rebuilt, so the committee is read once — and it was \
                 built WITH an oracle, which is what makes the seed half checked at all"
            );

            let wrong = certify_seeded(&f2, 9, &beacon_order(999))
                .finalization
                .certificate
                .seed;
            let mut tampered = certify_seeded(&f2, MINT, &beacon_order(131));
            tampered.finalization.certificate.seed = wrong;
            inlet.ingest(tampered).await;
            assert_eq!(
                marshal.calls.lock().unwrap().len(),
                4,
                "the surviving pin still rejects a seed-tampered cert — had the \
                 pin-less ingest replaced the entry, this one would have been admitted"
            );
            assert_eq!(
                mints.chain_reads(MINT),
                1,
                "and a THIRD certificate still buys no chain read"
            );
        });
    }

    // TODO: pin against the real `MarshalActor` — build it via
    // `MarshalActor::init` + `start((rx, NoopResolver), buffer_mailbox)` on the
    // deterministic runtime with NO resolver peers, feed a valid cert through a
    // real `MarshalMailbox`, and assert it dispatches the block to a recording
    // application-`Reporter` (body resolved locally, `NoopResolver` never fired)
    // and `processed_height` advanced. The real-marshal harness (archives +
    // `buffered::Engine` + `CertProvider` + `Epocher` + application reporter) is
    // heavy; the `MarshalSink` seam above keeps the call-order proof
    // deterministic and cheap. The follower path (`launch_follower`) exercises
    // the real marshal end-to-end under `make smoke-cert-follow`.
}

/// A no-op [`commonware_resolver::Resolver`] for the marshal's resolver channel: the
/// cert-inlet makes every body local before reporting its cert, so the marshal never
/// needs a peer fetch. The near-planeless follower runs the marshal with no resolver
/// peers and hands it this channel.
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

/// The marshal request key the follower's resolver serves: the ordering [`Digest`]
/// as a `Standard<OrderBlock>` commitment.
type MarshalRequest = commonware_consensus::marshal::resolver::handler::Request<Digest>;

/// In-flight `Finalized` pulls by height. A slot is `None` between the dedup
/// insert and the spawn, and holds the task's handle from then on, so `cancel`,
/// `clear` and `retain` can abort the pull rather than only forget it.
type Inflight =
    std::sync::Arc<std::sync::Mutex<std::collections::BTreeMap<u64, Option<Handle<()>>>>>;

/// RAII de-register for one in-flight [`UpstreamResolver`] fetch height. Removing
/// the height in `Drop` rather than as a trailing statement keeps the cleanup
/// panic-safe: if the spawned fetch task panics mid-pull, the height is still
/// removed and the marshal can re-request it on its next repair sweep.
struct InflightGuard<'a> {
    inflight: &'a Inflight,
    height: u64,
}

impl Drop for InflightGuard<'_> {
    fn drop(&mut self) {
        // A poisoned lock (a prior panic while holding it) still must not block
        // de-registration.
        let mut set = self
            .inflight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        set.remove(&self.height);
    }
}

/// An upstream-backed [`commonware_resolver::Resolver`] for the near-planeless
/// follower: it backfills the marshal's by-height gap from the cert upstream
/// instead of from peers, since a follower has no consensus-plane connectivity.
///
/// The inlet only ingests the upstream's live finalized stream, so after the
/// cold-start jump there is a gap between the marshal floor and the first ingested
/// cert. The marshal dispatches to the executor only contiguously from
/// `floor + 1`, so without this resolver the gap never fills and the executor
/// stays idle.
///
/// Only [`MarshalRequest::Finalized`] is served: it pulls `(finalization, block)`
/// from the upstream and delivers the encoded tuple back through the marshal
/// handler, which BLS-verifies it against the per-epoch committee. `Block` and
/// `Notarized` requests are no-ops. This only delivers into the marshal; the
/// executor remains the sole reth writer.
pub struct UpstreamResolver<E, U> {
    ctx: E,
    upstream: U,
    /// Deliver channel into the marshal actor; a resolved fetch lands here.
    handler: commonware_consensus::marshal::resolver::handler::Handler<Digest>,
    /// In-flight `Finalized` pulls, so repeated repair bursts for the same gap do
    /// not spawn duplicate pulls, and so a cancelled height's pull is aborted.
    inflight: Inflight,
    /// The seed-capture seam for the plane arm: this resolver is the door a
    /// plane-native validator's certificates come through, since the live-stream
    /// inlet stands up only with WS upstreams configured.
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
            inflight: std::sync::Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::new())),
            randomness,
        }
    }

    /// Spawn one by-height pull and deliver it, deduped by height. The marshal
    /// re-requests on its next repair sweep if the upstream did not have the height,
    /// so a transient miss self-heals.
    ///
    /// This is the second door that files sigma. It exists because there is no point
    /// after the marshal's verification where sigma is still visible, so the
    /// certificate goes to the beacon at ingress. A pure follower keeps what it files:
    /// its beacon holds its own seed store, and this is the only route by which its
    /// executor can derive a beacon-active block's `prev_randao`.
    ///
    /// Neither door prunes what it files: the seed index measures its own retention
    /// window from the highest epoch it holds.
    fn spawn_finalized(&self, height: commonware_consensus::types::Height) {
        let h = height.get();
        {
            let mut inflight = self.inflight.lock().unwrap();
            if inflight.contains_key(&h) {
                return; // already in flight
            }
            inflight.insert(h, None);
        }
        let upstream = self.upstream.clone();
        let mut handler = self.handler.clone();
        let inflight = self.inflight.clone();
        let randomness = self.randomness.clone();
        let handle = self
            .ctx
            .with_label("upstream_resolver_fetch")
            .spawn(move |_| async move {
                use commonware_codec::Encode as _;
                use commonware_resolver::Consumer as _;
                // RAII de-register: the height is removed on every exit of this
                // task, completion and panic-unwind alike. A trailing `remove`
                // would be skipped on unwind, leaving the height in `inflight`
                // forever, so the marshal could never re-fetch it and contiguous
                // dispatch would wedge permanently.
                let _guard = InflightGuard {
                    inflight: &inflight,
                    height: h,
                };
                // Deliberately the single-shot pull: this is the marshal's gap
                // repair, up to `MAX_REPAIR` concurrent by-height pulls per sweep.
                // A wider walk here would be that many short-lived connections per
                // second, forever, for a height nobody holds; the escape from a
                // permanently unservable gap is the re-jump.
                if let Some(uf) = upstream.get_finalization(height).await {
                    let round = uf.finalization.proposal.round;
                    let captured = uf.finalization.clone();
                    let key = MarshalRequest::Finalized { height };
                    let value = (uf.finalization, uf.block).encode();
                    // `deliver` routes into the marshal actor, which decodes and
                    // BLS-verifies the cert before storing it; a `false` return
                    // just leaves the height for the next repair sweep.
                    //
                    // The sigma capture rides that verdict: a held sigma is keyed
                    // on a round the responder chose, so taking it from an
                    // unverified pull would let one peer name any round it liked.
                    // After `true` the multisig has been checked against
                    // `committee[epoch]`, so the round came from a quorum.
                    if handler.deliver(key, value).await
                        && randomness.observe_certificate(ObservedCertificate::Finalization(
                            round, &captured,
                        )) == Observed::Refused
                    {
                        // A witness with no lever: this door holds no
                        // `RotateUpstream`, so the refusal is only logged here.
                        warn!(
                            height = h,
                            %round,
                            "upstream resolver: the certificate's σ is REFUSED under this \
                             epoch's attested key — this door cannot rotate away from the \
                             upstream that served it"
                        );
                    }
                }
            });
        // The task may already have finished and had its guard remove the slot; a
        // handle stored then would outlive the pull it names, so only an occupied
        // slot takes it. A dropped handle detaches, it does not abort.
        if let Some(slot) = self.inflight.lock().unwrap().get_mut(&h) {
            *slot = Some(handle);
        }
    }
}

/// Abort every pull in `slots` whose handle was stored; a slot still `None` was
/// never spawned past its dedup insert.
fn abort_pulls(slots: impl IntoIterator<Item = Option<Handle<()>>>) {
    for handle in slots.into_iter().flatten() {
        handle.abort();
    }
}

/// The follower's marshal resolver: either the upstream-backed backfill or a
/// [`NoopResolver`] when no upstream is configured. One concrete type so the
/// follower hands the marshal a single `Resolver` regardless of config.
#[derive(Clone)]
pub enum FollowerResolver<E, U> {
    /// Upstream-backed by-height backfill (the production follower path).
    Upstream(UpstreamResolver<E, U>),
    /// No upstream configured — fetches nothing; the follower would wedge, but this
    /// is not a reachable production config.
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
        targets: commonware_utils::vec::NonEmptyVec<Self::PublicKey>,
    ) {
        // On a `Hybrid` node every `Finalized` fetch reaches this resolver, targeted
        // or not; the single upstream is the only peer, so the target list is dropped.
        if let MarshalRequest::Finalized { height } = key {
            tracing::debug!(
                height = height.get(),
                targets = targets.len(),
                "upstream resolver: targeted by-height fetch on a node with no plane — the \
                 single upstream is the only peer, so the target list is ignored"
            );
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
            let removed = self.inflight.lock().unwrap().remove(&height.get());
            abort_pulls(removed);
        }
    }

    async fn clear(&mut self) {
        let removed: Vec<_> = std::mem::take(&mut *self.inflight.lock().unwrap())
            .into_values()
            .collect();
        abort_pulls(removed);
    }

    async fn retain(&mut self, predicate: impl Fn(&Self::Key) -> bool + Send + 'static) {
        let mut removed = Vec::new();
        self.inflight.lock().unwrap().retain(|&h, handle| {
            let keep = predicate(&MarshalRequest::Finalized {
                height: commonware_consensus::types::Height::new(h),
            });
            if !keep {
                removed.push(handle.take());
            }
            keep
        });
        abort_pulls(removed);
    }
}
