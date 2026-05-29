//! Per-epoch consensus engine: simplex::Engine + Inline wrapper for one epoch.
//!
//! Owns only the per-epoch components (simplex + Inline). The marshal mailbox,
//! buffered::Engine, archives, and EpochSchemeProvider are global singletons
//! living in [`crate::outer::OuterEngine`].

use std::sync::Arc;

use commonware_consensus::{
    marshal::{
        core::Mailbox as MarshalMailbox,
        standard::{Inline, Standard},
    },
    simplex::{self, config::ForwardingPolicy, elector::RoundRobin, types::Activity},
    types::{Epoch, FixedEpocher},
    Reporters,
};
use commonware_cryptography::{ed25519, Sha256};
use commonware_p2p::{Blocker, Receiver, Sender};
use commonware_parallel::Sequential;
use commonware_runtime::{
    buffer::paged::CacheRef, BufferPooler, Clock, ContextCell, Handle, Metrics, Spawner, Storage,
};
use commonware_utils::NZUsize;
use fluentbase_bls::{
    fluent_namespace,
    keys::ValidatorBlsKeypair,
    scheme::{build_signer, build_verifier},
    Scheme as BlsScheme,
};
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use rand_core::CryptoRngCore;

use crate::{
    application::FluentApp, block::Block, digest::Digest, elector_seed::epoch_leader_seed,
    scheme::epoch_committee_from_snapshot, slasher::Mailbox as SlasherMailbox,
    timeouts::ConsensusTimeouts,
};

const REPLAY_BUFFER: std::num::NonZeroUsize = NZUsize!(8 * 1024 * 1024);
const WRITE_BUFFER: std::num::NonZeroUsize = NZUsize!(1024 * 1024);
const FETCH_CONCURRENT: usize = 4;

type InlineFor<E, PB, BE, AB, Attrs> =
    Inline<E, BlsScheme, FluentApp<PB, BE, AB, Attrs>, Block, FixedEpocher>;

type ConsensusEngine<E, B, PB, BE, AB, Attrs> = simplex::Engine<
    E,
    BlsScheme,
    RoundRobin<Sha256>,
    B,
    Digest,
    InlineFor<E, PB, BE, AB, Attrs>,
    InlineFor<E, PB, BE, AB, Attrs>,
    Reporters<
        Activity<BlsScheme, Digest>,
        MarshalMailbox<BlsScheme, Standard<Block>>,
        SlasherMailbox,
    >,
    Sequential,
>;

/// Constructor parameters for [`EpochEngine`].
///
/// `EvSink` is intentionally absent: `marshal_mailbox` is the sole simplex
/// reporter (it impls `Reporter<Activity = Activity<S, V::Commitment>>`).
/// Slashing/evidence reporting is wired separately through the slasher actor.
pub struct EpochEngineConfig<B, PB, BE, AB, Attrs> {
    pub blocker: B,
    pub snapshot: ValidatorSetSnapshot,
    pub epoch: Epoch,
    /// Single cross-epoch `FixedEpocher` instance threaded from
    /// [`crate::outer::OuterBuilder::build`] (no per-epoch re-construction;
    /// marshal and engine share the same instance).
    pub epocher: FixedEpocher,
    pub chain_id: u64,
    pub signer_keypair: Option<ValidatorBlsKeypair>,
    pub app: FluentApp<PB, BE, AB, Attrs>,
    pub timeouts: ConsensusTimeouts,
    pub mailbox_size: usize,
    /// Callback that registers this epoch's [`BlsScheme`] in
    /// [`crate::outer::EpochSchemeProvider`] so marshal can verify
    /// cross-epoch finalization certificates (provider is never pruned).
    pub register_scheme: Arc<dyn Fn(Epoch, BlsScheme) + Send + Sync>,
}

/// Per-epoch consensus engine. Created by
/// [`crate::epoch_manager::Actor::enter`] on each boundary trigger.
pub struct EpochEngine<E, B, PB, BE, AB, Attrs>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics,
    B: Blocker<PublicKey = ed25519::PublicKey>,
    PB: crate::application::PayloadBuilderLike<
            BuiltSealed = reth_primitives_traits::SealedBlock<reth_ethereum_primitives::Block>,
        > + Clone
        + Send
        + Sync
        + 'static,
    BE: crate::application::BeaconEngineLike<
            PayloadAttrs = Attrs,
            ExecutionData = reth_primitives_traits::SealedBlock<reth_ethereum_primitives::Block>,
        > + Clone
        + Send
        + Sync
        + 'static,
    AB: crate::application::PayloadAttrsBuilderLike<Attrs = Attrs, Header = alloy_consensus::Header>
        + Clone
        + Send
        + Sync
        + 'static,
    Attrs: Clone + Send + Sync + 'static,
{
    context: ContextCell<E>,
    consensus: ConsensusEngine<E, B, PB, BE, AB, Attrs>,
}

impl<E, B, PB, BE, AB, Attrs> EpochEngine<E, B, PB, BE, AB, Attrs>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics,
    B: Blocker<PublicKey = ed25519::PublicKey> + Clone,
    PB: crate::application::PayloadBuilderLike<
            BuiltSealed = reth_primitives_traits::SealedBlock<reth_ethereum_primitives::Block>,
        > + Clone
        + Send
        + Sync
        + 'static,
    BE: crate::application::BeaconEngineLike<
            PayloadAttrs = Attrs,
            ExecutionData = reth_primitives_traits::SealedBlock<reth_ethereum_primitives::Block>,
        > + Clone
        + Send
        + Sync
        + 'static,
    AB: crate::application::PayloadAttrsBuilderLike<Attrs = Attrs, Header = alloy_consensus::Header>
        + Clone
        + Send
        + Sync
        + 'static,
    Attrs: Clone + Send + Sync + 'static,
{
    /// Build per-epoch simplex::Engine + Inline.
    ///
    /// `marshal_mailbox` + `page_cache` are passed in from [`crate::outer::OuterEngine`]
    /// (cross-epoch singletons).
    pub fn new(
        context: E,
        cfg: EpochEngineConfig<B, PB, BE, AB, Attrs>,
        marshal_mailbox: MarshalMailbox<BlsScheme, Standard<Block>>,
        slasher_mailbox: SlasherMailbox,
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
        let scheme: BlsScheme = cfg
            .signer_keypair
            .as_ref()
            .and_then(|keypair| build_signer(&namespace, bimap.clone(), keypair))
            .unwrap_or_else(|| {
                if cfg.signer_keypair.is_some() {
                    metrics::counter!("epoch_engine_rotated_out_total").increment(1);
                    tracing::warn!(
                        epoch = ?cfg.epoch,
                        "validator rotated out of committee; falling through to verifier mode"
                    );
                }
                build_verifier(&namespace, bimap)
            });

        (cfg.register_scheme)(cfg.epoch, scheme.clone());

        // Use the cross-epoch FixedEpocher threaded in via config,
        // not a per-epoch local re-construction.
        let inline = Inline::new(
            context.with_label("inline"),
            cfg.app,
            marshal_mailbox.clone(),
            cfg.epocher.clone(),
        );

        let t = cfg.timeouts;
        let consensus = simplex::Engine::new(
            context.with_label("simplex"),
            simplex::Config {
                scheme,
                elector: RoundRobin::<Sha256>::shuffled(&epoch_leader_seed(&cfg.snapshot)),
                blocker: cfg.blocker,
                automaton: inline.clone(),
                relay: inline,
                reporter: Reporters::from((marshal_mailbox, slasher_mailbox)),
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
            consensus,
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
        let _ = &self.context;
        self.consensus.start(vote, cert, resolver)
    }
}
