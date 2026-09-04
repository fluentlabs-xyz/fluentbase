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

/// The automaton+relay handed to simplex: the per-epoch `Inline` built in
/// [`EpochEngine::new`]. It used to be wrapped for a beacon seed-verify at
/// `certify`; the epoch key no longer rides a block, so there is nothing left to
/// check there and `Inline`'s own availability gate stands alone
/// ([`crate::beacon::certify`] records why).
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

/// Constructor parameters for [`EpochEngine`].
///
/// `EvSink` is intentionally absent: `marshal_mailbox` is the sole simplex
/// reporter (it impls `Reporter<Activity = Activity<S, V::Commitment>>`).
/// Slashing/evidence reporting is wired separately through the slasher actor.
pub struct EpochEngineConfig<B, XC, A> {
    pub blocker: B,
    pub snapshot: ValidatorSetSnapshot,
    pub epoch: Epoch,
    /// Base for the leader elector's seedless arm: the witness seed of the E-1
    /// terminal block, resolved by the epoch manager's boundary lookup
    /// (`epoch_manager::Actor::boundary_lookup`), else the constant derivation
    /// where no witness can exist.
    pub fallback_seed: [u8; 32],
    /// Single cross-epoch `OriginEpocher` instance threaded from
    /// [`crate::outer::OuterBuilder::build`] (no per-epoch re-construction;
    /// marshal and engine share the same instance). `origin = dposActivationBlock`.
    pub epocher: OriginEpocher,
    pub app: FluentApp<XC, A>,
    pub timeouts: ConsensusTimeouts,
    pub mailbox_size: usize,
    /// Callback that registers this epoch's [`BlsScheme`] in
    /// [`crate::outer::EpochSchemeProvider`] so marshal can verify
    /// cross-epoch finalization certificates (trailing-window pruned; see SCHEME_RETENTION_EPOCHS).
    pub register_scheme: Arc<dyn Fn(Epoch, BlsScheme) + Send + Sync>,
    /// The scheme this engine votes and verifies with, built by the randomness
    /// subsystem and handed down whole ([`crate::beacon::Randomness::signer_scheme`]).
    /// The engine no longer knows what a beacon key is: whether this scheme
    /// carries a seed partial, and whether it can sign at all, were decided
    /// above it.
    pub scheme: BlsScheme,
    /// DEVNET/TEST-ONLY byzantine validator behaviour (gated behind
    /// `dpos-devnet-byzantine`). `None` on every honest node. When
    /// `Some(ByzantineMode::Equivocate)` (and this node can sign), `new()` builds
    /// the [`Inner::Equivocate`] variant instead of the honest `simplex::Engine`.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub byzantine: Option<crate::byzantine::ByzantineMode>,
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
        // Every family this engine and its children register carries `epoch` as a
        // LABEL, not as part of the name. Without it the per-epoch engines all
        // register the same series under the same fixed name prefix, and a node
        // running N engines at once exports N copies of every simplex family with
        // identical label sets — which Prometheus ingests as "different value but
        // same timestamp" and drops silently at `up = 1` (measured: 223 series
        // repeated up to 7x, 55% of samples lost per scrape).
        //
        // `with_attribute` and NOT a per-epoch `with_label`: a label goes into the
        // metric NAME, so an epoch there would grow the family set without bound
        // and make every dashboard query epoch-specific. The runtime's exposition
        // writer groups all registrations of one family under a SINGLE HELP/TYPE
        // header (`commonware_runtime::utils::MetricEncoder`), so labelled
        // duplicates do not reproduce the second-HELP-line failure that costs the
        // whole scrape. Runtime task gauges deliberately ignore attributes, so
        // their cardinality is unchanged.
        let context = context.with_attribute("epoch", cfg.epoch.get());

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
        // Taken here, before `bimap` is moved into the verify-only scheme below,
        // so the app and the scheme demonstrably share one snapshot. See the
        // injection at the `Inline::new` site for why that adjacency matters.
        let committee_index = Arc::new(bimap.clone());

        // The scheme arrives BUILT. The reconciler
        // ([`crate::epoch_manager::Actor::reconcile_roles`]) owns the role
        // decision and asks the randomness subsystem for the scheme; non-members
        // (Verifier) and shareless beacon-active members (the share-gate) are
        // routed to a verify-only scheme WITHOUT a participating engine, so this
        // engine exists only for a member that holds a usable scheme. The one
        // exception is the rotated-out safety net — a (peer,bls)-key mismatch
        // yields a verify-only scheme here too, and the reconciler aborts this
        // engine on its next reconcile.
        //
        // The committee decode above is therefore performed twice for one spawn
        // (here, and inside `signer_scheme`). Deliberate: this decode feeds the
        // app's `committee_index`, and threading the decoded value through the
        // verdict would put a committee type back on the randomness surface.
        let scheme = cfg.scheme;

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
            Some(crate::byzantine::ByzantineMode::Equivocate)
        ) && {
            // Only a SIGNING member can equivocate. Asked of the scheme itself
            // rather than carried alongside it as a bool: `me()` is `Some`
            // exactly for the signer scheme `signer_scheme` builds, so the two
            // cannot drift apart.
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

        // Inject THIS epoch's pubkey→index map into the app the engine is about
        // to run. The injection sits here, adjacent to the `Inline::new` move,
        // for a reason worth stating: `bimap` is decoded from `cfg.snapshot`, the
        // SAME snapshot the reconciler handed `signer_scheme` to derive
        // `cfg.scheme` from, so the index the app computes for a block's leader
        // and the committee this engine votes with are one agreed snapshot by
        // construction — no shared registry, no lookup that can miss, nothing
        // node-local on a vote path with zero quorum slack.
        //
        // Every engine-owning instance reaches here — a signing member, and also
        // the key-mismatch verify-only safety net (`SignerVerdict::RotatedKey`)
        // that the reconciler aborts on its next pass. Both get a correct map. What does NOT reach
        // here is an instance with no engine at all, which is why
        // `FluentApp::committee_index` being `None` elsewhere is sound: those
        // instances cast no votes.
        let app = cfg.app.with_committee_index(committee_index);

        // Use the cross-epoch OriginEpocher threaded in via config,
        // not a per-epoch local re-construction.
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
                elector: WeightedVrf::try_new(&cfg.snapshot, cfg.fallback_seed)?,
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
