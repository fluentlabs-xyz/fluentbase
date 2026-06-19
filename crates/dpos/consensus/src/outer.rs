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

use crate::{
    application::{
        BeaconEngineLike, BeaconVerify, DerivedBlockBuilder, ExecutedChain, FluentApp,
        OrderingAssembler,
    },
    digest::Digest,
    epoch_manager,
    epocher::OriginEpocher,
    executor,
    feed_sink::FeedSink,
    order_block::OrderBlock,
    slasher,
    timeouts::ConsensusTimeouts,
};
use crate::{REPLAY_BUFFER, WRITE_BUFFER};
use alloy_primitives::{Address, B256};
use commonware_broadcast::buffered;
use commonware_consensus::{
    marshal::{
        self, core::Actor as MarshalActor, resolver::p2p as marshal_p2p, standard::Standard,
    },
    simplex::types::Finalization,
    types::{Epoch, Height, ViewDelta},
    Reporters,
};
use commonware_cryptography::{certificate::Provider as CertProvider, ed25519::PublicKey};
use commonware_p2p::{Blocker, Provider as PeerProvider, Receiver, Sender};
use commonware_parallel::Sequential;
use commonware_runtime::{
    buffer::paged::CacheRef, spawn_cell, BufferPooler, Clock, ContextCell, Handle, Metrics,
    Network as RNetwork, Pacer, Spawner, Storage,
};
use commonware_storage::archive::{immutable, Archive as _};
use commonware_utils::{NZUsize, NZU16, NZU64};
use fluentbase_bls::{keys::ValidatorBlsKeypair, PeerPubkey, Scheme as BlsScheme};
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use rand_core::CryptoRngCore;
use std::{
    collections::BTreeMap,
    num::{NonZeroU64, NonZeroUsize},
    sync::{atomic::AtomicU64, Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;

pub(crate) const PAGE_CACHE_PAGE_SIZE: std::num::NonZeroU16 = NZU16!(4_096);
pub(crate) const PAGE_CACHE_CAPACITY: NonZeroUsize = NZUsize!(8_192);
const IMMUTABLE_ITEMS_PER_SECTION: NonZeroU64 = NZU64!(262_144);
const PRUNABLE_ITEMS_PER_SECTION: NonZeroU64 = NZU64!(4_096);
pub(crate) const MAX_REPAIR: NonZeroUsize = NZUsize!(20);
const MAX_PENDING_ACKS: NonZeroUsize = NZUsize!(16);
const FREEZER_TABLE_RESIZE_FREQUENCY: u8 = 4;
const FREEZER_TABLE_RESIZE_CHUNK_SIZE: u32 = 1 << 16;
const FREEZER_VALUE_TARGET_SIZE: u64 = 1 << 30;
const FREEZER_VALUE_COMPRESSION: Option<u8> = Some(3);

// EpochSchemeProvider — minimal per-epoch BlsScheme registry; pruned to the
// trailing SCHEME_RETENTION_EPOCHS (the unified supervisor keeps one process
// alive across months — unbounded growth is no longer hypothetical).

/// Trailing epochs of BLS schemes retained for cross-epoch cert verification
/// (marshal backfill / catch-up register epochs in order, so older schemes
/// are never re-read once the frontier passes them).
const SCHEME_RETENTION_EPOCHS: usize = 8;

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
        while map.len() > SCHEME_RETENTION_EPOCHS {
            map.pop_first();
        }
    }
}

impl Default for EpochSchemeProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl EpochSchemeProvider {
    /// The scheme for the highest known epoch (the current committee). Its
    /// `participants()` are the peers to target for a finalization re-fetch on
    /// catch-up — they are connected and hold the durable finalizations.
    pub fn latest_scheme(&self) -> Option<Arc<BlsScheme>> {
        self.map.lock().unwrap().values().next_back().cloned()
    }
}

impl CertProvider for EpochSchemeProvider {
    type Scope = Epoch;
    type Scheme = BlsScheme;

    fn scoped(&self, scope: Epoch) -> Option<Arc<BlsScheme>> {
        self.map.lock().unwrap().get(&scope).cloned()
    }
}

type FinalizationsArchive<E> = immutable::Archive<E, Digest, Finalization<BlsScheme, Digest>>;
type FinalizedBlocksArchive<E> = immutable::Archive<E, Digest, OrderBlock>;
pub type MarshalMailbox = marshal::core::Mailbox<BlsScheme, Standard<OrderBlock>>;

/// Open the marshal's `finalized_blocks` immutable archive for a given
/// `partition_prefix`. Single source of the archive config so the cold-start
/// crash-survivor recovery (`dpos.rs`, opens it standalone before the engine is
/// built) and the marshal itself (`build`, below) never drift on partition names
/// or codec.
pub(crate) async fn init_finalized_blocks_archive<E>(
    context: &E,
    partition_prefix: &str,
) -> FinalizedBlocksArchive<E>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + RNetwork + Pacer,
{
    let page_cache = CacheRef::from_pooler(context, PAGE_CACHE_PAGE_SIZE, PAGE_CACHE_CAPACITY);
    immutable::Archive::init(
        context.with_label("finalized_blocks"),
        immutable::Config {
            metadata_partition: format!("{partition_prefix}-v2-finalized-blocks-metadata"),
            freezer_table_partition: format!(
                "{partition_prefix}-v2-finalized-blocks-freezer-table"
            ),
            freezer_table_initial_size: 1 << 16,
            freezer_table_resize_frequency: FREEZER_TABLE_RESIZE_FREQUENCY,
            freezer_table_resize_chunk_size: FREEZER_TABLE_RESIZE_CHUNK_SIZE,
            freezer_key_partition: format!("{partition_prefix}-v2-finalized-blocks-freezer-key"),
            freezer_key_page_cache: page_cache.clone(),
            freezer_key_write_buffer: WRITE_BUFFER,
            freezer_value_partition: format!(
                "{partition_prefix}-v2-finalized-blocks-freezer-value"
            ),
            freezer_value_write_buffer: WRITE_BUFFER,
            freezer_value_target_size: FREEZER_VALUE_TARGET_SIZE,
            freezer_value_compression: FREEZER_VALUE_COMPRESSION,
            ordinal_partition: format!("{partition_prefix}-v2-finalized-blocks-ordinal"),
            ordinal_write_buffer: WRITE_BUFFER,
            items_per_section: IMMUTABLE_ITEMS_PER_SECTION,
            codec_config: (),
            replay_buffer: REPLAY_BUFFER,
        },
    )
    .await
    .expect("init finalized blocks archive")
}

/// Init the by-height finalizations (certificate) archive. Shared by the
/// validator [`OuterBuilder::build`] and the cert-follower engine so the two
/// open byte-identical partitions with the same unbounded certificate codec
/// config — a follower started on a validator's data dir (or vice-versa) reads
/// the same archive without migration.
pub(crate) async fn init_finalizations_archive<E>(
    context: &E,
    partition_prefix: &str,
    page_cache: CacheRef,
) -> FinalizationsArchive<E>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + RNetwork + Pacer,
{
    use commonware_cryptography::certificate::Scheme as _;
    immutable::Archive::init(
        context.with_label("finalizations_by_height"),
        immutable::Config {
            metadata_partition: format!("{partition_prefix}-v3-finalizations-by-height-metadata"),
            freezer_table_partition: format!(
                "{partition_prefix}-v3-finalizations-by-height-freezer-table"
            ),
            freezer_table_initial_size: 1 << 16,
            freezer_table_resize_frequency: FREEZER_TABLE_RESIZE_FREQUENCY,
            freezer_table_resize_chunk_size: FREEZER_TABLE_RESIZE_CHUNK_SIZE,
            freezer_key_partition: format!(
                "{partition_prefix}-v3-finalizations-by-height-freezer-key"
            ),
            freezer_key_page_cache: page_cache,
            freezer_key_write_buffer: WRITE_BUFFER,
            freezer_value_partition: format!(
                "{partition_prefix}-v3-finalizations-by-height-freezer-value"
            ),
            freezer_value_write_buffer: WRITE_BUFFER,
            freezer_value_target_size: FREEZER_VALUE_TARGET_SIZE,
            freezer_value_compression: FREEZER_VALUE_COMPRESSION,
            ordinal_partition: format!("{partition_prefix}-v3-finalizations-by-height-ordinal"),
            ordinal_write_buffer: WRITE_BUFFER,
            items_per_section: IMMUTABLE_ITEMS_PER_SECTION,
            codec_config: BlsScheme::certificate_codec_config_unbounded(),
            replay_buffer: REPLAY_BUFFER,
        },
    )
    .await
    .expect("init finalizations archive")
}

type ExecutorActor<E, BE, D, XC> = executor::Actor<E, BE, D, XC, MarshalMailbox>;

/// Builder for [`OuterEngine`] — the user-facing entry point. The caller
/// hands it reth handles + genesis + cold-start EL state; `build`
/// constructs marshal → executor → FluentApp → epoch_manager in
/// dependency order.
pub struct OuterBuilder<B, P, BE, D, XC, A, R: slasher::StakingStateRead + Send + Sync + 'static> {
    // Identity / shared
    pub me: PublicKey,
    pub blocker: B,
    pub provider: P,
    pub chain_id: u64,
    pub epoch_length_blocks: NonZeroU64,
    /// `dposActivationBlock` — origin for the relative epoch numbering
    /// (`OriginEpocher`). Zero ⇒ absolute (non-migration / pristine genesis).
    pub dpos_activation_block: u64,
    pub signer_keypair: Option<ValidatorBlsKeypair>,
    /// Rotation-out signals to the unified supervisor (`None` = legacy).
    pub mode_events: Option<tokio::sync::mpsc::UnboundedSender<crate::dpos::ModeEvent>>,
    /// Per-epoch beacon resolver: returns each epoch's `BeaconKey` (live-DKG
    /// store + carry-forward + genesis fallback) so every per-epoch consensus
    /// scheme carries the seed partial under that epoch's `PK_epoch`.
    pub beacon_resolver: epoch_manager::BeaconResolver,
    /// Authoritative on-chain group-key resolver (`getEpochBeaconKey`) for the
    /// per-epoch VERIFY path (soft-enter catch-up + a member that missed the
    /// epoch's DKG). See [`epoch_manager::BeaconVerifyPk`].
    pub beacon_verify_pk: epoch_manager::BeaconVerifyPk,
    /// Beacon counters (cross-epoch singleton from `dpos.rs::launch`, already
    /// registered there). Threaded to the executor + each per-epoch engine.
    pub beacon_metrics: crate::beacon::metrics::BeaconMetrics,
    /// Live-DKG verify/propose context for `FluentApp` (the boundary "C" gate +
    /// the proposer's `beacon_outcome` assertion). `None` ⇒ no beacon gating.
    pub beacon_verify: Option<BeaconVerify>,
    pub timeouts: ConsensusTimeouts,
    pub mailbox_size: usize,
    pub deque_size: usize,
    pub partition_prefix: String,
    pub resolver_initial: Duration,
    pub resolver_timeout: Duration,
    pub resolver_fetch_retry: Duration,

    // FluentApp constructor args.
    pub genesis: OrderBlock,
    /// Unified-supervisor PROMOTION: write the synthesized anchor OrderBlock
    /// into the finalized-blocks archive before the marshal starts. The
    /// per-epoch `Inline::genesis(epoch > 0)` resolves the previous epoch's
    /// terminal block via `marshal.get_block(last(prev))` — a promoted node
    /// anchors exactly at that terminal block with an otherwise-empty (or
    /// floored-stale) archive, so without this seed the first signer view
    /// panics "missing starting epoch block". Idempotent (the archive
    /// ignores duplicate indices).
    pub seed_anchor_block: bool,
    pub beacon_engine: BE,
    /// OrderBlock → derived-EVM-block execution (node-side, reth-evm).
    pub deriver: D,
    /// Local derived-chain view (node-side, provider-backed, by-NUMBER).
    pub executed: XC,
    /// Pool-backed ordering assembly with the in-flight suffix overlay.
    pub assembler: Arc<A>,
    /// This node's proposals only — agreed data once embedded in an artifact.
    pub fee_recipient: Address,
    pub target_gas_limit: u64,
    /// Observer for finalized blocks — wired to
    /// [`fluentbase_staking_reader::EpochTransition::on_finalized`] for
    /// epoch-boundary detection (fires `boundary_tx` for `EpochManager::enter`).
    /// Required at the type level — tests pass `Arc::new(|_| {})`.
    pub boundary_hook: Arc<dyn Fn(OrderBlock) + Send + Sync>,

    /// Optional cert-feed sink: a second marshal application-`Reporter`
    /// ([`Reporters::from((app, feed))`]) that forwards finalized heights to a
    /// node-side feed actor serving the `consensus` RPC. `None` for nodes that
    /// do not serve the cert feed (e.g. tests); set on every production node.
    pub feed: Option<FeedSink>,

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
    /// Reth's in-memory canonical chain state — used by the
    /// resume-vs-migrate executor seed below (the verify-path race its
    /// ancestor-FCU guard once covered no longer exists: verify performs no
    /// EL calls under deferred execution).
    pub canonical_state:
        reth_chain_state::CanonicalInMemoryState<reth_ethereum_primitives::EthPrimitives>,

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

    /// DEVNET/TEST-ONLY byzantine validator behaviour (gated behind
    /// `dpos-devnet-byzantine`). `None` on every honest node. Threaded into
    /// [`epoch_manager::Config`] so the per-epoch engine can swap in a
    /// [`crate::byzantine::VoteEquivocator`].
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub byzantine: Option<crate::application::ByzantineMode>,
}

/// The global-singleton consensus driver wrapping a per-epoch
/// [`epoch_manager::Actor`].
pub struct OuterEngine<E, B, P, BE, D, XC, A, R>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + RNetwork + Pacer,
    B: Blocker<PublicKey = PublicKey> + Clone,
    P: PeerProvider<PublicKey = PublicKey> + Clone,
    BE: BeaconEngineLike<ExecutionData = D::Derived> + Clone + Send + Sync + 'static,
    D: DerivedBlockBuilder,
    XC: ExecutedChain,
    A: OrderingAssembler,
    R: slasher::StakingStateRead + Send + Sync + 'static,
{
    context: ContextCell<E>,
    buffered: buffered::Engine<E, PublicKey, OrderBlock, P>,
    buffer_mailbox: buffered::Mailbox<PublicKey, OrderBlock>,
    marshal: MarshalActor<
        E,
        Standard<OrderBlock>,
        EpochSchemeProvider,
        FinalizationsArchive<E>,
        FinalizedBlocksArchive<E>,
        OriginEpocher,
        Sequential,
    >,
    marshal_reporter_app: FluentApp<XC, A>,
    /// Clone of the marshal mailbox, exposed via [`OuterEngine::marshal_mailbox`]
    /// for the node-side cert feed/RPC (by-height `get_finalization`+`get_block`).
    cert_mailbox: MarshalMailbox,
    /// Optional cert-feed sink, composed with `marshal_reporter_app` at `start`.
    feed: Option<FeedSink>,
    executor: ExecutorActor<E, BE, D, XC>,
    epoch_manager: epoch_manager::Actor<E, B, XC, A>,
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

impl<B, P, BE, D, XC, A, R> OuterBuilder<B, P, BE, D, XC, A, R>
where
    B: Blocker<PublicKey = PublicKey> + Clone,
    P: PeerProvider<PublicKey = PublicKey> + Clone,
    BE: BeaconEngineLike<ExecutionData = D::Derived> + Clone + Send + Sync + 'static,
    D: DerivedBlockBuilder,
    XC: ExecutedChain,
    A: OrderingAssembler,
    R: slasher::StakingStateRead + Send + Sync + 'static,
{
    /// Construct the engine in dependency order:
    /// `buffered + archives + scheme_provider → marshal → executor →
    /// FluentApp → epoch_manager`.
    pub async fn build<E>(self, context: E) -> eyre::Result<OuterEngine<E, B, P, BE, D, XC, A, R>>
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
        // layout. Bumping MAX_COMMITTEE_SIZE past u8::MAX would silently
        // truncate to 0 on the Solidity side (turning off liveness
        // slashing) — fail at startup instead, before any block is
        // proposed. Mirrored constant: MAX_ACTIVE_VALIDATORS in
        // solidity-contracts/contracts/staking/ChainConfig.sol.
        assert!(
            fluentbase_p2p::constants::MAX_COMMITTEE_SIZE <= u8::MAX as u64,
            "wire format requires committee_size to fit u8; \
             MAX_COMMITTEE_SIZE = {} exceeds 255 — widen extra_data wire format \
             to u16 BE before bumping",
            fluentbase_p2p::constants::MAX_COMMITTEE_SIZE,
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

        let finalizations_by_height =
            init_finalizations_archive(&context, &self.partition_prefix, page_cache.clone()).await;

        let mut finalized_blocks =
            init_finalized_blocks_archive(&context, &self.partition_prefix).await;
        if self.seed_anchor_block {
            finalized_blocks
                .put(
                    self.genesis.height,
                    self.genesis.digest(),
                    self.genesis.clone(),
                )
                .await
                .map_err(|e| eyre::eyre!("seeding promotion anchor block into archive: {e:?}"))?;
        }

        // Single cross-epoch FixedEpocher + scheme provider. The same
        // instance is threaded into marshal::Config below AND into
        // epoch_manager::Config so all per-epoch engines + marshal share
        // one source of truth — no risk of divergent epoch math after a
        // hypothetical interval re-read (defense-in-depth).
        let scheme_provider = EpochSchemeProvider::new();
        let epocher = OriginEpocher::new(self.dpos_activation_block, self.epoch_length_blocks);

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
                self.dpos_activation_block,
            );
            let backfill_end_epoch = fluentbase_staking_reader::reader::epoch_of_block(
                last_consensus_finalized_height.get(),
                epoch_interval,
                self.dpos_activation_block,
            );
            if backfill_start_epoch != backfill_end_epoch {
                // Cross-epoch backfill: the lazy self-healing scheme cascade
                // handles the rotation. The marshal transiently ack-drops an
                // unregistered-epoch height and re-requests it via try_repair_gaps
                // once the boundary block's execution exposes the next committee
                // and the boundary hook registers scheme(E+1) (dpos.rs cold-start
                // registers the resumed epoch so the cascade starts at the right
                // epoch). Warn rather than bail so the reliance stays observable
                // if catch-up ever stalls.
                tracing::warn!(
                    backfill_range = ?backfill_range,
                    backfill_start_epoch,
                    backfill_end_epoch,
                    "DPoS init: backfill range crosses an epoch boundary; relying on \
                     the lazy scheme cascade to register each epoch as catch-up \
                     crosses it"
                );
            }
        }

        // Resume-vs-migrate executor seed. This uses the SAME consensus-archive
        // discriminator that `dpos.rs` uses to resolve the cold-start anchor
        // (`is_fresh_migration = last_consensus_finalized <= activation`): a genuine
        // first migration has an empty archive (`== 0`, well below the activation
        // block), whereas a restart restores it to the last DPoS finalized height.
        // When already-migrated, seed the executor HEAD from reth's actual head
        // (which the node still holds on disk) so it never issues a backward FCU to
        // a stale ancestor (reth spec-skips that → wedge). The FINALIZED seed stays
        // the consensus-archive value: reth's head can legitimately sit AHEAD of
        // consensus-finalized under reth-2.x eager verify-path canonicalization, so
        // seeding `finalized = head` would instruct reth to finalize a block
        // consensus never finalized — and if consensus then finalizes a sibling, the
        // stale finalized pointer breaks FCU monotonicity (finalized-not-ancestor) →
        // restart-proof wedge. `finalized < head` is a valid forward FCU; the
        // executor advances finalized forward as real finalizations land.
        let (initial_finalized, initial_head) =
            if last_consensus_finalized_height.get() > self.dpos_activation_block {
                // KNOWN, intentionally left as-is: `chain_info().best_number`
                // is subject to the best_number-freezes-during-pipeline-backfill
                // invariant — during a DEEP devp2p pipeline backfill it is frozen
                // (set only at on_backfill_sync_finished). The documented-correct
                // progress source would be `last_block_number()` /
                // StageCheckpointReader, but those live on the reth provider, which
                // is NOT threaded into this builder (here `provider` is the p2p
                // oracle); `canonical_state` exposes only chain_info(). Threading a
                // reth provider in solely for this seed would touch the
                // migrated-restart cold-start path for a coincidence-only gain, so
                // we leave it. Harm is bounded to migrated-restart-DURING-deep-
                // backfill (prod cold-start) and additionally suppressed by the
                // executor's `has_advanced_since_init` FCU-heartbeat gate (a stale
                // initial head is never re-sent until the first real consensus
                // advance). best_number/best_hash are mutually consistent (single
                // lock), so the seeded pair is never internally torn.
                let info = self.canonical_state.chain_info();
                (
                    self.initial_finalized,
                    (Height::new(info.best_number), info.best_hash),
                )
            } else {
                (self.initial_finalized, self.initial_head)
            };

        // Peer source for the executor's finalization re-fetch on catch-up: the
        // highest known epoch's committee (connected peers holding the durable
        // finalizations). Re-invoked per retry so it tracks the catch-up walk's
        // advancing epoch.
        let peers_for_finalization: executor::PeersForFinalization = {
            let sp = scheme_provider.clone();
            Arc::new(move || {
                use commonware_cryptography::certificate::Scheme as _;
                let scheme = sp.latest_scheme()?;
                let peers: Vec<PeerPubkey> = scheme.participants().iter().cloned().collect();
                commonware_utils::vec::NonEmptyVec::try_from(peers).ok()
            })
        };

        // Executor — depends on marshal_mailbox.
        let (executor, executor_mailbox) = executor::Actor::init(
            context.with_label("executor"),
            executor::Config {
                beacon_engine: self.beacon_engine.clone(),
                deriver: self.deriver,
                executed: self.executed.clone(),
                marshal: marshal_mailbox.clone(),
                fcu_heartbeat_interval: self.fcu_heartbeat_interval,
                last_consensus_finalized_height,
                last_execution_finalized_height: self.last_execution_finalized_height,
                initial_finalized,
                initial_head,
                fcu_pace: self.fcu_pace,
                peers_for_finalization,
                beacon_metrics: self.beacon_metrics.clone(),
            },
        );

        // Shared `round → recovered seed` map for the Stage-2 beacon certify gate
        // (`crate::beacon::certify`). The spec-exec reporter writes it (it already
        // recovers the seed per notarization); each per-epoch `BeaconCertify`
        // wrapper reads it. Cross-epoch singleton.
        let seed_store = crate::beacon::certify::new_seed_store();

        // Notarization arm of the simplex reporter — forwards `SpecNotarized`
        // to the executor for speculative execution. Built from a mailbox clone
        // before `FluentApp` consumes `executor_mailbox`. Also writes the recovered
        // seed into `seed_store` for the certify gate.
        let spec_exec_mailbox =
            crate::spec_exec::Mailbox::new(executor_mailbox.clone(), Some(seed_store.clone()));

        // FluentApp (needs executor_mailbox + marshal_mailbox + sidecar state).
        // The beacon seed feed lives here, NOT on the executor: the partial is
        // triggered at notarize-time (verify→true / own propose), so seed(h) is
        // recovered by the time h finalizes (sign-at-notarize).
        let latest_finalized_height = Arc::new(AtomicU64::new(0));
        let app = {
            let app = FluentApp::new(
                self.genesis,
                executor_mailbox,
                self.boundary_hook,
                Some(marshal_mailbox.clone()),
                latest_finalized_height,
                self.executed,
                self.assembler,
                self.fee_recipient,
                self.target_gas_limit,
            );
            match self.beacon_verify {
                Some(bv) => app.with_beacon(bv),
                None => app,
            }
        };
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
                chain_id: self.chain_id,
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
                mode_events: self.mode_events,
                app,
                timeouts: self.timeouts,
                mailbox_size: self.mailbox_size,
                beacon_resolver: self.beacon_resolver,
                beacon_verify_pk: self.beacon_verify_pk,
                marshal_mailbox: marshal_mailbox.clone(),
                slasher_mailbox,
                spec_exec_mailbox,
                seed_store,
                beacon_metrics: self.beacon_metrics,
                page_cache,
                register_scheme,
                #[cfg(feature = "dpos-devnet-byzantine")]
                byzantine: self.byzantine,
            },
        );

        Ok(OuterEngine {
            context: ContextCell::new(context),
            buffered,
            buffer_mailbox,
            marshal,
            marshal_reporter_app,
            cert_mailbox: marshal_mailbox.clone(),
            feed: self.feed,
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

impl<E, B, P, BE, D, XC, A, R> OuterEngine<E, B, P, BE, D, XC, A, R>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + RNetwork + Pacer,
    B: Blocker<PublicKey = PublicKey> + Clone,
    P: PeerProvider<PublicKey = PublicKey> + Clone,
    BE: BeaconEngineLike<ExecutionData = D::Derived> + Clone + Send + Sync + 'static,
    D: DerivedBlockBuilder,
    XC: ExecutedChain,
    A: OrderingAssembler,
    R: slasher::StakingStateRead + Send + Sync + 'static,
{
    /// Sender held by 03's `EpochTransition` to fire boundary triggers.
    pub fn boundary_sender(&self) -> mpsc::Sender<(Epoch, ValidatorSetSnapshot)> {
        self.boundary_tx.clone()
    }

    /// Clone of the marshal mailbox for the node-side cert feed/RPC. Call before
    /// [`OuterEngine::start`] (which consumes `self`) and hand it up to the node
    /// so its feed actor can answer `get_finalization`+`get_block` by height.
    pub fn marshal_mailbox(&self) -> MarshalMailbox {
        self.cert_mailbox.clone()
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
        // Compose the cert-feed sink as a second application-Reporter so it
        // observes every finalization alongside `FluentApp` (the executor path).
        // `From<(R1, Option<R2>)>` makes the feed optional; absent → app-only.
        let app_reporter: Reporters<marshal::Update<OrderBlock>, FluentApp<XC, A>, FeedSink> =
            Reporters::from((self.marshal_reporter_app, self.feed));
        let mut marshal_handle =
            self.marshal
                .start(app_reporter, self.buffer_mailbox, marshal_chan);
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
