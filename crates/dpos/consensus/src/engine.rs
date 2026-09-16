//! Per-epoch consensus engine: simplex::Engine + Inline wrapper for one epoch.
//!
//! Owns only the per-epoch components (simplex + Inline). The marshal mailbox,
//! buffered::Engine, archives, and EpochSchemeProvider are global singletons
//! living in [`crate::outer::OuterEngine`].

use crate::{
    application::{ExecutedChain, FluentApp, OrderingAssembler},
    digest::Digest,
    epocher::OriginEpocher,
    order_block::OrderBlock,
    scheme::epoch_committee_from_snapshot,
    slasher::Mailbox as SlasherMailbox,
    timeouts::ConsensusTimeouts,
    weighted_vrf::WeightedVrf,
    REPLAY_BUFFER, WRITE_BUFFER,
};
use commonware_consensus::{
    marshal::{
        core::Mailbox as MarshalMailbox,
        standard::{Inline, Standard},
    },
    simplex::{self, config::ForwardingPolicy, types::Activity},
    types::Epoch,
    Reporters,
};
use commonware_cryptography::ed25519;
use commonware_p2p::{Blocker, Receiver, Sender};
use commonware_parallel::Sequential;
use commonware_runtime::{
    buffer::paged::CacheRef, BufferPooler, Clock, ContextCell, Handle, Metrics, Spawner, Storage,
};
use fluentbase_bls::Scheme as BlsScheme;
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use rand_core::CryptoRngCore;
use std::sync::Arc;

const FETCH_CONCURRENT: usize = 4;

/// The automaton and relay handed to simplex: the per-epoch `Inline` built in
/// [`EpochEngine::new`]. `Inline`'s own availability gate is the only one; the
/// epoch key does not ride a block, so there is nothing to verify at certify.
type AutomatonFor<E, XC, A> = Inline<E, BlsScheme, FluentApp<XC, A>, OrderBlock, OriginEpocher>;

type ConsensusEngine<E, B, XC, A> = simplex::Engine<
    E,
    BlsScheme,
    WeightedVrf,
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

/// Constructor parameters for [`EpochEngine`]. `EvSink` is intentionally absent:
/// `marshal_mailbox` is the sole simplex reporter, and slashing/evidence
/// reporting is wired separately through the slasher actor.
pub struct EpochEngineConfig<B, XC, A> {
    pub blocker: B,
    pub snapshot: ValidatorSetSnapshot,
    pub epoch: Epoch,
    /// The leader lottery, built by the reconciler from the epoch's frozen
    /// committee record before this engine exists.
    ///
    /// Handed down built rather than derived here, for the same reason
    /// [`Self::scheme`] is: the weights it needs are non-optional only in the
    /// committee module's record, and deriving it here would mean a spawn that
    /// has already raised this epoch's scheme to the signer half can still
    /// discover it has no leader schedule.
    pub elector: WeightedVrf,
    /// Single cross-epoch `OriginEpocher` instance, shared by marshal and engine;
    /// `origin = dposActivationBlock`.
    pub epocher: OriginEpocher,
    pub app: FluentApp<XC, A>,
    pub timeouts: ConsensusTimeouts,
    pub mailbox_size: usize,
    /// The scheme this engine votes and verifies with, built by the randomness
    /// subsystem and handed down whole. Whether it carries a seed partial and
    /// whether it can sign at all were decided above this engine.
    pub scheme: BlsScheme,
    /// Prefix of this engine's journal partition (see [`engine_partition`]).
    /// Production passes `""`, so the on-disk name stays `consensus_epoch_{E}`;
    /// the in-crate deterministic testbed passes `node{i}-` because its nodes
    /// share one in-memory `Storage` and would otherwise replay each other's
    /// voter journal.
    pub partition_prefix: String,
    /// devnet/test-only byzantine validator behaviour (gated behind
    /// `dpos-devnet-byzantine`). `None` on every honest node. When
    /// `Some(ByzantineMode::Equivocate)` (and this node can sign), `new()` builds
    /// the [`Inner::Equivocate`] variant instead of the honest `simplex::Engine`.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub byzantine: Option<crate::byzantine::ByzantineMode>,
}

/// The journal partition of the ordering-plane engine for `epoch`, under
/// `prefix`; an empty prefix yields `consensus_epoch_{epoch}`.
pub fn engine_partition(prefix: &str, epoch: u64) -> String {
    format!("{prefix}consensus_epoch_{epoch}")
}

/// The per-epoch engine variant chosen in [`EpochEngine::new`]. Honest nodes are
/// always [`Inner::Normal`]; only a devnet/test byzantine node (gated behind
/// `dpos-devnet-byzantine`) takes [`Inner::Equivocate`], which swaps the honest
/// `simplex::Engine` for a [`crate::byzantine::VoteEquivocator`] on the vote
/// channel.
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
        // Every family this engine and its children register carries `epoch` as an
        // attribute rather than a name or label: with no epoch, N concurrent
        // engines export N copies of each simplex series with identical label sets,
        // which Prometheus drops silently; with `with_label` the epoch would grow
        // the family set without bound and make every dashboard query
        // epoch-specific.
        let context = context.with_attribute("epoch", cfg.epoch.get());

        // A non-unique committee is reachable from on-chain data, which does not
        // enforce cross-validator key uniqueness. Return an error so the caller can
        // skip entering this epoch rather than panicking the whole epoch_manager
        // task (which collapses the DPoS stack via the outer supervisor).
        let committee = epoch_committee_from_snapshot(&cfg.snapshot).map_err(|e| {
            eyre::eyre!(
                "epoch {} snapshot has non-unique participants: {e:?}",
                cfg.epoch.get(),
            )
        })?;
        let bimap = committee.bimap;
        // Taken before `bimap` is moved into the verify-only scheme below, so the
        // app and the scheme share one snapshot.
        let committee_index = Arc::new(bimap.clone());

        // The scheme arrives built: the reconciler owns the role decision and asks
        // the randomness subsystem for it, so this engine exists only for a member
        // that holds a usable scheme. The committee decode above is therefore
        // performed twice for one spawn — deliberate, because threading the decoded
        // value through the verdict would put a committee type back on the
        // randomness surface.
        let scheme = cfg.scheme;

        // devnet/test-only: a byzantine equivocator swaps the honest simplex engine
        // for a vote-channel double-signer. Only a signing member can equivocate;
        // a non-signing flagged node falls through to the honest engine. The scheme
        // is already in the committee module's map so peers can verify its votes'
        // signatures.
        #[cfg(feature = "dpos-devnet-byzantine")]
        if matches!(
            cfg.byzantine,
            Some(crate::byzantine::ByzantineMode::Equivocate)
        ) && {
            // Only a signing member can equivocate. Asked of the scheme itself
            // rather than carried as a bool, so the two cannot drift apart.
            use commonware_cryptography::certificate::Scheme as _;
            scheme.me().is_some()
        } {
            tracing::warn!(
                epoch = ?cfg.epoch,
                "BYZANTINE: this validator will EQUIVOCATE its votes — NEVER use in production"
            );
            return Ok(Self {
                context: ContextCell::new(context),
                inner: Inner::Equivocate(Box::new(scheme)),
            });
        }

        // Inject this epoch's pubkey→index map into the app: `bimap` is decoded
        // from the same snapshot the reconciler handed `signer_scheme` to derive
        // `cfg.scheme` from, so the index the app computes for a block's leader and
        // the committee this engine votes with are one agreed snapshot by
        // construction. An instance with no engine never reaches here, which is why
        // `FluentApp::committee_index` being `None` elsewhere is sound: those
        // instances cast no votes.
        let app = cfg.app.with_committee_index(committee_index);

        let inline = Inline::new(
            context.with_label("inline"),
            app,
            marshal_mailbox.clone(),
            cfg.epocher.clone(),
        );
        let automaton = inline;

        let t = cfg.timeouts;
        let consensus = simplex::Engine::new(
            context.with_label("simplex"),
            simplex::Config {
                scheme,
                elector: cfg.elector,
                blocker: cfg.blocker,
                automaton: automaton.clone(),
                relay: automaton,
                reporter: Reporters::from((
                    Reporters::from((marshal_mailbox, slasher_mailbox)),
                    spec_exec_mailbox,
                )),
                strategy: Sequential,
                partition: engine_partition(&cfg.partition_prefix, cfg.epoch.get()),
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

    /// Start the per-epoch engine on the three simplex p2p channels
    /// (vote/cert/resolver — per-epoch Mux subchannels).
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
            // devnet/test-only: the byzantine equivocator only needs the vote
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
