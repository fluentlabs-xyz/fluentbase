//! Per-epoch consensus engine: simplex::Engine + Inline wrapper for one epoch.
//!
//! Owns only the per-epoch components (simplex + Inline). The marshal mailbox,
//! buffered::Engine, archives, and EpochSchemeProvider are global singletons
//! living in [`crate::outer::OuterEngine`].

use crate::{
    application::{ExecutedChain, FluentApp, OrderingAssembler},
    beacon::certify::{BeaconCertify, SeedStore},
    digest::Digest,
    elector_seed::epoch_leader_seed,
    epocher::OriginEpocher,
    order_block::OrderBlock,
    scheme::epoch_committee_from_snapshot,
    slasher::Mailbox as SlasherMailbox,
    timeouts::ConsensusTimeouts,
};
use crate::{REPLAY_BUFFER, WRITE_BUFFER};
use commonware_consensus::{
    marshal::{
        core::Mailbox as MarshalMailbox,
        standard::{Inline, Standard},
    },
    simplex::{self, config::ForwardingPolicy, elector::RoundRobin, types::Activity},
    types::Epoch,
    Reporters,
};
use commonware_cryptography::{ed25519, Sha256};
use commonware_p2p::{Blocker, Receiver, Sender};
use commonware_parallel::Sequential;
use commonware_runtime::{
    buffer::paged::CacheRef, BufferPooler, Clock, ContextCell, Handle, Metrics, Spawner, Storage,
};
use fluentbase_bls::{
    beacon::seed_namespace,
    fluent_namespace,
    keys::ValidatorBlsKeypair,
    scheme::{build_signer, build_verifier, BeaconKey},
    Scheme as BlsScheme,
};
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use rand_core::CryptoRngCore;
use std::sync::Arc;

const FETCH_CONCURRENT: usize = 4;

/// The automaton+relay handed to simplex: `Inline` plus the Stage-2 beacon
/// seed-verify at `certify` ([`crate::beacon::certify`]). `BeaconCertify` wraps a
/// per-epoch `Inline` (built in [`EpochEngine::new`]).
type AutomatonFor<E, XC, A> = BeaconCertify<E, XC, A>;

type ConsensusEngine<E, B, XC, A> = simplex::Engine<
    E,
    BlsScheme,
    RoundRobin<Sha256>,
    B,
    Digest,
    AutomatonFor<E, XC, A>,
    AutomatonFor<E, XC, A>,
    Reporters<
        Activity<BlsScheme, Digest>,
        Reporters<
            Activity<BlsScheme, Digest>,
            MarshalMailbox<BlsScheme, Standard<OrderBlock>>,
            SlasherMailbox,
        >,
        crate::spec_exec::Mailbox,
    >,
    Sequential,
>;

/// Constructor parameters for [`EpochEngine`].
///
/// `EvSink` is intentionally absent: `marshal_mailbox` is the sole simplex
/// reporter (it impls `Reporter<Activity = Activity<S, V::Commitment>>`).
/// Slashing/evidence reporting is wired separately through the slasher actor.
pub struct EpochEngineConfig<B, XC, A> {
    pub blocker: B,
    pub snapshot: ValidatorSetSnapshot,
    pub epoch: Epoch,
    /// Single cross-epoch `OriginEpocher` instance threaded from
    /// [`crate::outer::OuterBuilder::build`] (no per-epoch re-construction;
    /// marshal and engine share the same instance). `origin = dposActivationBlock`.
    pub epocher: OriginEpocher,
    pub chain_id: u64,
    pub signer_keypair: Option<ValidatorBlsKeypair>,
    /// Rotation-out signals to the unified supervisor (`None` = legacy).
    pub mode_events: Option<tokio::sync::mpsc::UnboundedSender<crate::dpos::ModeEvent>>,
    pub app: FluentApp<XC, A>,
    pub timeouts: ConsensusTimeouts,
    pub mailbox_size: usize,
    /// Callback that registers this epoch's [`BlsScheme`] in
    /// [`crate::outer::EpochSchemeProvider`] so marshal can verify
    /// cross-epoch finalization certificates (trailing-window pruned; see SCHEME_RETENTION_EPOCHS).
    pub register_scheme: Arc<dyn Fn(Epoch, BlsScheme) + Send + Sync>,
    /// Per-epoch threshold beacon key (`PK_epoch` polynomial + this node's
    /// share + seed namespace), threaded so the combined scheme emits/verifies
    /// the seed partial. `None` ⇒ a fallback (pure-multisig) epoch. This is the
    /// node-LOCAL DKG material — used to SIGN only when its group key matches
    /// the authoritative on-chain key below.
    pub beacon: Option<BeaconKey>,
    /// Shared `round → recovered seed` map for the Stage-2 beacon certify gate
    /// ([`crate::beacon::certify`]). Cross-epoch singleton from
    /// [`crate::outer::OuterEngine`]; written by the spec-exec reporter, read by
    /// this epoch's [`BeaconCertify`] wrapper.
    pub seed_store: SeedStore,
    /// Beacon counters (cross-epoch singleton). This epoch's engine increments
    /// the demote counters when it self-demotes to the cert-follow plane.
    pub beacon_metrics: crate::beacon::metrics::BeaconMetrics,
    /// DEVNET/TEST-ONLY byzantine validator behaviour (gated behind
    /// `dpos-devnet-byzantine`). `None` on every honest node. When
    /// `Some(ByzantineMode::Equivocate)` (and this node can sign), `new()` builds
    /// the [`Inner::Equivocate`] variant instead of the honest `simplex::Engine`.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub byzantine: Option<crate::application::ByzantineMode>,
}

/// The per-epoch engine variant chosen in [`EpochEngine::new`]. Honest nodes are
/// always [`Inner::Normal`]; only a DEVNET/TEST byzantine node (gated behind
/// `dpos-devnet-byzantine`) takes [`Inner::Equivocate`], which swaps the honest
/// `simplex::Engine` for a [`crate::byzantine::VoteEquivocator`] on the vote
/// channel. The choice is made in `new()` (so the byzantine path never even
/// builds the simplex engine) and dispatched in [`EpochEngine::start`].
enum Inner<E, B, XC, A>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics,
    B: Blocker<PublicKey = ed25519::PublicKey>,
    XC: ExecutedChain,
    A: OrderingAssembler,
{
    // Boxed: the simplex engine is far larger than the byzantine `Equivocate`
    // variant; boxing keeps the enum small (clippy::large_enum_variant).
    Normal(Box<ConsensusEngine<E, B, XC, A>>),
    /// The flagged byzantine signer scheme, carried until `start()` spawns the
    /// equivocator on the vote channel. Boxed to match the boxed `Normal` variant
    /// so neither dominates the enum size (clippy::large_enum_variant).
    #[cfg(feature = "dpos-devnet-byzantine")]
    Equivocate(Box<BlsScheme>),
}

/// Per-epoch consensus engine. Created by
/// [`crate::epoch_manager::Actor::enter`] on each boundary trigger.
pub struct EpochEngine<E, B, XC, A>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics,
    B: Blocker<PublicKey = ed25519::PublicKey>,
    XC: ExecutedChain,
    A: OrderingAssembler,
{
    context: ContextCell<E>,
    inner: Inner<E, B, XC, A>,
}

impl<E, B, XC, A> EpochEngine<E, B, XC, A>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics,
    B: Blocker<PublicKey = ed25519::PublicKey> + Clone,
    XC: ExecutedChain,
    A: OrderingAssembler,
{
    /// Build per-epoch simplex::Engine + Inline.
    ///
    /// `marshal_mailbox` + `page_cache` are passed in from [`crate::outer::OuterEngine`]
    /// (cross-epoch singletons).
    pub fn new(
        context: E,
        cfg: EpochEngineConfig<B, XC, A>,
        marshal_mailbox: MarshalMailbox<BlsScheme, Standard<OrderBlock>>,
        slasher_mailbox: SlasherMailbox,
        spec_exec_mailbox: crate::spec_exec::Mailbox,
        page_cache: CacheRef,
    ) -> eyre::Result<Self> {
        // A non-unique committee is reachable from on-chain data
        // (`Staking.setConsensusKeys` does NOT enforce cross-validator
        // uniqueness of peerPubkey/blsPubkey). Return an error so the caller
        // (epoch_manager::enter) can skip entering this epoch gracefully rather
        // than panicking the whole epoch_manager task (which collapses the entire
        // DPoS stack via the outer supervisor).
        let committee = epoch_committee_from_snapshot(&cfg.snapshot).map_err(|e| {
            eyre::eyre!(
                "epoch {} snapshot has non-unique participants: {e:?}",
                cfg.epoch.get(),
            )
        })?;
        let bimap = committee.bimap;
        let namespace = fluent_namespace(cfg.chain_id);

        // Graceful rotation-out. `build_signer` returns
        // `Option<Scheme>` (crates/bls/src/scheme.rs:22-28) with `None`
        // meaning exactly "signer keypair's public key is not in the
        // committee BiMap" — the operator's validator was rotated out of
        // this epoch's committee. Fall through to verifier mode + emit a
        // metric so the operator can see the rotation event, instead of
        // panicking and killing the per-epoch task.
        //
        // A SIGNER uses its OWN local DKG material (the full polynomial + share it
        // computed) — never gated on the on-chain key, since it signs partials
        // under its own share. The on-chain group key is only for nodes that have
        // NO local material (a fresh joiner that missed the ceremony, or a
        // non-member): they verify assembled certs against it.
        // A member that holds the local polynomial (`Sharing`) is a SIGNER even
        // if its `Share` is `None` (a joiner that missed E's DKG): the combined
        // `sign()` self-suppresses the seed-bearing Notarize/Finalize (it returns
        // `None` when `share == None`, combined_scheme.rs:280) while still
        // producing a valid Nullify — so a shareless member stays out of the
        // notarization/finalization (beacon) quorum but participates in
        // view-changes. Per the joiner-share decision (2026-06-18, model B; see
        // the `dpos_beacon_share_reshare` brief). Only a member with NO local
        // polynomial in a beacon-active epoch falls back to verify-only against
        // the authoritative on-chain key (it cannot self-suppress, so signing
        // would emit a rejected seedless vote).
        // Beacon-active from `DETERMINISTIC_BOOTSTRAP_EPOCH` on — a LOCAL,
        // deterministic predicate that replaces the old on-chain `beacon_verify_pk`
        // Some/None probe (the PK_E layer is gone). A member with no local
        // polynomial in a beacon-active epoch still demotes (it cannot
        // self-suppress, so signing would emit a rejected seedless vote).
        let beacon_active = cfg.epoch.get() >= crate::beacon::actor::DETERMINISTIC_BOOTSTRAP_EPOCH;
        let can_sign_locally = match (&cfg.beacon, beacon_active) {
            // Have the local polynomial → signer (self-suppresses seed votes when
            // it holds no share; always able to Nullify).
            (Some(_), _) => true,
            // Pre-beacon (pure-multisig) epoch → a normal signer.
            (None, false) => true,
            // Beacon-active epoch, no local polynomial → verify-only.
            (None, true) => false,
        };
        // `build_signer` returns `None` exactly when the keypair is not in the
        // committee BiMap (genuinely rotated out) — distinct from a member whose
        // local key merely mismatches.
        let member_signer = cfg.signer_keypair.as_ref().and_then(|keypair| {
            build_signer(&namespace, bimap.clone(), keypair, cfg.beacon.clone())
        });
        // Verify-only scheme is MULTISIG-ONLY (`verify_certificate` ignores the
        // seed now that the PK_E layer is gone) — no group key needed. The
        // registered scheme only verifies assembled certs for the marshal (this
        // node's engine is aborted on demotion).
        let verify_only = |bimap| build_verifier(&namespace, bimap, None);
        // DEVNET/TEST-ONLY: whether this node is a signing committee member (the
        // first match arm below). A byzantine equivocator can only double-sign if it
        // holds a signing scheme; a non-member / no-local-polynomial node falls
        // through to the honest engine (GOTCHA 7 — rotated-out / no-polynomial).
        // NOTE: `can_sign_locally` is `(Some(_), _) => true` even for a member that
        // holds the polynomial WITHOUT a share — such a node still routes here, but
        // its combined `sign()` self-suppresses (returns `None`), so the
        // `VoteEquivocator` produces nothing; its startup probe warns LOUDLY for
        // that case rather than failing silently (the genesis smoke stack seeds it).
        #[cfg(feature = "dpos-devnet-byzantine")]
        let can_sign = member_signer.is_some() && can_sign_locally;
        let scheme: BlsScheme = match member_signer {
            Some(signer) if can_sign_locally => signer,
            Some(_) => {
                // Member with NO local beacon polynomial in a beacon-active epoch
                // (a joiner that missed the epoch's DKG: it holds only the on-chain
                // group key PK_E, not the full Sharing). A group-key-only scheme can
                // verify ASSEMBLED certs but NOT individual seed partials, so it
                // cannot run as a participating Simplex member — it would reject
                // every seeded peer vote (`verify_attestation` has no polynomial to
                // check the partial against) and block its peers into a wedge. Signal
                // the unified supervisor to keep this node on the cert-follow plane
                // for the epoch (it follows assembled certs via the group key, and
                // rejoins as a full member once a reshare lands its share — see
                // dpos_beacon_share_reshare). The verify-only engine built below is
                // aborted with the stack on demotion; legacy --dpos (no supervisor
                // listening) keeps the silent verifier.
                tracing::warn!(
                    epoch = ?cfg.epoch,
                    "committee member without a local beacon polynomial — staying on cert-follow plane (no partial-verify without the Sharing)"
                );
                if cfg.signer_keypair.is_some() {
                    cfg.beacon_metrics.engine_demoted_no_polynomial.inc();
                    if let Some(tx) = &cfg.mode_events {
                        let _ = tx.send(crate::dpos::ModeEvent::NoBeaconPolynomial {
                            epoch: cfg.epoch.get(),
                        });
                    }
                }
                verify_only(bimap)
            }
            None => {
                // Genuinely not a committee member: rotated out.
                if cfg.signer_keypair.is_some() {
                    metrics::counter!("epoch_engine_rotated_out_total").increment(1);
                    cfg.beacon_metrics.engine_demoted_rotated_out.inc();
                    tracing::warn!(
                        epoch = ?cfg.epoch,
                        "validator rotated out of committee; falling through to verifier mode"
                    );
                    // Unified supervisor demotes on this signal (the verifier
                    // engine built below is aborted with the stack); legacy
                    // --dpos has no listener and keeps the silent verifier.
                    if let Some(tx) = &cfg.mode_events {
                        let _ = tx.send(crate::dpos::ModeEvent::RotatedOut {
                            epoch: cfg.epoch.get(),
                        });
                    }
                }
                verify_only(bimap)
            }
        };

        (cfg.register_scheme)(cfg.epoch, scheme.clone());

        // DEVNET/TEST-ONLY: a byzantine equivocator swaps the honest simplex engine
        // for a vote-channel double-signer ([`crate::byzantine::VoteEquivocator`]).
        // Only a SIGNING member can equivocate (otherwise its scheme can't sign a
        // vote); a non-signing flagged node falls through to the honest engine.
        // The scheme is still registered above so peers can verify its equivocating
        // votes' signatures (the slasher needs the attributable vote half). We skip
        // building the simplex engine entirely on this path.
        #[cfg(feature = "dpos-devnet-byzantine")]
        if matches!(
            cfg.byzantine,
            Some(crate::application::ByzantineMode::Equivocate)
        ) && can_sign
        {
            tracing::warn!(
                epoch = ?cfg.epoch,
                "BYZANTINE: this validator will EQUIVOCATE its votes — NEVER use in production"
            );
            return Ok(Self {
                context: ContextCell::new(context),
                inner: Inner::Equivocate(Box::new(scheme)),
            });
        }

        // Use the cross-epoch OriginEpocher threaded in via config,
        // not a per-epoch local re-construction.
        let inline = Inline::new(
            context.with_label("inline"),
            cfg.app,
            marshal_mailbox.clone(),
            cfg.epocher.clone(),
        );
        // Stage-2 beacon seed-verify at `certify`: wrap `Inline` so the boundary
        // block's seed is checked against its OWN asserted PK_E before
        // finalization (see `crate::beacon::certify`). Non-boundary blocks are
        // left to `CombinedScheme::verify_certificate` (already seed-checked at
        // notarization). The wrapper delegates Automaton/Relay verbatim.
        let automaton = BeaconCertify::new(
            inline,
            context.with_label("beacon_certify_ctx"),
            marshal_mailbox.clone(),
            cfg.seed_store,
            seed_namespace(&namespace),
        );

        let t = cfg.timeouts;
        let consensus = simplex::Engine::new(
            context.with_label("simplex"),
            simplex::Config {
                scheme,
                elector: RoundRobin::<Sha256>::shuffled(&epoch_leader_seed(&cfg.snapshot)),
                blocker: cfg.blocker,
                automaton: automaton.clone(),
                relay: automaton,
                reporter: Reporters::from((
                    Reporters::from((marshal_mailbox, slasher_mailbox)),
                    spec_exec_mailbox,
                )),
                strategy: Sequential,
                partition: format!("consensus_epoch_{}", cfg.epoch.get()),
                mailbox_size: cfg.mailbox_size,
                epoch: cfg.epoch,
                replay_buffer: REPLAY_BUFFER,
                write_buffer: WRITE_BUFFER,
                page_cache,
                leader_timeout: t.leader,
                certification_timeout: t.certification,
                timeout_retry: t.timeout_retry,
                activity_timeout: t.activity,
                skip_timeout: t.skip,
                fetch_timeout: t.fetch,
                fetch_concurrent: FETCH_CONCURRENT,
                forwarding: ForwardingPolicy::SilentLeader,
            },
        );

        Ok(Self {
            context: ContextCell::new(context),
            inner: Inner::Normal(Box::new(consensus)),
        })
    }

    /// Start the per-epoch engine. Threads the 3 simplex p2p channels
    /// (vote/cert/resolver — per-epoch Mux subchannels from
    /// [`crate::epoch_manager::Actor`]).
    pub fn start(
        self,
        vote: (
            impl Sender<PublicKey = ed25519::PublicKey>,
            impl Receiver<PublicKey = ed25519::PublicKey>,
        ),
        cert: (
            impl Sender<PublicKey = ed25519::PublicKey>,
            impl Receiver<PublicKey = ed25519::PublicKey>,
        ),
        resolver: (
            impl Sender<PublicKey = ed25519::PublicKey>,
            impl Receiver<PublicKey = ed25519::PublicKey>,
        ),
    ) -> Handle<()> {
        let Self { context, inner } = self;
        match inner {
            Inner::Normal(consensus) => {
                // The simplex engine owns its own context; this cell is otherwise
                // unused on the normal path (kept for symmetry with the byzantine
                // arm, which moves it into the equivocator).
                let _ = context;
                (*consensus).start(vote, cert, resolver)
            }
            // DEVNET/TEST-ONLY: the byzantine equivocator only needs the vote
            // channel (it double-signs received Notarize/Finalize votes); cert and
            // resolver are dropped — it never runs marshal/executor/resolver.
            #[cfg(feature = "dpos-devnet-byzantine")]
            Inner::Equivocate(scheme) => {
                drop((cert, resolver));
                crate::byzantine::VoteEquivocator::new(context.into_present(), *scheme).start(vote)
            }
        }
    }
}
