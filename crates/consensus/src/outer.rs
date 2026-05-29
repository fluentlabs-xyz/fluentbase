//! OuterEngine — cross-epoch singleton wrapper around `EpochManager`.
//!
//! `OuterBuilder::build` constructs subsystems in dependency order:
//!   `marshal` → `executor` → `FluentApp` → `epoch_manager`.
//!
//! Lifetime alignment:
//! - **global singletons**: `buffered::Engine`, `marshal::core::Actor`,
//!   `finalizations_by_height`, `finalized_blocks`,
//!   `EpochSchemeProvider`, `executor`.
//! - **per-epoch**: `simplex::Engine` + `Inline` wrapper (inside
//!   `EpochEngine` via `EpochManager`).

use std::{
    collections::BTreeMap,
    num::{NonZeroU64, NonZeroUsize},
    sync::{atomic::AtomicU64, Arc, Mutex},
    time::Duration,
};

use alloy_consensus::Header;
use alloy_primitives::{Bytes, B256};
use alloy_rpc_types_engine::PayloadId;
use commonware_broadcast::buffered;
use commonware_consensus::{
    marshal::{
        self, core::Actor as MarshalActor, resolver::p2p as marshal_p2p, standard::Standard,
    },
    simplex::types::Finalization,
    types::{Epoch, FixedEpocher, Height, Round, ViewDelta},
};
use commonware_cryptography::{
    certificate::{Provider as CertProvider, Scheme as CertScheme},
    ed25519::PublicKey,
};
use commonware_p2p::{Blocker, Provider as PeerProvider, Receiver, Sender};
use commonware_parallel::Sequential;
use commonware_runtime::{
    buffer::paged::CacheRef, spawn_cell, BufferPooler, Clock, ContextCell, Handle, Metrics,
    Network as RNetwork, Pacer, Spawner, Storage,
};
use commonware_storage::archive::immutable;
use commonware_utils::{NZUsize, NZU16, NZU64};
use dashmap::DashMap;
use fluentbase_bls::{keys::ValidatorBlsKeypair, Scheme as BlsScheme};
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use rand_core::CryptoRngCore;
use reth_ethereum_primitives::Block as RethBlock;
use reth_primitives_traits::SealedBlock;
use tokio::sync::mpsc;

use crate::{
    application::{BeaconEngineLike, FluentApp, PayloadAttrsBuilderLike, PayloadBuilderLike},
    block::Block,
    digest::Digest,
    epoch_manager, executor, slasher,
    timeouts::ConsensusTimeouts,
};

const REPLAY_BUFFER: NonZeroUsize = NZUsize!(8 * 1024 * 1024);
const WRITE_BUFFER: NonZeroUsize = NZUsize!(1024 * 1024);
const PAGE_CACHE_PAGE_SIZE: std::num::NonZeroU16 = NZU16!(4_096);
const PAGE_CACHE_CAPACITY: NonZeroUsize = NZUsize!(8_192);
const IMMUTABLE_ITEMS_PER_SECTION: NonZeroU64 = NZU64!(262_144);
const PRUNABLE_ITEMS_PER_SECTION: NonZeroU64 = NZU64!(4_096);
const MAX_REPAIR: NonZeroUsize = NZUsize!(20);
const MAX_PENDING_ACKS: NonZeroUsize = NZUsize!(16);
const FREEZER_TABLE_RESIZE_FREQUENCY: u8 = 4;
const FREEZER_TABLE_RESIZE_CHUNK_SIZE: u32 = 1 << 16;
const FREEZER_VALUE_TARGET_SIZE: u64 = 1 << 30;
const FREEZER_VALUE_COMPRESSION: Option<u8> = Some(3);

// EpochSchemeProvider — minimal per-epoch BlsScheme registry; never pruned.

#[derive(Clone)]
pub struct EpochSchemeProvider {
    map: Arc<Mutex<BTreeMap<Epoch, Arc<BlsScheme>>>>,
}

impl EpochSchemeProvider {
    pub fn new() -> Self {
        Self {
            map: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Register a [`BlsScheme`] for `epoch`.
    ///
    /// Insert-or-equal with a verifier→signer direction guard.
    /// The only legitimate same-epoch re-registration path is the
    /// cold-start sequence in `dpos.rs` (`cold_start_register`), where a verifier-mode scheme
    /// is registered first (so marshal can verify cross-epoch certs
    /// before any `EpochEngine` for this epoch exists) and the engine
    /// later overwrites it with a signer-mode scheme. Three outcomes:
    ///
    /// 1. **Vacant slot** → insert. Normal path for every new epoch.
    /// 2. **Same committee, verifier → signer** → overwrite.
    /// 3. **Different committee, OR signer → verifier downgrade** →
    ///    refuse + log error. Either a bug, a malicious caller, or an
    ///    accidental late `cold_start_register` after the engine started.
    ///
    /// Committee equality goes through the upstream
    /// [`commonware_cryptography::certificate::Scheme::participants`]
    /// accessor (returns `&Set<PeerPubkey>`); direction via
    /// [`commonware_cryptography::certificate::Scheme::me`] (`Some(idx)` when
    /// signer, `None` when verifier). No new accessors needed on
    /// `BlsScheme` — both are on the upstream trait already.
    pub fn register(&self, epoch: Epoch, scheme: BlsScheme) {
        use commonware_cryptography::certificate::Scheme as _;
        let mut map = self.map.lock().unwrap();
        match map.entry(epoch) {
            std::collections::btree_map::Entry::Vacant(v) => {
                v.insert(Arc::new(scheme));
            }
            std::collections::btree_map::Entry::Occupied(mut o) => {
                let existing = o.get();
                if existing.participants() != scheme.participants() {
                    tracing::error!(
                        ?epoch,
                        "EpochSchemeProvider::register rejected re-register with \
                         different committee — preserving existing entry"
                    );
                    return;
                }
                let existing_is_signer = existing.me().is_some();
                let new_is_signer = scheme.me().is_some();
                if existing_is_signer && !new_is_signer {
                    tracing::error!(
                        ?epoch,
                        "EpochSchemeProvider::register refused signer→verifier \
                         downgrade — preserving existing entry (cold-start \
                         transition is verifier→signer only)"
                    );
                    return;
                }
                o.insert(Arc::new(scheme));
            }
        }
    }
}

impl Default for EpochSchemeProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CertProvider for EpochSchemeProvider {
    type Scope = Epoch;
    type Scheme = BlsScheme;

    fn scoped(&self, scope: Epoch) -> Option<Arc<BlsScheme>> {
        let got = self.map.lock().unwrap().get(&scope).cloned();
        got
    }
}

type FinalizationsArchive<E> = immutable::Archive<E, Digest, Finalization<BlsScheme, Digest>>;
type FinalizedBlocksArchive<E> = immutable::Archive<E, Digest, Block>;
type MarshalMailbox = marshal::core::Mailbox<BlsScheme, Standard<Block>>;

type ExecutorActor<E, BE, Attrs> = executor::Actor<E, BE, Attrs, MarshalMailbox>;

/// Builder for [`OuterEngine`] — the user-facing entry point. The caller
/// hands it reth handles + genesis + cold-start EL state; `build`
/// constructs marshal → executor → FluentApp → epoch_manager in
/// dependency order.
pub struct OuterBuilder<
    B,
    P,
    PB,
    BE,
    AB,
    Attrs,
    R: slasher::StakingStateRead + Send + Sync + 'static,
> {
    // Identity / shared
    pub me: PublicKey,
    pub blocker: B,
    pub provider: P,
    pub chain_id: u64,
    pub epoch_length_blocks: NonZeroU64,
    pub signer_keypair: Option<ValidatorBlsKeypair>,
    pub timeouts: ConsensusTimeouts,
    pub mailbox_size: usize,
    pub deque_size: usize,
    pub partition_prefix: String,
    pub resolver_initial: Duration,
    pub resolver_timeout: Duration,
    pub resolver_fetch_retry: Duration,

    // FluentApp constructor args.
    pub genesis: Block,
    pub payload_builder: PB,
    pub beacon_engine: BE,
    pub payload_attrs_builder: AB,
    pub payload_resolve_time: Duration,
    /// Observer for finalized blocks — wired to
    /// [`fluentbase_staking_reader::EpochTransition::on_finalized`] for
    /// epoch-boundary detection (fires `boundary_tx` for `EpochManager::enter`).
    /// Required at the type level — tests pass `Arc::new(|_| {})`.
    pub boundary_hook: Arc<dyn Fn(Block) + Send + Sync>,

    // Executor cold-start state (read from the reth provider).
    pub last_execution_finalized_height: u64,
    pub initial_finalized: (Height, B256),
    pub initial_head: (Height, B256),
    /// When migrating (Tempo→DPoS), the anchor height to seed the marshal's
    /// in-order dispatch floor so it does not backfill pre-anchor history it
    /// will never receive. `None` on normal restart (floor comes from metadata).
    pub marshal_floor: Option<Height>,
    pub fcu_heartbeat_interval: Duration,
    pub fcu_pace: Duration,
    /// Reth's in-memory canonical chain state — threaded into the
    /// executor so `canonicalize`'s spec-compliant ancestor-FCU guard
    /// can detect when reth's canonical was silently advanced by
    /// `FluentApp::verify`'s direct `new_payload` calls.
    pub canonical_state:
        reth_chain_state::CanonicalInMemoryState<reth_ethereum_primitives::EthPrimitives>,

    /// PhantomData carrier for `Attrs` (forwarded through FluentApp /
    /// executor; doesn't appear directly in any Builder field).
    pub _attrs: std::marker::PhantomData<Attrs>,

    /// Shared between [`crate::application::FluentApp`] (writer in
    /// `propose`) and `FluentPayloadBuilder` (reader in `try_build`).
    /// dpos.rs constructs a single `Arc<DashMap>` and threads it through
    /// both consumers so per-PayloadId `extra_data` injection is race-free.
    pub extra_data_registry: Arc<DashMap<PayloadId, Bytes>>,

    /// `Staking.sol` predeploy address (`StakingReaderConfig.staking_address`).
    pub slasher_staking_address: alloy_primitives::Address,
    /// Dedicated reader instance for slasher (NOT shared with ET).
    pub slasher_reader: R,
    /// Latest finalized hash provider (closure wrapping `node.provider`).
    pub slasher_latest_finalized_hash: slasher::actor::LatestFinalizedHash,
    /// Stale-epoch cache fallback (built in dpos.rs over the same
    /// `Arc<Mutex<ValidatorSetCache>>` that `EpochTransition` writes).
    pub slasher_stale_fallback: std::sync::Arc<dyn slasher::actor::StaleEpochFallback>,
    /// TxPool transport (signer + pool + provider wrapper from dpos.rs).
    pub slasher_sink: std::sync::Arc<dyn slasher::actor::SlasherTxSink>,
    /// WAL storage partition name. The actual `queue::shared` handles
    /// are initialised inside [`OuterBuilder::build`] under the slasher's
    /// own context label.
    pub slasher_wal_partition: String,
}

/// The global-singleton consensus driver wrapping a per-epoch
/// [`epoch_manager::Actor`].
pub struct OuterEngine<E, B, P, PB, BE, AB, Attrs, R>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + RNetwork + Pacer,
    B: Blocker<PublicKey = PublicKey> + Clone,
    P: PeerProvider<PublicKey = PublicKey> + Clone,
    PB: PayloadBuilderLike<BuiltSealed = SealedBlock<RethBlock>> + Clone + Send + Sync + 'static,
    BE: BeaconEngineLike<PayloadAttrs = Attrs, ExecutionData = SealedBlock<RethBlock>>
        + Clone
        + Send
        + Sync
        + 'static,
    AB: PayloadAttrsBuilderLike<Attrs = Attrs, Header = Header> + Clone + Send + Sync + 'static,
    Attrs: Clone + Send + Sync + 'static,
    R: slasher::StakingStateRead + Send + Sync + 'static,
{
    context: ContextCell<E>,
    buffered: buffered::Engine<E, PublicKey, Block, P>,
    buffer_mailbox: buffered::Mailbox<PublicKey, Block>,
    marshal: MarshalActor<
        E,
        Standard<Block>,
        EpochSchemeProvider,
        FinalizationsArchive<E>,
        FinalizedBlocksArchive<E>,
        FixedEpocher,
        Sequential,
    >,
    marshal_reporter_app: FluentApp<PB, BE, AB, Attrs>,
    executor: ExecutorActor<E, BE, Attrs>,
    epoch_manager: epoch_manager::Actor<E, B, PB, BE, AB, Attrs>,
    slasher: slasher::Actor<E, R>,
    boundary_tx: mpsc::Sender<(Epoch, ValidatorSetSnapshot)>,
    scheme_provider: EpochSchemeProvider,
    me: PublicKey,
    blocker: B,
    provider: P,
    mailbox_size: usize,
    resolver_initial: Duration,
    resolver_timeout: Duration,
    resolver_fetch_retry: Duration,
}

impl<B, P, PB, BE, AB, Attrs, R> OuterBuilder<B, P, PB, BE, AB, Attrs, R>
where
    B: Blocker<PublicKey = PublicKey> + Clone,
    P: PeerProvider<PublicKey = PublicKey> + Clone,
    PB: PayloadBuilderLike<BuiltSealed = SealedBlock<RethBlock>> + Clone + Send + Sync + 'static,
    BE: BeaconEngineLike<PayloadAttrs = Attrs, ExecutionData = SealedBlock<RethBlock>>
        + Clone
        + Send
        + Sync
        + 'static,
    AB: PayloadAttrsBuilderLike<Attrs = Attrs, Header = Header> + Clone + Send + Sync + 'static,
    Attrs: Clone + Send + Sync + 'static,
    R: slasher::StakingStateRead + Send + Sync + 'static,
{
    /// Construct the engine in dependency order:
    /// `buffered + archives + scheme_provider → marshal → executor →
    /// FluentApp → epoch_manager`.
    pub async fn build<E>(
        self,
        context: E,
    ) -> eyre::Result<OuterEngine<E, B, P, PB, BE, AB, Attrs, R>>
    where
        E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + RNetwork + Pacer,
    {
        // Fail loud and early on misconfigured timeouts so a deep panic
        // inside commonware (`voter/actor.rs:136`) becomes an actionable
        // startup error instead.
        self.timeouts
            .validated()
            .expect("ConsensusTimeouts invariants violated");
        // The simplex-attestation wire format encodes committee_size as
        // u8; in Solidity LivenessSlashing.processBitmap mirrors that
        // layout. Bumping MAX_PEER_SET_SIZE past u8::MAX would silently
        // truncate to 0 on the Solidity side (turning off liveness
        // slashing) — fail at startup instead, before any block is
        // proposed. Mirrored constant: MAX_ACTIVE_VALIDATORS in
        // solidity-contracts/contracts/staking/ChainConfig.sol.
        assert!(
            fluentbase_p2p::constants::MAX_PEER_SET_SIZE <= u8::MAX as u64,
            "wire format requires committee_size to fit u8; \
             MAX_PEER_SET_SIZE = {} exceeds 255 — widen extra_data wire format \
             to u16 BE before bumping",
            fluentbase_p2p::constants::MAX_PEER_SET_SIZE,
        );
        // round_index in FluentApp::report must retain at least
        // marshal's view_retention_timeout window (= activity * 10);
        // otherwise verify can request a cert for a round marshal still
        // archives but round_index has already evicted, returning false
        // → Nullify → liveness loss.
        assert!(
            crate::application::ROUND_INDEX_RETENTION
                >= self.timeouts.activity.get().saturating_mul(10),
            "ROUND_INDEX_RETENTION ({}) must be >= marshal view_retention_timeout \
             (activity * 10 = {}); a future bump of timeouts.activity past {} \
             requires widening ROUND_INDEX_RETENTION",
            crate::application::ROUND_INDEX_RETENTION,
            self.timeouts.activity.get().saturating_mul(10),
            crate::application::ROUND_INDEX_RETENTION / 10,
        );
        let (buffered, buffer_mailbox) = buffered::Engine::new(
            context.with_label("buffered"),
            buffered::Config {
                public_key: self.me.clone(),
                mailbox_size: self.mailbox_size,
                deque_size: self.deque_size,
                priority: true,
                codec_config: (),
                peer_provider: self.provider.clone(),
            },
        );

        let page_cache = CacheRef::from_pooler(&context, PAGE_CACHE_PAGE_SIZE, PAGE_CACHE_CAPACITY);

        let finalizations_by_height = immutable::Archive::init(
            context.with_label("finalizations_by_height"),
            immutable::Config {
                metadata_partition: format!(
                    "{}-finalizations-by-height-metadata",
                    self.partition_prefix
                ),
                freezer_table_partition: format!(
                    "{}-finalizations-by-height-freezer-table",
                    self.partition_prefix
                ),
                freezer_table_initial_size: 1 << 16,
                freezer_table_resize_frequency: FREEZER_TABLE_RESIZE_FREQUENCY,
                freezer_table_resize_chunk_size: FREEZER_TABLE_RESIZE_CHUNK_SIZE,
                freezer_key_partition: format!(
                    "{}-finalizations-by-height-freezer-key",
                    self.partition_prefix
                ),
                freezer_key_page_cache: page_cache.clone(),
                freezer_key_write_buffer: WRITE_BUFFER,
                freezer_value_partition: format!(
                    "{}-finalizations-by-height-freezer-value",
                    self.partition_prefix
                ),
                freezer_value_write_buffer: WRITE_BUFFER,
                freezer_value_target_size: FREEZER_VALUE_TARGET_SIZE,
                freezer_value_compression: FREEZER_VALUE_COMPRESSION,
                ordinal_partition: format!(
                    "{}-finalizations-by-height-ordinal",
                    self.partition_prefix
                ),
                ordinal_write_buffer: WRITE_BUFFER,
                items_per_section: IMMUTABLE_ITEMS_PER_SECTION,
                codec_config: BlsScheme::certificate_codec_config_unbounded(),
                replay_buffer: REPLAY_BUFFER,
            },
        )
        .await
        .expect("init finalizations archive");

        let finalized_blocks = immutable::Archive::init(
            context.with_label("finalized_blocks"),
            immutable::Config {
                metadata_partition: format!("{}-finalized-blocks-metadata", self.partition_prefix),
                freezer_table_partition: format!(
                    "{}-finalized-blocks-freezer-table",
                    self.partition_prefix
                ),
                freezer_table_initial_size: 1 << 16,
                freezer_table_resize_frequency: FREEZER_TABLE_RESIZE_FREQUENCY,
                freezer_table_resize_chunk_size: FREEZER_TABLE_RESIZE_CHUNK_SIZE,
                freezer_key_partition: format!(
                    "{}-finalized-blocks-freezer-key",
                    self.partition_prefix
                ),
                freezer_key_page_cache: page_cache.clone(),
                freezer_key_write_buffer: WRITE_BUFFER,
                freezer_value_partition: format!(
                    "{}-finalized-blocks-freezer-value",
                    self.partition_prefix
                ),
                freezer_value_write_buffer: WRITE_BUFFER,
                freezer_value_target_size: FREEZER_VALUE_TARGET_SIZE,
                freezer_value_compression: FREEZER_VALUE_COMPRESSION,
                ordinal_partition: format!("{}-finalized-blocks-ordinal", self.partition_prefix),
                ordinal_write_buffer: WRITE_BUFFER,
                items_per_section: IMMUTABLE_ITEMS_PER_SECTION,
                codec_config: (),
                replay_buffer: REPLAY_BUFFER,
            },
        )
        .await
        .expect("init finalized blocks archive");

        // Single cross-epoch FixedEpocher + scheme provider. The same
        // instance is threaded into marshal::Config below AND into
        // epoch_manager::Config so all per-epoch engines + marshal share
        // one source of truth — no risk of divergent epoch math after a
        // hypothetical interval re-read (defense-in-depth).
        let scheme_provider = EpochSchemeProvider::new();
        let epocher = FixedEpocher::new(self.epoch_length_blocks);

        let (marshal, marshal_mailbox, last_consensus_finalized_height) = MarshalActor::init(
            context.with_label("marshal"),
            finalizations_by_height,
            finalized_blocks,
            marshal::Config {
                provider: scheme_provider.clone(),
                epocher: epocher.clone(),
                partition_prefix: self.partition_prefix.clone(),
                mailbox_size: self.mailbox_size,
                view_retention_timeout: ViewDelta::new(
                    self.timeouts.activity.get().saturating_mul(10),
                ),
                prunable_items_per_section: PRUNABLE_ITEMS_PER_SECTION,
                replay_buffer: REPLAY_BUFFER,
                key_write_buffer: WRITE_BUFFER,
                value_write_buffer: WRITE_BUFFER,
                block_codec_config: (),
                max_repair: MAX_REPAIR,
                max_pending_acks: MAX_PENDING_ACKS,
                page_cache: page_cache.clone(),
                strategy: Sequential,
            },
        )
        .await;

        // Tempo→DPoS swap: seed the marshal's in-order floor to the anchor so it
        // dispatches from anchor+1 instead of chasing pre-anchor history that no
        // DPoS node holds (would otherwise stall Update::Block forever). Buffered
        // in the mailbox until the marshal actor starts in `run`; SetFloor is
        // raises-only, so this is a no-op on a normal restart that passed the flag.
        if let Some(floor) = self.marshal_floor {
            marshal_mailbox.set_floor(floor).await;
        }

        // Reject crash-recovery backfill ranges that cross an epoch
        // boundary — per-epoch BLS scheme rotation across backfill is
        // not yet supported.
        // Loud operator-actionable error beats a silent wrong-snapshot
        // read from staking_reader. Empty range = no-op for the
        // Tempo→DPoS migration path (cons_fin = 0).
        let backfill_range =
            (self.last_execution_finalized_height + 1)..=last_consensus_finalized_height.get();
        if !backfill_range.is_empty() {
            // On-chain `epochBlockInterval` is u32; epoch_of_block takes u32.
            // Guard the NonZeroU64→u32 narrowing: a value > u32::MAX would
            // truncate (and a nonzero multiple of 2^32 would truncate to 0 →
            // div-by-zero in epoch_of_block). Unreachable for a u32-sourced
            // interval, asserted as defense-in-depth.
            debug_assert!(
                self.epoch_length_blocks.get() <= u32::MAX as u64,
                "epoch_length_blocks exceeds u32 — epoch_of_block interval would truncate"
            );
            let epoch_interval = self.epoch_length_blocks.get() as u32;
            let backfill_start_epoch = fluentbase_staking_reader::reader::epoch_of_block(
                self.last_execution_finalized_height,
                epoch_interval,
            );
            let backfill_end_epoch = fluentbase_staking_reader::reader::epoch_of_block(
                last_consensus_finalized_height.get(),
                epoch_interval,
            );
            if backfill_start_epoch != backfill_end_epoch {
                eyre::bail!(
                    "DPoS init: backfill range {:?} crosses epoch boundary \
                     ({backfill_start_epoch} -> {backfill_end_epoch}). \
                     Per-epoch BLS scheme rotation across backfill is not \
                     yet supported. Operator action: snapshot-restore EL \
                     disk to the most recent epoch-boundary finalized \
                     block, or wipe commonware marshal storage and re-sync \
                     from peers.",
                    backfill_range,
                );
            }
        }

        // Executor — depends on marshal_mailbox.
        let (executor, executor_mailbox) = executor::Actor::init(
            context.with_label("executor"),
            executor::Config {
                beacon_engine: self.beacon_engine.clone(),
                marshal: marshal_mailbox.clone(),
                fcu_heartbeat_interval: self.fcu_heartbeat_interval,
                last_consensus_finalized_height,
                last_execution_finalized_height: self.last_execution_finalized_height,
                initial_finalized: self.initial_finalized,
                initial_head: self.initial_head,
                fcu_pace: self.fcu_pace,
                extra_data_registry: self.extra_data_registry.clone(),
                canonical_state: self.canonical_state.clone(),
            },
        );

        // FluentApp (needs executor_mailbox + marshal_mailbox + sidecar state).
        let latest_finalized_height = Arc::new(AtomicU64::new(0));
        let round_index = Arc::new(Mutex::new(BTreeMap::<Round, Height>::new()));
        let app = FluentApp::new(
            self.genesis,
            self.payload_builder,
            self.beacon_engine,
            self.payload_attrs_builder,
            executor_mailbox,
            self.boundary_hook,
            self.payload_resolve_time,
            Some(marshal_mailbox.clone()),
            latest_finalized_height,
            round_index,
            self.extra_data_registry,
        );
        let marshal_reporter_app = app.clone();

        let scheme_provider_for_cb = scheme_provider.clone();
        let register_scheme: Arc<dyn Fn(Epoch, BlsScheme) + Send + Sync> =
            Arc::new(move |epoch, scheme| scheme_provider_for_cb.register(epoch, scheme));

        // Slasher — built before EpochManager so its mailbox can be threaded
        // into `epoch_manager::Config` as the second arm of the simplex
        // `Reporters` multiplex.
        //
        // Initialise the durable WAL queue under the slasher's own context
        // label. The queue (writer, reader) pair is built here
        // because `queue::shared::init` is async and `Actor::init` is sync.
        let slasher_ctx = context.with_label("slasher");
        let (wal_writer, wal_reader) = slasher::actor::init_wal_queue(
            slasher_ctx.with_label("wal"),
            self.slasher_wal_partition,
        )
        .await
        .expect("slasher WAL queue init failed");
        let (slasher, slasher_mailbox) = slasher::Actor::init(
            slasher_ctx,
            slasher::Config {
                staking_address: self.slasher_staking_address,
                reader: self.slasher_reader,
                latest_finalized_hash: self.slasher_latest_finalized_hash,
                // Thread the per-epoch scheme provider for pre-submit verify.
                scheme_provider: scheme_provider.clone(),
                // Stale-epoch cache fallback.
                stale_fallback: self.slasher_stale_fallback,
                // TxPool transport (signer + pool + provider).
                sink: self.slasher_sink,
                // Durable WAL split between producer/consumer tasks.
                wal_writer,
                wal_reader,
            },
        );

        // EpochManager — gets the SAME FixedEpocher instance threaded
        // through to its per-epoch engines (single source of truth).
        let (epoch_manager, boundary_tx) = epoch_manager::Actor::new(
            context.with_label("epoch_manager"),
            epoch_manager::Config {
                me: self.me.clone(),
                blocker: self.blocker.clone(),
                chain_id: self.chain_id,
                epocher: epocher.clone(),
                signer_keypair: self.signer_keypair,
                app,
                timeouts: self.timeouts,
                mailbox_size: self.mailbox_size,
                marshal_mailbox,
                slasher_mailbox,
                page_cache,
                register_scheme,
            },
        );

        Ok(OuterEngine {
            context: ContextCell::new(context),
            buffered,
            buffer_mailbox,
            marshal,
            marshal_reporter_app,
            executor,
            epoch_manager,
            slasher,
            boundary_tx,
            scheme_provider,
            me: self.me,
            blocker: self.blocker,
            provider: self.provider,
            mailbox_size: self.mailbox_size,
            resolver_initial: self.resolver_initial,
            resolver_timeout: self.resolver_timeout,
            resolver_fetch_retry: self.resolver_fetch_retry,
        })
    }
}

impl<E, B, P, PB, BE, AB, Attrs, R> OuterEngine<E, B, P, PB, BE, AB, Attrs, R>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + RNetwork + Pacer,
    B: Blocker<PublicKey = PublicKey> + Clone,
    P: PeerProvider<PublicKey = PublicKey> + Clone,
    PB: PayloadBuilderLike<BuiltSealed = SealedBlock<RethBlock>> + Clone + Send + Sync + 'static,
    BE: BeaconEngineLike<PayloadAttrs = Attrs, ExecutionData = SealedBlock<RethBlock>>
        + Clone
        + Send
        + Sync
        + 'static,
    AB: PayloadAttrsBuilderLike<Attrs = Attrs, Header = Header> + Clone + Send + Sync + 'static,
    Attrs: Clone + Send + Sync + 'static,
    R: slasher::StakingStateRead + Send + Sync + 'static,
{
    /// Sender held by 03's `EpochTransition` to fire boundary triggers.
    pub fn boundary_sender(&self) -> mpsc::Sender<(Epoch, ValidatorSetSnapshot)> {
        self.boundary_tx.clone()
    }

    /// Cold-start: register the initial (pre-finalization) scheme.
    pub fn cold_start_register(&self, epoch: Epoch, scheme: BlsScheme) {
        self.scheme_provider.register(epoch, scheme);
    }

    /// 5-channel start. Threads:
    ///   vote/cert/resolver → EpochManager (per-epoch via Muxers)
    ///   broadcast → buffered::Engine
    ///   marshal_p2p → marshal::resolver::p2p::init → marshal::core::Actor
    /// Also spawns the executor in `run`.
    #[allow(clippy::too_many_arguments)]
    pub fn start<VS, VR, CS, CR, RS, RR, BS, BR, MS, MR>(
        mut self,
        ctx_for_resolver: E,
        votes: (VS, VR),
        certs: (CS, CR),
        resolver: (RS, RR),
        broadcast: (BS, BR),
        marshal_p2p_chan: (MS, MR),
    ) -> Handle<()>
    where
        VS: Sender<PublicKey = PublicKey>,
        VR: Receiver<PublicKey = PublicKey>,
        CS: Sender<PublicKey = PublicKey>,
        CR: Receiver<PublicKey = PublicKey>,
        RS: Sender<PublicKey = PublicKey>,
        RR: Receiver<PublicKey = PublicKey>,
        BS: Sender<PublicKey = PublicKey>,
        BR: Receiver<PublicKey = PublicKey>,
        MS: Sender<PublicKey = PublicKey>,
        MR: Receiver<PublicKey = PublicKey>,
    {
        let (marshal_rx, marshal_resolver) = marshal_p2p::init::<_, _, _, Digest, MS, MR, _>(
            &ctx_for_resolver,
            marshal_p2p::Config {
                public_key: self.me.clone(),
                peer_provider: self.provider.clone(),
                blocker: self.blocker.clone(),
                mailbox_size: self.mailbox_size,
                initial: self.resolver_initial,
                timeout: self.resolver_timeout,
                fetch_retry_timeout: self.resolver_fetch_retry,
                priority_requests: true,
                priority_responses: true,
            },
            marshal_p2p_chan,
        );

        spawn_cell!(
            self.context,
            self.run(
                votes,
                certs,
                resolver,
                broadcast,
                (marshal_rx, marshal_resolver),
            )
            .await
        )
    }

    #[allow(clippy::too_many_arguments)]
    async fn run<VS, VR, CS, CR, RS, RR, BS, BR, MarshalResolver>(
        self,
        votes: (VS, VR),
        certs: (CS, CR),
        resolver: (RS, RR),
        broadcast: (BS, BR),
        marshal_chan: (
            mpsc::Receiver<marshal::resolver::handler::Message<Digest>>,
            MarshalResolver,
        ),
    ) where
        VS: Sender<PublicKey = PublicKey>,
        VR: Receiver<PublicKey = PublicKey>,
        CS: Sender<PublicKey = PublicKey>,
        CR: Receiver<PublicKey = PublicKey>,
        RS: Sender<PublicKey = PublicKey>,
        RR: Receiver<PublicKey = PublicKey>,
        BS: Sender<PublicKey = PublicKey>,
        BR: Receiver<PublicKey = PublicKey>,
        MarshalResolver: commonware_resolver::Resolver<
            Key = marshal::resolver::handler::Request<Digest>,
            PublicKey = PublicKey,
        >,
    {
        // Start `epoch_manager` FIRST so its `boundary_rx` is
        // draining before `marshal` starts firing the `Update::Block`
        // path that ultimately triggers `boundary_hook` → bridge_tx.
        // The bridge channel buffers 64 triggers (`dpos.rs` `bridge_tx`/`bridge_rx`) which
        // absorbed the original ordering gap, but starting epoch_manager
        // first eliminates the window for live epoch transitions when
        // bursty finalization races a still-uninitialized consumer.
        let mut em_handle = self.epoch_manager.start(votes, certs, resolver);
        let mut buffered_handle = self.buffered.start(broadcast);
        let mut executor_handle = self.executor.start();
        let mut marshal_handle =
            self.marshal
                .start(self.marshal_reporter_app, self.buffer_mailbox, marshal_chan);
        let mut slasher_handle = self.slasher.start();

        // Supervisor: on first subsystem exit (clean or panic), abort the
        // other 4 to release runtime resources promptly. The outer `run` then
        // returns naturally; its caller (dpos.rs top-level select!) cancels the
        // shutdown_token, which triggers cooperative shutdown of peer-context
        // tasks (boundary_hook, bridge_forwarder). Handle::abort is idempotent
        // on already-completed handles (monorepo/runtime/src/utils/handle.rs:107-118).
        let exit = tokio::select! {
            r = &mut buffered_handle => ("buffered", r),
            r = &mut executor_handle => ("executor", r),
            r = &mut marshal_handle => ("marshal", r),
            r = &mut slasher_handle => ("slasher", r),
            r = &mut em_handle => ("epoch_manager", r),
        };

        match exit.1 {
            Ok(()) => tracing::warn!(subsystem = exit.0, "subsystem exited cleanly (unexpected)"),
            Err(e) => tracing::error!(subsystem = exit.0, error = ?e, "subsystem failed"),
        }

        buffered_handle.abort();
        executor_handle.abort();
        marshal_handle.abort();
        slasher_handle.abort();
        em_handle.abort();
    }
}
